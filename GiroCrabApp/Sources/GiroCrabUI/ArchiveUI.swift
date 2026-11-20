import SwiftUI
import GiroCrabCore

// Document archive: receipts that live in the user's own archive and are only
// linked from a booking. Everything here is opt-in: without a connected
// archive none of these views appears and nothing is requested.

extension ArchiveDocument: @retroactive Identifiable {}

// MARK: - Preview of a linked archive document

/// The file of a linked archive document, fetched when it is shown and kept
/// in memory for this session only.
struct ArchiveDocumentPreview: View {
    @Environment(AppModel.self) private var model
    let doc: DocumentInfo

    var body: some View {
        Group {
            switch model.archiveFiles[doc.id] {
            case .loaded(let data, let mimeType, _):
                if mimeType == "application/pdf" || data.starts(with: [0x25, 0x50, 0x44, 0x46]) {
                    PDFPreview(data: data)
                } else if let image = AppModel.image(from: data) {
                    image.resizable().scaledToFit()
                } else {
                    ContentUnavailableView("Keine Vorschau", systemImage: "doc")
                }
            case .failed(let problem):
                ContentUnavailableView {
                    Label("Beleg nicht verfügbar", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(problem)
                } actions: {
                    Button("Erneut versuchen") { model.reloadArchiveFile(doc) }
                    Button("Im Browser öffnen") { model.openDocumentLink(doc) }.disabled(doc.webURL == nil)
                }
            case .loading, .none:
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Wird aus dem Dokumentenarchiv geladen …").appFont(.caption).foregroundStyle(Theme.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        // receipts are full of names, IBANs and amounts (#301)
        .sensitive([.personal, .amounts])
        // again after a change of the connection: what was fetched went with the old one
        .task(id: "\(doc.id)-\(model.archiveGeneration)") { model.loadArchiveFile(doc) }
    }
}

// MARK: - Assigning documents of the archive to many bookings at once

/// Bookings without a receipt in the period shown, each with the document of
/// the archive that fits it. The user ticks what is right and links it in one
/// go. What is not clear-cut is marked and left unticked.
struct ArchiveMatchSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @State private var found: ArchiveMatches?
    /// Booking ids of the matches to link.
    @State private var chosen: Set<Int64> = []
    @State private var loading = true
    @State private var linking = false
    @State private var problem: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Belege aus dem Dokumentenarchiv zuordnen").appFont(.title3, weight: .semibold)
                    Text("Buchungen ohne Beleg · \(model.periodTitle) · alle Konten")
                        .appFont(.caption).foregroundStyle(Theme.secondary)
                }
                Spacer()
                Button("Abbrechen") { dismiss() }.keyboardShortcut(.cancelAction)
            }
            if let problem {
                Text(problem).appFont(.caption).foregroundStyle(Theme.warning).fixedSize(horizontal: false, vertical: true)
            }
            if loading {
                VStack(spacing: 8) {
                    ProgressView()
                    Text("Das Archiv wird durchgesehen …").appFont(.caption).foregroundStyle(Theme.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let found, !found.matches.isEmpty {
                List(found.matches, id: \.bookingId) { match in
                    row(match)
                }
                .listStyle(.inset)
                Text(summary(found)).appFont(.caption).foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Button(chosen.count == found.matches.count ? "Keine auswählen" : "Alle auswählen") {
                        chosen = chosen.count == found.matches.count ? [] : Set(found.matches.map(\.bookingId))
                    }
                    Spacer()
                    if linking { ProgressView().controlSize(.small) }
                    Button("\(chosen.count) Belege verlinken") { Task { await link(found) } }
                        .keyboardShortcut(.defaultAction)
                        .disabled(chosen.isEmpty || linking)
                }
            } else {
                ContentUnavailableView {
                    Label("Kein passender Beleg gefunden", systemImage: "doc.text.magnifyingglass")
                } description: {
                    if let found { Text(summary(found)) }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(16)
        #if os(macOS)
        .desktopFrame(minWidth: 680, minHeight: 560)
        #endif
        .task { await load() }
        // matches belong to the connection they were asked of
        .onChange(of: model.archiveGeneration) { _, _ in dismiss() }
    }

    private func row(_ match: ArchiveMatch) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Toggle(isOn: Binding(
                get: { chosen.contains(match.bookingId) },
                set: { on in if on { chosen.insert(match.bookingId) } else { chosen.remove(match.bookingId) } }
            )) {
                VStack(alignment: .leading, spacing: 2) {
                    // these sheets are part of a bug report's picture: merchant,
                    // amount and what the document is called are covered there
                    Text(match.booking).appFont(.callout, weight: .medium).lineLimit(1)
                        .sensitive([.personal, .amounts])
                    Label(documentLine(match.document), systemImage: "doc.text")
                        .appFont(.caption).foregroundStyle(Theme.secondary).lineLimit(2)
                        .sensitive()
                    if match.ambiguous {
                        Label("Passt auf mehrere Buchungen oder Belege – bitte prüfen", systemImage: "exclamationmark.triangle")
                            .appFont(.caption).foregroundStyle(Theme.warning)
                    } else if !Self.isClear(match) {
                        Label("Der Beleg nennt den Händler nicht – bitte prüfen", systemImage: "exclamationmark.triangle")
                            .appFont(.caption).foregroundStyle(Theme.warning)
                    }
                }
                // bookings and receipts name people and amounts (#301)
                .sensitive([.personal, .amounts])
            }
            #if os(macOS)
            .toggleStyle(.checkbox)
            #endif
            Spacer(minLength: 8)
            if let url = documentURL(match.document) {
                Button("Ansehen") { openURL(url) }
                    .help("Öffnet das Dokument im Archiv im Browser")
            }
        }
        .padding(.vertical, 2)
    }

    /// A match to tick without asking: no rival, and besides the amount the
    /// document names the booking's merchant. The same amount on a date
    /// nearby can be a coincidence: a receipt names many amounts.
    static func isClear(_ match: ArchiveMatch) -> Bool {
        !match.ambiguous && match.merchantFits
    }

    private func documentLine(_ document: ArchiveDocument) -> String {
        var parts = [document.title]
        if !document.created.isEmpty { parts.append(DateText.medium(document.created)) }
        if !document.correspondent.isEmpty { parts.append(document.correspondent) }
        return parts.joined(separator: " · ")
    }

    /// The document's page in the archive, as the archive's kind names it.
    private func documentURL(_ document: ArchiveDocument) -> URL? {
        document.url.isEmpty ? nil : URL(string: document.url)
    }

    private func summary(_ found: ArchiveMatches) -> String {
        var text = String(localized: "\(Int(found.openBookings)) Buchungen ohne Beleg, \(Int(found.documents)) Dokumente des Archivs geprüft. Zugeordnet wird nur, wo der Betrag im Dokument steht.")
        if found.more {
            text += " " + String(localized: "Es gab mehr, als ein Durchgang prüft: Ein kürzerer Zeitraum findet den Rest.")
        }
        return text
    }

    private func load() async {
        loading = true
        defer { loading = false }
        let result = await model.archiveMatches()
        problem = result.problem
        found = result.found
        // what is clear-cut is ticked, the rest is the user's call
        chosen = Set((result.found?.matches ?? []).filter(Self.isClear).map(\.bookingId))
    }

    private func link(_ found: ArchiveMatches) async {
        linking = true
        defer { linking = false }
        let linked = await model.attachArchiveMatches(found.matches.filter { chosen.contains($0.bookingId) })
        if linked > 0 { model.toast = String(localized: "\(linked) Belege aus dem Archiv verlinkt") }
        dismiss()
    }
}

// MARK: - Moving a stored file into the archive

/// Menu items of a stored file while an archive is connected: move it there,
/// try again after a failure, or keep it in the vault after all. A document
/// a bank delivered moves like a receipt of the user's own: its reference
/// keeps the origin the bank fetch knows it by.
struct ArchiveMoveMenuItems: View {
    @Environment(AppModel.self) private var model
    let doc: DocumentInfo

    var body: some View {
        if model.archiveConnected, !doc.isLink {
            if let move = model.archiveMoves[doc.id] {
                if move.hasFailed {
                    Button("Erneut ins Archiv verschieben") { model.moveToArchive(doc) }
                }
                Button("Im Tresor behalten") { model.keepInVault(doc) }
            } else {
                Button("Ins Archiv verschieben") { model.moveToArchive(doc) }
            }
            Divider()
        }
    }
}

// MARK: - Search and suggestions

/// One document of the archive in a result list.
private struct ArchiveDocumentRow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.textScale) private var textScale
    let document: ArchiveDocument
    var score: Int32?
    let isLinked: Bool
    let link: () -> Void
    @State private var thumbnail: Image?

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Group {
                if let thumbnail {
                    thumbnail.resizable().scaledToFill()
                } else {
                    Image(systemName: "doc.text").foregroundStyle(Theme.secondary)
                }
            }
            .frame(width: 44, height: 58)
            .background(Theme.canvas)
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Theme.border))
            // a picture of the document itself
            .sensitive([.personal, .amounts])
            VStack(alignment: .leading, spacing: 2) {
                Text(document.title).appFont(.callout, weight: .medium).lineLimit(2).sensitive()
                Text(subtitle).appFont(.caption).foregroundStyle(Theme.secondary).lineLimit(1).sensitive()
                if !document.excerpt.isEmpty {
                    // the document's own words: names and amounts
                    Text(document.excerpt).appFont(.caption2).foregroundStyle(Theme.secondary).lineLimit(2)
                        .sensitive([.personal, .amounts])
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if isLinked {
                Label("Verlinkt", systemImage: "checkmark.circle.fill")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
            } else {
                Button("Verlinken", action: link)
            }
        }
        .padding(.vertical, 2)
        .task(id: document.id) {
            if let data = await model.archiveThumbnail(document.id) { thumbnail = AppModel.image(from: data) }
        }
    }

    private var subtitle: String {
        var parts = [DateText.medium(document.created)]
        if !document.correspondent.isEmpty { parts.append(document.correspondent) }
        if !document.documentType.isEmpty { parts.append(document.documentType) }
        if score != nil { parts.append(String(localized: "passt zur Buchung")) }
        return parts.joined(separator: " · ")
    }
}

/// "Im Archiv suchen …": suggestions for the booking first, then a full-text
/// search of the archive. A sheet, so never part of a bug report screenshot.
struct ArchiveSearchSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let booking: Booking
    @State private var query = ""
    @State private var nearBooking = true
    @State private var suggestions: [ArchiveSuggestion] = []
    @State private var results: [ArchiveDocument] = []
    @State private var searching = false
    @State private var problem: String?
    @State private var linked: Set<Int64> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Beleg im Dokumentenarchiv suchen").appFont(.title3, weight: .semibold)
                    Text("\(booking.merchantName) · \(DateText.medium(booking.bookingDate))")
                        .appFont(.caption).foregroundStyle(Theme.secondary).sensitive()
                }
                Spacer()
                Button("Fertig") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            HStack {
                TextField("Suchbegriff", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .autocorrectionDisabled()
                    .onSubmit { Task { await search() } }
                Button("Suchen") { Task { await search() } }.disabled(searching)
            }
            Toggle("Nur Dokumente rund um das Buchungsdatum", isOn: $nearBooking)
                .onChange(of: nearBooking) { _, _ in Task { await search() } }
            if let problem {
                Text(problem).appFont(.caption).foregroundStyle(Theme.warning).fixedSize(horizontal: false, vertical: true)
            }
            List {
                if !suggestions.isEmpty {
                    Section("Vorschläge") {
                        ForEach(suggestions, id: \.document.id) { s in
                            ArchiveDocumentRow(document: s.document, score: s.score, isLinked: isLinked(s.document)) { link(s.document) }
                        }
                    }
                }
                Section(suggestions.isEmpty ? "" : String(localized: "Suchergebnisse")) {
                    ForEach(results) { d in
                        ArchiveDocumentRow(document: d, isLinked: isLinked(d)) { link(d) }
                    }
                    if searching {
                        HStack { Spacer(); ProgressView(); Spacer() }
                    } else if results.isEmpty && problem == nil {
                        Text("Keine Dokumente gefunden.").appFont(.callout).foregroundStyle(Theme.secondary)
                    }
                }
            }
            .listStyle(.inset)
        }
        .padding(16)
        #if os(macOS)
        .desktopFrame(minWidth: 620, minHeight: 540)
        #endif
        .task {
            query = booking.merchantName
            async let fitting = model.archiveSuggestions(for: booking, limit: 5)
            await search()
            suggestions = await fitting
        }
        // results belong to the connection they were asked of
        .onChange(of: model.archiveGeneration) { _, _ in dismiss() }
    }

    private func isLinked(_ d: ArchiveDocument) -> Bool {
        linked.contains(d.id) || model.documents(for: booking).contains { $0.archiveDoc == d.id }
    }

    private func link(_ d: ArchiveDocument) {
        problem = model.attachArchiveDocument(d, to: booking)
        if problem == nil { linked.insert(d.id) }
    }

    private func search() async {
        searching = true
        defer { searching = false }
        // a receipt is usually dated shortly before its booking
        let window = nearBooking ? Self.window(around: booking.bookingDate) : nil
        let found = await model.searchArchive(query, from: window?.from, to: window?.to)
        results = found.documents
        problem = found.problem
    }

    /// 60 days before to 14 days after the booking, as `YYYY-MM-DD`.
    static func window(around day: String) -> (from: String, to: String)? {
        let format = DateFormatter()
        format.calendar = Calendar(identifier: .gregorian)
        format.locale = Locale(identifier: "en_US_POSIX")
        format.timeZone = TimeZone(identifier: "UTC")
        format.dateFormat = "yyyy-MM-dd"
        guard let date = format.date(from: day) else { return nil }
        return (format.string(from: date.addingTimeInterval(-60 * 86_400)),
                format.string(from: date.addingTimeInterval(14 * 86_400)))
    }
}

// MARK: - Settings

/// Einstellungen › Dokumente › Dokumentenarchiv: address and token of the
/// user's own archive. Storing them sends nothing. Once connected, the
/// archive is asked when the user tests, searches, opens a linked document,
/// and (unasked) for suggestions when a booking without a receipt is shown.
struct ArchiveSettingsSection: View {
    @Environment(AppModel.self) private var model
    @State private var address = ""
    @State private var token = ""
    @State private var problem: String?
    @State private var testResult: String?
    @State private var testing = false
    /// The archives to choose from when it must be said which one the address is.
    @State private var choices: [ArchiveIdentity] = []
    @State private var askSameArchive = false
    @State private var askRemove = false
    @State private var askMoveAll = false

    var body: some View {
        let config = model.archiveConfig
        Section {
            TextField("Adresse", text: $address, prompt: Text(verbatim: "https://archiv.example"))
                .autocorrectionDisabled()
                #if os(iOS)
                .textInputAutocapitalization(.never)
                .keyboardType(.URL)
                #endif
            SecureField("Token", text: $token, prompt: Text(config?.hasToken == true ? "gespeichert" : "aus dem Profil im Archiv"))
            if config?.unencrypted == true || address.lowercased().hasPrefix("http://") {
                Label("Unverschlüsselte Verbindung: Token, Belege und Buchungsdaten sind im Netz mitlesbar. Nur im eigenen Netz oder über ein VPN verwenden.", systemImage: "exclamationmark.triangle")
                    .appFont(.caption).foregroundStyle(Theme.warning).fixedSize(horizontal: false, vertical: true)
            }
            if let problem {
                Text(problem).appFont(.caption).foregroundStyle(Theme.warning).fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Button(config == nil ? "Verbinden" : "Speichern") { save() }
                    .disabled(address.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (config == nil && token.isEmpty))
                Button("Verbindung testen") { Task { await test() } }
                    .disabled(config?.hasToken != true || testing)
                if testing { ProgressView().controlSize(.small) }
                Spacer()
                if config != nil {
                    Button("Verbindung entfernen …", role: .destructive) { askRemove = true }
                }
            }
            if let testResult {
                Text(testResult).appFont(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Text("Belege können in einem eigenen Dokumentenarchiv bleiben und werden in GiroCrab nur verlinkt. Unterstützt: \(archiveKinds().joined(separator: ", ")) und Archive mit derselben Schnittstelle. GiroCrab verbindet sich ausschließlich mit der hier eingetragenen Adresse und erst, nachdem sie eingerichtet ist: beim Suchen, beim Anzeigen verlinkter Belege und für Vorschläge zu Buchungen ohne Beleg. Adresse und Token bleiben auf diesem Gerät im verschlüsselten Tresor.")
                .appFont(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if model.archiveConnected {
                Toggle("Neue Belege ins Archiv verschieben", isOn: Binding(
                    get: { model.archiveUploadEnabled },
                    set: { model.setArchiveUpload($0) }
                ))
                Text("Eine Datei, die Sie in GiroCrab an eine Buchung hängen, wird ins Archiv hochgeladen und danach nur noch verlinkt. Bis das Archiv sie übernommen hat, bleibt sie im Tresor.")
                    .appFont(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Toggle("Dokumente der Banken ins Archiv verschieben", isOn: Binding(
                    get: { model.archiveBankEnabled },
                    set: { model.setArchiveBank($0) }
                ))
                Text("Kontoauszüge und Mitteilungen, die GiroCrab von nun an bei den Banken abruft, werden ins Archiv hochgeladen und danach nur noch verlinkt. Sie werden nicht erneut geladen, solange der Verweis besteht. Was schon im Tresor liegt, nimmt „Vorhandene Belege verschieben …“ mit.")
                    .appFont(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                Toggle("Bank, Dokumentart und Konto mitgeben", isOn: Binding(
                    get: { model.archiveLabelsEnabled },
                    set: { model.setArchiveLabels($0) }
                ))
                Text("Ein Dokument der Bank kommt im Archiv vorbelegt an: die Bank als Absender, die Art (zum Beispiel Kontoauszug) als Dokumenttyp und der Kontoname als Tag. Was im Archiv fehlt, legt GiroCrab dort ohne eigene Zuordnungsregel an; ohne das Recht dazu wird die Datei ohne diese Angaben hochgeladen. Eigene Belege bekommen keine: Deren Absender erkennt das Archiv selbst.")
                    .appFont(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                let waiting = Int(model.archiveSummary.waiting)
                let failed = Int(model.archiveSummary.failed)
                let inVault = Int(model.archiveSummary.inVault)
                HStack {
                    Button("Vorhandene Belege verschieben …") { askMoveAll = true }.disabled(inVault == 0)
                    Spacer()
                    Text(moveStatus(inVault: inVault, waiting: waiting, failed: failed))
                        .appFont(.caption).foregroundStyle(failed > 0 ? Theme.warning : .secondary)
                }
                Toggle("Buchungsdaten am Dokument vermerken", isOn: Binding(
                    get: { model.archiveFactsEnabled },
                    set: { model.setArchiveFacts($0) }
                ))
                Text("Schreibt Datum, Betrag, Gegenpartei und Konto der verknüpften Buchung in vier eigene Zusatzfelder des Dokuments im Archiv; ihre Namen enden auf „(GiroCrab)“, andere Felder bleiben unberührt. So finden Sie Belege dort nach Buchungen, zum Beispiel alle eines Monats. Der Token braucht dafür das Recht, Zusatzfelder anzulegen und Dokumente zu ändern. Beim Ausschalten bleiben die Angaben im Archiv stehen.")
                    .appFont(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                if model.archiveFactsEnabled, let status = factsStatus() {
                    HStack(alignment: .firstTextBaseline) {
                        Text(status).appFont(.caption).foregroundStyle(Theme.warning)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer()
                        Button("Erneut versuchen") { model.retryArchiveFacts() }
                    }
                }
            }
        } header: { Text("Dokumentenarchiv").appSectionHeader() }
        .confirmationDialog(model.archiveBankEnabled
                            ? Text("Belege und Bankdokumente ins Dokumentenarchiv verschieben?")
                            : Text("Eigene Belege ins Dokumentenarchiv verschieben?"),
                            isPresented: $askMoveAll) {
            Button("Verschieben") { Task { await model.queueAllReceiptsForArchive() } }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            if model.archiveBankEnabled {
                Text("Alle Dateien aus dem Tresor, die eigenen Belege und die Dokumente der Banken, werden ins Archiv hochgeladen und danach nur noch verlinkt. Jede Datei bleibt im Tresor, bis das Archiv sie übernommen hat.")
            } else {
                Text("Alle selbst hinzugefügten Belege aus dem Tresor werden ins Archiv hochgeladen und danach nur noch verlinkt. Dokumente der Banken bleiben im Tresor, solange sie nicht ins Archiv verschoben werden. Jede Datei bleibt im Tresor, bis das Archiv sie übernommen hat.")
            }
        }
        .onAppear { address = config?.url ?? "" }
        .onChange(of: config?.url) { _, new in address = new ?? "" }
        // lock, vault switch, change of the connection: a token typed for the
        // session before must not be stored in this one, nor its test result shown
        .onChange(of: model.archiveGeneration) { _, _ in
            token = ""; problem = nil; testResult = nil; testing = false
            askSameArchive = false; askRemove = false
            address = model.archiveConfig?.url ?? ""
        }
        .confirmationDialog("Welches Archiv ist das?", isPresented: $askSameArchive) {
            ForEach(choices, id: \.id) { archive in
                Button(archive.connected
                       ? String(localized: "Dasselbe Archiv wie bisher (\(Int(archive.documents)) Belege)")
                       : String(localized: "Das Archiv unter \(archive.host) (\(Int(archive.documents)) Belege)")) {
                    store(archiveId: archive.id)
                }
            }
            Button("Ein anderes, neues Archiv") { store(archiveId: nil) }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text("In diesem Tresor sind schon Belege aus einem Dokumentenarchiv verlinkt, von diesem oder einem anderen Gerät. Ist die Adresse dasselbe Archiv, bleiben sie dessen Dokumente. Bei einem neuen Archiv bleiben sie als einfache Links erhalten.")
        }
        .confirmationDialog("Verbindung zum Dokumentenarchiv entfernen?", isPresented: $askRemove) {
            Button("Entfernen", role: .destructive) {
                model.removeArchiveConnection()
                token = ""; problem = nil; testResult = nil
            }
            Button("Abbrechen", role: .cancel) {}
        } message: {
            Text("Adresse und Token werden gelöscht. Die verlinkten Belege bleiben als einfache Links erhalten, die Dokumente im Archiv werden nicht verändert.")
        }
    }

    /// What keeps booking data from reaching the archive (nil: nothing).
    /// The texts come from the core and name neither address nor token.
    private func factsStatus() -> String? {
        if let problem = model.archiveFactsProblem { return problem }
        let failed = Int(model.archiveFacts.failed)
        guard failed > 0 else { return nil }
        return String(localized: "\(failed) Dokumente nehmen die Buchungsdaten nicht an: \(model.archiveFacts.problem)")
    }

    private func moveStatus(inVault: Int, waiting: Int, failed: Int) -> String {
        var parts: [String] = []
        if inVault > 0 { parts.append(String(localized: "\(inVault) im Tresor")) }
        if waiting > 0 { parts.append(String(localized: "\(waiting) unterwegs")) }
        if failed > 0 { parts.append(String(localized: "\(failed) fehlgeschlagen")) }
        return parts.isEmpty ? String(localized: "Nichts mehr im Tresor zu verschieben.") : parts.joined(separator: " · ")
    }

    /// Which archive the address is must be the user's call whenever the
    /// vault holds something it could mean: a new address while documents of
    /// the connected archive are linked or files are on their way into it, or
    /// any connection while the vault has documents of an archive this device
    /// is not connected to (another device's, or from before a change).
    /// Everything is read fresh.
    private func save() {
        model.reloadArchiveConfig()
        let entered = address.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmed = entered.hasSuffix("/") ? String(entered.dropLast()) : entered
        let known = model.archiveIdentities()
        let current = known.first(where: \.connected)
        let others = known.filter { !$0.connected && $0.documents > 0 }
        let addressChanged = model.archiveConfig.map { $0.url != trimmed } ?? false
        if let current, !addressChanged {
            store(archiveId: current.id)
        } else if (current?.documents ?? 0) == 0 && others.isEmpty && model.archiveMoves.isEmpty {
            store(archiveId: current?.id)
        } else {
            choices = known.filter { $0.connected || $0.documents > 0 }
            askSameArchive = true
        }
    }

    private func store(archiveId: String?) {
        let (url, secret) = (address, token)
        testResult = nil
        // on success the generation changes and clears the fields
        if let refused = model.saveArchiveConnection(url: url, token: secret, archiveId: archiveId) {
            problem = refused
        } else {
            address = model.archiveConfig?.url ?? url
        }
    }

    private func test() async {
        testing = true
        defer { testing = false }
        let result = await model.testArchive()
        if let info = result.info {
            problem = nil
            testResult = String(localized: "Verbindung steht: \(Int(info.documentCount)) Dokumente sichtbar.")
        } else {
            testResult = nil
            problem = result.problem
        }
    }
}
