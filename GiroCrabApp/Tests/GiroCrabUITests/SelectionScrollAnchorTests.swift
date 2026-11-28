import SwiftUI
import XCTest
@testable import GiroCrabUI

/// #993: selecting a booking in the compact list must not jump it to the top.
final class SelectionScrollAnchorTests: XCTestCase {
    func testClickInCompactListDoesNotPinRowToTop() {
        XCTAssertNil(SelectionScrollAnchor.anchor(density: .compact, byKeyboard: false))
    }

    func testKeyboardInCompactListKeepsExpandedTitleVisible() {
        XCTAssertEqual(SelectionScrollAnchor.anchor(density: .compact, byKeyboard: true), .top)
    }

    func testStandardListScrollsMinimally() {
        XCTAssertNil(SelectionScrollAnchor.anchor(density: .standard, byKeyboard: false))
        XCTAssertNil(SelectionScrollAnchor.anchor(density: .standard, byKeyboard: true))
    }
}
