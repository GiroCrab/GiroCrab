import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Document archive: connection, the links into it and the filter for
/// bookings without a receipt. Nothing here reaches a network: the requests
/// themselves are tested in the core against a scripted server.
@MainActor
final class ArchiveModelTests: XCTestCase {
    private var model: AppModel!
    private let address = "http://192.168.1.20:8000"

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        XCTAssertNil(model.errorMessage)
    }

    override func tearDown() async throws { model = nil }

    private func plainBooking() throws -> Booking {
        try XCTUnwrap(model.bookings.first { $0.documentCount == 0 })
    }

    func testWithoutAnArchiveNothingIsAskedFor() async throws {
        XCTAssertNil(model.archiveConfig)
        XCTAssertFalse(model.archiveConnected)
        let booking = try plainBooking()
        let suggestions = await model.archiveSuggestions(for: booking)
        XCTAssertTrue(suggestions.isEmpty)
        let found = await model.searchArchive("rechnung", from: nil, to: nil)
        XCTAssertTrue(found.documents.isEmpty)
        XCTAssertNil(found.problem)
        let thumbnail = await model.archiveThumbnail(1)
        XCTAssertNil(thumbnail)
        XCTAssertNil(model.errorMessage)
    }

    func testConnectingChecksTheAddressAndKeepsTheTokenOutOfTheModel() throws {
        // unencrypted only inside the own network, never with credentials in the address
        XCTAssertNotNil(model.saveArchiveConnection(url: "http://8.8.8.8:8000", token: "t", archiveId: nil))
        XCTAssertNotNil(model.saveArchiveConnection(url: "https://name:geheim@archiv.example", token: "t", archiveId: nil))
        XCTAssertNotNil(model.saveArchiveConnection(url: address, token: "", archiveId: nil), "a first connection needs a token")
        XCTAssertFalse(model.archiveConnected)

        XCTAssertNil(model.saveArchiveConnection(url: address + "/", token: "geheimer-token", archiveId: nil))
        let config = try XCTUnwrap(model.archiveConfig)
        XCTAssertEqual(config.url, address)
        XCTAssertTrue(config.hasToken)
        XCTAssertTrue(config.unencrypted)
        XCTAssertTrue(model.archiveConnected)
        // the generic settings never list it
        XCTAssertFalse(model.settings.contains { $0.key.hasPrefix("archive.") })

        model.removeArchiveConnection()
        XCTAssertNil(model.archiveConfig)
        XCTAssertFalse(model.archiveConnected)
        XCTAssertNil(model.errorMessage)
    }

    func testALinkIntoTheArchiveCanBePreviewedAPlainLinkCannot() throws {
        let booking = try plainBooking()
        XCTAssertNil(model.saveArchiveConnection(url: address, token: "geheimer-token", archiveId: nil))
        XCTAssertNil(model.attachLink(address + "/documents/42/details", title: "Rechnung", to: booking))
        XCTAssertNil(model.attachLink("https://anderswo.example/beleg", title: "", to: booking))

        let docs = model.documents(for: booking)
        let archived = try XCTUnwrap(docs.first { $0.isArchiveDocument })
        let plain = try XCTUnwrap(docs.first { !$0.isArchiveDocument })
        XCTAssertEqual(archived.archiveDoc, 42)
        XCTAssertTrue(model.canPreview(archived))
        XCTAssertFalse(model.canPreview(plain))
        XCTAssertEqual(model.previewableDocument(for: booking)?.id, archived.id)
        model.reloadArchiveConfig()
        XCTAssertEqual(model.archiveConfig?.linkedDocuments, 1)
        XCTAssertEqual(model.archiveDocumentsElsewhere(), 0)

        // disconnected: the document is a plain link again, nothing is fetched
        model.removeArchiveConnection()
        let after = try XCTUnwrap(model.documents(for: booking).first { $0.title == "Rechnung" })
        XCTAssertFalse(after.isArchiveDocument)
        XCTAssertEqual(after.linkUrl, address + "/documents/42/details")
        XCTAssertNil(model.previewableDocument(for: booking))
        XCTAssertTrue(model.archiveFiles.isEmpty)
        // the vault remembers that the link belonged to an archive: connecting asks about it
        XCTAssertEqual(model.archiveDocumentsElsewhere(), 1)

        // the vault knows the archive by its documents and the address it had
        let known = model.archiveIdentities()
        XCTAssertEqual(known.count, 1)
        XCTAssertEqual(known.first?.documents, 1)
        XCTAssertEqual(known.first?.connected, false)
        let before = try XCTUnwrap(known.first?.id)

        // an archive nobody knows is refused, nothing is connected
        XCTAssertNotNil(model.saveArchiveConnection(url: "https://archiv.example", token: "t", archiveId: "unbekannt"))
        XCTAssertFalse(model.archiveConnected)

        // said to be the same archive: its document again, also at a new address
        XCTAssertNil(model.saveArchiveConnection(url: "https://archiv.example", token: "t", archiveId: before))
        XCTAssertEqual(model.documents(for: booking).first { $0.title == "Rechnung" }?.archiveDoc, 42)
        XCTAssertEqual(model.archiveIdentities().first { $0.connected }?.id, before)
        XCTAssertEqual(model.archiveDocumentsElsewhere(), 0)

        // a new archive: a plain link, and the vault still knows where it belongs
        // (the stored token is for the stored address only: a new one needs its own)
        XCTAssertNotNil(model.saveArchiveConnection(url: "https://neu.example", token: "", archiveId: nil))
        XCTAssertNotNil(model.saveArchiveConnection(url: "https://neu.example", token: "", archiveId: before))
        XCTAssertEqual(model.archiveConfig?.url, "https://archiv.example")
        XCTAssertNil(model.saveArchiveConnection(url: "https://neu.example", token: "t2", archiveId: nil))
        XCTAssertNil(model.documents(for: booking).first { $0.title == "Rechnung" }?.archiveDoc)
        XCTAssertEqual(model.archiveDocumentsElsewhere(), 1)
        XCTAssertEqual(model.archiveIdentities().first { !$0.connected }?.id, before)
    }

    /// A change of the connection ends what belongs to the one before:
    /// requests, files in memory, answers still on their way.
    func testAChangeOfTheConnectionDropsWhatWasFetched() async throws {
        XCTAssertNil(model.saveArchiveConnection(url: address, token: "geheimer-token", archiveId: nil))
        let generation = model.archiveGeneration
        model.archiveFiles[-1] = .loaded(Data([1]), mimeType: "application/pdf", fileName: "x.pdf")
        model.archiveThumbnails[1] = Data([1])
        var waited = false
        model.archiveWaiters[-1] = [{ waited = true }]

        XCTAssertNil(model.saveArchiveConnection(url: address, token: "neuer-token", archiveId: nil))
        XCTAssertGreaterThan(model.archiveGeneration, generation)
        XCTAssertTrue(model.archiveFiles.isEmpty)
        XCTAssertTrue(model.archiveThumbnails.isEmpty)
        XCTAssertTrue(model.archiveWaiters.isEmpty)
        XCTAssertFalse(waited)

        // an answer that arrives after the connection was removed is dropped
        async let late = model.archiveRequest { _ in
            try await Task.sleep(for: .milliseconds(150))
            return 1
        }
        try await Task.sleep(for: .milliseconds(30))
        model.removeArchiveConnection()
        let result = await late
        XCTAssertNil(result)
        XCTAssertTrue(model.archiveCancels.isEmpty)
    }

    func testTheFilterShowsOnlyBookingsWithoutAReceipt() throws {
        let all = model.bookings.count
        let with = model.bookings.filter { $0.documentCount > 0 }.count
        XCTAssertGreaterThan(with, 0)
        XCTAssertEqual(model.filterSymbol, "line.3.horizontal.decrease")

        model.missingDocumentOnly = true
        XCTAssertTrue(model.currentFilter.withoutDocument)
        XCTAssertFalse(model.currentFilter.listHasFlow)
        XCTAssertEqual(model.bookings.count, all - with)
        XCTAssertTrue(model.bookings.allSatisfy { $0.documentCount == 0 })
        XCTAssertEqual(model.filterSymbol, "line.3.horizontal.decrease.circle.fill")

        // a linked receipt takes its booking off the list
        let booking = try XCTUnwrap(model.bookings.first)
        XCTAssertNil(model.attachLink("https://archiv.example/documents/7", title: "", to: booking))
        XCTAssertFalse(model.bookings.contains { $0.id == booking.id })

        model.missingDocumentOnly = false
        XCTAssertEqual(model.bookings.count, all)
    }

    // MARK: Moving files into the archive

    /// Nothing listens there: a request fails at once and never leaves this Mac.
    private let nowhere = "http://127.0.0.1:9"

    func testAFileOnItsWayToTheArchiveStaysInTheVault() async throws {
        let vault = try XCTUnwrap(model.vault)
        let booking = try plainBooking()
        XCTAssertNil(model.saveArchiveConnection(url: nowhere, token: "geheimer-token", archiveId: nil))
        let id = try vault.attachDocumentToBooking(bookingId: booking.id, fileName: "Rechnung.pdf", bytes: Data("%PDF-1.4 inhalt".utf8))
        let doc = try XCTUnwrap(model.documents(for: booking).first { $0.id == id })

        model.moveToArchive(doc)
        XCTAssertEqual(model.archiveMoves[id]?.state, "queued")
        XCTAssertEqual(model.archiveMoves[id]?.hasFailed, false)
        // the archive cannot be reached: the file waits, and is still the booking's document
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(model.archiveMoves[id]?.state, "queued")
        XCTAssertNotNil(model.documentData(doc))
        XCTAssertTrue(model.documents(for: booking).contains { $0.id == id && !$0.isLink })
        XCTAssertNil(model.errorMessage)

        model.keepInVault(doc)
        XCTAssertNil(model.archiveMoves[id])
        XCTAssertNotNil(model.documentData(doc))

        // deleted while it waits: off the queue as well
        model.moveToArchive(doc)
        model.deleteDocument(doc)
        XCTAssertTrue(model.archiveMoves.isEmpty)
    }

    func testNewReceiptsAreQueuedOnlyWhenAskedForAndBankDocumentsNever() async throws {
        let booking = try plainBooking()
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("gc-test-\(UUID().uuidString).pdf")
        try Data("%PDF-1.4 inhalt".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertNil(model.saveArchiveConnection(url: nowhere, token: "geheimer-token", archiveId: nil))

        // switched off: a new file stays a plain vault document
        XCTAssertFalse(model.archiveUploadEnabled)
        model.attach([file], to: booking)
        XCTAssertTrue(model.archiveMoves.isEmpty)

        model.setArchiveUpload(true)
        XCTAssertTrue(model.archiveUploadEnabled)
        let other = try XCTUnwrap(model.bookings.first { $0.documentCount == 0 && $0.id != booking.id })
        model.attach([file], to: other)
        let queued = try XCTUnwrap(model.documents(for: other).first)
        XCTAssertEqual(model.archiveMoves.keys.sorted(), [queued.id])

        // everything of the user's own, on request; what the bank delivered stays
        let receipts = model.allDocuments(scope: .receipts).filter { !$0.isLink }.count
        XCTAssertEqual(Int(model.archiveSummary.inVault), receipts - 1)
        XCTAssertEqual(Int(model.archiveSummary.waiting), 1)
        let queuedNow = await model.queueAllReceiptsForArchive()
        XCTAssertEqual(queuedNow, receipts - 1)
        XCTAssertEqual(Int(model.archiveSummary.inVault), 0)
        XCTAssertEqual(Int(model.archiveSummary.waiting), receipts)
        let bank = Set(model.allDocuments(scope: .postbox).map(\.id))
        XCTAssertFalse(bank.isEmpty)
        XCTAssertTrue(bank.isDisjoint(with: model.archiveMoves.keys))

        // disconnecting clears the queue and the switch; the files stay
        model.removeArchiveConnection()
        XCTAssertTrue(model.archiveMoves.isEmpty)
        XCTAssertFalse(model.archiveUploadEnabled)
        XCTAssertEqual(model.allDocuments(scope: .receipts).filter { !$0.isLink }.count, receipts)
    }

    /// The banks' documents stay in the vault until the user says otherwise.
    /// The switch decides what "still in the vault" counts, and it goes with
    /// the connection, like the choice to pass on bank, kind and account.
    func testDocumentsOfTheBanksStayInTheVaultUntilSwitchedOn() async throws {
        XCTAssertFalse(model.archiveBankEnabled)
        XCTAssertNil(model.saveArchiveConnection(url: nowhere, token: "geheimer-token", archiveId: nil))
        XCTAssertFalse(model.archiveBankEnabled)
        XCTAssertTrue(model.archiveLabelsEnabled)
        let own = model.archiveSummary.inVault

        model.setArchiveBank(true)
        XCTAssertTrue(model.archiveBankEnabled)
        // the demo vault holds documents of its banks: they can be moved now
        XCTAssertGreaterThan(model.archiveSummary.inVault, own)
        model.setArchiveLabels(false)
        XCTAssertFalse(model.archiveLabelsEnabled)
        model.setArchiveBank(false)
        XCTAssertEqual(model.archiveSummary.inVault, own)

        // without the connection neither switch is left
        model.setArchiveBank(true)
        model.removeArchiveConnection()
        XCTAssertFalse(model.archiveBankEnabled)
        XCTAssertTrue(model.archiveLabelsEnabled)
        XCTAssertNil(model.errorMessage)
    }

    /// Writing booking data into the archive is the user's switch, runs only
    /// while it is on, and its state goes with the connection.
    func testBookingDataForTheArchiveIsOffUntilSwitchedOn() async throws {
        XCTAssertNil(model.saveArchiveConnection(url: nowhere, token: "geheimer-token", archiveId: nil))
        XCTAssertFalse(model.archiveFactsEnabled)
        let booking = try plainBooking()
        XCTAssertNil(model.attachLink(nowhere + "/documents/42/details", title: "Rechnung", to: booking))
        XCTAssertFalse(model.archiveFactsRunning, "switched off: a change to documents starts nothing")

        model.setArchiveFacts(true)
        XCTAssertTrue(model.archiveFactsEnabled)
        XCTAssertTrue(model.archiveFactsRunning)
        // nobody listens at that address: the run ends with the core's own sentence
        for _ in 0..<150 where model.archiveFactsRunning { try await Task.sleep(for: .milliseconds(100)) }
        XCTAssertFalse(model.archiveFactsRunning)
        let problem = try XCTUnwrap(model.archiveFactsProblem)
        XCTAssertFalse(problem.contains("127.0.0.1"))
        XCTAssertFalse(problem.contains("geheimer-token"))

        // the next change to documents tries again
        model.deleteDocument(try XCTUnwrap(model.documents(for: booking).first))
        XCTAssertTrue(model.archiveFactsRunning)

        // without the connection there is neither switch nor state
        model.removeArchiveConnection()
        XCTAssertFalse(model.archiveFactsEnabled)
        XCTAssertFalse(model.archiveFactsRunning)
        XCTAssertNil(model.archiveFactsProblem)
        model.documentsVersion += 1
        XCTAssertFalse(model.archiveFactsRunning)
    }

    /// The matches the user ticked are linked in one go. One that belongs to
    /// an earlier connection, or is linked already, is left out.
    func testAcceptedMatchesAreLinkedTogether() async throws {
        XCTAssertNil(model.saveArchiveConnection(url: address, token: "geheimer-token", archiveId: nil))
        let open = model.bookings.filter { $0.documentCount == 0 }
        let (first, second) = (open[0], open[1])
        let archive = try XCTUnwrap(model.connectedArchiveId())
        func match(_ booking: Booking, _ doc: Int64, _ archiveId: String) -> ArchiveMatch {
            ArchiveMatch(bookingId: booking.id, booking: "", document: ArchiveDocument(
                id: doc, archiveId: archiveId, title: "Rechnung \(doc)", created: "2026-11-05", correspondent: "",
                documentType: "", fileName: "", pageCount: 1, excerpt: "", linked: false), score: 8, merchantFits: true,
                         ambiguous: false)
        }
        let linked = await model.attachArchiveMatches([
            match(first, 7, archive), match(second, 8, "ein-anderes-archiv"), match(first, 7, archive),
        ])
        XCTAssertEqual(linked, 1)
        XCTAssertEqual(model.documents(for: first).map(\.archiveDoc), [7])
        XCTAssertTrue(model.documents(for: second).isEmpty)
        XCTAssertEqual(model.bookings.first { $0.id == first.id }?.documentCount, 1)
        let none = await model.attachArchiveMatches([])
        XCTAssertEqual(none, 0)

        // ticked without asking: no rival, and more than the amount fits
        var weak = match(second, 9, archive)
        XCTAssertTrue(ArchiveMatchSheet.isClear(weak))
        weak.merchantFits = false
        XCTAssertFalse(ArchiveMatchSheet.isClear(weak), "amount and date alone can be a coincidence")
        weak.merchantFits = true; weak.ambiguous = true
        XCTAssertFalse(ArchiveMatchSheet.isClear(weak))

        // without a connection nothing is asked and nothing linked
        model.removeArchiveConnection()
        let found = await model.archiveMatches()
        XCTAssertNil(found.found)
        XCTAssertNil(found.problem)
        let late = await model.attachArchiveMatches([match(second, 9, archive)])
        XCTAssertEqual(late, 0)
    }

    /// The app's side of the way against a real archive, for whoever has a
    /// test instance at hand (skipped otherwise):
    /// `GIROCRAB_LIVE_ARCHIVE=http://127.0.0.1:8000 GIROCRAB_LIVE_ARCHIVE_TOKEN=… swift test --filter ArchiveModelTests`.
    /// It needs at least one document there and changes nothing in the archive.
    func testAgainstARealArchive() async throws {
        let env = ProcessInfo.processInfo.environment
        guard let base = env["GIROCRAB_LIVE_ARCHIVE"], let token = env["GIROCRAB_LIVE_ARCHIVE_TOKEN"] else {
            throw XCTSkip("set GIROCRAB_LIVE_ARCHIVE and GIROCRAB_LIVE_ARCHIVE_TOKEN")
        }
        XCTAssertNil(model.saveArchiveConnection(url: base, token: token, archiveId: nil))
        let test = await model.testArchive()
        XCTAssertNil(test.problem)
        XCTAssertGreaterThan(try XCTUnwrap(test.info).documentCount, 0)

        // search, thumbnail, link, preview, Quick Look
        let found = await model.searchArchive("", from: nil, to: nil)
        XCTAssertNil(found.problem)
        let document = try XCTUnwrap(found.documents.first)
        let thumbnail = await model.archiveThumbnail(document.id)
        XCTAssertNotNil(thumbnail.flatMap { AppModel.image(from: $0) }, "a picture the app can draw")
        let booking = try plainBooking()
        XCTAssertNil(model.attachArchiveDocument(document, to: booking))
        let linked = try XCTUnwrap(model.documents(for: booking).first)
        XCTAssertTrue(linked.isArchiveDocument)
        XCTAssertEqual(linked.linkOrigin, String(localized: "Archiv"))

        model.loadArchiveFile(linked)
        for _ in 0..<100 {
            if case .loading? = model.archiveFiles[linked.id] { try await Task.sleep(for: .milliseconds(100)) } else { break }
        }
        guard case .loaded(let bytes, let mimeType, let fileName)? = model.archiveFiles[linked.id] else {
            return XCTFail("the file did not load: \(String(describing: model.archiveFiles[linked.id]))")
        }
        XCTAssertEqual(mimeType, "application/pdf")
        XCTAssertTrue(bytes.starts(with: Data("%PDF".utf8)))
        XCTAssertFalse(fileName.isEmpty)
        model.quickLook(linked)
        let shown = try XCTUnwrap(model.quickLookURL)
        XCTAssertEqual(try Data(contentsOf: shown), bytes)

        // suggestions and the pass over all open bookings answer without an error
        _ = await model.archiveSuggestions(for: booking)
        let matches = await model.archiveMatches()
        XCTAssertNil(matches.problem)
        XCTAssertNotNil(matches.found)

        // locked: the file in memory and the Quick Look copy are gone
        model.removeArchiveConnection()
        XCTAssertTrue(model.archiveFiles.isEmpty)
        XCTAssertNil(model.quickLookURL)
        XCTAssertFalse(FileManager.default.fileExists(atPath: shown.path))
    }

    func testTheSearchWindowReachesBackFromTheBooking() {
        let window = ArchiveSearchSheet.window(around: "2026-11-07")
        XCTAssertEqual(window?.from, "2026-09-08")
        XCTAssertEqual(window?.to, "2026-11-21")
        XCTAssertNil(ArchiveSearchSheet.window(around: "gestern"))
    }
}
