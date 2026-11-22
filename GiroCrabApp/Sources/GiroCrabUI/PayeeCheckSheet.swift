import SwiftUI
import GiroCrabCore

/// Result of the recipient name check (Verification of Payee) before a
/// transfer: the user decides whether to send it anyway.
struct PayeeCheckSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.textScale) private var textScale
    /// Sizes the macOS sheet to its form, like the transfer review (#879).
    @State private var formFit = FormFit()

    private var fittedHeight: CGFloat? {
        #if os(macOS)
        return formFit.height(min: 300, max: (NSScreen.main?.visibleFrame.height ?? 800) - 120)
        #else
        return nil
        #endif
    }

    // Same structure as the review step of the transfer sheet (#1564): title
    // bar, grouped rows with the warning as footer, button bar.
    var body: some View {
        if let pending = model.payeeCheck {
            let c = pending.check
            VStack(spacing: 0) {
                header(pending)
                Divider()
                Form {
                    Section {
                        row("Eingegeben", c.enteredName)
                        if let bank = c.bankName { row("Kontoinhaber", bank) }
                        row("IBAN", formatIBAN(c.iban), monospaced: true)
                    } footer: {
                        VStack(alignment: .leading, spacing: 8) {
                            note(explanation(c), symbol: "exclamationmark.shield")
                            if let notice = c.notice { note(notice, symbol: "building.columns") }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .formStyle(.grouped)
                .reportsFormFit($formFit)
                Divider()
                buttons(pending).padding(14)
            }
            .macFrame(width: 520, height: fittedHeight == nil ? 400 : nil)
            .frame(height: fittedHeight)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { formFit.total = $0 }
        }
    }

    private func header(_ pending: PendingPayeeCheck) -> some View {
        HStack(spacing: 12) {
            Image(systemName: symbol(pending.check.result))
                .appFont(.title2)
                .foregroundStyle(tint(pending.check.result))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title(pending.check.result)).appFont(.title2, weight: .semibold)
                Text("Empfängerüberprüfung · \(pending.connection.displayName)")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
                    .sensitive() // the user's own name for the connection
            }
            Spacer()
        }
        .padding(.horizontal, 20).padding(.vertical, 14)
    }

    // HIG: Cancel on the left, the safe default (Return) on the far right;
    // sending despite a mismatch is destructive and never the default.
    private func buttons(_ pending: PendingPayeeCheck) -> some View {
        let c = pending.check
        return HStack {
            Button("Abbrechen", role: .cancel) { model.cancelPayee() }
                .keyboardShortcut(.cancelAction)
            Spacer()
            if pending.busy { ProgressView().controlSize(.small) }
            if c.result == .match {
                Button("Weiter zur Freigabe") { model.confirmPayee() }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(pending.busy)
            } else {
                Button("Trotzdem senden", role: .destructive) { model.confirmPayee() }
                    .disabled(pending.busy)
                if c.result == .closeMatch, c.bankName != nil {
                    Button("Namen übernehmen") { model.cancelPayee(useBankName: true) }
                        .keyboardShortcut(.defaultAction)
                        .buttonStyle(.borderedProminent)
                        .disabled(pending.busy)
                }
            }
        }
    }

    /// A footer line with its symbol in a fixed column, so both lines start at the same edge.
    private func note(_ text: String, symbol: String) -> some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: symbol).frame(width: 20 * textScale)
        }
    }

    /// Label + value like `reviewRow` in the transfer sheet; every value is personal.
    private func row(_ label: LocalizedStringKey, _ value: String, monospaced: Bool = false) -> some View {
        LabeledContent(label) {
            Text(value)
                .appFont(.body, design: monospaced ? .monospaced : .default)
                .multilineTextAlignment(.trailing)
                .textSelection(.enabled)
                .sensitive()
        }
    }

    private func symbol(_ r: PayeeMatch) -> String {
        switch r {
        case .match: return "checkmark.seal.fill"
        case .closeMatch: return "exclamationmark.triangle.fill"
        case .noMatch: return "xmark.octagon.fill"
        case .notPossible: return "questionmark.circle.fill"
        }
    }

    private func tint(_ r: PayeeMatch) -> Color {
        switch r {
        case .match: return Theme.inflow
        case .closeMatch: return Theme.warning
        case .noMatch: return Theme.negative
        case .notPossible: return .secondary
        }
    }

    private func title(_ r: PayeeMatch) -> String {
        switch r {
        case .match: return String(localized: "Name stimmt überein")
        case .closeMatch: return String(localized: "Name stimmt nur teilweise überein")
        case .noMatch: return String(localized: "Name stimmt nicht überein")
        case .notPossible: return String(localized: "Name konnte nicht geprüft werden")
        }
    }

    private func explanation(_ c: PayeeCheck) -> String {
        switch c.result {
        case .match:
            return String(localized: "Die Bank verlangt trotzdem eine ausdrückliche Bestätigung, bevor sie die Freigabe anfordert.")
        case .closeMatch:
            return String(localized: "Der eingegebene Name weicht vom Namen des Kontoinhabers ab. Übernehmen Sie den Namen, wenn das der richtige Empfänger ist.")
        case .noMatch:
            return String(localized: "Das Konto gehört nicht zum eingegebenen Namen. Wenn Sie trotzdem senden, kann das Geld beim falschen Empfänger landen und lässt sich womöglich nicht zurückholen.")
        case .notPossible:
            let reason = c.reason.map { " (\($0))" } ?? ""
            return String(localized: "Die Empfängerbank konnte den Namen nicht prüfen\(reason). Senden Sie nur, wenn Sie sich bei IBAN und Empfänger sicher sind.")
        }
    }
}
