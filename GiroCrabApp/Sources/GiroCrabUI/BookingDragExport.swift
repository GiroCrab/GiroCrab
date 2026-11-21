import CoreTransferable
import Foundation
import UniformTypeIdentifiers
import GiroCrabCore

extension UTType {
    /// Bookings dragged into another app (receipt managers, spreadsheets): JSON in the form of
    /// `BookingDragExport`, described in `docs/booking-drag.md`. Declared in `Xcode/LaunchInfo.plist`.
    static let girocrabTransactions = UTType(exportedAs: "de.girocrab.transactions", conformingTo: .json)
}

/// The JSON a booking drag hands to other apps (version 1). The field names follow the
/// `transactions` method of the local interface (`docs/local-api.md`), so a receiving app reads
/// both with one parser. Empty texts are left out. The `id` is the booking's id in this vault: it
/// stays the same for the booking's lifetime on this Mac, another device has its own ids.
struct BookingDragExport: Codable, Equatable {
    static let currentVersion = 1

    struct Transaction: Codable, Equatable {
        var id: Int64
        var accountId: Int64
        var accountName: String
        var accountIban: String?
        var bookingDate: String
        var valueDate: String
        /// Negative = money out.
        var amountCents: Int64
        var currency: String
        var pending: Bool
        var counterparty: String?
        var counterpartyIban: String?
        var purpose: String?
        var bookingText: String?
        var categoryPath: String?
        var merchant: String?
        var note: String?
        var endToEndId: String?
        var mandateId: String?
        var creditorId: String?
    }

    var version: Int
    var creator: String
    var transactions: [Transaction]

    init(bookings: [Booking], accounts: [Int64: Account], creator: String = AppVersion.displayString) {
        version = Self.currentVersion
        self.creator = creator
        transactions = bookings.map { b in
            Transaction(
                id: b.id, accountId: b.accountId, accountName: b.accountName,
                accountIban: accounts[b.accountId]?.iban.nonBlank,
                bookingDate: b.bookingDate, valueDate: b.valueDate.nonBlank ?? b.bookingDate,
                amountCents: b.amountCents, currency: b.currency, pending: b.isPending,
                counterparty: b.counterparty.nonBlank, counterpartyIban: b.counterpartyIban.nonBlank,
                purpose: b.purpose.nonBlank, bookingText: b.bookingText.nonBlank,
                categoryPath: b.categoryPath.nonBlank, merchant: b.merchantName.nonBlank,
                note: b.note.nonBlank, endToEndId: b.endToEndId.nonBlank,
                mandateId: b.mandateId.nonBlank, creditorId: b.creditorId.nonBlank
            )
        }
    }

    func json() throws -> Data {
        let encoder = JSONEncoder()
        encoder.keyEncodingStrategy = .convertToSnakeCase
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(self)
    }

    static func decode(_ data: Data) throws -> BookingDragExport {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(BookingDragExport.self, from: data)
    }
}

#if os(macOS)
/// What a dragged booking row carries (macOS): the bookings as JSON for other apps and the
/// `BookingDrag` string for the sidebar's categories (#366). Built when the drag starts.
struct BookingDragItem: Transferable, Sendable {
    let ids: [Int64]
    let json: Data?

    init(bookings: [Booking], accounts: [Int64: Account]) {
        ids = bookings.map(\.id)
        json = try? BookingDragExport(bookings: bookings, accounts: accounts).json()
    }

    static var transferRepresentation: some TransferRepresentation {
        DataRepresentation(exportedContentType: .girocrabTransactions) { item in
            guard let json = item.json else { throw CocoaError(.fileWriteUnknown) }
            return json
        }
        ProxyRepresentation(exporting: { (item: BookingDragItem) in BookingDrag.payload(item.ids) })
    }
}
#endif

private extension String {
    /// nil for an empty or whitespace-only text.
    var nonBlank: String? {
        trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self
    }
}
