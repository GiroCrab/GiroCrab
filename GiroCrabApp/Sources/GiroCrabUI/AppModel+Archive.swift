import Foundation
import SwiftUI
import GiroCrabCore

extension AppModel {
    // MARK: - Document archive

    /// An archive is connected on this device (address and token stored).
    /// Without it nothing here sends a request.
    var archiveConnected: Bool { archiveConfig?.hasToken == true }

    func reloadArchiveConfig() {
        guard let vault else {
            archiveConfig = nil; archiveMoves = [:]; archiveUploadEnabled = false
            archiveFactsEnabled = false; archiveBankEnabled = false; archiveLabelsEnabled = true
            return
        }
        archiveConfig = (try? vault.archiveConfig()) ?? nil
        archiveUploadEnabled = (try? vault.archiveUploadEnabled()) ?? false
        archiveBankEnabled = (try? vault.archiveBankEnabled()) ?? false
        archiveLabelsEnabled = (try? vault.archiveLabelsEnabled()) ?? true
        archiveFactsEnabled = (try? vault.archiveFactsEnabled()) ?? false
        reloadArchiveMoves()
    }

    /// Files that were on their way when the app last quit, or before the
    /// connection was changed, go on (unlock, vault switch, connection stored),
    /// and the archive's documents are brought in line with their bookings.
    func resumeArchiveMoves() {
        if archiveSummary.waiting > 0 { processArchiveMoves() }
        queueBankDocuments()
        syncArchiveFacts()
    }

    /// Ends everything that belongs to the archive connection so far: running
    /// requests are cancelled, files and thumbnails in memory dropped, and
    /// answers still on their way no longer count (lock, vault switch, change
    /// of the connection).
    func dropArchiveCaches() {
        archiveGeneration += 1
        // Dropping a call here does not end it in the core: tell the vault
        // that was open that no further request may go out for it.
        vault?.archiveStop()
        archiveMoveRunning = false
        archiveFactsRunning = false
        archiveFactsAgain = false
        archiveFacts = ArchiveFactsReport(written: 0, refused: 0, waiting: 0, failed: 0, problem: "")
        archiveFactsProblem = nil
        archiveCancels.values.forEach { $0() }
        archiveCancels = [:]
        // a document of the archive shown in Quick Look: its decrypted copy goes too
        quickLookURL = nil
        Self.removeQuickLookFiles()
        archiveWaiters = [:]
        archiveFiles = [:]
        archiveThumbnails = [:]
    }

    /// The core's own sentence for an archive error. It never holds the
    /// address or the token (see `archive/client.rs`).
    func archiveMessage(_ error: Error) -> String {
        guard let e = error as? CoreError else { return error.localizedDescription }
        switch e {
        case .Network(let m), .Authentication(let m), .Protocol(let m), .InvalidInput(let m): return m
        case .MissingSetting: return String(localized: "Es ist kein Dokumentenarchiv verbunden.")
        default: return describe(error)
        }
    }

    /// One request to the connected archive. It is cancelled when the vault
    /// is locked or the connection changes, and its answer is dropped then
    /// (nil): nothing asked of one archive may be taken for another's.
    func archiveRequest<T: Sendable>(_ work: @escaping @Sendable (Vault) async throws -> T) async -> Result<T, Error>? {
        guard let vault, archiveConnected else { return nil }
        let generation = archiveGeneration
        let key = UUID()
        let task = Task {
            // not started when the connection ended meanwhile
            try Task.checkCancellation()
            return try await work(vault)
        }
        archiveCancels[key] = { task.cancel() }
        let result = await task.result
        archiveCancels[key] = nil
        guard generation == archiveGeneration, self.vault === vault, !task.isCancelled else { return nil }
        return result
    }

    /// References in this vault into an archive other than the connected one
    /// (made on another device, or before a change).
    func archiveDocumentsElsewhere() -> Int {
        Int((try? vault?.archiveDocumentsElsewhere()) ?? 0)
    }

    /// The archives the references of this vault belong to, read fresh: the
    /// settings ask which of them an address is.
    func archiveIdentities() -> [ArchiveIdentity] {
        (try? vault?.archiveIdentities()) ?? []
    }

    /// Stores address and token (an empty token keeps the stored one).
    /// `archiveId`: which known archive the address is, nil for a new one.
    /// Returns the core's message when it is refused, else nil.
    func saveArchiveConnection(url: String, token: String, archiveId: String?) -> String? {
        guard let vault else { return nil }
        do {
            _ = try vault.setArchiveConnection(url: url, token: token, archiveId: archiveId)
        } catch {
            return archiveMessage(error)
        }
        // everything of the connection before ends here; then the new state,
        // and what waits for this archive goes on
        dropArchiveCaches()
        reloadArchiveConfig()
        documentsVersion += 1
        resumeArchiveMoves()
        return nil
    }

    /// Disconnects the archive. The references stay as plain links.
    func removeArchiveConnection() {
        guard let vault else { return }
        runCore { try vault.removeArchiveConnection() }
        dropArchiveCaches()
        reloadArchiveConfig()
        documentsVersion += 1
    }

    /// Tries the connection. At most one of the two is set (none: locked or
    /// changed meanwhile).
    func testArchive() async -> (info: ArchiveInfo?, problem: String?) {
        switch await archiveRequest({ try await $0.archiveTest() }) {
        case .success(let info)?: return (info, nil)
        case .failure(let error)?: return (nil, archiveMessage(error))
        case nil: return (nil, nil)
        }
    }

    func searchArchive(_ query: String, from: String?, to: String?) async -> (documents: [ArchiveDocument], problem: String?) {
        switch await archiveRequest({ try await $0.archiveSearch(query: query, dateFrom: from, dateTo: to, limit: 30) }) {
        case .success(let documents)?: return (documents, nil)
        case .failure(let error)?: return ([], archiveMessage(error))
        case nil: return ([], nil)
        }
    }

    /// Documents of the archive that fit the booking. An unreachable archive
    /// simply has no suggestion: this runs unasked.
    func archiveSuggestions(for booking: Booking, limit: UInt32 = 3) async -> [ArchiveSuggestion] {
        let id = booking.id
        guard case .success(let found)? = await archiveRequest({ try await $0.archiveSuggestions(bookingId: id, limit: limit) }) else { return [] }
        return found
    }

    /// Bookings without a receipt in the period shown, and the documents of
    /// the archive that fit them (all accounts). Nothing is linked yet.
    func archiveMatches() async -> (found: ArchiveMatches?, problem: String?) {
        let (from, to) = periodRange
        switch await archiveRequest({ try await $0.archiveMatches(dateFrom: from, dateTo: to, limit: 300) }) {
        case .success(let found)?: return (found, nil)
        case .failure(let error)?: return (nil, archiveMessage(error))
        case nil: return (nil, nil)
        }
    }

    /// Links the matches the user accepted (only the vault is written; there
    /// may be hundreds, so off the main thread). The core looks at each one
    /// again as it writes: what got a receipt meanwhile is left out. Returns
    /// how many were linked.
    @discardableResult
    func attachArchiveMatches(_ matches: [ArchiveMatch]) async -> Int {
        guard archiveConnected, !matches.isEmpty else { return 0 }
        let linked = (try? await vaultOffMain { try $0.attachArchiveMatches(matches: matches) }) ?? nil
        guard let linked, linked > 0 else { return 0 }
        documentsVersion += 1
        reloadBookings()
        return Int(linked)
    }

    /// Links a document of the archive to a booking (only the vault is
    /// written). The core refuses a result of an earlier connection. Returns
    /// its message when it is refused, else nil.
    func attachArchiveDocument(_ document: ArchiveDocument, to booking: Booking) -> String? {
        guard let vault else { return nil }
        do {
            _ = try vault.attachArchiveDocument(bookingId: booking.id, archiveId: document.archiveId,
                                                archiveDoc: document.id, title: document.title)
        } catch {
            return archiveMessage(error)
        }
        documentsVersion += 1
        reloadBookings()
        return nil
    }

    /// Fetches the file of a linked archive document for the preview, once per
    /// session. `then` runs when it is there (also when it already was, and
    /// when a fetch started earlier comes in).
    func loadArchiveFile(_ doc: DocumentInfo, then: (@MainActor () -> Void)? = nil) {
        if case .loaded? = archiveFiles[doc.id] { then?(); return }
        guard archiveConnected, doc.isArchiveDocument else { return }
        if let then { archiveWaiters[doc.id, default: []].append(then) }
        if case .loading? = archiveFiles[doc.id] { return }
        // a handful of documents at most: these are decrypted bytes in memory
        if archiveFiles.count >= 8 {
            archiveFiles = archiveFiles.filter { if case .loading = $0.value { true } else { false } }
        }
        archiveFiles[doc.id] = .loading
        let id = doc.id
        archiveJob {
            // nil: locked or changed meanwhile, the state was dropped with everything else
            guard let result = await self.archiveRequest({ try await $0.archiveFile(docId: id, original: false) }) else { return }
            switch result {
            case .success(let file):
                self.archiveFiles[id] = .loaded(file.bytes, mimeType: file.mimeType, fileName: file.fileName)
                (self.archiveWaiters.removeValue(forKey: id) ?? []).forEach { $0() }
            case .failure(let error):
                self.archiveFiles[id] = .failed(self.archiveMessage(error))
                self.archiveWaiters[id] = nil
            }
        }
    }

    /// Work for the archive connection as it is at this moment. It does not
    /// start, and its requests do not count, once the connection has changed
    /// or the vault was locked or switched; then it is cancelled too.
    func archiveJob(_ work: @escaping @MainActor () async -> Void) {
        let generation = archiveGeneration
        let key = UUID()
        let task = Task {
            defer { if generation == self.archiveGeneration { self.archiveCancels[key] = nil } }
            guard generation == self.archiveGeneration else { return }
            await work()
        }
        archiveCancels[key] = { task.cancel() }
    }

    /// Forgets a failed fetch and tries again.
    func reloadArchiveFile(_ doc: DocumentInfo) {
        archiveFiles[doc.id] = nil
        loadArchiveFile(doc)
    }

    /// First page of an archive document, for the search results.
    func archiveThumbnail(_ archiveDoc: Int64) async -> Data? {
        if let cached = archiveThumbnails[archiveDoc] { return cached }
        guard case .success(let data)? = await archiveRequest({ try await $0.archiveThumbnail(archiveDoc: archiveDoc) }) else { return nil }
        if archiveThumbnails.count >= 60 { archiveThumbnails = [:] }
        archiveThumbnails[archiveDoc] = data
        return data
    }

    // MARK: Moving stored files into the archive

    func reloadArchiveMoves() {
        let moves = (try? vault?.archiveMoves()) ?? []
        archiveMoves = Dictionary(moves.map { ($0.docId, $0) }, uniquingKeysWith: { a, _ in a })
        // three SQL counts, no list (the settings show them)
        archiveSummary = (try? vault?.archiveMoveSummary()) ?? ArchiveMoveSummary(inVault: 0, waiting: 0, failed: 0)
    }

    /// Files the user adds from now on go into the archive.
    func setArchiveUpload(_ enabled: Bool) {
        guard let vault else { return }
        runCore { try vault.setArchiveUpload(enabled: enabled) }
        archiveUploadEnabled = (try? vault.archiveUploadEnabled()) ?? false
    }

    /// Documents the banks deliver from now on go into the archive.
    func setArchiveBank(_ enabled: Bool) {
        guard let vault else { return }
        runCore { try vault.setArchiveBank(enabled: enabled) }
        archiveBankEnabled = (try? vault.archiveBankEnabled()) ?? false
        // what counts as "still in the vault" follows the switch
        reloadArchiveMoves()
    }

    /// A document of the bank goes up with the bank as its sender, its kind
    /// and the account as a tag.
    func setArchiveLabels(_ enabled: Bool) {
        guard let vault else { return }
        runCore { try vault.setArchiveLabels(enabled: enabled) }
        archiveLabelsEnabled = (try? vault.archiveLabelsEnabled()) ?? true
    }

    /// After a fetch: the documents the banks delivered since they are moved
    /// are queued (and those another device moved its copy of already). One
    /// statement in the core, off the main thread; nothing while the banks'
    /// documents stay in the vault.
    func queueBankDocuments() {
        guard archiveBankEnabled, archiveConnected, let archive = connectedArchiveId() else { return }
        let generation = archiveGeneration
        Task { @MainActor [weak self] in
            guard let self else { return }
            let queued = (try? await self.vaultOffMain { try $0.queueBankDocuments(archiveId: archive) }) ?? nil
            guard generation == self.archiveGeneration, let queued, queued > 0 else { return }
            await self.refreshArchiveMoves()
            self.processArchiveMoves()
        }
    }

    /// Queues a stored file for the archive (again, after a failure) and
    /// starts working on the queue. The file stays in the vault until the
    /// archive has it.
    func moveToArchive(_ doc: DocumentInfo) {
        guard let vault, archiveConnected, !doc.isLink else { return }
        do {
            try vault.queueArchiveMove(docId: doc.id)
        } catch {
            showError(String(localized: "Beleg konnte nicht ins Archiv verschoben werden"), archiveMessage(error))
            return
        }
        reloadArchiveMoves()
        processArchiveMoves()
    }

    /// Queues files just stored (the ids the core returned for them) for
    /// the archive. One statement for a handful of ids.
    func queueNewReceipts(_ ids: [Int64]) {
        guard let vault, archiveConnected, !ids.isEmpty, let archive = connectedArchiveId() else { return }
        let queued = (try? vault.queueArchiveMoves(docIds: ids, archiveId: archive)) ?? 0
        guard queued > 0 else { return }
        reloadArchiveMoves()
        processArchiveMoves()
    }

    /// Queues all of the user's own receipts that are still in the vault, and
    /// the banks' documents while those are moved too (the core decides).
    /// There may be thousands: off the main thread, and only for the vault
    /// and the connection it was asked for. Returns how many.
    @discardableResult
    func queueAllReceiptsForArchive() async -> Int {
        // the archive the user means, named now: should another one be
        // connected by the time the worker runs, the core queues nothing
        guard archiveConnected, let archive = connectedArchiveId() else { return 0 }
        let generation = archiveGeneration
        let queued = (try? await vaultOffMain { try $0.queueArchiveMoves(docIds: nil, archiveId: archive) }) ?? nil
        guard generation == archiveGeneration, let queued, queued > 0 else { return 0 }
        await refreshArchiveMoves()
        processArchiveMoves()
        return Int(queued)
    }

    /// The id of the connected archive (nil without one).
    func connectedArchiveId() -> String? {
        archiveIdentities().first(where: \.connected)?.id
    }

    /// `reloadArchiveMoves` for a queue that may be long (everything the
    /// user owns, on its way): read off the main thread, shown only for the
    /// vault and connection it was read for.
    func refreshArchiveMoves() async {
        let generation = archiveGeneration
        let read = try? await vaultOffMain { vault -> ([ArchiveMove], ArchiveMoveSummary) in
            (try vault.archiveMoves(), try vault.archiveMoveSummary())
        }
        guard generation == archiveGeneration, let (moves, summary) = read ?? nil else { return }
        archiveMoves = Dictionary(moves.map { ($0.docId, $0) }, uniquingKeysWith: { a, _ in a })
        archiveSummary = summary
    }

    /// Takes a file off the queue; it stays in the vault.
    func keepInVault(_ doc: DocumentInfo) {
        guard let vault else { return }
        runCore { try vault.cancelArchiveMove(docId: doc.id) }
        reloadArchiveMoves()
    }

    /// Works on the queue until nothing waits any more: the archive needs a
    /// moment to take a file in. An unreachable archive ends the run, and so
    /// does a long stretch in which nothing moves; the next change, unlock or
    /// sync starts it again.
    func processArchiveMoves() {
        guard archiveConnected, !archiveMoveRunning else { return }
        archiveMoveRunning = true
        let generation = archiveGeneration
        archiveJob {
            defer { if generation == self.archiveGeneration { self.archiveMoveRunning = false } }
            var idle = 0
            var before: UInt32?
            while idle < 40 {
                // with the folder backup on, a file is in the folder before it
                // can leave the vault
                guard await self.mirrorBeforeMoving(), generation == self.archiveGeneration else { return }
                guard let result = await self.archiveRequest({ try await $0.processArchiveMoves() }) else { return }
                await self.refreshArchiveMoves()
                guard generation == self.archiveGeneration else { return }
                guard case .success(let report) = result else { return }
                if report.moved > 0 {
                    self.documentsVersion += 1
                    self.reloadBookings()
                }
                guard report.waiting > 0 else { return }
                let progress = report.moved > 0 || report.failed > 0 || report.waiting != before
                idle = progress ? 0 : idle + 1
                before = report.waiting
                try? await Task.sleep(for: .seconds(idle < 5 ? 2 : 5))
                guard generation == self.archiveGeneration, !Task.isCancelled else { return }
            }
        }
    }

    // MARK: Booking data on the archive's documents

    /// Linked documents of the archive carry date, amount, counterparty and
    /// account of their booking.
    func setArchiveFacts(_ enabled: Bool) {
        guard let vault else { return }
        runCore { try vault.setArchiveFacts(enabled: enabled) }
        archiveFactsEnabled = (try? vault.archiveFactsEnabled()) ?? false
        if !archiveFactsEnabled {
            archiveFacts = ArchiveFactsReport(written: 0, refused: 0, waiting: 0, failed: 0, problem: "")
            archiveFactsProblem = nil
        }
        syncArchiveFacts()
    }

    /// Tries again what the archive refused.
    func retryArchiveFacts() {
        guard let vault else { return }
        runCore { try vault.retryArchiveFacts() }
        syncArchiveFacts()
    }

    /// Brings the archive's documents in line with their bookings. Called
    /// after every change to documents; the core sends nothing when nothing
    /// differs. Changes in quick succession share one run.
    func syncArchiveFacts() {
        guard archiveConnected, archiveFactsEnabled else { return }
        if archiveFactsRunning { archiveFactsAgain = true; return }
        archiveFactsRunning = true
        let generation = archiveGeneration
        archiveJob {
            defer { if generation == self.archiveGeneration { self.archiveFactsRunning = false } }
            repeat {
                self.archiveFactsAgain = false
                try? await Task.sleep(for: .milliseconds(500))
                guard generation == self.archiveGeneration, !Task.isCancelled else { return }
                guard let result = await self.archiveRequest({ try await $0.processArchiveFacts() }) else { return }
                switch result {
                case .success(let report):
                    self.archiveFacts = report
                    self.archiveFactsProblem = nil
                    // a handful per run: on with the rest, as long as the run
                    // got anywhere (a refusal is counted and ends after a few)
                    if report.written + report.refused > 0, report.waiting > 0 { self.archiveFactsAgain = true }
                case .failure(let error):
                    self.archiveFactsProblem = self.archiveMessage(error)
                    return
                }
            } while self.archiveFactsAgain
        }
    }

    /// The document Space and ⌘Y show: the first stored file, or the first
    /// linked document of the connected archive. A plain link has no preview.
    func previewableDocument(for booking: Booking) -> DocumentInfo? {
        documents(for: booking).first { canPreview($0) }
    }

    func canPreview(_ doc: DocumentInfo) -> Bool {
        !doc.isLink || (doc.isArchiveDocument && archiveConnected)
    }
}
