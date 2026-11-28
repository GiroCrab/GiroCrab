import XCTest
@testable import GiroCrabUI

/// Account rows of the sidebar: spacious (bank below the name) or compact (one line, #884).
final class SidebarDensityTests: XCTestCase {
    func testSpaciousIsTheDefault() {
        XCTAssertEqual(SidebarDensity.defaultValue, .spacious)
        XCTAssertEqual(SidebarDensity.defaultsKey, "sidebarDensity")
    }

    func testStoredValuesStayReadable() {
        XCTAssertEqual(SidebarDensity(rawValue: "spacious"), .spacious)
        XCTAssertEqual(SidebarDensity(rawValue: "compact"), .compact)
    }

    func testCompactDropsOnlyThePlainBankLine() {
        XCTAssertTrue(SidebarDensity.spacious.showsSubtitle("Musterbank", bank: "Musterbank"))
        XCTAssertFalse(SidebarDensity.compact.showsSubtitle("Musterbank", bank: "Musterbank"))
        // namesake distinction, owner / IBAN hit while searching: still shown
        XCTAssertTrue(SidebarDensity.compact.showsSubtitle("Musterbank · Erika", bank: "Musterbank"))
        XCTAssertTrue(SidebarDensity.compact.showsSubtitle("Erika Mustermann", bank: "Musterbank"))
        XCTAssertFalse(SidebarDensity.compact.showsSubtitle("", bank: ""))
        // a namesake without owner or number: the bank is all that tells the accounts apart
        XCTAssertTrue(SidebarDensity.compact.showsSubtitle("Musterbank", bank: "Musterbank", hasNamesake: true))
        XCTAssertFalse(SidebarDensity.spacious.showsSubtitle("", bank: "Musterbank"))
    }

    func testCompactHasNoRepaymentOrArchiveLine() {
        XCTAssertTrue(SidebarDensity.spacious.showsStatusLine)
        XCTAssertFalse(SidebarDensity.compact.showsStatusLine)
    }

    func testInitialsOnlyWithSeveralOwners() {
        XCTAssertFalse(SidebarDensity.hasSeveralOwners([]))
        XCTAssertFalse(SidebarDensity.hasSeveralOwners(["Erika Mustermann", "erika mustermann ", ""]))
        XCTAssertTrue(SidebarDensity.hasSeveralOwners(["Erika Mustermann", "Anna Maria Mustermann"]))
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
