import XCTest
import GiroCrabCore
#if os(macOS)
import AppKit
#endif
@testable import GiroCrabUI

@MainActor
final class RefundLinkTests: XCTestCase {
    private var model: AppModel!
    private var refund: Int64 = 0
    private var original: Int64 = 0
    private var second: Int64 = 0
    private var bookings: [Int64: Booking] = [:]

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let ids = try await model.requireVaultOffMain { vault in
            let account = try vault.createOfflineAccount(name: "Refund test", kind: .checking, currency: "EUR", openingCents: 0)
            func booking(_ cents: Int64) throws -> Int64 {
                try vault.saveManualBooking(accountId: account, bookingId: nil,
                    setup: ManualBookingSetup(bookingDate: "2026-04-01", amountCents: cents, counterparty: "Test",
                        purpose: "REF-TEST-714", categoryId: nil, note: ""))
            }
            let ids = try (booking(10000), booking(-6000), booking(-7000))
            let filter = BookingFilter(accountIds: [account], categoryId: nil, includeSubcategories: true,
                uncategorizedOnly: false, search: nil, dateFrom: "2026-04-01", dateTo: "2026-04-01",
                limit: 0, offset: 0, scope: .all, merchantKey: nil, direction: .all, checked: nil, wholeDays: false)
            return (ids, try vault.listTransactions(filter: filter).transactions)
        }
        (refund, original, second) = ids.0
        bookings = Dictionary(uniqueKeysWithValues: ids.1.map { ($0.id, $0) })
    }

    override func tearDown() async throws { model = nil }

    private func pair(_ originalID: Int64? = nil, part: String = "") -> RefundEndpointPair {
        RefundEndpointPair(bookingID: refund, ownPart: part, paysBack: true,
            candidate: candidate(originalID ?? original, cents: -6000))
    }

    private func candidate(_ id: Int64, part: String = "", cents: Int64, open: Int64 = 6000) -> RefundCandidate {
        RefundCandidate(transactionId: id, part: part, partLabel: "", accountId: 0, bookingDate: "2026-04-01",
            counterparty: "Test", merchantName: "", amountCents: cents, openCents: open, currency: "EUR", categoryId: nil)
    }

    private func settle() async throws {
        for _ in 0..<1500 {
            if model.selectionLoadTask == nil && model.categoryRefreshTask?.isCancelled != false { return }
            try await Task.sleep(for: .milliseconds(2))
        }
        XCTFail("Refresh did not settle")
    }

    private func snapshot(_ id: Int64? = nil, part: String = "") async throws -> RefundSnapshot {
        try await settle()
        let key = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: id ?? refund,
            ownPart: part, revision: model.refundRevision)
        let booking = try XCTUnwrap(bookings[id ?? refund])
        let result = try await model.refundSnapshot(for: key, booking: booking)
        return try XCTUnwrap(result)
    }

    private func candidates(_ data: RefundSnapshot, query: String = "") async throws -> [RefundCandidate] {
        let context = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: data.booking.id,
            ownPart: data.selectedPart, revision: model.refundRevision)
        let result = try await model.refundCandidates(for: RefundCandidateKey(context: context, query: query), snapshot: data)
        return try XCTUnwrap(result)
    }

    func testPositiveLinkAndMarkerAndInvalidation() async throws {
        let totals = model.totalsVersion
        let charts = model.financialReportRevision
        let revision = model.refundRevision
        await model.linkRefund(pair())
        let data = try await snapshot()
        XCTAssertEqual(data.links.count, 1)
        XCTAssertEqual(data.booking.refundMarker, "refund")
        XCTAssertEqual(data.booking.refundCents, 6000)
        XCTAssertGreaterThan(model.totalsVersion, totals)
        XCTAssertGreaterThan(model.financialReportRevision, charts)
        XCTAssertGreaterThan(model.refundRevision, revision)
        XCTAssertTrue(model.dashboardStale || model.dashboardTask != nil)
        let rows = try await candidates(data)
        XCTAssertFalse(rows.contains { $0.transactionId == original })
        XCTAssertFalse(data.suggestions.contains { $0.candidate.transactionId == original })
    }

    func testInspectorLoadsCounterpartsForTheSelectedBooking() async throws {
        await model.linkRefund(pair())
        await model.linkRefund(pair(second))
        try await settle()
        model.selectedBookingID = refund
        try await settle()
        let key = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: refund,
            ownPart: "", revision: model.refundRevision)
        let result = try await model.refundCard(for: key)
        let data = try XCTUnwrap(result)
        XCTAssertEqual(Set(data.counterparts.keys), [original, second])
        XCTAssertEqual(data.links.count, 2)
        XCTAssertEqual(data.counterparts[original]?.accountName, "Refund test")
        XCTAssertEqual(data.counterparts[original]?.currency, "EUR")
        let reverseKey = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: original,
            ownPart: "", revision: model.refundRevision)
        let reverse = try await model.refundCard(for: reverseKey)
        XCTAssertEqual(reverse?.counterparts[refund]?.id, refund)
        await model.unlinkRefund(pair())
        try await settle()
        let stale = try await model.refundCard(for: key)
        XCTAssertNil(stale)
        let emptyKey = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: original,
            ownPart: "", revision: model.refundRevision)
        let empty = try await model.refundCard(for: emptyKey)
        XCTAssertEqual(empty?.links.count, 0)
    }

    func testInspectorJumpRevealsCounterpartOutsideTheCurrentFilters() async throws {
        await model.linkRefund(pair())
        model.period = .days(30)
        model.directionFilter = .income
        model.amountFilter = AmountFilter(minCents: 1_000_000, maxCents: nil)
        model.searchText = "no matching refund test booking"
        model.startSelectionLoad()
        try await settle()
        XCTAssertNil(model.booking(withID: original))
        model.revealBooking(original)
        for _ in 0..<300 where model.selectedBookingID != original {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(model.selectedBooking?.id, original)
        XCTAssertEqual(model.directionFilter, .all)
        XCTAssertEqual(model.period, .all)
        XCTAssertTrue(model.searchText.isEmpty)
        XCTAssertFalse(model.amountFilter.isActive)
    }

    func testInspectorJumpDoesNotSelectInAReplacementVault() async throws {
        model.revealBooking(original)
        let other = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.useVault(try XCTUnwrap(other.vault), demo: true)
        let selected = model.selectedBookingID
        try await Task.sleep(for: .milliseconds(500))
        try await settle()
        XCTAssertEqual(model.selectedBookingID, selected)
        XCTAssertNil(model.errorMessage)
    }

    func testNegativeReverseAssignment() async throws {
        let reverse = RefundEndpointPair(bookingID: original, ownPart: "", paysBack: false,
            candidate: candidate(refund, cents: 10000, open: 10000))
        await model.linkRefund(reverse)
        let data = try await snapshot(original)
        XCTAssertFalse(data.paysBack)
        XCTAssertEqual(data.links.first?.refundId, refund)
        XCTAssertEqual(data.links.first?.originalId, original)
        XCTAssertEqual(data.booking.refundMarker, "original")
    }

    func testSplitIdentityAndRoleFollowUUIDNotPosition() async throws {
        let id = refund
        let parts = try await model.requireVaultOffMain { vault in
            try vault.setTransactionSplits(transactionId: id, splits: [
                TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: 14000, note: "Positive"),
                TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: -4000, note: "Negative"),
            ])
            return try vault.listTransactionSplits(transactionId: id)
        }
        await model.linkRefund(pair(part: parts[0].uuid))
        let first = try await snapshot(part: parts[0].uuid)
        XCTAssertEqual(first.links.first?.resolvedRefundPart, parts[0].uuid)
        let negative = try await snapshot(part: parts[1].uuid)
        XCTAssertFalse(negative.paysBack)
        XCTAssertTrue(negative.links.isEmpty)
        try await model.requireVaultOffMain { vault in
            try vault.setTransactionSplits(transactionId: id, splits: parts.reversed().map {
                TransactionSplitInput(uuid: $0.uuid, categoryId: $0.categoryId, amountCents: $0.amountCents, note: $0.note)
            })
        }
        let reordered = try await snapshot(part: parts[0].uuid)
        XCTAssertEqual(reordered.links.first?.refundPart, parts[0].uuid)
    }

    func testZeroTotalSplitOpensButUnsplitZeroDoesNot() async throws {
        var booking = try await snapshot().booking
        booking.amountCents = 0
        booking.splitCount = 2
        model.openRefundPicker(for: booking)
        XCTAssertEqual(model.refundPickerBooking?.id, booking.id)
        model.refundPickerBooking = nil
        booking.splitCount = 0
        model.openRefundPicker(for: booking)
        XCTAssertNil(model.refundPickerBooking)
    }

    func testUnlinkRestoreKeepsAllocationOrderAndInlineUndo() async throws {
        await model.linkRefund(pair())
        await model.linkRefund(pair(second))
        let before = try await snapshot().links
        await model.unlinkRefund(pair())
        let undo = try XCTUnwrap(model.refundUndo)
        await model.restoreRefund(undo)
        let after = try await snapshot().links
        XCTAssertEqual(after.map(\.originalId), before.map(\.originalId))
        XCTAssertEqual(after.map(\.createdMs), before.map(\.createdMs))
        XCTAssertEqual(after.map(\.allocatedCents), before.map(\.allocatedCents))
        XCTAssertNil(model.refundUndo)
    }

    func testStaleUndoCannotRestoreNewerTombstone() async throws {
        await model.linkRefund(pair())
        await model.unlinkRefund(pair())
        let old = try XCTUnwrap(model.refundUndo)
        await model.restoreRefund(old)
        await model.unlinkRefund(pair())
        let newer = try XCTUnwrap(model.refundUndo)
        XCTAssertNotEqual(old.token, newer.token)
        await model.restoreRefund(old)
        XCTAssertNotNil(model.errorMessage)
        let data = try await snapshot()
        XCTAssertTrue(data.links.isEmpty)
    }

    func testExhaustedCandidateAndOwnEndpointAreDisabled() async throws {
        let data = try await snapshot()
        XCTAssertFalse(data.canLink(candidate(second, cents: -7000, open: 0)))
        await model.linkRefund(pair())
        await model.linkRefund(pair(second))
        let exhausted = try await snapshot()
        XCTAssertEqual(exhausted.ownOpen, 0)
        XCTAssertFalse(exhausted.canLink(candidate(second, cents: -7000)))
    }

    func testPickerStateDiscardsEveryOtherRequestKey() async throws {
        let data = try await snapshot()
        let key = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: refund, ownPart: "one", revision: 1)
        let state = RefundPickerState(loaded: (key, data))
        XCTAssertNotNil(state.snapshot(for: key))
        let keys = [
            RefundLoadKey(vaultRevision: key.vaultRevision, bookingID: refund, ownPart: "two", revision: 1),
            RefundLoadKey(vaultRevision: key.vaultRevision, bookingID: second, ownPart: "one", revision: 1),
            RefundLoadKey(vaultRevision: key.vaultRevision + 1, bookingID: refund, ownPart: "one", revision: 1),
            RefundLoadKey(vaultRevision: key.vaultRevision, bookingID: refund, ownPart: "one", revision: 2),
        ]
        for other in keys { XCTAssertNil(state.snapshot(for: other)) }
    }

    func testSearchChangesOnlyCandidateState() async throws {
        let data = try await snapshot()
        let context = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: refund, ownPart: data.selectedPart,
            revision: model.refundRevision)
        let key = RefundCandidateKey(context: context, query: "")
        let rows = try await candidates(data)
        XCTAssertTrue(rows.contains { $0.transactionId == original })
        let state = RefundPickerState(loaded: (context, data), loadedCandidates: (key, rows))
        XCTAssertNotNil(state.candidates(for: key))
        XCTAssertNil(state.candidates(for: RefundCandidateKey(context: context, query: "changed")))
        XCTAssertNotNil(state.snapshot(for: context))
        let empty = try await candidates(data, query: UUID().uuidString)
        XCTAssertTrue(empty.isEmpty)
        let stale = RefundLoadKey(vaultRevision: context.vaultRevision, bookingID: refund,
            ownPart: data.selectedPart, revision: context.revision - 1)
        let discarded = try await model.refundCandidates(for: RefundCandidateKey(context: stale, query: ""), snapshot: data)
        XCTAssertNil(discarded)
    }

    func testSnapshotIsScopedToBookingDay() async throws {
        try await settle()
        var booking = try XCTUnwrap(bookings[refund])
        booking.bookingDate = "1900-01-01"
        let key = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: refund, ownPart: "", revision: model.refundRevision)
        let result = try await model.refundSnapshot(for: key, booking: booking)
        XCTAssertNil(result)
    }

    func testVaultSwitchClosesPickerAndUndoCannotChangeNewVault() async throws {
        let booking = try await snapshot().booking
        await model.linkRefund(pair())
        await model.unlinkRefund(pair())
        let undo = try XCTUnwrap(model.refundUndo)
        model.openRefundPicker(for: booking)
        let oldPath = model.vaultFilePath
        let other = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertNotEqual(oldPath, other.vaultFilePath)
        model.useVault(try XCTUnwrap(other.vault), demo: true)
        XCTAssertNil(model.refundPickerBooking)
        XCTAssertNil(model.refundUndo)
        await model.restoreRefund(undo)
        XCTAssertNil(model.errorMessage)
    }

    func testEmptySearchAndInvalidLinkError() async throws {
        let data = try await snapshot()
        let empty = try await candidates(data, query: UUID().uuidString)
        XCTAssertTrue(empty.isEmpty)
        let invalid = RefundEndpointPair(bookingID: refund, ownPart: "", paysBack: true,
            candidate: candidate(refund, cents: 10000))
        await model.linkRefund(invalid)
        XCTAssertNotNil(model.errorMessage)
        let unchanged = try await snapshot()
        XCTAssertTrue(unchanged.links.isEmpty)
    }

    func testDeletedBookingClosesSheet() async throws {
        let data = try await snapshot()
        model.openRefundPicker(for: data.booking)
        let id = refund
        try await model.requireVaultOffMain { try $0.deleteManualBooking(bookingId: id) }
        let key = RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: refund, ownPart: "", revision: model.refundRevision)
        let result = try await model.refundSnapshot(for: key, booking: data.booking)
        XCTAssertNil(result)
        XCTAssertNil(model.refundPickerBooking)
    }

    #if os(macOS)
    func testRefundUndoDoesNotWriteIntoAnotherVault() async throws {
        try await assertRefundUndoStaysInVault(unlink: false)
    }

    func testRefundRestoreUndoDoesNotWriteIntoAnotherVault() async throws {
        try await assertRefundUndoStaysInVault(unlink: true)
    }

    private func assertRefundUndoStaysInVault(unlink: Bool) async throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 160, height: 100),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        let undo = try XCTUnwrap(model.windowUndoManager)
        let groupedByEvent = undo.groupsByEvent
        undo.groupsByEvent = false
        defer { undo.removeAllActions(); undo.groupsByEvent = groupedByEvent }
        let oldPin = try XCTUnwrap(model.pinVault())
        undo.beginUndoGrouping()
        await model.linkRefund(pair())
        undo.endUndoGrouping()
        if unlink {
            undo.beginUndoGrouping()
            await model.unlinkRefund(pair())
            undo.endUndoGrouping()
            XCTAssertNotNil(model.refundUndo)
        }
        XCTAssertNil(model.errorMessage)
        XCTAssertTrue(undo.canUndo)
        let other = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertNotEqual(model.vaultFilePath, other.vaultFilePath)
        let ids = try await other.requireVaultOffMain { vault in
            let account = try vault.createOfflineAccount(name: "Refund test", kind: .checking, currency: "EUR", openingCents: 0)
            return try [Int64(10000), -6000, -7000].map { amount in
                try vault.saveManualBooking(accountId: account, bookingId: nil,
                    setup: ManualBookingSetup(bookingDate: "2026-04-01", amountCents: amount, counterparty: "Test",
                        purpose: "REF-TEST-714", categoryId: nil, note: ""))
            }
        }
        XCTAssertEqual(ids, [refund, original, second])
        let endpoint = pair()
        if !unlink {
            try await other.requireVaultOffMain {
                try $0.linkRefund(refundId: endpoint.refundID, refundPart: endpoint.refundPart,
                    originalId: endpoint.originalID, originalPart: endpoint.originalPart)
            }
        }
        let otherPin = try XCTUnwrap(other.pinVault())
        let oldBefore = try await oldPin.run { try $0.refundLinks(transactionId: endpoint.refundID) }
        let otherBefore = try await otherPin.run { try $0.refundLinks(transactionId: endpoint.refundID) }
        model.useVault(try XCTUnwrap(other.vault), demo: true)
        undo.undo()
        await Task { @MainActor in }.value
        try await settle()
        let oldLinks = try await oldPin.run { try $0.refundLinks(transactionId: endpoint.refundID) }
        let otherLinks = try await otherPin.run { try $0.refundLinks(transactionId: endpoint.refundID) }
        XCTAssertEqual(oldLinks, oldBefore)
        XCTAssertEqual(otherLinks, otherBefore)
        XCTAssertEqual(oldLinks.filter { !$0.inactive }.count, unlink ? 0 : 1)
        XCTAssertEqual(otherLinks.filter { !$0.inactive }.count, unlink ? 0 : 1)
        XCTAssertNil(model.errorMessage)
    }
    #endif

    func testLinkingMarksBothRowsWithoutChangingAmountsOrCategories() async throws {
        let before = try await snapshot().booking
        let beforeOriginal = try await snapshot(original).booking
        model.sidebarSelection = .account(before.accountId)
        model.period = .all
        model.reloadAll()
        try await settle()
        let beforeRows = model.bookings
        XCTAssertNotNil(model.booking(withID: refund))
        XCTAssertNotNil(model.booking(withID: original))
        let revision = model.refundRevision
        await model.linkRefund(pair())
        let after = try await snapshot().booking
        let afterOriginal = try await snapshot(original).booking
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(after.refundMarker, "refund")
        XCTAssertEqual(afterOriginal.refundMarker, "original")
        XCTAssertEqual(after.refundCents, 6000)
        XCTAssertEqual(afterOriginal.refundedCents, 6000)
        XCTAssertEqual(model.booking(withID: refund)?.refundMarker, "refund")
        XCTAssertEqual(model.booking(withID: original)?.refundMarker, "original")
        XCTAssertEqual(model.bookings.map(\.id), beforeRows.map(\.id))
        XCTAssertEqual(model.bookings.map(\.amountCents), beforeRows.map(\.amountCents))
        XCTAssertEqual(model.bookings.map(\.categoryId), beforeRows.map(\.categoryId))
        XCTAssertGreaterThan(model.refundRevision, revision)
        for (old, new) in [(before, after), (beforeOriginal, afterOriginal)] {
            XCTAssertEqual(old.amountCents, new.amountCents)
            XCTAssertEqual(old.categoryId, new.categoryId)
        }
    }

    func testARefundTakesSeveralOriginalsAndEachLinkIsRemovedOnItsOwn() async throws {
        await model.linkRefund(pair())
        await model.linkRefund(pair(second))
        let linked = try await snapshot()
        XCTAssertEqual(linked.links.map(\.originalId), [original, second])
        await model.unlinkRefund(pair())
        let remaining = try await snapshot()
        XCTAssertEqual(remaining.links.map(\.originalId), [second])
        let removed = try await snapshot(original)
        XCTAssertEqual(removed.booking.refundMarker, "none")
        XCTAssertEqual(remaining.booking.refundMarker, "refund")
        await model.linkRefund(pair())
        let relinked = try await snapshot()
        for link in relinked.links { await model.unlinkRefund(RefundEndpointPair(link)) }
        let empty = try await snapshot()
        let secondRow = try await snapshot(second)
        XCTAssertTrue(empty.links.isEmpty)
        XCTAssertEqual(empty.booking.refundMarker, "none")
        XCTAssertEqual(secondRow.booking.refundMarker, "none")
        XCTAssertNil(model.errorMessage)
    }

    func testARefundLargerThanItsBookingShowsTheRestAsOpen() async throws {
        let smaller = try await addBooking(-2000)
        await model.linkRefund(pair(smaller))
        let first = try await snapshot()
        XCTAssertEqual(first.links.first?.allocatedCents, 2000)
        XCTAssertEqual(first.links.first?.originalOpenCents, 0)
        XCTAssertEqual(first.links.first?.refundOpenCents, 8000)
        XCTAssertEqual(first.ownOpen, 8000)
        await model.linkRefund(pair())
        let combined = try await snapshot()
        XCTAssertEqual(combined.links.map(\.allocatedCents), [2000, 6000])
        XCTAssertEqual(combined.links.map(\.originalOpenCents), [0, 0])
        XCTAssertTrue(combined.links.allSatisfy { $0.refundOpenCents == 2000 })
        XCTAssertEqual(combined.booking.refundCents, 8000)
        XCTAssertEqual(combined.ownOpen, 2000)
        let smallRow = try await snapshot(smaller)
        let originalRow = try await snapshot(original)
        XCTAssertEqual(smallRow.booking.refundedCents, 2000)
        XCTAssertEqual(originalRow.booking.refundedCents, 6000)
        XCTAssertNil(model.errorMessage)
    }

    func testAnInvalidLinkShowsTheCoreErrorAndChangesNothing() async throws {
        let before = try await snapshot()
        let beforeOriginal = try await snapshot(original)
        let invalid = RefundEndpointPair(bookingID: original, ownPart: "", paysBack: true,
            candidate: candidate(refund, cents: 10000))
        await model.linkRefund(invalid)
        XCTAssertNotNil(model.errorMessage)
        let after = try await snapshot()
        let afterOriginal = try await snapshot(original)
        XCTAssertEqual(after.booking, before.booking)
        XCTAssertEqual(afterOriginal.booking, beforeOriginal.booking)
        XCTAssertTrue(after.links.isEmpty)
        XCTAssertTrue(afterOriginal.links.isEmpty)
    }

    func testAUsedUpRefundIsNotOfferedForFurtherBookings() async throws {
        await model.linkRefund(pair())
        let partial = try await snapshot(original)
        XCTAssertEqual(partial.links.first?.refundOpenCents, 4000)
        await model.linkRefund(pair(second))
        let full = try await snapshot()
        let first = try await snapshot(original)
        let secondRow = try await snapshot(second)
        XCTAssertEqual(full.ownOpen, 0)
        XCTAssertEqual(first.links.first?.refundOpenCents, 0)
        XCTAssertEqual(secondRow.links.first?.refundOpenCents, 0)
        let offered = try await candidates(full)
        XCTAssertTrue(offered.isEmpty)
        let furtherOriginal = try await addBooking(-1000)
        let unusedRefund = try await addBooking(1000)
        let furtherCandidates = try await candidates(snapshot(furtherOriginal))
        XCTAssertFalse(furtherCandidates.contains { $0.transactionId == refund })
        XCTAssertTrue(furtherCandidates.contains { $0.transactionId == unusedRefund })
        XCTAssertNil(model.errorMessage)
    }

    func testTheInspectorCardHasItsPlaceInTheStandardLayout() {
        let visible = InspectorLayout.standard.visible
        XCTAssertEqual(visible.firstIndex(of: .refunds), visible.firstIndex(of: .tags).map { $0 + 1 })
        XCTAssertEqual(InspectorLayout(storageValue: "note,receipt").order.last, .rawData)
        XCTAssertTrue(InspectorLayout(storageValue: "note,receipt").visible.contains(.refunds))
    }

    func testUsedUpRefundTakesNoFurtherBookingAndFillsInLinkOrder() async throws {
        let small = try await addBooking(-2000, date: "2026-04-02")
        let large = try await addBooking(-15000, date: "2026-03-31")
        await model.linkRefund(pair(small))
        await model.linkRefund(pair(large))
        let filled = try await snapshot()
        XCTAssertEqual(filled.links.map(\.originalId), [small, large])
        XCTAssertEqual(filled.links.map(\.allocatedCents), [2000, 8000])
        XCTAssertEqual(filled.ownOpen, 0)
        XCTAssertNil(model.errorMessage)
        await model.linkRefund(pair())
        XCTAssertNotNil(model.errorMessage)
        let after = try await snapshot()
        XCTAssertEqual(after.links, filled.links)
        let untouched = try await snapshot(original)
        XCTAssertEqual(untouched.booking.refundMarker, "none")
    }

    func testLinkedRefundOfSplitOriginalLeavesIncomeAndKeepsSplit() async throws {
        let before = try await flow()
        let parts = try await split(original, amounts: [-1, -5999])
        let available = try await candidates(snapshot())
        let offered = available.filter { $0.transactionId == original }
        XCTAssertEqual(Set(offered.map(\.part)), Set(parts.map(\.uuid)))
        XCTAssertEqual(Set(offered.map(\.amountCents)), [-1, -5999])
        await model.linkRefund(pair())
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil
        let endpoint = RefundEndpointPair(bookingID: refund, ownPart: "", paysBack: true,
            candidate: candidate(original, part: parts[1].uuid, cents: -5999))
        await model.linkRefund(endpoint)
        let linked = try await snapshot()
        let after = try await flow()
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(linked.links.map(\.originalPart), [parts[1].uuid])
        XCTAssertEqual(linked.links.first?.allocatedCents, 5999)
        XCTAssertEqual(after.incomeCents, before.incomeCents - 5999)
        XCTAssertEqual(after.consumptionCents, before.consumptionCents - 5999)
        let id = original
        let preserved = try await model.requireVaultOffMain { try $0.listTransactionSplits(transactionId: id) }
        XCTAssertEqual(preserved, parts)
        let reverse = try await snapshot(original, part: parts[1].uuid)
        XCTAssertEqual(reverse.links.map(\.refundId), [refund])
        await model.unlinkRefund(endpoint)
        let restored = try await flow()
        XCTAssertEqual(restored, before)
    }

    func testLinkingReloadsTheExpensePieAndLowersTheCategory() async throws {
        let ids = [refund, original]
        let category = try await model.requireVaultOffMain { vault in
            let category = try vault.createCategory(name: "Refund category", parentId: nil, sfSymbol: "bag", colorHex: "#4C9A2A")
            for id in ids { try vault.setTransactionCategory(transactionId: id, categoryId: category) }
            return category
        }
        model.reloadCategories()
        let sums = model.categories.map(\.totalCents)
        let revision = model.financialReportRevision
        let before = try await categoryExpense(category)
        XCTAssertEqual(before, 6000)
        await model.linkRefund(pair())
        try await settle()
        let after = try await categoryExpense(category)
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(model.categories.map(\.totalCents), sums)
        XCTAssertGreaterThan(model.financialReportRevision, revision)
        XCTAssertEqual(after, before - 6000)
        await model.unlinkRefund(pair())
        let restored = try await categoryExpense(category)
        XCTAssertEqual(restored, before)
    }

    func testAPartOfASplitIncomeIsTheRefund() async throws {
        let parts = try await split(refund, amounts: [9999, 1], notes: ["", "Outlay"])
        await model.linkRefund(pair(part: UUID().uuidString))
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil
        await model.linkRefund(pair(part: parts[1].uuid))
        let linked = try await snapshot(part: parts[1].uuid)
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(linked.links.map(\.refundPart), [parts[1].uuid])
        XCTAssertEqual(linked.links.first?.refundLabel, "Outlay")
        XCTAssertEqual(linked.links.first?.allocatedCents, 1)
        XCTAssertEqual(linked.booking.refundMarker, "refund")
        let reverse = try await snapshot(original)
        XCTAssertEqual(reverse.links.first?.allocatedCents, 1)
        await model.unlinkRefund(pair(part: parts[1].uuid))
        let unlinked = try await snapshot(part: parts[1].uuid)
        XCTAssertTrue(unlinked.links.isEmpty)
        XCTAssertEqual(unlinked.booking.refundMarker, "none")
    }

    func testStoreCreditInAPurchasePaysBackABooking() async throws {
        let parts = try await split(second, amounts: [-14000, 7000], notes: ["", "Store credit"])
        let whole = RefundEndpointPair(bookingID: second, ownPart: "", paysBack: true,
            candidate: candidate(original, cents: -6000))
        await model.linkRefund(whole)
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil
        let endpoint = RefundEndpointPair(bookingID: second, ownPart: parts[1].uuid, paysBack: true,
            candidate: candidate(original, cents: -6000))
        let offered = try await model.requireVaultOffMain { [id = original] in
            try $0.refundCandidates(transactionId: id, ownPart: "", query: "", limit: 200)
        }
        XCTAssertTrue(offered.contains { $0.transactionId == second && $0.part == parts[1].uuid && $0.amountCents == 7000 })
        await model.linkRefund(endpoint)
        let linked = try await snapshot(second, part: parts[1].uuid)
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(linked.amount, 7000)
        XCTAssertEqual(linked.links.map(\.originalId), [original])
        XCTAssertEqual(linked.links.first?.allocatedCents, 6000)
        XCTAssertEqual(linked.ownOpen, 1000)
        XCTAssertEqual(linked.booking.refundMarker, "refund")
        let returned = try await snapshot(original)
        XCTAssertEqual(returned.booking.refundMarker, "original")
        let remaining = try await model.requireVaultOffMain { [id = original] in
            try $0.refundCandidates(transactionId: id, ownPart: "", query: "", limit: 200)
        }
        XCTAssertFalse(remaining.contains { $0.transactionId == second && $0.part == parts[1].uuid })
        await model.unlinkRefund(endpoint)
        let unlinked = try await snapshot(second, part: parts[1].uuid)
        let originalRow = try await snapshot(original)
        XCTAssertEqual(unlinked.booking.refundMarker, "none")
        XCTAssertEqual(originalRow.booking.refundMarker, "none")
    }

    func testSuggestionsAreAtMostThreeCandidates() async throws {
        _ = try await addBooking(-2000)
        _ = try await addBooking(-3000)
        let data = try await snapshot()
        let available = try await candidates(data)
        XCTAssertGreaterThanOrEqual(available.count, 4)
        XCTAssertEqual(data.suggestions.count, 3)
        XCTAssertTrue(data.suggestions.allSatisfy { suggestion in
            available.contains { $0.transactionId == suggestion.candidate.transactionId && $0.part == suggestion.candidate.part }
        })
        let first = try XCTUnwrap(data.suggestions.first?.candidate)
        await model.linkRefund(RefundEndpointPair(bookingID: refund, ownPart: "", paysBack: true, candidate: first))
        let after = try await snapshot()
        XCTAssertLessThanOrEqual(after.suggestions.count, 3)
        XCTAssertFalse(after.suggestions.contains { $0.candidate.transactionId == first.transactionId && $0.candidate.part == first.part })
        XCTAssertNil(model.errorMessage)
    }

    private func addBooking(_ amount: Int64, date: String = "2026-04-01") async throws -> Int64 {
        let account = try XCTUnwrap(bookings[refund]).accountId
        let booking = try await model.requireVaultOffMain { vault in
            let id = try vault.saveManualBooking(accountId: account, bookingId: nil,
                setup: ManualBookingSetup(bookingDate: date, amountCents: amount, counterparty: "Test",
                    purpose: "REF-TEST-714", categoryId: nil, note: ""))
            let filter = BookingFilter(accountIds: [account], categoryId: nil, includeSubcategories: true,
                uncategorizedOnly: false, search: nil, dateFrom: date, dateTo: date,
                limit: 0, offset: 0, scope: .all, merchantKey: nil, direction: .all, checked: nil, wholeDays: false)
            return try XCTUnwrap(vault.listTransactions(filter: filter).transactions.first { $0.id == id })
        }
        bookings[booking.id] = booking
        return booking.id
    }

    private func split(_ id: Int64, amounts: [Int64], notes: [String] = ["", ""]) async throws -> [TransactionSplit] {
        try await model.requireVaultOffMain { vault in
            try vault.setTransactionSplits(transactionId: id, splits: zip(amounts, notes).map {
                TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: $0.0, note: $0.1)
            })
            return try vault.listTransactionSplits(transactionId: id)
        }
    }

    private func flow() async throws -> SelectionFlow {
        let account = try XCTUnwrap(bookings[refund]).accountId
        return try await model.requireVaultOffMain { try $0.selectionFlow(accountIds: [account], dateFrom: nil, dateTo: nil) }
    }

    private func categoryExpense(_ category: Int64) async throws -> Int64 {
        let account = try XCTUnwrap(bookings[refund]).accountId
        return try await model.requireVaultOffMain { vault in
            let filter = BookingFilter(accountIds: [account], categoryId: nil, includeSubcategories: true,
                uncategorizedOnly: false, search: nil, dateFrom: nil, dateTo: nil,
                limit: 0, offset: 0, scope: .all, merchantKey: nil, direction: .all, checked: nil, wholeDays: false)
            return try vault.listCategoryExpenses(filter: filter).first { $0.categoryId == category }?.cents ?? 0
        }
    }
}
