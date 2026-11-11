import XCTest
@testable import GiroCrabUI

/// Print dialog report builder (GitHub #53, part 2).
@MainActor
final class PrintReportTests: XCTestCase {
    private let rows = [
        PrintRow(id: 1, accountId: 10, accountName: "Giro", date: "2025-03-01", amountCents: 250_000,
                 counterparty: "Arbeitgeber", purpose: "Gehalt März\nPersonalnummer 42", category: "Einkommen"),
        PrintRow(id: 2, accountId: 10, accountName: "Giro", date: "2025-03-02", amountCents: -4_599,
                 counterparty: "Supermarkt <Nord>", purpose: "Einkauf", category: "Lebensmittel", note: "Wocheneinkauf"),
        PrintRow(id: 3, accountId: 20, accountName: "Kreditkarte", date: "2025-03-02", amountCents: -1_200,
                 counterparty: "Café", purpose: "Kartenzahlung"),
    ]

    private func report(_ rows: [PrintRow]? = nil) -> PrintReport {
        PrintReport(title: "Übersicht", periodLabel: "März 2025", rows: rows ?? self.rows,
                    accounts: [PrintAccount(id: 10, name: "Giro", iban: "DE02120300000000202051", balanceCents: 1_000_000,
                                            balanceDate: "2025-03-31", currency: "EUR"),
                               PrintAccount(id: 20, name: "Kreditkarte", iban: "", balanceCents: -1_200,
                                            balanceDate: "2025-03-31", currency: "EUR")])
    }

    func testDirectionFilterAndSortOrder() {
        var o = PrintOptions()
        XCTAssertEqual(PrintReportBuilder.rows(rows, options: o).map(\.id), [3, 2, 1])
        o.newestFirst = false
        XCTAssertEqual(PrintReportBuilder.rows(rows, options: o).map(\.id), [1, 2, 3])
        o.direction = .income
        XCTAssertEqual(PrintReportBuilder.rows(rows, options: o).map(\.id), [1])
        o.direction = .expenses
        XCTAssertEqual(PrintReportBuilder.rows(rows, options: o).map(\.id), [2, 3])
    }

    func testTotals() {
        let t = PrintReportBuilder.totals(rows)
        XCTAssertEqual(t.incomeCents, 250_000)
        XCTAssertEqual(t.expenseCents, -5_799)
        XCTAssertEqual(t.netCents, 244_201)
    }

    func testHTMLEscapesAndHonoursOptions() {
        var o = PrintOptions()
        o.showCategory = false
        o.showNote = false
        o.firstLineOnly = true
        let html = PrintReportBuilder.html(report(), options: o)
        XCTAssertTrue(html.contains("Supermarkt &lt;Nord&gt;"))
        XCTAssertFalse(html.contains("<Nord>"))
        XCTAssertTrue(html.contains("Gehalt März"))
        XCTAssertFalse(html.contains("Personalnummer"))
        XCTAssertFalse(html.contains("Wocheneinkauf"))
        XCTAssertFalse(html.contains("Lebensmittel"))
    }

    func testGroupByAccountAddsSectionsPerAccount() {
        var o = PrintOptions()
        o.groupByAccount = true
        let html = PrintReportBuilder.html(report(), options: o)
        XCTAssertEqual(html.components(separatedBy: "<h2>").count - 1, 2)
        XCTAssertTrue(html.contains("DE02120300000000202051"))
        o.groupByAccount = false
        XCTAssertEqual(PrintReportBuilder.html(report(), options: o).components(separatedBy: "<h2>").count - 1, 0)
    }

    func testDailyBalanceOncePerDay() {
        var o = PrintOptions()
        o.showDailyBalance = true
        var r = report()
        r.dailyBalances = [0: ["2025-03-01": 777_701, "2025-03-02": 777_702]]
        let html = PrintReportBuilder.html(r, options: o)
        // two bookings on 2025-03-02, but its end-of-day balance is printed once
        XCTAssertEqual(html.components(separatedBy: Money.format(777_702)).count - 1, 1)
        XCTAssertEqual(html.components(separatedBy: Money.format(777_701)).count - 1, 1)
    }

    func testEmptySelection() {
        let html = PrintReportBuilder.html(report([]), options: PrintOptions())
        XCTAssertTrue(html.contains("Keine Buchungen ausgewählt."))
    }

    func testPrintSourceFollowsTheListFilter() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.period = .all
        let source = try XCTUnwrap(model.printSource())
        XCTAssertEqual(source.rows.count, Int(model.page?.totalCount ?? 0))
        model.period = .lastMonth
        let (from, to) = model.periodRange
        let month = try XCTUnwrap(model.printSource())
        XCTAssertTrue(month.rows.allSatisfy { $0.date >= from! && $0.date <= to! })
    }

    func testOptionsRoundTrip() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "PrintReportTests"))
        defaults.removePersistentDomain(forName: "PrintReportTests")
        var o = PrintOptions()
        o.direction = .expenses
        o.groupByAccount = true
        o.save(defaults)
        XCTAssertEqual(PrintOptions.load(defaults), o)
    }
}
