// test: serial-defaults (shares UserDefaults.standard across xctest processes)
import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Per-category default period (#425).
@MainActor
final class CategoryPeriodTests: XCTestCase {
    private let key = "period.byCategory"

    override func setUp() { AppDefaults.store.removeObject(forKey: key) }
    override func tearDown() { AppDefaults.store.removeObject(forKey: key) }

    func testStorageKeyRoundTrip() {
        let now = Date()
        let menu = Period.menu(now: now).flatMap { $0 } + [.days(7)]
        for p in menu {
            let k = try? XCTUnwrap(p.storageKey(now: now))
            XCTAssertEqual(k.flatMap { Period(storageKey: $0, now: now) }, p)
        }
        XCTAssertNil(Period.custom.storageKey())
        XCTAssertNil(Period(storageKey: "year:9"))
        XCTAssertNil(Period(storageKey: "bogus"))
    }

    /// The period belongs to the filter set (#1469): the unpinned categories share one, the account views have
    /// their own, a pinned category keeps its own.
    func testPeriodBelongsToTheFilterSet() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let ids = model.categories.map(\.id)
        let (a, b) = (try XCTUnwrap(ids.first), try XCTUnwrap(ids.dropFirst().first))
        let year = Period.menu()[4][0]

        model.sidebarSelection = .allAccounts
        model.interactivePeriod = .months12
        model.sidebarSelection = .category(a)
        XCTAssertEqual(model.period, .all, "the category views do not take over the account's period")
        model.interactivePeriod = year
        model.sidebarSelection = .category(b)
        XCTAssertEqual(model.period, year, "unpinned categories share the global period")
        model.toggleCategoryPin()
        model.interactivePeriod = .days(7)
        model.sidebarSelection = .category(a)
        XCTAssertEqual(model.period, year, "the pinned category's period stays with it")
        model.sidebarSelection = .allAccounts
        XCTAssertEqual(model.period, .months12, "the account views keep their own period")
        model.sidebarSelection = .category(b)
        XCTAssertEqual(model.period, .days(7), "the pin keeps the period of its category")
        model.sidebarSelection = .category(a)
        XCTAssertEqual(model.period, .all, "the global filters were reset in the account view")
    }
}
