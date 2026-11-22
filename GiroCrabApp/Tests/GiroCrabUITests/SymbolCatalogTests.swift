import XCTest
@testable import GiroCrabUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Category symbols for donations, vehicles, software, shipping, pets and
/// more: listed, found by name, and no symbol twice in the catalogue.
final class SymbolCatalogTests: XCTestCase {
    func testNewSymbolsAreListed() {
        // All of these exist from macOS 14 / iOS 17 (the deployment target).
        for symbol in ["heart.circle", "pianokeys", "guitars", "play.tv", "dog", "cat", "fish",
                       "chevron.left.forwardslash.chevron.right", "checkmark.seal", "shippingbox",
                       "suitcase.rolling"] {
            XCTAssertTrue(SymbolCatalog.isListed(symbol), symbol)
        }
    }

    func testMotorcycleListedWhereTheSystemHasIt() {
        // `motorcycle` ships from macOS 15 / iOS 18; older systems drop it from the list.
        #if os(macOS)
        let has = NSImage(systemSymbolName: "motorcycle", accessibilityDescription: nil) != nil
        #else
        let has = UIImage(systemName: "motorcycle") != nil
        #endif
        XCTAssertEqual(SymbolCatalog.isListed("motorcycle"), has)
    }

    func testSearchFindsDonationSymbols() {
        // "Spende"/"Donation" (whatever the test locale) is a prefix of "Spenden"/"Donations".
        let query = String(localized: SymbolCatalog.name(of: "hand.raised")!)
        let hits = Set(SymbolCatalog.search(query).flatMap { $0.items.map(\.symbol) })
        XCTAssertTrue(hits.isSuperset(of: ["hand.raised", "heart.circle"]))
    }

    func testNoSymbolListedTwice() {
        let all = SymbolCatalog.sections.flatMap { $0.items.map(\.symbol) }
        XCTAssertEqual(all.count, Set(all).count)
    }
}
