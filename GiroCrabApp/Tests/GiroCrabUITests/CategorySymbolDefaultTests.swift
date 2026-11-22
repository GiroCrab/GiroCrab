import XCTest
@testable import GiroCrabUI

/// #1610: new and imported categories share one default symbol, and both
/// tag variants can be picked for existing categories.
final class CategorySymbolDefaultTests: XCTestCase {
    func testDefaultMatchesCoreDefault() {
        // The core stores "tag" for an empty symbol (create_category, schema
        // default, file/database import); the UI must not diverge from it.
        XCTAssertEqual(SymbolCatalog.defaultCategorySymbol, "tag")
    }

    func testBothTagVariantsAreListed() {
        XCTAssertTrue(SymbolCatalog.isListed("tag"))
        XCTAssertTrue(SymbolCatalog.isListed("tag.fill"))
    }

    func testDefaultIsListed() {
        XCTAssertTrue(SymbolCatalog.isListed(SymbolCatalog.defaultCategorySymbol))
    }
}
