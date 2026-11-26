import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Tagesgeld as the sender of an internal transfer (GitHub #582).
@MainActor
final class SavingsTransferSourceTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    override func tearDown() async throws { model = nil }

    func testSavingsSourcesAreKeptOutOfTheOtherTransferForms() {
        let savings = Set(model.savingsTransferSources.map(\.id))
        XCTAssertTrue(savings.isDisjoint(with: model.transferAccounts.map(\.id)))
        XCTAssertTrue(model.savingsTransferSources.allSatisfy { $0.kind == .savings })
        XCTAssertTrue(savings.isSubset(of: model.transferFormAccounts.map(\.id)))
        XCTAssertTrue(savings.isDisjoint(with: model.giroCodeAccounts.map(\.id)))
    }

    func testSavingsAccountTransfersOnlyToOwnCheckingAccounts() throws {
        let source = try XCTUnwrap(model.allAccounts.first { $0.kind == .savings && !$0.iban.isEmpty })
        let targets = model.internalTransferTargets(from: source)
        XCTAssertFalse(targets.isEmpty)
        XCTAssertTrue(targets.allSatisfy { $0.kind == .checking && !$0.iban.isEmpty && $0.id != source.id })
    }

    func testSavingsAccountMayOnlySendToOwnCheckingIBAN() throws {
        let source = try XCTUnwrap(model.allAccounts.first { $0.kind == .savings && !$0.iban.isEmpty })
        let own = try XCTUnwrap(model.internalTransferTargets(from: source).first)
        XCTAssertTrue(model.mayTransfer(from: source, toIBAN: own.iban))
        XCTAssertTrue(model.mayTransfer(from: source, toIBAN: own.iban.lowercased()))
        XCTAssertFalse(model.mayTransfer(from: source, toIBAN: "DE02120300000000202051"))
        XCTAssertFalse(model.mayTransfer(from: source, toIBAN: ""))
        let other = try XCTUnwrap(model.allAccounts.first { $0.kind == .savings && $0.id != source.id })
        XCTAssertFalse(model.mayTransfer(from: source, toIBAN: other.iban))
        let checking = try XCTUnwrap(model.allAccounts.first { $0.kind == .checking })
        XCTAssertTrue(model.mayTransfer(from: checking, toIBAN: "DE02120300000000202051"))
    }

    func testDraftKeepsTheChosenOwnTargetForANewAttempt() {
        var draft = TransferDraft(accountID: 1)
        XCTAssertNil(draft.ownTargetID)
        draft.ownTargetID = 42
        model.transferDraft = draft
        model.transferInstantDeclined = nil
        model.transferDraft.instant = false // as `sendAsRegularTransfer` does
        XCTAssertEqual(model.transferDraft.ownTargetID, 42)
        XCTAssertNil(TransferDraft(accountID: 1).ownTargetID, "a new transfer starts without a target")
    }
}
