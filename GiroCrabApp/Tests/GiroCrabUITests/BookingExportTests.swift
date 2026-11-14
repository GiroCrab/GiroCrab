import XCTest
import PDFKit
@testable import GiroCrabUI
import GiroCrabCore

@MainActor
final class BookingExportTests: XCTestCase {
    private let rows = [
        PrintRow(id: 1, accountId: 10, accountName: "Giro", date: "2025-03-01", amountCents: 250_000,
                 counterparty: "Arbeitgeber", purpose: "Gehalt März\nPersonalnummer 42", category: "Einkommen"),
        PrintRow(id: 2, accountId: 10, accountName: "Giro", date: "2025-03-02", amountCents: -4_599,
                 counterparty: "Supermarkt <Nord>", purpose: "Einkauf", category: "Lebensmittel", note: "Wocheneinkauf"),
    ]

    private func report() -> PrintReport {
        var r = PrintReport(title: "Giro", periodLabel: "März 2025", rows: rows,
                            accounts: [PrintAccount(id: 10, name: "Giro", iban: "DE02120300000000202051",
                                                    balanceCents: 1_000_000, balanceDate: "2025-03-31", currency: "EUR",
                                                    owner: "Max Mustermann")])
        r.dailyBalances = [0: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000],
                           10: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000]]
        return r
    }

    func testCSVIncludesDailyBalanceAndEscapes() throws {
        var o = ExportOptions()
        o.format = .csv
        o.showDailyBalance = true
        o.showCurrentBalance = true
        o.newestFirst = false
        let csv = BookingExportBuilder.csv(report(), options: o)
        XCTAssertTrue(csv.hasPrefix("\u{FEFF}"))
        XCTAssertTrue(csv.contains("# Giro;10000,00;EUR;2025-03-31"))
        XCTAssertTrue(csv.contains("Tagessaldo"))
        XCTAssertTrue(csv.contains("Supermarkt <Nord>"))
        let lines = csv.split(separator: "\r\n").map(String.init)
        let day1 = try XCTUnwrap(lines.first { $0.hasPrefix("2025-03-01") })
        let day2 = try XCTUnwrap(lines.first { $0.hasPrefix("2025-03-02") })
        XCTAssertTrue(day1.contains(";10045,99"))
        XCTAssertTrue(day2.contains(";10000,00"))
    }

    func testCSVWithoutBalancesOmitsColumns() {
        var o = ExportOptions()
        o.format = .csv
        o.showDailyBalance = false
        o.showCurrentBalance = false
        o.showNote = false
        let csv = BookingExportBuilder.csv(report(), options: o)
        XCTAssertFalse(csv.contains("Tagessaldo"))
        XCTAssertFalse(csv.contains("\u{FEFF}# "))
        XCTAssertFalse(csv.contains("Wocheneinkauf"))
    }

    func testCamt053V02FromDemoBookings() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        let account = try! XCTUnwrap(model.allAccounts.first { !$0.iban.isEmpty })
        let bookings = model.exportBookings().filter { $0.accountId == account.id }.prefix(5).map { $0 }
        XCTAssertFalse(bookings.isEmpty)
        let first = bookings.map(\.bookingDate).min()!
        let onFirst = bookings.filter { $0.bookingDate == first }
        let opening = BookingExportBuilder.camtOpeningBalance(
            endOfDay: [:], firstDate: first, bookingsOnFirstDay: Array(onFirst))
        let xml = BookingExportBuilder.camt053(
            account: account, bookings: Array(bookings), format: .camt053v02,
            openingCents: opening.cents, openingDate: opening.date)
        XCTAssertTrue(xml.contains("camt.053.001.02"))
        XCTAssertTrue(xml.contains("<BkToCstmrStmt>"))
        XCTAssertTrue(xml.contains("<Ntry>"))
        XCTAssertTrue(xml.contains("<Cd>OPBD</Cd>"))
        XCTAssertTrue(xml.contains("<Cd>CLBD</Cd>"))
        XCTAssertTrue(xml.contains(account.iban.replacingOccurrences(of: " ", with: ""))
                      || xml.contains("<IBAN>"))
    }

    func testCamt053V08UsesPtyWrapper() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let account = try! XCTUnwrap(model.allAccounts.first)
        let bookings = Array(model.exportBookings().filter { $0.accountId == account.id }.prefix(3))
        guard !bookings.isEmpty else { return }
        let xml = BookingExportBuilder.camt053(
            account: account, bookings: bookings, format: .camt053v08,
            openingCents: 100_000, openingDate: bookings.map(\.bookingDate).min()!)
        XCTAssertTrue(xml.contains("camt.053.001.08"))
        // v08 party name under Pty
        if xml.contains("<Cdtr>") || xml.contains("<Dbtr>") {
            XCTAssertTrue(xml.contains("<Pty>"), xml)
        }
    }

    func testCamtOpeningBalanceFromEndOfDay() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let b = try! XCTUnwrap(model.exportBookings().first)
        let day = b.bookingDate
        let eod: Int64 = 50_000
        let o = BookingExportBuilder.camtOpeningBalance(
            endOfDay: [day: eod], firstDate: day, bookingsOnFirstDay: [b])
        XCTAssertEqual(o.cents, eod - b.amountCents)
        XCTAssertEqual(o.date, day)
    }

    func testPrintMappingIsAlwaysWhiteLabel() {
        var o = ExportOptions()
        o.showDailyBalance = false
        o.showCurrentBalance = false
        let p = o.asPrintOptions
        XCTAssertEqual(p.logo, .off)
        XCTAssertFalse(p.showDailyBalance)
        XCTAssertFalse(p.showCurrentBalance)
    }

    func testDefaultFilenameUsesAccountAndDateRange() {
        let name = BookingExportBuilder.defaultFilename(
            accounts: report().accounts, rows: rows, format: .pdf)
        XCTAssertTrue(name.hasPrefix("Umsaetze-Giro-"), name)
        XCTAssertTrue(name.contains("2025-03-01_2025-03-02"), name)
        XCTAssertTrue(name.hasSuffix(".pdf"), name)

        let camt = BookingExportBuilder.defaultFilename(
            accounts: report().accounts, rows: rows, format: .camt053v08)
        XCTAssertTrue(camt.contains("CAMT053-001-08"), camt)

        let prepared = BookingExportBuilder.prepareStatement(report())
        XCTAssertEqual(prepared.title, "Giro")
        XCTAssertEqual(prepared.periodLabel, "01.03.2025 – 02.03.2025")
    }

    func testStatementStyleUsesGermanDatesIBANAndPeriodBalances() {
        var o = ExportOptions()
        o.format = .pdf
        var prepared = BookingExportBuilder.prepareStatement(report())
        prepared.dailyBalances = [10: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000],
                                  0: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000]]
        let html = PrintReportBuilder.html(prepared, options: o.asPrintOptions, paper: .a4)
        XCTAssertTrue(o.asPrintOptions.statementStyle)
        XCTAssertFalse(html.contains("Kontoauszug"), html)
        XCTAssertTrue(html.contains("Giro"), html)
        XCTAssertTrue(html.contains("01.03.2025") || html.contains("02.03.2025"), html)
        XCTAssertTrue(html.contains("DE02 1203 0000 0000 2020 51") || html.contains("DE021203"), html)
        XCTAssertTrue(html.contains("Max Mustermann"), html)
        XCTAssertTrue(html.contains("Anfangssaldo"), html)
        XCTAssertTrue(html.contains("Endsaldo"), html)
        XCTAssertTrue(html.contains("period-balances"), html)
        XCTAssertTrue(html.contains("Erstellt am"), html) // in page footer script
        XCTAssertFalse(html.contains("Einnahmen"), html)
        XCTAssertFalse(html.contains("Ausgaben"), html)
        // Opening 01.03 = 10045,99 − 2500,00 = 7545,99
        XCTAssertTrue(html.contains("7.545,99") || html.contains("7545,99"), html)
        XCTAssertTrue(html.contains("10.000,00") || html.contains("10000,00"), html)
    }

    func testOpeningAndClosingBalanceFromDaily() {
        let open = PrintReportBuilder.openingBalance(
            rows: rows, endOfDay: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000])
        XCTAssertEqual(open?.date, "2025-03-01")
        XCTAssertEqual(open?.cents, 1_004_599 - 250_000)
        let close = PrintReportBuilder.closingBalance(
            rows: rows, endOfDay: ["2025-03-01": 1_004_599, "2025-03-02": 1_000_000])
        XCTAssertEqual(close?.date, "2025-03-02")
        XCTAssertEqual(close?.cents, 1_000_000)
    }



    func testExportOptionsRoundTripAndXmlMigration() throws {
        var o = ExportOptions()
        o.format = .camt053v02
        o.scope = .selection
        o.periodPreset = .thisYear
        o.selectedAccountIDs = [42]
        o.accountOutput = .perAccount
        o.showCategory = false
        o.showNote = true
        o.showDailyBalance = false
        let defaults = UserDefaults(suiteName: "BookingExportTests")!
        defaults.removePersistentDomain(forName: "BookingExportTests")
        o.save(defaults)
        let loaded = ExportOptions.load(defaults)
        XCTAssertEqual(loaded.format, .camt053v02)
        XCTAssertEqual(loaded.scope, .selection)
        XCTAssertEqual(loaded.periodPreset, .thisYear)
        XCTAssertEqual(loaded.selectedAccountIDs, [42])
        XCTAssertEqual(loaded.accountOutput, .perAccount)
        XCTAssertFalse(loaded.showCategory)
        XCTAssertTrue(loaded.showNote)
        XCTAssertTrue(loaded.hasNonDefaultContent)

        // Legacy accountID / bulkPerAccount
        let legacyIDs = Data(#"{"format":"pdf","accountID":7,"bulkPerAccount":true}"#.utf8)
        let migratedIDs = try JSONDecoder().decode(ExportOptions.self, from: legacyIDs)
        XCTAssertEqual(migratedIDs.selectedAccountIDs, [7])
        XCTAssertEqual(migratedIDs.accountOutput, .perAccount)

        // Legacy "xml" → camt053v08
        let legacy = Data(#"{"format":"xml","scope":"filtered"}"#.utf8)
        let migrated = try JSONDecoder().decode(ExportOptions.self, from: legacy)
        XCTAssertEqual(migrated.format, .camt053v08)
        XCTAssertEqual(migrated.periodPreset, .lastMonth)
    }

    func testExportSourceRespectsAccountAndPeriod() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        let account = try! XCTUnwrap(model.allAccounts.first { !$0.iban.isEmpty })
        let all = try! XCTUnwrap(model.exportSource(accountIDs: nil, period: .current, bookingIDs: nil))
        let one = try! XCTUnwrap(model.exportSource(accountIDs: [account.id], period: .lastMonth, bookingIDs: nil))
        XCTAssertTrue(one.rows.allSatisfy { $0.accountId == account.id })
        XCTAssertEqual(one.periodLabel, Period.lastMonth.title)
        XCTAssertLessThanOrEqual(one.rows.count, all.rows.count)
    }

    func testOpenBookingExportPrefersSelection() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        let ids = Array(model.exportBookings().prefix(2).map(\.id))
        XCTAssertEqual(ids.count, 2)
        model.selectedBookingIDs = Set(ids)
        model.selectedBookingID = ids[0]
        model.openBookingExport(selection: true)
        XCTAssertTrue(model.showExportSheet)
        XCTAssertTrue(model.exportPreferSelection)
        XCTAssertEqual(model.exportBookingIDs, Set(ids))
    }

    func testPDFA4PageMediaBox() async throws {
        var report = report()
        report.generatedAt = Date()
        var o = ExportOptions()
        o.format = .pdf
        o.showDailyBalance = false
        o.showCurrentBalance = false
        let html = PrintReportBuilder.html(report, options: o.asPrintOptions, paper: .a4)
        let data = try await HTMLPDFRenderer.shared.pdf(html: html, paper: .a4)
        let doc = try XCTUnwrap(PDFDocument(data: data))
        XCTAssertGreaterThanOrEqual(doc.pageCount, 1)
        let page = try XCTUnwrap(doc.page(at: 0))
        let box = page.bounds(for: .mediaBox)
        XCTAssertEqual(box.width, 210 * 72 / 25.4, accuracy: 4)
        XCTAssertEqual(box.height, 297 * 72 / 25.4, accuracy: 4)
        XCTAssertGreaterThan(box.height, box.width)
        let text = page.string ?? ""
        XCTAssertTrue(text.contains("Giro") || text.contains("Übersicht") || text.contains("Arbeitgeber"), text)
        // Upright: title before payee. Upside-down exports reverse that order.
        if let titleAt = (text.range(of: "Giro") ?? text.range(of: "Übersicht"))?.lowerBound,
           let rowAt = text.range(of: "Arbeitgeber")?.lowerBound {
            XCTAssertLessThan(titleAt, rowAt, "PDF appears upside-down: \(text.prefix(400))")
        }
    }




}
