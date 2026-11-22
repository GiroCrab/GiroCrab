import XCTest
import SwiftUI
import GiroCrabCore
@testable import GiroCrabUI

@MainActor final class AssetClassificationTests: XCTestCase {
    func testDonutSelectionUsesPositiveSectorRanges() {
        let slices = [
            WealthSlice(id: "equity", label: "Aktien", cents: 7000, color: .blue),
            WealthSlice(id: "unknown", label: "Differenz", cents: -200, color: .gray),
            WealthSlice(id: "empty", label: "Leer", cents: 0, color: .gray),
            WealthSlice(id: "crypto", label: "Krypto", cents: 3000, color: .red),
        ]
        XCTAssertEqual(WealthSlice.id(at: 0, in: slices), "equity")
        XCTAssertEqual(WealthSlice.id(at: 6999.9, in: slices), "equity")
        XCTAssertEqual(WealthSlice.id(at: 7000, in: slices), "crypto")
        XCTAssertEqual(WealthSlice.id(at: 9999.9, in: slices), "crypto")
        for invalid in [-1.0, 10000, .nan, .infinity] {
            XCTAssertNil(WealthSlice.id(at: invalid, in: slices))
        }
    }
    func testClassViewPreservesGrossTotalWithMissingDepotPositionsAndCardCredit() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let summary = WealthSummary(accounts: model.allAccounts, tangibleIds: Set(model.allAccounts.filter { model.isTangible($0.id) }.map(\.id)))
        // No positions loaded yet: account values must appear as unclassified rest, not disappear.
        let rows = WealthClassRow.make(summary, assets: [:])
        XCTAssertEqual(rows.reduce(0) { $0 + $1.cents }, summary.grossCents)
        XCTAssertEqual(rows.filter { $0.classification.assetClass == .cash }.reduce(0) { $0 + $1.cents }, summary.cashCents + summary.creditCents)
        XCTAssertEqual(rows.filter { $0.classification.assetClass == .unclassified }.reduce(0) { $0 + $1.cents }, summary.assetsCents)
        XCTAssertTrue(rows.allSatisfy { $0.classification.securityKey == nil })
    }
    func testSummaryScopeShowsOnlyTheSelectedAccountBucket() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let full = WealthSummary(accounts: model.allAccounts, tangibleIds: Set(model.allAccounts.filter { model.isTangible($0.id) }.map(\.id)))
        XCTAssertEqual(full.scoped(to: .all), full)
        for (scope, amount) in [(WealthScope.cash, full.cashCents), (.securities, full.cents(.securities)), (.tangibles, full.cents(.tangibles))] {
            let selected = full.scoped(to: scope)
            XCTAssertEqual(selected.grossCents, amount)
            XCTAssertEqual(selected.netCents, amount)
            XCTAssertEqual(selected.liabilitiesCents, 0)
        }
    }
    func testPositionsAndRemainderAreNotDoubleCounted() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        var depot = try XCTUnwrap(model.allAccounts.first { $0.kind == .brokerage && !model.isTangible($0.id) })
        depot.balanceCents = 10000; depot.currency = "EUR"; depot.inTotal = true; depot.archivedUnix = nil
        let holding = Holding(isin: "US0378331005", wkn: "", name: "Apple", quantity: 1, currentPriceCents: 2000, purchasePriceCents: 1000, valueCents: 2000)
        let classification = classifyAsset(holding: holding, imported: ClassificationValues(assetClass: nil, instrument: nil), user: ClassificationValues(assetClass: nil, instrument: nil), confirmedDirectCrypto: false)
        let rows = WealthClassRow.make(WealthSummary(accounts: [depot]), assets: [depot.id: [ClassifiedAsset(holding: holding, classification: classification, assetItemId: nil)]])
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows.reduce(0) { $0 + $1.cents }, 10000)
        XCTAssertEqual(rows.first { $0.classification.assetClass == .unclassified }?.cents, 8000)
        XCTAssertEqual(rows.first { $0.classification.assetClass == .equity }?.cents, 2000)
        XCTAssertEqual(rows.first { $0.classification.assetClass == .equity }?.identifier, "US0378331005")
        XCTAssertEqual(rows.first { $0.classification.assetClass == .equity }?.quantity, 1)
    }
    func testPropertyExclusionPreservesFundsRemainderAndLiabilities() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        var account = try XCTUnwrap(model.allAccounts.first { $0.kind == .brokerage })
        account.balanceCents = 52000000; account.currency = "EUR"; account.inTotal = true; account.archivedUnix = nil
        var loan = try XCTUnwrap(model.allAccounts.first { $0.kind == .loan })
        loan.balanceCents = -10000000; loan.currency = "EUR"; loan.inTotal = true; loan.archivedUnix = nil
        func item(_ name: String, _ cents: Int64, _ instrument: InstrumentKind) -> ClassifiedAsset {
            let holding = Holding(isin: "", wkn: "", name: name, quantity: 1, currentPriceCents: cents, purchasePriceCents: 0, valueCents: cents)
            let classification = classifyAsset(holding: holding, imported: ClassificationValues(assetClass: .realEstate, instrument: instrument), user: ClassificationValues(assetClass: nil, instrument: nil), confirmedDirectCrypto: false)
            return ClassifiedAsset(holding: holding, classification: classification, assetItemId: nil)
        }
        let assets = [account.id: [item("Haus", 50000000, .directHolding), item("Immobilienfonds", 1000000, .fund)]]
        let full = WealthSummary(accounts: [account, loan], tangibleIds: [account.id])
        let selection = WealthAssetSelection(summary: full, assets: assets, hideDirectProperty: true)
        XCTAssertEqual(selection.directPropertyCount, 1)
        XCTAssertEqual(selection.summary.grossCents, 2000000)
        XCTAssertEqual(selection.summary.liabilitiesCents, -10000000)
        XCTAssertEqual(selection.summary.netCents, -8000000)
        XCTAssertEqual(selection.assets[account.id]?.map(\.holding.name), ["Immobilienfonds"])
        let rows = WealthClassRow.make(selection.summary, assets: selection.assets)
        XCTAssertEqual(rows.reduce(0) { $0 + $1.cents }, selection.summary.grossCents)
        XCTAssertEqual(rows.first { $0.classification.assetClass == .unclassified }?.cents, 1000000)
        XCTAssertEqual(WealthAssetSelection(summary: full, assets: assets, hideDirectProperty: false).summary, full)
    }
    func testNegativeRemainderKeepsValuationDifferenceVisible() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        var depot = try XCTUnwrap(model.allAccounts.first { $0.kind == .brokerage })
        depot.balanceCents = -100; depot.currency = "EUR"; depot.inTotal = true; depot.archivedUnix = nil
        let rows = WealthClassRow.make(WealthSummary(accounts: [depot]), assets: [:])
        XCTAssertEqual(rows.reduce(0) { $0 + $1.cents }, -100)
        XCTAssertEqual(rows.first?.classification.assetClass, .unclassified)
    }
    func testIndependentUserClassificationAndAutomaticFallback() {
        let h = Holding(isin: "IE00B4L5Y983", wkn: "", name: "World ETF", quantity: 1, currentPriceCents: 100, purchasePriceCents: 50, valueCents: 100)
        let c = classifyAsset(holding: h, imported: ClassificationValues(assetClass: nil, instrument: nil), user: ClassificationValues(assetClass: .other, instrument: nil), confirmedDirectCrypto: false)
        XCTAssertEqual(c.assetClass, .other); XCTAssertEqual(c.assetClassSource, .user)
        XCTAssertEqual(c.instrument, .etf); XCTAssertEqual(c.instrumentSource, .rule)
        XCTAssertEqual(c.securityKey, "isin:IE00B4L5Y983")
    }
    private func classification(key: String?, _ asset: AssetClass, _ instrument: InstrumentKind) -> AssetClassification {
        AssetClassification(securityKey: key, assetClass: asset, assetClassSource: .rule, instrument: instrument, instrumentSource: .rule)
    }
    func testSachwertRowsOmitSelfEvidentDetails() {
        let house = classification(key: "asset:1", .realEstate, .directHolding)
        XCTAssertEqual(house.compactTitle, AssetClass.realEstate.title)
        XCTAssertEqual(WealthClassRow(id: "a", name: "Haus", accountName: "Sachwerte", cents: 1, scope: .tangibles, classification: house, quantity: 1).subtitle, "Sachwerte")
        XCTAssertTrue(WealthClassRow(id: "b", name: "Münze", accountName: "Sachwerte", cents: 1, scope: .tangibles, classification: house, quantity: 2).subtitle.hasSuffix("· Sachwerte"))
        // a directly held coin in a depot keeps the instrument: there it is information
        let coin = classification(key: "crypto:BTC", .crypto, .directHolding)
        XCTAssertEqual(coin.compactTitle, AssetClass.crypto.title + " · " + InstrumentKind.directHolding.title)
    }
    func testAssignQueueSkipsUnstorablePositionsAndRepeatedSecurities() {
        let etf = classification(key: "isin:IE00B4L5Y983", .unclassified, .etf)
        let request = ClassificationEditRequest(entries: [
            .init(name: "ETF Depot A", classification: etf),
            .init(name: "Rest", classification: classification(key: nil, .unclassified, .unclassified)),
            .init(name: "ETF Depot B", classification: etf),
            .init(name: "Auto", classification: classification(key: "asset:9", .unclassified, .directHolding)),
        ])
        XCTAssertEqual(request.entries.map(\.name), ["ETF Depot A", "Auto"])
    }
    func testUnclassifiedIsNotAnAssignableChoice() {
        XCTAssertFalse(AssetClass.assignable.contains(.unclassified))
        XCTAssertFalse(InstrumentKind.assignable.contains(.unclassified))
        XCTAssertEqual(AssetClass.assignable.count, AssetClass.choices.count - 1)
    }
    func testPropertyCountIsKeptPerBucketWhetherHiddenOrNot() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        var account = try XCTUnwrap(model.allAccounts.first { $0.kind == .brokerage })
        account.balanceCents = 100; account.currency = "EUR"; account.inTotal = true; account.archivedUnix = nil
        let holding = Holding(isin: "", wkn: "", name: "Haus", quantity: 1, currentPriceCents: 100, purchasePriceCents: 0, valueCents: 100)
        let assets = [account.id: [ClassifiedAsset(holding: holding, classification: classification(key: "asset:1", .realEstate, .directHolding), assetItemId: 1)]]
        let full = WealthSummary(accounts: [account], tangibleIds: [account.id])
        for hidden in [false, true] {
            let selection = WealthAssetSelection(summary: full, assets: assets, hideDirectProperty: hidden)
            XCTAssertEqual(selection.directProperty[.tangibles], 1)
            XCTAssertNil(selection.directProperty[.securities])
        }
    }
    func testOwnerFilterIsOfferedOnlyWithSeveralOwners() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        var accounts = Array(model.allAccounts.prefix(4))
        XCTAssertEqual(accounts.count, 4)
        // the demo: two owners, and the Sachwerte account without one
        XCTAssertEqual(WealthOwnerFilter.options(for: model.allAccounts),
                       [.all, .owner("Anton Mustermann"), .owner("Erika Mustermann"), .unassigned])
        // one owner and accounts without one: nobody to tell apart
        XCTAssertTrue(WealthOwnerFilter.options(for: model.allAccounts.filter { $0.owner != "Anton Mustermann" }).isEmpty)
        accounts[0].owner = "Erika Muster"; accounts[1].owner = " Erika Muster "
        accounts[2].owner = "anton Muster"; accounts[3].owner = ""
        XCTAssertEqual(WealthOwnerFilter.options(for: accounts), [.all, .owner("anton Muster"), .owner("Erika Muster"), .unassigned])
        XCTAssertEqual(accounts.filter(WealthOwnerFilter.owner("Erika Muster").keeps).count, 2)
        XCTAssertEqual(accounts.filter(WealthOwnerFilter.unassigned.keeps).map(\.id), [accounts[3].id])
        XCTAssertEqual(accounts.filter(WealthOwnerFilter.all.keeps).count, 4)
        // without accounts lacking an owner there is no such chip
        XCTAssertEqual(WealthOwnerFilter.options(for: Array(accounts.prefix(3))), [.all, .owner("anton Muster"), .owner("Erika Muster")])
    }
    func testInstrumentGroupingSeparatesEtfsFromSingleStocks() {
        func row(_ name: String, _ instrument: InstrumentKind) -> WealthClassRow {
            WealthClassRow(id: name, name: name, accountName: "Depot", cents: 100, scope: .securities,
                           classification: AssetClassification(securityKey: "isin:\(name)", assetClass: .equity, assetClassSource: .rule, instrument: instrument, instrumentSource: .rule))
        }
        let rows = [row("ETF", .etf), row("Aktie", .singleSecurity), row("Offen", .unclassified)]
        XCTAssertEqual(Set(rows.map { $0.group(by: .assetClass).key }), ["equity"])
        XCTAssertEqual(rows.map { $0.group(by: .instrument).key }, ["etf", "single", "unclassified"])
        XCTAssertEqual(rows.map { $0.group(by: .instrument).unclassified }, [false, false, true])
        // under an instrument heading the row names the class instead of repeating the instrument
        XCTAssertTrue(rows[0].subtitle(by: .instrument).hasSuffix(AssetClass.equity.title))
        XCTAssertTrue(rows[0].subtitle(by: .assetClass).hasSuffix("ETF"))
        // the instrument view is a securities matter; elsewhere the stored choice falls back
        XCTAssertEqual(WealthGrouping.instrument.effective(in: .securities), .instrument)
        XCTAssertEqual(WealthGrouping.instrument.effective(in: .all), .assetClass)
        XCTAssertEqual(WealthGrouping.account.effective(in: .tangibles), .account)
    }
    func testAccountKindGroupingIsOfferedForCashOnly() {
        XCTAssertEqual(WealthGrouping.available(in: .cash), [.account, .accountKind])
        XCTAssertEqual(WealthGrouping.accountKind.effective(in: .cash), .accountKind)
        // elsewhere it falls back to the account view, never to the class view
        for scope in [WealthScope.all, .securities, .tangibles] {
            XCTAssertFalse(WealthGrouping.available(in: scope).contains(.accountKind))
            XCTAssertEqual(WealthGrouping.accountKind.effective(in: scope), .account)
        }
        // cash offers no class view: every cash account has the class "Liquidität & Geldmarkt"
        XCTAssertEqual(WealthGrouping.assetClass.effective(in: .cash), .account)
        XCTAssertEqual(WealthGrouping.instrument.effective(in: .cash), .account)
    }
    func testLiquidityGradeFollowsKindClassAndDirectHolding() {
        XCTAssertEqual([AccountKind.checking, .savings, .paymentService, .creditCard, .cash].map(LiquidityGrade.of), Array(repeating: .immediate, count: 5))
        XCTAssertEqual(LiquidityGrade.of(.timeDeposit), .shortTerm)
        XCTAssertEqual(LiquidityGrade.of(.buildingSavings), .longTerm)
        // a depot's positions decide, a loan is a debt
        XCTAssertNil(LiquidityGrade.of(.brokerage))
        XCTAssertNil(LiquidityGrade.of(.loan))
        func c(_ asset: AssetClass, _ instrument: InstrumentKind = .etf, key: String = "isin:X") -> AssetClassification {
            AssetClassification(securityKey: key, assetClass: asset, assetClassSource: .rule, instrument: instrument, instrumentSource: .rule)
        }
        XCTAssertEqual(LiquidityGrade.of(c(.cash), tangible: false), .immediate)
        XCTAssertEqual(LiquidityGrade.of(c(.equity), tangible: false), .shortTerm)
        XCTAssertEqual(LiquidityGrade.of(c(.unclassified, .unclassified, key: ""), tangible: false), .shortTerm)
        XCTAssertEqual(LiquidityGrade.of(c(.realEstate, .directHolding), tangible: false), .illiquid)
        XCTAssertEqual(LiquidityGrade.of(c(.crypto, .directHolding), tangible: false), .shortTerm)
        XCTAssertEqual(LiquidityGrade.of(c(.commodity, .directHolding, key: "asset:1"), tangible: false), .illiquid)
        // the remainder of a Sachwerte account has no position, its tab decides
        XCTAssertEqual(LiquidityGrade.of(c(.unclassified, .unclassified, key: ""), tangible: true), .illiquid)
    }
    func testLiquidityGroupingSplitsOneDepotAndKeepsTheGrossTotal() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        func account(_ kind: AccountKind, _ cents: Int64) throws -> Account {
            var a = try XCTUnwrap(model.allAccounts.first { $0.kind == kind && !model.isTangible($0.id) } ?? model.allAccounts.first)
            a.kind = kind; a.balanceCents = cents; a.currency = "EUR"; a.inTotal = true; a.archivedUnix = nil
            return a
        }
        var checking = try account(.checking, 1000), fixed = try account(.timeDeposit, 2000), bauspar = try account(.buildingSavings, 4000)
        var card = try account(.creditCard, 50), depot = try account(.brokerage, 30000)
        checking.id = 1; fixed.id = 2; bauspar.id = 3; card.id = 4; depot.id = 5
        func holding(_ name: String, _ cents: Int64, _ asset: AssetClass) -> ClassifiedAsset {
            ClassifiedAsset(holding: Holding(isin: "DE\(name)", wkn: "", name: name, quantity: 1, currentPriceCents: cents, purchasePriceCents: cents, valueCents: cents),
                            classification: AssetClassification(securityKey: "isin:DE\(name)", assetClass: asset, assetClassSource: .rule, instrument: .etf, instrumentSource: .rule),
                            assetItemId: nil)
        }
        let summary = WealthSummary(accounts: [checking, fixed, bauspar, card, depot])
        let rows = WealthClassRow.make(summary, assets: [depot.id: [holding("Geldmarkt", 10000, .cash), holding("Welt", 20000, .equity)]])
        func total(_ grade: LiquidityGrade) -> Int64 { rows.filter { $0.group(by: .liquidity) == grade.group }.reduce(0) { $0 + $1.cents } }
        // the depot lands on two grades; the positive card balance is at hand
        XCTAssertEqual(total(.immediate), 1000 + 50 + 10000)
        XCTAssertEqual(total(.shortTerm), 2000 + 20000)
        XCTAssertEqual(total(.longTerm), 4000)
        XCTAssertEqual(total(.illiquid), 0)
        XCTAssertEqual(LiquidityGrade.allCases.map(total).reduce(0, +), summary.grossCents)
        // the row says why it is there
        XCTAssertTrue(try XCTUnwrap(rows.first { $0.kind == .timeDeposit }).subtitle(by: .liquidity).hasSuffix(AccountKind.timeDeposit.label))
        XCTAssertTrue(try XCTUnwrap(rows.first { $0.name == "Geldmarkt" }).subtitle(by: .liquidity).hasSuffix(AssetClass.cash.title))
    }
    func testLiquidityGroupingIsOfferedInAllOnly() {
        XCTAssertEqual(WealthGrouping.available(in: .all), [.account, .assetClass, .liquidity])
        for scope in [WealthScope.cash, .securities, .tangibles] {
            XCTAssertFalse(WealthGrouping.available(in: scope).contains(.liquidity))
            XCTAssertEqual(WealthGrouping.liquidity.effective(in: scope), .account)
        }
    }
    func testSliceFilterLimitsListsAndTreatsSonstigeAsEverythingElse() {
        let slices = [WealthSlice(id: "a", label: "A", cents: 3, color: .blue), WealthSlice(id: "b", label: "B", cents: 2, color: .red),
                      WealthSlice(id: "rest", label: "Sonstige", cents: 1, color: .gray)]
        let none = WealthView.SliceFilter(selected: "all", slices: slices)
        XCTAssertFalse(none.isActive)
        XCTAssertTrue(none.keeps("anything"))
        let one = WealthView.SliceFilter(selected: "b", slices: slices)
        XCTAssertTrue(one.isActive)
        XCTAssertEqual(["a", "b", "c"].filter(one.keeps), ["b"])
        // the folded remainder: every row that has no slice of its own
        let rest = WealthView.SliceFilter(selected: "rest", slices: slices)
        XCTAssertEqual(["a", "b", "c", "d"].filter(rest.keeps), ["c", "d"])
        // a slice that no longer exists filters nothing
        XCTAssertFalse(WealthView.SliceFilter(selected: "gone", slices: slices).isActive)
    }
}
