import Foundation
import SwiftUI
import UniformTypeIdentifiers
import GiroCrabCore
#if os(macOS)
import AppKit
#endif

extension AppModel {
    // MARK: - Printing (GitHub #53) & Umsatz-Export (#53 PDF / #372 CAMT)

    /// Filtered bookings without the list page limit (for CSV/XML export builders).
    func exportBookings() -> [Booking] {
        guard let vault else { return [] }
        var filter = currentFilter
        filter.limit = 0
        return (try? vault.listTransactions(filter: filter).transactions) ?? []
    }

    /// Writes export bytes: either straight into Downloads (or last folder), or via save panel.
    func presentExportFile(data: Data, format: BookingExportFormat, filename: String, askWhere: Bool = false) {
        #if os(macOS)
        if !askWhere {
            let url = ExportDirectory.preferred.appendingPathComponent(filename)
            do {
                try data.write(to: url, options: .atomic)
                ExportDirectory.remember(fileURL: url)
                toast = String(localized: "Gespeichert: \(filename)")
            } catch {
                showError(String(localized: "Export fehlgeschlagen"), error.localizedDescription)
            }
            return
        }
        let panel = NSSavePanel()
        panel.canCreateDirectories = true
        panel.isExtensionHidden = false
        panel.nameFieldStringValue = filename
        panel.allowedContentTypes = [format.contentType]
        panel.directoryURL = ExportDirectory.preferred
        panel.title = String(localized: "Umsätze sichern")
        panel.prompt = String(localized: "Sichern")
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try data.write(to: url, options: .atomic)
            ExportDirectory.remember(fileURL: url)
        } catch {
            showError(String(localized: "Export fehlgeschlagen"), error.localizedDescription)
        }
        #else
        exportFile = ExportFileDocument(data: data)
        exportContentType = format.contentType
        exportFilename = filename
        showExportExporter = true
        #endif
    }

    /// Saves several files into Downloads (or last folder) without a panel.
    func presentExportFiles(_ files: [(data: Data, filename: String)]) {
        guard !files.isEmpty else { return }
        #if os(macOS)
        var saved = 0
        var lastURL: URL?
        for file in files {
            let url = ExportDirectory.preferred.appendingPathComponent(file.filename)
            do {
                try file.data.write(to: url, options: .atomic)
                lastURL = url
                saved += 1
            } catch {
                showError(String(localized: "Export fehlgeschlagen"), error.localizedDescription)
                return
            }
        }
        if let lastURL { ExportDirectory.remember(fileURL: lastURL) }
        toast = saved == 1
            ? String(localized: "Gespeichert: \(files[0].filename)")
            : String(localized: "\(saved) Dateien in Downloads gespeichert")
        #else
        // iOS: fall back to the first file via the share sheet.
        if let first = files.first {
            presentExportFile(data: first.data, format: .pdf, filename: first.filename, askWhere: true)
        }
        #endif
    }

    /// Everything the list shows (account, period, search, scope, status) without the list limit.
    func printSource() -> PrintReport? {
        exportSource(accountIDs: nil, period: .current, bookingIDs: nil, options: nil)
    }

    /// Export/print rows for explicit accounts + period (export sheet does not mutate the list).
    func exportSource(accountIDs: [Int64]?, period: ExportPeriodPreset, bookingIDs: Set<Int64>?,
                      options: ExportOptions? = nil) -> PrintReport? {
        guard let vault else { return nil }
        var filter = currentFilter
        filter.limit = 0
        if let accountIDs, !accountIDs.isEmpty {
            filter.accountIds = accountIDs
        }
        applyExportPeriod(period, to: &filter, options: options)
        do {
            var rows = try vault.listTransactions(filter: filter).transactions.map(PrintRow.init)
            if let bookingIDs {
                rows = rows.filter { bookingIDs.contains($0.id) }
            }
            let accounts = allAccounts.map {
                PrintAccount(id: $0.id, name: $0.name, iban: $0.iban, balanceCents: $0.balanceCents,
                             balanceDate: $0.balanceDate, currency: $0.currency, owner: $0.owner)
            }
            let periodLabel = exportPeriodLabel(period, bookingIDs: bookingIDs, options: options)
            let title: String = {
                if let accountIDs, accountIDs.count == 1,
                   let a = allAccounts.first(where: { $0.id == accountIDs[0] }) {
                    return a.name
                }
                return self.title
            }()
            var report = PrintReport(title: title, periodLabel: periodLabel, rows: rows, accounts: accounts)
            switch directionFilter {
            case .all: break
            case .income: report.filterNotes = [PrintDirection.income.title]
            case .expense: report.filterNotes = [PrintDirection.expenses.title]
            }
            return report
        } catch {
            showError(String(localized: "Export nicht möglich"), error.localizedDescription)
            return nil
        }
    }

    private func exportPeriodLabel(_ period: ExportPeriodPreset, bookingIDs: Set<Int64>?,
                                   options: ExportOptions?) -> String {
        if bookingIDs != nil { return String(localized: "Auswahl") }
        // „Neue Umsätze“ ignores the period
        if period == .current { return sidebarSelection == .newBookings ? Period.all.title : periodTitle }
        if period == .custom, let o = options, !o.customFrom.isEmpty {
            return "\(PrintReportBuilder.statementDate(o.customFrom)) – \(PrintReportBuilder.statementDate(o.customTo))"
        }
        if period == .sinceSalary, let c = salaryCycle {
            return String(localized: "Seit Gehalt am \(PrintReportBuilder.statementDate(c.lastDate))")
        }
        return period.title
    }

    /// Bookings for CAMT/CSV builders with the same account/period override as `exportSource`.
    func exportBookings(accountIDs: [Int64]?, period: ExportPeriodPreset, bookingIDs: Set<Int64>?,
                        options: ExportOptions? = nil) -> [Booking] {
        guard let vault else { return [] }
        var filter = currentFilter
        filter.limit = 0
        if let accountIDs, !accountIDs.isEmpty { filter.accountIds = accountIDs }
        applyExportPeriod(period, to: &filter, options: options)
        var rows = (try? vault.listTransactions(filter: filter).transactions) ?? []
        if let bookingIDs { rows = rows.filter { bookingIDs.contains($0.id) } }
        return rows
    }

    private func applyExportPeriod(_ period: ExportPeriodPreset, to filter: inout BookingFilter, options: ExportOptions?) {
        let salary = salaryCycle?.lastDate
        let from = options?.customFrom ?? ""
        let to = options?.customTo ?? ""
        guard let range = period.range(customFrom: from, customTo: to, salaryFrom: salary) else { return }
        // `.all` → open range; `.current` returns nil above and keeps list dates.
        filter.dateFrom = range.0
        filter.dateTo = range.1
    }

    /// End-of-day balances since `from` for each account (key = id) and combined (key 0).
    func printDailyBalances(accountIds: [Int64], from: String) -> [Int64: [String: Int64]] {
        guard let vault, !accountIds.isEmpty else { return [:] }
        let today = Calendar.current.startOfDay(for: Date())
        let start = DateText.date(from) ?? today
        let days = UInt32(max(1, (Calendar.current.dateComponents([.day], from: start, to: today).day ?? 0) + 1))
        func map(_ ids: [Int64]) -> [String: Int64] {
            let points = (try? vault.balanceHistory(accountIds: ids, days: days)) ?? []
            return Dictionary(points.map { ($0.date, $0.balanceCents) }, uniquingKeysWith: { _, b in b })
        }
        var out: [Int64: [String: Int64]] = [0: map(accountIds)]
        for id in accountIds { out[id] = map([id]) }
        return out
    }
}
