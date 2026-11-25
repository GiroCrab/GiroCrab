import XCTest
@testable import GiroCrabUI

/// Visibility of the header analysis (GitHub #1760): „Auswertungen anzeigen“
/// hides the KPIs; the chart follows only „Diagramm anzeigen“. Folding the
/// chart keeps its controls so the chart can be shown again.
final class StatisticsVisibilityTests: XCTestCase {
    func testEverythingShowsWithAllSwitchesOn() {
        let elements = ViewLayout.headerAnalysis(statistics: true, chartCollapsed: false, compact: false)
        XCTAssertTrue(elements.chart)
        XCTAssertTrue(elements.controls)
        XCTAssertTrue(elements.kpis)
    }

    func testStatisticsOffHidesKpisButKeepsTheChart() {
        let elements = ViewLayout.headerAnalysis(statistics: false, chartCollapsed: false, compact: false)
        XCTAssertTrue(elements.chart)
        XCTAssertTrue(elements.controls)
        XCTAssertFalse(elements.kpis)
    }

    func testFoldingTheChartKeepsControlsAndKpis() {
        let elements = ViewLayout.headerAnalysis(statistics: true, chartCollapsed: true, compact: false)
        XCTAssertFalse(elements.chart)
        XCTAssertTrue(elements.controls)
        XCTAssertTrue(elements.kpis)
    }

    func testStatisticsAndChartFoldAreIndependent() {
        let elements = ViewLayout.headerAnalysis(statistics: false, chartCollapsed: true, compact: false)
        XCTAssertFalse(elements.chart)
        XCTAssertTrue(elements.controls)
        XCTAssertFalse(elements.kpis)
    }

    func testCompactHeaderHidesAnalysisRegardlessOfTheOtherSwitches() {
        for statistics in [false, true] {
            for collapsed in [false, true] {
                let elements = ViewLayout.headerAnalysis(statistics: statistics, chartCollapsed: collapsed, compact: true)
                XCTAssertFalse(elements.chart)
                XCTAssertFalse(elements.controls)
                XCTAssertFalse(elements.kpis)
            }
        }
    }
}
