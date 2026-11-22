import XCTest
@testable import GiroCrabUI

/// #1538: the demo banner keeps the same gap to the first card in the booking
/// list (`cardGap` stack) and in the asset account view (`assetBlockSpacing` stack).
final class DemoBannerSpacingTests: XCTestCase {
    func testBannerGapMatchesBookingListInAssetView() {
        let spacing = ViewLayout.assetBlockSpacing
        let effective = spacing + ViewLayout.demoBannerBottomPadding(stackSpacing: spacing)
        XCTAssertEqual(effective, ViewLayout.cardGap)
    }

    func testNoCorrectionInCardGapStack() {
        XCTAssertEqual(ViewLayout.demoBannerBottomPadding(stackSpacing: ViewLayout.cardGap), 0)
    }

    func testAssetBlocksKeepTheirSpacing() {
        XCTAssertEqual(ViewLayout.assetBlockSpacing, 14)
    }
}
