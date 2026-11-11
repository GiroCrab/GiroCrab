import Foundation
import GiroCrabCore

// MARK: - Print report (GitHub #53, part 2)
//
// Pure model + HTML builder for the booking printout. The sheet collects the
// options and the selected bookings; `PrintReportBuilder.html` renders them.
// HTML is printed through WebKit (macOS) / UIKit print formatters (iOS), which
// paginate and repeat table headers on every page.

/// Which bookings to print by direction of the amount.
enum PrintDirection: String, CaseIterable, Identifiable, Codable {
    case all, income, expenses
    var id: String { rawValue }

    var title: String {
        switch self {
        case .all: return String(localized: "Einnahmen und Ausgaben")
        case .income: return String(localized: "Nur Einnahmen")
        case .expenses: return String(localized: "Nur Ausgaben")
        }
    }

    func includes(_ amountCents: Int64) -> Bool {
        switch self {
        case .all: return true
        case .income: return amountCents > 0
        case .expenses: return amountCents < 0
        }
    }
}

/// User choices of the print dialog; remembered across launches.
struct PrintOptions: Codable, Equatable {
    var showCategory = true
    var showNote = true
    var showTotals = true
    var groupByAccount = false
    var showCurrentBalance = true
    var showDailyBalance = false
    var firstLineOnly = false
    var direction: PrintDirection = .all
    var newestFirst = true

    private static let key = "print.options"

    static func load(_ defaults: UserDefaults = .standard) -> PrintOptions {
        guard let data = defaults.data(forKey: key),
              let o = try? JSONDecoder().decode(PrintOptions.self, from: data) else { return PrintOptions() }
        return o
    }

    func save(_ defaults: UserDefaults = .standard) {
        if let data = try? JSONEncoder().encode(self) { defaults.set(data, forKey: Self.key) }
    }
}

/// One booking as the printout needs it (decoupled from the UniFFI record).
struct PrintRow: Identifiable, Equatable {
    let id: Int64
    let accountId: Int64
    let accountName: String
    let date: String
    let amountCents: Int64
    let currency: String
    let counterparty: String
    let purpose: String
    let category: String
    let note: String

    init(id: Int64, accountId: Int64, accountName: String, date: String, amountCents: Int64,
         currency: String = "EUR", counterparty: String, purpose: String, category: String = "", note: String = "") {
        self.id = id; self.accountId = accountId; self.accountName = accountName; self.date = date
        self.amountCents = amountCents; self.currency = currency; self.counterparty = counterparty
        self.purpose = purpose; self.category = category; self.note = note
    }

    init(_ b: Booking) {
        let name = b.merchant.name.isEmpty ? b.counterparty : b.merchant.name
        self.init(id: b.id, accountId: b.accountId, accountName: b.accountName, date: b.bookingDate,
                  amountCents: b.amountCents, currency: b.currency, counterparty: name,
                  purpose: b.cleanPurpose.isEmpty ? b.purpose : b.cleanPurpose,
                  category: b.categoryPath, note: b.note)
    }
}

/// Account header data (name, IBAN, current balance).
struct PrintAccount: Equatable {
    let id: Int64
    let name: String
    let iban: String
    let balanceCents: Int64
    let balanceDate: String
    let currency: String
}

struct PrintReport {
    /// Sidebar selection, e.g. "Girokonto" or "Übersicht".
    var title: String
    /// Period label, e.g. "Letzter Monat" or "15.03.2025 – 30.06.2025".
    var periodLabel: String
    /// Selected bookings (any order; the builder filters and sorts).
    var rows: [PrintRow]
    var accounts: [PrintAccount] = []
    /// End-of-day balances per account id; key 0 = all printed accounts combined.
    var dailyBalances: [Int64: [String: Int64]] = [:]
    var generatedAt = Date()
}

enum PrintReportBuilder {
    /// Rows after the direction filter, in print order.
    static func rows(_ rows: [PrintRow], options: PrintOptions) -> [PrintRow] {
        rows.filter { options.direction.includes($0.amountCents) }
            .sorted { a, b in
                if a.date != b.date { return options.newestFirst ? a.date > b.date : a.date < b.date }
                return options.newestFirst ? a.id > b.id : a.id < b.id
            }
    }

    struct Totals: Equatable {
        var incomeCents: Int64 = 0
        var expenseCents: Int64 = 0
        var netCents: Int64 { incomeCents + expenseCents }
    }

    static func totals(_ rows: [PrintRow]) -> Totals {
        var t = Totals()
        for r in rows {
            if r.amountCents > 0 { t.incomeCents += r.amountCents } else { t.expenseCents += r.amountCents }
        }
        return t
    }

    /// First line of the payment reference ("Verwendungszweck").
    static func firstLine(_ s: String) -> String {
        s.split(whereSeparator: \.isNewline).first.map { String($0).trimmingCharacters(in: .whitespaces) } ?? ""
    }

    static func html(_ report: PrintReport, options: PrintOptions) -> String {
        let printed = rows(report.rows, options: options)
        let groups: [(account: Int64?, rows: [PrintRow])]
        if options.groupByAccount {
            var order: [Int64] = []
            var byAccount: [Int64: [PrintRow]] = [:]
            for r in printed {
                if byAccount[r.accountId] == nil { order.append(r.accountId) }
                byAccount[r.accountId, default: []].append(r)
            }
            // stable, readable order: by account name
            order.sort { (byAccount[$0]?.first?.accountName ?? "") < (byAccount[$1]?.first?.accountName ?? "") }
            groups = order.map { ($0, byAccount[$0] ?? []) }
        } else {
            groups = [(nil, printed)]
        }
        let showAccountColumn = !options.groupByAccount && Set(printed.map(\.accountId)).count > 1
        let currency = printed.first?.currency ?? "EUR"

        var h = """
        <!doctype html><html lang="\(esc(Locale.current.language.languageCode?.identifier ?? "de"))"><head><meta charset="utf-8">
        <title>\(esc(report.title))</title><style>\(css)</style></head><body>
        <header><h1>\(esc(report.title))</h1>
        <p class="meta">\(esc(report.periodLabel)) · \(esc(String(localized: "\(printed.count) Buchungen"))) · \(esc(String(localized: "Erstellt am \(DateText.medium(DateText.iso(report.generatedAt)))")))</p>
        """
        h += accountSummary(report, groups: groups, options: options)
        h += "</header>"

        if printed.isEmpty {
            h += "<p class=\"empty\">\(esc(String(localized: "Keine Buchungen ausgewählt.")))</p>"
        }
        for g in groups where !g.rows.isEmpty {
            let account = g.account.flatMap { id in report.accounts.first { $0.id == id } }
            h += "<section>"
            if options.groupByAccount {
                let name = account?.name ?? g.rows.first?.accountName ?? ""
                h += "<h2>\(esc(name))"
                if let iban = account?.iban, !iban.isEmpty { h += " <span class=\"iban\">\(esc(iban))</span>" }
                h += "</h2>"
            }
            let balances = options.showDailyBalance ? report.dailyBalances[g.account ?? 0] : nil
            h += table(g.rows, options: options, showAccount: showAccountColumn, balances: balances)
            if options.showTotals && options.groupByAccount && groups.count > 1 {
                h += totalsBlock(totals(g.rows), currency: g.rows.first?.currency ?? currency, label: String(localized: "Summe Konto"))
            }
            h += "</section>"
        }
        if options.showTotals && !printed.isEmpty {
            h += totalsBlock(totals(printed), currency: currency, label: String(localized: "Summe"))
        }
        h += "</body></html>"
        return h
    }

    // MARK: - Pieces

    private static func accountSummary(_ report: PrintReport, groups: [(account: Int64?, rows: [PrintRow])],
                                       options: PrintOptions) -> String {
        guard options.showCurrentBalance else { return "" }
        let ids = Set(report.rows.map(\.accountId))
        let accounts = report.accounts.filter { ids.contains($0.id) }.sorted { $0.name < $1.name }
        guard !accounts.isEmpty else { return "" }
        var h = "<ul class=\"balances\">"
        for a in accounts {
            h += "<li><span class=\"name\">\(esc(a.name))</span> <span class=\"value\">\(esc(Money.format(a.balanceCents, currency: a.currency)))</span>"
            h += " <span class=\"asof\">\(esc(String(localized: "Kontostand am \(DateText.medium(a.balanceDate))")))</span></li>"
        }
        h += "</ul>"
        return h
    }

    private static func table(_ rows: [PrintRow], options: PrintOptions, showAccount: Bool, balances: [String: Int64]?) -> String {
        var h = "<table class=\"bookings\"><thead><tr>"
        h += "<th class=\"date\">\(esc(String(localized: "Datum")))</th>"
        if showAccount { h += "<th>\(esc(String(localized: "Konto")))</th>" }
        h += "<th>\(esc(String(localized: "Empfänger / Verwendungszweck")))</th>"
        if options.showCategory { h += "<th>\(esc(String(localized: "Kategorie")))</th>" }
        h += "<th class=\"amount\">\(esc(String(localized: "Betrag")))</th>"
        if balances != nil { h += "<th class=\"amount\">\(esc(String(localized: "Tagessaldo")))</th>" }
        h += "</tr></thead><tbody>"
        for (i, r) in rows.enumerated() {
            h += "<tr>"
            h += "<td class=\"date\">\(esc(DateText.medium(r.date)))</td>"
            if showAccount { h += "<td>\(esc(r.accountName))</td>" }
            let purpose = options.firstLineOnly ? firstLine(r.purpose) : r.purpose
            h += "<td><div class=\"party\">\(esc(r.counterparty))</div>"
            if !purpose.isEmpty { h += "<div class=\"purpose\">\(esc(purpose).replacingOccurrences(of: "\n", with: "<br>"))</div>" }
            if options.showNote && !r.note.isEmpty { h += "<div class=\"note\">\(esc(r.note))</div>" }
            h += "</td>"
            if options.showCategory { h += "<td class=\"category\">\(esc(r.category))</td>" }
            h += "<td class=\"amount\">\(esc(Money.format(r.amountCents, currency: r.currency)))</td>"
            if let balances {
                // End-of-day balance on the day's row closest to the end of that day.
                let next = i + 1 < rows.count ? rows[i + 1].date : nil
                let prev = i > 0 ? rows[i - 1].date : nil
                let edge = options.newestFirst ? prev != r.date : next != r.date
                let value = edge ? balances[r.date].map { Money.format($0, currency: r.currency) } ?? "" : ""
                h += "<td class=\"amount balance\">\(esc(value))</td>"
            }
            h += "</tr>"
        }
        h += "</tbody></table>"
        return h
    }

    private static func totalsBlock(_ t: Totals, currency: String, label: String) -> String {
        """
        <table class="totals"><tbody>
        <tr><td>\(esc(String(localized: "Einnahmen")))</td><td class="amount">\(esc(Money.format(t.incomeCents, currency: currency)))</td></tr>
        <tr><td>\(esc(String(localized: "Ausgaben")))</td><td class="amount">\(esc(Money.format(t.expenseCents, currency: currency)))</td></tr>
        <tr class="net"><td>\(esc(label))</td><td class="amount">\(esc(Money.format(t.netCents, currency: currency)))</td></tr>
        </tbody></table>
        """
    }

    static func esc(_ s: String) -> String {
        var out = ""
        out.reserveCapacity(s.count)
        for c in s {
            switch c {
            case "&": out += "&amp;"
            case "<": out += "&lt;"
            case ">": out += "&gt;"
            case "\"": out += "&quot;"
            case "'": out += "&#39;"
            default: out.append(c)
            }
        }
        return out
    }

    private static let css = """
    @page { margin: 16mm 14mm; }
    body { font: 9.5pt -apple-system, "Helvetica Neue", Helvetica, sans-serif; color: #111; margin: 0; }
    h1 { font-size: 15pt; margin: 0 0 2pt; }
    h2 { font-size: 11pt; margin: 14pt 0 4pt; }
    .meta { color: #555; margin: 0 0 8pt; }
    .iban { font-weight: normal; color: #555; font-size: 9pt; }
    table { width: 100%; border-collapse: collapse; }
    thead { display: table-header-group; }
    tr { page-break-inside: avoid; break-inside: avoid; }
    th { text-align: left; font-weight: 600; border-bottom: 1pt solid #333; padding: 3pt 4pt; }
    td { vertical-align: top; padding: 3pt 4pt; border-bottom: 0.5pt solid #ddd; }
    .date { white-space: nowrap; width: 1%; }
    .amount { text-align: right; white-space: nowrap; font-variant-numeric: tabular-nums; width: 1%; }
    .party { font-weight: 500; }
    .purpose, .category { color: #444; }
    .note { color: #444; font-style: italic; }
    .balance { color: #444; }
    .balances { list-style: none; margin: 0 0 6pt; padding: 0; }
    .balances li { margin: 1pt 0; }
    .balances .name { font-weight: 500; }
    .balances .value { font-variant-numeric: tabular-nums; margin-left: 6pt; }
    .balances .asof { color: #555; margin-left: 6pt; }
    .totals { width: auto; margin: 10pt 0 0 auto; page-break-inside: avoid; break-inside: avoid; }
    .totals td { border: none; padding: 1pt 0 1pt 16pt; }
    .totals .net td { border-top: 1pt solid #333; font-weight: 600; }
    .empty { color: #555; }
    """
}
