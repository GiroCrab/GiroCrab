import XCTest
import SwiftUI
@testable import GiroCrabUI

final class PieHoverTests: XCTestCase {
    private let frame = CGRect(x: 0, y: 0, width: 200, height: 200)
    private let parts: [(id: String, cents: Int64)] = [("a", 750), ("b", 250), ("zero", 0)]

    /// Stand-in for `ChartProxy.value(atAngle:)`: the cumulative value grows linearly with the angle.
    private func value(_ angle: Angle) -> Double? { angle.radians / (2 * .pi) * 1000 }

    private func hit(_ x: Double, _ y: Double, inner: CGFloat = 0.5) -> String? {
        PieHit.id(at: CGPoint(x: x, y: y), in: frame, innerRatio: inner, parts: parts, value: value)
    }

    func testSlicesFollowTheAngleClockwiseFromTwelveOClock() {
        XCTAssertEqual(hit(150, 20), "a")    // top right: 0–90°
        XCTAssertEqual(hit(180, 100), "a")   // three o'clock: 90°
        XCTAssertEqual(hit(100, 190), "a")   // six o'clock: 180°, still within the first 75 %
        XCTAssertEqual(hit(20, 100), "b")    // nine o'clock: 270°
        XCTAssertEqual(hit(60, 20), "b")     // top left: 315°
    }

    func testHoleAndOutsideAreNoSlice() {
        XCTAssertNil(hit(100, 100))          // centre
        XCTAssertNil(hit(100, 70))           // inside the hole (30 < 50)
        XCTAssertNil(hit(5, 5))              // corner, outside the ring
        XCTAssertNotNil(hit(100, 80, inner: 0))
    }

    func testEmptyRingAndMissingValueGiveNoSlice() {
        XCTAssertNil(PieHit.id(at: CGPoint(x: 150, y: 20), in: frame, innerRatio: 0.5, parts: [], value: value))
        XCTAssertNil(PieHit.id(at: CGPoint(x: 150, y: 20), in: frame, innerRatio: 0.5, parts: parts, value: { _ in nil }))
        XCTAssertNil(PieHit.id(at: CGPoint(x: 150, y: 20), in: .zero, innerRatio: 0.5, parts: parts, value: value))
    }

    func testShareTextNeedsAPositiveTotal() {
        XCTAssertEqual(PieValueMode.shareText(1, of: 0), "–")
        XCTAssertEqual(PieValueMode.shareText(250, of: 1000), DepotFormat.percent(25, digits: 1))
    }

    func testBothModesHaveATitle() {
        XCTAssertEqual(PieValueMode.allCases.count, 2)
        XCTAssertTrue(PieValueMode.allCases.allSatisfy { !$0.title.isEmpty })
        XCTAssertEqual(PieValueMode(rawValue: "share"), .share)
    }

    func testHeaderPieFoldPreservesPersonalFlagOnlyOnNamedAccountSlices() {
        let parts = [
            HeaderPieSlices.Part(id: "1", label: "Girokonto Max", cents: 600, personal: true),
            HeaderPieSlices.Part(id: "2", label: "Tagesgeld", cents: 300, personal: true),
            HeaderPieSlices.Part(id: "3", label: "Sparkonto", cents: 100, personal: true),
        ]
        let folded = HeaderPieSlices.fold(parts, limit: 2)
        XCTAssertEqual(folded.map(\.id), ["1", "rest"])
        XCTAssertTrue(folded[0].personal)
        XCTAssertFalse(folded[1].personal, "aggregated Sonstige label is not personal")
        XCTAssertFalse(HeaderPieSlices.Part(id: "cat", label: "Miete", cents: 500).personal)
    }

    func testCompositionLegendOpacityPreservesRealSelectionDuringHover() {
        XCTAssertEqual(CompositionLegend.entryOpacity(id: "a", selected: "a", hovered: "b"), 1.0)
        XCTAssertEqual(CompositionLegend.entryOpacity(id: "b", selected: "a", hovered: "b"), 0.75)
        XCTAssertEqual(CompositionLegend.entryOpacity(id: "c", selected: "a", hovered: "b"), 0.35)
        XCTAssertEqual(CompositionLegend.entryOpacity(id: "b", selected: nil, hovered: "b"), 1.0)
        XCTAssertEqual(CompositionLegend.entryOpacity(id: "a", selected: nil, hovered: "b"), 0.5)
    }
}
