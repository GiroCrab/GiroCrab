import SwiftUI
import GiroCrabCore

/// Right column of „Abos“ (GitHub #1787): the selected subscription with its rhythm, contract fields
/// and bookings. Design page: `docs/ABOS_UI.md`.
struct SubscriptionInspector: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let item = model.subscriptionItems.first(where: { $0.id == model.selectedSubscriptionID }) {
            // a new identity per subscription: drafts and pending saves never leak into the next one
            SubscriptionDetailView(item: item).id(item.id)
        } else {
            VStack(alignment: .leading, spacing: 8) {
                Label("Kein Abo ausgewählt", systemImage: "repeat.circle").appFont(.headline)
                Text("Wähle ein Abo, um Vertragsdaten und Buchungen zu sehen.")
                    .appFont(.callout).foregroundStyle(Theme.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .flatSection()
        }
    }
}

private struct SubscriptionDetailView: View {
    @Environment(\.negativeAmountStyle) private var amountStyle
    @Environment(AppModel.self) private var model
    let item: SubscriptionItem
    @State private var draft: SubscriptionDetails
    @State private var manual: ManualSubscription?
    @State private var amountText: String
    @State private var saveTask: Task<Void, Never>?
    @State private var pending = false
    @State private var showAllBookings = false
    @State private var confirmRemove = false

    init(item: SubscriptionItem) {
        self.item = item
        _draft = State(initialValue: item.details)
        _manual = State(initialValue: item.manual)
        _amountText = State(initialValue: item.manual.map { Money.plain(abs($0.amountCents)) } ?? "")
    }

    /// Stable stand-in for an unreadable date: a binding that returns `Date()` changes on every read.
    private static let fallbackDate = Calendar.current.startOfDay(for: Date())

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Divider()
            VStack(alignment: .leading, spacing: 12) {
                Text("Information").appFont(.callout, weight: .semibold)
                if manual != nil { manualFields }
                rhythmField
                contractFields
                // below the fields: a line that appears or disappears must not push the controls above it
                VStack(alignment: .leading, spacing: 4) {
                    SubscriptionContractLine(item: item)
                    SubscriptionExtraLines(item: item)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .flatSection()
            if !item.isManual {
                Divider()
                bookingList.frame(maxWidth: .infinity, alignment: .leading).flatSection()
            }
            Divider()
            actions.flatSection(vertical: 10)
        }
        .onChange(of: draft) { old, new in
            // text is saved after a pause, switches and pickers at once
            scheduleSave(delay: old.note == new.note ? 0 : 400)
        }
        .onChange(of: manual) { _, _ in scheduleSave(delay: 400) }
        .onAppear {
            // an older cancellation without an end: fix the assumed end (the next payment) so it stops moving
            if draft.cancelled, draft.contractEnd == nil, let due = item.nextDue { draft.contractEnd = due }
        }
        .onDisappear { flush() }
        .confirmationDialog("Abo „\(item.name)“ löschen?", isPresented: $confirmRemove) {
            Button("Löschen", role: .destructive) {
                model.removeManualSubscription(id: item.manual?.id ?? "")
                model.selectedSubscriptionID = nil
            }
        }
    }

    // MARK: saving

    /// Saved shortly after the last change, and when the inspector switches to another subscription.
    private func scheduleSave(delay: Int = 400) {
        pending = true
        saveTask?.cancel()
        if delay == 0 { flush(); return }
        saveTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(delay))
            guard !Task.isCancelled else { return }
            flush()
        }
    }

    /// One snapshot of both drafts (fields and contract details), so a pending save of one never
    /// replaces or reverts the other.
    private func commit() {
        if var m = manual {
            m.details = draft
            model.updateManualSubscription(m)
        } else {
            model.setSubscriptionDetails(item, draft)
        }
    }

    private func flush() {
        saveTask?.cancel()
        guard pending else { return }
        pending = false
        commit()
    }

    // MARK: header

    private var header: some View {
        HStack(alignment: .top, spacing: 12) {
            SubscriptionIcon(item: item, size: 44)
            VStack(alignment: .leading, spacing: 4) {
                Text(item.name).appFont(.title3, weight: .semibold).sensitive()
                Text("\(Money.format(item.amountCents, currency: item.currency)) / \(item.group.perPeriod)")
                    .appFont(.callout, weight: .semibold, monospacedDigit: true)
                    .foregroundStyle(item.isEnded ? Theme.secondary : amountStyle.ink(item.amountCents)).sensitive(.amounts)
                if item.group != .yearly {
                    Text("\(Money.format(item.yearlyCents, currency: item.currency)) / Jahr")
                        .appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary).sensitive(.amounts)
                }
                if item.group != .monthly {
                    Text("\(Money.format(item.monthlyCents, currency: item.currency)) / Monat")
                        .appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary).sensitive(.amounts)
                }
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .flatSection()
    }

    // MARK: fields

    private func fieldRow<Content: View>(_ title: LocalizedStringKey, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).appFont(.caption).foregroundStyle(Theme.secondary)
            content().appFont(.callout).frame(minHeight: 24, alignment: .leading)
        }
    }

    private var manualFields: some View {
        VStack(alignment: .leading, spacing: 12) {
            fieldRow("Name") {
                TextField("Name", text: Binding(get: { manual?.name ?? "" }, set: { manual?.name = $0 }))
                    .textFieldStyle(.roundedBorder).sensitive()
            }
            fieldRow("Art") {
                Picker("Art", selection: Binding(
                    get: { (manual?.amountCents ?? 0) > 0 },
                    set: { income in
                        let v = abs(manual?.amountCents ?? 0)
                        manual?.amountCents = income ? v : -v
                    })) {
                    Text("Ausgabe").tag(false)
                    Text("Einnahme").tag(true)
                }
                .labelsHidden().pickerStyle(.segmented).fixedSize()
            }
            fieldRow("Betrag pro Zahlung") {
                HStack(spacing: 6) {
                    TextField("0,00", text: $amountText)
                        .textFieldStyle(.roundedBorder).frame(width: 100).multilineTextAlignment(.trailing)
                        .sensitive(.amounts)
                        .onChange(of: amountText) { _, t in
                            if t.contains("-") { amountText = t.replacingOccurrences(of: "-", with: ""); return }
                            if let c = Money.parseCents(t), c > 0 { manual?.amountCents = (manual?.amountCents ?? -1) > 0 ? c : -c }
                        }
                    Text(manual?.currency ?? "EUR").appFont(.callout).foregroundStyle(Theme.secondary)
                }
            }
            fieldRow("Symbol") {
                SubscriptionSymbolPicker(symbol: Binding(get: { manual?.symbol }, set: { manual?.symbol = $0 }))
            }
            fieldRow("Eine Zahlung am") {
                DatePicker("", selection: Binding(
                    get: { DateText.parseISODay(manual?.startDate ?? "") ?? Self.fallbackDate },
                    set: { manual?.startDate = DateText.iso($0) }), displayedComponents: .date)
                    .labelsHidden()
            }
        }
    }

    /// Manual entries keep their rhythm themselves; a detected series is changed like in the recurring
    /// list (the series key may change, the fields move along).
    @ViewBuilder private var rhythmField: some View {
        fieldRow("Turnus") {
            if let s = item.series {
                Picker("Turnus", selection: Binding(
                    get: { s.interval },
                    set: { if $0 != .custom { model.changeSubscriptionRhythm(s, to: $0) } })) {
                    if s.interval == .custom { Text(RecurrenceInterval.custom.label).tag(RecurrenceInterval.custom) }
                    ForEach(RecurrenceInterval.fixed, id: \.self) { Text($0.label).tag($0) }
                }
                .labelsHidden().pickerStyle(.menu).fixedSize()
            } else {
                Picker("Turnus", selection: Binding(get: { manual?.group ?? 0 }, set: { manual?.group = $0 })) {
                    ForEach([SubscriptionGroup.monthly, .quarterly, .halfYearly, .yearly]) { Text($0.title).tag($0.rawValue) }
                }
                .labelsHidden().pickerStyle(.menu).fixedSize()
            }
        }
    }

    private var contractFields: some View {
        VStack(alignment: .leading, spacing: 12) {
            fieldRow("Kündigungsfrist") {
                HStack(spacing: 6) {
                    TextField("Kündigungsfrist", value: $draft.noticeValue, format: .number, prompt: Text(""))
                        .textFieldStyle(.roundedBorder).frame(width: 56).multilineTextAlignment(.trailing)
                        .onChange(of: draft.noticeValue) { _, v in if let v, v < 0 { draft.noticeValue = abs(v) } }
                    Picker("Einheit", selection: $draft.noticeUnit) {
                        ForEach(NoticeUnit.allCases) { Text($0.title).tag($0) }
                    }
                    .labelsHidden().pickerStyle(.menu).fixedSize()
                }
            }
            fieldRow("Vertragsende") {
                HStack(spacing: 8) {
                    Toggle("Endet am", isOn: Binding(
                        get: { draft.contractEnd != nil },
                        set: { draft.contractEnd = $0 ? (draft.contractEnd ?? item.nextDue ?? DateText.iso(Date())) : nil }))
                        .toggleStyle(.checkbox)
                    if let end = draft.contractEnd {
                        DatePicker("", selection: Binding(
                            get: { DateText.parseISODay(end) ?? Self.fallbackDate },
                            set: { draft.contractEnd = DateText.iso($0) }), displayedComponents: .date)
                            .labelsHidden()
                    }
                }
            }
            fieldRow("Verlängerung um") {
                HStack(spacing: 6) {
                    TextField("Verlängerung", value: $draft.renewalMonths, format: .number, prompt: Text(""))
                        .textFieldStyle(.roundedBorder).frame(width: 56).multilineTextAlignment(.trailing)
                        .onChange(of: draft.renewalMonths) { _, v in if let v, v < 0 { draft.renewalMonths = abs(v) } }
                    Text("Monate").appFont(.callout).foregroundStyle(Theme.secondary)
                }
            }
            fieldRow("Gekündigt") {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 8) {
                        Toggle("Gekündigt am", isOn: Binding(
                            get: { draft.cancelled },
                            set: { on in
                                draft.cancelled = on
                                // the end is fixed now: the next payment moves on with every booking, the end must not
                                if on, draft.contractEnd == nil { draft.contractEnd = item.nextDue ?? DateText.iso(Date()) }
                                draft.cancelledOn = on ? (draft.cancelledOn ?? DateText.iso(Date())) : nil
                            })).toggleStyle(.checkbox)
                        if draft.cancelled, let on = draft.cancelledOn {
                            DatePicker("", selection: Binding(
                                get: { DateText.parseISODay(on) ?? Self.fallbackDate },
                                set: { draft.cancelledOn = DateText.iso($0) }), displayedComponents: .date)
                                .labelsHidden()
                        }
                    }
                    if draft.cancelled, let inTime = item.cancelledInTime, let deadline = item.status?.deadline {
                        if inTime {
                            Label("Rechtzeitig gekündigt (Frist \(DateText.medium(DateText.iso(deadline))))", systemImage: "checkmark.circle")
                                .appFont(.caption).foregroundStyle(Theme.inflow)
                        } else if let missed = item.status?.missedDeadline, let end = item.status?.end {
                            Label("Nach Fristende gekündigt (Frist \(DateText.medium(DateText.iso(missed)))) · Vertrag läuft bis \(DateText.medium(DateText.iso(end)))", systemImage: "exclamationmark.triangle")
                                .appFont(.caption).foregroundStyle(Theme.warning)
                        }
                    }
                }
            }
            fieldRow("Probezeit") {
                HStack(spacing: 8) {
                    Toggle("Endet am", isOn: Binding(
                        get: { draft.trialEnd != nil },
                        set: { draft.trialEnd = $0 ? (draft.trialEnd ?? item.nextDue ?? DateText.iso(Date())) : nil }))
                        .toggleStyle(.checkbox)
                    if let end = draft.trialEnd {
                        DatePicker("", selection: Binding(
                            get: { DateText.parseISODay(end) ?? Self.fallbackDate },
                            set: { draft.trialEnd = DateText.iso($0) }), displayedComponents: .date)
                            .labelsHidden()
                    }
                }
            }
            fieldRow("Notiz") {
                TextField("Notiz", text: $draft.note).textFieldStyle(.roundedBorder).sensitive()
            }
        }
    }

    // MARK: bookings

    @ViewBuilder private var bookingList: some View {
        let all = model.subscriptionBookings[item.id] ?? []
        VStack(alignment: .leading, spacing: 6) {
            Text("Buchungen (\(all.count))").appFont(.callout, weight: .semibold)
            if all.isEmpty {
                Text("Keine Buchungen gefunden.").appFont(.caption).foregroundStyle(Theme.secondary)
            }
            ForEach(showAllBookings ? all : Array(all.prefix(6)), id: \.id) { b in
                Button { model.revealBooking(b.id, inAccount: item.accountId) } label: {
                    HStack(spacing: 8) {
                        Text(DateText.medium(b.bookingDate)).appFont(.caption, monospacedDigit: true)
                            .foregroundStyle(Theme.secondary).fixedSize()
                        Spacer(minLength: 4)
                        Text(Money.format(b.amountCents, currency: item.currency)).appFont(.caption, monospacedDigit: true)
                            .foregroundStyle(amountStyle.ink(b.amountCents)).sensitive(.amounts).fixedSize()
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("In der Buchungsliste anzeigen")
            }
            if all.count > 6 {
                Button(showAllBookings ? "Weniger anzeigen" : "Alle \(all.count) Buchungen anzeigen") {
                    showAllBookings.toggle()
                }
                .buttonStyle(.link).appFont(.caption)
            }
        }
    }

    private var actions: some View {
        HStack(spacing: 12) {
            if let s = item.series {
                Button("Buchungen im Konto anzeigen") {
                    if let b = model.subscriptionBookings[item.id]?.first, !b.merchant.key.isEmpty {
                        model.focusMerchant(of: b, inAccount: item.accountId, allPeriods: true)
                    } else {
                        model.revealBooking(s.lastBookingId, inAccount: item.accountId)
                    }
                }
                Button("Kein Abo mehr", role: .destructive) {
                    model.setSubscription(s, marked: false)
                    model.selectedSubscriptionID = nil
                }
            } else {
                Button("Abo löschen …", role: .destructive) { confirmRemove = true }
            }
        }
        .buttonStyle(.link).appFont(.callout)
    }
}
