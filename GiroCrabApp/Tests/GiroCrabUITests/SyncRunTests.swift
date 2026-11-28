import XCTest
@testable import GiroCrabUI

/// State of the sync window (`SyncRun`, docs/ABRUF_UI.md) without any bank.
final class SyncRunTests: XCTestCase {
    private func run(_ n: Int) -> SyncRun {
        SyncRun(logins: (1...n).map { (Int64($0), "Bank \($0)") })
    }

    func testANewRunWaitsForEveryLogin() {
        let r = run(3)
        XCTAssertEqual(r.total, 3)
        XCTAssertEqual(r.remaining, 3)
        XCTAssertEqual(r.completed, 0)
        XCTAssertEqual(r.position, 1)
        XCTAssertFalse(r.isFinished)
        XCTAssertNil(r.running)
        XCTAssertTrue(r.entries.allSatisfy { $0.status == .waiting })
    }

    func testDuplicateLoginsAreListedOnce() {
        let r = SyncRun(logins: [(1, "A"), (2, "B"), (1, "A")])
        XCTAssertEqual(r.entries.map(\.connectionID), [1, 2])
    }

    func testProgressCountsEveryEndAndFinishesWhenNothingIsOpen() {
        var r = run(4)
        r.start(1)
        XCTAssertEqual(r.running?.connectionID, 1)
        XCTAssertEqual(r.remaining, 4, "the running login is still to come")
        r.finish(1, imported: 12, detail: nil)
        r.start(2)
        r.fail(2, message: "Zeitüberschreitung")
        r.skip(3, reason: "Von Ihnen übersprungen")
        XCTAssertEqual(r.completed, 3)
        XCTAssertEqual(r.position, 4)
        XCTAssertFalse(r.isFinished)
        r.start(4)
        r.finish(4, imported: nil, detail: "Im Browser-Fenster beendet")
        XCTAssertTrue(r.isFinished)
        XCTAssertEqual(r.position, 4, "never past the last login")
        XCTAssertEqual(r.doneCount, 2)
        XCTAssertEqual(r.failedCount, 1)
        XCTAssertEqual(r.skippedCount, 1)
    }

    func testNoEndOverwritesAnotherEnd() {
        var r = run(3)
        r.finish(1, imported: 3, detail: nil)
        r.fail(2, message: "x")
        r.skip(3, reason: "z")
        // a late skip, failure or result of an ended login changes nothing
        r.skip(1, reason: "y")
        r.fail(1, message: "late")
        r.skip(2, reason: "y")
        r.finish(2, imported: 1, detail: nil)
        r.finish(3, imported: 1, detail: nil)
        r.fail(3, message: "late")
        XCTAssertEqual(r.status(of: 1), .done(imported: 3, detail: nil))
        XCTAssertEqual(r.status(of: 2), .failed(message: "x"))
        XCTAssertEqual(r.status(of: 3), .skipped(reason: "z"))
    }

    func testAFetchedLoginIsNotRetried() {
        var r = run(2)
        r.finish(1, imported: 3, detail: nil)
        r.skip(2, reason: "y")
        XCTAssertFalse(r.retry(1))
        XCTAssertEqual(r.status(of: 1), .done(imported: 3, detail: nil))
        XCTAssertTrue(r.isFinished)
    }

    func testSkipRemainingLeavesFinishedLoginsAlone() {
        var r = run(3)
        r.finish(1, imported: 0, detail: nil)
        r.start(2)
        r.skipRemaining(reason: "Abruf abgebrochen")
        XCTAssertEqual(r.status(of: 1), .done(imported: 0, detail: nil))
        XCTAssertEqual(r.status(of: 2), .skipped(reason: "Abruf abgebrochen"))
        XCTAssertEqual(r.status(of: 3), .skipped(reason: "Abruf abgebrochen"))
        XCTAssertTrue(r.isFinished)
    }

    func testRetryReopensOnlyEndedLogins() {
        var r = run(3)
        r.fail(1, message: "x")
        r.skip(2, reason: "y")
        r.start(3)
        XCTAssertTrue(r.retry(1))
        XCTAssertTrue(r.retry(2))
        XCTAssertFalse(r.retry(3), "a running login is not retried")
        XCTAssertFalse(r.retry(99), "not in this run")
        XCTAssertEqual(r.status(of: 1), .waiting)
        XCTAssertEqual(r.status(of: 2), .waiting)
        XCTAssertEqual(r.remaining, 3)
    }

    func testUnknownLoginsAreIgnored() {
        var r = run(1)
        r.start(42)
        r.finish(42, imported: 1, detail: nil)
        XCTAssertFalse(r.contains(42))
        XCTAssertEqual(r.status(of: 1), .waiting)
    }
}
