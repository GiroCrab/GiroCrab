import XCTest
import GiroCrabCore
@testable import GiroCrabUI

@MainActor
final class OrderAccountTests: XCTestCase {
    private func seeded() throws -> (AppModel, Account) {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let vault = try XCTUnwrap(model.vault)
        let cid = try vault.addBankConnection(connector: .amazonOrders, displayName: "Bestellungen Test", loginName: "demo")
        let json = #"{"orders":[{"code":"028-0000337-0000001","date":"17. November 2026","items":[{"title":"Demo-Buch"}],"rows":[{"label":"Gesamtsumme","value":"45,98 €","bold":true},{"label":"Erstattung gesamt","value":"10,00 €"}]}]}"#
        _ = try vault.importAmazonOrders(connectionId: cid, ordersJson: json)
        model.reloadAll()
        return (model, try XCTUnwrap(model.allAccounts.first { $0.connectionId == cid }))
    }

    func testAmazonUsesOrderKindAndSeparateEvidenceFigures() throws {
        let (m, account) = try seeded()
        XCTAssertEqual(account.kind, .order)
        XCTAssertFalse(account.includeInTotal)
        XCTAssertTrue(AccountKind.importChoices.contains(.order))
        m.sidebarSelection = .account(account.id)
        let s = try XCTUnwrap(m.orderAccountSummary)
        XCTAssertEqual(s.orderCount, 1)
        XCTAssertEqual(s.creditCount, 1)
        XCTAssertEqual(s.orderValueCents, 4598)
        XCTAssertEqual(s.creditValueCents, 1000)
        XCTAssertEqual(s.orderValueCents - s.creditValueCents, 3598)
        m.listLimit = 1
        m.reloadBookings()
        XCTAssertEqual(m.bookings.count, 2, "whole-day pages keep both bookings of the same day")
        XCTAssertEqual(m.orderAccountSummary?.orderCount, 1)
        XCTAssertEqual(m.orderAccountSummary?.creditCount, 1)
        m.setAccountIncludeInTotal(account, true)
        XCTAssertEqual(m.orderAccountSummary?.orderValueCents, 4598)
        XCTAssertTrue(m.liquidAccounts.contains { $0.id == account.id })
    }

    func testOrderPeriodAndReturnToBankClearOrderFigures() throws {
        let (m, account) = try seeded()
        m.sidebarSelection = .account(account.id)
        m.customPeriodFrom = "2026-12-01"
        m.customPeriodTo = "2026-12-31"
        m.period = .custom
        XCTAssertEqual(m.orderAccountSummary?.orderCount, 0)
        XCTAssertEqual(m.orderAccountSummary?.creditCount, 0)
        XCTAssertEqual(m.orderAccountSummary?.orderValueCents, 0)
        m.sidebarSelection = .account(try XCTUnwrap(m.allAccounts.first { $0.kind == .checking }).id)
        XCTAssertNil(m.selectedOrderAccountID)
        XCTAssertNil(m.orderAccountSummary)
    }

    func testFiveDigitOrdersAndMultipleCreditsOnOneDayStayComplete() throws {
        let (m, account) = try seeded()
        let vault = try XCTUnwrap(m.vault)
        let cid = try XCTUnwrap(account.connectionId)
        let json = #"{"orders":[{"code":"028-0000337-0000002","date":"17. November 2026","items":[{"title":"Demo Großbestellung A"}],"rows":[{"label":"Gesamtsumme","value":"12.345,67 €","bold":true},{"label":"Erstattung gesamt","value":"1.234,56 €"}]},{"code":"028-0000337-0000003","date":"17. November 2026","items":[{"title":"Demo Großbestellung B"}],"rows":[{"label":"Gesamtsumme","value":"23.456,78 €","bold":true}]}]}"#
        _ = try vault.importAmazonOrders(connectionId: cid, ordersJson: json)
        _ = try vault.importAmazonOrders(connectionId: cid, ordersJson: json)
        m.reloadAll()
        m.sidebarSelection = .account(account.id)
        m.listLimit = 1
        m.reloadBookings()
        XCTAssertEqual(m.bookings.count, 5, "same-day orders and credits share one complete day page")
        let summary = try XCTUnwrap(m.orderAccountSummary)
        XCTAssertEqual(summary.orderCount, 3)
        XCTAssertEqual(summary.creditCount, 2)
        XCTAssertEqual(summary.orderValueCents, 3_584_843)
        XCTAssertEqual(summary.creditValueCents, 124_456)
        XCTAssertEqual(summary.orderValueCents - summary.creditValueCents, 3_460_387)
        var filter = m.currentFilter
        filter.wholeDays = false
        XCTAssertEqual(try vault.listTransactions(filter: filter).transactions.count, 1)
        XCTAssertEqual(m.orderAccountSummary?.orderCount, 3, "KPI counts are independent of the page size")
        m.searchText = "unfindbarer Demo-Suchbegriff"
        m.reloadBookings()
        XCTAssertEqual(m.orderAccountSummary?.orderValueCents, 3_584_843, "the period KPI is independent of list search")
    }

    func testChangingKindRefreshesTheCurrentAccountHeader() throws {
        let (m, _) = try seeded()
        let bank = try XCTUnwrap(m.allAccounts.first { $0.kind == .checking })
        m.sidebarSelection = .account(bank.id)
        XCTAssertNil(m.orderAccountSummary)
        m.setAccountKind(bank.id, to: .order)
        XCTAssertEqual(m.selectedOrderAccountID, bank.id)
        XCTAssertNotNil(m.orderAccountSummary)
    }

    func testInteractiveSelectionLoadsOrderFigures() async throws {
        let (m, account) = try seeded()
        m.interactiveSidebarSelection = .account(account.id)
        let task = try XCTUnwrap(m.selectionLoadTask)
        await task.value
        XCTAssertEqual(m.orderAccountSummary?.orderCount, 1)
        XCTAssertEqual(m.orderAccountSummary?.creditValueCents, 1000)
        m.selectionPlaceholderDelay = .zero
        m.customPeriodFrom = "2026-12-01"
        m.customPeriodTo = "2026-12-31"
        m.interactivePeriod = .custom
        XCTAssertNil(m.orderAccountSummary, "old order figures must not appear under the new period")
        await m.selectionLoadTask?.value
        XCTAssertEqual(m.orderAccountSummary?.orderCount, 0)
        m.interactiveSidebarSelection = .allAccounts
        await m.selectionLoadTask?.value
        XCTAssertNil(m.orderAccountSummary)
        XCTAssertNil(m.errorMessage)
    }
}
