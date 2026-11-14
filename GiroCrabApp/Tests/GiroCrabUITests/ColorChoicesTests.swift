import XCTest
@testable import GiroCrabUI

/// The colour picker: fixed swatches plus the custom colours already in use.
final class ColorChoicesTests: XCTestCase {
    private let palette = ["#2563EB", "#D97706"]

    func testExtraSwatchesAddYellowBrownAndSlate() {
        let all = ColorChoices.swatches(palette: palette)
        XCTAssertEqual(Array(all.prefix(2)), palette)
        XCTAssertEqual(all.count, palette.count + 3)
        XCTAssertEqual(Set(all).count, all.count)
    }

    func testCustomColoursAreTheUsedOnesOutsideTheSwatches() {
        let used = ["#2563EB", "#ff0000", "#FF0000", "", "#00ff00", "#FF0000", ColorChoices.extras[0]]
        XCTAssertEqual(ColorChoices.customColors(used: used, palette: palette), ["#FF0000", "#00FF00"])
    }

    func testCustomColoursAreCapped() {
        let used = (0..<40).map { String(format: "#0000%02X", $0) }
        XCTAssertEqual(ColorChoices.customColors(used: used, palette: palette).count, ColorChoices.maxCustom)
    }
}
