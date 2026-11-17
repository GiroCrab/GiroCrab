import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The booking list shows count and sum of a manual multiple selection.
@MainActor
final class SelectionSummaryTests: XCTestCase {
    func testSelectedTotalAddsUpTheSelectedBookings() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.statusFilter = .all
        let list = Array(model.visibleBookings.prefix(4))
        XCTAssertEqual(list.count, 4)
        model.selectedBookingIDs = Set(list.map(\.id))
        model.selectedBookingID = list.first?.id
        XCTAssertTrue(model.isMultiSelection)
        XCTAssertEqual(model.selectedBookings.count, 4)
        XCTAssertEqual(model.selectedTotalCents, list.reduce(Int64(0)) { $0 + $1.amountCents })

        model.selectedBookingID = list.first?.id
        model.selectedBookingIDs = [list[0].id]
        XCTAssertFalse(model.isMultiSelection)
        XCTAssertEqual(model.selectedTotalCents, list[0].amountCents)
    }
}
