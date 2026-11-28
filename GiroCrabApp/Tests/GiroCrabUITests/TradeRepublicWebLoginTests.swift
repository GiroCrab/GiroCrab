import XCTest
import WebKit
@testable import GiroCrabUI

/// #2074: Trade Republic's web app checks the user agent for iPhone/iPad/Android and then
/// shows only "get the app", without a login form. The login browser must ask for the
/// desktop website on every platform (macOS and iOS share `makeConfiguration`).
@MainActor
final class TradeRepublicWebLoginTests: XCTestCase {
    func testLoginBrowserRequestsTheDesktopWebsite() {
        let cfg = TRWebCoordinator.makeConfiguration()
        XCTAssertEqual(cfg.defaultWebpagePreferences.preferredContentMode, .desktop)
    }
}
