import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// #337 group level: a pure order selection (user group, kind section, bank of
/// only order accounts) is an evidence context — the sidebar sum shows the
/// evidence value, the header shows „Nach Gutschriften" and the order tiles
/// instead of money KPIs — while the overall money totals keep these accounts
/// out. Mixed selections stay money contexts; currencies are never added up.
@MainActor
final class OrderGroupTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    private var amazon: Account {
        get throws {
            try XCTUnwrap(model.allAccounts.first { $0.kind == .order }, "demo has the Amazon order account")
        }
    }

    private var orderGroupUUID: String {
        get throws {
            try XCTUnwrap(model.userGroups.first { $0.accounts.contains { $0.kind == .order } }?.uuid,
                          "demo has the order group")
        }
    }

    /// The sidebar group sum shows the evidence value (not 0,00 €), although
    /// the account stays out of Verfügbar and every overall money figure.
    func testSidebarGroupSumShowsTheEvidenceValue() throws {
        let amazon = try amazon
        XCTAssertFalse(amazon.inTotal, "demo order account starts excluded")

        let uuid = try orderGroupUUID
        let group = try XCTUnwrap(model.userGroups.first { $0.uuid == uuid })
        XCTAssertEqual(group.accounts.map(\.id), [amazon.id])
        XCTAssertEqual(group.balanceCents, amazon.balanceCents,
                       "group sum = evidence value of its order account")

        // no double counting: Verfügbar stays without the order account
        XCTAssertFalse(model.liquidAccounts.map(\.id).contains(amazon.id))
    }

    /// Selecting the order group loads order evidence („Nach Gutschriften",
    /// the three tiles) and no money KPIs.
    func testPureOrderGroupIsAnEvidenceContext() throws {
        let amazon = try amazon
        model.sidebarSelection = .userGroup(try orderGroupUUID)
        XCTAssertEqual(model.orderSelectionAccountIDs, [amazon.id])
        XCTAssertTrue(model.isOrderSelection)

        model.reloadSelectionKPIs()
        let summary = try XCTUnwrap(model.orderAccountSummary)
        XCTAssertEqual(summary.orderCount, 3, "three demo orders")
        XCTAssertEqual(summary.creditCount, 1, "one demo credit")
        XCTAssertEqual(summary.orderValueCents, 126_38)
        XCTAssertEqual(summary.creditValueCents, 10_00)
        XCTAssertEqual(model.orderSelectionCurrency, "EUR")
        XCTAssertNil(model.selectionFlow, "evidence tiles, no money KPIs")
        // the header value of an order selection: „Nach Gutschriften"
        XCTAssertEqual(summary.orderValueCents - summary.creditValueCents, 116_38)
    }

    /// The kind section of the order account sums and selects the same way.
    func testKindSectionOfOrderAccountsIsAnEvidenceContext() throws {
        let amazon = try amazon
        let section = try XCTUnwrap(model.groups.first { $0.accounts.contains { $0.id == amazon.id } })
        XCTAssertEqual(section.balanceCents, amazon.balanceCents,
                       "kind section sum = evidence value")

        model.sidebarSelection = .group(section.id)
        XCTAssertTrue(model.isOrderSelection)
        XCTAssertEqual(model.orderSelectionAccountIDs, [amazon.id])
    }

    /// A group that also holds a bank account stays a money context: money
    /// KPIs (filtered), no order tiles; the order bookings stay listed.
    func testMixedGroupStaysAMoneyContext() throws {
        let amazon = try amazon
        let giro = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        let uuid = try orderGroupUUID
        model.moveAccount(giro.id, to: uuid)
        model.reloadAccounts()

        model.sidebarSelection = .userGroup(uuid)
        XCTAssertNil(model.orderSelectionAccountIDs, "mixed selection is no evidence context")
        XCTAssertFalse(model.isOrderSelection)

        model.reloadSelectionKPIs()
        XCTAssertTrue(model.orderCurrencySummaries.isEmpty)
        XCTAssertNotNil(model.selectionFlow, "money KPIs for a mixed group")
    }

    /// Several currencies of one selection stay separate: the list arrives
    /// core-sorted (primary currency first), the figures show only the primary
    /// entry, the rest becomes a note and is never added up.
    func testExtraCurrenciesAreNeverSummed() throws {
        model.orderCurrencySummaries = [
            OrderCurrencySummary(currency: "EUR", orderCount: 2, creditCount: 1,
                                 orderValueCents: 50_00, creditValueCents: 10_00),
            OrderCurrencySummary(currency: "USD", orderCount: 1, creditCount: 0,
                                 orderValueCents: 20_00, creditValueCents: 0),
        ]
        XCTAssertEqual(model.orderSelectionCurrency, "EUR")
        XCTAssertEqual(model.orderAccountSummary?.orderValueCents, 50_00, "EUR figures only")
        XCTAssertEqual(model.orderAccountSummary?.orderCount, 2)
        XCTAssertEqual(model.orderSelectionExtraCurrencies.map(\.currency), ["USD"])
        XCTAssertEqual(model.orderSelectionExtraCurrencies.first?.cents, 20_00)

        model.orderCurrencySummaries = [
            OrderCurrencySummary(currency: "CHF", orderCount: 1, creditCount: 0,
                                 orderValueCents: 5_00, creditValueCents: 0),
        ]
        XCTAssertEqual(model.orderSelectionCurrency, "CHF", "single further currency becomes primary")
        XCTAssertTrue(model.orderSelectionExtraCurrencies.isEmpty)
    }

    /// Order accounts carry no permanent exclusion marker: being outside the
    /// money totals is their normal state, the context-menu switch shows it
    /// (user decision, 17.11.2026). Switched-off money accounts keep it.
    func testOrderAccountHasNoExclusionMarker() throws {
        let amazon = try amazon
        XCTAssertFalse(amazon.inTotal)
        XCTAssertFalse(amazon.showsTotalExclusionIndicator, "normal state, no eye.slash/dimming")

        // a switched-off money account keeps the marker
        let giro = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking && $0.inTotal })
        try model.vault?.setAccountIncludeInTotal(accountId: giro.id, includeInTotal: false)
        model.reloadAccounts()
        let off = try XCTUnwrap(model.allAccounts.first { $0.id == giro.id })
        XCTAssertFalse(off.inTotal)
        XCTAssertTrue(off.showsTotalExclusionIndicator)

        // an opted-in order account is plainly included: no marker either
        try model.vault?.setAccountIncludeInTotal(accountId: amazon.id, includeInTotal: true)
        model.reloadAccounts()
        let on = try XCTUnwrap(model.allAccounts.first { $0.id == amazon.id })
        XCTAssertTrue(on.inTotal)
        XCTAssertFalse(on.showsTotalExclusionIndicator)
    }

    /// The evidence value shows positive (a minus would read like debt), the
    /// signed balance stays everywhere money is meant: money accounts and an
    /// opted-in order account (#337, user decision 17.11.2026).
    func testOrderAccountDisplaysPositiveEvidenceValue() throws {
        let amazon = try amazon
        XCTAssertLessThan(amazon.balanceCents, 0, "the stored balance is negative (charges)")
        XCTAssertEqual(amazon.displayBalanceCents, abs(amazon.balanceCents),
                       "sidebar shows the evidence value positive")
        XCTAssertEqual(amazon.displayBalanceCents, 116_38)

        // a money account keeps its sign, positive or negative
        let giro = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        XCTAssertEqual(giro.displayBalanceCents, giro.balanceCents)

        // an order account counted into the totals is a money figure again
        try model.vault?.setAccountIncludeInTotal(accountId: amazon.id, includeInTotal: true)
        model.reloadAccounts()
        let on = try XCTUnwrap(model.allAccounts.first { $0.id == amazon.id })
        XCTAssertTrue(on.inTotal)
        XCTAssertEqual(on.displayBalanceCents, on.balanceCents, "opted in: signed balance")
    }
}
