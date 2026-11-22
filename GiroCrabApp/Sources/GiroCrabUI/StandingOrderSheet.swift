import SwiftUI
import GiroCrabCore

/// Dauerauftrag anlegen, ändern oder löschen (GitHub #496): a form, then a read-only
/// review, then the bank's TAN request (`TwoFactorSheet`). A deletion starts at the review.
struct StandingOrderSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    #if os(iOS)
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    #endif
    @State private var reviewing = false
    /// The recipient list under the name field and the own account chosen for an Umbuchung (#1475),
    /// both shared with the transfer form (`RecipientEntry.swift`).
    @State private var suggestions = RecipientSuggestions()
    @State private var ownTargetID: Int64?
    /// "Empfänger verwalten" (#978).
    @State private var managingRecipients = false
    /// What a paste, a template or a dropped invoice filled (#663).
    @State private var importNote: String?
    @State private var dropTargeted = false

    private var draft: StandingDraft { model.standingDraft }
    private var isDelete: Bool { draft.action == .delete }
    private var account: Account? { model.allAccounts.first { $0.id == draft.accountID } }
    private var pinStored: Bool {
        guard let id = account?.connectionId else { return false }
        return model.isDemo || (model.connectionStatus(id)?.hasPin ?? false)
    }

    /// Own accounts an Umbuchung may go to (not the sending account, the same rules as the transfer).
    private var ownTargets: [Account] { model.ownTransferTargets(from: account) }

    /// Fills name and IBAN from the chosen own account.
    private func applyOwnTarget(_ id: Int64?) {
        guard let t = ownTargets.first(where: { $0.id == id }) else { return }
        model.standingDraft.name = AppModel.ownRecipientName(t)
        model.standingDraft.iban = BatchRow.grouped(t.iban)
        model.standingDraft.bic = ""
        suggestions.items = []
    }

    /// Switches between a transfer to someone else and an Umbuchung: leaving Umbuchung empties the
    /// fields again, so no own account stays in a field that looks typed.
    private func setOwnTransfer(_ on: Bool) {
        guard on != draft.ownTransfer else { return }
        model.standingDraft.ownTransfer = on
        if on {
            if !ownTargets.contains(where: { $0.id == ownTargetID }) { ownTargetID = ownTargets.first?.id }
            applyOwnTarget(ownTargetID)
        } else {
            model.standingDraft.name = ""
            model.standingDraft.iban = ""
            model.standingDraft.bic = ""
        }
    }

    /// Takes name and IBAN of a known recipient and closes the list.
    private func pick(_ r: KnownRecipient) {
        suggestions.take(r)
        model.standingDraft.takeRecipient(name: r.name, iban: r.iban)
    }

    private var editing: Bool { !reviewing && !isDelete }
    private var bicText: String { draft.bic.trimmingCharacters(in: .whitespaces) }
    private var bicValid: Bool { bicText.isEmpty || TransferSheetView.looksLikeBIC(bicText) }

    #if os(macOS)
    private func pasteFromClipboard() { report(PaymentParser.parseClipboard()) }
    #endif

    private func importInvoice(_ url: URL, token: UUID) async {
        importNote = String(localized: "Lese Rechnung …")
        report(await PaymentParser.parseFile(url) ?? ParsedPayment(), token: token)
    }

    private func report(_ p: ParsedPayment, token: UUID? = nil) {
        guard !p.isEmpty else { importNote = String(localized: "Nichts Erkennbares gefunden"); return }
        if let token { guard model.fillStandingDraft(p, ifToken: token) else { return } } else { model.fillStandingDraft(p) }
        importNote = String(localized: "Felder ausgefüllt – bitte prüfen")
    }

    /// An order whose IBAN is an own account opens as Umbuchung (the stored name stays).
    private func detectOwnTransfer() {
        guard !isDelete, !draft.ownTransfer,
              let own = AppModel.ownAccount(withIBAN: draft.iban, in: ownTargets) else { return }
        ownTargetID = own.id
        model.standingDraft.ownTransfer = true
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
            HStack(spacing: 12) {
                Text(title).appFont(.title2, weight: .semibold)
                Spacer()
                if editing {
                    // the helpers of the transfer form: templates, recipients, paste
                    TransferTemplatesMenu(standing: true)
                    Button { managingRecipients = true } label: { Label("Empfänger", systemImage: "person.2") }
                        .help("Empfänger verwalten: veraltete IBANs nicht mehr vorschlagen")
                        #if os(iOS)
                        .labelStyle(.iconOnly)
                        #endif
                    #if os(macOS)
                    Button { pasteFromClipboard() } label: { Label("Einfügen", systemImage: "doc.on.clipboard") }
                        .keyboardShortcut("v", modifiers: [.command, .shift])
                        .help("Erkennt IBAN, Betrag, Empfänger und Zweck (⇧⌘V; ⌘V außerhalb von Textfeldern). Rechnung (PDF, ZUGFeRD, GiroCode) hierher ziehen.")
                    #else
                    PaymentPasteButton { report(PaymentParser.parseText($0)) }
                        .labelStyle(.iconOnly)
                    #endif
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20).padding(.vertical, 14)
            // Überweisung ⇄ Umbuchung under the title, as in the transfer form (#1475); offered when there is an own account to book to
            if editing && (!ownTargets.isEmpty || draft.ownTransfer) {
                OwnTransferSwitch(own: Binding(get: { draft.ownTransfer }, set: { setOwnTransfer($0) }))
                    .padding(.bottom, 10)
            }
            Divider()
            if reviewing || isDelete {
                #if os(iOS)
                // large text: the review scrolls instead of being squeezed (values stay whole, buttons stay reachable)
                ScrollView { review }
                #else
                review
                #endif
            } else {
                editor
            }
            #if os(iOS)
            Spacer(minLength: 0)  // a tall sheet keeps the title and the content at the top
            #endif
            if let e = model.transferError {
                Label(e, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(Theme.negative).appFont(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 20).padding(.vertical, 8)
            }
            Divider()
            buttons.padding(14)
        }
        .macFrame(width: 560)
        .overlay {
            if dropTargeted {
                RoundedRectangle(cornerRadius: 12).strokeBorder(Theme.accent, style: StrokeStyle(lineWidth: 2, dash: [6]))
                    .background(Theme.accent.opacity(0.06)).allowsHitTesting(false)
            }
        }
        .onDrop(of: [.fileURL], isTargeted: $dropTargeted) { providers in
            guard editing, let p = providers.first else { return false }
            let token = model.standingImportToken  // read now: the sheet may be closed or reopened before the file arrives
            _ = p.loadObject(ofClass: URL.self) { url, _ in
                guard let url else { return }
                Task { @MainActor in await importInvoice(url, token: token) }
            }
            return true
        }
        // ⌘V outside a text field (inside one, normal paste keeps working).
        #if os(macOS)
        .onPasteCommand(of: [.plainText]) { _ in if editing { pasteFromClipboard() } }
        #endif
        .task {
            reviewing = reviewing || isDelete
            detectOwnTransfer()
            if model.options.demoSheet == "standing-order-confirm" { reviewing = true } // screenshots
        }
        // what was hidden or shown again there applies to an open list at once
        .capturableSheet(isPresented: $managingRecipients, onDismiss: { suggestions.refresh(model: model) }) {
            KnownRecipientsSheet()
        }
    }

    // MARK: editor

    private func intervalText(_ n: Int) -> String {
        if draft.unit == "W" { return n == 1 ? String(localized: "1 Woche") : String(localized: "\(n) Wochen") }
        return n == 1 ? String(localized: "1 Monat") : String(localized: "\(n) Monate")
    }

    private var editor: some View {
        @Bindable var model = model
        return Form {
            Section {
                LabeledContent("Von Konto") {
                    SourceAccountPicker(selection: $model.standingDraft.accountID, accounts: model.standingOrderAccounts)
                }
                .disabled(draft.action == .change)
            }
            Section {
                if draft.ownTransfer {
                    OwnTargetFields(targets: ownTargets, selection: $ownTargetID, name: draft.name,
                                    select: applyOwnTarget)
                } else {
                    // LabeledContent keeps the field names on iOS, where a TextField in a Form shows only its prompt
                    LabeledContent("Empfänger") {
                        RecipientNameField(name: $model.standingDraft.name, suggestions: suggestions, pick: pick)
                    }
                    LabeledContent("IBAN") {
                        HStack(spacing: 8) {
                            IBANInput(iban: $model.standingDraft.iban)
                            FieldValidity(ok: draft.iban.isEmpty ? nil : draft.ibanValid,
                                          valid: "Prüfsumme korrekt", invalid: "IBAN ungültig (Prüfsumme/Länge)")
                        }
                        .withBankLine(draft.iban, alignment: transferFieldEdge)
                    }
                    LabeledContent("BIC") {
                        HStack(spacing: 8) {
                            TextField("BIC", text: $model.standingDraft.bic, prompt: Text("optional"))
                                .transferField()
                                .appFont(.body, design: .monospaced)
                                .autocorrectionDisabled()
                                // 8 or 11 characters: as wide as the amount (HIG: size fits the input)
                                .frame(maxWidth: 160 * textScale)
                                .sensitive()
                            FieldValidity(ok: bicText.isEmpty ? nil : bicValid, valid: "BIC gültig", invalid: "BIC ungültig")
                            Spacer(minLength: 0)
                        }
                    }
                }
            } footer: {
                if let importNote { Text(importNote) }
            }
            Section {
                LabeledContent("Betrag") {
                    HStack(spacing: 6) {
                        TextField("Betrag", text: $model.standingDraft.amountText, prompt: Text(Money.plain(0)))
                            .transferField(alignment: .trailing)
                            .appFont(.body, monospacedDigit: true)
                            #if os(iOS)
                            .keyboardType(.decimalPad)
                            #endif
                            .frame(maxWidth: 160 * textScale)
                            .sensitive(.amounts)
                        Text("€").foregroundStyle(Theme.secondary)
                        Spacer(minLength: 0)
                    }
                }
                LabeledContent("Verwendungszweck") {
                    TextField("Verwendungszweck", text: $model.standingDraft.purpose, prompt: Text("optional"), axis: .vertical)
                        .transferField()
                        .lineLimit(1...3)
                        .sensitive()
                }
            }
            Section {
                Picker("Rhythmus", selection: $model.standingDraft.unit) {
                    Text("Monate").tag("M")
                    Text("Wochen").tag("W")
                }
                let intervals = draft.allowedIntervals(model.standingLimits)
                if intervals.isEmpty {
                    Stepper(value: $model.standingDraft.interval, in: 1...99) {
                        HStack {
                            Text("Alle")
                            Spacer()
                            Text(intervalText(draft.interval)).appFont(.body, monospacedDigit: true).foregroundStyle(Theme.secondary)
                        }
                    }
                } else {
                    Picker("Alle", selection: $model.standingDraft.interval) {
                        ForEach(intervals, id: \.self) { Text(intervalText($0)).tag($0) }
                    }
                }
                let days = draft.allowedDays(model.standingLimits)
                if draft.unit == "W" {
                    Picker("Wochentag", selection: $model.standingDraft.execDay) {
                        ForEach(days.isEmpty ? Array(1...7) : days, id: \.self) { Text(StandingDraft.weekdayName($0)).tag($0) }
                    }
                } else {
                    Picker("Ausführungstag", selection: $model.standingDraft.execDay) {
                        ForEach(days.isEmpty ? Array(1...30) : days.filter { $0 < 31 }, id: \.self) { Text("\($0).").tag($0) }
                        if days.isEmpty || days.contains(31) { Text("Letzter Tag des Monats").tag(31) }
                    }
                }
                DatePicker("Erste Ausführung", selection: $model.standingDraft.firstDate,
                           in: draft.firstDateRange(model.standingLimits), displayedComponents: .date)
                Toggle("Mit Enddatum", isOn: $model.standingDraft.hasEnd)
                if draft.hasEnd {
                    DatePicker("Letzte Ausführung", selection: $model.standingDraft.lastDate,
                               in: draft.firstDate..., displayedComponents: .date)
                }
            }
        }
        .formStyle(.grouped)
        // The list floats over the form below the name field, so a row cannot clip it.
        .recipientSuggestionList(suggestions, pick: pick)
        .onChange(of: draft.purpose) { _, new in
            if new.count > 140 { model.standingDraft.purpose = String(new.prefix(140)) }
        }
        .onChange(of: draft.accountID) {
            model.refreshStandingLimits()
            // the sending account is no target: keep a valid one, or fall back to a transfer
            guard draft.ownTransfer else { return }
            if ownTargets.isEmpty { setOwnTransfer(false); return }
            if !ownTargets.contains(where: { $0.id == ownTargetID }) {
                ownTargetID = ownTargets.first?.id
                applyOwnTarget(ownTargetID)
            }
        }
        .onChange(of: draft.unit) { model.standingDraft.conform(to: model.standingLimits) }
        // The first execution and the execution day describe the same schedule: the one changed last wins.
        .onChange(of: draft.firstDate) { model.standingDraft.syncDay(to: model.standingLimits) }
        .onChange(of: draft.execDay) { model.standingDraft.syncDate(to: model.standingLimits) }
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
                    .appFont(.title, weight: .semibold, monospacedDigit: true).fixedSize().sensitive(.amounts)
                Text("\(draft.rhythmText) · \(draft.execDayText)").appFont(.callout).foregroundStyle(Theme.secondary)
                line("Von Konto", account.map { "\($0.name) · \(formatIBAN($0.iban))" } ?? "")
                line("Empfänger", draft.name)
                line("IBAN", draft.iban, mono: true)
                if let bank = bankNameForIban(iban: draft.iban) { line("Bank", bank) }
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
            // never truncate: recipient and IBAN are what the person checks here
            let t = Text(value).appFont(.callout, monospacedDigit: mono).textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if sensitive { t.sensitive() } else { t }
        }
    }

    // MARK: buttons

    private var cancelButton: some View {
        Button("Abbrechen", role: .cancel) { model.cancelStandingSheet(); dismiss() }
            .keyboardShortcut(.cancelAction)
    }

    @ViewBuilder private var backButton: some View {
        if (reviewing || isDelete) && !isDelete {
            Button("Zurück") { reviewing = false; model.transferError = nil }
        }
    }

    @ViewBuilder private var primaryButton: some View {
        if reviewing || isDelete {
            if isDelete {
                // HIG: a destructive action is never the default button (Return does nothing here)
                Button("Dauerauftrag löschen", role: .destructive) { model.startStandingOrder() }
                    .buttonStyle(.borderedProminent).tint(.red)
            } else {
                Button("Jetzt mit TAN freigeben") { model.startStandingOrder() }
                    .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent)
            }
        } else {
            Button("Weiter") {
                model.transferError = nil
                model.invalidateStandingImports()  // an invoice still being read must not change what is under review
                model.standingDraft.iban = TransferSheetView.groupedIBAN(draft.iban)  // grouped even if the field never lost the focus
                reviewing = true
            }
                .keyboardShortcut(.defaultAction).buttonStyle(.borderedProminent).disabled(!draft.isValid)
        }
    }

    @ViewBuilder private var buttons: some View {
        #if os(iOS)
        if dynamicTypeSize.isAccessibilitySize {
            // large text: stacked, the main action on top (side by side the labels break into many lines)
            VStack(spacing: 12) {
                primaryButton.frame(maxWidth: .infinity).fixedSize(horizontal: false, vertical: true)
                backButton
                cancelButton
            }
        } else {
            sideBySideButtons
        }
        #else
        sideBySideButtons
        #endif
    }

    private var sideBySideButtons: some View {
        HStack {
            cancelButton
            Spacer()
            backButton
            primaryButton
        }
    }
}
