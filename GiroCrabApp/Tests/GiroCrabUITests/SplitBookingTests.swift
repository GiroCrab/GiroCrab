import XCTest
@testable import GiroCrabUI
import GiroCrabCore

/// #383: the split flow behind „Umsatz aufteilen (Splitbuchung)“ saves and reads back.
@MainActor
final class SplitBookingTests: XCTestCase {
    func testSaveSplitsRoundTrips() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        let half = booking.amountCents / 2
        let parts = [
            TransactionSplitInput(categoryId: booking.categoryId, amountCents: half, note: "a"),
            TransactionSplitInput(categoryId: nil, amountCents: booking.amountCents - half, note: "b"),
        ]
        XCTAssertNil(model.saveSplits(parts, for: booking))
        let stored = await model.splits(for: booking)
        XCTAssertEqual(stored?.count, 2)
        XCTAssertEqual(stored?.reduce(0) { $0 + $1.amountCents }, booking.amountCents)
    }

    /// #427: +59 EUR split into +200 EUR and a counter part of -141 EUR.
    func testMixedSignSplitIsValidAndRoundTrips() async throws {
        XCTAssertTrue(SplitMath.isValid(total: 5_900, parts: [20_000, -14_100]))
        XCTAssertEqual(SplitMath.rest(total: 5_900, splitOff: [20_000]), -14_100)
        XCTAssertFalse(SplitMath.isValid(total: 5_900, parts: [20_000, -14_000]))
        XCTAssertFalse(SplitMath.isValid(total: 5_900, parts: [5_900, 0]))
        XCTAssertFalse(SplitMath.isValid(total: 5_900, parts: [5_900]))
        // #1663: absurd parts never trap; a sum beyond Int64 is just invalid
        XCTAssertFalse(SplitMath.isValid(total: 1, parts: [Int64.max, Int64.max]))
        XCTAssertFalse(SplitMath.isValid(total: 1, parts: [Int64.min, Int64.min]))
        XCTAssertTrue(SplitMath.isValid(total: 8_743, parts: [223_372_036_854_783_743, 9_000_000_000_000_000_000, -9_223_372_036_854_775_000]))

        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        let big = booking.amountCents * 3
        let parts = [
            TransactionSplitInput(categoryId: booking.categoryId, amountCents: big, note: "a"),
            TransactionSplitInput(categoryId: nil, amountCents: booking.amountCents - big, note: "b"),
        ]
        XCTAssertNil(model.saveSplits(parts, for: booking))
        let stored = await model.splits(for: booking)
        XCTAssertEqual(stored?.map(\.amountCents).sorted(), [big, booking.amountCents - big].sorted())
    }

    /// #1663: a part is an amount or a share; the first part keeps the rest and the odd cent.
    func testSplitPartsReadSharesAndTheRestKeepsTheOddCent() {
        XCTAssertEqual(SplitMath.parse("50 %", total: 8_743), 4_371)
        XCTAssertEqual(SplitMath.parse("43,71", total: 8_743), 4_371)
        XCTAssertEqual(SplitMath.parse("−10 %", total: 8_743), -874)
        XCTAssertNil(SplitMath.parse("", total: 8_743))
        XCTAssertNil(SplitMath.parse("halb", total: 8_743))
        XCTAssertEqual(SplitMath.share(of: 8_743, 1, 2), 4_371)
        XCTAssertEqual(SplitMath.share(of: 10_000, 1, 3), 3_333)
        XCTAssertEqual(SplitMath.rest(total: 8_743, splitOff: [4_371]), 4_372)
        XCTAssertEqual(SplitMath.rest(total: 5_900, splitOff: [20_000]), -14_100)
        XCTAssertNil(SplitMath.rest(total: 8_743, splitOff: [4_371, nil]))
        XCTAssertNil(SplitMath.rest(total: 1, splitOff: [Int64.min]))
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
        XCTAssertEqual(SplitMath.share(of: 8_743, 19, 100), 1_661)
        XCTAssertEqual(SplitMath.share(of: 8_743, 7, 100), 612)
        for percent in SplitMath.shares {
            let label = SplitMath.label(percent)
            XCTAssertTrue(label.hasPrefix("\(percent)") && label.hasSuffix("%"), label)
        }
    }

    /// #1663: an unsplit booking opens with the menu share in the second part; a split one with its
    /// stored parts in the booking's direction, the share not applied, and a changed amount noticed.
    func testOpeningPutsThePresetInPartTwoAndLoadsStoredParts() {
        let fresh = SplitMath.opening(total: 8_743, sign: -1, stored: [], presetPercent: 19)
        XCTAssertEqual(fresh.amounts, ["", Money.plain(1_661)])
        XCTAssertFalse(fresh.storedPartsDiffer)
        XCTAssertNil(fresh.ignoredShare)
        XCTAssertEqual(SplitMath.opening(total: 8_743, sign: -1, stored: [], presetPercent: nil).amounts, ["", ""])

        // expense −87,43 € stored as −50,00 € and −37,43 €
        let split = SplitMath.opening(total: 8_743, sign: -1, stored: [-5_000, -3_743], presetPercent: 19)
        XCTAssertEqual(split.amounts, [Money.plain(5_000), Money.plain(3_743)])
        XCTAssertFalse(split.storedPartsDiffer)
        XCTAssertEqual(split.ignoredShare, SplitMath.label(19))
        // #427 counter part comes back negative in the booking's direction
        XCTAssertEqual(SplitMath.opening(total: 5_900, sign: 1, stored: [20_000, -14_100], presetPercent: nil).amounts,
                       [Money.plain(20_000), Money.plain(-14_100)])
        // the booking became −89,00 € after the parts were saved
        XCTAssertTrue(SplitMath.opening(total: 8_900, sign: -1, stored: [-5_000, -3_743], presetPercent: nil).storedPartsDiffer)
    }

    /// #1663: the share buttons fill the part typed in last, else the last part, never the rest.
    func testShareButtonsFallBackToTheLastPart() {
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "a"), 1)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: nil), 2)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "removed"), 2)
        XCTAssertEqual(SplitMath.pickIndex(ids: ["rest", "a", "b"], target: "rest"), 2)
        XCTAssertNil(SplitMath.pickIndex(ids: ["rest"], target: nil))
    }

    /// #1663: what the sheet shows and saves for the parts as typed.
    func testDraftValidityHintsAndSavedSigns() {
        // nothing typed yet: not savable
        let empty = SplitDraft(total: 8_743, splitOffTexts: [""])
        XCTAssertEqual(empty.rest, 8_743)
        XCTAssertFalse(empty.valid)
        XCTAssertEqual(empty.problem, .zeroPart)
        XCTAssertNil(empty.amountsToSave(sign: -1))

        let half = SplitDraft(total: 8_743, splitOffTexts: ["50 %"])
        XCTAssertTrue(half.valid)
        XCTAssertNil(half.problem)
        XCTAssertEqual(half.amountsToSave(sign: -1), [-4_372, -4_371])   // expense
        XCTAssertEqual(half.amountsToSave(sign: 1), [4_372, 4_371])      // income
        XCTAssertEqual(SplitDraft(total: 8_743, splitOffTexts: [" 43,71 "]).amountsToSave(sign: -1), [-4_372, -4_371])

        XCTAssertEqual(SplitDraft(total: 8_743, splitOffTexts: ["halb"]).problem, .unreadable)
        XCTAssertNil(SplitDraft(total: 8_743, splitOffTexts: ["halb"]).rest)
        XCTAssertEqual(SplitDraft(total: 8_743, splitOffTexts: ["0"]).problem, .zeroPart)
        XCTAssertEqual(SplitDraft(total: 8_743, splitOffTexts: ["87,43"]).problem, .zeroPart)   // rest 0
        XCTAssertEqual(SplitDraft(total: 8_743, splitOffTexts: ["50 %", ""]).problem, .zeroPart) // added, empty

        // #427: the rest counter-books, stays savable and is flagged
        let counter = SplitDraft(total: 5_900, splitOffTexts: ["200"])
        XCTAssertTrue(counter.valid)
        XCTAssertTrue(counter.restIsNegative)
        XCTAssertEqual(counter.amountsToSave(sign: 1), [-14_100, 20_000])
        XCTAssertFalse(half.restIsNegative)

        // absurd input never traps
        XCTAssertFalse(SplitDraft(total: 1, splitOffTexts: ["92233720368547758", "92233720368547758"]).valid)
    }
}
