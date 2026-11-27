import XCTest
import GiroCrabCore
@testable import GiroCrabUI

#if os(macOS)
/// The database import's account step: an account whose IBAN fits several accounts waits
/// for the user's choice instead of becoming a new account (#1993).
final class MoneyMoneyDbChoiceTests: XCTestCase {
    private typealias Choice = MoneyMoneyDatabaseImportView.Choice

    private func account(existing: Int64?, needsChoice: Bool) -> MoneyMoneyDbAccount {
        MoneyMoneyDbAccount(mmId: 1, name: "Wallet", iban: "DE89370400440532013000", accountNumber: "", currency: "EUR",
                            bankCode: "", bankName: "", owner: "", groupName: "", mmBankId: nil, isPortfolio: false,
                            suggestedKind: .checking, existingAccountId: existing, existingAccountName: nil,
                            targetNeedsChoice: needsChoice, bookingCount: 4, pendingCount: 0, holdingCount: 0,
                            notesCount: 0, firstDate: nil, lastDate: nil, statementCount: 0, computedBalanceCents: 0,
                            balanceSource: "", closed: false, reusedAccountId: nil)
    }

    func testSuggestionTakesTheTargetOrWaitsForAChoice() {
        XCTAssertEqual(Choice.suggested(account(existing: 7, needsChoice: false)),
                       Choice(selected: true, kind: .checking, target: 7))
        let open = Choice.suggested(account(existing: nil, needsChoice: true))
        XCTAssertTrue(open.undecided)
        XCTAssertNil(open.target)
    }

    func testAnUndecidedAccountBlocksTheStepUntilChosenOrLeftOut() {
        let ready = Choice(selected: true, kind: .checking, target: 7)
        var open = Choice.suggested(account(existing: nil, needsChoice: true))
        XCTAssertFalse(Choice.ready([ready, open]))
        open.selected = false
        XCTAssertTrue(Choice.ready([ready, open]), "left out of the import")
        open.selected = true
        open.undecided = false // "Neues Konto" chosen
        XCTAssertTrue(Choice.ready([ready, open]))
        XCTAssertFalse(Choice.ready([Choice]()))
    }
}
#endif
