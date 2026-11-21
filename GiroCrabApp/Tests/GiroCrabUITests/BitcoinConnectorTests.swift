import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Bitcoin wallet (watch-only xpub/ypub/zpub): set up like a bank login without
/// a password, its own server is an optional setting of the connection.
final class BitcoinConnectorTests: XCTestCase {
    func testCatalogEntryNeedsNoSecretAndNoRequiredSetting() throws {
        let bank = try XCTUnwrap(bankCatalog().first { $0.kind == .bitcoin })
        XCTAssertEqual(bank.name, "Bitcoin")
        XCTAssertTrue(bank.requiredSettings.isEmpty, "mempool.space works without any setting")
        // a label in brackets means "no password": the sync starts without a login prompt
        XCTAssertTrue(bank.secretLabel.hasPrefix("("))
        XCTAssertTrue(bank.loginLabel.contains("xpub"))
        XCTAssertEqual(bank.sfSymbol, "bitcoinsign.circle")
    }

    func testWalletIsNoPortalLoginAndHasItsOwnLook() {
        XCTAssertNil(ConnectorKind.bitcoin.portalURL)
        XCTAssertEqual(ConnectorKind.bitcoin.displayName, "Bitcoin")
        XCTAssertEqual(ConnectorKind.bitcoin.tintHex, "#F7931A")
        XCTAssertEqual(ConnectorKind.bitcoin.monogram, "BTC")
    }
}
