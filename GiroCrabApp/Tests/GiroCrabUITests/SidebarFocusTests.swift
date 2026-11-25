#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import GiroCrabUI

// test: serial-defaults
// (the main window's `@AppStorage` values; and waiting for the focus needs a machine that is not busy)

/// Keyboard focus between the sidebar and the lists of the main column (#1779), in a real window:
/// `@FocusState` only works there.
@MainActor
final class SidebarFocusTests: XCTestCase {
    /// SwiftUI moves the focus only in the key window; a test process does not get one reliably.
    private final class KeyWindow: NSWindow {
        override var isKeyWindow: Bool { true }
        override var isMainWindow: Bool { true }
        override var canBecomeKey: Bool { true }
    }

    private var model: AppModel!
    private var window: NSWindow!
    private var sidebar: NSTableView!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        window = KeyWindow(contentRect: NSRect(x: 0, y: 0, width: 1400, height: 900),
                           styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.alphaValue = 0 // nothing flashes up on the developer's screen
        window.contentView = NSHostingView(rootView: MainSplitView().environment(model))
        window.makeKeyAndOrderFront(nil)
        try await settle()
        sidebar = try XCTUnwrap(Self.table(in: XCTUnwrap(window.contentView)))
    }

    override func tearDown() async throws {
        window.orderOut(nil)
        window.contentView = nil
        window = nil
        sidebar = nil
        model = nil
    }

    private static func table(in view: NSView) -> NSTableView? {
        if let table = view as? NSTableView { return table }
        return view.subviews.lazy.compactMap { table(in: $0) }.first
    }

    /// The list appears with its data, and takes the focus only then.
    private func settle() async throws { try await Task.sleep(for: .milliseconds(600)) }

    private var sidebarHasFocus: Bool { window.firstResponder === sidebar }

    private func account(_ name: String) throws -> SidebarItem {
        .account(try XCTUnwrap(model.allAccounts.first { $0.name == name }).id)
    }

    /// Like ↓ from `first` to `second` in the sidebar.
    private func assertSidebarKeepsFocus(from first: SidebarItem, to second: SidebarItem,
                                         file: StaticString = #filePath, line: UInt = #line) async throws {
        model.selectFromSidebar(first)
        try await settle()
        window.makeFirstResponder(sidebar)
        try await settle()
        XCTAssertTrue(sidebarHasFocus, "precondition", file: file, line: line)
        model.selectFromSidebar(second)
        try await settle()
        XCTAssertTrue(sidebarHasFocus, "\(String(describing: window.firstResponder))", file: file, line: line)
    }

    func test_direct_debits_leave_focus_in_sidebar() async throws {
        try await assertSidebarKeepsFocus(from: .planned, to: .directDebits)
        XCTAssertFalse(model.mandates.isEmpty) // the list was really there
    }

    func test_loan_after_tangibles_leaves_focus_in_sidebar() async throws {
        try await assertSidebarKeepsFocus(from: account("Sachwerte"), to: account("Immobilien-Darlehen"))
    }

    func test_all_accounts_after_wealth_leave_focus_in_sidebar() async throws {
        try await assertSidebarKeepsFocus(from: .wealth, to: .allAccounts)
    }

    /// A list opened from elsewhere (a tile in „Vermögen“, the palette) still takes the focus.
    func test_list_takes_focus_when_sidebar_has_none() async throws {
        model.selectFromSidebar(.wealth)
        try await settle()
        window.makeFirstResponder(sidebar)
        try await settle()
        XCTAssertTrue(model.sidebarHasKeyboardFocus)
        window.makeFirstResponder(nil)
        try await settle()
        XCTAssertFalse(model.sidebarHasKeyboardFocus)
        let before = window.firstResponder
        model.sidebarSelection = .allAccounts
        try await settle()
        XCTAssertFalse(sidebarHasFocus)
        XCTAssertFalse(window.firstResponder === before, "\(String(describing: before))")
    }
}
#endif
