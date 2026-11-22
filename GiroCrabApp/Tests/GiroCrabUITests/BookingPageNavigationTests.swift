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

    private let ids = (0..<25).map { "b\($0)" }

    func testMoveOntoSelectedBoundaryRowRequestsScroll() {
        // Pos1 on the selected first row: same row, explicit scroll request
        let home = AppModel.keyboardMove(ids: ids, current: "b0", delta: -Int.max)
        XCTAssertEqual(home?.index, 0)
        XCTAssertEqual(home?.rescroll, true)
        // Ende and Bild↓ on the selected last row
        let end = AppModel.keyboardMove(ids: ids, current: "b24", delta: Int.max)
        XCTAssertEqual(end?.index, 24)
        XCTAssertEqual(end?.rescroll, true)
        let pageDown = AppModel.keyboardMove(ids: ids, current: "b24", delta: step)
        XCTAssertEqual(pageDown?.index, 24)
        XCTAssertEqual(pageDown?.rescroll, true)
        // Bild↑ and arrow up on the selected first row
        XCTAssertEqual(AppModel.keyboardMove(ids: ids, current: "b0", delta: -step)?.rescroll, true)
        XCTAssertEqual(AppModel.keyboardMove(ids: ids, current: "b0", delta: -1)?.rescroll, true)
    }

    func testMoveToOtherRowLeavesScrollToSelectionChange() {
        let home = AppModel.keyboardMove(ids: ids, current: "b7", delta: -Int.max)
        XCTAssertEqual(home?.index, 0)
        XCTAssertEqual(home?.rescroll, false)
        let pageDown = AppModel.keyboardMove(ids: ids, current: "b20", delta: step)
        XCTAssertEqual(pageDown?.index, 24)
        XCTAssertEqual(pageDown?.rescroll, false)
    }

    func testMoveWithoutVisibleSelection() {
        // nothing selected, or the selection is no longer in the list (filter changed)
        let none = AppModel.keyboardMove(ids: ids, current: nil, delta: Int.max)
        XCTAssertEqual(none?.index, 24)
        XCTAssertEqual(none?.rescroll, false)
        let gone = AppModel.keyboardMove(ids: ids, current: "missing", delta: 1)
        XCTAssertEqual(gone?.index, 0)
        XCTAssertEqual(gone?.rescroll, false)
        XCTAssertNil(AppModel.keyboardMove(ids: [String](), current: nil, delta: 1))
    }
}
