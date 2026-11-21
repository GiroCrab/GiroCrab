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

    func testDocumentsGoToTheirBookingAsBase64() throws {
        let bookings = Array(model.bookings.prefix(2))
        var export = BookingDragExport(bookings: bookings, accounts: [:])
        let pdf = Data("%PDF-1.4 test".utf8)
        export.attach([bookings[1].id: [.init(fileName: "beleg.pdf", mimeType: "application/pdf", title: nil, data: pdf)]])
        XCTAssertNil(export.transactions[0].documents)
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: export.json()) as? [String: Any])
        let second = try XCTUnwrap((object["transactions"] as? [[String: Any]])?[1])
        let doc = try XCTUnwrap((second["documents"] as? [[String: Any]])?.first)
        XCTAssertEqual(doc["file_name"] as? String, "beleg.pdf")
        XCTAssertEqual(doc["mime_type"] as? String, "application/pdf")
        XCTAssertEqual(doc["data"] as? String, pdf.base64EncodedString())
    }

    func testDragDocumentsReadsTheVaultsDocuments() async throws {
        let booking = try XCTUnwrap(model.bookings.first)
        let pdf = Data("%PDF-1.4 drag".utf8)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("drag-\(UUID().uuidString).pdf")
        try pdf.write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(model.attach([file], to: booking))
        let docs = await model.dragDocuments(for: [booking.id], from: model.pinVault())
        XCTAssertEqual(docs[booking.id]?.first?.data, pdf)
        XCTAssertEqual(docs[booking.id]?.first?.fileName, file.lastPathComponent)
    }

    /// Booking ids are per vault: a drag that outlives its vault must not read another one's documents.
    func testDragDocumentsNeedTheVaultTheDragStartedIn() async throws {
        let booking = try XCTUnwrap(model.bookings.first)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("drag-\(UUID().uuidString).pdf")
        try Data("%PDF-1.4 drag".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(model.attach([file], to: booking))
        let pin = model.pinVault()
        // Another vault open (a second demo vault stands in for the switch): same ids, other data.
        let other = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        let fromOther = await other.dragDocuments(for: [booking.id], from: pin)
        XCTAssertTrue(fromOther.isEmpty)
        let withoutPin = await model.dragDocuments(for: [booking.id], from: nil)
        XCTAssertTrue(withoutPin.isEmpty)
        model.releaseVaultState()
        let afterLock = await model.dragDocuments(for: [booking.id], from: pin)
        XCTAssertTrue(afterLock.isEmpty)
    }

    #if os(macOS)
    func testDragItemKeepsTheSidebarPayload() throws {
        let bookings = Array(model.bookings.prefix(3))
        let item = BookingDragItem(bookings: bookings, accounts: model.accountsByID)
        XCTAssertEqual(BookingDrag.ids(BookingDrag.payload(item.ids)), bookings.map(\.id))
        XCTAssertEqual(item.export.transactions.map(\.id), bookings.map(\.id))
        XCTAssertNil(item.documents, "no reader given")
    }
    #endif
}
