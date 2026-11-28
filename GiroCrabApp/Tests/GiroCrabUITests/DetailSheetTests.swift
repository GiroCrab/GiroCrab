// test: serial-defaults
import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// iOS detail sheet (PR "Buchungsliste als Hauptfläche"): it belongs to the list it was opened
/// from and gives way to an editor opened from it.
@MainActor
final class DetailSheetTests: XCTestCase {
    private func demoModel() -> AppModel {
        AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
    }

    func testDetailSheetStaysOnlyOnItsBookingList() {
        XCTAssertTrue(AppModel.keepsDetailSheet(from: .bookings, to: .bookings, presentation: .bookings))
        XCTAssertTrue(AppModel.keepsDetailSheet(from: .planned, to: .planned, presentation: .bookings))
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .bookings, to: .planned, presentation: .bookings))
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .planned, to: .documents, presentation: .bookings))
        // A depot or Sachwerte account keeps the column at `.bookings`.
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .bookings, to: .bookings, presentation: .securities(1)))
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .bookings, to: .bookings, presentation: .tangible(1)))
    }

    func testSubscriptionDetailSheetStaysOnlyOnTheSubscriptionsList() {
        XCTAssertTrue(AppModel.keepsDetailSheet(from: .subscriptions, to: .subscriptions, presentation: .bookings))
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .subscriptions, to: .planned, presentation: .bookings))
        XCTAssertFalse(AppModel.keepsDetailSheet(from: .bookings, to: .subscriptions, presentation: .bookings))
    }

    func testStandingOrderEditorClosesDetailSheet() {
        let model = demoModel()
        model.showBookingSheet = true
        model.openStandingDemo(model.standingDraft)
        XCTAssertFalse(model.showBookingSheet)
        XCTAssertTrue(model.showStandingSheet)
    }

    func testMandateEditorClosesDetailSheetAndKeepsSelection() throws {
        let model = demoModel()
        guard let mandate = model.mandates.first else { throw XCTSkip("demo vault without mandates") }
        model.selectedMandateID = mandate.id
        model.showBookingSheet = true
        model.editMandate(mandate)
        XCTAssertFalse(model.showBookingSheet)
        XCTAssertNotNil(model.mandateDraft)
        XCTAssertEqual(model.selectedMandateID, mandate.id)
    }
}
