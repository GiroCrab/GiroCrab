import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Standing orders can be created, changed and deleted (#496): the draft, its request and the
/// link from a „Geplant“ row to the stored order.
@MainActor
final class StandingOrderTests: XCTestCase {
    private func order(id: Int64 = 7, canChange: Bool = true, canDelete: Bool = true) -> StandingOrderInfo {
        StandingOrderInfo(
            id: id, accountId: 1, accountName: "Giro", orderId: "A1", recipientName: "Vermieter GmbH",
            recipientIban: "DE89370400440532013000", amountCents: 85_000, purpose: "Miete",
            firstDate: "2026-01-01", lastDate: nil, timeUnit: "M", interval: 1, execDay: 3,
            canChange: canChange, canDelete: canDelete)
    }

    func testDraftFromOrderKeepsEveryField() throws {
        let d = StandingDraft(order: order(), action: .change)
        XCTAssertEqual(d.orderRowID, 7)
        XCTAssertEqual(d.cents, 85_000)
        XCTAssertTrue(d.ibanValid)
        XCTAssertFalse(d.hasEnd)
        let req = try XCTUnwrap(d.request(demo: true))
        XCTAssertEqual(req.action, .change)
        XCTAssertEqual(req.orderRowId, 7)
        XCTAssertEqual(req.timeUnit, "M")
        XCTAssertEqual(req.interval, 1)
        XCTAssertEqual(req.execDay, 3)
        XCTAssertNil(req.lastDate)
        XCTAssertNil(req.recipientBic)
    }

    func testValidationRejectsBadInput() {
        var d = StandingDraft(order: order(), action: .change)
        XCTAssertTrue(d.isValid)
        d.amountText = "0"
        XCTAssertFalse(d.isValid)
        d.amountText = "10,00"
        d.iban = "DE00 0000 0000 0000 0000 00"
        XCTAssertFalse(d.isValid)
        d.iban = "DE89 3704 0044 0532 0130 00"
        d.hasEnd = true
        d.lastDate = Calendar.current.date(byAdding: .day, value: -1, to: d.firstDate) ?? d.firstDate
        XCTAssertFalse(d.isValid, "the last execution must not precede the first")
        d.hasEnd = false
        d.name = String(repeating: "x", count: 71)
        XCTAssertFalse(d.isValid)
    }

    func testRhythmWords() {
        var d = StandingDraft()
        XCTAssertEqual(d.rhythmText, String(localized: "monatlich"))
        d.interval = 3
        XCTAssertEqual(d.rhythmText, String(localized: "vierteljährlich"))
        d.unit = "W"
        d.interval = 1
        XCTAssertEqual(d.rhythmText, String(localized: "wöchentlich"))
        XCTAssertEqual(StandingDraft.weekdayName(1), Calendar.current.weekdaySymbols[1])
        XCTAssertEqual(StandingDraft.weekdayName(7), Calendar.current.weekdaySymbols[0])
    }

    func testPlannedRowFindsItsOrder() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.standingOrders = [order(id: 7)]
        func row(_ id: String, kind: PlannedKind = .standingOrder) -> PlannedPayment {
            PlannedPayment(id: id, kind: kind, accountId: 1, accountName: "Giro", date: "2026-12-03",
                           counterparty: "Vermieter GmbH", counterpartyIban: "", amountCents: -85_000,
                           purpose: "Miete", rhythmUnit: "M", rhythmInterval: 1)
        }
        XCTAssertEqual(model.standingOrder(for: row("standing-7-2026-12-03"))?.id, 7)
        XCTAssertNil(model.standingOrder(for: row("standing-8-2026-12-03")))
        XCTAssertNil(model.standingOrder(for: row("transfer-7", kind: .scheduledTransfer)))
    }

    func testFormFollowsTheBanksLimits() {
        let limits = StandingOrderLimits(
            minDaysAhead: 3, maxDaysAhead: 60, months: [1, 3, 6, 12], weeks: [], daysOfMonth: [1, 15, 31], weekdays: [])
        var d = StandingDraft(order: order(), action: .change)
        d.interval = 2
        d.execDay = 10
        XCTAssertEqual(d.allowedIntervals(limits), [1, 3, 6, 12])
        XCTAssertEqual(d.allowedDays(limits), [1, 15, 31])
        d.conform(to: limits)
        XCTAssertEqual(d.interval, 1)
        XCTAssertEqual(d.execDay, 15, "the nearest allowed day")
        let range = d.firstDateRange(limits)
        XCTAssertGreaterThanOrEqual(d.firstDate, range.lowerBound)
        XCTAssertLessThanOrEqual(d.firstDate, range.upperBound)
        let days = Calendar.current.dateComponents([.day], from: Calendar.current.startOfDay(for: Date()), to: range.lowerBound).day
        XCTAssertEqual(days, 3)
        // Weekly: the lists of the monthly rhythm do not apply.
        d.unit = "W"
        XCTAssertTrue(d.allowedIntervals(limits).isEmpty)
        XCTAssertTrue(d.allowedDays(limits).isEmpty)
    }

    func testUnknownLimitsOfferEverything() {
        var d = StandingDraft(order: order(), action: .change)
        d.conform(to: nil)
        XCTAssertEqual(d.interval, 1)
        XCTAssertEqual(d.execDay, 3)
        XCTAssertGreaterThanOrEqual(d.firstDate, StandingDraft.tomorrow.addingTimeInterval(-86_400))
    }
}
