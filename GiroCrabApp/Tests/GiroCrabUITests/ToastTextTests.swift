import XCTest
@testable import GiroCrabUI

/// The toast overlay must not show account or merchant names in a bug report
/// capture or in privacy mode (#1967 review).
final class ToastTextTests: XCTestCase {
    private let toast = "Kein passender Umsatz gefunden – beim Konto „Girokonto Muster“ abgelegt"

    func testUnredactedToastKeepsItsText() {
        XCTAssertEqual(ToastText.shown(toast, redaction: []), toast)
    }

    func testPersonalRedactionHidesTheAccountName() {
        let shown = ToastText.shown(toast, redaction: [.personal])
        XCTAssertFalse(shown.contains("Girokonto Muster"))
        XCTAssertEqual(shown, String(localized: "Kein Klartext – Hinweis ausgeblendet"))
    }

    func testLabelRedactionHidesTheToastLikeTheWindowTitle() {
        let shown = ToastText.shown(toast, redaction: [.labels])
        XCTAssertFalse(shown.contains("Girokonto Muster"))
    }

    func testAmountsOnlyRedactionKeepsTheToast() {
        XCTAssertEqual(ToastText.shown(toast, redaction: [.amounts]), toast)
    }
}
