import XCTest
@testable import GiroCrabUI

/// Sync feedback when nothing was written (#1703).
final class CloudSyncFeedbackTests: XCTestCase {
    func testEmptyVaultWithoutMergeIsReportedAsNotSaved() {
        let note = CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: false, merged: false)
        XCTAssertNotNil(note)
        XCTAssertFalse(note?.contains("aktualisiert") ?? true)
    }

    func testNoNoteWhenWritten() {
        XCTAssertNil(CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: true, merged: false))
        XCTAssertNil(CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: true, merged: true))
    }

    func testNoNoteWhenSomethingWasMerged() {
        XCTAssertNil(CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: false, merged: true))
    }

    func testNoNoteForMergeOnlyRun() {
        XCTAssertNil(CloudSyncManager.skippedWriteNote(mergeOnly: true, wrote: false, merged: false))
    }

    // MARK: automatic poll (`perform`)

    private let written = CloudSyncManager.writtenText(.folder)

    func testPollWithoutWriteOrMergeKeepsNotSavedNote() {
        let note = CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: false, merged: false)
        XCTAssertEqual(CloudSyncManager.pollStats(previous: note, recorded: written, merged: false, wrote: false), note)
    }

    func testPollWithoutWriteOrMergeKeepsEarlierMergeText() {
        let earlier = "3 neue / 1 aktualisierte Buchungen, 0 Konten"
        XCTAssertEqual(CloudSyncManager.pollStats(previous: earlier, recorded: written, merged: false, wrote: false), earlier)
    }

    func testPollWithoutWriteOrMergeKeepsMissingStatus() {
        XCTAssertNil(CloudSyncManager.pollStats(previous: nil, recorded: written, merged: false, wrote: false))
    }

    func testPollThatWroteTakesRecordedText() {
        let note = CloudSyncManager.skippedWriteNote(mergeOnly: false, wrote: false, merged: false)
        XCTAssertEqual(CloudSyncManager.pollStats(previous: note, recorded: written, merged: false, wrote: true), written)
    }

    func testPollThatMergedTakesRecordedText() {
        let merged = "2 neue / 0 aktualisierte Buchungen, 1 Konten"
        XCTAssertEqual(CloudSyncManager.pollStats(previous: written, recorded: merged, merged: true, wrote: false), merged)
    }
}
