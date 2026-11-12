import SwiftUI
import GiroCrabCore

/// "Bank hinzufügen" as an assistant with the same frame as the MoneyMoney
/// import (header, fixed content area, Abbrechen / Zurück / Fortfahren):
/// 1. Bank – search by name, BLZ or IBAN; results grouped by how they connect,
/// 2. Zugang – the way to connect (when the bank offers several) and the login,
/// 3. Einrichten – only when GiroCrab still needs something once (FinTS product
///    ID, Enable Banking application).
/// "Hinzufügen" closes the assistant; the first sync (PIN, TAN, bank consent)
/// then runs in its usual sheet – no sheet on top of a sheet.
struct AddBankSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    enum Step { case bank, access, setup }

    @State private var step: Step = .bank
    @State private var query = ""
    @State private var selected: BankSearchResult?
    @State private var selectedID: String?
    /// Ways to reach the selected bank (FinTS, own API, Enable Banking).
    @State private var ways: [BankSearchResult] = []
    @State private var setupValues: [String: String] = [:]
    @State private var submit = AddBankSubmit()

    private static let suggestions = ["Sparkasse", "Volksbank", "ING", "DKB", "Postbank", "N26", "Revolut", "C24", "bunq", "Trade Republic", "Scalable Capital", "PayPal"]
    private static let manualFinTs = BankSearchResult(id: "fints:custom", name: "", mode: .finTs, country: "DE", blz: "", url: "", connector: nil, aspsp: "", tested: "")

    private var results: [BankSearchResult] {
        searchBanks(query: query, livePsd2: model.livePsd2Banks, perGroupLimit: 12)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ImportAssistantHeader(title: title, subtitle: subtitle, symbol: "building.columns")
            Group {
                switch step {
                case .bank: bankStep
                case .access: accessStep
                case .setup: setupStep
                }
            }
            #if os(macOS)
            .frame(maxWidth: .infinity, minHeight: 470, maxHeight: 470, alignment: .top)
            #endif
            AssistantFooter(
                cancel: { dismiss() },
                back: step == .bank ? nil : { back() },
                nextTitle: isLastStep ? "Hinzufügen" : "Fortfahren",
                nextDisabled: !canContinue,
                next: { next() }
            )
        }
        .padding(20)
        .macFrame(width: 860)
        .onAppear { model.loadLivePsd2Banks() }
    }

    // MARK: - Header

    private var title: String {
        switch step {
        case .bank: return String(localized: "Bank hinzufügen")
        case .access:
            guard let s = selected else { return "" }
            return s.id == Self.manualFinTs.id ? String(localized: "Andere FinTS-Bank") : s.name
        case .setup: return String(localized: "Einrichten")
        }
    }

    private var subtitle: String {
        switch step {
        case .bank:
            return String(localized: "Suchen Sie Ihre Bank nach Name, Bankleitzahl oder IBAN.")
        case .access:
            return String(localized: "Wie GiroCrab die Umsätze abruft. PIN und TAN fragt GiroCrab erst beim ersten Abruf ab.")
        case .setup:
            return String(localized: "Diese Angaben braucht GiroCrab einmalig, danach gelten sie für alle Banken dieser Art.")
        }
    }

    // MARK: - Navigation

    /// Settings this way to connect still needs (none for the demo bank).
    private var missingSettings: [String] {
        guard let s = selected else { return [] }
        let kind = s.connector ?? (s.mode == .finTs ? .finTsGeneric : .enableBanking)
        guard let d = bankCatalog().first(where: { $0.kind == kind }) else { return [] }
        return d.requiredSettings.filter { key in !(model.settingEntry(key)?.hasValue ?? false) }
    }

    private var isLastStep: Bool {
        step == .setup || (step == .access && missingSettings.isEmpty)
    }

    private var canContinue: Bool {
        switch step {
        case .bank: return selected != nil
        case .access: return submit.canSubmit
        case .setup:
            return !missingSettings.contains { (setupValues[$0] ?? "").trimmingCharacters(in: .whitespaces).isEmpty }
        }
    }

    private func next() {
        guard canContinue else { return }
        switch step {
        case .bank:
            guard let s = selected else { return }
            ways = Self.ways(to: s, in: results)
            step = .access
        case .access:
            if missingSettings.isEmpty { submit.action() } else { step = .setup }
        case .setup:
            for (k, v) in setupValues where !v.trimmingCharacters(in: .whitespaces).isEmpty {
                model.saveSetting(k, v)
            }
            submit.action()
        }
    }

    private func back() {
        switch step {
        case .bank: break
        case .access: step = .bank
        case .setup: step = .access
        }
    }

    /// The selected result plus the same bank in the other groups, in GiroCrab's
    /// order of preference: FinTS, the bank's own API, Enable Banking.
    static func ways(to r: BankSearchResult, in all: [BankSearchResult]) -> [BankSearchResult] {
        func key(_ s: String) -> String { s.lowercased().filter { $0.isLetter || $0.isNumber } }
        let k = key(r.name)
        guard k.count >= 3 else { return [r] }
        var out = [r]
        for m in [BankAccessMode.finTs, .ownApi, .psd2] where m != r.mode {
            if let other = all.first(where: {
                $0.mode == m && { let o = key($0.name); return o.count >= 3 && (o.hasPrefix(k) || k.hasPrefix(o)) }($0)
            }) {
                out.append(other)
            }
        }
        let order: [BankAccessMode] = [.finTs, .ownApi, .psd2]
        return out.sorted { order.firstIndex(of: $0.mode)! < order.firstIndex(of: $1.mode)! }
    }

    // MARK: - 1. Bank

    /// A native single-selection list: the chosen bank is highlighted like
    /// any selected row (HIG: checkmarks mean "toggle several"), arrow keys
    /// move, double-click or Return continues.
    private var bankStep: some View {
        VStack(alignment: .leading, spacing: 10) {
            searchField
            List(selection: $selectedID) {
                if query.trimmingCharacters(in: .whitespaces).isEmpty {
                    Section {
                        suggestionRow
                    } header: {
                        Text("Häufig gesucht")
                    }
                } else {
                    resultSections
                }
                Section {
                    row(symbol: "square.and.pencil", title: String(localized: "Andere FinTS-Bank"),
                        detail: String(localized: "Bankleitzahl und FinTS-Adresse selbst eingeben"))
                        .tag(Self.manualFinTs.id)
                } header: {
                    Text("Nicht gefunden?")
                }
            }
            #if os(macOS)
            .listStyle(.inset)
            #endif
            .contextMenu(forSelectionType: String.self, menu: { _ in EmptyView() }, primaryAction: { ids in
                guard let id = ids.first else { return }
                selectedID = id
                choose(id)
                next()
            })
            .onChange(of: selectedID) { _, id in choose(id) }
            HStack(spacing: 18) {
                Button("Demo-Bank ausprobieren") {
                    model.addDemoBank()
                    dismiss()
                }
                .help(String(localized: "Ohne echte Bank: Kennung „demo“, PIN 12345, beliebige TAN"))
                #if os(macOS)
                Button("Von MoneyMoney umsteigen …") {
                    dismiss()
                    model.showMoneyMoneyImport = true
                }
                #endif
            }
            .buttonStyle(.link)
            .font(.callout)
        }
    }

    private func choose(_ id: String?) {
        guard let id else { selected = nil; return }
        selected = id == Self.manualFinTs.id ? Self.manualFinTs : results.first { $0.id == id }
    }

    private var searchField: some View {
        #if os(macOS)
        SystemSearchField(text: $query, prompt: String(localized: "Bank, BLZ oder IBAN suchen …"))
            .onChange(of: query) { _, _ in selectedID = nil }
        #else
        TextField("Bank, BLZ oder IBAN suchen …", text: $query)
            .textFieldStyle(.roundedBorder)
            .autocorrectionDisabled()
            .textInputAutocapitalization(.never)
            .onChange(of: query) { _, _ in selectedID = nil }
        #endif
    }

    private var suggestionRow: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(Self.suggestions, id: \.self) { s in
                    Button(s) { query = s }
                        .buttonStyle(.bordered)
                        .controlSize(.small)
                }
            }
        }
    }

    @ViewBuilder private var resultSections: some View {
        let all = results
        if all.isEmpty {
            Section {
                Text("Keine Bank gefunden. Prüfen Sie die Schreibweise oder geben Sie die Bankleitzahl ein.")
                    .foregroundStyle(.secondary)
            }
        } else {
            ForEach([BankAccessMode.finTs, .ownApi, .psd2], id: \.self) { mode in
                let rows = all.filter { $0.mode == mode }
                if !rows.isEmpty {
                    Section {
                        ForEach(rows, id: \.id) { r in
                            row(symbol: Self.symbol(r), title: r.name, detail: Self.detailLine(r))
                                .tag(r.id)
                        }
                    } header: {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(Self.groupTitle(mode))
                            Text(Self.groupHint(mode)).font(.caption).fontWeight(.regular).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }

    /// One bank row. Hierarchical styles, so the text stays readable on the
    /// selection highlight.
    private func row(symbol: String, title: String, detail: String?) -> some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .frame(width: 22)
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                if let detail {
                    Text(detail).font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .padding(.vertical, 2)
    }

    // MARK: - 2. Access

    @ViewBuilder private var accessStep: some View {
        if let s = selected {
            Form {
                if ways.count > 1 {
                    Section {
                        Picker("Verbindung", selection: Binding(
                            get: { s.id },
                            set: { id in if let w = ways.first(where: { $0.id == id }) { selected = w } }
                        )) {
                            ForEach(ways, id: \.id) { w in
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(Self.groupTitle(w.mode))
                                    Text(Self.groupHint(w.mode)).font(.caption).foregroundStyle(Theme.secondary)
                                }
                                .padding(.vertical, 2)
                                .tag(w.id)
                            }
                        }
                        #if os(macOS)
                        .pickerStyle(.radioGroup)
                        #else
                        .pickerStyle(.inline)
                        #endif
                        .labelsHidden()
                    } header: {
                        Text("Verbindung")
                    } footer: {
                        Text("GiroCrab empfiehlt FinTS: direkt zur Bank, ohne Anmeldung bei Dritten.")
                            .font(.caption).foregroundStyle(Theme.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                Section {
                    switch s.mode {
                    case .finTs:
                        FinTsAddForm(prefill: s.id == Self.manualFinTs.id ? nil : s, onAdded: { dismiss() }, submit: submit)
                    case .ownApi:
                        if let k = s.connector { KindAddForm(kind: k, onAdded: { dismiss() }, submit: submit) }
                    case .psd2:
                        if let k = s.connector {
                            KindAddForm(kind: k, onAdded: { dismiss() }, submit: submit)
                        } else {
                            Psd2AddForm(bank: s, onAdded: { dismiss() }, submit: submit)
                        }
                    }
                } header: {
                    Text("Zugang")
                } footer: {
                    Text("Gibt es den Zugang schon (gleiche Bank, gleiche Kennung), wird er aktualisiert statt doppelt angelegt.")
                        .font(.caption).foregroundStyle(Theme.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                // a new form reports its own readiness
                .id(s.id)
            }
            .formStyle(.grouped)
            .scrollContentBackground(.hidden)
        }
    }

    // MARK: - 3. Setup

    private var setupStep: some View {
        Form {
            let keys = missingSettings
            if keys.contains(where: { BankSetupSheet.enableBankingKeys.contains($0) }) {
                EnableBankingSetupSection(values: $setupValues)
            }
            let other = keys.filter { !BankSetupSheet.enableBankingKeys.contains($0) }
            if !other.isEmpty {
                Section {
                    ForEach(other, id: \.self) { key in settingField(key) }
                } footer: {
                    if other.contains("fints.product_id") {
                        VStack(alignment: .leading, spacing: 4) {
                            Text("FinTS-Programme brauchen eine kostenlose Produktregistrierung bei der Deutschen Kreditwirtschaft. Sie dauert meist einige Tage.")
                            Link("Produktregistrierung öffnen",
                                 destination: URL(string: "https://www.fints.org/de/hersteller/produktregistrierung")!)
                        }
                        .font(.caption).foregroundStyle(Theme.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
        .formStyle(.grouped)
        .scrollContentBackground(.hidden)
    }

    @ViewBuilder private func settingField(_ key: String) -> some View {
        let entry = model.settingEntry(key)
        let label = entry?.label ?? settingLabels[key] ?? key
        let binding = Binding(get: { setupValues[key] ?? "" }, set: { setupValues[key] = $0 })
        if entry?.isSecret ?? key.contains("secret") {
            SecureField(label, text: binding, prompt: Text("Erforderlich"))
        } else {
            TextField(label, text: binding, prompt: Text("Erforderlich"))
                .autocorrectionDisabled()
        }
    }

    // MARK: - Labels

    static func groupTitle(_ m: BankAccessMode) -> LocalizedStringKey {
        switch m {
        case .finTs: return "Direkt (FinTS 3.0 / HBCI)"
        case .ownApi: return "Direkt (Eigene API)"
        case .psd2: return "PSD2 (via Enable Banking)"
        }
    }

    static func groupHint(_ m: BankAccessMode) -> LocalizedStringKey {
        switch m {
        case .finTs: return "Keine Registrierung bei Dritten nötig"
        case .ownApi: return "Direkter Zugang über die API der Bank"
        case .psd2: return "Benötigt eine kostenlose eigene Enable-Banking-Anwendung"
        }
    }

    static func symbol(_ r: BankSearchResult) -> String {
        if let k = r.connector, let d = bankCatalog().first(where: { $0.kind == k }) { return d.sfSymbol }
        return r.mode == .psd2 ? "building.columns.circle" : "building.columns"
    }

    static func detailLine(_ r: BankSearchResult) -> String? {
        switch r.mode {
        case .finTs: return String(localized: "BLZ \(r.blz)")
        case .psd2: return r.country.isEmpty ? nil : r.country
        case .ownApi: return nil
        }
    }
}
