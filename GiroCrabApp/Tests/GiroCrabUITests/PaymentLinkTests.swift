import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// `girocrab://transfer?…` links (docs/payment-link.md).
@MainActor
final class PaymentLinkTests: XCTestCase {
    private func payment(_ link: String) -> Result<ParsedPayment, PaymentLink.Failure> {
        PaymentLink.payment(from: URL(string: link)!)
    }

    func testParsesAllFields() throws {
        let p = try payment("girocrab://transfer?name=Stadtwerke%20M%C3%BCnchen&iban=de89%203704%200044%200532%200130%2000&bic=cobadeffxxx&amount=85,5&purpose=Abschlag%0AStrom").get()
        XCTAssertEqual(p.name, "Stadtwerke München")
        XCTAssertEqual(p.iban, "DE89370400440532013000")
        XCTAssertEqual(p.bic, "COBADEFFXXX")
        XCTAssertEqual(p.amountText, Money.plain(8550))
        XCTAssertEqual(p.purpose, "Abschlag Strom", "single line")
    }

    func testDotAmountAndLimits() throws {
        let long = String(repeating: "x", count: 200)
        let p = try payment("girocrab://transfer?iban=DE89370400440532013000&amount=12.50&name=\(long)&purpose=\(long)").get()
        XCTAssertEqual(p.amountText, Money.plain(1250))
        XCTAssertEqual(p.name?.count, 70)
        XCTAssertEqual(p.purpose?.count, 140)
    }

    func testOptionalFieldsAndBadAmountsStayEmpty() throws {
        let p = try payment("girocrab://transfer?iban=DE89370400440532013000&amount=-5&name=%20&foo=bar").get()
        XCTAssertNil(p.amountText)
        XCTAssertNil(p.name)
        XCTAssertNil(p.purpose)
        XCTAssertNil(try payment("girocrab://transfer?iban=DE89370400440532013000&amount=abc").get().amountText)
    }

    func testRefusesWhatIsNoTransfer() {
        XCTAssertEqual(payment("girocrab://transfer?name=x"), .failure(.missingIBAN))
        XCTAssertEqual(payment("girocrab://transfer?iban=DE89370400440532013000&currency=usd"), .failure(.currency("USD")))
        XCTAssertEqual(payment("girocrab://other?iban=DE89370400440532013000"), .failure(.unsupported))
        XCTAssertEqual(payment("https://transfer?iban=DE89370400440532013000"), .failure(.unsupported))
        XCTAssertNoThrow(try payment("GIROCRAB://Transfer?IBAN=DE89370400440532013000&currency=EUR").get())
    }

    // MARK: model

    private func demoModel() -> AppModel {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        return model
    }

    private let link = URL(string: "girocrab://transfer?name=Stadtwerke&iban=DE89370400440532013000&amount=85&purpose=Abschlag")!

    func testLinkOpensAFilledFormMarkedAsFromLink() throws {
        let model = demoModel()
        try XCTSkipUnless(model.canTransfer, "demo vault without transfer accounts")
        model.openLink(link)
        XCTAssertTrue(model.showTransferSheet)
        XCTAssertTrue(model.transferDraft.fromLink)
        XCTAssertFalse(model.transferDraft.ownTransfer)
        XCTAssertEqual(model.transferDraft.name, "Stadtwerke")
        XCTAssertEqual(model.transferDraft.iban, TransferSheetView.groupedIBAN("DE89370400440532013000"))
        XCTAssertEqual(model.transferDraft.amountText, Money.plain(8500))
        XCTAssertNotEqual(model.allAccounts.first { $0.id == model.transferDraft.accountID }?.kind, .savings)
        XCTAssertNil(model.pendingPaymentLink)
        // A new transfer starts without the warning.
        model.showTransferSheet = false
        model.openTransfer()
        XCTAssertFalse(model.transferDraft.fromLink)
    }

    func testLinkWaitsForARunningSync() async throws {
        let model = demoModel()
        try XCTSkipUnless(model.canTransfer, "demo vault without transfer accounts")
        model.syncingConnectionID = 1
        model.openLink(link)
        XCTAssertFalse(model.showTransferSheet)
        XCTAssertNotNil(model.pendingPaymentLink)
        model.syncingConnectionID = nil
        for _ in 0..<20 where !model.showTransferSheet { await Task.yield() }
        XCTAssertTrue(model.showTransferSheet)
        XCTAssertTrue(model.transferDraft.fromLink)
    }

    /// Also an open form on its result step: the link waits until the form is closed.
    func testLinkWaitsForAnOpenFormAndNeverReplacesIt() async throws {
        let model = demoModel()
        try XCTSkipUnless(model.canTransfer, "demo vault without transfer accounts")
        model.openTransfer()
        model.transferDraft.name = "Eigene Eingabe"
        model.openLink(link)
        XCTAssertEqual(model.transferDraft.name, "Eigene Eingabe")
        XCTAssertFalse(model.transferDraft.fromLink)
        XCTAssertNotNil(model.pendingPaymentLink)
        XCTAssertNil(model.errorMessage)
        model.showTransferSheet = false
        for _ in 0..<20 where !model.showTransferSheet { await Task.yield() }
        XCTAssertTrue(model.showTransferSheet)
        XCTAssertEqual(model.transferDraft.name, "Stadtwerke")
        XCTAssertTrue(model.transferDraft.fromLink)
    }

    func testLinkWaitsUntilTheAccountsAreLoaded() async throws {
        let model = demoModel()
        try XCTSkipUnless(model.canTransfer, "demo vault without transfer accounts")
        model.accountsLoaded = false
        model.openLink(link)
        XCTAssertFalse(model.showTransferSheet)
        XCTAssertNil(model.errorMessage)
        model.applyAccountGroups(model.groups, model.userGroups)
        for _ in 0..<20 where !model.showTransferSheet { await Task.yield() }
        XCTAssertTrue(model.showTransferSheet)
    }

    func testNewerLinkReplacesAWaitingOne() async throws {
        let model = demoModel()
        try XCTSkipUnless(model.canTransfer, "demo vault without transfer accounts")
        model.syncingConnectionID = 1
        model.openLink(link)
        model.openLink(URL(string: "girocrab://transfer?name=Neu&iban=DE89370400440532013000")!)
        model.syncingConnectionID = nil
        for _ in 0..<20 where !model.showTransferSheet { await Task.yield() }
        XCTAssertEqual(model.transferDraft.name, "Neu")
    }

    func testInvalidLinkShowsAnError() {
        let model = demoModel()
        model.openLink(URL(string: "girocrab://transfer?name=x")!)
        XCTAssertFalse(model.showTransferSheet)
        XCTAssertNotNil(model.errorMessage)
    }
}
