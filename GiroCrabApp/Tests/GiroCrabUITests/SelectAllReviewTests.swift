import XCTest
@testable import GiroCrabUI

// test: serial-defaults
/// Review follow-ups of #1696: a custom period with new bounds ends a multiple selection,
/// unchanged bounds keep it; locking drops a ⌘A that waits for the next list.
@MainActor
final class SelectAllReviewTests: XCTestCase {
    private func demoModel() -> AppModel {
        AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    private func daysAgo(_ n: Int) -> Date {
        Calendar.current.date(byAdding: .day, value: -n, to: Date())!
    }

    func testNewCustomBoundsDropMultiSelection() throws {
        let model = demoModel()
        model.applyCustomPeriod(from: daysAgo(90), to: Date())
        let ids = model.visibleBookings.map(\.id)
        try XCTSkipIf(ids.count < 2, "demo vault has too few bookings in the period")
        model.selectedBookingID = ids[0]
        model.selectedBookingIDs = Set(ids.prefix(2))
        XCTAssertTrue(model.isMultiSelection)

        // same bounds: still the same period, the selection stays
        model.applyCustomPeriod(from: daysAgo(90), to: Date())
        XCTAssertTrue(model.isMultiSelection)

        // other bounds while the period stays `.custom`: a new list, no multiple selection
        model.applyCustomPeriod(from: daysAgo(60), to: Date())
        XCTAssertFalse(model.isMultiSelection)
    }

    func testReleasingVaultDropsPendingSelectAll() {
        let model = demoModel()
        model.selectAllAfterReload = true
        model.releaseVaultState()
        XCTAssertFalse(model.selectAllAfterReload)
        XCTAssertNil(model.shownListFilter)
    }
}
