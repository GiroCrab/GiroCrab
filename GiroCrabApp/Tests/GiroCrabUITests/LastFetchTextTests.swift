import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// #1716: after a single-account fetch the account shows its own time; the other accounts of the login
/// keep the login's last full sync.
@MainActor
final class LastFetchTextTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws { model = nil }

    private func online(_ pick: (Account) -> Bool) throws -> Account {
        try XCTUnwrap(model.allAccounts.first { !model.isOffline($0.id) && $0.connectionId != nil && pick($0) })
    }

    func testAccountFetchedOnItsOwnShowsItsOwnTimeAndSaysSo() throws {
        let own = try online { $0.lastFetchUnix != nil }
        let fetch = try XCTUnwrap(model.lastFetchText(for: own))
        XCTAssertFalse(fetch.text.isEmpty)
        XCTAssertTrue(
            try XCTUnwrap(fetch.help).contains("\n"),
            "the tooltip names the difference to the whole login: \(String(describing: fetch.help))")
    }

    func testOtherAccountsKeepTheLoginsTimeWithoutNote() throws {
        let other = try online { $0.lastFetchUnix == nil }
        let fetch = try XCTUnwrap(model.lastFetchText(for: other))
        XCTAssertFalse(try XCTUnwrap(fetch.help).contains("\n"))
    }
}
