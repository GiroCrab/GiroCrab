// test: serial-defaults
// (the main window's `@AppStorage` values; and the timing needs a machine that is not busy)
#if os(macOS)
import AppKit
import SwiftUI
import XCTest
@testable import GiroCrabUI

/// ↑/↓ in the booking list scroll the selected booking into view, in a real window.
@MainActor
final class BookingListScrollTests: XCTestCase {
    private var model: AppModel!
    private var window: NSWindow!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1400, height: 800),
                          styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.alphaValue = 0 // nothing flashes up on the developer's screen
        window.contentView = NSHostingView(rootView: MainSplitView().environment(model).environment(\.bookingDensity, .standard))
        window.orderFront(nil)
        try await wait(for: "list") { (try? self.list()) != nil }
    }

    override func tearDown() async throws {
        window.orderOut(nil)
        window.contentView = nil
        window = nil
        model = nil
    }

    private func wait(for what: String, file: StaticString = #filePath, line: UInt = #line,
                      until condition: () -> Bool) async throws {
        for _ in 0..<1000 {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("timeout: \(what)", file: file, line: line)
    }

    /// The selection's data is in and its list taller than the window.
    private func show(_ item: SidebarItem, file: StaticString = #filePath, line: UInt = #line) async throws {
        model.sidebarSelection = item
        try await wait(for: "list of \(item)", file: file, line: line) {
            guard self.model.selectionSettled, let list = try? self.list(), let document = list.documentView else { return false }
            return self.model.shownSelection == item && document.frame.height > list.contentView.bounds.height
        }
        try await scrollSettles()
    }

    /// Waits until the list has stood still for 300 ms.
    private func scrollSettles() async throws {
        var last = try offset(), unchanged = 0
        for _ in 0..<500 where unchanged < 30 {
            try await Task.sleep(for: .milliseconds(10))
            let now = try offset()
            unchanged = now == last ? unchanged + 1 : 0
            last = now
        }
    }

    private func list() throws -> NSScrollView {
        func scrollViews(_ view: NSView) -> [NSScrollView] {
            (view as? NSScrollView).map { [$0] } ?? view.subviews.flatMap(scrollViews)
        }
        let candidates = scrollViews(try XCTUnwrap(window.contentView)).filter { !($0.documentView is NSTableView) }
        return try XCTUnwrap(candidates.max { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height })
    }

    private func offset() throws -> CGFloat { try list().contentView.bounds.minY }

    func test_new_bookings_scroll_with_selection() async throws {
        try await show(.newBookings)
        XCTAssertGreaterThan(model.visibleBookings.count, 10)
        XCTAssertEqual(try offset(), 0)
        model.moveSelection(.max)
        try await scrollSettles()
        XCTAssertGreaterThan(try offset(), 0)
        model.moveSelection(-.max)
        try await scrollSettles()
        XCTAssertLessThan(try offset(), try list().contentView.bounds.height)
    }

    /// Key repeat: the list is where the selection is, not only once the key is released.
    func test_list_keeps_up_with_held_arrow_key() async throws {
        try await show(.allAccounts)
        for _ in 0..<25 {
            model.moveSelection(1)
            try await Task.sleep(for: .milliseconds(35))
        }
        let held = try offset()
        try await scrollSettles()
        XCTAssertGreaterThan(try offset(), 0)
        XCTAssertEqual(held, try offset(), accuracy: 120) // about two rows
    }
}
#endif
