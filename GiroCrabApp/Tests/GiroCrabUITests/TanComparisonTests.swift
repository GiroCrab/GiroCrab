import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// TAN-Abgleich-Hinweis (#949): Betrag und IBAN-Ende beim Freigeben einer Einzelüberweisung.
@MainActor
final class TanComparisonTests: XCTestCase {
    func testGroupsIBANAndTakesLastFourCharacters() throws {
        let c = try XCTUnwrap(TanComparison(iban: "de89370400440532013000", amountCents: 12_345))
        XCTAssertEqual(c.iban, "DE89 3704 0044 0532 0130 00")
        XCTAssertEqual(c.endDigits, "3000")
        XCTAssertEqual(c.amount, Money.format(12_345))
    }

    func testPrefixLeavesOutLastFourCharactersAcrossGroupBoundary() throws {
        // 22 characters: the last group has only two, so the end spans two groups
        let c = try XCTUnwrap(TanComparison(iban: "DE89370400440532013000", amountCents: 100))
        XCTAssertEqual(c.ibanPrefix, "DE89 3704 0044 0532 01")
        XCTAssertEqual(c.ibanPrefix.replacingOccurrences(of: " ", with: "") + c.endDigits, "DE89370400440532013000")
    }

    func testRejectsIBANThatIsTooShort() {
        XCTAssertNil(TanComparison(iban: "DE8", amountCents: 100))
        XCTAssertNil(TanComparison(iban: "", amountCents: 100))
    }

    func testModelOffersNoComparisonWithoutRunningTransfer() {
        let model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        model.transferDraft.iban = "DE89370400440532013000"
        model.transferDraft.amountText = "12,34"
        XCTAssertNil(model.tanComparison)
    }
}
