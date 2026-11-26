import XCTest
@testable import GiroCrabUI

// test: serial-defaults
/// A ⌘A that waited for a new list must not hang when no list comes (natural search, #1696).
@MainActor
final class PendingSelectAllTests: XCTestCase {
    private func loadedModel() throws -> AppModel {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.loadAllBookings()
        XCTAssertFalse(model.hasMoreBookings)
        try XCTSkipIf(model.visibleBookings.count < 2, "demo vault needs at least two bookings")
        return model
    }

    func testPendingSelectAllRunsOnShownListWhenNoReloadFollows() throws {
        let model = try loadedModel()
        model.selectAllAfterReload = true

        model.runPendingSelectAll()

        XCTAssertFalse(model.selectAllAfterReload)
        XCTAssertEqual(model.selectedBookingIDs, Set(model.visibleBookings.map(\.id)))
    }

    func testPendingSelectAllWaitsWhileAReloadRuns() throws {
        let model = try loadedModel()
        model.selectBooking(model.visibleBookings[0].id)
        model.searchTask = Task { try? await Task.sleep(nanoseconds: 5_000_000_000) }
        defer { model.searchTask?.cancel() }
        model.selectAllAfterReload = true

        model.runPendingSelectAll()

        // the new list's `publish` selects: nothing of the old one meanwhile
        XCTAssertTrue(model.selectAllAfterReload)
        XCTAssertEqual(model.selectedBookingIDs.count, 1)
    }

    func testNothingPendingChangesNothing() throws {
        let model = try loadedModel()
        let first = model.visibleBookings[0].id
        model.selectBooking(first)

        model.runPendingSelectAll()

        XCTAssertEqual(model.selectedBookingIDs, [first])
    }
}
