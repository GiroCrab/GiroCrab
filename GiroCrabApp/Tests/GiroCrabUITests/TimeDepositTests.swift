import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Festgeld (#1002): counts towards the wealth, but not towards "Verfügbar", and no transfer goes out of it.
@MainActor
final class TimeDepositTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws { model = nil }

    private enum TestError: Error { case accountMissing }

    /// A Festgeld account next to the demo accounts; goes through the vault like `OfflineAccountsTests`.
    private func addTimeDeposit() async throws -> Account {
        let vault = try XCTUnwrap(model.vault)
        let id = try vault.createOfflineAccount(name: "Festgeld 12 Monate", kind: .timeDeposit, currency: "EUR", openingCents: 5_000_00)
        model.offlineStore.accountIDs.insert(id)
        model.reloadAccounts()
        for _ in 0..<300 {
            if let a = model.account(for: id), a.balanceCents == 5_000_00 { return a }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("The sidebar did not show the new account")
        throw TestError.accountMissing
    }

    func testKindIsOfferedInPickersWithOwnLabel() {
        XCTAssertTrue(AccountKind.importChoices.contains(.timeDeposit))
        XCTAssertEqual(Set(AccountKind.importChoices.map(\.label)).count, AccountKind.importChoices.count)
    }

    func testTimeDepositIsWealthButNotAvailable() async throws {
        let before = model.liquidBalance
        let deposit = try await addTimeDeposit()
        XCTAssertEqual(deposit.kind, .timeDeposit)
        XCTAssertFalse(model.liquidAccounts.contains { $0.id == deposit.id })
        XCTAssertEqual(model.liquidBalance, before, "'Verfügbar' stays as it was")
        XCTAssertEqual(WealthBucket.bucket(for: deposit), .cash)
        XCTAssertTrue(WealthSummary(accounts: model.allAccounts).cash.contains { $0.id == deposit.id })
    }

    /// A connected account with an IBAN that can send today (the demo Tagesgeld) stops being a source once it is a Festgeld.
    func testNoTransferGoesOutOfTimeDeposit() async throws {
        let source = try XCTUnwrap(model.savingsTransferSources.first)
        XCTAssertTrue(model.transferFormAccounts.contains { $0.id == source.id })
        XCTAssertTrue(model.canSendTransfer(from: source.id))

        model.setAccountKind(source.id, to: .timeDeposit)
        XCTAssertEqual(model.account(for: source.id)?.kind, .timeDeposit)
        XCTAssertFalse(model.savingsTransferSources.contains { $0.id == source.id })
        XCTAssertFalse(model.transferFormAccounts.contains { $0.id == source.id })
        XCTAssertFalse(model.canSendTransfer(from: source.id))
    }
}
