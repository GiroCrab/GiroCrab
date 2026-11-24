// test: serial-defaults
import XCTest
@testable import GiroCrabUI

/// Bitcoin Core's data directory as Bitcoin-Qt remembers it (`strDataDir`).
final class BitcoinCoreDatadirTests: XCTestCase {
    private var suiteName = ""
    private var defaults: UserDefaults!

    override func setUp() {
        suiteName = "BitcoinCoreDatadirTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
    }

    func testTheDirectoryBitcoinQtUsesIsFound() {
        defaults.set("/Volumes/Node/BitCoin", forKey: "strDataDir")
        XCTAssertEqual(BitcoinCoreDatadir.detected(defaults), "/Volumes/Node/BitCoin")
    }

    func testNothingIsFoundWithoutBitcoinQt() {
        XCTAssertNil(BitcoinCoreDatadir.detected(defaults))
        defaults.set("  ", forKey: "strDataDir")
        XCTAssertNil(BitcoinCoreDatadir.detected(defaults))
        XCTAssertNil(BitcoinCoreDatadir.detected(nil))
    }

    /// A Mac without a node (the vault came over iCloud): no folder exists, nothing is fetched there.
    func testANodeExistsOnlyWhereOneOfItsFoldersIs() throws {
        let missing = "/nonexistent-\(UUID().uuidString)"
        XCTAssertFalse(BitcoinCoreDatadir.nodeOnThisMac(configured: missing, detected: nil, standard: missing))
        XCTAssertFalse(BitcoinCoreDatadir.nodeOnThisMac(configured: "", detected: nil, standard: missing))
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("btc-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        XCTAssertTrue(BitcoinCoreDatadir.nodeOnThisMac(configured: dir.path, detected: nil, standard: missing))
        XCTAssertTrue(BitcoinCoreDatadir.nodeOnThisMac(configured: missing, detected: dir.path, standard: missing))
        XCTAssertTrue(BitcoinCoreDatadir.nodeOnThisMac(configured: "", detected: nil, standard: dir.path))
    }
}
