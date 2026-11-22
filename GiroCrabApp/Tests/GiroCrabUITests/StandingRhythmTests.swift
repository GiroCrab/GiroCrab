import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The rhythm and number entry of the standing order form (#1475).
final class StandingRhythmTests: XCTestCase {

    func testTypedNumberMovesToTheNearestValueTheBankAllows() {
        let allowed = [1, 2, 3, 6, 12]
        XCTAssertEqual(NumberEntryRow.nearest(4, range: 1...99, allowed: allowed), 3)
        XCTAssertEqual(NumberEntryRow.nearest(5, range: 1...99, allowed: allowed), 6)
        XCTAssertEqual(NumberEntryRow.nearest(60, range: 1...99, allowed: allowed), 12)
        XCTAssertEqual(NumberEntryRow.nearest(6, range: 1...99, allowed: allowed), 6)
        // no limit from the bank: only the range counts
        XCTAssertEqual(NumberEntryRow.nearest(0, range: 1...99, allowed: []), 1)
        XCTAssertEqual(NumberEntryRow.nearest(150, range: 1...99, allowed: []), 99)
        XCTAssertEqual(NumberEntryRow.nearest(31, range: 1...31, allowed: []), 31)
    }

    func testStepGoesByOneOrToTheNextAllowedValueAndStopsAtTheEnds() {
        XCTAssertEqual(NumberEntryRow.step(5, by: 1, range: 1...12, allowed: []), 6)
        XCTAssertEqual(NumberEntryRow.step(5, by: -1, range: 1...12, allowed: []), 4)
        XCTAssertEqual(NumberEntryRow.step(12, by: 1, range: 1...12, allowed: []), 12)
        XCTAssertEqual(NumberEntryRow.step(1, by: -1, range: 1...12, allowed: []), 1)
        let allowed = [1, 2, 3, 6, 12]
        XCTAssertEqual(NumberEntryRow.step(3, by: 1, range: 1...12, allowed: allowed), 6)
        XCTAssertEqual(NumberEntryRow.step(6, by: -1, range: 1...12, allowed: allowed), 3)
        XCTAssertEqual(NumberEntryRow.step(12, by: 1, range: 1...12, allowed: allowed), 12)
        XCTAssertEqual(NumberEntryRow.step(1, by: -1, range: 1...12, allowed: allowed), 1)
    }

    /// A typed number is stepped from as typed: 5 up with 1, 2, 3, 6, 12 gives 6, not 12.
    func testStepFromATypedNumberDoesNotSkipAValue() {
        XCTAssertEqual(NumberEntryRow.step(5, by: 1, range: 1...12, allowed: [1, 2, 3, 6, 12]), 6)
        XCTAssertEqual(NumberEntryRow.step(5, by: -1, range: 1...12, allowed: [1, 2, 3, 6, 12]), 3)
    }

    /// A value the bank holds above the range (a day 99) is not moved to 31 by a press.
    func testStepStartKeepsAValueOutsideTheRange() {
        XCTAssertEqual(NumberEntryRow.stepStart(typed: 5, value: 1, range: 1...31), 5)
        XCTAssertEqual(NumberEntryRow.stepStart(typed: nil, value: 99, range: 1...31), 99)
        XCTAssertEqual(NumberEntryRow.stepStart(typed: 99, value: 20, range: 1...31), 20)
        XCTAssertEqual(NumberEntryRow.step(99, by: -1, range: 1...99, allowed: [99]), 99)
    }

    /// The rhythm list: the usual ones without limits, what the bank reports with limits.
    func testRhythmChoicesFollowTheBank() {
        let d = StandingDraft()
        XCTAssertEqual(d.rhythmChoices(nil).count, 6)
        let limits = StandingOrderLimits(
            minDaysAhead: 0, maxDaysAhead: 0, months: [1, 3, 12], weeks: [1, 2], daysOfMonth: [], weekdays: [])
        XCTAssertEqual(d.rhythmChoices(limits).map { "\($0.unit)\($0.interval)" }, ["W1", "W2", "M1", "M3", "M12"])
        // a rhythm outside the usual ones that the bank reports can be picked (nothing of the usual ones matches)
        let odd = StandingOrderLimits(
            minDaysAhead: 0, maxDaysAhead: 0, months: [4], weeks: [2], daysOfMonth: [], weekdays: [])
        let choices = d.rhythmChoices(odd)
        XCTAssertTrue(choices.contains(StandingRhythm(unit: "W", interval: 2)))
        XCTAssertTrue(choices.contains(StandingRhythm(unit: "M", interval: 4)))
    }

    /// A schedule the order holds stays offered, also after another one was chosen and the draft changed.
    func testHeldRhythmStaysOfferedAfterAnotherChoice() {
        let held = StandingRhythm(unit: "W", interval: 4)
        var d = StandingDraft()
        d.unit = "W"
        d.interval = 4
        XCTAssertTrue(d.rhythmChoices(nil, keeping: held).contains(held))
        d.unit = "M"
        d.interval = 1
        XCTAssertTrue(d.rhythmChoices(nil, keeping: held).contains(held))
        XCTAssertFalse(d.rhythmChoices(nil).contains(held))
        XCTAssertEqual(d.rhythmText(of: StandingRhythm(unit: "M", interval: 3)), "vierteljährlich")
    }
}
