import SwiftUI
import WebKit
#if os(macOS)
import AppKit
#else
import UIKit
#endif

// MARK: - Print dialog (GitHub #53, part 2)

/// Options on the left, the bookings to print (with checkboxes) on the right.
/// "Drucken …" hands the HTML report to the system print panel, which also
/// offers "Als PDF sichern".
struct PrintSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var options = PrintOptions.load()
    @State private var report: PrintReport?
    @State private var selected: Set<Int64> = []

    private var visibleRows: [PrintRow] {
        PrintReportBuilder.rows(report?.rows ?? [], options: options)
    }

    private var selectedRows: [PrintRow] {
        visibleRows.filter { selected.contains($0.id) }
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle("Drucken")
                #if os(iOS)
                .navigationBarTitleDisplayMode(.inline)
                #endif
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Abbrechen") { dismiss() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Drucken …") { print() }
                            .disabled(selectedRows.isEmpty)
                    }
                }
        }
        #if os(macOS)
        .frame(minWidth: 820, idealWidth: 920, minHeight: 520, idealHeight: 640)
        #endif
        .onAppear {
            report = model.printSource()
            selected = Set(report?.rows.map(\.id) ?? [])
        }
        .onChange(of: options) { _, o in o.save() }
    }

    @ViewBuilder
    private var content: some View {
        #if os(macOS)
        HStack(spacing: 0) {
            optionsForm.frame(width: 300)
            Divider()
            bookingList
        }
        #else
        Form {
            optionSections
            Section {
                selectionHeader
                ForEach(visibleRows) { row($0) }
            } header: {
                Text("Buchungen")
            }
        }
        #endif
    }

    // MARK: Options

    private var optionsForm: some View {
        Form { optionSections }
            .formStyle(.grouped)
    }

    @ViewBuilder
    private var optionSections: some View {
        Section("Auswahl") {
            Picker("Buchungen", selection: $options.direction) {
                ForEach(PrintDirection.allCases) { Text($0.title).tag($0) }
            }
            Picker("Reihenfolge", selection: $options.newestFirst) {
                Text("Neueste zuerst").tag(true)
                Text("Älteste zuerst").tag(false)
            }
            LabeledContent("Zeitraum", value: model.periodTitle)
        }
        Section {
            Toggle("Kategorie", isOn: $options.showCategory)
            Toggle("Notizen", isOn: $options.showNote)
            Toggle("Nur erste Zeile des Verwendungszwecks", isOn: $options.firstLineOnly)
            Toggle("Summe der Ein- und Ausgänge", isOn: $options.showTotals)
            Toggle("Nach Konto gruppieren", isOn: $options.groupByAccount)
            Toggle("Aktueller Kontostand", isOn: $options.showCurrentBalance)
            Toggle("Tagessaldo", isOn: $options.showDailyBalance)
        } header: {
            Text("Inhalt")
        } footer: {
            if options.showDailyBalance {
                Text("Der Tagessaldo ist der Kontostand am Ende des Tages, unabhängig von der Auswahl.")
            }
        }
    }

    // MARK: Bookings

    private var bookingList: some View {
        VStack(spacing: 0) {
            selectionHeader
                .padding(.horizontal, 16).padding(.vertical, 10)
            Divider()
            if visibleRows.isEmpty {
                ContentUnavailableView("Keine Buchungen", systemImage: "doc.text",
                                       description: Text("Im gewählten Zeitraum und Filter gibt es nichts zu drucken."))
            } else {
                List(visibleRows) { row($0) }
                    .listStyle(.inset)
            }
        }
    }

    private var selectionHeader: some View {
        let totals = PrintReportBuilder.totals(selectedRows)
        return HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(selectedRows.count) von \(visibleRows.count) ausgewählt")
                    .font(.headline)
                Text("Einnahmen \(Money.format(totals.incomeCents)) · Ausgaben \(Money.format(totals.expenseCents))")
                    .font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
            Spacer()
            Button("Alle") { selected.formUnion(visibleRows.map(\.id)) }
                .disabled(selectedRows.count == visibleRows.count)
            Button("Keine") { selected.subtract(visibleRows.map(\.id)) }
                .disabled(selectedRows.isEmpty)
        }
    }

    private func row(_ r: PrintRow) -> some View {
        Toggle(isOn: Binding(
            get: { selected.contains(r.id) },
            set: { on in if on { selected.insert(r.id) } else { selected.remove(r.id) } }
        )) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(r.counterparty).lineLimit(1)
                    Text("\(DateText.medium(r.date)) · \(r.accountName)")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer()
                Text(Money.format(r.amountCents, currency: r.currency))
                    .monospacedDigit()
                    .foregroundStyle(r.amountCents > 0 ? Theme.inflow : .primary)
            }
        }
        #if os(macOS)
        .toggleStyle(.checkbox)
        #endif
    }

    // MARK: Output

    private func print() {
        guard var r = report else { return }
        r.rows = selectedRows
        r.generatedAt = Date()
        if options.showDailyBalance, let earliest = r.rows.map(\.date).min() {
            let ids = Array(Set(r.rows.map(\.accountId))).sorted()
            r.dailyBalances = model.printDailyBalances(accountIds: ids, from: earliest)
        }
        let html = PrintReportBuilder.html(r, options: options)
        let jobName = "GiroCrab – \(r.title) – \(r.periodLabel)"
        #if os(macOS)
        // The print panel opens as a sheet on the main window; close ours first.
        dismiss()
        DispatchQueue.main.async { HTMLPrinter.shared.print(html: html, jobName: jobName) }
        #else
        let controller = UIPrintInteractionController.shared
        let info = UIPrintInfo(dictionary: nil)
        info.jobName = jobName
        info.outputType = .general
        controller.printInfo = info
        controller.printFormatter = UIMarkupTextPrintFormatter(markupText: html)
        controller.present(animated: true)
        #endif
    }
}

#if os(macOS)
/// Loads the report into an off-screen WKWebView and runs the system print panel.
@MainActor
final class HTMLPrinter: NSObject, WKNavigationDelegate {
    static let shared = HTMLPrinter()
    private var webView: WKWebView?
    private var jobName = ""

    func print(html: String, jobName: String) {
        self.jobName = jobName
        // A4 width at 72 dpi; WebKit paginates to the paper size chosen in the panel.
        let web = WKWebView(frame: NSRect(x: 0, y: 0, width: 595, height: 842))
        web.navigationDelegate = self
        webView = web
        web.loadHTMLString(html, baseURL: nil)
    }

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        MainActor.assumeIsolated { runPrint(webView) }
    }

    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        MainActor.assumeIsolated { self.webView = nil }
    }

    private func runPrint(_ web: WKWebView) {
        let info = NSPrintInfo.shared.copy() as! NSPrintInfo
        info.horizontalPagination = .fit
        info.verticalPagination = .automatic
        info.isHorizontallyCentered = false
        info.isVerticallyCentered = false
        info.topMargin = 0; info.bottomMargin = 0; info.leftMargin = 0; info.rightMargin = 0 // CSS @page margins
        let op = web.printOperation(with: info)
        op.jobTitle = jobName
        op.showsPrintPanel = true
        op.showsProgressPanel = true
        op.printPanel.options.formUnion([.showsPaperSize, .showsOrientation, .showsScaling, .showsPreview])
        // WKWebView's print view needs a frame, or the output is blank.
        op.view?.frame = web.bounds
        if let window = NSApp.keyWindow ?? NSApp.mainWindow {
            op.runModal(for: window, delegate: self, didRun: #selector(printDidRun(_:success:contextInfo:)), contextInfo: nil)
        } else {
            op.run()
            webView = nil
        }
    }

    @objc private func printDidRun(_ op: NSPrintOperation, success: Bool, contextInfo: UnsafeMutableRawPointer?) {
        webView = nil
    }
}
#endif
