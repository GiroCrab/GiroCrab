import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The list loads one page (whole days) and appends further pages in the background.
@MainActor
final class BookingPagingTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        XCTAssertNil(model.errorMessage)
    }

    override func tearDown() async throws { model = nil }

    /// Everything matching the current filter, as the core orders it.
    private func allMatching() throws -> [Booking] {
        var filter = model.currentFilter
        filter.limit = 0
        return try XCTUnwrap(model.vault).listTransactions(filter: filter).transactions
    }

    private func loadFirstPage(of size: UInt32) {
        model.appendPageSize = 7
        model.listLimit = size
        model.reloadBookings()
    }

    private func awaitCount(above count: Int) async throws {
        for _ in 0..<300 {
            if model.bookings.count > count, !model.isLoadingMoreBookings { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("The next page did not arrive")
    }

    func testFirstPageEndsOnAWholeDay() throws {
        let all = try allMatching()
        XCTAssertGreaterThan(all.count, 40)
        loadFirstPage(of: 20)
        XCTAssertGreaterThanOrEqual(model.bookings.count, 20)
        XCTAssertLessThan(model.bookings.count, all.count)
        XCTAssertEqual(model.bookings.map(\.id), Array(all.prefix(model.bookings.count)).map(\.id))
        XCTAssertNotEqual(all[model.bookings.count].bookingDate, model.bookings.last?.bookingDate, "a day is never split")
        XCTAssertTrue(model.hasMoreBookings)
        XCTAssertEqual(model.bookingCountLabel, all.count)
        // each day card sums the whole day
        for day in model.dayGroups {
            XCTAssertEqual(day.sum, all.filter { $0.bookingDate == day.date }.reduce(0) { $0 + $1.amountCents }, day.date)
        }
    }

    func testLoadMoreAppendsPagesUntilEverythingIsLoaded() async throws {
        let all = try allMatching()
        loadFirstPage(of: 20)
        var rounds = 0
        while model.hasMoreBookings {
            let before = model.bookings.count
            model.loadMoreBookings()
            XCTAssertTrue(model.isLoadingMoreBookings)
            XCTAssertEqual(model.bookings.count, before, "the page arrives in the background")
            try await awaitCount(above: before)
            XCTAssertEqual(model.listLimit, UInt32(model.bookings.count))
            rounds += 1
            XCTAssertLessThan(rounds, 100)
        }
        XCTAssertGreaterThan(rounds, 1, "several pages were appended")
        XCTAssertEqual(model.bookings.map(\.id), all.map(\.id), "same order, nothing twice, nothing missing")
        XCTAssertEqual(model.dayGroups.flatMap { $0.bookings.map(\.id) }, all.map(\.id))
        model.loadMoreBookings()
        XCTAssertFalse(model.isLoadingMoreBookings, "nothing left to load")
    }

    func testAnEditReloadsWhatIsOnScreen() async throws {
        loadFirstPage(of: 20)
        let first = model.bookings.count
        model.loadMoreBookings()
        try await awaitCount(above: first)
        let loaded = model.bookings.map(\.id)
        let booking = try XCTUnwrap(model.bookings.last)
        model.setChecked(!booking.checked, for: booking)
        XCTAssertEqual(model.bookings.map(\.id), loaded, "the list keeps its length after an edit")
        XCTAssertEqual(model.bookings.last?.checked, !booking.checked)
    }

    /// ⌘A selects every booking matching the filter, not only the loaded pages (#1696).
    func testSelectAllLoadsAndSelectsEveryBooking() async throws {
        let all = try allMatching()
        loadFirstPage(of: 20)
        XCTAssertTrue(model.hasMoreBookings)
        model.selectAllBookings()
        XCTAssertTrue(model.isSelectingAllBookings)
        XCTAssertFalse(model.isMultiSelection, "no partial selection while the rest loads")
        try await awaitSelectAll()
        XCTAssertFalse(model.hasMoreBookings)
        XCTAssertEqual(model.bookings.map(\.id), all.map(\.id))
        XCTAssertEqual(model.selectedBookingIDs, Set(all.map(\.id)))
        XCTAssertEqual(model.selectedBookingIDs.count, model.bookingCountLabel)
        // the sum and every action on the selection cover all bookings
        let sums = Dictionary(grouping: all, by: \.currency).mapValues { $0.reduce(Int64(0)) { $0 + $1.amountCents } }
        XCTAssertEqual(Dictionary(uniqueKeysWithValues: model.selectionTotals.map { ($0.currency, $0.cents) }), sums)
        let first = try XCTUnwrap(model.bookings.first)
        XCTAssertEqual(Set(model.categorizeTargets(for: first).map(\.id)), Set(all.map(\.id)))
        // an edit keeps the whole list loaded
        model.setChecked(!first.checked, for: first)
        XCTAssertEqual(model.bookings.count, all.count)
    }

    func testAFullReloadCancelsSelectAll() async throws {
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        loadFirstPage(of: 20)
        model.selectAllBookings()
        XCTAssertTrue(model.isSelectingAllBookings)
        model.sidebarSelection = .account(account.id) // synchronous full reload
        XCTAssertFalse(model.isSelectingAllBookings)
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertFalse(model.isMultiSelection, "the old list's bookings are not selected")
    }

    private func awaitSelectAll() async throws {
        for _ in 0..<300 where model.isSelectingAllBookings {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertFalse(model.isSelectingAllBookings)
    }

    func testANewerSelectionSupersedesSelectAll() async throws {
        loadFirstPage(of: 20)
        let row = try XCTUnwrap(model.bookings.dropFirst().first)
        model.selectAllBookings()
        XCTAssertTrue(model.isSelectingAllBookings)
        model.selectBooking(row.id) // click while the rest loads
        XCTAssertFalse(model.isSelectingAllBookings)
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(model.selectedBookingIDs, [row.id], "the click is not overwritten")
    }

    /// A checked row kept in place in „Neu“ does not match the filter: the next page starts
    /// at the right booking and ⌘A selects exactly the matching ones.
    func testSelectAllSkipsTheLingeringRowAndNoMatchingBooking() async throws {
        model.statusFilter = .new
        loadFirstPage(of: 5)
        XCTAssertTrue(model.hasMoreBookings)
        let booking = try XCTUnwrap(model.bookings.first)
        model.selectedBookingID = booking.id
        model.setChecked(true, for: booking)
        XCTAssertTrue(model.bookings.contains { $0.id == booking.id }, "the row lingers")
        let matching = try allMatching()
        XCTAssertFalse(matching.contains { $0.id == booking.id })
        XCTAssertGreaterThan(matching.count, model.loadedMatchingCount)
        model.selectAllBookings()
        try await awaitSelectAll()
        XCTAssertEqual(model.selectedBookingIDs, Set(matching.map(\.id)))
        XCTAssertFalse(model.bookings.contains { $0.id == booking.id }, "the selection moved on")
        XCTAssertEqual(model.bookings.map(\.id), matching.map(\.id), "nothing skipped, nothing twice")
    }

    func testAFullReloadDropsAPageStillOnItsWay() async throws {
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        loadFirstPage(of: 20)
        model.loadMoreBookings()
        XCTAssertTrue(model.isLoadingMoreBookings)
        model.sidebarSelection = .account(account.id) // synchronous full reload
        XCTAssertFalse(model.isLoadingMoreBookings)
        let expected = model.bookings.map(\.id)
        try await Task.sleep(for: .milliseconds(200))
        XCTAssertEqual(model.bookings.map(\.id), expected, "the old list's page is not appended")
        XCTAssertTrue(model.bookings.allSatisfy { $0.accountId == account.id })
    }

    func testStatusFilterIsAppliedByTheCore() throws {
        let all = try allMatching()
        let new = all.filter { !$0.checked }
        XCTAssertFalse(new.isEmpty)
        model.statusFilter = .new
        XCTAssertEqual(model.currentFilter.checked, false)
        XCTAssertTrue(model.bookings.allSatisfy { !$0.checked })
        XCTAssertEqual(model.bookingCountLabel, new.count, "the count covers all pages")
        XCTAssertEqual(model.bookings.map(\.id), Array(new.prefix(model.bookings.count)).map(\.id))

        // checking the selected booking: its row stays until the selection moves on
        let booking = try XCTUnwrap(model.bookings.first)
        let next = try XCTUnwrap(model.bookings.dropFirst().first)
        model.selectedBookingID = booking.id
        model.setChecked(true, for: booking)
        XCTAssertTrue(model.bookings.contains { $0.id == booking.id })
        XCTAssertEqual(model.selectedBooking?.id, booking.id)
        XCTAssertEqual(model.selectedBooking?.checked, true)
        model.selectedBookingID = next.id
        XCTAssertFalse(model.bookings.contains { $0.id == booking.id })

        model.statusFilter = .checked
        XCTAssertTrue(model.bookings.allSatisfy(\.checked))
        XCTAssertTrue(model.bookings.contains { $0.id == booking.id })
        model.statusFilter = .all
        XCTAssertNil(model.currentFilter.checked)
        XCTAssertEqual(model.bookingCountLabel, all.count)
    }

    func testRevealLoadsBackToAnOlderBooking() async throws {
        let all = try allMatching()
        let oldest = try XCTUnwrap(all.last)
        loadFirstPage(of: 20)
        XCTAssertFalse(model.bookings.contains { $0.id == oldest.id })
        model.revealBooking(oldest.id)
        for _ in 0..<200 where model.selectedBookingID != oldest.id {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(model.selectedBookingID, oldest.id)
        XCTAssertEqual(model.selectedBooking?.id, oldest.id, "the booking is loaded")
        XCTAssertTrue(model.dayGroups.contains { $0.date == oldest.bookingDate })
    }
}
