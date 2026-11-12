import SwiftUI
import UniformTypeIdentifiers
import GiroCrabCore

/// "Aus MoneyMoney importieren …": either reads MoneyMoney's own database
/// (recommended, everything in one go) or a per-account CSV/XML export. Both
/// are assistants with the same frame (title, steps, Back / Continue), so
/// switching the source doesn't change the sheet.
struct MoneyMoneyImportSheet: View {
    @Environment(AppModel.self) private var model

    enum Source: Hashable { case database, export }
    #if os(macOS)
    @State private var source: Source = .database
    #else
    // MoneyMoney's database only exists on macOS; iOS offers the file export only.
    @State private var source: Source = .export
    #endif

    /// An import is running in the view below.
    @State private var importRunning = false

    var body: some View {
        Group {
            #if os(macOS)
            switch source {
            case .database: MoneyMoneyDatabaseImportView(running: $importRunning, source: $source)
            case .export: MoneyMoneyExportImportView(running: $importRunning, source: $source)
            }
            #else
            MoneyMoneyExportImportView(running: $importRunning, source: nil)
            #endif
        }
        .padding(20)
        .macFrame(width: 860)
        // the system sheet background: grouped forms then sit on it like in
        // System Settings instead of forming a box inside a coloured card
        .onAppear {
            if model.demoImportPreview != nil { source = .export }
            #if os(macOS)
            if model.demoDbPreview != nil { source = .database }
            #endif
        }
    }
}

/// Title row of the import assistants: symbol, step title and subtitle, and
/// "Schritt n von m" on the right.
struct ImportAssistantHeader: View {
    let title: String
    let subtitle: String
    /// (current, total); nil = no numbered step (running import, result).
    var step: (Int, Int)?

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "square.and.arrow.down.on.square").font(.title).foregroundStyle(Theme.accent)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.title3.weight(.semibold))
                if !subtitle.isEmpty {
                    Text(subtitle).font(.callout).foregroundStyle(Theme.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 12)
            if let (n, total) = step {
                Text("Schritt \(n) von \(total)")
                    .font(.caption).foregroundStyle(Theme.tertiary)
            }
        }
    }
}

/// Per-account CSV/XML export (one account at a time), as a two-step
/// assistant: 1. the file, 2. target account and what to take over.
struct MoneyMoneyExportImportView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// Tells the sheet that an import is running (it locks the source switch).
    @Binding var running: Bool
    /// Database or export file (macOS); nil = no switch (iOS).
    var source: Binding<MoneyMoneyImportSheet.Source>?

    enum Step: Hashable { case file, options }

    @State private var step: Step = .file
    @State private var picking = false
    @State private var targeted = false
    @State private var fileName = ""
    @State private var fileData: Data?
    /// nil = "Neues Konto aus Datei anlegen"
    @State private var target: Int64?
    @State private var preview: MoneyMoneyImportPreview?
    @State private var error: String?
    @State private var loading = false
    @State private var importing = false
    @State private var keepCategories = true
    @State private var keepNotes = true

    // MoneyMoney XML/CSV, CAMT.053 (.xml) and MT940 (.sta / .mt940).
    private let types: [UTType] = [.xml, .commaSeparatedText, .plainText, .data]

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ImportAssistantHeader(
                title: step == .file ? String(localized: "Aus MoneyMoney importieren") : String(localized: "Übernahme"),
                subtitle: subtitle,
                step: (step == .file ? 1 : 2, 2))
            Group {
                switch step {
                case .file: fileStep
                case .options: optionsStep
                }
            }
            #if os(macOS)
            .frame(maxWidth: .infinity, minHeight: 470, maxHeight: 470, alignment: .top)
            #endif
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout).foregroundStyle(Theme.warning)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            footer
        }
        .interactiveDismissDisabled(importing)
        .onChange(of: importing) { _, now in running = now }
        .onAppear {
            if let demo = model.demoImportPreview {
                preview = demo
                fileName = "MoneyMoney-Umsaetze.xml"
                fileData = Data([0])
            }
        }
        .onChange(of: target) { _, _ in refreshPreview() }
        .onChange(of: keepCategories) { _, _ in refreshPreview() }
        .fileImporter(isPresented: $picking, allowedContentTypes: types) { result in
            if case .success(let url) = result { load(url) }
        }
    }

    private var subtitle: String {
        switch step {
        case .file:
            return String(localized: "Umsätze eines Kontos aus einer MoneyMoney-Exportdatei – ohne Duplikate bei späteren Abrufen.")
        case .options:
            guard let p = preview else { return fileName }
            return [fileName, String(localized: "\(p.newBookings.formatted()) neue Buchungen"), yearSpan(p)]
                .compactMap { $0 }.joined(separator: " · ")
        }
    }

    // MARK: - 1. File

    private var fileStep: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let source {
                Picker("Quelle", selection: source) {
                    Text("MoneyMoney-Datenbank").tag(MoneyMoneyImportSheet.Source.database)
                    Text("Exportdatei (CSV/XML)").tag(MoneyMoneyImportSheet.Source.export)
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                .disabled(importing)
            }
            Form {
                Section {
                    LabeledContent("Exportdatei") {
                        HStack(spacing: 8) {
                            if loading { ProgressView().controlSize(.small) }
                            Text(fileData == nil ? String(localized: "Nicht ausgewählt") : fileName)
                                .foregroundStyle(fileData == nil ? Theme.tertiary : Theme.text)
                                .lineLimit(1).truncationMode(.middle)
                            Button(fileData == nil ? "Auswählen …" : "Ändern …") { picking = true }
                        }
                    }
                } footer: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("In MoneyMoney: Konto wählen, alle Umsätze markieren (⌘A), dann „Umsätze exportieren …“ als XML oder CSV. Sie können die Datei auch hierher ziehen.")
                        Text("Auch möglich: Kontoauszüge als CAMT.053 (XML) oder MT940 (.sta / .mt940).")
                    }
                    .font(.caption).foregroundStyle(Theme.secondary)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
            .overlay {
                // the whole step takes a dropped file; outlined while one hovers
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(Theme.accent, lineWidth: 2)
                    .opacity(targeted ? 1 : 0)
                    .allowsHitTesting(false)
            }
        }
        .dropDestination(for: URL.self) { urls, _ in
            guard let url = urls.first else { return false }
            load(url)
            return true
        } isTargeted: { targeted = $0 }
    }

    // MARK: - 2. Options

    private var optionsStep: some View {
        Form {
            Section {
                Picker("Zielkonto", selection: $target) {
                    Text("Neues Konto aus der Datei").tag(Int64?.none)
                    ForEach(model.allAccounts) { a in Text("\(a.name) · \(a.bankName)").tag(Int64?.some(a.id)) }
                }
                Toggle("Kategorien übernehmen", isOn: $keepCategories)
                Toggle("Notizen übernehmen", isOn: $keepNotes)
            } footer: {
                if target == nil, let p = preview {
                    footerText(String(localized: "Neues Archivkonto „\(p.suggestedAccountName)“ – der Saldo ist die Summe der Umsätze."))
                }
            }
            if let p = preview {
                Section {
                    LabeledContent("Neue Buchungen", value: p.newBookings.formatted())
                    if p.duplicatesSkippedOrEnriched > 0 {
                        LabeledContent("Bereits vorhanden", value: p.duplicatesSkippedOrEnriched.formatted())
                    }
                    LabeledContent("Notizen", value: p.notesCount.formatted())
                    if !p.detectedCategories.isEmpty {
                        LabeledContent("Kategorien") {
                            Text(p.detectedCategories.prefix(6).joined(separator: ", ") + (p.detectedCategories.count > 6 ? " …" : ""))
                                .lineLimit(2).multilineTextAlignment(.trailing)
                        }
                    }
                } header: {
                    Text("Vorschau")
                } footer: {
                    if p.duplicatesSkippedOrEnriched > 0 {
                        footerText(String(localized: "Vorhandene Buchungen werden nicht doppelt angelegt, sie erhalten nur Kategorie und Notiz."))
                    }
                }
            }
        }
        .formStyle(.grouped)
        .disabled(importing)
    }

    private func footerText(_ text: String) -> some View {
        Text(text)
            .font(.caption).foregroundStyle(Theme.secondary)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Footer

    private var footer: some View {
        HStack(spacing: 8) {
            Button("Abbrechen") { dismiss() }
                .keyboardShortcut(.cancelAction)
                .disabled(importing)
            Spacer()
            if importing { ProgressView().controlSize(.small) }
            if step == .options {
                Button("Zurück") { error = nil; step = .file }
                    .disabled(importing)
            }
            Button(step == .file ? "Fortfahren" : (importing ? "Importiere …" : "Importieren")) {
                if step == .file { error = nil; step = .options } else { runImport() }
            }
            .buttonStyle(.borderedProminent)
            .keyboardShortcut(.defaultAction)
            .disabled(step == .file
                      ? (preview == nil || loading)
                      : (importing || (preview?.totalRows ?? 0) == 0))
        }
    }

    private func yearSpan(_ p: MoneyMoneyImportPreview) -> String? {
        let span = [p.dateRangeStart, p.dateRangeEnd].compactMap { $0.map { String($0.prefix(4)) } }
        guard span.count == 2 else { return nil }
        return span[0] == span[1] ? span[0] : "\(span[0])–\(span[1])"
    }

    // MARK: - Actions

    private func load(_ url: URL) {
        let ok = url.startAccessingSecurityScopedResource()
        defer { if ok { url.stopAccessingSecurityScopedResource() } }
        do {
            fileData = try Data(contentsOf: url)
            fileName = url.lastPathComponent
            refreshPreview()
        } catch {
            self.error = describe(error)
        }
    }

    private func refreshPreview() {
        guard let vault = model.vault, let data = fileData else { return }
        let (name, tgt) = (fileName, target)
        loading = true; error = nil
        Task {
            let result = await Task.detached { () -> Result<MoneyMoneyImportPreview, Error> in
                Result { try vault.previewMoneymoneyImport(data: data, fileName: name, targetAccountId: tgt) }
            }.value
            loading = false
            switch result {
            case .success(let p): preview = p
            case .failure(let e):
                preview = nil
                if case CoreError.InvalidInput = e {
                    error = String(localized: "In dieser Datei wurden keine MoneyMoney-Umsätze erkannt (erwartet: CSV mit Datum/Betrag oder XML <transactions>).")
                } else { error = describe(e) }
            }
        }
    }

    private func runImport() {
        guard let vault = model.vault, let data = fileData else { return }
        let (name, tgt, cats, notes) = (fileName, target, keepCategories, keepNotes)
        importing = true
        Task {
            let result = await Task.detached { () -> Result<MoneyMoneyImportResult, Error> in
                Result { try vault.importMoneymoneyData(data: data, fileName: name, targetAccountId: tgt, preserveCategories: cats, preserveNotes: notes) }
            }.value
            importing = false
            switch result {
            case .success(let r):
                model.finishMoneyMoneyImport(r)
                dismiss()
            case .failure(let e): error = describe(e)
            }
        }
    }
}

extension AppModel {
    func finishMoneyMoneyImport(_ r: MoneyMoneyImportResult) {
        reloadAll()
        if r.accountCreated { sidebarSelection = .account(r.accountId) }
        toast = String(localized: "MoneyMoney: \(r.imported) Buchungen importiert")
            + (r.enriched > 0 ? String(localized: ", \(r.enriched) ergänzt") : "")
            + (r.duplicates > 0 ? String(localized: ", \(r.duplicates) bereits vorhanden") : "")
    }
}
