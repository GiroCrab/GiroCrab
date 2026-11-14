import Foundation
import SwiftUI
import UniformTypeIdentifiers
import PDFKit
import CoreGraphics
import GiroCrabCore
#if os(macOS)
import WebKit
import AppKit
#elseif canImport(UIKit)
import UIKit
import WebKit
#endif

// MARK: - Umsatz-Export (GitHub #53 PDF-Optionen, GitHub #372 CAMT.053)
//
// Quick export without the print panel (#53’s layout/options reused for PDF).
// Formats: white-label PDF (A4 Hochformat), Excel-CSV, CAMT.053.001.08 / .02 (#372).

enum BookingExportFormat: String, CaseIterable, Identifiable, Codable {
    case pdf
    case csv
    case camt053v08
    case camt053v02
    var id: String { rawValue }

    var title: String {
        switch self {
        case .pdf: return String(localized: "PDF")
        case .csv: return "CSV"
        case .camt053v08: return String(localized: "CAMT.053")
        case .camt053v02: return String(localized: "CAMT.053 (älter)")
        }
    }

    var subtitle: String {
        switch self {
        case .pdf: return String(localized: "Umsatzliste · A4 Hochformat")
        case .csv: return String(localized: "Excel · Semikolon · UTF-8")
        case .camt053v08: return String(localized: "ISO 20022 · aktueller Standard")
        case .camt053v02: return String(localized: "ISO 20022 · für ältere Programme")
        }
    }

    var contentType: UTType {
        switch self {
        case .pdf: return .pdf
        case .csv: return .commaSeparatedText
        case .camt053v08, .camt053v02: return .xml
        }
    }

    var fileExtension: String {
        switch self {
        case .pdf: return "pdf"
        case .csv: return "csv"
        case .camt053v08, .camt053v02: return "xml"
        }
    }

    var help: String { subtitle }

    var isCamt: Bool {
        switch self {
        case .camt053v08, .camt053v02: return true
        default: return false
        }
    }

    /// Migrate older saved preference `"xml"` → CAMT.053.001.08.
    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        if let v = BookingExportFormat(rawValue: raw) {
            self = v
        } else if raw == "xml" {
            self = .camt053v08
        } else {
            self = .pdf
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }
}

/// Which bookings the quick export uses (#372: filtered list or multi-selection).
enum ExportScope: String, CaseIterable, Identifiable, Codable {
    case filtered, selection
    var id: String { rawValue }

    var title: String {
        switch self {
        case .filtered: return String(localized: "Zeitraum")
        case .selection: return String(localized: "Auswahl")
        }
    }
}

/// Period chosen inside the export sheet (does not change the main list).
enum ExportPeriodPreset: String, CaseIterable, Identifiable, Codable {
    case current
    case thisMonth
    case lastMonth
    case last3Months
    case last12Months
    case thisQuarter
    case thisYear
    case lastYear
    case sinceSalary
    case all
    case custom

    var id: String { rawValue }

    /// The list period this preset stands for (`nil` for `.current` / `.custom`).
    func listPeriod(now: Date = Date()) -> Period? {
        let menu = Period.menu(now: now)
        switch self {
        case .current, .custom: return nil
        case .thisMonth: return menu[2][0]
        case .lastMonth: return Period.previousMonth(now: now)
        case .last3Months: return .days(90)
        case .last12Months: return .months12
        case .thisQuarter: return menu[3][0]
        case .thisYear: return menu[4][0]
        case .lastYear: return Period.previousYear(now: now)
        case .sinceSalary: return .sinceSalary
        case .all: return .all
        }
    }

    var title: String {
        switch self {
        case .current: return String(localized: "Aktuelle Ansicht")
        case .custom: return String(localized: "Eigener Zeitraum")
        default: return listPeriod()?.title ?? ""
        }
    }

    /// Presets shown as a flat list (custom uses date pickers below).
    static var menuCases: [ExportPeriodPreset] {
        [.current, .thisMonth, .lastMonth, .last3Months, .last12Months,
         .thisQuarter, .thisYear, .lastYear, .sinceSalary, .all, .custom]
    }

    /// ISO from/to for vault queries; `nil` range means “use AppModel.periodRange”.
    /// `.custom` needs `customFrom`/`customTo` from `ExportOptions`.
    func range(now: Date = Date(), customFrom: String = "", customTo: String = "",
               salaryFrom: String? = nil) -> (String?, String?)? {
        switch self {
        case .current: return nil
        case .sinceSalary:
            if let salaryFrom { return (salaryFrom, nil) }
            return Period.sinceSalary.range(now: now)
        case .custom:
            let a = customFrom.isEmpty ? nil : customFrom
            let b = customTo.isEmpty ? nil : customTo
            return (a, b)
        default:
            return listPeriod(now: now)?.range(now: now)
        }
    }
}

/// Combined file vs one file per selected account.
enum ExportAccountOutput: String, CaseIterable, Identifiable, Codable {
    case combined
    case perAccount
    var id: String { rawValue }

    var title: String {
        switch self {
        case .combined: return String(localized: "Alle in einer Datei")
        case .perAccount: return String(localized: "Jedes Konto einzeln")
        }
    }

    var help: String {
        switch self {
        case .combined: return String(localized: "Eine Umsatzliste mit den gewählten Konten")
        case .perAccount: return String(localized: "Eine Datei pro gewähltem Konto nach Downloads")
        }
    }
}

/// Choices of the export dialog; remembered across launches (separate from print).
struct ExportOptions: Codable, Equatable {
    var format: BookingExportFormat = .pdf
    var scope: ExportScope = .filtered
    var periodPreset: ExportPeriodPreset = .lastMonth
    /// ISO bounds when `periodPreset == .custom` (export-only, not the main list).
    var customFrom: String = ""
    var customTo: String = ""
    /// Checked accounts in the picker (empty until the sheet bootstraps).
    var selectedAccountIDs: [Int64] = []
    /// One file vs one file per account (ignored for CAMT).
    var accountOutput: ExportAccountOutput = .combined
    var showCategory = true
    var showNote = false
    /// Income/expense totals – off for Umsatzliste PDFs (forced in `asPrintOptions`).
    var showTotals = false
    /// Multi-account PDFs group by account.
    var groupByAccount = true
    /// Current live balance – off for Umsatzliste; period opening/closing are used instead.
    var showCurrentBalance = false
    var showDailyBalance = false
    var firstLineOnly = true
    var direction: PrintDirection = .all
    /// Oldest first (statement convention).
    var newestFirst = false

    private static let key = "export.options"

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let d = ExportOptions.defaults
        format = try c.decodeIfPresent(BookingExportFormat.self, forKey: .format) ?? d.format
        scope = try c.decodeIfPresent(ExportScope.self, forKey: .scope) ?? d.scope
        periodPreset = try c.decodeIfPresent(ExportPeriodPreset.self, forKey: .periodPreset) ?? d.periodPreset
        customFrom = try c.decodeIfPresent(String.self, forKey: .customFrom) ?? d.customFrom
        customTo = try c.decodeIfPresent(String.self, forKey: .customTo) ?? d.customTo
        if let ids = try c.decodeIfPresent([Int64].self, forKey: .selectedAccountIDs) {
            selectedAccountIDs = ids
        } else if let legacy = try c.decodeIfPresent(Int64.self, forKey: .accountID) {
            selectedAccountIDs = [legacy]
        } else {
            selectedAccountIDs = d.selectedAccountIDs
        }
        if let out = try c.decodeIfPresent(ExportAccountOutput.self, forKey: .accountOutput) {
            accountOutput = out
        } else if let bulk = try c.decodeIfPresent(Bool.self, forKey: .bulkPerAccount) {
            accountOutput = bulk ? .perAccount : .combined
        } else {
            accountOutput = d.accountOutput
        }
        showCategory = try c.decodeIfPresent(Bool.self, forKey: .showCategory) ?? d.showCategory
        showNote = try c.decodeIfPresent(Bool.self, forKey: .showNote) ?? d.showNote
        showTotals = try c.decodeIfPresent(Bool.self, forKey: .showTotals) ?? d.showTotals
        groupByAccount = try c.decodeIfPresent(Bool.self, forKey: .groupByAccount) ?? d.groupByAccount
        showCurrentBalance = try c.decodeIfPresent(Bool.self, forKey: .showCurrentBalance) ?? d.showCurrentBalance
        showDailyBalance = try c.decodeIfPresent(Bool.self, forKey: .showDailyBalance) ?? d.showDailyBalance
        firstLineOnly = try c.decodeIfPresent(Bool.self, forKey: .firstLineOnly) ?? d.firstLineOnly
        direction = try c.decodeIfPresent(PrintDirection.self, forKey: .direction) ?? d.direction
        newestFirst = try c.decodeIfPresent(Bool.self, forKey: .newestFirst) ?? d.newestFirst
    }

    private enum CodingKeys: String, CodingKey {
        case format, scope, periodPreset, customFrom, customTo, selectedAccountIDs, accountOutput
        case accountID, bulkPerAccount // legacy decode only
        case showCategory, showNote, showTotals, groupByAccount, showCurrentBalance
        case showDailyBalance, firstLineOnly, direction, newestFirst
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(format, forKey: .format)
        try c.encode(scope, forKey: .scope)
        try c.encode(periodPreset, forKey: .periodPreset)
        try c.encode(customFrom, forKey: .customFrom)
        try c.encode(customTo, forKey: .customTo)
        try c.encode(selectedAccountIDs, forKey: .selectedAccountIDs)
        try c.encode(accountOutput, forKey: .accountOutput)
        try c.encode(showCategory, forKey: .showCategory)
        try c.encode(showNote, forKey: .showNote)
        try c.encode(showTotals, forKey: .showTotals)
        try c.encode(groupByAccount, forKey: .groupByAccount)
        try c.encode(showCurrentBalance, forKey: .showCurrentBalance)
        try c.encode(showDailyBalance, forKey: .showDailyBalance)
        try c.encode(firstLineOnly, forKey: .firstLineOnly)
        try c.encode(direction, forKey: .direction)
        try c.encode(newestFirst, forKey: .newestFirst)
    }

    private static let defaults = ExportOptions()

    static func load(_ defaults: UserDefaults = .standard) -> ExportOptions {
        guard let data = defaults.data(forKey: key) else { return ExportOptions() }
        do {
            return try JSONDecoder().decode(ExportOptions.self, from: data)
        } catch {
            // Prefer a partial restore over wiping the user's content toggles.
            if let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                return ExportOptions.partial(from: json)
            }
            return ExportOptions()
        }
    }

    /// Best-effort restore when the full Codable decode fails (schema drift).
    private static func partial(from json: [String: Any]) -> ExportOptions {
        var o = ExportOptions()
        if let s = json["format"] as? String, let f = BookingExportFormat(rawValue: s) { o.format = f }
        else if let s = json["format"] as? String, s == "xml" { o.format = .camt053v08 }
        if let s = json["scope"] as? String, let v = ExportScope(rawValue: s) { o.scope = v }
        if let s = json["periodPreset"] as? String, let v = ExportPeriodPreset(rawValue: s) { o.periodPreset = v }
        if let s = json["customFrom"] as? String { o.customFrom = s }
        if let s = json["customTo"] as? String { o.customTo = s }
        if let ids = json["selectedAccountIDs"] as? [Int] { o.selectedAccountIDs = ids.map(Int64.init) }
        else if let id = json["accountID"] as? Int { o.selectedAccountIDs = [Int64(id)] }
        if let s = json["accountOutput"] as? String, let v = ExportAccountOutput(rawValue: s) { o.accountOutput = v }
        else if let bulk = json["bulkPerAccount"] as? Bool { o.accountOutput = bulk ? .perAccount : .combined }
        if let b = json["showCategory"] as? Bool { o.showCategory = b }
        if let b = json["showNote"] as? Bool { o.showNote = b }
        if let b = json["showTotals"] as? Bool { o.showTotals = b }
        if let b = json["groupByAccount"] as? Bool { o.groupByAccount = b }
        if let b = json["showCurrentBalance"] as? Bool { o.showCurrentBalance = b }
        if let b = json["showDailyBalance"] as? Bool { o.showDailyBalance = b }
        if let b = json["firstLineOnly"] as? Bool { o.firstLineOnly = b }
        if let b = json["newestFirst"] as? Bool { o.newestFirst = b }
        return o
    }

    func save(_ defaults: UserDefaults = .standard) {
        guard let data = try? JSONEncoder().encode(self) else { return }
        defaults.set(data, forKey: Self.key)
        defaults.synchronize()
    }

    /// True when content toggles differ from a fresh default (opens „Weitere Optionen“).
    var hasNonDefaultContent: Bool {
        let d = ExportOptions.defaults
        return showCategory != d.showCategory
            || showNote != d.showNote
            || showDailyBalance != d.showDailyBalance
            || groupByAccount != d.groupByAccount
            || firstLineOnly != d.firstLineOnly
            || newestFirst != d.newestFirst
    }

    /// Print options used for the PDF HTML (always white-label Umsatzliste).
    var asPrintOptions: PrintOptions {
        var p = PrintOptions()
        p.showCategory = showCategory
        p.showNote = showNote
        p.showTotals = false
        p.groupByAccount = groupByAccount
        p.showCurrentBalance = false
        p.showDailyBalance = showDailyBalance
        p.firstLineOnly = firstLineOnly
        p.direction = direction
        p.newestFirst = newestFirst
        p.logo = .off
        p.statementStyle = true
        return p
    }
}

/// Last save folder for Umsatz-Export (defaults to Downloads).
enum ExportDirectory {
    private static let key = "export.lastDirectory"

    static var preferred: URL {
        if let path = UserDefaults.standard.string(forKey: key) {
            let url = URL(fileURLWithPath: path, isDirectory: true)
            var isDir: ObjCBool = false
            if FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue {
                return url
            }
        }
        return FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
            ?? FileManager.default.homeDirectoryForCurrentUser
    }

    /// `filename` in `directory`, or the first free `Name-2.ext`, `Name-3.ext`, … so an
    /// export never replaces an existing file without asking.
    static func freeURL(in directory: URL, filename: String,
                        exists: (URL) -> Bool = { FileManager.default.fileExists(atPath: $0.path) }) -> URL {
        let first = directory.appendingPathComponent(filename)
        guard exists(first) else { return first }
        let ext = (filename as NSString).pathExtension
        let stem = (filename as NSString).deletingPathExtension
        var n = 2
        while true {
            let name = ext.isEmpty ? "\(stem)-\(n)" : "\(stem)-\(n).\(ext)"
            let url = directory.appendingPathComponent(name)
            if !exists(url) { return url }
            n += 1
        }
    }

    static func remember(fileURL: URL) {
        UserDefaults.standard.set(fileURL.deletingLastPathComponent().path, forKey: key)
    }
}

/// Bytes handed to `.fileExporter` (PDF / CSV / CAMT.053).
struct ExportFileDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.pdf, .commaSeparatedText, .xml] }
    var data: Data
    init(data: Data) { self.data = data }
    init(configuration: ReadConfiguration) throws { data = configuration.file.regularFileContents ?? Data() }
    func fileWrapper(configuration: WriteConfiguration) -> FileWrapper { FileWrapper(regularFileWithContents: data) }
}

enum BookingExportBuilder {
    /// Excel CSV with optional Tagessaldo column and Kontostand header lines.
    static func csv(_ report: PrintReport, options: ExportOptions) -> String {
        let print = options.asPrintOptions
        let rows = PrintReportBuilder.rows(report.rows, options: print)
        var out = "\u{FEFF}"
        if options.showCurrentBalance {
            let ids = Set(rows.map(\.accountId))
            for a in report.accounts.filter({ ids.contains($0.id) }).sorted(by: { $0.name < $1.name }) {
                out += "# \(csvField(a.name, text: true));\(csvAmount(a.balanceCents));\(csvField(a.currency));\(csvField(a.balanceDate))\r\n"
            }
        }
        var header = ["Datum", "Konto", "Name", "Verwendungszweck"]
        if options.showCategory { header.append("Kategorie") }
        if options.showNote { header.append("Notiz") }
        header.append(contentsOf: ["Betrag", "Währung"])
        if options.showDailyBalance { header.append("Tagessaldo") }
        out += header.map { csvField($0) }.joined(separator: ";") + "\r\n"

        let balances = options.showDailyBalance ? report.dailyBalances[0] : nil
        for (i, r) in rows.enumerated() {
            var cols = [
                csvField(r.date),
                csvField(r.accountName, text: true),
                csvField(r.counterparty, text: true),
                csvField(PrintReportBuilder.displayPurpose(r, firstLineOnly: options.firstLineOnly), text: true),
            ]
            if options.showCategory { cols.append(csvField(r.category, text: true)) }
            if options.showNote { cols.append(csvField(r.note, text: true)) }
            cols.append(csvAmount(r.amountCents))
            cols.append(csvField(r.currency))
            if let balances {
                let next = i + 1 < rows.count ? rows[i + 1].date : nil
                let prev = i > 0 ? rows[i - 1].date : nil
                let edge = options.newestFirst ? prev != r.date : next != r.date
                let value = edge ? balances[r.date].map(csvAmount) ?? "" : ""
                cols.append(value)
            }
            out += cols.joined(separator: ";") + "\r\n"
        }
        return out
    }

    /// CAMT.053 account statement for one account (GitHub #372).
    /// OPBD = balance at the start of the first day; CLBD = OPBD + booked entries. Pending
    /// entries (`PDNG`) are listed but not part of CLBD, like on a bank statement.
    static func camt053(
        account: Account,
        bookings: [Booking],
        format: BookingExportFormat,
        openingCents: Int64,
        openingDate: String,
        created: Date = Date()
    ) -> String {
        let version = format == .camt053v02 ? "02" : "08"
        let ns = "urn:iso:std:iso:20022:tech:xsd:camt.053.001.\(version)"
        let v08 = version == "08"
        let ordered = bookings.sorted { a, b in
            if a.bookingDate != b.bookingDate { return a.bookingDate < b.bookingDate }
            return a.id < b.id
        }
        let booked = ordered.filter { !$0.isPending }.reduce(Int64(0)) { $0 + $1.amountCents }
        let closing = openingCents + booked
        let currency = ordered.first?.currency ?? account.currency
        let from = ordered.first?.bookingDate ?? openingDate
        let to = ordered.last?.bookingDate ?? openingDate
        let stamp = isoDateTime(created)
        let stmtId = String("GC-\(account.id)-\(from)-\(to)".prefix(35))
        let iban = account.iban.replacingOccurrences(of: " ", with: "").uppercased()
        let ownBIC = iban.isEmpty ? "" : (bicForIban(iban: iban) ?? "")
        let bicTag = v08 ? "BICFI" : "BIC"

        var out = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
        out += "<Document xmlns=\"\(ns)\">\n"
        out += "<BkToCstmrStmt>\n"
        out += "<GrpHdr><MsgId>\(esc(stmtId))</MsgId><CreDtTm>\(esc(stamp))</CreDtTm></GrpHdr>\n"
        out += "<Stmt>\n"
        out += "<Id>\(esc(stmtId))</Id><ElctrncSeqNb>1</ElctrncSeqNb><CreDtTm>\(esc(stamp))</CreDtTm>\n"
        // Acct: Id, Ccy, Ownr, Svcr – schema order.
        out += "<Acct><Id>\(accountIdXML(iban: iban, other: account.accountNumber, fallback: "GC-\(account.id)"))</Id>"
        out += "<Ccy>\(esc(currency))</Ccy>"
        let owner = account.owner.isEmpty ? account.name : account.owner
        if !owner.isEmpty {
            out += "<Ownr><Nm>\(esc(max140(owner)))</Nm></Ownr>"
        }
        if !ownBIC.isEmpty {
            out += "<Svcr><FinInstnId><\(bicTag)>\(esc(ownBIC))</\(bicTag)></FinInstnId></Svcr>"
        }
        out += "</Acct>\n"
        out += balanceXML(code: "OPBD", cents: openingCents, currency: currency, date: openingDate.isEmpty ? from : openingDate)
        out += balanceXML(code: "CLBD", cents: closing, currency: currency, date: to)
        for b in ordered {
            out += entryXML(b, v08: v08)
        }
        out += "</Stmt>\n</BkToCstmrStmt>\n</Document>\n"
        return out
    }

    /// Suggested save name, e.g. `Umsaetze-Girokonto-2025-03-01_2025-03-31.pdf`.
    static func defaultFilename(
        accounts: [PrintAccount],
        rows: [PrintRow],
        format: BookingExportFormat,
        fallbackTitle: String = ""
    ) -> String {
        let ids = Set(rows.map(\.accountId))
        let used = accounts.filter { ids.contains($0.id) }.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        let dates = rows.map(\.date).filter { !$0.isEmpty }.sorted()
        let from = dates.first
        let to = dates.last
        let range: String
        if let from, let to {
            range = from == to ? from : "\(from)_\(to)"
        } else {
            range = DateText.iso(Date())
        }
        let accountPart: String
        switch used.count {
        case 0:
            accountPart = "Umsaetze-\(safeFileComponent(fallbackTitle.isEmpty ? "Export" : fallbackTitle))"
        case 1:
            accountPart = "Umsaetze-\(safeFileComponent(used[0].name))"
        default:
            accountPart = "Umsaetze-\(used.count)-Konten"
        }
        switch format {
        case .camt053v08: return "\(accountPart)-CAMT053-001-08-\(range).xml"
        case .camt053v02: return "\(accountPart)-CAMT053-001-02-\(range).xml"
        case .pdf, .csv: return "\(accountPart)-\(range).\(format.fileExtension)"
        }
    }

    /// Title, period range and IBAN notes for a white-label Umsatzliste.
    static func prepareStatement(_ report: PrintReport) -> PrintReport {
        var out = report
        let ids = Set(report.rows.map(\.accountId))
        let used = report.accounts.filter { ids.contains($0.id) }
            .sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
        let dates = report.rows.map(\.date).filter { !$0.isEmpty }.sorted()
        if let from = dates.first, let to = dates.last {
            out.periodLabel = from == to
                ? PrintReportBuilder.statementDate(from)
                : "\(PrintReportBuilder.statementDate(from)) – \(PrintReportBuilder.statementDate(to))"
        }
        switch used.count {
        case 0:
            out.title = String(localized: "Umsätze")
        case 1:
            out.title = used[0].name
            out.filterNotes = out.filterNotes.filter { !$0.contains(used[0].name) }
        default:
            out.title = String(localized: "Umsätze")
        }
        return out
    }

    private static func safeFileComponent(_ s: String) -> String {
        let trimmed = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "Umsaetze" }
        let replaced = trimmed
            .replacingOccurrences(of: "/", with: "-")
            .replacingOccurrences(of: ":", with: "-")
            .replacingOccurrences(of: "\\", with: "-")
            .replacingOccurrences(of: "\n", with: " ")
        let collapsed = replaced
            .split(whereSeparator: { $0.isWhitespace })
            .joined(separator: "-")
        return String(collapsed.prefix(80))
    }

    /// Opening balance at the start of `firstDate` (= end of the previous day) from the
    /// end-of-day history (#372). `nil` when the history has no value for that day (e.g. a
    /// foreign-currency account): a CAMT file must not invent a 0 opening balance.
    static func camtOpeningBalance(endOfDay: [String: Int64], firstDate: String) -> Int64? {
        guard let prev = previousISODate(firstDate) else { return nil }
        return endOfDay[prev]
    }

    /// Day before `iso` (`yyyy-MM-dd`), for loading the history the opening balance needs.
    static func previousISODate(_ iso: String) -> String? {
        guard let d = DateText.date(iso) else { return nil }
        guard let prev = Calendar(identifier: .gregorian).date(byAdding: .day, value: -1, to: d) else { return nil }
        return DateText.iso(prev)
    }

    /// One CSV cell. `text` cells get the formula guard: a leading `=`, `+`, `-`, `@`, tab
    /// or CR would make Excel/Numbers evaluate the cell (`-2+3`, `=HYPERLINK(…)`).
    static func csvField(_ v: String, text: Bool = false) -> String {
        if text, let first = v.unicodeScalars.first, "=+-@\t\r".unicodeScalars.contains(first) {
            return csvField("'" + v)
        }
        let s = v.replacingOccurrences(of: "\r", with: " ").replacingOccurrences(of: "\n", with: " ")
        if s.contains(";") || s.contains("\"") {
            return "\"" + s.replacingOccurrences(of: "\"", with: "\"\"") + "\""
        }
        return s
    }

    private static func csvAmount(_ cents: Int64) -> String {
        let sign = cents < 0 ? "-" : ""
        let a = cents.magnitude
        return "\(sign)\(a / 100),\(String(format: "%02llu", a % 100))"
    }

    private static func xmlAmount(_ cents: Int64) -> String {
        let a = cents.magnitude
        return "\(a / 100).\(String(format: "%02llu", a % 100))"
    }

    private static func balanceXML(code: String, cents: Int64, currency: String, date: String) -> String {
        let ind = cents < 0 ? "DBIT" : "CRDT"
        return "<Bal><Tp><CdOrPrtry><Cd>\(code)</Cd></CdOrPrtry></Tp>"
            + "<Amt Ccy=\"\(esc(currency))\">\(xmlAmount(cents))</Amt>"
            + "<CdtDbtInd>\(ind)</CdtDbtInd><Dt><Dt>\(esc(date))</Dt></Dt></Bal>\n"
    }

    private static func entryXML(_ b: Booking, v08: Bool) -> String {
        let ind = b.amountCents < 0 ? "DBIT" : "CRDT"
        let status = b.isPending ? "PDNG" : "BOOK"
        let value = b.valueDate.isEmpty ? b.bookingDate : b.valueDate
        let name = b.merchant.name.isEmpty ? b.counterparty : b.merchant.name
        let purpose = b.cleanPurpose.isEmpty ? b.purpose : b.cleanPurpose
        let peerIBAN = b.counterpartyIban.replacingOccurrences(of: " ", with: "").uppercased()
        let peerIsIBAN = isIBAN(peerIBAN)
        let peerBIC = peerIsIBAN ? bicForIban(iban: peerIBAN) : nil
        let bicTag = v08 ? "BICFI" : "BIC"

        var e = "<Ntry>"
        e += "<Amt Ccy=\"\(esc(b.currency))\">\(xmlAmount(b.amountCents))</Amt>"
        e += "<CdtDbtInd>\(ind)</CdtDbtInd>"
        // .08: Sts is a choice (ExternalEntryStatus1Code), .02: a plain code.
        e += v08 ? "<Sts><Cd>\(status)</Cd></Sts>" : "<Sts>\(status)</Sts>"
        e += "<BookgDt><Dt>\(esc(b.bookingDate))</Dt></BookgDt>"
        e += "<ValDt><Dt>\(esc(value))</Dt></ValDt>"
        // BkTxCd is mandatory in both versions; empty when the bank sent no GVC.
        let gvc = String(b.gvc.prefix(35))
        e += gvc.isEmpty ? "<BkTxCd/>" : "<BkTxCd><Prtry><Cd>\(esc(gvc))</Cd></Prtry></BkTxCd>"
        e += "<NtryDtls><TxDtls>"
        // TxDtls order: RltdPties, RltdAgts, Purp, RmtInf.
        if !name.isEmpty || !peerIBAN.isEmpty {
            e += "<RltdPties>"
            // Own account credits → debtor is peer; debits → creditor is peer.
            let peerTag = b.amountCents < 0 ? "Cdtr" : "Dbtr"
            let acctTag = b.amountCents < 0 ? "CdtrAcct" : "DbtrAcct"
            if !name.isEmpty {
                e += v08
                    ? "<\(peerTag)><Pty><Nm>\(esc(max140(name)))</Nm></Pty></\(peerTag)>"
                    : "<\(peerTag)><Nm>\(esc(max140(name)))</Nm></\(peerTag)>"
            }
            if !peerIBAN.isEmpty {
                e += "<\(acctTag)><Id>\(accountIdXML(iban: peerIBAN, other: peerIBAN, fallback: ""))</Id></\(acctTag)>"
            }
            e += "</RltdPties>"
            if let bic = peerBIC, !bic.isEmpty {
                let agent = b.amountCents < 0 ? "CdtrAgt" : "DbtrAgt"
                e += "<RltdAgts><\(agent)><FinInstnId><\(bicTag)>\(esc(bic))</\(bicTag)></FinInstnId></\(agent)></RltdAgts>"
            }
        }
        // ExternalPurpose1Code: 1–4 characters.
        if (1...4).contains(b.purposeCode.count) {
            e += "<Purp><Cd>\(esc(b.purposeCode))</Cd></Purp>"
        }
        let lines = ustrdLines(purpose)
        if !lines.isEmpty {
            e += "<RmtInf>" + lines.map { "<Ustrd>\(esc($0))</Ustrd>" }.joined() + "</RmtInf>"
        }
        e += "</TxDtls></NtryDtls></Ntry>\n"
        return e
    }

    /// `<IBAN>` when `iban` is one, else `<Othr><Id>` (Max34Text) – the schema's IBAN
    /// pattern rejects account numbers and card numbers.
    private static func accountIdXML(iban: String, other: String, fallback: String) -> String {
        if isIBAN(iban) { return "<IBAN>\(esc(iban))</IBAN>" }
        let id = other.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? fallback : other
        return "<Othr><Id>\(esc(String(id.trimmingCharacters(in: .whitespacesAndNewlines).prefix(34))))</Id></Othr>"
    }

    /// ISO 20022 IBAN2007Identifier: `[A-Z]{2}[0-9]{2}[a-zA-Z0-9]{1,30}`.
    static func isIBAN(_ s: String) -> Bool {
        s.range(of: "^[A-Z]{2}[0-9]{2}[a-zA-Z0-9]{1,30}$", options: .regularExpression) != nil
    }

    /// Remittance text as `Ustrd` lines of at most 140 characters (Max140Text).
    static func ustrdLines(_ text: String) -> [String] {
        let flat = text.split(whereSeparator: \.isNewline).joined(separator: " ")
            .trimmingCharacters(in: .whitespaces)
        guard !flat.isEmpty else { return [] }
        var out: [String] = []
        var rest = Substring(flat)
        while !rest.isEmpty {
            out.append(String(rest.prefix(140)))
            rest = rest.dropFirst(140)
        }
        return out
    }

    private static func max140(_ s: String) -> String { String(s.prefix(140)) }

    private static func isoDateTime(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: date)
    }

    private static func esc(_ s: String) -> String { PrintReportBuilder.esc(s) }
}

#if os(macOS) || os(iOS)
/// Renders the report HTML to a multi-page A4 PDF.
/// Captures each laid-out `.page` via `createPDF(rect:)` and draws it into a
/// fresh A4 PDF page (never mutates media boxes – that mirrored the content).
@MainActor
final class HTMLPDFRenderer: NSObject, WKNavigationDelegate {
    static let shared = HTMLPDFRenderer()
    private var webView: WKWebView?
    private var continuation: CheckedContinuation<Data, Error>?
    private var paper = PrintReportBuilder.Paper.a4

    enum RenderError: LocalizedError {
        case empty
        case mergeFailed
        var errorDescription: String? {
            switch self {
            case .empty: return String(localized: "PDF konnte nicht erzeugt werden.")
            case .mergeFailed: return String(localized: "PDF-Seiten konnten nicht zusammengefügt werden.")
            }
        }
    }

    func pdf(html: String, paper: PrintReportBuilder.Paper = .a4) async throws -> Data {
        if continuation != nil {
            webView = nil
            continuation?.resume(throwing: CancellationError())
            continuation = nil
        }
        self.paper = paper
        return try await withCheckedThrowingContinuation { cont in
            continuation = cont
            let w = paper.widthMM * 96 / 25.4
            let h = paper.heightMM * 96 / 25.4
            let web = WKWebView(frame: CGRect(x: 0, y: 0, width: w, height: h))
            web.navigationDelegate = self
            webView = web
            web.loadHTMLString(html, baseURL: nil)
        }
    }

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        MainActor.assumeIsolated {
            DispatchQueue.main.async { self.capturePages(webView) }
        }
    }

    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        MainActor.assumeIsolated { fail(error) }
    }

    nonisolated func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        MainActor.assumeIsolated { fail(error) }
    }

    private func capturePages(_ web: WKWebView) {
        let js = """
        (function(){
          var pages=document.querySelectorAll('.page');
          function box(el){
            var r=el.getBoundingClientRect();
            return {x:r.left+window.scrollX,y:r.top+window.scrollY,w:r.width,h:r.height};
          }
          if(pages.length) return Array.from(pages).map(box);
          var b=document.body;
          return [{x:0,y:0,w:Math.max(b.scrollWidth,document.documentElement.scrollWidth),
                   h:Math.max(b.scrollHeight,document.documentElement.scrollHeight)}];
        })()
        """
        web.evaluateJavaScript(js) { [weak self] result, error in
            Task { @MainActor in
                guard let self else { return }
                if let error { self.fail(error); return }
                let rects = Self.parseRects(result)
                guard !rects.isEmpty else { self.fail(RenderError.empty); return }
                do {
                    let data = try await self.renderA4(web, rects: rects)
                    self.continuation?.resume(returning: data)
                } catch {
                    self.continuation?.resume(throwing: error)
                }
                self.continuation = nil
                self.webView = nil
            }
        }
    }

    /// Fit each WKWebView `createPDF` slice onto a real A4 PDF page.
    /// Important: CGPDFContext is bottom-left origin like PDFPage – do **not**
    /// apply an AppKit-style Y-flip here (that mirrored/upside-down the file).
    private func renderA4(_ web: WKWebView, rects: [CGRect]) async throws -> Data {
        let pageW = CGFloat(paper.widthMM * 72 / 25.4)
        let pageH = CGFloat(paper.heightMM * 72 / 25.4)
        let out = PDFDocument()
        for (i, rect) in rects.enumerated() {
            let config = WKPDFConfiguration()
            config.rect = rect
            let slice = try await Self.createPDF(web, configuration: config)
            guard let sliceDoc = PDFDocument(data: slice), let src = sliceDoc.page(at: 0) else {
                throw RenderError.mergeFailed
            }
            var media = CGRect(x: 0, y: 0, width: pageW, height: pageH)
            let data = NSMutableData()
            guard let consumer = CGDataConsumer(data: data as CFMutableData),
                  let ctx = CGContext(consumer: consumer, mediaBox: &media, nil) else {
                throw RenderError.mergeFailed
            }
            ctx.beginPDFPage(nil)
            let srcBox = src.bounds(for: .mediaBox)
            let scale = min(pageW / max(srcBox.width, 1), pageH / max(srcBox.height, 1))
            let drawW = srcBox.width * scale
            let drawH = srcBox.height * scale
            let dx = (pageW - drawW) / 2
            let dy = (pageH - drawH) / 2
            ctx.saveGState()
            ctx.translateBy(x: dx, y: dy)
            ctx.scaleBy(x: scale, y: scale)
            src.draw(with: .mediaBox, to: ctx)
            ctx.restoreGState()
            ctx.endPDFPage()
            ctx.closePDF()
            guard let pageDoc = PDFDocument(data: data as Data), let page = pageDoc.page(at: 0) else {
                throw RenderError.mergeFailed
            }
            out.insert(page, at: i)
        }
        guard let data = out.dataRepresentation(), !data.isEmpty else { throw RenderError.mergeFailed }
        return data
    }

    private static func createPDF(_ web: WKWebView, configuration: WKPDFConfiguration) async throws -> Data {
        try await withCheckedThrowingContinuation { cont in
            web.createPDF(configuration: configuration) { result in
                switch result {
                case .success(let data): cont.resume(returning: data)
                case .failure(let error): cont.resume(throwing: error)
                }
            }
        }
    }

    private static func parseRects(_ result: Any?) -> [CGRect] {
        guard let rows = result as? [[String: Any]] else { return [] }
        return rows.compactMap { d in
            guard let x = d["x"] as? Double, let y = d["y"] as? Double,
                  let w = d["w"] as? Double, let h = d["h"] as? Double,
                  w > 1, h > 1 else { return nil }
            return CGRect(x: x, y: y, width: w, height: h)
        }
    }

    private func fail(_ error: Error) {
        continuation?.resume(throwing: error)
        continuation = nil
        webView = nil
    }
}
#endif
