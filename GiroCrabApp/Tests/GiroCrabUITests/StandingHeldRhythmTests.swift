import XCTest
@testable import GiroCrabUI

/// A rhythm the bank holds for an order (e.g. every 4 weeks) stays available for the whole editing session.
final class StandingHeldRhythmTests: XCTestCase {
    private let w4 = StandingRhythm(unit: "W", interval: 4)
    private let m1 = StandingRhythm(unit: "M", interval: 1)

    private func heldW4Draft() -> StandingDraft {
        var d = StandingDraft()
        d.action = .change
        d.unit = "W"
        d.interval = 4
        d.heldRhythm = d.rhythm
        return d
    }

    func testHeldRhythmStaysOfferedAfterAnotherRhythmWasPicked() {
        var d = heldW4Draft()
        XCTAssertTrue(d.rhythmChoices(nil).contains(w4))
        d.unit = m1.unit
        d.interval = m1.interval
        let choices = d.rhythmChoices(nil)
        XCTAssertTrue(choices.contains(w4), "the held rhythm can be picked again")
        XCTAssertTrue(choices.contains(m1))
        XCTAssertEqual(choices.filter { $0 == w4 }.count, 1)
    }

    func testNewOrderOffersNoHeldRhythm() {
        var d = StandingDraft()
        d.unit = "M"
        d.interval = 1
        XCTAssertNil(d.heldRhythm)
        XCTAssertFalse(d.rhythmChoices(nil).contains(w4))
    }

    func testPickingTheHeldRhythmAgainKeepsItsInterval() {
        var d = heldW4Draft()
        d.unit = "M"
        d.interval = 1
        // picked again: the bank reports only weekly, but W4 is what it holds
        d.unit = "W"
        d.interval = 4
        XCTAssertTrue(d.keepsInterval(allowed: [1]))
    }

    func testAnUnheldIntervalTheBankDoesNotListIsStillConformed() {
        var d = heldW4Draft()
        d.interval = 2  // W2: neither reported nor held
        XCTAssertFalse(d.keepsInterval(allowed: [1]))
        var fresh = StandingDraft()
        fresh.unit = "W"
        fresh.interval = 4
        XCTAssertFalse(fresh.keepsInterval(allowed: [1]), "without a held order W4 moves to what the bank lists")
        XCTAssertTrue(fresh.keepsInterval(allowed: []), "no limits: any interval stays")
    }
}
