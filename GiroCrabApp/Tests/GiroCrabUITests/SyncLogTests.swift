import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Abrufprotokoll (#238): the model writes rows the log window reads.
@MainActor
final class SyncLogTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws { model = nil }

    private func rows(_ filter: SyncLogFilter = SyncLogFilter(connectionId: nil, onlyErrors: false, search: nil, limit: 0)) throws -> [SyncLogEntry] {
        try XCTUnwrap(model.vault).listSyncLog(filter: filter)
    }

    func testDemoVaultHasLogRowsWithErrors() throws {
        XCTAssertFalse(try rows().isEmpty)
        XCTAssertTrue(try rows().contains { $0.outcome == .error })
    }

    func testLogSyncWritesScrubbedRowAndBumpsVersion() throws {
        let conn = try XCTUnwrap(model.connections.first)
        let before = model.syncLogVersion
        model.logSync(connectionID: conn.id, source: .manual, outcome: .error,
                      message: "Anmeldung fehlgeschlagen, PIN=123456 für \(conn.loginName)")
        XCTAssertEqual(model.syncLogVersion, before + 1)
        let newest = try XCTUnwrap(rows().first)
        XCTAssertEqual(newest.outcome, .error)
        XCTAssertEqual(newest.connectionName, conn.displayName)
        XCTAssertFalse(newest.message.contains("123456"))
        XCTAssertFalse(newest.message.contains(conn.loginName))
    }

    func testResortingDocumentsReportsTheResultInTheSyncLog() async throws {
        let id = try XCTUnwrap(model.connections.first { $0.connector == .tradeRepublic }).id
        let before = model.syncLogVersion

        await model.resortPortalDocuments(id)

        XCTAssertEqual(model.syncLogVersion, before + 1)
        let filter = SyncLogFilter(connectionId: id, onlyErrors: false, search: nil, limit: 1)
        let entry = try XCTUnwrap(rows(filter).first)
        XCTAssertEqual(entry.outcome, .ok)
        XCTAssertEqual(entry.source, .manual)
        XCTAssertEqual(model.toast, entry.message)
    }

    func testOnlyErrorsFilter() throws {
        let errors = try rows(SyncLogFilter(connectionId: nil, onlyErrors: true, search: nil, limit: 0))
        XCTAssertFalse(errors.isEmpty)
        XCTAssertTrue(errors.allSatisfy { $0.outcome == .error || $0.outcome == .partial })
    }

    /// #1627: a sync that stored balances but could not read the bookings is no success.
    func testFinishedSyncWithWarningsIsLoggedAsPartial() throws {
        let conn = try XCTUnwrap(model.connections.first)
        func finished(_ warnings: [String], hints: [String] = []) -> SyncStep {
            .finished(summary: SyncSummary(accountsUpdated: 1, transactionsImported: 0, transactionsCategorized: 0,
                                           transactionsSkippedArchived: 0, newAccounts: [], revokedAccounts: [],
                                           postboxDocuments: nil, warnings: warnings, hints: hints))
        }
        let filter = SyncLogFilter(connectionId: conn.id, onlyErrors: false, search: nil, limit: 1)

        model.handle(finished(["Girokonto: Umsätze (HKCAZ/camt.052) – TAN verlangt", "Visa: Kartenumsätze – TAN verlangt"],
                              hints: ["Zeitraum auf 90 Tage stellen"]),
                     connection: conn)
        let partial = try XCTUnwrap(rows(filter).first)
        XCTAssertEqual(partial.outcome, .partial)
        XCTAssertTrue(partial.message.contains("Girokonto"), partial.message)
        XCTAssertTrue(partial.message.contains(" / "), partial.message)
        // the log row says what to do about it, too
        XCTAssertTrue(partial.message.hasSuffix("Zeitraum auf 90 Tage stellen"), partial.message)
        let problems = try rows(SyncLogFilter(connectionId: conn.id, onlyErrors: true, search: nil, limit: 1))
        XCTAssertEqual(problems.first?.id, partial.id)

        model.handle(finished([]), connection: conn)
        XCTAssertEqual(try XCTUnwrap(rows(filter).first).outcome, .ok)
    }

    func testPartialHasItsOwnTitleAndSymbol() {
        XCTAssertEqual(SyncLogOutcome.partial.title, "Teilweise")
        // shape, not colour alone, tells the results apart
        let symbols = [SyncLogOutcome.ok, .partial, .error, .cancelled].map(\.symbol)
        XCTAssertEqual(Set(symbols).count, symbols.count)
    }

    func testCSVHasHeaderAndOneLinePerEntry() throws {
        let all = try rows()
        let csv = AppModel.syncLogCSV(all)
        XCTAssertEqual(csv.split(separator: "\n").count, all.count + 1)
        XCTAssertTrue(csv.hasPrefix("Zeit;Konto;"))
    }
}
