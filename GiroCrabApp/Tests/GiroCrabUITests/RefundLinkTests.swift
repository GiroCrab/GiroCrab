import XCTest
import GiroCrabCore
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

    func testRelinkingMovesTheMarkerAndUnlinkingClearsIt() throws {
        let (refund, originals) = try pair()
        model.linkRefund(refundID: refund.id, originalID: originals[0].id)
        model.linkRefund(refundID: refund.id, originalID: originals[1].id)
        XCTAssertEqual(kind(originals[0].id), RefundLinkKind.none)
        XCTAssertEqual(kind(originals[1].id), .original)
        XCTAssertEqual(try XCTUnwrap(model.vault).refundLinks(transactionId: refund.id).original?.id, originals[1].id)

        model.unlinkRefund(refundID: refund.id)
        XCTAssertEqual(kind(refund.id), RefundLinkKind.none)
        XCTAssertEqual(kind(originals[1].id), RefundLinkKind.none)
        XCTAssertNil(try XCTUnwrap(model.vault).refundLinks(transactionId: refund.id).original)
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

    func testTheInspectorCardHasItsPlaceInTheStandardLayout() {
        let visible = InspectorLayout.standard.visible
        XCTAssertEqual(visible.firstIndex(of: .refunds), visible.firstIndex(of: .tags).map { $0 + 1 })
        // a layout saved before the card existed gets it at the end
        XCTAssertEqual(InspectorLayout(storageValue: "note,receipt").order.last, .rawData)
        XCTAssertTrue(InspectorLayout(storageValue: "note,receipt").visible.contains(.refunds))
    }
}
