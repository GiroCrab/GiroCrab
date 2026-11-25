import XCTest
@testable import GiroCrabUI

/// Autofill of the stored portal login (#1500): a restart may catch the vault while
/// it is still opening — that must be retried, not mistaken for "nothing stored",
/// or the login form stays empty for the whole session.
@MainActor
final class PortalStoredLoginTests: XCTestCase {
    private func step(autofillDisabled: Bool = false, vaultAnswered: Bool = true,
                      storedLogin: String = "", password: String = "",
                      connectionLogin: String = "", amazonVisa: Bool = false,
                      tryCount: Int = 0) -> PortalWebSheet.StoredLoginStep {
        PortalWebSheet.storedLoginStep(autofillDisabled: autofillDisabled, vaultAnswered: vaultAnswered,
                                       storedUser: storedLogin, password: password,
                                       connectionLogin: connectionLogin, connectionIsAmazonVisa: amazonVisa,
                                       tryCount: tryCount)
    }

    func testVaultThatHasNotAnsweredYetIsRetried() {
        // Restart + first poll while the vault is still opening – even with
        // credentials stored: only an answered vault decides "nothing stored".
        if case .retry = step(vaultAnswered: false) { } else { XCTFail("soll wiederholen") }
        if case .retry = step(vaultAnswered: false, storedLogin: "max@example.com", password: "x") { }
        else { XCTFail("gespeicherte Daten bleiben beim ersten Poll unversucht") }
    }

    func testNothingStoredStopsThePoll() {
        if case .done = step() { } else { XCTFail("nichts gespeichert: fertig") }
        if case .done = step(autofillDisabled: true, storedLogin: "max@example.com", password: "x") { }
        else { XCTFail("Autofilling aus: fertig") }
    }

    func testAVaultThatStaysClosedStopsThePollAfterAWhile() {
        // a real read error (or a vault that never opens) must not poll and log
        // forever: after 20 tries the autofill is dropped
        if case .retry = step(vaultAnswered: false, tryCount: 19) { }
        else { XCTFail("unter der Grenze wird noch gelesen") }
        if case .done = step(vaultAnswered: false, tryCount: 20) { }
        else { XCTFail("bei dem Grenzwert endet das Poll") }
        if case .done = step(autofillDisabled: true, vaultAnswered: false, tryCount: 19) { }
        else { XCTFail("Autofilling aus: ohne Versuche fertig") }
    }

    func testAmazonVisaUsesTheConnectionEmailAsUser() {
        if case let .fill(user, pw) = step(password: "code", connectionLogin: "max@example.com", amazonVisa: true) {
            XCTAssertEqual(user, "max@example.com")
            XCTAssertEqual(pw, "code")
        } else { XCTFail("E-Mail der Verbindung wird als Benutzername benutzt") }
        if case .done = step(password: "code", connectionLogin: "max@example.com") { }
        else { XCTFail("ohne Fallback-Regel: Amex-Login ohne gespeicherten Namen bleibt aus") }
    }

    func testAmazonVisaWithoutStoredPasswordFillsNothing() {
        // Opting out clears the stored values, but not the autofill flag: without a
        // stored password the connection email must not be filled in either
        if case .done = step(connectionLogin: "max@example.com", amazonVisa: true) { }
        else { XCTFail("ohne gespeichertes Passwort wird auch die E-Mail nicht eingesetzt") }
    }

    func testStoredUserWithoutPasswordStillFillsTheName() {
        if case let .fill(user, pw) = step(storedLogin: "max@example.com") {
            XCTAssertEqual(user, "max@example.com")
            XCTAssertEqual(pw, "")
        } else { XCTFail("auch der gespeicherte Name wird eingetragen") }
    }
}
