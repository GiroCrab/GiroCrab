import XCTest
@testable import GiroCrabUI

/// Sidebar categories of a booking filter (#1896): a filter change must not keep the previous set.
@MainActor
final class ScopeCategoriesTests: XCTestCase {
    private func demoModel() -> AppModel {
        AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    func testFilterChangeDropsPreviousCategoriesUntilLoaded() {
        let model = demoModel()
        // stands in for the set of the filter shown before
        model.scopeCategoryIDs = [-1]
        model.scope = .investments
        // the new set loads off the main thread; until then nothing of the old filter is listed
        XCTAssertNil(model.scopeCategoryIDs)
    }

    func testSameFilterKeepsLoadedCategories() {
        let model = demoModel()
        model.scope = .investments
        model.scopeCategoryIDs = [-1]
        model.scope = .investments
        XCTAssertEqual(model.scopeCategoryIDs, [-1])
    }
}
