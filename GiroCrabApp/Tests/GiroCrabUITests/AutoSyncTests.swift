// test: serial-defaults
import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Scheduled balance fetch (GiroCrab/GiroCrab#546): interval, skipping while
/// the vault is locked, catching up after the unlock, and what the menu bar
/// popover may say about it.
final class AutoSyncTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_800_000_000)
    private var unix: Int64 { Int64(now.timeIntervalSince1970) }
    private func hoursAgo(_ h: Double) -> Int64 { unix - Int64(h * 3600) }

    func testOffIsTheDefaultAndNegativeOrMissingMeansOff() {
        XCTAssertEqual(AutoSync.hours(stored: nil), 0)
        XCTAssertEqual(AutoSync.hours(stored: -1), 0)
        XCTAssertEqual(AutoSync.hours(stored: 6), 6)
        XCTAssertEqual(AutoSync.hourOptions.first, 0)
        XCTAssertEqual(AutoSync.hourOptions, [0, 1, 2, 4, 6, 12, 24])
    }

    func testDueOnlyAfterTheIntervalHasPassed() {
        XCTAssertTrue(AutoSync.isDue(lastSyncUnix: nil, lastAttemptUnix: nil, hours: 4, now: now), "never fetched")
        XCTAssertFalse(AutoSync.isDue(lastSyncUnix: hoursAgo(3.9), lastAttemptUnix: nil, hours: 4, now: now))
        XCTAssertTrue(AutoSync.isDue(lastSyncUnix: hoursAgo(4), lastAttemptUnix: nil, hours: 4, now: now))
        XCTAssertFalse(AutoSync.isDue(lastSyncUnix: nil, lastAttemptUnix: nil, hours: 0, now: now), "off is never due")
    }

    func testEveryOfferedIntervalIsHonoured() {
        for h in AutoSync.hourOptions where h > 0 {
            XCTAssertFalse(AutoSync.isDue(lastSyncUnix: hoursAgo(Double(h) - 0.01), lastAttemptUnix: nil, hours: h, now: now))
            XCTAssertTrue(AutoSync.isDue(lastSyncUnix: hoursAgo(Double(h)), lastAttemptUnix: nil, hours: h, now: now))
        }
    }

    func testARecentAttemptDelaysTheNextOneEvenWhenTheLastFetchIsOld() {
        // a login that needs a TAN fails without a fetch: no retry at every tick
        XCTAssertFalse(AutoSync.isDue(lastSyncUnix: hoursAgo(30), lastAttemptUnix: hoursAgo(1), hours: 6, now: now))
        XCTAssertTrue(AutoSync.isDue(lastSyncUnix: hoursAgo(30), lastAttemptUnix: hoursAgo(7), hours: 6, now: now))
        // a manual fetch newer than the last attempt counts as well
        XCTAssertFalse(AutoSync.isDue(lastSyncUnix: hoursAgo(1), lastAttemptUnix: hoursAgo(30), hours: 6, now: now))
    }

    func testLockedVaultSkipsTheFetchAndDoesNotConsumeIt() {
        XCTAssertEqual(AutoSync.decide(hours: 2, isUnlocked: false, lastSyncUnix: hoursAgo(10), lastAttemptUnix: nil, now: now),
                       .skippedLocked)
        // nothing was recorded while locked, so the first tick after the unlock runs it
        XCTAssertEqual(AutoSync.decide(hours: 2, isUnlocked: true, lastSyncUnix: hoursAgo(10), lastAttemptUnix: nil, now: now),
                       .run)
    }

    // MARK: back-off after a demanded approval (#1997)

    /// A bank that demands a fresh approval for every fetch can never be
    /// fetched quietly: the next attempt waits a day instead of logging in at
    /// every interval.
    func testScaBackOffWaitsADayAfterABlockedScheduledFetch() {
        let suite = "AutoSyncTests.sca.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        XCTAssertFalse(AutoSync.scaBackOffActive(connectionID: 7, defaults: defaults, now: now), "nothing blocked yet")
        AutoSync.recordScaBlock(connectionID: 7, defaults: defaults, now: now.addingTimeInterval(-23 * 3600))
        XCTAssertTrue(AutoSync.scaBackOffActive(connectionID: 7, defaults: defaults, now: now), "still within a day")
        AutoSync.recordScaBlock(connectionID: 7, defaults: defaults, now: now.addingTimeInterval(-25 * 3600))
        XCTAssertFalse(AutoSync.scaBackOffActive(connectionID: 7, defaults: defaults, now: now), "a day later it is tried again")
        AutoSync.recordScaBlock(connectionID: 7, defaults: defaults, now: now.addingTimeInterval(-1 * 3600))
        AutoSync.clearScaBlock(connectionID: 7, defaults: defaults)
        XCTAssertFalse(AutoSync.scaBackOffActive(connectionID: 7, defaults: defaults, now: now), "a quiet fetch that worked clears the block")
    }

    func testDecisionOrder() {
        XCTAssertEqual(AutoSync.decide(hours: 0, isUnlocked: true, lastSyncUnix: nil, lastAttemptUnix: nil, now: now), .off)
        // off wins over locked: a disabled interval is not a "skipped" fetch
        XCTAssertEqual(AutoSync.decide(hours: 0, isUnlocked: false, lastSyncUnix: nil, lastAttemptUnix: nil, now: now), .off)
        XCTAssertEqual(AutoSync.decide(hours: 6, isUnlocked: true, lastSyncUnix: hoursAgo(1), lastAttemptUnix: nil, now: now), .notDue)
    }

    func testTheIntervalSurvivesARelaunch() {
        // one name for both instances: the second one is "the next app start"
        let suite = "AutoSyncTests.relaunch.\(UUID().uuidString)"
        UserDefaults(suiteName: suite)!.removePersistentDomain(forName: suite)
        defer { UserDefaults(suiteName: suite)?.removePersistentDomain(forName: suite) }
        UserDefaults(suiteName: suite)!.set(12, forKey: AutoSync.hoursKey)
        // a new UserDefaults instance = the next app start
        let stored = UserDefaults(suiteName: suite)!.object(forKey: AutoSync.hoursKey) as? Int
        XCTAssertEqual(AutoSync.hours(stored: stored), 12)
    }

    // MARK: popover notices

    func testNoticesNeverCarryDetailWhileLockedOrPrivate() {
        let status = AutoSyncStatus(lastRun: now, failureDetail: "Musterbank: Konto DE89370400440532013000 abgelehnt")
        for visibility in [MenuBarSummary.Visibility.locked, .covered] {
            let text = status.notice(visibility, intervalOn: true).joined(separator: " ")
            XCTAssertFalse(text.contains("Musterbank"), "no bank name while \(visibility)")
            XCTAssertFalse(text.contains("DE89"), "no IBAN while \(visibility)")
        }
        XCTAssertTrue(status.notice(.visible, intervalOn: true).joined().contains("Musterbank"))
    }

    func testLockedNoticeOnlyWhenTheIntervalIsOn() {
        let status = AutoSyncStatus()
        XCTAssertEqual(status.notice(.locked, intervalOn: true).count, 1)
        XCTAssertTrue(status.notice(.locked, intervalOn: false).isEmpty)
        XCTAssertTrue(status.notice(.visible, intervalOn: true).isEmpty, "nothing to say without a failure")
        XCTAssertTrue(status.notice(.covered, intervalOn: true).isEmpty)
    }

    func testPrivateNoticeNamesTheFailureWithoutDetail() {
        let status = AutoSyncStatus(lastRun: now, failureDetail: "geheim")
        XCTAssertEqual(status.notice(.covered, intervalOn: false).count, 1)
        XCTAssertFalse(status.notice(.covered, intervalOn: false)[0].contains("geheim"))
    }
}

@MainActor
final class ConnectionAutoSyncTests: XCTestCase {
    func testBackgroundFetchUsesEachLoginsSchedule() async throws {
        let model = AppModel(options: LaunchOptions(arguments: [], environment: [:]))
        model.bankingPolicy = BankingPolicy(publicKeys: [], store: MemoryBankingPolicyStore(),
                                            fetcher: nil, bundled: nil, distribution: .developerID,
                                            bundleVersion: "1")
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let vault = try Vault.open(path: folder.appendingPathComponent("test.girocrab").path,
                                   key: Data(repeating: 0x54, count: 32))
        let kinds: [ConnectorKind] = [.volkswagenBank, .opelDirektbank, .fordMoney, .hanseaticBank]
        let ids = try kinds.map { kind in
            // The demo login routes straight to the simulated connector even outside the demo vault.
            try vault.addBankConnection(connector: kind, displayName: "", loginName: "demo")
        }
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let defaults = AppDefaults.store
        let attemptKeys = ids.map(AutoSync.attemptKey)
        let oldAttempts = attemptKeys.map { defaults.object(forKey: $0) }
        defer {
            model.releaseVaultState()
            for (key, value) in zip(attemptKeys, oldAttempts) { defaults.set(value, forKey: key) }
        }
        model.useVault(vault, demo: true)
        // Allow a manual tick without starting the periodic task or marking this as a personal vault.
        model.demoActive = false
        for (id, schedule) in zip(ids, [AutoSync.never, 0, 6, 2]) {
            XCTAssertTrue(model.setConnectionPin(id, "12345"))
            XCTAssertTrue(model.setSyncSchedule(id, schedule))
        }
        for key in attemptKeys {
            defaults.set(now.timeIntervalSince1970 - 3 * 3600, forKey: key)
        }
        model.autoSyncTick(now: now)
        XCTAssertEqual(model.syncingConnectionID, ids.last)
        XCTAssertEqual(model.quietSyncConnectionID, ids.last)
        XCTAssertEqual(model.autoSyncStatus.lastRun, now)
    }

    /// A sync asked for in the closing Einstellungen sheet (#1831) is the user's, like an open
    /// PIN prompt: no scheduled fetch of another login starts before it.
    func testNoScheduledFetchWhileASyncWaitsForSettingsToClose() throws {
        let model = AppModel(options: LaunchOptions(arguments: [], environment: [:]))
        model.bankingPolicy = BankingPolicy(publicKeys: [], store: MemoryBankingPolicyStore(),
                                            fetcher: nil, bundled: nil, distribution: .developerID,
                                            bundleVersion: "1")
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let vault = try Vault.open(path: folder.appendingPathComponent("test.girocrab").path,
                                   key: Data(repeating: 0x54, count: 32))
        let asked = try vault.addBankConnection(connector: .fordMoney, displayName: "", loginName: "demo")
        let scheduled = try vault.addBankConnection(connector: .hanseaticBank, displayName: "", loginName: "demo")
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let key = AutoSync.attemptKey(scheduled)
        let oldAttempt = AppDefaults.store.object(forKey: key)
        defer {
            model.releaseVaultState()
            AppDefaults.store.set(oldAttempt, forKey: key)
        }
        model.useVault(vault, demo: true)
        model.demoActive = false
        for id in [asked, scheduled] { XCTAssertTrue(model.setConnectionPin(id, "12345")) }
        XCTAssertTrue(model.setSyncSchedule(asked, AutoSync.never))
        XCTAssertTrue(model.setSyncSchedule(scheduled, 2))
        AppDefaults.store.set(now.timeIntervalSince1970 - 3 * 3600, forKey: key)

        model.showSettings = true
        model.requestSync(try XCTUnwrap(model.connections.first { $0.id == asked }))
        model.autoSyncTick(now: now)
        XCTAssertNil(model.syncingConnectionID)
        XCTAssertNil(model.quietSyncConnectionID)

        model.settingsDismissed()
        XCTAssertEqual(model.syncingConnectionID, asked)
        XCTAssertNil(model.quietSyncConnectionID)
    }

    /// A scheduled fetch that failed on a demanded approval backs off a day
    /// (#1997): no further login attempt at every tick, and the connection
    /// view says why nothing runs automatically.
    func testAConnectionBlockedByADemandedApprovalWaitsADay() throws {
        let model = AppModel(options: LaunchOptions(arguments: [], environment: [:]))
        model.bankingPolicy = BankingPolicy(publicKeys: [], store: MemoryBankingPolicyStore(),
                                            fetcher: nil, bundled: nil, distribution: .developerID,
                                            bundleVersion: "1")
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let vault = try Vault.open(path: folder.appendingPathComponent("test.girocrab").path,
                                   key: Data(repeating: 0x54, count: 32))
        let id = try vault.addBankConnection(connector: .hanseaticBank, displayName: "", loginName: "demo")
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let attemptKey = AutoSync.attemptKey(id)
        let scaKey = AutoSync.scaBackOffKey(id)
        let oldAttempt = AppDefaults.store.object(forKey: attemptKey)
        let oldSca = AppDefaults.store.object(forKey: scaKey)
        defer {
            model.releaseVaultState()
            AppDefaults.store.set(oldAttempt, forKey: attemptKey)
            AppDefaults.store.set(oldSca, forKey: scaKey)
        }
        model.useVault(vault, demo: true)
        model.demoActive = false
        XCTAssertTrue(model.setConnectionPin(id, "12345"))
        XCTAssertTrue(model.setSyncSchedule(id, 1))
        AppDefaults.store.set(now.timeIntervalSince1970 - 2 * 3600, forKey: attemptKey)

        // the last scheduled fetch was blocked by a demanded approval: no new attempt
        AutoSync.recordScaBlock(connectionID: id, defaults: AppDefaults.store, now: now.addingTimeInterval(-1 * 3600))
        model.autoSyncTick(now: now)
        XCTAssertNil(model.syncingConnectionID)
        XCTAssertNil(model.quietSyncConnectionID)
        let hint = model.autoSyncHint(try XCTUnwrap(model.connections.first { $0.id == id }), now: now)
        XCTAssertTrue(hint?.contains("Freigabe") == true, "\(hint ?? "nil")")

        // without the block the tick fetches it
        AutoSync.clearScaBlock(connectionID: id, defaults: AppDefaults.store)
        model.autoSyncTick(now: now)
        XCTAssertEqual(model.syncingConnectionID, id)
        XCTAssertEqual(model.quietSyncConnectionID, id)
        // nothing blocks it any more: no hint
        XCTAssertFalse(model.autoSyncHint(try XCTUnwrap(model.connections.first { $0.id == id }), now: now)?.contains("Freigabe") == true)
    }

    func testAQuietChallengeOfAnotherConnectorRecordsNoBackOffAndAManualSuccessClearsIt() throws {
        let model = AppModel(options: LaunchOptions(arguments: [], environment: [:]))
        model.bankingPolicy = BankingPolicy(publicKeys: [], store: MemoryBankingPolicyStore(),
                                            fetcher: nil, bundled: nil, distribution: .developerID,
                                            bundleVersion: "1")
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: false)
        defer { try? FileManager.default.removeItem(at: folder) }
        let vault = try Vault.open(path: folder.appendingPathComponent("test.girocrab").path,
                                   key: Data(repeating: 0x54, count: 32))
        let id = try vault.addBankConnection(connector: .hanseaticBank, displayName: "", loginName: "demo")
        let attemptKey = AutoSync.attemptKey(id)
        let scaKey = AutoSync.scaBackOffKey(id)
        let oldAttempt = AppDefaults.store.object(forKey: attemptKey)
        let oldSca = AppDefaults.store.object(forKey: scaKey)
        defer {
            model.releaseVaultState()
            AppDefaults.store.set(oldAttempt, forKey: attemptKey)
            AppDefaults.store.set(oldSca, forKey: scaKey)
        }
        model.useVault(vault, demo: true)
        model.demoActive = false
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let connection = try XCTUnwrap(model.connections.first { $0.id == id })
        let challenge = Challenge(kind: .decoupledPush, title: "Freigabe", message: "Bitte freigeben",
                                  methodName: "App", redirectUrl: nil, image: nil,
                                  expiresInSeconds: nil, pollIntervalSeconds: 2)

        // A scheduled fetch of a connector that is no FinTS login stops at its
        // challenge too — but the approval back-off stays a FinTS matter (#1997).
        model.quietSyncConnectionID = id
        model.handle(.needsChallenge(challenge: challenge), connection: connection)
        XCTAssertFalse(AutoSync.scaBackOffActive(connectionID: id, defaults: AppDefaults.store, now: now))

        // A manual sync that went through ends the back-off: the bank allowed
        // the fetch again, the user saw and answered what the bank wanted.
        AutoSync.recordScaBlock(connectionID: id, defaults: AppDefaults.store, now: now)
        model.handle(.finished(summary: SyncSummary(accountsUpdated: 1, transactionsImported: 0,
                                                    transactionsCategorized: 0, transactionsSkippedArchived: 0,
                                                    newAccounts: [], revokedAccounts: [], postboxDocuments: nil,
                                                    warnings: [], hints: [])), connection: connection)
        XCTAssertFalse(AutoSync.scaBackOffActive(connectionID: id, defaults: AppDefaults.store, now: now))
    }
}
