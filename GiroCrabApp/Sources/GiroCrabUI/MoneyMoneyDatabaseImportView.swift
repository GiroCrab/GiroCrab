#if os(macOS)
import AppKit
import SwiftUI
import GiroCrabCore

/// MoneyMoney database import: pick MoneyMoney's data folder (database plus
/// statement PDFs) or a single `MoneyMoney.sqlite` (e.g. a backup; the PDFs
/// can then come from a separately chosen folder), enter the MoneyMoney
/// password, preview, then import all accounts in one go. Everything is only
/// read (never changed); the password is dropped from the view state afterwards.
struct MoneyMoneyDatabaseImportView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    /// Tells the sheet that an import is running (it locks the source tabs).
    @Binding var running: Bool

    struct Choice: Equatable {
        var selected: Bool
        var kind: AccountKind
        /// nil = "Neues Konto"
        var target: Int64?
    }

    /// MoneyMoney's data folder or a single database file.
    @State private var fileURL: URL?
    /// Folder with the statement PDFs, for a database without its folder (backup).
    @State private var statementsURL: URL?
    @State private var importStatements = true
    @State private var password = ""
    @State private var preview: MoneyMoneyDbPreview?
    @State private var choices: [Int64: Choice] = [:]
    @State private var importCategories = true
    @State private var importNotes = true
    @State private var importGroups = true
    @State private var createConnections = true
    @State private var importPins = false
    @State private var loading = false
    @State private var importing = false
    /// Where the running import is (shown instead of the options).
    @State private var progress = ImportProgressState()
    /// The running import's observer; `cancel()` stops it.
    @State private var relay: ImportProgressRelay?
    @State private var cancelling = false
    @State private var error: String?
    @State private var result: MoneyMoneyDbImportResult?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let result {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        resultCard(result)
                        if !result.psd2Connections.isEmpty { psd2Card(result.psd2Connections) }
                        if !result.accountChecks.isEmpty { checkCard(result.accountChecks) }
                    }
                    .padding(.trailing, 6)
                }
                .frame(minHeight: 280, idealHeight: 460, maxHeight: 560) // like the preview; room for "Details"
            } else if importing {
                // nothing can be changed while the import runs: progress instead of the options
                progressCard
            } else {
                info
                fileAndPassword
                if loading { ProgressView().controlSize(.small) }
                if let preview {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 14) {
                            totals(preview)
                            accountList(preview)
                            HStack(alignment: .top, spacing: 14) {
                                bankList(preview)
                                optionsCard(preview).frame(width: 330)
                            }
                        }
                        .padding(.trailing, 6)
                    }
                    .frame(minHeight: 280, idealHeight: 460, maxHeight: 560) // fits a 13" screen
                }
            }
            if let error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.callout).foregroundStyle(Theme.warning)
                    .fixedSize(horizontal: false, vertical: true) // wrap, never truncate
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
            footer
        }
        .interactiveDismissDisabled(importing)
        .onChange(of: importing) { _, now in running = now }
        .onAppear {
            if let demo = model.demoDbPreview {
                fileURL = URL(fileURLWithPath: "/Users/max/Library/Containers/com.moneymoney-app.retail/Data/Library/Application Support/MoneyMoney")
                password = "demo-passwort"
                apply(demo)
            }
        }
        .onDisappear { password = "" }
    }

    // MARK: - Pieces

    private var info: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "lock.shield").foregroundStyle(Theme.accent)
            VStack(alignment: .leading, spacing: 4) {
                Text("MoneyMoney speichert alles in einer mit Ihrem MoneyMoney-Passwort verschlüsselten Datenbank. GiroCrab öffnet sie nur lesend – die Datei und MoneyMoney bleiben unverändert.")
                Text("Kein Datenbank-Passwort in MoneyMoney? Dann vorher eines festlegen (Einstellungen → Sicherheit).")
                    .foregroundStyle(Theme.secondary)
            }
            .font(.caption).foregroundStyle(Theme.text).fixedSize(horizontal: false, vertical: true)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Theme.canvas))
    }

    private var fileAndPassword: some View {
        Grid(alignment: .leading, horizontalSpacing: 10, verticalSpacing: 8) {
            GridRow {
                Text("Quelle").font(.callout).foregroundStyle(Theme.secondary)
                HStack {
                    Text(fileURL?.lastPathComponent ?? String(localized: "MoneyMoney-Ordner oder MoneyMoney.sqlite auswählen …"))
                        .font(.callout.weight(fileURL == nil ? .regular : .medium))
                        .foregroundStyle(fileURL == nil ? Theme.secondary : Theme.text)
                        .lineLimit(1).truncationMode(.middle)
                    Spacer()
                    Button(fileURL == nil ? "Auswählen …" : "Andere Quelle …") { pickFile() }.controlSize(.small)
                }
            }
            GridRow {
                Text("Passwort").font(.callout).foregroundStyle(Theme.secondary)
                HStack {
                    SecureField("MoneyMoney-Datenbank-Passwort", text: $password)
                        .textFieldStyle(.roundedBorder)
                        .onSubmit { loadPreview() }
                    Button("Vorschau") { loadPreview() }
                        .disabled(fileURL == nil || loading || importing)
                }
            }
        }
    }

    private var progressCard: some View {
        let p = progress.value
        return VStack(alignment: .leading, spacing: 10) {
            Label("Import läuft …", systemImage: "square.and.arrow.down.on.square").font(.headline)
            ProgressView(value: p.fraction)
                .progressViewStyle(.linear)
                .tint(Theme.accent)
            Text(progressStatus(p)).font(.callout).foregroundStyle(Theme.text).lineLimit(1).truncationMode(.middle)
            if p.bookingsTotal > 0 || p.documentsTotal > 0 {
                Text(progressCounts(p)).font(.caption.monospacedDigit()).foregroundStyle(Theme.secondary)
            }
            Text(cancelling
                 ? "Der Import wird abgebrochen …"
                 : "Bitte warten Sie, bis der Import fertig ist. „Import abbrechen“ verwirft alles – es wird dann nichts übernommen.")
                .font(.caption).foregroundStyle(Theme.secondary).fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }

    private func progressStatus(_ p: MoneyMoneyDbImportProgress) -> String {
        if p.accountIndex == 0 { return String(localized: "MoneyMoney-Datenbank wird gelesen …") }
        if p.accountIndex > p.accountCount { return String(localized: "Bankzugänge und Gruppen werden angelegt …") }
        return String(localized: "Konto \(p.accountIndex) von \(p.accountCount) · \(p.accountName)")
    }

    private func progressCounts(_ p: MoneyMoneyDbImportProgress) -> String {
        var s = String(localized: "\(p.bookingsDone.formatted()) von \(p.bookingsTotal.formatted()) Buchungen")
        if p.documentsTotal > 0 {
            s += " · " + String(localized: "\(p.documentsDone.formatted()) von \(p.documentsTotal.formatted()) Dokumenten")
        }
        return s
    }

    private func totals(_ p: MoneyMoneyDbPreview) -> some View {
        let span = [p.dateRangeStart, p.dateRangeEnd].compactMap { $0.map { String($0.prefix(4)) } }
        let years = span.count == 2 ? (span[0] == span[1] ? span[0] : "\(span[0]) – \(span[1])") : "–"
        return VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 18) {
                stat(years, "Zeitraum")
                stat("\(p.accounts.count)", "Konten")
                stat(p.totalBookings.formatted(), "Buchungen")
                stat("\(p.notesCount)", "Notizen")
                stat("\(p.categoryPaths.count)", "Kategorien")
                if p.statementCount > 0 { stat(p.statementCount.formatted(), "Dokumente") }
            }
            if p.pendingBookings > 0 {
                Text("\(p.pendingBookings) vorgemerkte Buchungen werden nicht übernommen – die Bank liefert sie nach der Buchung erneut.")
                    .font(.caption).foregroundStyle(Theme.secondary)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(padding: 12)
    }

    private func accountList(_ p: MoneyMoneyDbPreview) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Konten").font(.subheadline.weight(.semibold)).padding(.bottom, 6)
            ForEach(Array(p.accounts.enumerated()), id: \.element.mmId) { i, a in
                if i > 0 { Divider() }
                accountRow(a)
            }
        }
        .card(padding: 12)
    }

    private func accountRow(_ a: MoneyMoneyDbAccount) -> some View {
        let choice = binding(a)
        return HStack(alignment: .center, spacing: 10) {
            Toggle("„\(a.name)“ importieren", isOn: choice.selected).labelsHidden().toggleStyle(.checkbox)
            VStack(alignment: .leading, spacing: 2) {
                Text(a.name).font(.callout.weight(.medium)).lineLimit(1)
                Text(accountSubtitle(a)).font(.caption).foregroundStyle(Theme.secondary).lineLimit(1)
                Text(bookingSummary(a)).font(.caption2).foregroundStyle(Theme.tertiary).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing, spacing: 2) {
                Text(Money.format(a.computedBalanceCents, currency: a.currency))
                    .font(.callout.monospacedDigit()).foregroundStyle(Theme.text)
                Text("Saldo").font(.caption2).foregroundStyle(Theme.tertiary)
            }
            .help(a.balanceSource)
            .frame(width: 110, alignment: .trailing)
            Picker("Kontoart", selection: choice.kind) {
                ForEach(AccountKind.importChoices, id: \.self) { k in Text(k.label).tag(k) }
            }
            .labelsHidden().frame(width: 150)
            Picker("Ziel", selection: choice.target) {
                Text("Neues Konto").tag(Int64?.none)
                ForEach(model.allAccounts) { acc in
                    Text(acc.id == a.existingAccountId ? String(localized: "\(acc.name) (gleiche IBAN)") : acc.name).tag(Int64?.some(acc.id))
                }
                if let id = a.existingAccountId, !model.allAccounts.contains(where: { $0.id == id }) {
                    Text(a.existingAccountName ?? String(localized: "Vorhandenes Konto")).tag(Int64?.some(id))
                }
            }
            .labelsHidden().frame(width: 190)
        }
        .padding(.vertical, 6)
        .opacity(choice.wrappedValue.selected ? 1 : 0.5)
    }

    private func bankList(_ p: MoneyMoneyDbPreview) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Bankzugänge").font(.subheadline.weight(.semibold)).padding(.bottom, 6)
            ForEach(Array(p.banks.enumerated()), id: \.element.mmId) { i, b in
                if i > 0 { Divider() }
                HStack(spacing: 10) {
                    let psd2 = psd2Bank(b)
                    Image(systemName: b.connector != nil ? "checkmark.circle.fill" : psd2 != nil ? "link.circle.fill" : "xmark.circle")
                        .foregroundStyle(b.connector != nil ? Theme.inflow : psd2 != nil ? Theme.accent : Theme.tertiary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(b.name.isEmpty ? (b.connector?.displayName ?? "Bank") : b.name).font(.callout.weight(.medium))
                        Text([b.bankCode.isEmpty ? nil : String(localized: "BLZ \(b.bankCode)"), b.loginName.isEmpty ? nil : String(localized: "Anmeldename \(b.loginName)")]
                            .compactMap { $0 }.joined(separator: " · "))
                            .font(.caption).foregroundStyle(Theme.secondary)
                        if let psd2 {
                            Text(enableBankingReady
                                 ? String(localized: "Über Enable Banking (PSD2): \(psd2.name)")
                                 : String(localized: "Über Enable Banking (PSD2): \(psd2.name) – Enable Banking wird bei der ersten Freigabe eingerichtet"))
                                .font(.caption).foregroundStyle(Theme.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if b.hasSavedPin {
                        Image(systemName: "key").font(.caption).foregroundStyle(Theme.secondary).help("In MoneyMoney ist eine PIN gespeichert")
                    }
                    Text(bankStatus(b)).font(.caption).foregroundStyle(b.connector == nil && psd2Bank(b) == nil ? Theme.tertiary : Theme.text)
                        .frame(width: 110, alignment: .trailing)
                }
                .padding(.vertical, 6)
            }
        }
        .card(padding: 12)
    }

    private func optionsCard(_ p: MoneyMoneyDbPreview) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Toggle("Kategorien übernehmen (\(p.categoryPaths.count))", isOn: $importCategories)
            if p.replacesStarterCategories && importCategories {
                Text("Ersetzt die GiroCrab-Startkategorien und ihre Regeln – Ihr Tresor enthält noch keine Buchungen.")
                    .font(.caption).foregroundStyle(Theme.secondary).padding(.leading, 2)
            }
            Toggle("Notizen übernehmen (\(p.notesCount))", isOn: $importNotes)
            let groupCount = Set(p.accounts.map(\.groupName).filter { !$0.isEmpty }).count
            if groupCount > 0 {
                Toggle("Kontogruppen übernehmen (\(groupCount))", isOn: $importGroups)
            }
            if p.statementCount > 0 {
                statementOptions(p)
            }
            Toggle("Bankzugänge anlegen", isOn: $createConnections)
            Toggle("Gespeicherte PINs übernehmen", isOn: $importPins)
                .disabled(!createConnections)
            Text("Die PINs werden dann im verschlüsselten GiroCrab-Tresor gespeichert. Ohne diese Option werden sie nicht einmal gelesen.")
                .font(.caption).foregroundStyle(Theme.secondary).padding(.leading, 2)
        }
        .toggleStyle(.switch).controlSize(.small)
        .frame(maxWidth: .infinity, alignment: .leading)
        .card(padding: 12)
    }

    /// Statement PDFs: MoneyMoney's database only lists them, the files are
    /// in its `Statements` folder – or, for a backup, wherever the user saved them.
    @ViewBuilder
    private func statementOptions(_ p: MoneyMoneyDbPreview) -> some View {
        let size = ByteCountFormatter.string(fromByteCount: Int64(p.statementsBytes), countStyle: .file)
        Toggle("Kontoauszüge & Dokumente übernehmen (\(p.statementsFound))", isOn: $importStatements)
            .disabled(p.statementsFound == 0)
        VStack(alignment: .leading, spacing: 4) {
            if p.statementsDir == nil {
                Text("MoneyMoney verzeichnet \(p.statementCount) PDF-Dokumente, die Dateien liegen aber nicht bei dieser Datenbank. Wenn Sie sie gesichert haben, wählen Sie den Ordner aus – sonst werden sie ausgelassen und lassen sich später nachholen.")
            } else if p.statementsFound < p.statementCount {
                Text("\(p.statementsFound) von \(p.statementCount) PDF-Dokumenten gefunden (\(size)). Die übrigen werden ausgelassen.")
            } else {
                Text("\(p.statementCount) PDF-Dokumente (\(size)) kommen verschlüsselt in den GiroCrab-Tresor.")
            }
            if p.statementsFound < p.statementCount {
                Button(statementsURL == nil ? "Ordner mit Auszügen wählen …" : "Anderen Ordner wählen …") { pickStatementsFolder() }
                    .controlSize(.small)
                    .disabled(loading || importing)
            }
        }
        .font(.caption).foregroundStyle(Theme.secondary).padding(.leading, 2)
    }

    private func resultCard(_ r: MoneyMoneyDbImportResult) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Label("Import abgeschlossen", systemImage: "checkmark.seal.fill").font(.headline).foregroundStyle(Theme.inflow)
            HStack(spacing: 18) {
                stat("\(r.accountsCreated)", "Konten neu")
                stat("\(r.accountsMerged)", "Konten ergänzt")
                stat(r.bookingsNew.formatted(), "neue Buchungen")
                stat("\(r.bookingsDuplicates)", "bereits vorhanden")
            }
            Group {
                if r.documentsImported > 0 {
                    Text("\(r.documentsImported) Kontoauszüge und Dokumente (PDF) übernommen.")
                }
                if r.documentsExisting > 0 {
                    Text("\(r.documentsExisting) Dokumente waren bereits vorhanden.")
                }
                if r.documentsMissing > 0 {
                    Text("\(r.documentsMissing) Dokumente wurden nicht gefunden – sie lassen sich später mit dem Ordner der PDF-Dateien nachholen.")
                }
                if r.documentsMismatched > 0 {
                    Text("\(r.documentsMismatched) Dateien stimmen nicht mit der Prüfsumme aus MoneyMoney überein und wurden nicht übernommen.")
                }
                if r.documentsTooLarge > 0 {
                    Text("\(r.documentsTooLarge) Dokumente sind größer als 25 MB und wurden nicht übernommen.")
                }
                if r.documentsUnlisted > 0 {
                    Text("\(r.documentsUnlisted) PDF-Dateien im Ordner kennt MoneyMoney nicht – sie gehören zu keinem Konto und wurden nicht übernommen.")
                }
                if r.holdingsImported > 0 {
                    Text("\(r.holdingsImported) Depotpositionen übernommen.")
                }
                if r.categoriesCreated > 0 || r.notesImported > 0 {
                    Text("\(r.categoriesCreated) Kategorien angelegt, \(r.notesImported) Notizen übernommen.")
                }
                if r.rulesCreated > 0 {
                    Text("\(r.rulesCreated) Regeln übernommen.")
                }
                if r.rulesDisabled > 0 {
                    Text("\(r.rulesDisabled) davon ausgeschaltet – sie nutzen Angaben, die GiroCrab nicht kennt, gelten nur für bestimmte Konten oder sind fehlerhaft.")
                }
                if r.connectionsCreated > 0 || r.connectionsExisting > 0 {
                    Text(String(localized: "\(r.connectionsCreated) Bankzugänge angelegt")
                        + (r.connectionsExisting > 0 ? String(localized: ", \(r.connectionsExisting) bereits vorhanden") : "")
                        + (r.pinsStored > 0 ? String(localized: ", \(r.pinsStored) PINs im Tresor gespeichert.") : "."))
                }
                if r.banksSkippedUnsupported > 0 {
                    Text("\(r.banksSkippedUnsupported) Bankzugänge werden von GiroCrab noch nicht unterstützt und wurden übersprungen.")
                }
                if r.pendingSkipped > 0 {
                    Text("\(r.pendingSkipped) vorgemerkte Buchungen wurden nicht übernommen.")
                }
            }
            .font(.caption).foregroundStyle(Theme.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }

    private var footer: some View {
        HStack {
            Spacer()
            if result != nil {
                Button("Fertig") { dismiss() }.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            } else if importing {
                // closing the dialog would leave the import running unseen: cancel it instead
                Button(cancelling ? "Wird abgebrochen …" : "Import abbrechen") {
                    cancelling = true
                    relay?.cancel()
                }
                .keyboardShortcut(.cancelAction)
                .disabled(cancelling)
            } else {
                Button("Abbrechen") { password = ""; dismiss() }.keyboardShortcut(.cancelAction)
                Button(importing ? "Importiere …" : "Importieren") { runImport() }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
                    .disabled(preview == nil || importing || !choices.values.contains(where: \.selected))
            }
        }
    }

    /// Enable Banking connections the import set up: what is left to do –
    /// link the accounts to the user's application, then the bank consent.
    private func psd2Card(_ list: [MoneyMoneyDbPsd2Connection]) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Noch zu tun: Freigabe über Enable Banking", systemImage: "link.circle.fill")
                .font(.callout.weight(.medium)).foregroundStyle(Theme.accent)
            Group {
                if enableBankingReady {
                    Text("Starten Sie die Freigabe bei der Bank. Im eingeschränkten Modus sieht Ihre Enable-Banking-Anwendung nur verknüpfte Konten – falls diese Konten noch nicht verknüpft sind, holen Sie das auf enablebanking.com nach.")
                } else {
                    Text("Für diese Banken braucht GiroCrab eine kostenlose Enable-Banking-Anwendung. Sie wird bei der ersten Freigabe eingerichtet: Verknüpfen Sie beim Aktivieren der Anwendung alle unten aufgeführten Konten auf einmal – GiroCrab zeigt sie dabei noch einmal an. Danach starten Sie die Freigabe je Bank.")
                }
            }
            .font(.caption).foregroundStyle(Theme.secondary)
            .fixedSize(horizontal: false, vertical: true)
            Link("Anwendungen auf enablebanking.com öffnen", destination: URL(string: "https://enablebanking.com/cp/applications")!)
                .font(.caption)
            ForEach(list, id: \.connectionId) { c in
                Divider()
                HStack(alignment: .top, spacing: 12) {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(c.bankName).font(.callout.weight(.medium))
                        ForEach(Array(c.accounts.enumerated()), id: \.offset) { _, a in
                            Text([a.name, a.iban.isEmpty ? (a.accountNumber.isEmpty ? nil : String(localized: "Konto \(a.accountNumber)")) : formatIBAN(a.iban)]
                                .compactMap { $0 }.joined(separator: " · "))
                                .font(.caption.monospacedDigit()).foregroundStyle(Theme.secondary)
                                .textSelection(.enabled)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Button("Freigabe starten") { startConsent(c.connectionId) }
                        .controlSize(.small)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }

    /// Closes the import and starts the bank consent of that connection.
    private func startConsent(_ connectionID: Int64) {
        guard let conn = model.connections.first(where: { $0.id == connectionID }) else { return }
        dismiss()
        model.requestSync(conn)
    }

    /// "Abgleich mit MoneyMoney": every imported account read back from the
    /// vault. All fine → one line (details on demand); otherwise the
    /// accounts that differ, with the numbers.
    private func checkCard(_ checks: [MoneyMoneyDbAccountCheck]) -> some View {
        let problems = checks.filter { !$0.ok }
        return VStack(alignment: .leading, spacing: 8) {
            if problems.isEmpty {
                Label(checks.count == 1
                      ? String(localized: "Das Konto stimmt mit MoneyMoney überein.")
                      : String(localized: "Alle \(checks.count) Konten stimmen mit MoneyMoney überein."),
                      systemImage: "checkmark.circle.fill")
                    .font(.callout.weight(.medium)).foregroundStyle(Theme.inflow)
                DisclosureGroup("Details") { checkRows(checks) }
                    .font(.caption)
            } else {
                Label(String(localized: "\(problems.count) von \(checks.count) Konten weichen von MoneyMoney ab"),
                      systemImage: "exclamationmark.triangle.fill")
                    .font(.callout.weight(.medium)).foregroundStyle(Theme.warning)
                Text("Bitte prüfen Sie diese Konten. Ein erneuter Import ergänzt fehlende Buchungen, ohne Duplikate anzulegen.")
                    .font(.caption).foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                checkRows(problems)
                if problems.count < checks.count {
                    DisclosureGroup("Alle Konten") { checkRows(checks) }
                        .font(.caption)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
    }

    private func checkRows(_ checks: [MoneyMoneyDbAccountCheck]) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 6) {
            GridRow {
                Text("")
                Text("Konto")
                Text("Buchungen").gridColumnAlignment(.trailing)
                Text("Saldo MoneyMoney").gridColumnAlignment(.trailing)
                Text("Saldo GiroCrab").gridColumnAlignment(.trailing)
            }
            .font(.caption2).foregroundStyle(Theme.secondary)
            ForEach(checks, id: \.mmId) { c in
                GridRow {
                    Image(systemName: c.ok ? "checkmark.circle" : "exclamationmark.triangle")
                        .foregroundStyle(c.ok ? Theme.inflow : Theme.warning)
                        .accessibilityLabel(c.ok ? Text("Stimmt") : Text("Weicht ab"))
                    VStack(alignment: .leading, spacing: 1) {
                        Text(c.name).lineLimit(1)
                        if let note = checkNote(c) {
                            Text(note).font(.caption2).foregroundStyle(Theme.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    Text("\(c.foundBookings + c.deletedBookings) / \(c.mmBookings)")
                        .monospacedDigit()
                        .foregroundStyle(c.foundBookings + c.deletedBookings == c.mmBookings ? Theme.text : Theme.warning)
                    Text(Money.format(c.mmBalanceCents, currency: c.currency)).monospacedDigit()
                    Text(Money.format(c.girocrabBalanceCents, currency: c.currency)).monospacedDigit()
                        .foregroundStyle(c.balanceFromBank || c.girocrabBalanceCents == c.mmBalanceCents ? Theme.text : Theme.warning)
                }
                .font(.caption)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    /// Short explanation under the account name, only where something is special.
    private func checkNote(_ c: MoneyMoneyDbAccountCheck) -> String? {
        var parts: [String] = []
        let missing = Int(c.mmBookings) - Int(c.foundBookings + c.deletedBookings)
        if missing > 0 { parts.append(String(localized: "\(missing) Buchungen fehlen")) }
        if c.deletedBookings > 0 { parts.append(String(localized: "\(c.deletedBookings) in GiroCrab gelöscht")) }
        if c.otherBookings > 0 { parts.append(String(localized: "\(c.otherBookings) weitere von der Bank")) }
        if c.balanceFromBank { parts.append(String(localized: "Saldo von der Bank")) }
        if !c.mmBalanceShown { parts.append(String(localized: "MoneyMoney-Saldo berechnet")) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private func stat(_ value: String, _ label: LocalizedStringKey) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(value).font(.title3.weight(.semibold).monospacedDigit()).foregroundStyle(Theme.text)
            Text(label).font(.caption2).foregroundStyle(Theme.secondary)
        }
    }

    // MARK: - Helpers

    private func binding(_ a: MoneyMoneyDbAccount) -> Binding<Choice> {
        Binding(
            get: { choices[a.mmId] ?? Choice(selected: true, kind: a.suggestedKind, target: a.existingAccountId) },
            set: { choices[a.mmId] = $0 }
        )
    }

    private func accountSubtitle(_ a: MoneyMoneyDbAccount) -> String {
        var parts: [String] = []
        if !a.bankName.isEmpty { parts.append(a.bankName) }
        if !a.iban.isEmpty { parts.append(TransferSheetView.groupedIBAN(a.iban)) } else if !a.accountNumber.isEmpty { parts.append(String(localized: "Konto \(a.accountNumber)")) }
        if a.currency != "EUR" { parts.append(a.currency) }
        if !a.groupName.isEmpty { parts.append(String(localized: "Gruppe „\(a.groupName)“")) }
        return parts.joined(separator: " · ")
    }

    private func bookingSummary(_ a: MoneyMoneyDbAccount) -> String {
        let n = a.bookingCount.formatted()
        var s = String(localized: "\(n) Buchungen")
        if let f = a.firstDate, let l = a.lastDate { s += " · \(DateText.medium(f)) – \(DateText.medium(l))" }
        if a.pendingCount > 0 { s += " · " + String(localized: "\(a.pendingCount) vorgemerkt") }
        if a.holdingCount > 0 { s += " · " + String(localized: "\(a.holdingCount) Depotpositionen") }
        if a.statementCount > 0 { s += " · " + String(localized: "\(a.statementCount) Dokumente") }
        return s
    }

    /// Enable Banking bank for a MoneyMoney bank without own connector: the
    /// catalogue suggestion, else a match in the live list (when loaded).
    private func psd2Bank(_ b: MoneyMoneyDbBank) -> Psd2Bank? {
        guard b.connector == nil else { return nil }
        if let s = b.psd2Bank { return s }
        guard !model.livePsd2Banks.isEmpty else { return nil }
        return matchPsd2BankByName(bankName: b.name, candidates: model.livePsd2Banks)
    }

    /// The user's own Enable Banking application is set up.
    private var enableBankingReady: Bool {
        model.settingEntry("enablebanking.application_id")?.hasValue == true
            && model.settingEntry("enablebanking.private_key_pem")?.hasValue == true
    }

    private func bankStatus(_ b: MoneyMoneyDbBank) -> String {
        if b.connector == nil, psd2Bank(b) != nil {
            return createConnections ? String(localized: "wird angelegt (PSD2)") : String(localized: "wird nicht angelegt")
        }
        guard b.connector != nil else { return String(localized: "nicht unterstützt") }
        if b.existingConnectionId != nil { return String(localized: "bereits vorhanden") }
        return createConnections ? String(localized: "wird angelegt") : String(localized: "wird nicht angelegt")
    }

    private func apply(_ p: MoneyMoneyDbPreview) {
        preview = p
        if p.banks.contains(where: { $0.connector == nil }) { model.loadLivePsd2Banks() }
        var c: [Int64: Choice] = [:]
        for a in p.accounts {
            c[a.mmId] = choices[a.mmId] ?? Choice(selected: true, kind: a.suggestedKind, target: a.existingAccountId)
        }
        choices = c
    }

    // MARK: - Actions

    private func pickFile() {
        #if os(macOS)
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        // the folder brings the statement PDFs along; a single file is a backup
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = String(localized: "MoneyMoney-Ordner auswählen (mit Kontoauszügen) – oder eine einzelne Datenbank, z. B. ein Backup (MoneyMoney.sqlite)")
        panel.prompt = String(localized: "Auswählen")
        panel.treatsFilePackagesAsDirectories = true
        // opens inside MoneyMoney's folder: "Auswählen" right away takes the folder
        let dir = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Containers/com.moneymoney-app.retail/Data/Library/Application Support/MoneyMoney")
        if FileManager.default.fileExists(atPath: dir.path) { panel.directoryURL = dir }
        if panel.runModal() == .OK, let url = panel.url {
            fileURL = url
            statementsURL = nil
            preview = nil
            choices = [:]
            error = nil
        }
        #endif
    }

    /// Folder with separately saved statement PDFs (for a backup database).
    private func pickStatementsFolder() {
        #if os(macOS)
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.message = String(localized: "Ordner mit den gesicherten Kontoauszügen (PDF) auswählen – Unterordner werden mit durchsucht")
        panel.prompt = String(localized: "Auswählen")
        if panel.runModal() == .OK, let url = panel.url {
            statementsURL = url
            loadPreview()
        }
        #endif
    }

    private func loadPreview() {
        guard let vault = model.vault, let url = fileURL else { return }
        let statements = statementsURL
        let pw = password
        loading = true; error = nil
        Task {
            let result = await Task.detached { () -> Result<MoneyMoneyDbPreview, Error> in
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                let ok2 = statements?.startAccessingSecurityScopedResource() ?? false
                defer { if ok2 { statements?.stopAccessingSecurityScopedResource() } }
                return Result { try vault.previewMoneymoneyDatabase(path: url.path, password: pw, statementsDir: statements?.path) }
            }.value
            loading = false
            switch result {
            case .success(let p): apply(p)
            case .failure(let e):
                preview = nil
                if case CoreError.Authentication(let m) = e { error = m } else { error = describe(e) }
            }
        }
    }

    private func runImport() {
        guard let vault = model.vault, let url = fileURL, let preview else { return }
        if model.demoDbPreview != nil {
            // demo: nothing to read – show what the summary looks like
            result = MoneyMoneyDbImportResult(accountsCreated: 5, accountsMerged: 1, groupsCreated: 2, bookingsNew: preview.totalBookings, bookingsDuplicates: 0, bookingsEnriched: 0, pendingSkipped: preview.pendingBookings, notesImported: preview.notesCount, categoriesCreated: UInt32(preview.categoryPaths.count), connectionsCreated: 4, connectionsExisting: 0, pinsStored: 0, banksSkippedUnsupported: 1, documentsImported: importStatements ? preview.statementsFound : 0, documentsExisting: 0, documentsMissing: 0, documentsMismatched: 0, documentsTooLarge: 0, documentsUnlisted: 0, holdingsImported: 0, rulesCreated: 0, rulesDisabled: 0,
                accountChecks: preview.accounts.map { a in
                    MoneyMoneyDbAccountCheck(mmId: a.mmId, accountId: 0, name: a.name, currency: a.currency, mmBookings: a.bookingCount, foundBookings: a.bookingCount, deletedBookings: 0, otherBookings: 0, mmBalanceCents: a.computedBalanceCents, mmBalanceShown: true, girocrabBalanceCents: a.computedBalanceCents, balanceFromBank: false, ok: true)
                },
                psd2Connections: preview.banks.filter { $0.connector == nil && $0.psd2Bank != nil }.map { b in
                    MoneyMoneyDbPsd2Connection(connectionId: -b.mmId, bankName: b.name, created: true,
                        accounts: preview.accounts.filter { $0.bankName == b.name }.map {
                            MoneyMoneyDbPsd2Account(name: $0.name, iban: $0.iban, accountNumber: $0.accountNumber)
                        })
                })
            password = ""
            return
        }
        let options = MoneyMoneyDbImportOptions(
            accounts: preview.accounts.map { a in
                let c = choices[a.mmId] ?? Choice(selected: true, kind: a.suggestedKind, target: a.existingAccountId)
                return MoneyMoneyDbAccountChoice(mmId: a.mmId, import: c.selected, kind: c.kind, targetAccountId: c.target)
            },
            importCategories: importCategories,
            importNotes: importNotes,
            createConnections: createConnections,
            importPins: createConnections && importPins,
            importGroups: importGroups,
            importStatements: importStatements && preview.statementsFound > 0,
            statementsDir: statementsURL?.path,
            psd2Banks: preview.banks.compactMap { b in
                psd2Bank(b).map { MoneyMoneyDbPsd2Choice(mmBankId: b.mmId, aspsp: $0.aspsp, country: $0.country) }
            })
        let statements = statementsURL
        let pw = password
        progress.value = MoneyMoneyDbImportProgress(fraction: 0, accountIndex: 0, accountCount: 0, accountName: "", bookingsDone: 0, bookingsTotal: 0, documentsDone: 0, documentsTotal: 0)
        let observer = ImportProgressRelay(state: progress)
        relay = observer
        cancelling = false
        importing = true; error = nil
        Task {
            let result = await Task.detached { () -> Result<MoneyMoneyDbImportResult, Error> in
                let ok = url.startAccessingSecurityScopedResource()
                defer { if ok { url.stopAccessingSecurityScopedResource() } }
                let ok2 = statements?.startAccessingSecurityScopedResource() ?? false
                defer { if ok2 { statements?.stopAccessingSecurityScopedResource() } }
                return Result { try vault.importMoneymoneyDatabaseWithProgress(path: url.path, password: pw, options: options, observer: observer) }
            }.value
            importing = false
            relay = nil
            // after a cancel the form comes back as it was, ready for another try
            let wasCancelled = cancelling
            if !wasCancelled { password = "" }
            cancelling = false
            switch result {
            case .success(let r):
                self.result = r
                model.finishMoneyMoneyDbImport(r)
            case .failure where wasCancelled:
                error = String(localized: "Import abgebrochen – es wurde nichts übernommen.")
            case .failure(let e):
                if case CoreError.Authentication(let m) = e { error = m } else { error = describe(e) }
            }
        }
    }
}

/// The running import's progress, for the view (main actor only).
@MainActor @Observable
final class ImportProgressState {
    var value = MoneyMoneyDbImportProgress(fraction: 0, accountIndex: 0, accountCount: 0, accountName: "", bookingsDone: 0, bookingsTotal: 0, documentsDone: 0, documentsTotal: 0)
}

/// Receives the core's progress on the importing thread, passes it on to the
/// view (at most ~20 times a second) and tells the core when to cancel.
final class ImportProgressRelay: MoneyMoneyImportObserver, @unchecked Sendable {
    private let lock = NSLock()
    private var cancelled = false
    private var lastSent = Date.distantPast
    private var lastAccount: UInt32 = .max
    private let state: ImportProgressState

    init(state: ImportProgressState) { self.state = state }

    func cancel() {
        lock.lock(); cancelled = true; lock.unlock()
    }

    func progress(progress: MoneyMoneyDbImportProgress) -> Bool {
        lock.lock()
        let now = Date()
        let send = progress.accountIndex != lastAccount || progress.fraction >= 1 || now.timeIntervalSince(lastSent) > 0.05
        if send { lastSent = now; lastAccount = progress.accountIndex }
        let go = !cancelled
        lock.unlock()
        if send {
            let state = state
            Task { @MainActor in state.value = progress }
        }
        return go
    }
}

extension AccountKind {
    static let importChoices: [AccountKind] = [.checking, .savings, .creditCard, .paymentService, .brokerage, .cash, .loan]

    var label: String {
        switch self {
        case .checking: return String(localized: "Girokonto")
        case .savings: return String(localized: "Tagesgeld / Sparen")
        case .creditCard: return String(localized: "Kreditkarte")
        case .paymentService: return String(localized: "Zahlungsdienst")
        case .brokerage: return String(localized: "Depot")
        case .cash: return String(localized: "Bargeld")
        case .loan: return String(localized: "Darlehen")
        }
    }
}

extension AppModel {
    func finishMoneyMoneyDbImport(_ r: MoneyMoneyDbImportResult) {
        reloadAll()
        if !r.psd2Connections.isEmpty { psd2LinkHints = r.psd2Connections }
        toast = String(localized: "MoneyMoney: \(r.bookingsNew) Buchungen in \(r.accountsCreated + r.accountsMerged) Konten importiert")
            + (r.bookingsDuplicates > 0 ? String(localized: ", \(r.bookingsDuplicates) bereits vorhanden") : "")
    }
}

extension AppModel {
    /// Invented MoneyMoney database (public example IBANs, fictional names) for
    /// `--demo-sheet moneymoney-db` screenshots.
    static func demoMoneyMoneyDbPreview(existingGiro: Account?) -> MoneyMoneyDbPreview {
        func acc(_ id: Int64, _ name: String, _ bank: String, _ code: String, iban: String = "", number: String = "", currency: String = "EUR",
                 kind: AccountKind, portfolio: Bool = false, existing: Account? = nil, count: UInt32, pending: UInt32 = 0,
                 from: String?, to: String?, balance: Int64, group: String = "Privat") -> MoneyMoneyDbAccount {
            MoneyMoneyDbAccount(mmId: id, name: name, iban: iban, accountNumber: number, currency: currency, bankCode: code, bankName: bank,
                                owner: "Max Mustermann", groupName: group, mmBankId: nil, isPortfolio: portfolio, suggestedKind: kind,
                                existingAccountId: existing?.id, existingAccountName: existing?.name, bookingCount: count, pendingCount: pending, holdingCount: portfolio ? 3 : 0,
                                notesCount: 0, firstDate: from, lastDate: to, statementCount: kind == .checking ? 48 : 0, computedBalanceCents: balance,
                                balanceSource: "Anfangssaldo laut MoneyMoney-Saldenverlauf + Umsätze")
        }
        let accounts = [
            acc(1, "Girokonto", "comdirect", "20041111", iban: "DE89370400440532013000", kind: .checking, existing: existingGiro,
                count: 4812, pending: 3, from: "2009-04-01", to: "2026-11-09", balance: 3_245_18),
            acc(2, "Visa-Karte", "comdirect", "20041111", number: "4871 •••• 1234", kind: .creditCard, count: 1204, pending: 2,
                from: "2011-02-14", to: "2026-11-08", balance: -412_90),
            acc(3, "Tagesgeld", "1822direkt", "50050201", iban: "DE02120300000000202051", kind: .savings, count: 96,
                from: "2014-06-30", to: "2026-10-31", balance: 12_500_00, group: "Sparen"),
            acc(4, "Depot", "comdirect", "20041111", number: "123456789", kind: .brokerage, portfolio: true, count: 0,
                from: nil, to: nil, balance: 0, group: "Sparen"),
            acc(5, "PayPal", "PayPal", "", number: "max@example.com", kind: .paymentService, count: 640,
                from: "2013-08-02", to: "2026-11-05", balance: 23_45),
            acc(6, "Reisekasse USD", "Musterbank", "12345678", number: "1000200", currency: "USD", kind: .checking, count: 38,
                from: "2019-07-10", to: "2025-09-22", balance: 310_75, group: ""),
            acc(7, "C24 Smartkonto", "C24 Bank", "50024024", iban: "DE02500105170137075030", kind: .checking, count: 57,
                from: "2024-03-01", to: "2026-11-07", balance: 1_840_12),
        ]
        let banks = [
            MoneyMoneyDbBank(mmId: 1, bankCode: "20041111", name: "comdirect", loginName: "12345678", customerId: "12345678",
                             connector: .comdirect, hasSavedPin: true, existingConnectionId: nil, accountCount: 3, psd2Bank: nil),
            MoneyMoneyDbBank(mmId: 2, bankCode: "50050201", name: "1822direkt", loginName: "max.mustermann", customerId: "",
                             connector: .direkt1822, hasSavedPin: true, existingConnectionId: nil, accountCount: 1, psd2Bank: nil),
            MoneyMoneyDbBank(mmId: 3, bankCode: "", name: "PayPal", loginName: "max@example.com", customerId: "",
                             connector: .payPal, hasSavedPin: false, existingConnectionId: nil, accountCount: 1, psd2Bank: nil),
            MoneyMoneyDbBank(mmId: 4, bankCode: "12345678", name: "Musterbank", loginName: "mustermann", customerId: "",
                             connector: nil, hasSavedPin: false, existingConnectionId: nil, accountCount: 1, psd2Bank: nil),
            // no own connector, but reachable through Enable Banking (PSD2)
            MoneyMoneyDbBank(mmId: 5, bankCode: "50024024", name: "C24 Bank", loginName: "max.mustermann", customerId: "",
                             connector: nil, hasSavedPin: false, existingConnectionId: nil, accountCount: 1,
                             psd2Bank: psd2Catalog().first { $0.aspsp == "C24" }),
        ]
        let categories = ["Gehalt", "Haushalt › Lebensmittel", "Haushalt › Drogerie", "Wohnen › Miete", "Wohnen › Nebenkosten › Strom",
                          "Mobilität › Tanken", "Mobilität › ÖPNV", "Freizeit › Restaurants", "Freizeit › Sport", "Versicherungen",
                          "Steuern", "Reisen", "Abos", "Gesundheit", "Geschenke", "Sparen"]
        return MoneyMoneyDbPreview(accounts: accounts, banks: banks, categoryPaths: categories,
                                   totalBookings: accounts.reduce(0) { $0 + $1.bookingCount },
                                   pendingBookings: 5, notesCount: 312, splitParts: 14,
                                   dateRangeStart: "2009-04-01", dateRangeEnd: "2026-11-09", replacesStarterCategories: true,
                                   databasePath: "MoneyMoney.sqlite",
                                   statementCount: accounts.reduce(0) { $0 + $1.statementCount }, statementsDir: "Statements",
                                   statementsFound: accounts.reduce(0) { $0 + $1.statementCount }, statementsBytes: 38_400_000)
    }
}

#endif
