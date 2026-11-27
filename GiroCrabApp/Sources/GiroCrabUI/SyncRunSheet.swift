import SwiftUI
import GiroCrabCore

extension EnvironmentValues {
    /// A sync prompt (PIN, setup, TAN) shown inside the sync window: the window has the
    /// run's progress and "Alle abbrechen", so the prompt leaves both out.
    @Entry var inSyncRunWindow = false
}

/// Sync window of "Alle abrufen" over several logins (design: docs/ABRUF_UI.md).
///
/// Stays open for the whole run: every login with its status, how many are still to come,
/// and below the list the prompt of the running login (PIN, setup, TAN, approval).
/// Browser logins open as a sheet over it. Closed with "Fertig" once the run is over.
struct SyncRunSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.textScale) private var textScale
    @State private var listHeight: CGFloat = 0

    var body: some View {
        if let run = model.syncRun {
            VStack(spacing: 0) {
                header(run)
                    .padding(.horizontal, 24)
                    .padding(.top, 20)
                    .padding(.bottom, 12)
                ScrollView {
                    VStack(spacing: 0) {
                        ForEach(run.entries) { entry in
                            SyncRunRow(entry: entry, runFinished: run.isFinished)
                            if entry.id != run.entries.last?.id { Divider().padding(.leading, 60) }
                        }
                    }
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { listHeight = $0 }
                }
                .frame(height: min(max(listHeight, 44), listCap))
                Divider()
                current(run)
                    .frame(maxWidth: .infinity)
                Divider()
                footer(run)
                    .padding(.horizontal, 24)
                    .padding(.vertical, 14)
            }
            .macFrame(minWidth: 480)
            .environment(\.inSyncRunWindow, true)
            // Browser logins of the run (Trade Republic, FNZ, a bank asking for one) over the window.
            .capturableSheet(item: Binding(get: { model.webLoginConnection },
                                           set: { model.webLoginConnection = $0 })) { c in
                WebLoginContent(connection: c).environment(model)
            }
            .modifier(ModelErrorAlert(enabled: true))
        }
    }

    /// Room for the list; the prompt below needs the rest (iPhone: half the screen at most).
    private var listCap: CGFloat {
        #if os(iOS)
        240 * textScale
        #else
        (model.syncRunPromptShown ? 220 : 360) * textScale
        #endif
    }

    private func header(_ run: SyncRun) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Text(run.isFinished ? "Abruf abgeschlossen" : "Abruf läuft")
                    .appFont(.title3, weight: .semibold)
                Spacer()
                Text(run.isFinished ? Self.counts(run) : String(localized: "noch \(run.remaining) von \(run.total)"))
                    .appFont(.callout, monospacedDigit: true)
                    .foregroundStyle(Theme.secondary)
            }
            ProgressView(value: Double(run.completed), total: Double(max(run.total, 1)))
                .tint(run.isFinished && run.failedCount == 0 ? Theme.inflow : Theme.accent)
                .accessibilityLabel(Text("Fortschritt des Abrufs"))
                .accessibilityValue(Text("\(run.completed) von \(run.total) Bankzugängen erledigt"))
        }
    }

    /// "4 abgerufen · 1 übersprungen · 1 Fehler", only what occurred.
    static func counts(_ run: SyncRun) -> String {
        var parts = [String(localized: "\(run.doneCount) abgerufen")]
        if run.skippedCount > 0 { parts.append(String(localized: "\(run.skippedCount) übersprungen")) }
        if run.failedCount > 0 { parts.append(String(localized: "\(run.failedCount) mit Fehler")) }
        return parts.joined(separator: " · ")
    }

    /// The prompt of the running login, or what happens without one.
    @ViewBuilder
    private func current(_ run: SyncRun) -> some View {
        if let login = model.pendingLogin {
            PinSheet(connection: login.connection).id(login.id)
        } else if let request = model.setupRequest {
            BankSetupSheet(request: request).id(request.id)
        } else if let active = model.activeChallenge {
            TwoFactorSheet().id(active.id)
        } else if run.isFinished {
            Label(SyncRunSheet.finishedNote(run), systemImage: run.failedCount == 0 ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .appFont(.callout)
                .foregroundStyle(run.failedCount == 0 ? Theme.inflow : Theme.secondary)
                .padding(24)
        } else {
            HStack(spacing: 10) {
                ProgressView().controlSize(.small)
                if let entry = run.running {
                    Text(model.connectionName(entry))
                        .appFont(.callout, weight: .medium)
                        .sensitive()
                    Text("wird abgerufen …").appFont(.callout).foregroundStyle(Theme.secondary)
                } else {
                    Text("Nächster Bankzugang …").appFont(.callout).foregroundStyle(Theme.secondary)
                }
            }
            .padding(24)
        }
    }

    static func finishedNote(_ run: SyncRun) -> String {
        run.failedCount == 0
            ? String(localized: "Alle Bankzugänge sind durch.")
            : String(localized: "Fehlgeschlagene Bankzugänge lassen sich einzeln erneut abrufen.")
    }

    @ViewBuilder
    private func footer(_ run: SyncRun) -> some View {
        HStack {
            if run.isFinished {
                Spacer()
                Button { model.closeSyncRun() } label: { Text("Fertig").frame(minWidth: 80) }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
            } else {
                Button("Alle abbrechen") { model.cancelAllSync() }
                    .help("Den Abruf für alle restlichen Bankzugänge beenden.")
                Spacer()
                // a prompt has its own "Überspringen"; without one, the running sync is skipped here
                if !model.syncRunPromptShown, run.running != nil {
                    Button("Überspringen") { model.skipCurrentSync() }
                        .help("Diesen Bankzugang auslassen und mit dem nächsten weitermachen.")
                }
            }
        }
    }
}

/// One login of the run: logo, name, status line, and for a failed or skipped one
/// "Erneut versuchen" (plus, once the run is over, what else its error offers).
private struct SyncRunRow: View {
    @Environment(AppModel.self) private var model
    let entry: SyncRunEntry
    let runFinished: Bool

    private var connection: BankConnection? { model.connections.first { $0.id == entry.connectionID } }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            if let c = connection {
                SymbolTile(symbol: c.sfSymbol, tint: c.tint, size: 24)
            } else {
                SymbolTile(symbol: "building.columns", tint: Theme.secondary, size: 24)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(model.connectionName(entry))
                    .appFont(.body, weight: entry.status == .running ? .semibold : .regular)
                    .lineLimit(1)
                    .sensitive()
                statusLine
                if !actions.isEmpty {
                    HStack(spacing: 12) {
                        ForEach(Array(actions.enumerated()), id: \.offset) { _, a in
                            Button(a.title, action: a.run)
                                .buttonStyle(.borderless)
                                .appFont(.caption)
                        }
                    }
                    .padding(.top, 2)
                }
            }
            Spacer(minLength: 8)
            statusIcon
                .frame(width: 20, height: 20)
        }
        .padding(.horizontal, 24)
        .padding(.vertical, 8)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private var statusLine: some View {
        switch entry.status {
        case .waiting:
            Text("Wartet").appFont(.caption).foregroundStyle(Theme.tertiary)
        case .running:
            Text(model.syncRunPromptText(for: entry.connectionID))
                .appFont(.caption).foregroundStyle(Theme.accent)
        case .done(let imported, let detail):
            VStack(alignment: .leading, spacing: 1) {
                if let imported {
                    Text("\(imported) neue Buchungen").appFont(.caption, monospacedDigit: true)
                        .foregroundStyle(Theme.secondary)
                        .sensitive(.amounts)
                }
                if let detail {
                    // may name new accounts
                    Text(detail).appFont(.caption).foregroundStyle(Theme.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .sensitive()
                }
            }
        case .skipped(let reason):
            Text(reason).appFont(.caption).foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: false, vertical: true)
        case .failed(let message):
            Text(message).appFont(.caption).foregroundStyle(Theme.negative)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
    }

    @ViewBuilder
    private var statusIcon: some View {
        switch entry.status {
        case .waiting:
            Image(systemName: "circle").foregroundStyle(Theme.tertiary)
                .accessibilityLabel(Text("Wartet"))
        case .running:
            ProgressView().controlSize(.small)
                .accessibilityLabel(Text("Läuft"))
        case .done:
            Image(systemName: "checkmark.circle.fill").foregroundStyle(Theme.inflow)
                .accessibilityLabel(Text("Fertig"))
        case .skipped:
            Image(systemName: "forward.end.circle").foregroundStyle(Theme.secondary)
                .accessibilityLabel(Text("Übersprungen"))
        case .failed:
            Image(systemName: "xmark.octagon.fill").foregroundStyle(Theme.negative)
                .accessibilityLabel(Text("Fehler"))
        }
    }

    private var actions: [(title: String, run: () -> Void)] {
        var out: [(title: String, run: () -> Void)] = []
        let id = entry.connectionID
        switch entry.status {
        case .failed:
            out.append((String(localized: "Erneut versuchen"), { model.retryInSyncRun(id) }))
            if runFinished {
                for (i, a) in (model.syncRunActions[id] ?? []).enumerated() {
                    out.append((a.title, { model.runSyncRunAction(id, i) }))
                }
            }
        case .skipped(let reason) where AppModel.retryableSkip(reason):
            out.append((String(localized: "Erneut versuchen"), { model.retryInSyncRun(id) }))
        default:
            break
        }
        return connection == nil ? [] : out
    }
}

/// The in-app browser login of a connection (from the main window or the sync window).
struct WebLoginContent: View {
    let connection: BankConnection

    var body: some View {
        let c = connection
        if c.connector == .tradeRepublic { WebLoginSheet(connection: c) }
        else if c.connector == .amazonOrders { AmazonWebSheet(connection: c) }
        else if c.connector == .fnz { FnzWebSheet(connection: c) }
        else if c.connector == .equatePlus { EquatePlusWebSheet(connection: c) }
        else if c.connector == .quirion { QuirionWebSheet(connection: c) }
        else if c.connector == .sharinbox { SharinboxWebSheet(connection: c) }
        else if c.connector == .axaShareplan { AxaShareplanWebSheet(connection: c) }
        else if c.connector == .unionInvestment { UnionInvestmentWebSheet(connection: c) }
        else if c.connector == .airPlus { AirPlusWebSheet(connection: c) }
        else if c.connector == .berghausCie { BerghausCieWebSheet(connection: c) }
        else if c.connector == .schwaebischHall { SchwaebischHallWebSheet(connection: c) }
        else { PortalWebSheet(connection: c) }
    }
}

/// The model's error alert (title, retry, alternatives). On the main window, and on the
/// sync window while that is open: an alert of the main window would not show over it.
struct ModelErrorAlert: ViewModifier {
    @Environment(AppModel.self) private var model
    let enabled: Bool

    func body(content: Content) -> some View {
        content.alert(model.errorTitle ?? String(localized: "Etwas ist schiefgelaufen"),
                      isPresented: Binding(get: { enabled && model.errorMessage != nil },
                                           set: { if !$0 { model.errorMessage = nil } })) {
            if let retry = model.errorRetry {
                Button("Erneut versuchen") { model.errorOnClose = nil; model.errorMessage = nil; retry() }
            }
            ForEach(Array(model.errorAlternatives.enumerated()), id: \.offset) { _, alt in
                Button(alt.title) { model.errorOnClose = nil; model.errorMessage = nil; alt.action() }
            }
            if let close = model.errorCloseTitle {
                Button(close, role: .cancel) { model.errorMessage = nil }
            } else {
                Button("OK", role: .cancel) { model.errorMessage = nil }
            }
        } message: {
            Text(model.errorMessage ?? "")
        }
    }
}
