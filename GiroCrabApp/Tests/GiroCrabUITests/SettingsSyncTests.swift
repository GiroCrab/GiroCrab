// test: serial-defaults
import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// iOS: a sync started in Einstellungen (a sheet over the main view) closes the
/// sheet first, so its PIN/TAN prompts and errors can appear (#1831).
@MainActor
final class SettingsSyncTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws {
        model.cancelAllSync()
        model = nil
    }

    /// A demo login that syncs in the app (asks for a TAN), not in a portal or by file.
    private func login() throws -> BankConnection {
        guard let c = model.connections.first(where: { !model.usesPortal($0) && !model.usesFileImport($0) }) else {
            throw XCTSkip("demo vault needs a login that syncs in the app")
        }
        return c
    }

    private func waitUntil(_ what: String, timeout: Double = 5, _ condition: () -> Bool) async throws {
        let end = Date().addingTimeInterval(timeout)
        while !condition() {
            if Date() > end { XCTFail("Timed out waiting for \(what)"); return }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    func testSyncFromSettingsClosesThemAndStartsOnceTheyAreGone() async throws {
        let c = try login()
        model.showSettings = true
        model.requestFullSync(c)
        XCTAssertFalse(model.showSettings)
        XCTAssertNil(model.syncingConnectionID, "not while the sheet is still on screen")

        model.settingsDismissed()
        try await waitUntil("TAN prompt of \(c.displayName)") { model.activeChallenge?.connection.id == c.id }
    }

    func testClosingSettingsWithoutASyncStartsNothing() async throws {
        model.showSettings = true
        model.showSettings = false   // closed by the user
        model.settingsDismissed()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNil(model.syncingConnectionID)
        XCTAssertNil(model.activeChallenge)
    }

    func testSyncOutsideSettingsStartsRightAway() async throws {
        let c = try login()
        model.requestFullSync(c)
        try await waitUntil("TAN prompt of \(c.displayName)") { model.activeChallenge?.connection.id == c.id }
        model.settingsDismissed()   // a later close of Einstellungen starts no second sync
        XCTAssertEqual(model.syncingConnectionID, c.id)
    }

    func testLockedMeanwhileStartsNothing() async throws {
        let c = try login()
        model.showSettings = true
        model.requestFullSync(c)
        model.releaseVaultState()   // what locking does
        model.settingsDismissed()
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertNil(model.syncingConnectionID)
        XCTAssertNil(model.activeChallenge)
    }
}
