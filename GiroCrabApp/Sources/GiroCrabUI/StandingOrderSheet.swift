import SwiftUI
import GiroCrabCore

/// Dauerauftrag anlegen, ändern oder löschen (GitHub #496): a form, then a read-only
/// review, then the bank's TAN request (`TwoFactorSheet`). A deletion starts at the review.
struct StandingOrderSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    @State private var reviewing = false

    private var draft: StandingDraft { model.standingDraft }
    private var isDelete: Bool { draft.action == .delete }
    private var account: Account? { model.allAccounts.first { $0.id == draft.accountID } }
    private var pinStored: Bool {
        guard let id = account?.connectionId else { return false }
        return model.isDemo || (model.connectionStatus(id)?.hasPin ?? false)
    }

    private var title: LocalizedStringKey {
        switch draft.action {
        case .create: reviewing ? "Dauerauftrag prüfen" : "Neuer Dauerauftrag"
        case .change: reviewing ? "Änderung prüfen" : "Dauerauftrag ändern"
        case .delete: "Dauerauftrag löschen"
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            Text(title).appFont(.title2, weight: .semibold)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 20).padding(.vertical, 14)
            Divider()
            if reviewing || isDelete { review } else { editor }
            if let e = model.transferError {
                Label(e, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red).appFont(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20).padding(.vertical, 8)
            }
            Divider()
            buttons.padding(14)
        }
        #if os(macOS)
        .frame(width: 520 * textScale)
        #endif
        .task {
            reviewing = reviewing || isDelete
            if model.options.demoSheet == "standing-order-confirm" { reviewing = true } // screenshots
        }
    }

    // MARK: editor

    private var intervalText: String {
        let n = draft.interval
        if draft.unit == "W" { return n == 1 ? String(localized: "1 Woche") : String(localized: "\(n) Wochen") }
        return n == 1 ? String(localized: "1 Monat") : String(localized: "\(n) Monate")
    }

    private var editor: some View {
        @Bindable var model = model
        return Form {
            Section {
                Picker("Von Konto", selection: $model.standingDraft.accountID) {
                    ForEach(model.standingOrderAccounts) { a in
                        Text("\(a.name) · \(formatIBAN(a.iban))").tag(Optional(a.id))
                    }
                }
                .disabled(draft.action == .change)
            }
            Section {
                TextField("Empfänger", text: $model.standingDraft.name, prompt: Text("Name"))
                    .sensitive()
                HStack {
                    TextField("IBAN", text: $model.standingDraft.iban, prompt: Text("DE00 …"))
                        .appFont(.body, design: .monospaced)
                        .sensitive()
                    if !draft.iban.isEmpty && !draft.ibanValid {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.negative)
                            .help("IBAN ungültig (Prüfsumme/Länge)")
                            .accessibilityLabel("IBAN ungültig")
                    }
                }
                TextField("Betrag", text: $model.standingDraft.amountText, prompt: Text("0,00"))
                    .multilineTextAlignment(.trailing).appFont(.body, monospacedDigit: true)
                    .sensitive()
                TextField("Verwendungszweck", text: $model.standingDraft.purpose, prompt: Text("optional"))
                    .sensitive()
            }
            Section {
                Picker("Rhythmus", selection: $model.standingDraft.unit) {
                    Text("Monate").tag("M")
                    Text("Wochen").tag("W")
                }
                Stepper(value: $model.standingDraft.interval, in: 1...99) {
                    HStack {
                        Text("Alle")
                        Spacer()
                        Text(intervalText)
                            .monospacedDigit().foregroundStyle(Theme.secondary)
                    }
                }
                if draft.unit == "W" {
                    Picker("Wochentag", selection: $model.standingDraft.execDay) {
                        ForEach(1...7, id: \.self) { Text(StandingDraft.weekdayName($0)).tag($0) }
                    }
                } else {
                    Picker("Ausführungstag", selection: $model.standingDraft.execDay) {
                        ForEach(1...30, id: \.self) { Text("\($0).").tag($0) }
                        Text("Letzter Tag des Monats").tag(31)
                    }
                }
                DatePicker("Erste Ausführung", selection: $model.standingDraft.firstDate,
                           in: StandingDraft.tomorrow..., displayedComponents: .date)
                Toggle("Mit Enddatum", isOn: $model.standingDraft.hasEnd)
                if draft.hasEnd {
                    DatePicker("Letzte Ausführung", selection: $model.standingDraft.lastDate,
                               in: draft.firstDate..., displayedComponents: .date)
                }
            }
        }
        .formStyle(.grouped)
        .frame(minHeight: 420)
    }

    // MARK: review

    private var review: some View {
        VStack(alignment: .leading, spacing: 12) {
            if isDelete {
                Label("Der Dauerauftrag wird bei der Bank gelöscht. Danach werden keine weiteren Zahlungen mehr ausgeführt.",
                      systemImage: "exclamationmark.triangle")
                    .appFont(.callout).foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: 10) {
                Text(Money.format(draft.cents ?? 0))
                    .appFont(.title, weight: .semibold, monospacedDigit: true).fixedSize().sensitive()
                Text("\(draft.rhythmText) · \(draft.execDayText)").appFont(.callout).foregroundStyle(Theme.secondary)
                line("Von Konto", account.map { "\($0.name) · \(formatIBAN($0.iban))" } ?? "")
                line("Empfänger", draft.name)
                line("IBAN", draft.iban, mono: true)
                if !draft.purpose.isEmpty { line("Verwendungszweck", draft.purpose) }
                line("Erste Ausführung", DateText.medium(DateText.iso(draft.firstDate)), sensitive: false)
                line("Letzte Ausführung",
                     draft.hasEnd ? DateText.medium(DateText.iso(draft.lastDate)) : String(localized: "bis auf Widerruf"),
                     sensitive: false)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .card()
            if !pinStored {
                SecureField("Banking-PIN", text: Bindable(model).standingDraft.pin, prompt: Text("wird nicht gespeichert"))
                    .frame(maxWidth: 360 * textScale)
            }
            if !isDelete {
                Label("Bitte Empfänger und IBAN genau prüfen. Die Bank bestätigt den Auftrag mit einer TAN.",
                      systemImage: "exclamationmark.shield")
                    .appFont(.callout).foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func line(_ label: LocalizedStringKey, _ value: String, mono: Bool = false, sensitive: Bool = true) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).appFont(.caption).foregroundStyle(Theme.secondary)
            let t = Text(value).appFont(.callout, monospacedDigit: mono).textSelection(.enabled)
            if sensitive { t.sensitive() } else { t }
        }
    }

    // MARK: buttons

    private var buttons: some View {
        HStack {
            Button("Abbrechen", role: .cancel) { model.cancelStandingSheet(); dismiss() }
                .keyboardShortcut(.cancelAction)
            Spacer()
            if reviewing || isDelete {
                if !isDelete { Button("Zurück") { reviewing = false; model.transferError = nil } }
                if isDelete {
                    Button("Dauerauftrag löschen", role: .destructive) { model.startStandingOrder() }
                        .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent).tint(.red)
                } else {
                    Button("Jetzt mit TAN freigeben") { model.startStandingOrder() }
                        .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
                }
            } else {
                Button("Weiter") { model.transferError = nil; reviewing = true }
                    .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent).disabled(!draft.isValid)
            }
        }
    }
}
