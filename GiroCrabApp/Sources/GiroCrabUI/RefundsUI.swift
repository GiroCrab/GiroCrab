import SwiftUI
import GiroCrabCore

// Refund links (#714): marker in the booking rows, context menu entries, the sheet that picks
// the other booking and the inspector card. Layout and rules: `docs/ERSTATTUNGEN_UI.md`.

/// Small arrow behind the name of a linked booking: a refund, or a booking that has refunds.
struct RefundLinkMarker: View {
    let kind: RefundLinkKind

    var body: some View {
        if kind != .none {
            Image(systemName: kind == .refund ? "arrow.uturn.backward" : "arrow.uturn.backward.circle")
                .appFont(.caption, weight: .semibold)
                .foregroundStyle(Theme.secondary)
                .help(kind == .refund ? "Erstattung – mit der ursprünglichen Buchung verknüpft" : "Mit einer Erstattung verknüpft")
                .accessibilityLabel(kind == .refund ? "Verknüpfte Erstattung" : "Hat verknüpfte Erstattung")
        }
    }
}

/// Context menu of a booking row: incoming bookings get their original, outgoing ones their refunds.
struct RefundLinkMenu: View {
    let model: AppModel
    let booking: Booking

    var body: some View {
        if booking.amountCents > 0 {
            Button(booking.refundLink == .refund ? "Ursprüngliche Buchung ändern …" : "Ursprüngliche Buchung zuordnen …",
                   systemImage: "arrow.uturn.backward") { model.openRefundPicker(for: booking) }
            if booking.refundLink == .refund {
                Button("Verknüpfung zur ursprünglichen Buchung aufheben", systemImage: "xmark") {
                    model.unlinkRefund(refundID: booking.id)
                }
            }
        } else if booking.amountCents < 0 {
            Button(booking.refundLink == .original ? "Erstattungen bearbeiten …" : "Erstattung zuordnen …",
                   systemImage: "arrow.uturn.backward") { model.openRefundPicker(for: booking) }
        }
    }
}

/// One booking of a refund link: name, date and account, the amount at the end.
private struct RefundBookingLine: View {
    let row: RefundBooking
    /// The sheet has room for the purpose; the inspector card has not.
    var showsPurpose = false

    private var purpose: String {
        row.purpose.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(row.displayName.isEmpty ? String(localized: "Ohne Namen") : row.displayName)
                    .appDefaultFont()
                    .foregroundStyle(Theme.text)
                    .lineLimit(1)
                    .sensitive()
                Text(verbatim: "\(DateText.medium(row.bookingDate)) · \(row.accountName)")
                    .appFont(.caption)
                    .foregroundStyle(Theme.secondary)
                    .lineLimit(1)
                    .sensitive()
                if showsPurpose, !purpose.isEmpty {
                    Text(purpose)
                        .appFont(.caption)
                        .foregroundStyle(Theme.tertiary)
                        .lineLimit(1)
                        .sensitive()
                }
            }
            .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
            AmountLabel(cents: row.amountCents, currency: row.currency)
                .lineLimit(1)
                .fixedSize()
        }
    }
}

/// "Refunded so far" of an original, only when every refund has the original's currency
/// (amounts of different currencies are never summed, #569).
private enum RefundSum {
    static func text(originalCents: Int64, currency: String, refunds: [RefundBooking]) -> String? {
        let code = { (c: String) in c.isEmpty ? "EUR" : c.uppercased() }
        guard !refunds.isEmpty, refunds.allSatisfy({ code($0.currency) == code(currency) }) else { return nil }
        let sum = refunds.reduce(Int64(0)) { $0 + $1.amountCents }
        return String(localized: "Erstattet: \(Money.format(sum, currency: code(currency))) von \(Money.format(abs(originalCents), currency: code(currency)))")
    }
}

/// "Ursprüngliche Buchung zuordnen …" / "Erstattung zuordnen …" (#714): the linked bookings on
/// top, below them the candidates the core suggests (other direction, within twelve months, all
/// accounts; same counterparty first). A click links or unlinks at once; the sheet stays open so
/// that an original can take several partial refunds. Layout and rules: `docs/ERSTATTUNGEN_UI.md`.
struct RefundPickerSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let booking: Booking
    @State private var linked: [RefundBooking] = []
    @State private var candidates: [RefundBooking] = []
    @State private var loaded = false
    @State private var search = ""
    @State private var loadError: String?
    /// Counts the loads: only the newest one may show its result.
    @State private var loads = 0

    /// The sheet was opened from a refund: one original is chosen. Otherwise refunds are collected.
    private var fromRefund: Bool { booking.amountCents > 0 }
    private var suggestions: [RefundBooking] {
        let ids = Set(linked.map(\.id))
        return candidates.filter { !ids.contains($0.id) }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(fromRefund ? "Ursprüngliche Buchung zuordnen" : "Erstattung zuordnen").appFont(.headline)
                Spacer()
                Button("Fertig") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
            Divider()
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(booking.merchant.name.isEmpty ? (booking.counterparty.isEmpty ? booking.bookingText : booking.counterparty) : booking.merchant.name)
                            .appFont(.body, weight: .semibold)
                            .lineLimit(1)
                            .sensitive()
                        Text(verbatim: "\(DateText.medium(booking.bookingDate)) · \(booking.accountName)")
                            .appFont(.caption).foregroundStyle(Theme.secondary)
                            .lineLimit(1)
                            .sensitive()
                    }
                    .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                    AmountLabel(cents: booking.amountCents, currency: booking.currency).fixedSize()
                }
                Text(fromRefund
                     ? "Wählen Sie die Buchung, die mit dieser Erstattung zurückgezahlt wurde. Vorgeschlagen werden Ausgaben aller Konten aus den zwölf Monaten davor."
                     : "Wählen Sie die Erstattungen zu dieser Buchung, auch mehrere Teilerstattungen. Vorgeschlagen werden Eingänge aller Konten aus den zwölf Monaten danach.")
                    .appFont(.callout)
                    .foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Die Verknüpfung dient nur der Zuordnung: Beträge, Kategorien und Kontostände ändern sich nicht.")
                    .appFont(.caption)
                    .foregroundStyle(Theme.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                TextField("Name oder Verwendungszweck suchen", text: $search)
                    .textFieldStyle(.roundedBorder)
                    .sensitive() // typed names
                if let loadError {
                    Label("Buchungen konnten nicht geladen werden", systemImage: "exclamationmark.triangle")
                        .appFont(.caption, weight: .semibold).foregroundStyle(Theme.warning)
                    Text(loadError).appFont(.caption2).foregroundStyle(Theme.tertiary).lineLimit(2)
                }
            }
            .padding(16)
            if loaded && linked.isEmpty && suggestions.isEmpty && loadError == nil {
                Group {
                    if search.trimmingCharacters(in: .whitespaces).isEmpty {
                        ContentUnavailableView {
                            Label("Keine passenden Buchungen", systemImage: "arrow.uturn.backward")
                        } description: {
                            Text(fromRefund
                                 ? "In den zwölf Monaten vor dieser Erstattung gibt es keine Ausgabe."
                                 : "In den zwölf Monaten nach dieser Buchung gibt es keinen Eingang, der noch keiner anderen Buchung zugeordnet ist.")
                        }
                    } else {
                        ContentUnavailableView.search
                    }
                }
                .frame(maxHeight: .infinity)
            } else {
                list
            }
        }
        .macFrame(minWidth: 540, idealWidth: 580, minHeight: 520)
        .task(id: "\(model.vaultRevision)|\(model.refundLinksRevision)|\(search)") { await reload() }
        #if os(macOS)
        .onExitCommand { dismiss() }
        #endif
    }

    private var list: some View {
        List {
            if !linked.isEmpty {
                Section {
                    ForEach(linked, id: \.id) { row($0, isLinked: true) }
                } header: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(fromRefund ? "Verknüpfte ursprüngliche Buchung" : "Verknüpfte Erstattungen (\(linked.count))")
                        if !fromRefund, let sum = RefundSum.text(originalCents: booking.amountCents, currency: booking.currency, refunds: linked) {
                            Text(sum).sensitive(.amounts)
                        }
                    }
                }
            }
            if !suggestions.isEmpty {
                Section {
                    ForEach(suggestions, id: \.id) { row($0, isLinked: false) }
                } header: {
                    Text("Vorschläge")
                }
            }
        }
    }

    private func row(_ r: RefundBooking, isLinked: Bool) -> some View {
        HStack(spacing: 10) {
            Image(systemName: isLinked ? "checkmark.circle.fill" : "circle")
                .foregroundStyle(isLinked ? Theme.accent : Theme.tertiary)
                .accessibilityHidden(true)
            RefundBookingLine(row: r, showsPurpose: true)
            Button(isLinked ? "Aufheben" : "Zuordnen") { toggle(r, isLinked: isLinked) }
                .buttonStyle(.borderless)
                .help(isLinked ? "Verknüpfung aufheben" : (fromRefund && !linked.isEmpty ? "Statt der bisherigen Buchung zuordnen" : "Mit dieser Buchung verknüpfen"))
        }
        .contentShape(Rectangle())
        .onTapGesture(count: 2) { toggle(r, isLinked: isLinked) }
    }

    private func toggle(_ r: RefundBooking, isLinked: Bool) {
        let refundID = fromRefund ? booking.id : r.id
        if isLinked {
            model.unlinkRefund(refundID: refundID)
        } else {
            model.linkRefund(refundID: refundID, originalID: fromRefund ? r.id : booking.id)
        }
    }

    private func reload() async {
        loads += 1
        let run = loads
        let id = booking.id
        let query = search
        let refund = fromRefund
        do {
            let result = try await model.vaultOffMain { vault -> ([RefundBooking], [RefundBooking]) in
                let links = try vault.refundLinks(transactionId: id)
                let linked = refund ? (links.original.map { [$0] } ?? []) : links.refunds
                return (linked, try vault.refundCandidates(transactionId: id, query: query, limit: 200))
            }
            guard run == loads, let result else { return }
            (linked, candidates) = result
            loadError = nil
        } catch {
            guard run == loads else { return }
            loadError = describe(error)
        }
        loaded = true
    }
}

/// Inspector card of a linked booking (#714): a refund shows its original, an original its
/// refunds. A click on a line selects that booking in the list; ✕ removes the link.
struct RefundLinksCard: View {
    @Environment(AppModel.self) private var model
    let booking: Booking
    @State private var links: RefundLinks?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label("Erstattung", systemImage: "arrow.uturn.backward")
                    .appFont(.subheadline, weight: .semibold).foregroundStyle(Theme.text)
                Spacer()
                Button(booking.amountCents > 0 ? "Ändern …" : "Bearbeiten …") { model.openRefundPicker(for: booking) }
                    .buttonStyle(.borderless)
                    .appFont(.caption)
            }
            if let original = links?.original {
                Text("Ursprüngliche Buchung").appFont(.caption).foregroundStyle(Theme.secondary)
                line(original, refundID: booking.id)
            }
            if let refunds = links?.refunds, !refunds.isEmpty {
                Text(refunds.count == 1 ? "Erstattung dieser Buchung" : "Erstattungen dieser Buchung (\(refunds.count))")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
                ForEach(refunds, id: \.id) { line($0, refundID: $0.id) }
                if let sum = RefundSum.text(originalCents: booking.amountCents, currency: booking.currency, refunds: refunds) {
                    Text(sum)
                        .sensitive(.amounts)
                        .appFont(.caption)
                        .foregroundStyle(Theme.secondary)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .card()
        // the marker also changes when a sync brings a link from another device
        .task(id: "\(booking.id)|\(booking.refundLink)|\(model.refundLinksRevision)|\(model.vaultRevision)") {
            let id = booking.id
            let read = try? await model.vaultOffMain { try $0.refundLinks(transactionId: id) }
            guard !Task.isCancelled else { return }
            links = read ?? nil
        }
    }

    private func line(_ r: RefundBooking, refundID: Int64) -> some View {
        HStack(spacing: 6) {
            Button { model.revealBooking(r.id) } label: {
                HStack(spacing: 6) {
                    RefundBookingLine(row: r)
                    Image(systemName: "chevron.right")
                        .appFont(.caption2, weight: .semibold)
                        .foregroundStyle(Theme.tertiary)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Diese Buchung in der Liste anzeigen")
            Button { model.unlinkRefund(refundID: refundID) } label: {
                Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.tertiary)
            }
            .buttonStyle(.borderless)
            .help("Verknüpfung aufheben")
            .accessibilityLabel("Verknüpfung aufheben")
        }
    }
}
