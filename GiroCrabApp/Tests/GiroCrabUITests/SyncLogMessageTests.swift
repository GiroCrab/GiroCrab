import XCTest
import GiroCrabCore
@testable import GiroCrabUI

@MainActor
final class SyncLogMessageTests: XCTestCase {
    private func summary(imported: UInt32 = 0, documents: UInt32 = 0, securities: UInt32 = 0, holdings: UInt32 = 0, incomplete: Bool = false,
                         remaining: UInt32 = 0, total: UInt32 = 0, documentsOff: Bool = false,
                         warnings: [String] = [], hints: [String] = []) -> SyncSummary {
        SyncSummary(accountsUpdated: 1, transactionsImported: imported, transactionsCategorized: 0,
                    transactionsSkippedArchived: 0, newAccounts: [], revokedAccounts: [], postboxDocuments: nil, warnings: warnings, hints: hints,
                    documentsImported: documents, securityTransactions: securities, holdings: holdings,
                    historyIncomplete: incomplete, historyRemaining: remaining, historyTotal: total, documentsOff: documentsOff)
    }

    func testWarningsAndHintsFollowTheCounts() {
        XCTAssertEqual(AppModel.syncLogMessage(summary(warnings: ["W"], hints: ["H"])), "Keine neuen Buchungen · W · H")
    }

    func testNothingDeliveredKeepsTheShortText() {
        XCTAssertEqual(AppModel.syncLogMessage(summary()), "Keine neuen Buchungen")
    }

    func testEverythingIsNamedInOrderAndZerosStayOut() {
        let text = AppModel.syncLogMessage(summary(imported: 3, documents: 2, securities: 414, holdings: 32))
        XCTAssertEqual(text, "3 neue Buchungen · 2 neue Dokumente · 414 Wertpapiergeschäfte · 32 Positionen")
        XCTAssertEqual(AppModel.syncLogMessage(summary(holdings: 32)), "Keine neuen Buchungen · 32 Positionen")
    }

    func testAnIncompleteHistoryIsSaidInTheLogAndTheNotice() {
        XCTAssertEqual(AppModel.syncLogMessage(summary(holdings: 32, incomplete: true)),
                       "Keine neuen Buchungen · 32 Positionen · Historie noch nicht vollständig")
        XCTAssertTrue(AppModel.historyNote(summary(incomplete: true)).contains("Historie"))
        XCTAssertEqual(AppModel.historyNote(summary()), "")
    }

    func testTheProgressNamesWhatIsLeft() {
        let p = HistoryProgress(remaining: 600, total: 2_400)
        XCTAssertEqual(p.fraction, 0.75, accuracy: 0.0001)
        XCTAssertTrue(p.text.contains("600") && p.text.contains("2"))
        XCTAssertEqual(HistoryProgress(remaining: 5, total: 0).text, "")
        XCTAssertEqual(HistoryProgress(remaining: 5, total: 0).fraction, 0)
        let note = AppModel.historyNote(summary(incomplete: true, remaining: 600, total: 2_400))
        XCTAssertTrue(note.contains("600"))
    }

    func testASwitchedOffDocumentRetrievalShowsInTheLog() {
        XCTAssertEqual(AppModel.syncLogMessage(summary(documentsOff: true)), "Keine neuen Buchungen · Dokumentenabruf aus")
        XCTAssertEqual(AppModel.syncLogMessage(summary(imported: 3, documentsOff: true)), "3 neue Buchungen · Dokumentenabruf aus")
    }

    /// Review of #1660: connection ids belong to one vault, so the progress of a history does not
    /// show in the demo (or the other way round) after a vault swap.
    func testHistoryProgressDoesNotCarryOverToAnotherVault() {
        let model = AppModel(options: LaunchOptions(arguments: ["--fresh-vault"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        model.historyIncomplete[1] = HistoryProgress(remaining: 5, total: 10)
        model.showDemo()
        XCTAssertTrue(model.demoActive)
        XCTAssertTrue(model.historyIncomplete.isEmpty)
    }
}
