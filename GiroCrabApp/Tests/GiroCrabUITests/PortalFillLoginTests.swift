#if os(macOS)
import XCTest
import SwiftUI
import WebKit
@testable import GiroCrabUI

/// Real WebKit, local HTML with a fictional host as base URL, ephemeral storage, no network.
@MainActor
final class PortalFillLoginTests: XCTestCase {
    private func fill(existingUser: String, user: String = "fiktiv") async throws -> (Bool, String, String) {
        _ = NSApplication.shared
        let browser = PortalBrowser(url: URL(string: "about:blank")!, domains: PortalDomains(["portal.example"]),
            captureTraffic: false, websiteDataStore: .nonPersistent())
        defer { browser.teardown() }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 650),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        browser.webView.frame = window.contentLayoutRect
        window.contentView = browser.webView
        defer { window.close() }
        let html = """
        <html><body><script>window.fixtureReady=true;</script>
        <form><input id="user" name="username" value="\(existingUser)"><input id="pw" type="password"></form>
        </body></html>
        """
        browser.webView.loadHTMLString(html, baseURL: URL(string: "https://portal.example/login")!)
        var loaded = false
        for _ in 0..<200 {
            if (try? await browser.webView.evaluateJavaScript("window.fixtureReady") as? Bool) == true,
               !browser.webView.isLoading { loaded = true; break }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertTrue(loaded)
        let result = await browser.fillLogin(user: user, password: "fiktiv-pw")
        let u = try await browser.webView.evaluateJavaScript("document.getElementById('user').value") as? String ?? ""
        let p = try await browser.webView.evaluateJavaScript("document.getElementById('pw').value") as? String ?? ""
        return (result, u, p)
    }

    func testEmptyUserFieldIsFilledTogetherWithPassword() async throws {
        let (ok, user, pw) = try await fill(existingUser: "")
        XCTAssertTrue(ok)
        XCTAssertEqual(user, "fiktiv")
        XCTAssertEqual(pw, "fiktiv-pw")
    }

    func testMatchingUserFieldKeepsUserAndFillsPassword() async throws {
        let (ok, user, pw) = try await fill(existingUser: "fiktiv")
        XCTAssertTrue(ok)
        XCTAssertEqual(user, "fiktiv")
        XCTAssertEqual(pw, "fiktiv-pw")
    }

    func testDifferentUserFieldAbortsAndWritesNothing() async throws {
        let (ok, user, pw) = try await fill(existingUser: "anderer")
        XCTAssertFalse(ok)
        XCTAssertEqual(user, "anderer")
        XCTAssertEqual(pw, "")
    }
}
#endif
