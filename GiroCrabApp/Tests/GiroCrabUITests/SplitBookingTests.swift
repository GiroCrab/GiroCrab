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

    /// #1663: „Umsatz aufteilen“ › „19 %“ opens the sheet with that share; closing it drops the share.
    func testRowMenuShareOpensTheSheetWithThatShare() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let booking = try XCTUnwrap(model.bookings.first { $0.amountCents < 0 })
        model.openSplit(booking, percent: 19)
        XCTAssertEqual(model.splitBooking?.id, booking.id)
        XCTAssertEqual(model.splitPresetPercent, 19)
        XCTAssertEqual(SplitMath.share(of: 8_743, 19, 100), 1_661)
        model.splitBooking = nil
        XCTAssertNil(model.splitPresetPercent)
        model.openSplit(booking)
        XCTAssertNil(model.splitPresetPercent)
        XCTAssertEqual(SplitMath.shares, [7, 19, 50])
        XCTAssertEqual(SplitMath.share(of: 8_743, 7, 100), 612)
    }
}
