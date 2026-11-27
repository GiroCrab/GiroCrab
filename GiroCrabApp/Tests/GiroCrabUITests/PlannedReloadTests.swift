import XCTest
@testable import GiroCrabUI

/// #1745: a sync or account fetch must make an open „Geplant“ page reload
/// (its `.task` is keyed on `plannedVersion`).
@MainActor
final class PlannedReloadTests: XCTestCase {
    private func demoModel() -> AppModel {
        AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    func testBackgroundReloadAfterSyncReloadsPlanned() {
        let model = demoModel()
        XCTAssertTrue(model.isUnlocked)
        let before = model.plannedVersion
        model.reloadAllInBackground()
        XCTAssertGreaterThan(model.plannedVersion, before)
    }

    func testReloadAllAfterAccountFetchReloadsPlanned() {
        let model = demoModel()
        XCTAssertTrue(model.isUnlocked)
        let before = model.plannedVersion
        model.reloadAll()
        XCTAssertGreaterThan(model.plannedVersion, before)
    }
}
