import Foundation
import SwiftUI
import GiroCrabCore

extension AppModel {
    // MARK: - Recurring bookings (GitHub #35)

    func refreshRecurring() {
        settlePendingSelectionLoad()
        guard let vault else { recurring = []; recurringMonth = nil; return }
        let ids = selectionAccountIDs
        let c = Calendar(identifier: .gregorian).dateComponents([.year, .month], from: Date())
        runCore {
            let snapshot = try vault.recurringSnapshot(accountIds: ids, year: Int32(c.year ?? 2026), month: UInt32(c.month ?? 1))
            recurring = snapshot.series
            recurringHidden = snapshot.ignored
            recurringMonth = snapshot.month
        }
        loadSubscriptionData()
    }

    /// The recurring series a booking belongs to, if any.
    func recurringSeries(for b: Booking) -> RecurringSeries? {
        guard !b.recurrenceKey.isEmpty else { return nil }
        return recurringByKey[b.recurrenceKey]?.first { s in
            (!s.amountSpecific || abs(b.amountCents - s.amountCents) * 100 <= abs(s.amountCents) * 15)
        }
    }

    func markRecurring(_ b: Booking, _ interval: RecurrenceInterval, customDates: [String] = []) {
        guard let vault else { return }
        let before = recurringSeries(for: b)
        runCore { try vault.markBookingRecurring(bookingId: b.id, interval: interval, customDates: customDates) }
        refreshRecurring()
        if let before { carrySubscription(from: before) }
    }

    /// Changes the interval of a whole series (row context menu in the recurring list);
    /// the series' latest booking carries the manual mark.
    func setRecurringInterval(_ s: RecurringSeries, _ interval: RecurrenceInterval) {
        guard let vault, s.interval != interval else { return }
        runCore { try vault.markBookingRecurring(bookingId: s.lastBookingId, interval: interval, customDates: []) }
        refreshRecurring()
        carrySubscription(from: s)
    }

    /// "Nicht wiederkehrend" for a detected or marked series.
    func ignoreRecurring(_ s: RecurringSeries) {
        guard let vault else { return }
        runCore { try vault.ignoreRecurring(seriesKey: s.seriesKey) }
        refreshRecurring()
    }

    /// Back to automatic detection (also shows a hidden series again).
    func resetRecurring(_ s: RecurringSeries) { resetRecurring(key: s.seriesKey) }

    func resetRecurring(key: String) {
        guard let vault else { return }
        runCore { try vault.resetRecurring(seriesKey: key) }
        refreshRecurring()
    }

    /// The latest `revealBooking` call: an older jump still waiting gives way to a newer one.
    @MainActor private static var revealRequest = 0

    /// Selects a booking in the list, widening the period if it is outside of it.
    /// With `account` the booking is shown in that account with every filter cleared (from the
    /// subscription list, whose column is not a booking list); without, only when it is not visible yet.
    func revealBooking(_ id: Int64, inAccount account: Int64? = nil) {
        guard let pin = pinVault() else { return }
        AppModel.revealRequest &+= 1
        let request = AppModel.revealRequest
        if let account {
            sidebarSelection = .account(account)
            period = .all
            directionFilter = .all
            statusFilter = .all
            tagFilter = BookingTagFilter()
            amountFilter = AmountFilter()
            searchText = ""
        } else if booking(withID: id) == nil {
            sidebarSelection = .allAccounts
            period = .all
            scope = .all
            resetListFilters(inBackground: true)
        }
        let generation = listGeneration
        // After a closing sheet / list reload, so the list can scroll to the row.
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 350_000_000)
            guard let self, !Task.isCancelled, isCurrent(pin), request == AppModel.revealRequest else { return }
            // an older booking is not on the first page: load back to its day. Only this path
            // depends on the list generation; a loaded target is selected even when a load that
            // started before the jump finished meanwhile.
            if booking(withID: id) == nil {
                guard generation == listGeneration else { return }
                let filter = currentFilter
                do {
                    let count = try await pin.run { vault -> UInt32? in
                        guard let day = try vault.bookingDate(bookingId: id) else { return nil }
                        var narrowed = filter
                        if narrowed.dateFrom.map({ $0 < day }) ?? true { narrowed.dateFrom = day }
                        return try vault.countTransactions(filter: narrowed)
                    }
                    guard !Task.isCancelled, isCurrent(pin), request == AppModel.revealRequest,
                          generation == listGeneration, let count else { return }
                    listLimit = max(listLimit, count)
                    startSelectionLoad()
                    let loadGeneration = listGeneration
                    await selectionLoadTask?.value
                    guard !Task.isCancelled, isCurrent(pin), request == AppModel.revealRequest,
                          loadGeneration == listGeneration else { return }
                } catch {
                    if isCurrent(pin), request == AppModel.revealRequest, generation == listGeneration {
                        errorMessage = describe(error)
                    }
                    return
                }
            }
            guard booking(withID: id) != nil else { return }
            if selectedBookingID == id { selectedBookingID = nil }
            selectedBookingID = id
        }
    }
}
