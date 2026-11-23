import XCTest
@testable import GiroCrabUI

/// The bank logins' own Enable Banking applications via the iCloud Keychain
/// (#962): which side wins per login (`EnableBankingLoginSync.reconcile`).
final class EnableBankingLoginSyncTests: XCTestCase {
    private typealias Sync = EnableBankingLoginSync

    func testNothingSharedAndDefaultHereDoesNothing() {
        XCTAssertEqual(Sync.reconcile(here: "", synced: nil, store: nil), .none)
    }

    func testOwnApplicationNotSharedYetUploads() {
        XCTAssertEqual(Sync.reconcile(here: "app-b", synced: nil, store: nil), .upload("app-b"))
    }

    /// The repro of #962: Bank 2 got B on the Mac, the iPhone still uses the default.
    func testDefaultHereTakesOverOtherDevicesApplication() {
        XCTAssertEqual(Sync.reconcile(here: "", synced: nil, store: "app-b"), .adopt("app-b"))
    }

    func testSameOnBothSidesOnlyRemembersIt() {
        XCTAssertEqual(Sync.reconcile(here: "app-b", synced: nil, store: "app-b"), .markSynced("app-b"))
        XCTAssertEqual(Sync.reconcile(here: "app-b", synced: "app-b", store: "app-b"), .none)
    }

    func testChangeOnOtherDeviceWins() {
        XCTAssertEqual(Sync.reconcile(here: "app-b", synced: "app-b", store: "app-c"), .adopt("app-c"))
    }

    func testResetToDefaultOnOtherDeviceArrives() {
        XCTAssertEqual(Sync.reconcile(here: "app-b", synced: "app-b", store: ""), .adopt(""))
    }

    func testChangeHereIsShared() {
        XCTAssertEqual(Sync.reconcile(here: "app-c", synced: "app-b", store: "app-b"), .upload("app-c"))
        // back to the default here: shared as "" so the other devices follow
        XCTAssertEqual(Sync.reconcile(here: "", synced: "app-b", store: "app-b"), .upload(""))
    }

    func testChangeOnBothSidesIsNeverSilentlyReplaced() {
        XCTAssertEqual(Sync.reconcile(here: "app-c", synced: "app-b", store: "app-d"), .conflict)
        // own application here (e.g. pinned) never shared, another one in the store
        XCTAssertEqual(Sync.reconcile(here: "app-a", synced: nil, store: "app-b"), .conflict)
    }

    func testWhitespaceDoesNotCountAsChange() {
        XCTAssertEqual(Sync.reconcile(here: " app-b\n", synced: "app-b", store: "app-b "), .none)
    }

    func testShareOnlyNeverTakesOver() {
        XCTAssertEqual(Sync.filter(.adopt("app-c"), here: "app-b", forced: false, adopting: false), .none)
        XCTAssertEqual(Sync.filter(.conflict, here: "app-b", forced: false, adopting: false), .none)
        XCTAssertEqual(Sync.filter(.upload("app-b"), here: "app-b", forced: false, adopting: false), .upload("app-b"))
    }

    func testSavedHereWinsOverOtherDevice() {
        XCTAssertEqual(Sync.filter(.conflict, here: "app-c", forced: true, adopting: false), .upload("app-c"))
        XCTAssertEqual(Sync.filter(.adopt("app-d"), here: "app-c", forced: true, adopting: false), .upload("app-c"))
        XCTAssertEqual(Sync.filter(.none, here: "app-c", forced: true, adopting: false), .none)
    }

    func testOpenKeepsEveryAction() {
        XCTAssertEqual(Sync.filter(.adopt("app-c"), here: "", forced: false, adopting: true), .adopt("app-c"))
        XCTAssertEqual(Sync.filter(.conflict, here: "app-b", forced: false, adopting: true), .conflict)
    }

    func testNoticeNamesBanksAndShortIDsNeverKeys() throws {
        let notice = try XCTUnwrap(Sync.notice(assigned: [("Bank 2", "0f3c9a71-2b4e-4d1a-9c55-7e8f00a1b2c3"), ("Bank 3", "")],
                                               conflicts: ["Bank 4"]))
        XCTAssertTrue(notice.message.contains("Bank 2"))
        XCTAssertTrue(notice.message.contains("0f3c9a71…"))
        XCTAssertFalse(notice.message.contains("2b4e"))
        XCTAssertTrue(notice.message.contains("Bank 3"))
        XCTAssertTrue(notice.message.contains("Bank 4"))
    }

    func testNoNoticeWhenNothingHappened() {
        XCTAssertNil(Sync.notice(assigned: [], conflicts: []))
    }

    /// Lock/reset closes the gate: no upload writes after it.
    func testClosedUploadGateSkipsWrites() {
        let gate = EnableBankingUploadGate()
        var writes = 0
        XCTAssertTrue(gate.run { writes += 1 })
        gate.close()
        XCTAssertFalse(gate.run { writes += 1 })
        XCTAssertEqual(writes, 1)
    }

    /// The demo vault never shares (or takes over) a login's application.
    @MainActor
    func testDemoVaultNeverSharesLogins() async throws {
        final class Recorder: EnableBankingLoginStore {
            var loads = 0
            var saved: [[String: String]] = []
            func loadApp(id: String) -> EnableBankingApp? { nil }
            func saveApp(_ app: EnableBankingApp) throws {}
            func loadLogins() -> [String: String]? { loads += 1; return [:] }
            func saveLogin(syncKey: String, applicationID: String) throws { saved.append([syncKey: applicationID]) }
        }
        let wasOn = VaultKeyManager.shared.syncViaICloudKeychain
        VaultKeyManager.shared.syncViaICloudKeychain = true
        defer { VaultKeyManager.shared.syncViaICloudKeychain = wasOn }
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertTrue(model.isDemo)
        let store = Recorder()
        model.enableBankingLoginStore = store
        model.syncEnableBankingLogins(.open)
        await Task.yield()
        XCTAssertEqual(store.loads, 0)
        XCTAssertTrue(store.saved.isEmpty)
    }
}
