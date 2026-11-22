import XCTest
import GiroCrabCore
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
}
