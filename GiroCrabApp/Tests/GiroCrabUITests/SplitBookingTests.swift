// test: serial-defaults
import XCTest
@testable import GiroCrabUI
import GiroCrabCore

/// #383: the split flow behind „Umsatz aufteilen (Splitbuchung)“ saves and reads back.
@MainActor
final class SplitBookingTests: XCTestCase {
    func testSplitSavesInvalidateTotalsOnlyOnSuccess() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        let half = booking.amountCents / 2
        let parts = [
            TransactionSplitInput(uuid: nil, categoryId: booking.categoryId, amountCents: half, note: "a"),
            TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: booking.amountCents - half, note: "b"),
        ]
        var version = model.totalsVersion
        XCTAssertNil(model.saveSplits(parts, for: booking))
        XCTAssertGreaterThan(model.totalsVersion, version)
        version = model.totalsVersion
        XCTAssertNotNil(model.saveSplits([parts[0]], for: booking))
        XCTAssertEqual(model.totalsVersion, version)
        XCTAssertNil(model.saveSplits([], for: booking))
        XCTAssertGreaterThan(model.totalsVersion, version)
    }

    func testSaveSplitsRoundTrips() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        let half = booking.amountCents / 2
        let parts = [
            TransactionSplitInput(uuid: nil, categoryId: booking.categoryId, amountCents: half, note: "a"),
            TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: booking.amountCents - half, note: "b"),
        ]
        XCTAssertNil(model.saveSplits(parts, for: booking))
        let stored = await model.splits(for: booking)
        XCTAssertEqual(stored?.count, 2)
        XCTAssertEqual(stored?.reduce(0) { $0 + $1.amountCents }, booking.amountCents)
    }

    /// #427: +59 EUR split into +200 EUR and a counter part of -141 EUR.
    func testMixedSignSplitIsValidAndRoundTrips() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        let big = booking.amountCents * 3
        let parts = [
            TransactionSplitInput(uuid: nil, categoryId: booking.categoryId, amountCents: big, note: "a"),
            TransactionSplitInput(uuid: nil, categoryId: nil, amountCents: booking.amountCents - big, note: "b"),
        ]
        XCTAssertNil(model.saveSplits(parts, for: booking))
        let stored = await model.splits(for: booking)
        XCTAssertEqual(stored?.map(\.amountCents).sorted(), [big, booking.amountCents - big].sorted())
    }

    /// Quick picks display the share in the booking's direction.
    func testPresetTextKeepsTheOddCentInTheRemainder() {
        XCTAssertEqual(SplitMath.presetText(50, bookingCents: 8_743), Money.plain(4_371))
        XCTAssertEqual(SplitMath.presetText(50, bookingCents: -8_743), Money.plain(4_371))
        XCTAssertEqual(SplitMath.presetText(50, bookingCents: .min), Money.plain(4_611_686_018_427_387_904))
    }

    /// #1663: „Umsatz aufteilen“ › „19 %“ hands the share to the sheet; closing it drops the share.
    func testRowMenuShareReachesTheSheet() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        model.openSplit(booking, percent: 19)
        XCTAssertEqual(model.splitBooking?.id, booking.id)
        XCTAssertEqual(model.splitPresetPercent, 19)
        model.splitBooking = nil
        XCTAssertNil(model.splitPresetPercent)
        model.openSplit(booking)
        XCTAssertNil(model.splitPresetPercent)
    }

    /// #1663: plain shares of the amount (not the VAT contained in it), labelled with the number.
    func testSharesArePlainSharesOfTheAmount() {
        XCTAssertEqual(SplitMath.shares, [7, 19, 50])
        XCTAssertEqual(splitShareCents(totalCents: 8_743, numerator: 19, denominator: 100), 1_661)
        XCTAssertEqual(splitShareCents(totalCents: 8_743, numerator: 7, denominator: 100), 612)
        for percent in SplitMath.shares {
            let label = SplitMath.label(percent)
            // the sign's position follows the locale ("19 %", "19%", "%19")
            XCTAssertTrue(label.contains("\(percent)") && label.contains("%"), label)
        }
    }

    /// #1663: an unsplit booking opens with the menu share in the second part; a split one with its
    /// stored parts in the booking's direction, the share not applied, and a changed amount noticed.
    func testOpeningPutsThePresetInPartTwoAndLoadsStoredParts() {
        let fresh = SplitMath.opening(bookingCents: -8_743, stored: [], presetPercent: 19)
        XCTAssertEqual(fresh.amounts, ["", Money.plain(1_661)])
        XCTAssertFalse(fresh.storedPartsDiffer)
        XCTAssertNil(fresh.ignoredShare)
        XCTAssertEqual(SplitMath.opening(bookingCents: -8_743, stored: [], presetPercent: nil).amounts, ["", ""])

        // expense −87,43 € stored as −50,00 € and −37,43 €
        let split = SplitMath.opening(bookingCents: -8_743, stored: [-5_000, -3_743], presetPercent: 19)
        XCTAssertEqual(split.amounts, [Money.plain(5_000), Money.plain(3_743)])
        XCTAssertFalse(split.storedPartsDiffer)
        XCTAssertEqual(split.ignoredShare, SplitMath.label(19))
        // #427 counter part comes back negative in the booking's direction
        XCTAssertEqual(SplitMath.opening(bookingCents: 5_900, stored: [20_000, -14_100], presetPercent: nil).amounts,
                       [Money.plain(20_000), Money.plain(-14_100)])
        // the booking became −89,00 € after the parts were saved
        XCTAssertTrue(SplitMath.opening(bookingCents: -8_900, stored: [-5_000, -3_743], presetPercent: nil).storedPartsDiffer)
    }

    /// #1663: the share buttons fill the part typed in last, else the last part, never the rest.
    func testShareButtonsFallBackToTheLastPart() {
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "a"), 1)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: nil), 2)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "removed"), 2)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "rest"), 2)
        XCTAssertNil(SplitMath.pickIndex(ids: ["rest"], target: nil))
    }

    func testEditorUsesTheCoreRemainderAndSignedSaveAmounts() {
        let draft = calculateSplitDraft(bookingCents: -8_743, splitOffTexts: ["50 %"])
        XCTAssertEqual(draft.restCents.map { SplitMath.amountText($0, bookingCents: -8_743) }, Money.plain(4_372))
        XCTAssertEqual(draft.amountsToSave, [-4_372, -4_371])
        XCTAssertNil(draft.problem)
        XCTAssertFalse(draft.restIsNegative)

        let counter = calculateSplitDraft(bookingCents: -5_900, splitOffTexts: ["200"])
        XCTAssertTrue(counter.restIsNegative)
        XCTAssertEqual(counter.amountsToSave, [14_100, -20_000])
    }
}
