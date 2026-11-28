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

    // MARK: Checked toasts (#1998)

    func testCheckedToastWithoutNamesStaysReadableInPrivacyMode() {
        XCTAssertEqual(ToastText.shown("Zuordnung aufgehoben", hiding: [], redaction: [.personal]), "Zuordnung aufgehoben")
    }

    func testPersonalRedactionMasksOnlyTheNamedAccount() {
        let shown = ToastText.shown(toast, hiding: ["Girokonto Muster"], redaction: [.personal])
        XCTAssertEqual(shown, "Kein passender Umsatz gefunden – beim Konto „•••“ abgelegt")
    }

    func testLabelRedactionMasksTheNamesToo() {
        let shown = ToastText.shown(toast, hiding: ["Girokonto Muster"], redaction: [.labels])
        XCTAssertFalse(shown.contains("Girokonto Muster"))
    }

    func testLongerNameIsMaskedWholeWhenItContainsAShorterOne() {
        let shown = ToastText.shown("2 Belege an „Muster GmbH“ angehängt", hiding: ["Muster", "Muster GmbH"], redaction: [.personal])
        XCTAssertEqual(shown, "2 Belege an „•••“ angehängt")
    }

    func testEmptyNameDoesNotChangeTheText() {
        XCTAssertEqual(ToastText.shown("Beleg zugeordnet: ", hiding: [""], redaction: [.personal]), "Beleg zugeordnet: ")
    }

    func testAmountsOnlyRedactionKeepsCheckedNames() {
        XCTAssertEqual(ToastText.shown(toast, hiding: ["Girokonto Muster"], redaction: [.amounts]), toast)
    }

    func testExportToastMasksOnlyTheTargetFolder() {
        let shown = ToastText.shown("3 Belege nach „Steuer 2025“ exportiert", hiding: ["Steuer 2025"], redaction: [.personal])
        XCTAssertEqual(shown, "3 Belege nach „•••“ exportiert")
    }

    func testPathCopiedToastStaysReadableInPrivacyMode() {
        XCTAssertEqual(ToastText.shown("Pfad kopiert", hiding: [], redaction: [.personal]), "Pfad kopiert")
    }
}
