#if os(macOS)
import XCTest
@testable import GiroCrabUI

/// Booking texts get their AppKit text view only when it is needed (scrolling a long list).
final class BookingRowTextTests: XCTestCase {
    func testTextViewOnlyUnderThePointer() {
        var presence = BookingTextViewPresence()
        XCTAssertFalse(presence.isShown, "a row scrolled past builds no text view")
        presence.isHovered = true
        XCTAssertTrue(presence.isShown, "click or drag on the text reaches the text view")
        presence.isHovered = false
        XCTAssertFalse(presence.isShown)
    }

    func testSelectionStaysCopyableAfterThePointerLeaves() {
        var presence = BookingTextViewPresence()
        presence.isHovered = true
        presence.hasFocus = true // text selected
        presence.isHovered = false
        XCTAssertTrue(presence.isShown, "⌘C still copies the selection")
        presence.hasFocus = false // clicked elsewhere
        XCTAssertFalse(presence.isShown)
    }
}
#endif
