import XCTest
@testable import GiroCrabUI

/// Account rows of the sidebar: spacious (one line) or compact (#884).
final class SidebarDensityTests: XCTestCase {
    func testSpaciousIsTheDefault() {
        XCTAssertEqual(SidebarDensity.defaultValue, .spacious)
        XCTAssertEqual(SidebarDensity.defaultsKey, "sidebarDensity")
    }

    func testStoredValuesStayReadable() {
        XCTAssertEqual(SidebarDensity(rawValue: "spacious"), .spacious)
        XCTAssertEqual(SidebarDensity(rawValue: "compact"), .compact)
    }

    func testOwnerInitials() {
        XCTAssertEqual(SidebarDensity.initials(of: "Erika Mustermann"), "EM")
        XCTAssertEqual(SidebarDensity.initials(of: "anna maria mustermann"), "AM")
        XCTAssertEqual(SidebarDensity.initials(of: "Dr. Erika Mustermann"), "EM")
        XCTAssertEqual(SidebarDensity.initials(of: "Mustermann, Erika"), "EM")
        XCTAssertEqual(SidebarDensity.initials(of: "Erika und Max Mustermann"), "EM")
        XCTAssertEqual(SidebarDensity.initials(of: "Muster GmbH & Co. KG"), "MK")
        XCTAssertEqual(SidebarDensity.initials(of: "Erika"), "E")
        XCTAssertEqual(SidebarDensity.initials(of: ""), "")
        XCTAssertEqual(SidebarDensity.initials(of: "Erika Mustermann, Max Mustermann"), "EM")
        XCTAssertEqual(SidebarDensity.initials(of: "Muster GmbH, Berlin"), "MB")
    }
}
