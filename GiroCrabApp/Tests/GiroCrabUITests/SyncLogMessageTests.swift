import XCTest
import GiroCrabCore
@testable import GiroCrabUI

@MainActor
final class SyncLogMessageTests: XCTestCase {
    private func summary(imported: UInt32 = 0, documents: UInt32 = 0, securities: UInt32 = 0, holdings: UInt32 = 0, incomplete: Bool = false,
                         remaining: UInt32 = 0, total: UInt32 = 0, documentsOff: Bool = false) -> SyncSummary {
        SyncSummary(accountsUpdated: 1, transactionsImported: imported, transactionsCategorized: 0,
                    transactionsSkippedArchived: 0, newAccounts: [], revokedAccounts: [], postboxDocuments: nil,
                    documentsImported: documents, securityTransactions: securities, holdings: holdings,
                    historyIncomplete: incomplete, historyRemaining: remaining, historyTotal: total, documentsOff: documentsOff)
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
}
