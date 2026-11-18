import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Linked receipts (#590): a receipt that is only a reference to a document
/// kept elsewhere. The app stores and opens the address, it never fetches it.
@MainActor
final class DocumentLinkTests: XCTestCase {
    private var model: AppModel!
    private let link = "https://archiv.example/documents/123"

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        XCTAssertNil(model.errorMessage)
    }

    override func tearDown() async throws { model = nil }

    /// A demo booking without any document.
    private func plainBooking() throws -> Booking {
        try XCTUnwrap(model.bookings.first { $0.documentCount == 0 })
    }

    func testALinkIsAReceiptWithoutAFile() throws {
        let booking = try plainBooking()
        let before = model.allDocuments().count
        XCTAssertNil(model.attachLink(link, title: "", to: booking))

        let doc = try XCTUnwrap(model.documents(for: booking).first)
        XCTAssertEqual(model.documents(for: booking).count, 1)
        XCTAssertTrue(doc.isLink)
        XCTAssertEqual(doc.linkUrl, link)
        XCTAssertEqual(doc.webURL?.absoluteString, link)
        XCTAssertEqual(doc.linkHost, "archiv.example")
        XCTAssertEqual(doc.title, "archiv.example/documents/123")
        XCTAssertEqual(doc.bookingId, booking.id)
        XCTAssertEqual(doc.accountId, booking.accountId)
        XCTAssertEqual(docSymbol(doc), "link")
        // nothing to preview, and no core call that could only fail
        XCTAssertNil(model.documentData(doc))
        XCTAssertNil(model.errorMessage)

        // it counts as an own receipt, never as a bank document
        XCTAssertEqual(model.allDocuments().count, before + 1)
        XCTAssertFalse(doc.isBankDocument)
        XCTAssertTrue(model.allDocuments(scope: .receipts).contains { $0.id == doc.id })
        XCTAssertFalse(model.allDocuments(scope: .postbox).contains { $0.id == doc.id })
        XCTAssertEqual(model.allDocuments(search: "archiv.example").map(\.id), [doc.id])
        XCTAssertTrue(model.allDocuments(accountId: booking.accountId).contains { $0.id == doc.id })
        // ids of links and of stored files never collide (List selection, ForEach)
        let ids = model.allDocuments().map(\.id)
        XCTAssertEqual(Set(ids).count, ids.count)
    }

    func testARefusedLinkReportsWhyAndStoresNothing() throws {
        let booking = try plainBooking()
        for bad in ["", "ftp://archiv.example/beleg.pdf", "file:///etc/hosts", "javascript:alert(1)", "https://name:geheim@archiv.example/1"] {
            let problem = model.attachLink(bad, title: "", to: booking)
            XCTAssertNotNil(problem, bad)
            // the message never repeats the address
            XCTAssertFalse(problem?.contains("archiv.example") ?? false, bad)
        }
        XCTAssertNil(model.attachLink(link, title: "Rechnung", to: booking))
        XCTAssertNotNil(model.attachLink(link, title: "nochmal", to: booking), "once per booking")
        XCTAssertEqual(model.documents(for: booking).map(\.title), ["Rechnung"])
        // refusals stay next to the field: no alert
        XCTAssertNil(model.errorMessage)
    }

    func testADropOnABookingTakesAWebAddressAsLinkAndNeverReadsIt() throws {
        let booking = try plainBooking()
        let web = try XCTUnwrap(URL(string: link))
        let other = try XCTUnwrap(URL(string: "ftp://archiv.example/beleg.pdf"))
        model.attach([web, other], to: booking)
        XCTAssertNil(model.errorMessage)
        let docs = model.documents(for: booking)
        XCTAssertEqual(docs.map(\.linkUrl), [link])

        // dropped beside the bookings: nothing to match, nothing fetched, nothing stored
        let before = model.allDocuments().count
        model.toast = nil
        model.dropAnywhere([web])
        XCTAssertNotNil(model.toast)
        XCTAssertEqual(model.allDocuments().count, before)
        model.toast = nil
        model.dropAnywhere([other])
        XCTAssertNil(model.toast)
        XCTAssertEqual(model.allDocuments().count, before)
    }

    func testUnlinkingAndRemovingTouchOnlyTheReference() throws {
        let booking = try plainBooking()
        XCTAssertNil(model.attachLink(link, title: "Rechnung", to: booking))
        let doc = try XCTUnwrap(model.documents(for: booking).first)

        model.linkDocument(doc, to: nil)
        XCTAssertTrue(model.documents(for: booking).isEmpty)
        let unlinked = try XCTUnwrap(model.allDocuments(unlinkedOnly: true, scope: .receipts).first { $0.id == doc.id })
        XCTAssertNil(unlinked.bookingId)
        XCTAssertEqual(unlinked.accountId, booking.accountId, "stays with its account")

        model.linkDocument(unlinked, to: booking)
        XCTAssertEqual(model.documents(for: booking).map(\.id), [doc.id])

        model.deleteDocument(doc)
        XCTAssertTrue(model.documents(for: booking).isEmpty)
        XCTAssertFalse(model.allDocuments().contains { $0.id == doc.id })
        XCTAssertNil(model.errorMessage)
    }

    /// The vault can come from another device: the app opens only what the
    /// core would store.
    func testOnlyWebAddressesAreOpened() throws {
        let booking = try plainBooking()
        XCTAssertNil(model.attachLink(link, title: "", to: booking))
        var doc = try XCTUnwrap(model.documents(for: booking).first)
        XCTAssertNotNil(doc.webURL)
        for bad in ["javascript:alert(1)", "file:///etc/hosts", "ftp://archiv.example/x", "https://", "kein link"] {
            doc.linkUrl = bad
            XCTAssertNil(doc.webURL, bad)
            XCTAssertEqual(doc.linkHost, "", bad)
        }
        doc.linkUrl = ""
        XCTAssertFalse(doc.isLink)
        XCTAssertNil(doc.webURL)
        XCTAssertTrue(AppModel.isWebLink(try XCTUnwrap(URL(string: "HTTPS://archiv.example"))))
        XCTAssertFalse(AppModel.isWebLink(URL(fileURLWithPath: "/tmp/beleg.pdf")))
    }

    /// Space and ⌘Y preview the first stored file; a link has none.
    func testQuickLookIgnoresLinks() throws {
        let booking = try plainBooking()
        XCTAssertNil(model.attachLink(link, title: "", to: booking))
        let doc = try XCTUnwrap(model.documents(for: booking).first)
        model.quickLook(doc)
        XCTAssertNil(model.quickLookURL)
        XCTAssertNil(model.errorMessage)
    }
}
