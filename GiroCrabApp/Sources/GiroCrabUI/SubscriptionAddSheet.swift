import SwiftUI
import GiroCrabCore

/// „Abo hinzufügen“ (GitHub #1787): pick from the detected recurring expenses (search, sort, one tap
/// per subscription; the sheet stays open to add several), or create a subscription by hand.
struct SubscriptionAddSheet: View {
    @Environment(\.negativeAmountStyle) private var amountStyle
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var manualMode = false
    @State private var search = ""
    @State private var sortBy = Order.name
    @State private var accountFilter: Int64?

    enum Order: String, CaseIterable, Identifiable {
        case name, amount, lastDate
        var id: String { rawValue }
        var title: String {
            switch self {
            case .name: String(localized: "Name")
            case .amount: String(localized: "Betrag")
            case .lastDate: String(localized: "Letzte Zahlung")
            }
        }
    }

    private var candidates: [RecurringSeries] {
        let q = search.trimmingCharacters(in: .whitespaces)
        let all = model.subscriptionCandidates.filter {
            (shownAccount == nil || $0.accountId == shownAccount) && (q.isEmpty || $0.name.localizedCaseInsensitiveContains(q))
        }
        switch sortBy {
        case .name: return all.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        case .amount: return all.sorted { abs($0.amountCents) > abs($1.amountCents) }
        case .lastDate: return all.sorted { $0.lastDate > $1.lastDate }
        }
    }

    /// The chosen account while it still has candidates (else the list is not narrowed by a hidden filter).
    private var shownAccount: Int64? {
        accountFilter.flatMap { id in candidateAccounts.contains { $0.id == id } ? id : nil }
    }

    /// Accounts that have at least one detected series left.
    private var candidateAccounts: [(id: Int64, name: String)] {
        let ids = Set(model.subscriptionCandidates.map(\.accountId))
        return model.allAccounts.filter { ids.contains($0.id) }.map { (id: $0.id, name: $0.name) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(manualMode ? "Abo von Hand anlegen" : "Abo hinzufügen").appFont(.headline)
                Spacer()
                Button("Fertig") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            .padding(16)
            Divider()
            if manualMode {
                ManualSubscriptionForm(back: { manualMode = false }, done: { dismiss() })
            } else {
                picker
            }
        }
        .desktopFrame(minWidth: 520, idealWidth: 580, minHeight: manualMode ? 620 : 480, idealHeight: manualMode ? 630 : 560)
        #if os(macOS)
        .onExitCommand { dismiss() }
        #endif
    }

    private var picker: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass").foregroundStyle(Theme.tertiary).accessibilityHidden(true)
                    TextField("Erkannte Buchungen durchsuchen", text: $search).textFieldStyle(.plain)
                }
                .padding(.horizontal, 8).padding(.vertical, 5)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                HStack(spacing: 10) {
                    if candidateAccounts.count > 1 {
                        Picker("Konto", selection: Binding(get: { shownAccount }, set: { accountFilter = $0 })) {
                            Text("Alle Konten").tag(Int64?.none)
                            ForEach(candidateAccounts, id: \.id) { Text($0.name).tag(Int64?.some($0.id)) }
                        }
                        .pickerStyle(.menu).fixedSize().sensitive()
                    }
                    Picker("Sortieren", selection: $sortBy) {
                        ForEach(Order.allCases) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.menu).fixedSize()
                    Spacer(minLength: 0)
                }
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
            if candidates.isEmpty {
                ContentUnavailableView(
                    search.isEmpty ? "Keine weiteren Serien erkannt" : "Nichts gefunden",
                    systemImage: search.isEmpty ? "repeat" : "magnifyingglass",
                    description: Text(search.isEmpty
                        ? "Du kannst ein Abo von Hand anlegen oder eine Buchung in der Detailansicht als wiederkehrend markieren."
                        : "Kein erkannter Eintrag passt zur Suche."))
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(candidates, id: \.seriesKey) { s in candidateRow(s) }
                    .listStyle(.inset)
            }
            Divider()
            HStack {
                Text("\(candidates.count) erkannte Serien").appFont(.caption).foregroundStyle(Theme.secondary)
                Spacer()
                Button("Von Hand anlegen …") { manualMode = true }
            }
            .padding(12)
        }
    }

    private func candidateRow(_ s: RecurringSeries) -> some View {
        HStack(spacing: 10) {
            Image(systemName: s.categorySymbol.isEmpty ? "repeat" : s.categorySymbol)
                .foregroundStyle(Theme.secondary).frame(width: 22).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(s.name).lineLimit(1).sensitive()
                Text("\(s.intervalText) · zuletzt am \(DateText.medium(s.lastDate))")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
            }
            Spacer()
            Text(Money.format(s.amountCents, currency: model.allAccounts.first { $0.id == s.accountId }?.currency ?? "EUR")).monospacedDigit().foregroundStyle(amountStyle.ink(s.amountCents)).sensitive(.amounts)
            Button("Hinzufügen") { model.setSubscription(s, marked: true) }
                .buttonStyle(.borderless)
        }
        .padding(.vertical, 2)
    }
}

private struct ManualSubscriptionForm: View {
    @Environment(AppModel.self) private var model
    let back: () -> Void
    let done: () -> Void
    @State private var name = ""
    @State private var income = false
    @State private var amountText = ""
    @State private var group = SubscriptionGroup.monthly
    @State private var currency = "EUR"
    @State private var date = Date()
    @State private var symbol: String?
    @State private var noticeValue: Int?
    @State private var noticeUnit = NoticeUnit.months
    @State private var renewalMonths: Int?
    @State private var hasEnd = false
    @State private var end = Date()
    @State private var hasTrial = false
    @State private var trialEnd = Date()

    private var currencies: [String] {
        Array(Set(model.allAccounts.map(\.currency) + ["EUR"])).sorted { $0 == "EUR" ? true : ($1 == "EUR" ? false : $0 < $1) }
    }
    private var cents: Int64? { Money.parseCents(amountText).flatMap { $0 > 0 ? $0 : nil } }
    private var valid: Bool { !name.trimmingCharacters(in: .whitespaces).isEmpty && cents != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Form {
                TextField("Name", text: $name).sensitive()
                Picker("Art", selection: $income) {
                    Text("Ausgabe").tag(false)
                    Text("Einnahme").tag(true)
                }
                .pickerStyle(.segmented)
                LabeledContent("Betrag pro Zahlung") {
                    HStack(spacing: 6) {
                        TextField("Betrag pro Zahlung", text: $amountText, prompt: Text("0,00"))
                            .textFieldStyle(.roundedBorder).labelsHidden().frame(width: 110).multilineTextAlignment(.trailing)
                            .sensitive(.amounts)
                            .onChange(of: amountText) { _, t in if t.contains("-") { amountText = t.replacingOccurrences(of: "-", with: "") } }
                        Picker("Währung", selection: $currency) {
                            ForEach(currencies, id: \.self) { Text($0).tag($0) }
                        }
                        .labelsHidden().fixedSize()
                    }
                }
                Picker("Turnus", selection: $group) {
                    ForEach([SubscriptionGroup.monthly, .quarterly, .halfYearly, .yearly]) { Text($0.title).tag($0) }
                }
                DatePicker("Eine Zahlung am", selection: $date, displayedComponents: .date)
                LabeledContent("Symbol") { SubscriptionSymbolPicker(symbol: $symbol) }
                Section {
                    LabeledContent("Kündigungsfrist") {
                        HStack(spacing: 6) {
                            TextField("Kündigungsfrist", value: $noticeValue, format: .number, prompt: Text(""))
                                .textFieldStyle(.roundedBorder).labelsHidden().frame(width: 64).multilineTextAlignment(.trailing)
                                .onChange(of: noticeValue) { _, v in if let v, v < 0 { noticeValue = abs(v) } }
                            Picker("Einheit", selection: $noticeUnit) {
                                ForEach(NoticeUnit.allCases) { Text($0.title).tag($0) }
                            }
                            .labelsHidden().fixedSize()
                        }
                    }
                    LabeledContent("Vertragsende") {
                        HStack(spacing: 8) {
                            if hasEnd { DatePicker("", selection: $end, displayedComponents: .date).labelsHidden() }
                            Toggle("Endet am", isOn: $hasEnd).macCheckbox()
                        }
                    }
                    LabeledContent("Verlängerung um") {
                        HStack(spacing: 6) {
                            TextField("Verlängerung", value: $renewalMonths, format: .number, prompt: Text(""))
                                .textFieldStyle(.roundedBorder).labelsHidden().frame(width: 64).multilineTextAlignment(.trailing)
                                .onChange(of: renewalMonths) { _, v in if let v, v < 0 { renewalMonths = abs(v) } }
                            Text("Monate").foregroundStyle(Theme.secondary)
                        }
                    }
                    LabeledContent("Probezeit") {
                        HStack(spacing: 8) {
                            if hasTrial { DatePicker("", selection: $trialEnd, displayedComponents: .date).labelsHidden() }
                            Toggle("Endet am", isOn: $hasTrial).macCheckbox()
                        }
                    }
                } header: { Text("Vertrag (optional)").appSectionHeader() }
            }
            .formStyle(.grouped)
            Text("Aus dem Datum und dem Turnus berechnet GiroCrab die nächste Zahlung. Eine Notiz trägst du danach in der rechten Spalte ein.")
                .appFont(.caption).foregroundStyle(Theme.secondary).padding(.horizontal, 20)
            Spacer(minLength: 0)
            Divider()
            HStack {
                Button("Zurück") { back() }
                Spacer()
                Button("Anlegen") {
                    guard let c = cents else { return }
                    var details = SubscriptionDetails()
                    if let n = noticeValue, n > 0 { details.noticeValue = n; details.noticeUnit = noticeUnit }
                    if let r = renewalMonths, r > 0 { details.renewalMonths = r }
                    if hasEnd { details.contractEnd = DateText.iso(end) }
                    if hasTrial { details.trialEnd = DateText.iso(trialEnd) }
                    model.addManualSubscription(
                        name: name.trimmingCharacters(in: .whitespaces), amountCents: income ? c : -c, group: group,
                        currency: currency, startDate: DateText.iso(date), symbol: symbol, details: details)
                    done()
                }
                .disabled(!valid)
            }
            .padding(12)
        }
    }
}

/// Picks the symbol of a manual subscription from a short list (nil: the standard one).
struct SubscriptionSymbolPicker: View {
    @Binding var symbol: String?
    @State private var open = false

    static let symbols = [
        "repeat", "play.tv", "music.note", "film", "gamecontroller", "newspaper", "book", "cloud", "lock.shield",
        "shield", "car", "bolt", "flame", "drop", "wifi", "phone", "house", "cart", "dumbbell", "figure.run",
        "heart", "cross.case", "graduationcap", "creditcard", "banknote", "gift", "fork.knife", "pawprint",
        "airplane", "tram"]

    var body: some View {
        Button { open = true } label: {
            Image(systemName: symbol ?? "repeat").frame(width: 28, height: 22)
        }
        .accessibilityLabel(Text("Symbol wählen"))
        .capturablePopover(isPresented: $open) {
            LazyVGrid(columns: Array(repeating: GridItem(.fixed(34), spacing: 6), count: 6), spacing: 6) {
                ForEach(Self.symbols, id: \.self) { name in
                    Button { symbol = name == "repeat" ? nil : name; open = false } label: {
                        Image(systemName: name).frame(width: 30, height: 28)
                            .background((symbol ?? "repeat") == name ? Theme.card : Color.clear,
                                        in: RoundedRectangle(cornerRadius: 6, style: .continuous))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(12)
        }
    }
}
