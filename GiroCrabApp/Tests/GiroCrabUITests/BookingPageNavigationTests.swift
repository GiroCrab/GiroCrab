import XCTest
@testable import GiroCrabUI

/// Keyboard paging in the booking list (GitHub #1561).
final class BookingPageNavigationTests: XCTestCase {
    private let step = AppModel.keyboardPageStep

    func testPageDownMovesOnePageAndClampsAtEnd() {
        XCTAssertEqual(AppModel.selectionTarget(from: 0, delta: step, count: 25), step)
        XCTAssertEqual(AppModel.selectionTarget(from: 20, delta: step, count: 25), 24)
        XCTAssertEqual(AppModel.selectionTarget(from: 24, delta: step, count: 25), 24)
    }

    func testPageUpMovesOnePageAndClampsAtStart() {
        XCTAssertEqual(AppModel.selectionTarget(from: 24, delta: -step, count: 25), 24 - step)
        XCTAssertEqual(AppModel.selectionTarget(from: 3, delta: -step, count: 25), 0)
    }

    func testHomeAndEndDoNotOverflow() {
        XCTAssertEqual(AppModel.selectionTarget(from: 7, delta: Int.max, count: 25), 24)
        XCTAssertEqual(AppModel.selectionTarget(from: 7, delta: -Int.max, count: 25), 0)
        XCTAssertEqual(AppModel.selectionTarget(from: nil, delta: -Int.max, count: 25), 0)
        XCTAssertEqual(AppModel.selectionTarget(from: nil, delta: Int.max, count: 25), 24)
    }

    func testArrowStepsAndEmptyList() {
        XCTAssertEqual(AppModel.selectionTarget(from: nil, delta: 1, count: 25), 0)
        XCTAssertEqual(AppModel.selectionTarget(from: 5, delta: -1, count: 25), 4)
        XCTAssertNil(AppModel.selectionTarget(from: nil, delta: step, count: 0))
    }
}
