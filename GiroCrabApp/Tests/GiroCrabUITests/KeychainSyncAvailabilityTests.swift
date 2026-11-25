import Security
import XCTest
@testable import GiroCrabUI

/// iCloud Keychain sharing needs the data-protection keychain. A build without
/// the entitlement (the notarized Mac app so far) must not offer a switch that
/// can only fail (#909).
final class KeychainSyncAvailabilityTests: XCTestCase {
    func testMissingEntitlementMeansUnavailable() {
        XCTAssertFalse(VaultKeyManager.iCloudKeychainAvailable(probe: errSecMissingEntitlement))
    }

    func testReachableKeychainMeansAvailable() {
        XCTAssertTrue(VaultKeyManager.iCloudKeychainAvailable(probe: errSecItemNotFound))
        XCTAssertTrue(VaultKeyManager.iCloudKeychainAvailable(probe: errSecSuccess))
    }

    #if os(macOS)
    /// The `swift test` process carries no entitlements, like the notarized Mac app.
    /// Only a write reveals that: a lookup answers "not found" either way.
    func testUnentitledProcessCannotShare() {
        XCTAssertFalse(VaultKeyManager.canShareViaICloudKeychain)
    }
    #endif

    /// #1798: the note is there before either switch is turned, and as long as the key is shared.
    func testTradeOffIsShownBeforeAndWhileSharing() throws {
        func note(_ available: Bool, _ shared: Bool, _ sync: Bool) -> String? {
            KeychainSyncNote.tradeOff(available: available, shared: shared, syncEnabled: sync)
        }
        // the iCloud switch shares the key as well
        let before = try XCTUnwrap(note(true, false, false))
        XCTAssertTrue(before.contains("Touch ID"), before)
        let atKeySwitch = try XCTUnwrap(note(true, false, true))
        XCTAssertTrue(before.hasSuffix(atKeySwitch), before)
        XCTAssertNotEqual(before, atKeySwitch)
        XCTAssertEqual(note(true, true, true), atKeySwitch)
        // switching the sync off does not unshare the key
        let still = try XCTUnwrap(note(true, true, false))
        XCTAssertTrue(still.hasSuffix(atKeySwitch), still)
        XCTAssertNotEqual(still, before)
        XCTAssertNotEqual(still, atKeySwitch)
        XCTAssertEqual(note(false, true, true), atKeySwitch)
        XCTAssertNil(note(false, false, false))
        XCTAssertNil(note(false, false, true))
    }

    func testUnavailableSwitchExplainsItself() {
        XCTAssertNil(KeychainSyncNote.text(available: true))
        let note = KeychainSyncNote.text(available: false)
        XCTAssertNotNil(note)
        XCTAssertTrue(note?.contains("Tresor-Schlüssel kopieren") == true, note ?? "")
    }
}
