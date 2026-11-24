import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Abo-Übersicht (#1787): Beträge, Vertragsfristen, Sortierung, Filter und Speicherung.
final class SubscriptionsTests: XCTestCase {

    private func day(_ s: String) -> Date { DateText.parseISODay(s)! }

    private func item(_ name: String, cents: Int64, group: SubscriptionGroup = .monthly, currency: String = "EUR",
                      next: String? = nil, details: SubscriptionDetails = SubscriptionDetails()) -> SubscriptionItem {
        SubscriptionItem(
            id: name, name: name, amountCents: cents, group: group, interval: RecurrenceInterval.manual(group),
            customCount: 0, currency: currency, accountId: nil, categorySymbol: "", nextDue: next, lastDate: nil,
            series: nil, manual: nil, details: details)
    }

    // MARK: amounts

    func testYearlyAndMonthlyAmountsFollowTheRhythm() {
        XCTAssertEqual(SubscriptionMath.yearlyCents(amountCents: -999, interval: .monthly), -11_988)
        XCTAssertEqual(SubscriptionMath.yearlyCents(amountCents: -2_990, interval: .quarterly), -11_960)
        XCTAssertEqual(SubscriptionMath.yearlyCents(amountCents: -6_000, interval: .halfYearly), -12_000)
        XCTAssertEqual(SubscriptionMath.yearlyCents(amountCents: -12_000, interval: .yearly), -12_000)
        XCTAssertEqual(SubscriptionMath.yearlyCents(amountCents: -6_000, interval: .custom, customCount: 2), -12_000)
        XCTAssertEqual(SubscriptionMath.monthlyCents(amountCents: -3_000, interval: .quarterly), -1_000)
        XCTAssertEqual(SubscriptionMath.monthlyCents(amountCents: -6_000, interval: .halfYearly), -1_000)
    }

    func testMonthlyAmountIsRoundedToTheNearestCent() {
        XCTAssertEqual(SubscriptionMath.monthlyCents(amountCents: -9_999, interval: .yearly), -833)
        XCTAssertEqual(SubscriptionMath.monthlyCents(amountCents: 9_999, interval: .yearly), 833)
        XCTAssertEqual(SubscriptionMath.monthlyCents(amountCents: -1_200, interval: .custom, customCount: 0), -100)
    }

    func testTotalsStayPerCurrencyWithEuroFirst() {
        let items = [item("a", cents: -1_000), item("b", cents: -500, currency: "USD"), item("c", cents: -250)]
        let totals = AppModel.subscriptionTotals(items, cents: \.amountCents)
        XCTAssertEqual(totals.map(\.currency), ["EUR", "USD"])
        XCTAssertEqual(totals.map(\.cents), [-1_250, -500])
    }

    // MARK: contract

    func testDeadlineIsTheContractEndMinusTheNoticePeriod() throws {
        var d = SubscriptionDetails()
        d.contractEnd = "2027-03-31"
        d.noticeValue = 3
        d.noticeUnit = .months
        let s = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-23")))
        XCTAssertEqual(DateText.iso(try XCTUnwrap(s.deadline)), "2026-12-31")
        XCTAssertEqual(s.daysToDeadline, 38)
        XCTAssertFalse(s.expired)
        d.noticeUnit = .weeks
        d.noticeValue = 4
        let w = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-23")))
        XCTAssertEqual(DateText.iso(try XCTUnwrap(w.deadline)), "2027-03-03")
    }

    func testPastTermRollsForwardByTheRenewalOrExpires() throws {
        var d = SubscriptionDetails()
        d.contractEnd = "2026-06-30"
        d.noticeValue = 1
        d.renewalMonths = 12
        let renewed = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-23")))
        XCTAssertEqual(DateText.iso(renewed.end), "2027-06-30")
        XCTAssertEqual(DateText.iso(try XCTUnwrap(renewed.deadline)), "2027-05-30")
        XCTAssertFalse(renewed.expired)
        d.renewalMonths = nil
        let expired = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-23")))
        XCTAssertTrue(expired.expired)
        XCTAssertEqual(DateText.iso(expired.end), "2026-06-30")
    }

    func testNoEndMeansNoStatus() {
        XCTAssertNil(SubscriptionContract.status(SubscriptionDetails(), today: day("2026-11-23")))
    }

    func testNoticeInDaysAndTheRunningTermWithoutRenewal() throws {
        var d = SubscriptionDetails()
        d.contractEnd = "2027-01-31"; d.noticeValue = 30; d.noticeUnit = .days
        let s = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-24")))
        XCTAssertEqual(DateText.iso(try XCTUnwrap(s.deadline)), "2027-01-01")
        XCTAssertEqual(s.daysToEnd, 68)
        XCTAssertFalse(s.deadlinePassed)

        // the end is today and one week was needed: the deadline lies a week back, the contract just ends
        var ending = SubscriptionDetails()
        ending.contractEnd = "2026-11-24"; ending.noticeValue = 1; ending.noticeUnit = .weeks
        let e = try XCTUnwrap(SubscriptionContract.status(ending, today: day("2026-11-24")))
        XCTAssertEqual(DateText.iso(try XCTUnwrap(e.deadline)), "2026-11-17")
        XCTAssertTrue(e.deadlinePassed)
        XCTAssertFalse(e.expired)
        XCTAssertEqual(e.daysToEnd, 0)
    }

    func testMissedDeadlineRunsTheContractIntoTheNextTerm() throws {
        var d = SubscriptionDetails()
        d.contractEnd = "2026-12-15"; d.noticeValue = 4; d.noticeUnit = .weeks; d.renewalMonths = 12
        let s = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-24")))
        // 17.11.2026 is gone, so the next chance is the term ending 15.12.2027
        XCTAssertEqual(DateText.iso(s.end), "2027-12-15")
        XCTAssertEqual(DateText.iso(try XCTUnwrap(s.deadline)), "2027-11-17")
        XCTAssertFalse(s.deadlinePassed)
    }

    func testCancelledContractKeepsItsEndAndEndsOnIt() throws {
        var d = SubscriptionDetails()
        d.cancelled = true; d.contractEnd = "2026-12-31"; d.renewalMonths = 12; d.noticeValue = 1
        let running = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-11-24")))
        XCTAssertEqual(DateText.iso(running.end), "2026-12-31", "a cancelled contract does not renew")
        XCTAssertFalse(running.ended)
        let endDay = try XCTUnwrap(SubscriptionContract.status(d, today: day("2026-12-31")))
        XCTAssertFalse(endDay.ended, "still running on the last day")
        let over = try XCTUnwrap(SubscriptionContract.status(d, today: day("2027-01-01")))
        XCTAssertTrue(over.ended, "the end is over")
        let later = try XCTUnwrap(SubscriptionContract.status(d, today: day("2027-02-01")))
        XCTAssertEqual(DateText.iso(later.end), "2026-12-31")
        XCTAssertTrue(later.ended)
    }

    func testNextPaymentOfAManualSubscriptionCountsFromItsAnchor() {
        let today = day("2026-11-23")
        XCTAssertEqual(SubscriptionContract.nextPayment(anchor: "2026-01-15", months: 1, today: today), "2026-12-15")
        XCTAssertEqual(SubscriptionContract.nextPayment(anchor: "2026-01-31", months: 3, today: today), "2027-01-31")
        XCTAssertEqual(SubscriptionContract.nextPayment(anchor: "2026-11-23", months: 12, today: today), "2026-11-23")
        XCTAssertEqual(SubscriptionContract.nextPayment(anchor: "2030-02-01", months: 12, today: today), "2030-02-01")
    }

    // MARK: sort and filter

    func testSortOrders() {
        var cancelled = SubscriptionDetails(); cancelled.cancelled = true
        var soon = SubscriptionDetails(); soon.contractEnd = "2099-01-31"; soon.noticeValue = 1
        var later = SubscriptionDetails(); later.contractEnd = "2099-06-30"; later.noticeValue = 1
        let a = item("Anna", cents: -500, group: .yearly, next: "2026-12-30")
        let b = item("bert", cents: -2_000, next: "2026-12-01", details: later)
        let c = item("Clara", cents: -100, next: nil, details: soon)
        let d = item("Dora", cents: -900, next: "2026-12-05", details: cancelled)
        let all = [d, c, b, a]
        XCTAssertEqual(SubscriptionSort.name.sorted(all).map(\.name), ["Anna", "bert", "Clara", "Dora"])
        // most expensive per year first (yearly cents are the more negative)
        XCTAssertEqual(SubscriptionSort.yearlyAmount.sorted(all).map(\.name), ["bert", "Dora", "Clara", "Anna"])
        XCTAssertEqual(SubscriptionSort.nextPayment.sorted(all).map(\.name), ["bert", "Dora", "Anna", "Clara"])
        // soonest deadline first; no deadline and cancelled contracts last
        XCTAssertEqual(SubscriptionSort.deadline.sorted(all).map(\.name), ["Clara", "bert", "Anna", "Dora"])
    }

    func testStatusAndKindFilters() {
        var cancelledRunning = SubscriptionDetails(); cancelledRunning.cancelled = true; cancelledRunning.contractEnd = "2099-01-31"
        var ended = SubscriptionDetails(); ended.cancelled = true; ended.contractEnd = "2020-01-31"
        var due = SubscriptionDetails()
        due.contractEnd = DateText.iso(Calendar.current.date(byAdding: .day, value: 20, to: Date())!)
        due.noticeValue = 1
        due.noticeUnit = .weeks
        let running = item("a", cents: -100)
        let waiting = item("b", cents: -100, details: cancelledRunning)
        let over = item("c", cents: -100, details: ended)
        let urgent = item("d", cents: -100, details: due)
        let income = item("e", cents: 5_000)

        XCTAssertTrue(SubscriptionStatusFilter.all.matches(over))
        XCTAssertTrue(SubscriptionStatusFilter.active.matches(running))
        XCTAssertTrue(SubscriptionStatusFilter.active.matches(waiting), "cancelled but still running is active")
        XCTAssertFalse(SubscriptionStatusFilter.active.matches(over))
        XCTAssertTrue(SubscriptionStatusFilter.inactive.matches(over))
        XCTAssertFalse(SubscriptionStatusFilter.inactive.matches(waiting))
        XCTAssertTrue(SubscriptionStatusFilter.cancelled.matches(waiting))
        XCTAssertTrue(SubscriptionStatusFilter.cancelled.matches(over))
        XCTAssertFalse(SubscriptionStatusFilter.cancelled.matches(running))
        XCTAssertTrue(SubscriptionStatusFilter.deadlineSoon.matches(urgent))
        XCTAssertFalse(SubscriptionStatusFilter.deadlineSoon.matches(running))
        XCTAssertFalse(SubscriptionStatusFilter.deadlineSoon.matches(waiting))

        XCTAssertTrue(SubscriptionKindFilter.income.matches(income))
        XCTAssertFalse(SubscriptionKindFilter.income.matches(running))
        XCTAssertTrue(SubscriptionKindFilter.expenses.matches(running))
        XCTAssertTrue(SubscriptionKindFilter.all.matches(income))
    }

    func testAttentionColours() {
        var soon = SubscriptionDetails()
        soon.contractEnd = DateText.iso(Calendar.current.date(byAdding: .day, value: 25, to: Date())!)
        soon.noticeValue = 10; soon.noticeUnit = .days
        var far = soon; far.contractEnd = DateText.iso(Calendar.current.date(byAdding: .day, value: 100, to: Date())!)
        var waiting = SubscriptionDetails(); waiting.cancelled = true; waiting.contractEnd = "2099-01-31"
        var over = SubscriptionDetails(); over.cancelled = true; over.contractEnd = "2020-01-31"
        var noEnd = SubscriptionDetails(); noEnd.cancelled = true
        XCTAssertEqual(item("a", cents: -1, details: soon).attention, .deadlineSoon)
        XCTAssertEqual(item("b", cents: -1, details: far).attention, .none)
        XCTAssertEqual(item("c", cents: -1, details: waiting).attention, .cancelledRunning)
        XCTAssertEqual(item("d", cents: -1, details: over).attention, .ended)
        XCTAssertEqual(item("e", cents: -1, details: noEnd).attention, .ended, "no end known: nothing to wait for")
        XCTAssertEqual(item("f", cents: -1).attention, .none)
        XCTAssertNotNil(SubscriptionAttention.deadlineSoon.color)
        XCTAssertNotNil(SubscriptionAttention.cancelledRunning.color)
        XCTAssertNil(SubscriptionAttention.ended.color)
    }

    func testAmountBasisConvertsFromTheYearlyAmount() {
        let quarterly = item("q", cents: -2_990, group: .quarterly)
        XCTAssertEqual(quarterly.cents(in: .original), -2_990)
        XCTAssertEqual(quarterly.cents(in: .quarterly), -2_990)
        XCTAssertEqual(quarterly.cents(in: .yearly), -11_960)
        XCTAssertEqual(quarterly.cents(in: .halfYearly), -5_980)
        XCTAssertEqual(quarterly.cents(in: .monthly), -997) // 11960 / 12 = 996.67
        XCTAssertEqual(quarterly.period(in: .original), SubscriptionGroup.quarterly.perPeriod)
        XCTAssertEqual(quarterly.period(in: .monthly), SubscriptionAmountBasis.monthly.period)
        XCTAssertEqual(item("i", cents: 12_000, group: .yearly).cents(in: .monthly), 1_000)
    }

    // MARK: storage

    func testDataSurvivesEncodingAndBrokenJSONGivesNothing() {
        var d = SubscriptionDetails()
        d.note = "3 Monate Frist"; d.noticeValue = 3; d.contractEnd = "2027-01-31"; d.renewalMonths = 12; d.cancelled = true
        let data = SubscriptionData(
            entries: ["key-a": d],
            manual: [ManualSubscription(id: "m1", name: "Verein", amountCents: -5_000, group: SubscriptionGroup.yearly.rawValue,
                                        currency: "EUR", startDate: "2026-02-01")])
        XCTAssertEqual(SubscriptionStore.decode(SubscriptionStore.encode(data)), data)
        XCTAssertEqual(SubscriptionStore.decode(nil), SubscriptionData())
        XCTAssertEqual(SubscriptionStore.decode("nicht json"), SubscriptionData())
    }

    func testNotesOfTheFirstVersionMoveIntoTheDetails() {
        let migrated = SubscriptionStore.decode(nil, legacy: #"{"k":"läuft bis Dezember"}"#)
        XCTAssertEqual(migrated.entries["k"]?.note, "läuft bis Dezember")
        XCTAssertFalse(migrated.entries["k"]?.cancelled ?? true)
    }

    func testDetailsWithMissingFieldsStillDecode() throws {
        let d = try JSONDecoder().decode(SubscriptionDetails.self, from: Data(#"{"note":"x"}"#.utf8))
        XCTAssertEqual(d.note, "x")
        XCTAssertNil(d.noticeValue)
        XCTAssertEqual(d.noticeUnit, .months)
    }

    // MARK: model

    @MainActor
    func testMarkingKeepsDetailsPerSeriesAndPersistsInTheVault() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        let first = try XCTUnwrap(model.recurring.first { $0.amountCents < 0 }, "the demo vault has recurring expenses")

        XCTAssertFalse(model.isSubscription(first))
        model.setSubscription(first, marked: true)
        var item = try XCTUnwrap(model.subscriptionItems.first { $0.id == first.seriesKey })
        var details = item.details
        details.note = "Kündigung bis 30.09."
        details.noticeValue = 6
        details.noticeUnit = .weeks
        model.setSubscriptionDetails(item, details)
        XCTAssertFalse(model.subscriptionCandidates.contains { $0.seriesKey == first.seriesKey })

        // read back from the vault, not from memory
        model.subscriptionData = SubscriptionData()
        model.loadSubscriptionData()
        item = try XCTUnwrap(model.subscriptionItems.first { $0.id == first.seriesKey })
        XCTAssertEqual(item.details, details)

        model.setSubscription(first, marked: false)
        XCTAssertFalse(model.isSubscription(first))
    }

    @MainActor
    func testManualSubscriptionCanBeAddedChangedAndRemoved() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.addManualSubscription(name: "Fitnessstudio", amountCents: -2_990, group: .quarterly, currency: "EUR", startDate: "2026-01-10")
        let item = try XCTUnwrap(model.subscriptionItems.first { $0.isManual })
        XCTAssertEqual(item.amountCents, -2_990)
        XCTAssertFalse(item.isIncome)
        XCTAssertEqual(item.group, .quarterly)
        XCTAssertEqual(item.yearlyCents, -11_960)
        XCTAssertNotNil(item.nextDue)

        var m = try XCTUnwrap(item.manual)
        m.name = "Studio"
        model.updateManualSubscription(m)
        model.subscriptionData = SubscriptionData()
        model.loadSubscriptionData()
        XCTAssertEqual(model.subscriptionItems.first { $0.isManual }?.name, "Studio")

        model.removeManualSubscription(id: m.id)
        XCTAssertTrue(model.subscriptionItems.filter(\.isManual).isEmpty)
    }

    @MainActor
    func testBookingsOfAMarkedSeriesAreLoaded() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        let first = try XCTUnwrap(model.recurring.first { $0.amountCents < 0 })
        model.setSubscription(first, marked: true)
        await model.reloadSubscriptionBookings()
        let bookings = try XCTUnwrap(model.subscriptionBookings[first.seriesKey])
        XCTAssertFalse(bookings.isEmpty)
        XCTAssertTrue(bookings.allSatisfy { $0.recurrenceKey == first.baseKey })
    }

    @MainActor
    func testRhythmChangeKeepsTheSubscriptionAndItsFields() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        // a rhythm change can give a series another key; every expense series must survive it
        for s in model.recurring.filter({ $0.amountCents < 0 }) {
            model.setSubscription(s, marked: true)
            var item = try XCTUnwrap(model.subscriptionItems.first { $0.id == s.seriesKey })
            var d = item.details
            d.note = "Frist beachten"; d.noticeValue = 2
            model.setSubscriptionDetails(item, d)

            let target: RecurrenceInterval = s.interval == .quarterly ? .yearly : .quarterly
            model.changeSubscriptionRhythm(s, to: target)

            item = try XCTUnwrap(model.subscriptionItems.first { $0.series?.baseKey == s.baseKey },
                                 "\(s.name) vanished after the rhythm change")
            XCTAssertEqual(item.series?.interval, target)
            XCTAssertEqual(item.details, d)
            XCTAssertNil(model.subscriptionData.entries[s.seriesKey == item.id ? "" : s.seriesKey],
                         "the old key must not stay behind")
            model.setSubscription(item.series!, marked: false)
        }
    }

    @MainActor
    func testIncomesCanBeMarkedAndAreKeptApartFromExpenses() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        let income = try XCTUnwrap(model.recurring.first { $0.amountCents > 0 }, "the demo vault has a recurring income")
        XCTAssertTrue(model.subscriptionCandidates.contains { $0.seriesKey == income.seriesKey })
        model.setSubscription(income, marked: true)
        let item = try XCTUnwrap(model.subscriptionItems.first { $0.id == income.seriesKey })
        XCTAssertTrue(item.isIncome)
        XCTAssertGreaterThan(item.monthlyCents, 0)
        model.addManualSubscription(name: "Miete Garage", amountCents: 8_000, group: .monthly, currency: "EUR", startDate: "2026-01-01")
        let manual = try XCTUnwrap(model.subscriptionItems.first { $0.isManual })
        XCTAssertTrue(manual.isIncome)
        XCTAssertEqual(manual.amountCents, 8_000, "an income keeps its sign")
        let all = model.subscriptionItems
        XCTAssertEqual(all.filter(\.isIncome).count, 2)
    }

    @MainActor
    func testRemovingTheContractEndRemovesTheDeadline() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        let first = try XCTUnwrap(model.recurring.first { $0.amountCents < 0 })
        model.setSubscription(first, marked: true)
        var item = try XCTUnwrap(model.subscriptionItems.first { $0.id == first.seriesKey })
        var d = item.details
        d.contractEnd = "2099-01-31"; d.noticeValue = 1; d.noticeUnit = .weeks; d.renewalMonths = 1
        model.setSubscriptionDetails(item, d)
        item = try XCTUnwrap(model.subscriptionItems.first { $0.id == first.seriesKey })
        XCTAssertNotNil(item.status)
        d.contractEnd = nil
        model.setSubscriptionDetails(item, d)
        model.subscriptionData = SubscriptionData()
        model.loadSubscriptionData()
        item = try XCTUnwrap(model.subscriptionItems.first { $0.id == first.seriesKey })
        XCTAssertNil(item.details.contractEnd)
        // the next payment stands in for the end now
        XCTAssertNotNil(item.status, "the next payment stands in for the end")
    }

    @MainActor
    func testOneOffsOfThePayeeAreNotPartOfTheSeries() async throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        for s in model.recurring { model.setSubscription(s, marked: true) }
        await model.reloadSubscriptionBookings()
        XCTAssertFalse(model.subscriptionBookings.isEmpty)
        for s in model.recurring {
            for b in model.subscriptionBookings[s.seriesKey] ?? [] {
                XCTAssertLessThanOrEqual(abs(b.amountCents), abs(s.amountCents) * 4, "\(s.name): \(b.amountCents)")
            }
        }
    }

    @MainActor
    func testNextPaymentStandsInForAMissingContractEnd() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.refreshRecurring()
        model.addManualSubscription(name: "Gym", amountCents: -2_990, group: .monthly, currency: "EUR", startDate: "2026-01-10")
        var item = try XCTUnwrap(model.subscriptionItems.first { $0.isManual })
        XCTAssertNil(item.status, "no notice period: nothing to count back from")
        var d = item.details
        d.noticeValue = 7; d.noticeUnit = .days
        model.setSubscriptionDetails(item, d)
        item = try XCTUnwrap(model.subscriptionItems.first { $0.isManual })
        let status = try XCTUnwrap(item.status)
        XCTAssertEqual(DateText.iso(status.end) >= DateText.iso(Date()), true)
        XCTAssertNotNil(status.deadline)
        d.cancelled = true
        model.setSubscriptionDetails(item, d)
        item = try XCTUnwrap(model.subscriptionItems.first { $0.isManual })
        XCTAssertNotNil(item.effectiveDetails.contractEnd, "a cancelled contract without an end runs until the next payment")
    }

    func testCancelledContractByEndDay() throws {
        func make(_ offset: Int) -> SubscriptionItem {
            var d = SubscriptionDetails()
            d.cancelled = true
            d.contractEnd = DateText.iso(Calendar.current.date(byAdding: .day, value: offset, to: Date())!)
            return item("x", cents: -100, details: d)
        }
        XCTAssertTrue(make(-1).isEnded, "ended yesterday")
        XCTAssertFalse(make(0).isEnded, "ends today: still running today")
        XCTAssertFalse(make(1).isEnded, "runs until tomorrow")
        XCTAssertEqual(make(-1).status?.ended, true)
        XCTAssertEqual(make(1).status?.daysToEnd, 1)
    }

    func testLateCancellationRunsAnotherTerm() throws {
        var d = SubscriptionDetails()
        d.cancelled = true; d.contractEnd = "2099-11-25"; d.noticeValue = 1; d.noticeUnit = .months
        d.cancelledOn = "2099-11-10"
        let late = item("l", cents: -100, details: d)
        XCTAssertEqual(late.cancelledInTime, false)
        XCTAssertEqual(DateText.iso(try XCTUnwrap(late.status?.end)), "2099-12-25", "one more monthly term")
        XCTAssertEqual(late.status?.missedDeadline.map(DateText.iso), "2099-10-25")
        d.cancelledOn = "2099-10-20"
        let early = item("e", cents: -100, details: d)
        XCTAssertEqual(early.cancelledInTime, true)
        XCTAssertEqual(DateText.iso(try XCTUnwrap(early.status?.end)), "2099-11-25")
    }

    func testPriceChangeNeedsTwoEqualPaymentsBeforeTheRise() {
        XCTAssertEqual(SubscriptionPriceChange.detect([-1_199, -999, -999]), SubscriptionPriceChange(old: -999, new: -1_199))
        XCTAssertNil(SubscriptionPriceChange.detect([-999, -999, -999]))
        XCTAssertNil(SubscriptionPriceChange.detect([-899, -999, -999]), "a cut is no rise")
        XCTAssertNil(SubscriptionPriceChange.detect([-1_199, -999]), "too few payments")
        XCTAssertNil(SubscriptionPriceChange.detect([-1_300, -1_100, -1_200]), "a varying bill is no rise")
        XCTAssertEqual(SubscriptionPriceChange.detect([1_500, 1_000, 1_000])?.delta, 500)
    }

    func testCancellationTimelinessAndTrial() throws {
        var d = SubscriptionDetails()
        d.cancelled = true; d.contractEnd = "2099-12-31"; d.noticeValue = 1; d.noticeUnit = .months
        d.cancelledOn = "2099-11-30"
        XCTAssertEqual(item("a", cents: -1, details: d).cancelledInTime, true)
        d.cancelledOn = "2099-12-15"
        XCTAssertEqual(item("b", cents: -1, details: d).cancelledInTime, false)
        d.cancelledOn = nil
        XCTAssertNil(item("c", cents: -1, details: d).cancelledInTime)
        var t = SubscriptionDetails()
        t.trialEnd = "2099-01-01"
        XCTAssertNotNil(item("t", cents: -1, details: t).trialEndDate)
        t.trialEnd = "2020-01-01"
        XCTAssertNil(item("t", cents: -1, details: t).trialEndDate, "a trial that is over shows nothing")
        let data = try JSONDecoder().decode(SubscriptionDetails.self, from: JSONEncoder().encode(d))
        XCTAssertEqual(data, d)
    }
}
