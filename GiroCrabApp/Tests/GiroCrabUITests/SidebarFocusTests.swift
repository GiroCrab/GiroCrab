// test: serial-defaults
// (the main window's `@AppStorage` values; and waiting for the focus needs a machine that is not busy)
#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import GiroCrabUI

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
        try await wait(for: "sidebar and list") { Self.table(in: self.content) != nil && !self.mainLists.isEmpty }
        sidebar = try XCTUnwrap(Self.table(in: content))
        try await focusSettles()
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

    private var content: NSView { window.contentView ?? NSView() }

    /// Scroll views on screen other than the sidebar's: another one means that another view has appeared.
    private var mainLists: [NSScrollView] {
        func all(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { $0.documentView is NSTableView ? [] : [$0] } ?? view.subviews.flatMap(all)
        }
        return all(content)
    }

    private func wait(for what: String, file: StaticString = #filePath, line: UInt = #line,
                      until condition: () -> Bool) async throws {
        for _ in 0..<1000 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("timeout: \(what)", file: file, line: line)
    }

    /// A view takes the focus in the update after it appears: waits until nothing moved for 300 ms.
    private func focusSettles() async throws {
        var last = window.firstResponder, unchanged = 0
        for _ in 0..<500 where unchanged < 30 {
            try await Task.sleep(for: .milliseconds(10))
            unchanged = window.firstResponder === last ? unchanged + 1 : 0
            last = window.firstResponder
        }
    }

    /// Selects `item` and waits for its data, for its view to appear and for the focus.
    private func show(_ item: SidebarItem, file: StaticString = #filePath, line: UInt = #line) async throws {
        let before = mainLists // held, so that no address is reused
        model.selectFromSidebar(item)
        try await wait(for: "view of \(item)", file: file, line: line) {
            self.model.selectionSettled && (item != .directDebits || !self.model.mandates.isEmpty)
                && self.mainLists.contains { list in !before.contains { $0 === list } }
        }
        try await focusSettles()
    }

    private func focusSidebar() async throws {
        window.makeFirstResponder(sidebar)
        try await wait(for: "sidebar focus") { self.model.sidebarHasKeyboardFocus }
    }

    private var sidebarHasFocus: Bool { window.firstResponder === sidebar }

    private func account(_ name: String) throws -> SidebarItem {
        .account(try XCTUnwrap(model.allAccounts.first { $0.name == name }).id)
    }

    /// Like ↓ from `first` to `second` in the sidebar.
    private func assertSidebarKeepsFocus(from first: SidebarItem, to second: SidebarItem,
                                         file: StaticString = #filePath, line: UInt = #line) async throws {
        try await show(first, file: file, line: line)
        try await focusSidebar()
        XCTAssertTrue(sidebarHasFocus, "precondition", file: file, line: line)
        try await show(second, file: file, line: line)
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
        try await show(.wealth)
        try await focusSidebar()
        window.makeFirstResponder(nil)
        try await wait(for: "no sidebar focus") { !self.model.sidebarHasKeyboardFocus }
        let before = window.firstResponder
        try await show(.allAccounts)
        XCTAssertFalse(sidebarHasFocus)
        XCTAssertFalse(window.firstResponder === before, "\(String(describing: before))")
    }
}
#endif
