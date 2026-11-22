import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// "<Konto> abrufen" shows the sidebar spinner on that account only, not on
/// every account of its login (#1572).
@MainActor
final class SyncAccountIndicatorTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws {
        model.cancelAllSync()
        model = nil
    }

    /// A login with at least two active accounts that can fetch one of them.
    private func loginWithTwoAccounts() throws -> (BankConnection, Account, Account) {
        for c in model.connections where model.supportsAccountSync(c) {
            let accounts = model.accountsByID.values.filter { $0.connectionId == c.id && !$0.isArchived }.sorted { $0.id < $1.id }
            if accounts.count >= 2 { return (c, accounts[0], accounts[1]) }
        }
        throw XCTSkip("demo vault needs a login with two active accounts")
    }

    func testAccountSyncMarksOnlyThatAccount() throws {
        let (connection, first, second) = try loginWithTwoAccounts()
        model.syncAccountID = second.id   // as `requestSyncAccount` leaves it
        model.startSync(connection, secret: "demo")
        XCTAssertTrue(model.isSyncing(second))
        XCTAssertFalse(model.isSyncing(first))
    }

    func testFullSyncMarksEveryAccountOfTheLogin() throws {
        let (connection, first, second) = try loginWithTwoAccounts()
        model.syncAccountID = second.id
        model.startSync(connection, secret: "demo")
        model.cancelAllSync()
        XCTAssertFalse(model.isSyncing(second))

        model.syncAccountID = nil   // as `requestFullSync` leaves it
        model.startSync(connection, secret: "demo")
        XCTAssertTrue(model.isSyncing(first))
        XCTAssertTrue(model.isSyncing(second))
        let other = model.accountsByID.values.first { $0.connectionId != connection.id }
        if let other { XCTAssertFalse(model.isSyncing(other)) }
    }
}
