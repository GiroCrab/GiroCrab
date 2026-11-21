import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Bookings dragged into another app (`de.girocrab.transactions`, docs/booking-drag.md).
@MainActor
final class BookingDragExportTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws { model = nil }

    func testJSONUsesLocalAPIFieldNamesAndRoundTrips() throws {
        let bookings = Array(model.bookings.prefix(2))
        XCTAssertEqual(bookings.count, 2)
        let export = BookingDragExport(bookings: bookings, accounts: model.accountsByID, creator: "GiroCrab test")
        let data = try export.json()
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["version"] as? Int, 1)
        XCTAssertEqual(object["creator"] as? String, "GiroCrab test")
        let first = try XCTUnwrap((object["transactions"] as? [[String: Any]])?.first)
        XCTAssertEqual(first["id"] as? Int64, bookings[0].id)
        XCTAssertEqual(first["amount_cents"] as? Int64, bookings[0].amountCents)
        XCTAssertEqual(first["booking_date"] as? String, bookings[0].bookingDate)
        XCTAssertNotNil(first["account_id"])
        XCTAssertNil(first["amountCents"], "snake_case like the local API")
        XCTAssertEqual(try BookingDragExport.decode(data), export)
    }

    func testEmptyTextsAreLeftOutAndValueDateFallsBack() throws {
        var b = try XCTUnwrap(model.bookings.first)
        b.note = "  "
        b.counterpartyIban = ""
        b.valueDate = ""
        let t = try XCTUnwrap(BookingDragExport(bookings: [b], accounts: [:]).transactions.first)
        XCTAssertNil(t.note)
        XCTAssertNil(t.counterpartyIban)
        XCTAssertNil(t.accountIban)
        XCTAssertEqual(t.valueDate, b.bookingDate)
        let json = try XCTUnwrap(String(data: BookingDragExport(bookings: [b], accounts: [:]).json(), encoding: .utf8))
        XCTAssertFalse(json.contains("\"note\""))
    }

    #if os(macOS)
    func testDragItemKeepsTheSidebarPayload() throws {
        let bookings = Array(model.bookings.prefix(3))
        let item = BookingDragItem(bookings: bookings, accounts: model.accountsByID)
        XCTAssertEqual(BookingDrag.ids(BookingDrag.payload(item.ids)), bookings.map(\.id))
        let json = try XCTUnwrap(item.json)
        XCTAssertEqual(try BookingDragExport.decode(json).transactions.map(\.id), bookings.map(\.id))
    }
    #endif
}
