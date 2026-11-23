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
    /// Most document bytes one drag carries (before base64); documents past it stay behind.
    static let documentByteLimit = 100 * 1024 * 1024

    /// A receipt or statement attached to the booking; `data` is base64 in the JSON.
    struct Document: Codable, Equatable, Sendable {
        var fileName: String
        var mimeType: String
        var title: String?
        var data: Data
    }

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
        /// Missing when the booking has none (or the drag's limit was reached).
        var documents: [Document]?
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

    /// Adds the documents read for the bookings (by booking id).
    mutating func attach(_ documents: [Int64: [Document]]) {
        for i in transactions.indices {
            if let docs = documents[transactions[i].id], !docs.isEmpty { transactions[i].documents = docs }
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
/// `BookingDrag` string for the sidebar's categories (#366). The JSON is built when another app asks for it, and the
/// documents are read then, never for the sidebar.
struct BookingDragItem: @unchecked Sendable {
    typealias DocumentReader = @MainActor ([Int64]) async -> [Int64: [BookingDragExport.Document]]

    let bookings: [Booking]
    let accounts: [Int64: Account]
    /// Reads the documents of the given bookings (`AppModel.dragDocuments`); nil = without documents.
    let documents: DocumentReader?

    init(bookings: [Booking], accounts: [Int64: Account], documents: DocumentReader? = nil) {
        self.bookings = bookings
        self.accounts = accounts
        self.documents = documents
    }

    var ids: [Int64] { bookings.map(\.id) }

    /// The JSON model, without documents.
    var export: BookingDragExport { BookingDragExport(bookings: bookings, accounts: accounts) }

    func itemProvider() -> NSItemProvider {
        // The sidebar reads the drag pasteboard for hover and loads NSString on drop.
        // Register the text directly so both work when the item also offers JSON.
        let text = BookingDrag.payload(ids)
        let provider = NSItemProvider(object: text as NSString)
        provider.registerDataRepresentation(forTypeIdentifier: UTType.plainText.identifier,
                                            visibility: .all) { completion in
            completion(Data(text.utf8), nil)
            return nil
        }
        provider.registerDataRepresentation(forTypeIdentifier: UTType.girocrabTransactions.identifier,
                                            visibility: .all) { completion in
            let task = Task {
                do {
                    var export = self.export
                    let withDocuments = bookings.filter { $0.documentCount > 0 }.map(\.id)
                    if let documents, !withDocuments.isEmpty { export.attach(await documents(withDocuments)) }
                    completion(try export.json(), nil)
                } catch {
                    completion(nil, error)
                }
            }
            let progress = Progress(totalUnitCount: 1)
            progress.cancellationHandler = { task.cancel() }
            return progress
        }
        return provider
    }
}
#endif

private extension String {
    /// nil for an empty or whitespace-only text.
    var nonBlank: String? {
        trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : self
    }
}
