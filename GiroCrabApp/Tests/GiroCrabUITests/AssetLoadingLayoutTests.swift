import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// #1537: the asset account pages must not jump while they load.
final class AssetLoadingLayoutTests: XCTestCase {
    func testTangibleAssetListWaitsForClassification() {
        // still loading: neither card nor list, so the list cannot take the card's place
        XCTAssertFalse(TangibleAccountDetailView.showsAssetList(classificationLoaded: false, hasError: false))
        // loaded: list together with the composition card
        XCTAssertTrue(TangibleAccountDetailView.showsAssetList(classificationLoaded: true, hasError: false))
        // failed: the list stays usable below the error
        XCTAssertTrue(TangibleAccountDetailView.showsAssetList(classificationLoaded: false, hasError: true))
    }

    func testLoadingIndicatorDelayIsShortButNotZero() {
        XCTAssertGreaterThan(DelayedLoadingIndicator.delay, .zero)
        XCTAssertLessThan(DelayedLoadingIndicator.delay, .seconds(1))
    }
}
