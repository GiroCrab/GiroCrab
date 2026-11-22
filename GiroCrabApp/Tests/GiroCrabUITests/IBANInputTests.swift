import XCTest
@testable import GiroCrabUI

/// The IBAN field of the transfer and standing order forms (#1475).
final class IBANInputTests: XCTestCase {

    func testIBANIsGroupedAtOnceOnlyWhenTypedAtTheEndOrChangedFromOutside() {
        // typing at the end: the grouping follows at once (the cursor is at the end anyway)
        XCTAssertTrue(IBANInput.regroupsAtOnce(old: "DE89 3704", new: "DE89 37045", focused: true))
        XCTAssertTrue(IBANInput.regroupsAtOnce(old: "DE89 37045", new: "DE89 3704", focused: true))
        // a correction in the middle keeps the text (and so the cursor) as typed
        XCTAssertFalse(IBANInput.regroupsAtOnce(old: "DE89 3704 0044", new: "DE89 3794 0044", focused: true))
        XCTAssertFalse(IBANInput.regroupsAtOnce(old: "DE89 3704 0044", new: "DE89 370 0044", focused: true))
        XCTAssertFalse(IBANInput.regroupsAtOnce(old: "DE89 3704 0044", new: "DE893704 0044", focused: true), "only a space removed")
        // a pick, a template or a paste into another field: grouped at once
        XCTAssertTrue(IBANInput.regroupsAtOnce(old: "DE89 3704 0044", new: "de12500105170648489890", focused: false))
    }
}
