import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// TAN-Abgleich-Hinweis (#949): Betrag und IBAN-Ende beim Freigeben einer Einzelüberweisung.
@MainActor
final class TanComparisonTests: XCTestCase {
    func testGroupsIBANAndTakesLastFourCharacters() throws {
        let c = try XCTUnwrap(TanComparison(iban: "de89370400440532013000", amountCents: 12_345))
        XCTAssertEqual(c.iban, "DE89 3704 0044 0532 0130 00")
        XCTAssertEqual(c.endCharacters, "3000")
        XCTAssertEqual(c.amount, Money.format(12_345))
    }

    func testPrefixLeavesOutLastFourCharactersAcrossGroupBoundary() throws {
        // 22 characters: the last group has only two, so the end spans two groups
        let c = try XCTUnwrap(TanComparison(iban: "DE89370400440532013000", amountCents: 100))
        XCTAssertEqual(c.ibanPrefix, "DE89 3704 0044 0532 01")
        XCTAssertEqual(c.ibanPrefix.replacingOccurrences(of: " ", with: "") + c.endCharacters, "DE89370400440532013000")
    }

    func testRejectsIBANThatIsTooShort() {
        XCTAssertNil(TanComparison(iban: "DE8", amountCents: 100))
        XCTAssertNil(TanComparison(iban: "", amountCents: 100))
    }

    func testModelOffersNoComparisonWithoutRunningTransfer() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.transferDraft.iban = "DE89370400440532013000"
        model.transferDraft.amountText = "12,34"
        XCTAssertNil(model.tanComparison)
    }

    // (a) login SCA before the order was sent, or a sync's challenge: no card
    func testNoComparisonBeforeOrderApproval() throws {
        let snap = try XCTUnwrap(TanComparison(iban: "DE89370400440532013000", amountCents: 1_234))
        XCTAssertNil(AppModel.visibleTanComparison(
            snapshot: snap, orderApproval: false, isBatch: false, isStanding: false, isDebit: false))
        XCTAssertEqual(AppModel.visibleTanComparison(
            snapshot: snap, orderApproval: true, isBatch: false, isStanding: false, isDebit: false), snap)
    }

    func testModelHidesSnapshotWithoutOrderChallenge() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.activeTransferComparison = try XCTUnwrap(TanComparison(iban: "DE89370400440532013000", amountCents: 1_234))
        // no challenge at all (still logging in): nothing to compare yet
        XCTAssertNil(model.tanComparison)
    }

    // (b) batch transfer, standing order and direct debit never show the single-transfer card
    func testNoComparisonForBatchStandingOrDebit() throws {
        let snap = try XCTUnwrap(TanComparison(iban: "DE89370400440532013000", amountCents: 1_234))
        XCTAssertNil(AppModel.visibleTanComparison(
            snapshot: snap, orderApproval: true, isBatch: true, isStanding: false, isDebit: false))
        XCTAssertNil(AppModel.visibleTanComparison(
            snapshot: snap, orderApproval: true, isBatch: false, isStanding: true, isDebit: false))
        XCTAssertNil(AppModel.visibleTanComparison(
            snapshot: snap, orderApproval: true, isBatch: false, isStanding: false, isDebit: true))
    }

    // (c) the card shows what was sent, not the form edited afterwards
    func testComparisonKeepsSentValuesWhenDraftChanges() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.transferDraft.iban = "DE89370400440532013000"
        model.transferDraft.amountText = "12,34"
        model.activeTransferComparison = TanComparison(iban: model.transferDraft.iban, amountCents: 1_234)
        model.transferDraft.iban = "DE02120300000000202051"
        model.transferDraft.amountText = "999,00"
        let shown = try XCTUnwrap(AppModel.visibleTanComparison(
            snapshot: model.activeTransferComparison, orderApproval: true,
            isBatch: model.activeTransferIsBatch, isStanding: model.activeTransferIsStanding,
            isDebit: model.activeTransferIsDebit))
        XCTAssertEqual(shown.endCharacters, "3000")
        XCTAssertEqual(shown.amount, Money.format(1_234))
    }

    // (d) an IBAN may end in letters: "Zeichen", not "Ziffern"
    func testInstructionSpeaksOfCharacters() {
        XCTAssertTrue(TanComparisonHint.instruction.contains("letzten 4 Zeichen"))
        XCTAssertFalse(TanComparisonHint.instruction.contains("Ziffern"))
    }
}
