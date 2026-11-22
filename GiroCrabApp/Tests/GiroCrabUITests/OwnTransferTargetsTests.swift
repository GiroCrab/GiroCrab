import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The recipient entry the transfer and the standing order form share (#1475): the own accounts an
/// Umbuchung may go to, the recipient list state and the standing draft's Umbuchung.
@MainActor
final class OwnTransferTargetsTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    override func tearDown() async throws { model = nil }

    func testTargetsExcludeSenderArchivedAndAccountsWithoutIBAN() throws {
        let source = try XCTUnwrap(model.transferAccounts.first)
        let targets = model.ownTransferTargets(from: source)
        XCTAssertFalse(targets.isEmpty)
        XCTAssertFalse(targets.contains { $0.id == source.id })
        XCTAssertTrue(targets.allSatisfy { !$0.iban.isEmpty && !$0.isArchived })
    }

    func testSavingsAccountBooksOnlyToOwnCheckingAccounts() throws {
        let source = try XCTUnwrap(model.allAccounts.first { $0.kind == .savings && !$0.iban.isEmpty })
        let targets = model.ownTransferTargets(from: source)
        XCTAssertFalse(targets.isEmpty)
        XCTAssertTrue(targets.allSatisfy { $0.kind == .checking })
        XCTAssertEqual(targets.map(\.id), model.internalTransferTargets(from: source).map(\.id))
    }

    func testStandingOrderAccountsNeverIncludeSavings() {
        XCTAssertTrue(model.standingOrderAccounts.allSatisfy { $0.kind != .savings })
    }

    func testOwnAccountIsFoundByIBANWithoutSpacesOrCase() throws {
        let source = try XCTUnwrap(model.transferAccounts.first)
        let targets = model.ownTransferTargets(from: source)
        let target = try XCTUnwrap(targets.first)
        let typed = formatIBAN(target.iban).lowercased()
        XCTAssertEqual(AppModel.ownAccount(withIBAN: typed, in: targets)?.id, target.id)
        XCTAssertNil(AppModel.ownAccount(withIBAN: "", in: targets))
        XCTAssertNil(AppModel.ownAccount(withIBAN: "DE00 0000 0000 0000 0000 00", in: targets))
        XCTAssertNil(AppModel.ownAccount(withIBAN: source.iban, in: targets), "the sender is no target")
    }

    func testRecipientNameIsOwnerElseAccountName() throws {
        var a = try XCTUnwrap(model.allAccounts.first)
        a.owner = "Erika Muster"
        XCTAssertEqual(AppModel.ownRecipientName(a), "Erika Muster")
        a.owner = ""
        XCTAssertEqual(AppModel.ownRecipientName(a), a.name)
    }

    func testStandingDraftStartsAsTransferToSomeoneElse() {
        XCTAssertFalse(StandingDraft().ownTransfer)
    }

    func testSuggestionsStayEmptyWithoutText() {
        let s = RecipientSuggestions()
        s.focused = true
        s.name = "  "
        s.refresh(model: model)
        XCTAssertTrue(s.items.isEmpty)
    }

    func testSuggestionsOnlyWhileFocusedAndNotJustPicked() throws {
        let s = RecipientSuggestions()
        s.name = "a"
        s.focused = false
        s.refresh(model: model)
        XCTAssertTrue(s.items.isEmpty, "closed field: no list")

        s.focused = true
        s.refresh(model: model)
        s.edited = true
        s.refresh(model: model)
        guard let first = s.items.first else { return }  // the demo vault knows no recipient for "a"
        s.take(first)
        XCTAssertTrue(s.items.isEmpty)
        s.name = first.name
        s.refresh(model: model)
        XCTAssertTrue(s.items.isEmpty, "a name just taken from the list does not reopen it")
    }

    func testListStaysClosedUntilTheNameIsEdited() {
        // "Dauerauftrag ändern" opens with a name in the focused field: no list until the person types
        let s = RecipientSuggestions()
        s.name = "Hausverwaltung Nord"
        s.focused = true
        s.refresh(model: model)
        XCTAssertTrue(s.items.isEmpty)
        XCTAssertFalse(s.edited)
    }

    func testArrowKeysStayInsideTheList() {
        let s = RecipientSuggestions()
        XCTAssertEqual(s.move(1), .ignored, "no list: the key goes on")
    }

    // MARK: paste and templates in the standing order form

    func testPasteFillsTheStandingDraftAndLeavesUmbuchung() {
        model.standingDraft.ownTransfer = true
        let p = ParsedPayment(name: "Stadtwerke", iban: "de89370400440532013000", bic: "COBADEFFXXX",
                              amountText: "85,00", purpose: "Abschlag")
        model.fillStandingDraft(p)
        let d = model.standingDraft
        XCTAssertFalse(d.ownTransfer)
        XCTAssertEqual(d.name, "Stadtwerke")
        XCTAssertEqual(d.iban, "DE89 3704 0044 0532 0130 00")
        XCTAssertEqual(d.bic, "COBADEFFXXX")
        XCTAssertEqual(d.amountText, "85,00")
        XCTAssertEqual(d.purpose, "Abschlag")
        XCTAssertTrue(d.ibanValid)
    }

    func testPasteKeepsWhatWasTypedWhenOnlyEmptyFieldsAreFilled() {
        model.standingDraft.name = "Vermieter"
        model.fillStandingDraft(ParsedPayment(name: "Anderer", iban: "DE89370400440532013000"), onlyEmpty: true)
        XCTAssertEqual(model.standingDraft.name, "Vermieter")
        XCTAssertFalse(model.standingDraft.iban.isEmpty)
    }

    func testTemplateFillsTheStandingDraftNotTheTransfer() {
        let t = TransferTemplate(id: 1, name: "Miete", recipientName: "Vermieter GmbH",
                                 recipientIban: "DE89370400440532013000", recipientBic: "",
                                 defaultAmountCents: 85_000, defaultPurpose: "Miete")
        let before = model.transferDraft.name
        model.applyTransferTemplate(t, standing: true)
        XCTAssertEqual(model.standingDraft.name, "Vermieter GmbH")
        XCTAssertEqual(model.standingDraft.cents, 85_000)
        XCTAssertEqual(model.transferDraft.name, before)
    }

    // MARK: IBAN field and typed numbers (#1475)

    /// A late invoice result must not fill a sheet that was closed or reopened meanwhile.
    func testLateInvoiceResultIsDroppedAfterTheSheetChanged() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.showStandingSheet = true
        let token = model.standingImportToken
        var p = ParsedPayment()
        p.name = "Alt"
        model.cancelStandingSheet()
        XCTAssertFalse(model.fillStandingDraft(p, ifToken: token))
        XCTAssertNotEqual(model.standingDraft.name, "Alt")
    }
}
