import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Crypto holdings (#1836): a new holding is a depot the app knows as crypto, and a change of its
/// positions reloads the views even when the balance stays the same (no price yet).
@MainActor
final class CryptoHoldingTests: XCTestCase {
    func testANewHoldingIsSelectedAndAChangeBumpsTheRevision() throws {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        model.depotTab = .activities
        model.cryptoAccountCreated(try XCTUnwrap(model.vault).createCryptoAccount(name: "Wallet"))
        XCTAssertEqual(model.depotTab, .positions, "opens where a position can be added")
        guard case .account(let id) = model.sidebarSelection else { return XCTFail("not selected") }
        XCTAssertTrue(model.isCrypto(id))
        XCTAssertEqual(model.account(for: id)?.kind, .brokerage)
        XCTAssertEqual(model.depotKind(of: id), .securities)

        // synthetic data; no price is fetched here
        try XCTUnwrap(model.vault).saveCryptoPosition(accountId: id, previousSymbol: nil, symbol: "BTC",
                                                       quantity: 0.5, purchasePriceCents: nil)
        let revision = model.cryptoStore.revision
        model.cryptoChanged()
        XCTAssertEqual(model.cryptoStore.revision, revision + 1)

        model.deleteCryptoPosition(CryptoPositionRequest(accountId: id, previousSymbol: "BTC"))
        XCTAssertNil(model.errorMessage)
        XCTAssertTrue(try XCTUnwrap(model.vault).listCryptoPositions(accountId: id).isEmpty)
    }

    func testAPriceFetchReloadsAnOpenSyncLog() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let version = model.syncLogVersion
        model.cryptoPricesLogged()
        XCTAssertEqual(model.syncLogVersion, version &+ 1)
    }
}
