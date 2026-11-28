import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// "Alle abrufen": cancelling one login's prompt skips it instead of ending
/// the whole batch, and the sync window keeps every login's status until
/// "Fertig" (demo vault: every login asks for a TAN).
@MainActor
final class SyncBatchTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws {
        model.cancelAllSync()
        model = nil
    }

    /// The logins "Alle abrufen" works through, in order.
    private var batch: [BankConnection] {
        model.connections.filter { !model.usesPortal($0) && !$0.allAccountsArchived }
    }

    private func waitUntil(_ what: String, timeout: Double = 5,
                           file: StaticString = #filePath, line: UInt = #line,
                           _ condition: () -> Bool) async throws {
        let end = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > end { XCTFail("Timed out waiting for \(what)", file: file, line: line); return }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    private func waitForChallenge(of c: BankConnection, file: StaticString = #filePath, line: UInt = #line) async throws {
        try await waitUntil("TAN prompt of \(c.displayName)", file: file, line: line) {
            model.activeChallenge?.connection.id == c.id
        }
    }

    func testSkippingGoesOnWithTheNextLoginAndKeepsTheSkippedOnesInTheWindow() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()

        try await waitForChallenge(of: logins[0])
        XCTAssertEqual(model.syncBatchProgress?.current, 1)
        XCTAssertEqual(model.syncBatchProgress?.total, logins.count)
        XCTAssertTrue(model.canSkipInSyncBatch)
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .running)
        XCTAssertEqual(model.syncRun?.status(of: logins[1].id), .waiting)
        XCTAssertEqual(model.syncRun?.remaining, logins.count)
        model.skipCurrentSync()

        try await waitForChallenge(of: logins[1])
        XCTAssertEqual(model.syncBatchProgress?.current, 2)
        model.skipCurrentSync()

        for (i, c) in logins.enumerated().dropFirst(2) {
            try await waitForChallenge(of: c)
            XCTAssertEqual(model.syncBatchProgress?.current, i + 1)
            XCTAssertTrue(model.canSkipInSyncBatch, "also the last login is skipped, not cancelled")
            model.skipCurrentSync()
        }

        // the last skip ends its bank dialog in the background
        try await waitUntil("run over") { model.syncRun?.isFinished == true && model.syncingConnectionID == nil }
        for c in logins { XCTAssertEqual(model.syncRun?.status(of: c.id), .skipped(reason: AppModel.skippedByUser), c.displayName) }
        XCTAssertNil(model.syncBatchProgress)
        XCTAssertFalse(model.canSkipInSyncBatch)
        XCTAssertNil(model.syncingConnectionID)
        XCTAssertFalse(model.toast?.contains("Übersprungen") == true, "the window tells it, no notice on top")

        // "Fertig" closes the window
        model.closeSyncRun()
        XCTAssertNil(model.syncRun)
    }

    /// The window stays while a run goes on: it cannot be closed before the end.
    func testTheWindowCannotBeClosedWhileTheRunGoesOn() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.closeSyncRun()
        XCTAssertNotNil(model.syncRun)
        XCTAssertTrue(model.syncRunActive)
    }

    /// A finished login shows its result in its row instead of a notice.
    func testAFinishedLoginIsMarkedDoneWithItsBookings() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.submitCode("123456")
        try await waitForChallenge(of: logins[1])
        guard case .done(let imported, _)? = model.syncRun?.status(of: logins[0].id) else {
            return XCTFail("first login not done: \(String(describing: model.syncRun?.status(of: logins[0].id)))")
        }
        XCTAssertNotNil(imported)
        XCTAssertEqual(model.syncBatchProgress?.current, 2)
    }

    func testSyncAllIncludesManualAndScheduledLoginsAndSkipsNever() async throws {
        model.bankingPolicy = BankingPolicy(publicKeys: [], store: MemoryBankingPolicyStore(),
                                            fetcher: nil, bundled: nil, distribution: .developerID,
                                            bundleVersion: "1")
        let logins = batch
        XCTAssertGreaterThanOrEqual(logins.count, 3)
        guard logins.count >= 3 else { return }
        let never = logins[0]
        let manual = logins[1]
        let scheduled = logins[2]
        XCTAssertTrue(model.setSyncSchedule(never.id, AutoSync.never))
        XCTAssertTrue(model.setSyncSchedule(manual.id, 0))
        XCTAssertTrue(model.setSyncSchedule(scheduled.id, 4))

        model.requestSyncAll()
        try await waitForChallenge(of: manual)
        XCTAssertEqual(model.syncBatchProgress?.total, logins.count - 1)
        model.skipCurrentSync()
        try await waitForChallenge(of: scheduled)

        model.cancelAllSync()
        model.requestFullSync(never)
        try await waitForChallenge(of: never)
    }

    func testSchedulesAreStoredPerConnectionInItsVault() throws {
        let defaults = AppDefaults.store
        let previous = defaults.object(forKey: AutoSync.hoursKey)
        defer { defaults.set(previous, forKey: AutoSync.hoursKey) }
        defaults.set(6, forKey: AutoSync.hoursKey)

        let connections = Array(batch.prefix(3))
        XCTAssertEqual(connections.count, 3)
        guard connections.count == 3 else { return }
        let first = connections[0].id
        let second = connections[1].id
        let third = connections[2].id
        XCTAssertEqual(model.syncSchedule(first), 6)
        XCTAssertTrue(model.setSyncSchedule(first, AutoSync.never))
        XCTAssertTrue(model.setSyncSchedule(second, 0))
        let vault = try XCTUnwrap(model.vault)
        XCTAssertEqual(try vault.getSetting(key: AutoSync.scheduleKey(first)), "-1")
        XCTAssertEqual(try vault.getSetting(key: AutoSync.scheduleKey(second)), "0")
        XCTAssertEqual(model.syncSchedule(third), 6)
        XCTAssertEqual(model.syncSchedule(first), AutoSync.never)
        XCTAssertEqual(model.syncSchedule(second), 0)
    }

    func testCancelAllEndsTheBatch() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])

        model.cancelAllSync()
        XCTAssertNil(model.activeChallenge)
        XCTAssertNil(model.syncBatchProgress)
        XCTAssertFalse(model.canSkipInSyncBatch)
        // the window stays with every login as skipped
        XCTAssertEqual(model.syncRun?.isFinished, true)
        for c in logins { XCTAssertEqual(model.syncRun?.status(of: c.id), .skipped(reason: AppModel.syncRunCancelled)) }

        // no further login is asked for
        try await Task.sleep(for: .milliseconds(500))
        XCTAssertNil(model.activeChallenge)
        XCTAssertNil(model.pendingLogin)
        XCTAssertNil(model.setupRequest)
    }

    /// A failed login ends only itself (#1041): its row shows the error, no alert
    /// stops the run, and it goes on with the next login right away.
    func testAFailedLoginIsShownInItsRowAndTheRunGoesOn() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.Network(message: "Zeitüberschreitung"))

        XCTAssertNil(model.errorMessage)
        guard case .failed(let message)? = model.syncRun?.status(of: logins[0].id) else {
            return XCTFail("first login not failed")
        }
        XCTAssertTrue(message.contains("Zeitüberschreitung"))
        try await waitForChallenge(of: logins[1])
        XCTAssertEqual(model.syncBatchProgress?.current, 2)
    }

    /// "Eigene Registrierung eintragen …" opens Settings: the window closes first,
    /// no queue stays behind.
    func testEnteringTheOwnRegistrationEndsTheRun() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.enterOwnProductID()
        XCTAssertNil(model.syncBatchProgress)
        XCTAssertNil(model.syncRun)
        XCTAssertTrue(model.syncQueue.isEmpty)
    }

    /// "Erneut versuchen" in a failed row: the login comes again after the ones
    /// still waiting.
    func testRetryInAFailedRowFetchesItAgainAfterTheRest() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.Network(message: "x"))
        try await waitForChallenge(of: logins[1])
        model.retryInSyncRun(logins[0].id)
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .waiting)

        for c in logins.dropFirst() {
            try await waitForChallenge(of: c)
            model.skipCurrentSync()
        }
        try await waitForChallenge(of: logins[0])
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .running)
        model.skipCurrentSync()
        try await waitUntil("run over") { model.syncRun?.isFinished == true }
    }

    /// Retry once the run is over: the window comes back to life for that login.
    func testRetryAfterTheEndRestartsThatLogin() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.cancelAllSync()
        try await waitUntil("cancelled session gone") { model.syncingConnectionID == nil }
        XCTAssertEqual(model.syncRun?.isFinished, true)

        model.retryInSyncRun(logins[1].id)
        try await waitForChallenge(of: logins[1])
        XCTAssertTrue(model.syncRunActive)
        XCTAssertEqual(model.syncRun?.status(of: logins[1].id), .running)
        model.skipCurrentSync()
        try await waitUntil("run over again") { model.syncRun?.isFinished == true }
    }

    /// A missing setting opens the setup form and keeps the batch: cancelling the
    /// form skips this login, completing it fetches the login and goes on.
    func testSetupFormKeepsTheBatch() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.MissingSetting(key: "x"))
        XCTAssertEqual(model.setupRequest?.connection.id, logins[0].id)
        XCTAssertNotNil(model.syncBatchProgress, "the batch waits for the form")
        model.cancelSetupRequest()
        try await waitForChallenge(of: logins[1])

        model.finishSync(error: CoreError.MissingSetting(key: "x"))
        let request = try XCTUnwrap(model.setupRequest)
        model.completeSetup(request, values: [:])
        try await waitForChallenge(of: logins[1])
        XCTAssertEqual(model.syncBatchProgress?.current, 2, "the same login again")
        model.skipCurrentSync()
        try await waitForChallenge(of: logins[2])
    }

    /// A login that needs the browser (Trade Republic) keeps the batch: cancelling
    /// the browser window skips it and goes on with the next login (#1041).
    func testCancellingTheBrowserLoginGoesOnWithTheNextLogin() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.BrowserLoginRequired(message: "Bitte im Browser anmelden"))
        XCTAssertEqual(model.webLoginConnection?.id, logins[0].id)
        XCTAssertNotNil(model.syncBatchProgress, "the batch waits for the browser login")

        model.cancelWebLogin()
        try await waitForChallenge(of: logins[1])
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(model.syncBatchProgress?.current, 2)
    }

    /// A browser window closed without a fetch is no success: the login is skipped
    /// (with "Erneut versuchen"), and the run goes on exactly once.
    func testClosingTheBrowserWindowWithoutAnImportSkipsTheLogin() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.BrowserLoginRequired(message: "Bitte im Browser anmelden"))
        model.webLoginConnection = nil   // a window's own close button
        try await waitForChallenge(of: logins[1])
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .skipped(reason: AppModel.browserClosed))
        XCTAssertTrue(AppModel.retryableSkip(AppModel.browserClosed))
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(model.activeChallenge?.connection.id, logins[1].id, "not advanced twice")
        XCTAssertEqual(model.syncRun?.status(of: logins[2].id), .waiting)
    }

    /// A browser window that imported the login itself (FNZ) marks it fetched when it closes.
    func testABrowserImportCountsWhenTheWindowCloses() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.BrowserLoginRequired(message: "Bitte im Browser anmelden"))
        model.webLoginImportedInSyncRun(logins[0].id, imported: 7)
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .running, "counted only once the window closes")
        model.webLoginConnection = nil
        try await waitForChallenge(of: logins[1])
        guard case .done(let imported, _)? = model.syncRun?.status(of: logins[0].id) else {
            return XCTFail("browser import not counted")
        }
        XCTAssertEqual(imported, 7)
    }

    /// "Alle abbrechen" while a PIN prompt is open (no bank dialog yet) ends the run at once:
    /// nothing stays waiting, and a retry starts right away.
    func testCancelAllAtAPromptEndsTheRunAtOnce() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.MissingSetting(key: "x"))   // setup prompt, no session
        XCTAssertNotNil(model.setupRequest)
        model.cancelAllSync()
        XCTAssertEqual(model.syncRun?.isFinished, true)
        XCTAssertNil(model.syncingConnectionID)
        model.retryInSyncRun(logins[1].id)
        try await waitForChallenge(of: logins[1])
    }

    /// An import reported by an earlier attempt never counts for a retry of the login.
    func testABrowserImportOfAnEarlierAttemptIsNotCountedForARetry() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.BrowserLoginRequired(message: "Bitte im Browser anmelden"))
        model.webLoginImportedInSyncRun(logins[0].id, imported: 7)
        model.cancelAllSync()
        XCTAssertTrue(model.syncRunBrowserImports.isEmpty)
        try await waitUntil("run over") { model.syncRun?.isFinished == true && model.syncingConnectionID == nil }

        model.retryInSyncRun(logins[0].id)
        try await waitForChallenge(of: logins[0])
        model.finishSync(error: CoreError.BrowserLoginRequired(message: "Bitte im Browser anmelden"))
        model.webLoginConnection = nil   // closed without an import this time
        try await waitUntil("retry ended") { model.syncRun?.isFinished == true }
        XCTAssertEqual(model.syncRun?.status(of: logins[0].id), .skipped(reason: AppModel.browserClosed))
    }

    /// "Alle abbrechen" ends the bank dialog before the window lets another login start:
    /// a retry right after it waits for that, then runs.
    func testRetryRightAfterCancelAllWaitsForTheClosingDialog() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 2, "demo vault needs at least two logins")
        model.requestSyncAll()
        try await waitForChallenge(of: logins[0])
        model.cancelAllSync()
        XCTAssertEqual(model.syncingConnectionID, logins[0].id, "kept until the bank dialog is closed")
        model.retryInSyncRun(logins[1].id)
        XCTAssertEqual(model.syncRun?.status(of: logins[1].id), .waiting, "queued, not started yet")
        try await waitForChallenge(of: logins[1])
        XCTAssertEqual(model.syncRun?.status(of: logins[1].id), .running)
    }

    /// Trade Republic with an expired session and no stored PIN (#1128): the core asks
    /// for the PIN, so the PIN sheet opens (with its browser login) instead of an alert
    /// whose "Erneut versuchen" fails the same way. In a batch it waits for the sheet.
    func testExpiredTradeRepublicSessionWithoutStoredPinOpensThePinSheet() async throws {
        let logins = batch
        let i = try XCTUnwrap(logins.dropLast().firstIndex { $0.connector == .tradeRepublic },
                              "demo vault needs a Trade Republic login that is not the last one")
        XCTAssertFalse(model.connectionStatus(logins[i].id)?.hasPin ?? false)
        model.requestSyncAll()
        for l in logins[..<i] {
            try await waitForChallenge(of: l)
            model.skipCurrentSync()
        }
        try await waitForChallenge(of: logins[i])
        model.failedSyncSecret = ""
        model.finishSync(error: CoreError.Authentication(message: "Sitzung abgelaufen"))
        XCTAssertEqual(model.pendingLogin?.connection.id, logins[i].id)
        XCTAssertNil(model.errorMessage)
        XCTAssertNotNil(model.syncBatchProgress, "the batch waits for the PIN sheet")

        model.cancelPendingLogin()
        try await waitForChallenge(of: logins[i + 1])
        XCTAssertNil(model.errorMessage)
    }

    /// A PIN typed in the sheet (or a stored one) that Trade Republic refuses stays an error.
    func testRefusedTradeRepublicPinStaysAnError() async throws {
        let tr = try XCTUnwrap(model.connections.first { $0.connector == .tradeRepublic })
        model.syncingConnectionID = tr.id
        model.failedSyncSecret = "1234"
        model.finishSync(error: CoreError.Authentication(message: "PIN falsch"))
        XCTAssertNil(model.pendingLogin)
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil

        model.setConnectionPin(tr.id, "1234")
        defer { model.setConnectionPin(tr.id, nil) }
        model.syncingConnectionID = tr.id
        model.finishSync(error: CoreError.Authentication(message: "Sitzung abgelaufen"))
        XCTAssertNil(model.pendingLogin)
        XCTAssertNotNil(model.errorMessage)
    }

    /// Errors answered with a prompt are no failure of the batch: the prompt decides.
    func testErrorsWithAPromptAreNotCountedAsFailures() {
        XCTAssertTrue(AppModel.errorOpensPrompt(CoreError.BrowserLoginRequired(message: "")))
        XCTAssertTrue(AppModel.errorOpensPrompt(CoreError.MissingSetting(key: "iban")))
        XCTAssertFalse(AppModel.errorOpensPrompt(CoreError.Network(message: "")))
        XCTAssertFalse(AppModel.errorOpensPrompt(CoreError.Bank(code: "9050", message: "")))
    }

    /// "Weiter" and right after it "Überspringen": the bank's late answer for
    /// the skipped login must neither skip another login nor end the batch.
    func testLateAnswerOfASkippedLoginIsDropped() async throws {
        let logins = batch
        try XCTSkipIf(logins.count < 3, "demo vault needs at least three logins")
        let codeLogin = try XCTUnwrap(logins.dropLast().firstIndex { $0.connector == .tradeRepublic },
                                      "demo vault needs a code login that is not the last one")
        model.requestSyncAll()
        for c in logins[..<codeLogin] {
            try await waitForChallenge(of: c)
            model.skipCurrentSync()
        }
        try await waitForChallenge(of: logins[codeLogin])

        model.submitCode("1234")
        // let the code reach the bank, then skip while its answer is on the way
        try await waitUntil("code sent") { model.activeChallenge?.busy == true }
        await Task.yield()
        model.skipCurrentSync()

        let next = logins[codeLogin + 1]
        try await waitForChallenge(of: next)
        try await Task.sleep(for: .milliseconds(500))   // room for a late answer
        XCTAssertEqual(model.activeChallenge?.connection.id, next.id)
        XCTAssertEqual(model.syncBatchProgress?.current, codeLogin + 2)
        XCTAssertNil(model.errorMessage)
    }
}
