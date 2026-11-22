import XCTest
import GiroCrabCore
#if os(macOS)
import AppKit
#endif
@testable import GiroCrabUI

/// Refund links (#714): the model writes the link, patches the loaded rows' markers without a
/// list reload and leaves amounts and categories alone.
@MainActor
final class RefundLinkTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        XCTAssertNil(model.errorMessage)
    }

    override func tearDown() async throws {
        model = nil
        AppDefaults.store.removeObject(forKey: AppModel.lastPeriodKey)
    }

    /// A loaded incoming booking and two loaded outgoing ones the core suggests for it.
    private func pair() throws -> (refund: Booking, originals: [Booking]) {
        let vault = try XCTUnwrap(model.vault)
        for refund in model.bookings where refund.amountCents > 0 {
            let candidates = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 50)
            let loaded = candidates.compactMap { model.booking(withID: $0.id) }
            if loaded.count >= 2 { return (refund, Array(loaded.prefix(2))) }
        }
        throw XCTSkip("The demo vault has no incoming booking with two loaded candidates")
    }

    private func kind(_ id: Int64) -> RefundLinkKind? { model.booking(withID: id)?.refundLink }

    #if os(macOS)
    func testRefundUndoDoesNotWriteIntoAnotherVault() throws {
        try assertRefundUndoStaysInVault(unlink: false)
    }

    func testRefundRestoreUndoDoesNotWriteIntoAnotherVault() throws {
        try assertRefundUndoStaysInVault(unlink: true)
    }

    private func assertRefundUndoStaysInVault(unlink: Bool) throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 160, height: 100),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        let undo = try XCTUnwrap(model.windowUndoManager)
        undo.groupsByEvent = false
        defer { undo.groupsByEvent = true }
        let (refund, originals) = try pair()
        let vault = try XCTUnwrap(model.vault)
        undo.beginUndoGrouping()
        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        undo.endUndoGrouping()
        if unlink {
            undo.beginUndoGrouping()
            model.unlinkRefund(refundID: refund.id, originalID: originals[0].id)
            undo.endUndoGrouping()
        }
        XCTAssertNil(model.errorMessage)
        XCTAssertTrue(undo.canUndo)
        let other = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let otherVault = try XCTUnwrap(other.vault)
        if !unlink {
            try otherVault.linkRefundParts(refundId: refund.id, refundPart: 0, originalId: originals[0].id, originalPart: 0)
        }
        model.useVault(otherVault, demo: true)
        undo.undo()
        XCTAssertEqual(try otherVault.refundLinks(transactionId: refund.id).originals.count, unlink ? 0 : 1)
        XCTAssertEqual(try vault.refundLinks(transactionId: refund.id).originals.count, unlink ? 0 : 1)
    }
    #endif

    func testLinkingMarksBothRowsWithoutChangingAmountsOrCategories() throws {
        let (refund, originals) = try pair()
        let original = originals[0]
        let revision = model.refundLinksRevision
        let before = model.bookings.map { [$0.amountCents, $0.categoryId ?? -1] }

        model.linkRefund(refundID: refund.id, originalID: original.id)

        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(kind(refund.id), .refund)
        XCTAssertEqual(kind(original.id), .original)
        XCTAssertGreaterThan(model.refundLinksRevision, revision)
        XCTAssertEqual(model.bookings.map { [$0.amountCents, $0.categoryId ?? -1] }, before)
        // the rows match what the core lists
        let listed = try XCTUnwrap(model.vault).listTransactions(filter: model.currentFilter).transactions
        XCTAssertEqual(listed.first { $0.id == refund.id }?.refundLink, .refund)
        XCTAssertEqual(listed.first { $0.id == original.id }?.refundLink, .original)
    }

    func testARefundTakesSeveralOriginalsAndEachLinkIsRemovedOnItsOwn() throws {
        let (refund, originals) = try pair()
        let vault = try XCTUnwrap(model.vault)
        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        model.linkRefund(refundID: refund.id, originalID: originals[1].id)
        XCTAssertEqual(kind(originals[0].id), .original)
        XCTAssertEqual(kind(originals[1].id), .original)
        XCTAssertEqual(Set(try vault.refundLinks(transactionId: refund.id).originals.map(\.id)), Set(originals.map(\.id)))

        model.unlinkRefund(refundID: refund.id, originalID: originals[0].id)
        XCTAssertEqual(kind(originals[0].id), RefundLinkKind.none)
        XCTAssertEqual(kind(refund.id), .refund)
        XCTAssertEqual(try vault.refundLinks(transactionId: refund.id).originals.map(\.id), [originals[1].id])

        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        model.unlinkRefundOriginals(refundID: refund.id)
        XCTAssertEqual(kind(refund.id), RefundLinkKind.none)
        XCTAssertEqual(kind(originals[1].id), RefundLinkKind.none)
        XCTAssertTrue(try vault.refundLinks(transactionId: refund.id).originals.isEmpty)
    }

    /// A refund larger than its booking: the booking is never refunded beyond its amount, the
    /// rest of the refund is shown as open and goes to the next booking.
    func testARefundLargerThanItsBookingShowsTheRestAsOpen() throws {
        let vault = try XCTUnwrap(model.vault)
        var found: (Booking, [Booking])?
        for refund in model.bookings where refund.amountCents > 0 && refund.currency == "EUR" {
            let smaller = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200)
                .filter { $0.currency == "EUR" && -$0.amountCents < refund.amountCents / 2 }
                .compactMap { model.booking(withID: $0.id) }
            if smaller.count >= 2 { found = (refund, Array(smaller.prefix(2))); break }
        }
        let (refund, originals) = try XCTUnwrap(found)
        let money = { Money.format($0, currency: "EUR") }

        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        let first = try vault.refundLinks(transactionId: originals[0].id)
        XCTAssertEqual(first.refundedCents, -originals[0].amountCents)
        let firstLines = RefundSum.lines(booking: originals[0], links: first)
        XCTAssertEqual(firstLines.count, 2)
        XCTAssertTrue(firstLines[0].contains(money(-originals[0].amountCents)))
        XCTAssertFalse(firstLines[0].contains(money(refund.amountCents)))

        model.linkRefund(refundID: refund.id, originalID: originals[1].id)
        let own = try vault.refundLinks(transactionId: refund.id)
        let assigned = -(originals[0].amountCents + originals[1].amountCents)
        XCTAssertEqual(own.assignedCents, assigned)
        // each linked booking shows its share: at most its amount
        XCTAssertEqual(Dictionary(uniqueKeysWithValues: own.originals.map { ($0.id, $0.shareCents) }),
                       Dictionary(uniqueKeysWithValues: originals.map { ($0.id, Optional(-$0.amountCents)) }))
        XCTAssertNil(try vault.refundCandidates(transactionId: refund.id, query: "", limit: 1).first?.shareCents)
        // the row marker carries the amount only while the booking is covered in part
        for b in model.bookings where b.id == refund.id || originals.contains(where: { $0.id == b.id }) {
            let links = try vault.refundLinks(transactionId: b.id)
            let covered = b.id == refund.id ? links.assignedCents : links.refundedCents
            XCTAssertEqual(b.refundPartCents, covered.flatMap { $0 < abs(b.amountCents) ? $0 : nil })
            XCTAssertEqual(b.refundIncomeCents, b.id == refund.id ? b.amountCents - (covered ?? b.amountCents) : 0)
        }
        let ownLines = RefundSum.lines(booking: refund, links: own)
        XCTAssertEqual(ownLines.count, 2)
        XCTAssertTrue(ownLines[1].contains(money(refund.amountCents - assigned)))
    }

    /// A refund larger than its first booking and used up by the second: the first booking's
    /// lines no longer say that the rest can go to further bookings.
    func testAUsedUpRefundIsNotOfferedForFurtherBookings() throws {
        let vault = try XCTUnwrap(model.vault)
        var found: (Booking, [Booking])?
        for refund in model.bookings where refund.amountCents > 0 && refund.currency == "EUR" {
            let all = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200)
                .filter { $0.currency == "EUR" && $0.part == 0 }
            guard let small = all.first(where: { -$0.amountCents < refund.amountCents }),
                  let rest = all.first(where: { $0.id != small.id && -$0.amountCents >= refund.amountCents + small.amountCents }),
                  let a = model.booking(withID: small.id), let b = model.booking(withID: rest.id) else { continue }
            found = (refund, [a, b]); break
        }
        let (refund, originals) = try XCTUnwrap(found)
        let further = "weiteren Buchungen"

        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        let open = RefundSum.lines(booking: originals[0], links: try vault.refundLinks(transactionId: originals[0].id))
        XCTAssertEqual(open.count, 2)
        XCTAssertTrue(open[1].contains(further), open[1])

        model.linkRefund(refundID: refund.id, originalID: originals[1].id)
        XCTAssertEqual(try vault.refundLinks(transactionId: refund.id).refundOpenCents, 0)
        let usedUp = RefundSum.lines(booking: originals[0], links: try vault.refundLinks(transactionId: originals[0].id))
        XCTAssertEqual(usedUp.count, 2)
        XCTAssertFalse(usedUp[1].contains(further), usedUp[1])
        XCTAssertEqual(RefundSum.lines(booking: refund, links: try vault.refundLinks(transactionId: refund.id)).count, 1)
    }

    func testAnInvalidLinkShowsTheCoreErrorAndChangesNothing() throws {
        let (refund, originals) = try pair()
        // the wrong way round: an outgoing booking as the refund
        model.linkRefund(refundID: originals[0].id, originalID: refund.id)
        XCTAssertNotNil(model.errorMessage)
        XCTAssertEqual(kind(refund.id), RefundLinkKind.none)
        XCTAssertEqual(kind(originals[0].id), RefundLinkKind.none)
    }

    func testThePickerOpensOnlyForBookingsWithAnAmount() throws {
        let (refund, _) = try pair()
        model.openRefundPicker(for: refund)
        XCTAssertEqual(model.refundPickerBooking?.id, refund.id)
        // the open sheet's booking follows the link state
        let original = try XCTUnwrap(try XCTUnwrap(model.vault).refundCandidates(transactionId: refund.id, query: "", limit: 1).first)
        model.linkRefund(refundID: refund.id, originalID: original.id)
        XCTAssertEqual(model.refundPickerBooking?.refundLink, .refund)
    }

    func testThePickerClosesWhenItsVaultIsGone() throws {
        let (refund, _) = try pair()
        model.openRefundPicker(for: refund)
        XCTAssertNotNil(model.refundPickerBooking)
        model.releaseVaultState()
        XCTAssertNil(model.refundPickerBooking)
    }

    func testTheInspectorCardHasItsPlaceInTheStandardLayout() {
        let visible = InspectorLayout.standard.visible
        XCTAssertEqual(visible.firstIndex(of: .refunds), visible.firstIndex(of: .tags).map { $0 + 1 })
        // a layout saved before the card existed gets it at the end
        XCTAssertEqual(InspectorLayout(storageValue: "note,receipt").order.last, .rawData)
        XCTAssertTrue(InspectorLayout(storageValue: "note,receipt").visible.contains(.refunds))
    }

    /// A refund that is used up takes no further booking, and the shares follow the order of linking.
    func testUsedUpRefundTakesNoFurtherBookingAndFillsInLinkOrder() throws {
        let vault = try XCTUnwrap(model.vault)
        var found: (Booking, RefundBooking, RefundBooking)?
        for refund in model.bookings where refund.amountCents > 1 && refund.currency == "EUR" && refund.refundLink == .none {
            let all = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200)
                .filter { $0.currency == "EUR" && $0.openCents == -$0.amountCents }
            if let small = all.first(where: { -$0.amountCents < refund.amountCents }),
               let large = all.first(where: { -$0.amountCents >= refund.amountCents && $0.bookingDate < small.bookingDate }) {
                found = (refund, small, large); break
            }
        }
        let (refund, small, large) = try XCTUnwrap(found)
        // the later booking is linked first: it is filled first, the earlier one gets the rest
        model.linkRefund(refundID: refund.id, originalID: small.id)
        model.linkRefund(refundID: refund.id, originalID: large.id)
        var links = try vault.refundLinks(transactionId: refund.id)
        XCTAssertEqual(links.originals.map(\.id), [small.id, large.id])
        XCTAssertEqual(links.originals.map(\.shareCents), [-small.amountCents, refund.amountCents + small.amountCents])
        XCTAssertEqual(links.refundOpenCents, 0)
        XCTAssertNil(model.errorMessage)

        let third = try XCTUnwrap(try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200)
            .first { $0.id != small.id && $0.id != large.id })
        model.linkRefund(refundID: refund.id, originalID: third.id)
        XCTAssertNotNil(model.errorMessage)
        links = try vault.refundLinks(transactionId: refund.id)
        XCTAssertEqual(links.originals.map(\.id), [small.id, large.id])
    }

    /// A linked refund is no income as far as it is assigned: the core returns that part as a reduced expense, for a split
    /// original too, and the split stays as it is.
    func testLinkedRefundOfSplitOriginalLeavesIncomeAndKeepsSplit() throws {
        let vault = try XCTUnwrap(model.vault)
        let flow = { try vault.selectionFlow(accountIds: [], dateFrom: nil, dateTo: nil) }
        let before = try flow()
        let refund = try XCTUnwrap(model.bookings.first { $0.amountCents > 0 && $0.flow == .income && $0.currency == "EUR" })
        let candidates = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200)
        let original = try XCTUnwrap(candidates.first { $0.amountCents < -1 })
        let parts = [
            TransactionSplitInput(categoryId: nil, amountCents: -1, note: ""),
            TransactionSplitInput(categoryId: nil, amountCents: original.amountCents + 1, note: "")
        ]
        try vault.setTransactionSplits(transactionId: original.id, splits: parts)

        // the picker lists the parts instead of the split booking; a link names one of them
        let offered = try vault.refundCandidates(transactionId: refund.id, query: "", limit: 200).filter { $0.id == original.id }
        XCTAssertEqual(offered.map(\.part), [1, 2])
        XCTAssertEqual(offered.map(\.amountCents), parts.map(\.amountCents))
        model.linkRefund(refundID: refund.id, originalID: original.id)
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil
        model.linkRefund(refundID: refund.id, originalID: original.id, part: 2)

        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(try vault.refundLinks(transactionId: refund.id).originals.map(\.part), [2])
        // only what the chosen part takes up of the refund moves; the rest stays income
        let assigned = min(refund.amountCents, -(original.amountCents + 1))
        XCTAssertEqual(try flow().incomeCents, before.incomeCents - assigned)
        XCTAssertEqual(try flow().consumptionCents, before.consumptionCents - assigned)
        XCTAssertEqual(try vault.listTransactionSplits(transactionId: original.id).map(\.amountCents), parts.map(\.amountCents))
        XCTAssertEqual(try vault.refundLinks(transactionId: original.id).refunds.map(\.id), [refund.id])

        model.unlinkRefundOriginals(refundID: refund.id)
        XCTAssertEqual(try flow().incomeCents, before.incomeCents)
    }

    /// The pie of the expenses reads its slices again after a link, also when refund and booking
    /// share a category and every category sum stays, and its category gets smaller.
    func testLinkingReloadsTheExpensePieAndLowersTheCategory() throws {
        let (refund, originals) = try pair()
        let original = originals[0]
        let vault = try XCTUnwrap(model.vault)
        let category = try XCTUnwrap(model.categories.first { $0.depth == 0 }).id
        try vault.setTransactionCategory(transactionId: refund.id, categoryId: category)
        try vault.setTransactionCategory(transactionId: original.id, categoryId: category)
        model.reloadCategories()
        let expenses = { try XCTUnwrap(vault.listCategoryExpenses(dateFrom: nil, dateTo: nil, accountIds: []).first { $0.id == category }).totalCents }
        let (sums, key, before) = (model.categories.map(\.totalCents), AccountExpensePie.key(model), try expenses())

        model.linkRefund(refundID: refund.id, originalID: original.id)

        XCTAssertNil(model.errorMessage)
        model.reloadCategories()
        XCTAssertEqual(model.categories.map(\.totalCents), sums)
        XCTAssertNotEqual(AccountExpensePie.key(model), key)
        XCTAssertEqual(try expenses(), before + min(refund.amountCents, -original.amountCents))

        model.unlinkRefundOriginals(refundID: refund.id)
        XCTAssertEqual(try expenses(), before)
    }

    /// Removing the split of a linked booking leaves a link to the whole booking, and the open
    /// views of the links read them again.
    func testRemovingTheSplitLinksTheRefundToTheWholeBooking() throws {
        let (refund, originals) = try pair()
        let original = try XCTUnwrap(originals.first { $0.amountCents < -1 })
        let vault = try XCTUnwrap(model.vault)
        try vault.setTransactionSplits(transactionId: original.id, splits: [
            TransactionSplitInput(categoryId: nil, amountCents: -1, note: ""),
            TransactionSplitInput(categoryId: nil, amountCents: original.amountCents + 1, note: "")
        ])
        model.linkRefund(refundID: refund.id, originalID: original.id, part: 2)
        XCTAssertNil(model.errorMessage)
        XCTAssertEqual(try vault.refundLinks(transactionId: refund.id).originals.map(\.part), [2])
        let revision = model.refundLinksRevision

        XCTAssertNil(model.saveSplits([], for: try XCTUnwrap(model.booking(withID: original.id))))

        XCTAssertGreaterThan(model.refundLinksRevision, revision)
        let links = try vault.refundLinks(transactionId: refund.id).originals
        XCTAssertEqual(links.map(\.part), [0])
        XCTAssertEqual(links.first?.shareCents, min(refund.amountCents, -original.amountCents))
        XCTAssertEqual(kind(original.id), .original)
    }

    /// A split income is linked by the part of it that is the refund: only that part pays back.
    func testAPartOfASplitIncomeIsTheRefund() throws {
        let (refund, originals) = try pair()
        let original = originals[0]
        let vault = try XCTUnwrap(model.vault)
        try XCTSkipIf(refund.amountCents < 2 || original.amountCents > -2, "The demo pair is too small to split")
        try vault.setTransactionSplits(transactionId: refund.id, splits: [
            TransactionSplitInput(categoryId: nil, amountCents: refund.amountCents - 1, note: ""),
            TransactionSplitInput(categoryId: nil, amountCents: 1, note: "Auslage")
        ])
        // a part the income does not have
        model.linkRefund(refundID: refund.id, originalID: original.id, refundPart: 3)
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil

        model.linkRefund(refundID: refund.id, originalID: original.id, refundPart: 2)

        XCTAssertNil(model.errorMessage)
        let links = try vault.refundLinks(transactionId: refund.id)
        XCTAssertEqual(links.originals.map(\.refundPart), [2])
        XCTAssertEqual(links.originals.first?.refundPartLabel, "Auslage")
        XCTAssertEqual(links.assignedCents, 1)
        XCTAssertEqual(try vault.refundLinks(transactionId: original.id).refunds.map(\.amountCents), [1])
        XCTAssertEqual(kind(refund.id), .refund)

        model.unlinkRefundOriginals(refundID: refund.id)
        XCTAssertTrue(try vault.refundLinks(transactionId: refund.id).originals.isEmpty)
        XCTAssertEqual(kind(refund.id), RefundLinkKind.none)
    }

    /// Store credit that paid a part of a purchase (an incoming part of an outgoing booking)
    /// pays back the booking the credit was given for; the purchase is marked as what pays back.
    func testStoreCreditInAPurchasePaysBackABooking() throws {
        // the credit is spent after the booking it was given for: the selection looks ahead
        let originals = try pair().originals.sorted { ($0.bookingDate, $0.id) < ($1.bookingDate, $1.id) }
        let (returned, purchase) = (originals[0], originals[1])
        let vault = try XCTUnwrap(model.vault)
        try vault.setTransactionSplits(transactionId: purchase.id, splits: [
            TransactionSplitInput(categoryId: nil, amountCents: purchase.amountCents * 2, note: ""),
            TransactionSplitInput(categoryId: nil, amountCents: -purchase.amountCents, note: "Guthaben")
        ])
        // the purchase as a whole pays nothing back
        model.linkRefund(refundID: purchase.id, originalID: returned.id)
        XCTAssertNotNil(model.errorMessage)
        model.errorMessage = nil

        model.linkRefund(refundID: purchase.id, originalID: returned.id, refundPart: 2)

        XCTAssertNil(model.errorMessage)
        let links = try vault.refundLinks(transactionId: purchase.id)
        XCTAssertEqual(links.originals.map(\.id), [returned.id])
        XCTAssertEqual(links.refundTotalCents, -purchase.amountCents)
        XCTAssertEqual(links.assignedCents, min(-purchase.amountCents, -returned.amountCents))
        XCTAssertEqual(kind(purchase.id), .refund)
        XCTAssertEqual(kind(returned.id), .original)
        // the selection of the returned booking lists the credit part
        let offered = try vault.refundPartCandidates(transactionId: returned.id, paysBack: false, query: "", limit: 200)
        XCTAssertTrue(offered.contains { $0.id == purchase.id && $0.refundPart == 2 && $0.amountCents == -purchase.amountCents })

        model.unlinkRefundOriginals(refundID: purchase.id)
        XCTAssertEqual(kind(purchase.id), RefundLinkKind.none)
        XCTAssertEqual(kind(returned.id), RefundLinkKind.none)
    }

    /// The suggestions are candidates too, at most three, and never an already linked booking.
    func testSuggestionsAreAtMostThreeCandidates() throws {
        let vault = try XCTUnwrap(model.vault)
        let (refund, originals) = try pair()
        let candidates = Set(try vault.refundCandidates(transactionId: refund.id, query: "", limit: 100_000).map(\.id))
        let top = try vault.refundSuggestions(transactionId: refund.id)
        XCTAssertLessThanOrEqual(top.count, 3)
        XCTAssertTrue(top.allSatisfy { candidates.contains($0.booking.id) })
        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        XCTAssertFalse(try vault.refundSuggestions(transactionId: refund.id).contains { $0.booking.id == originals[0].id })
    }
}
