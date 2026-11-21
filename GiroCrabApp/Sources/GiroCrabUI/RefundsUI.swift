import SwiftUI
import GiroCrabCore

// Refund links (#714): marker in the booking rows, context menu entries, the sheet that picks
// the other booking and the inspector card. Layout and rules: `docs/ERSTATTUNGEN_UI.md`.

/// Small arrow behind the name of a linked booking: a refund, or a booking that has refunds.
struct RefundLinkMarker: View {
    let kind: RefundLinkKind
    /// Only for a booking its links cover in part: the covered cents, shown next to the symbol.
    var partCents: Int64?
    var currency = "EUR"

    var body: some View {
        if kind != .none {
            HStack(spacing: 3) {
                Image(systemName: "arrow.uturn.backward")
                    .appFont(.caption, weight: .semibold)
                    .help(kind == .refund ? "Erstattung – mit der ursprünglichen Buchung verknüpft" : "Mit einer Erstattung verknüpft")
                    .accessibilityLabel(kind == .refund ? "Verknüpfte Erstattung" : "Hat verknüpfte Erstattung")
                if let partCents {
                    Text(Money.format(partCents, currency: currency.isEmpty ? "EUR" : currency.uppercased()))
                        .appFont(.caption, monospacedDigit: true)
                        .lineLimit(1)
                        .fixedSize()
                        .sensitive(.amounts)
                        .help(kind == .refund ? "Erst dieser Teil der Erstattung ist Buchungen zugeordnet. Der Rest zählt weiter als Einnahme." : "Dieser Teil der Buchung wurde erstattet.")
                }
            }
            .foregroundStyle(Theme.secondary)
        }
    }
}

/// Context menu of a booking row: incoming bookings get their original, outgoing ones their refunds.
struct RefundLinkMenu: View {
    let model: AppModel
    let booking: Booking

    var body: some View {
        if booking.amountCents > 0 {
            Button(booking.refundLink == .refund ? "Ursprüngliche Buchungen bearbeiten …" : "Ursprüngliche Buchung zuordnen …",
                   systemImage: "arrow.uturn.backward") { model.openRefundPicker(for: booking) }
            if booking.refundLink == .refund {
                Button("Verknüpfung zur ursprünglichen Buchung aufheben", systemImage: "xmark") {
                    model.unlinkRefundOriginals(refundID: booking.id)
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
                // a split booking is linked by one of its parts
                if row.part > 0 {
                    Text(verbatim: [String(localized: "Teil \(Int(row.part))"), row.partLabel].filter { !$0.isEmpty }.joined(separator: " · "))
                        .appFont(.caption, weight: .medium)
                        .foregroundStyle(Theme.accent)
                        .lineLimit(1)
                        .sensitive(.labels)
                }
                if showsPurpose, !purpose.isEmpty {
                    Text(purpose)
                        .appFont(.caption)
                        .foregroundStyle(Theme.tertiary)
                        .lineLimit(1)
                        .sensitive()
                }
            }
            .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
            VStack(alignment: .trailing, spacing: 1) {
                AmountLabel(cents: row.amountCents, currency: row.currency)
                    .lineLimit(1)
                    .fixedSize()
                // a linked booking: what this link takes of the refund
                if let share = row.shareCents {
                    Text(RefundSum.share(share, currency: row.currency))
                        .appFont(.caption, monospacedDigit: true)
                        .foregroundStyle(share == 0 ? Theme.warning : Theme.secondary)
                        .lineLimit(1)
                        .fixedSize()
                        .sensitive(.amounts)
                        .help(share == 0 ? "Die Erstattung ist bereits vollständig anderen Buchungen zugeordnet." : "So viel der Erstattung entfällt auf diese Verknüpfung.")
                }
            }
        }
    }
}

/// What the links of a booking add up to, from the core's figures (`RefundLinks`): an original
/// is never refunded beyond its amount and a refund counts once, so a refund that is larger than
/// its booking leaves a rest for further bookings. Nothing when currencies differ (#569).
enum RefundSum {
    /// "Zugeordnet: 12,00 €" below the amount of a linked booking.
    static func share(_ cents: Int64, currency: String) -> String {
        String(localized: "Zugeordnet: \(Money.format(cents, currency: currency.isEmpty ? "EUR" : currency.uppercased()))")
    }

    static func lines(booking: Booking, links: RefundLinks) -> [String] {
        let currency = booking.currency.isEmpty ? "EUR" : booking.currency.uppercased()
        let money = { (cents: Int64) in Money.format(cents, currency: currency) }
        var out: [String] = []
        if let refunded = links.refundedCents {
            out.append(String(localized: "Erstattet: \(money(refunded)) von \(money(abs(booking.amountCents)))"))
            let total = links.refunds.reduce(Int64(0)) { $0 + $1.amountCents }
            if total > refunded {
                out.append(String(localized: "Die Erstattungen betragen zusammen \(money(total)). Was über diese Buchung hinausgeht, zählt hier nicht und kann weiteren Buchungen zugeordnet werden."))
            }
        }
        if let assigned = links.assignedCents {
            out.append(String(localized: "Zugeordnet: \(money(assigned)) von \(money(booking.amountCents))"))
            if assigned < booking.amountCents {
                out.append(String(localized: "Noch offen: \(money(booking.amountCents - assigned)). Sie können weitere Buchungen zuordnen."))
            }
        }
        return out
    }
}

/// "Ursprüngliche Buchung zuordnen …" / "Erstattung zuordnen …" (#714): the linked bookings on
/// top, then up to three suggestions of the core (same amount, same merchant within two months,
/// same reference), below them the searchable list of the other bookings (other direction, all
/// accounts; without a search term twelve months). A click links or unlinks at once; the sheet stays open so
/// that an original can take several partial refunds and a refund several originals. Layout and rules: `docs/ERSTATTUNGEN_UI.md`.
struct RefundPickerSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let booking: Booking
    @State private var links: RefundLinks?
    @State private var candidates: [RefundBooking] = []
    @State private var top: [RefundSuggestion] = []
    @State private var loaded = false
    @State private var search = ""
    /// The outgoing parts of a split booking the sheet was opened from, and the one the next
    /// refund is linked to.
    @State private var ownParts: [(part: UInt32, split: TransactionSplit)] = []
    @State private var ownPart: UInt32 = 0
    @State private var loadError: String?
    /// Counts the loads: only the newest one may show its result.
    @State private var loads = 0

    /// The sheet was opened from a refund: its originals are chosen. Otherwise refunds are collected.
    private var fromRefund: Bool { booking.amountCents > 0 }
    private var linked: [RefundBooking] { (fromRefund ? links?.originals : links?.refunds) ?? [] }
    private var isSearching: Bool { !search.trimmingCharacters(in: .whitespaces).isEmpty }
    /// The list below the suggestions; a search shows every hit, also a suggested one.
    private var others: [RefundBooking] {
        var ids = Set(linked.map(Self.key))
        if !isSearching { ids.formUnion(top.map { Self.key($0.booking) }) }
        return candidates.filter { !ids.contains(Self.key($0)) }
    }
    /// A split booking appears once per part.
    private static func key(_ r: RefundBooking) -> String { "\(r.id)-\(r.part)" }
    private func partTitle(_ p: (part: UInt32, split: TransactionSplit)) -> String {
        let category = p.split.categoryId.flatMap { id in model.categories.first { $0.id == id }?.name } ?? ""
        return ([String(localized: "Teil \(Int(p.part))"), category, p.split.note].filter { !$0.isEmpty } + [Money.format(p.split.amountCents, currency: booking.currency.isEmpty ? "EUR" : booking.currency.uppercased())])
            .joined(separator: " · ")
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
                     ? "Wählen Sie die Buchungen, die mit dieser Erstattung zurückgezahlt wurden, auch mehrere. Zur Auswahl stehen Ausgaben aller Konten aus den zwölf Monaten davor, mit der Suche auch ältere."
                     : "Wählen Sie die Erstattungen zu dieser Buchung, auch mehrere Teilerstattungen. Zur Auswahl stehen Eingänge aller Konten aus den zwölf Monaten danach, mit der Suche auch spätere.")
                    .appFont(.callout)
                    .foregroundStyle(Theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Text("Die Verknüpfung dient nur der Zuordnung: Beträge, Kategorien und Kontostände ändern sich nicht.")
                    .appFont(.caption)
                    .foregroundStyle(Theme.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                if !ownParts.isEmpty {
                    Picker("Erstattung für", selection: $ownPart) {
                        ForEach(ownParts, id: \.part) { Text(partTitle($0)).tag($0.part) }
                    }
                    .sensitive([.labels, .amounts])
                }
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
            if loaded && linked.isEmpty && top.isEmpty && others.isEmpty && loadError == nil {
                Group {
                    if !isSearching {
                        ContentUnavailableView {
                            Label("Keine passenden Buchungen", systemImage: "arrow.uturn.backward")
                        } description: {
                            Text(fromRefund
                                 ? "In den zwölf Monaten vor dieser Erstattung gibt es keine Ausgabe."
                                 : "In den zwölf Monaten nach dieser Buchung gibt es keinen Eingang.")
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
                    ForEach(linked, id: \.self) { row($0, isLinked: true) }
                } header: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(fromRefund ? "Verknüpfte ursprüngliche Buchungen (\(linked.count))" : "Verknüpfte Erstattungen (\(linked.count))")
                        if let links {
                            ForEach(RefundSum.lines(booking: booking, links: links), id: \.self) {
                                Text($0).sensitive(.amounts).fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
            if links?.openCents == 0, !(top.isEmpty && others.isEmpty) {
                Label(fromRefund ? "Die Erstattung ist vollständig zugeordnet. Heben Sie eine Verknüpfung auf, um eine andere Buchung zu wählen."
                          : "Die Buchung ist vollständig erstattet. Heben Sie eine Verknüpfung auf, um eine andere Erstattung zu wählen.",
                      systemImage: "checkmark.circle")
                    .appFont(.caption)
                    .foregroundStyle(Theme.secondary)
            }
            if !top.isEmpty {
                Section {
                    ForEach(top, id: \.booking) { row($0.booking, isLinked: false, reason: $0.reason) }
                } header: {
                    Text("Vorschläge")
                }
            }
            if !others.isEmpty {
                Section {
                    ForEach(others, id: \.self) { row($0, isLinked: false) }
                } header: {
                    Text(isSearching ? "Suchergebnisse" : "Weitere Buchungen")
                }
            }
        }
    }

    private static func label(_ reason: RefundMatch) -> LocalizedStringKey {
        switch reason {
        case .sameAmount: "Gleicher Betrag"
        case .sameMerchant: "Gleicher Händler"
        case .sameReference: "Gleiche Referenz"
        }
    }

    /// The whole row is one button, the circle in front included.
    /// Why an unlinked row cannot be chosen: this booking or that one has nothing left to link.
    private func blocked(_ r: RefundBooking) -> LocalizedStringKey? {
        if links?.openCents == 0 {
            return fromRefund ? "Die Erstattung ist bereits vollständig zugeordnet." : "Die Buchung ist bereits vollständig erstattet."
        }
        if r.openCents == 0 {
            return fromRefund ? "Diese Buchung ist bereits vollständig erstattet." : "Diese Erstattung ist bereits vollständig zugeordnet."
        }
        return nil
    }

    private func row(_ r: RefundBooking, isLinked: Bool, reason: RefundMatch? = nil) -> some View {
        let blocked = isLinked ? nil : blocked(r)
        return Button { toggle(r, isLinked: isLinked) } label: {
            HStack(spacing: 10) {
                Image(systemName: isLinked ? "checkmark.circle.fill" : "circle")
                    .appFont(.title3)
                    .foregroundStyle(isLinked ? Theme.accent : Theme.tertiary)
                    .accessibilityHidden(true)
                RefundBookingLine(row: r, showsPurpose: true)
                if let reason {
                    Text(Self.label(reason))
                        .appFont(.caption2, weight: .medium)
                        .foregroundStyle(Theme.accent)
                        .padding(.horizontal, 6).padding(.vertical, 2)
                        .background(Theme.accent.opacity(0.12), in: Capsule())
                        .fixedSize()
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(blocked != nil)
        .opacity(blocked == nil ? 1 : 0.45)
        .help(blocked ?? (isLinked ? "Verknüpfung aufheben" : "Mit dieser Buchung verknüpfen"))
        .accessibilityAddTraits(isLinked ? .isSelected : [])
    }

    private func toggle(_ r: RefundBooking, isLinked: Bool) {
        let refundID = fromRefund ? booking.id : r.id
        let originalID = fromRefund ? r.id : booking.id
        if isLinked {
            model.unlinkRefund(refundID: refundID, originalID: originalID, part: r.part)
        } else {
            model.linkRefund(refundID: refundID, originalID: originalID, part: fromRefund ? r.part : ownPart)
        }
    }

    private func reload() async {
        loads += 1
        let run = loads
        let id = booking.id
        let query = search
        do {
            let outgoing = !fromRefund
            let result = try await model.vaultOffMain { vault -> (RefundLinks, [RefundSuggestion], [RefundBooking], [TransactionSplit]) in
                (try vault.refundLinks(transactionId: id), try vault.refundSuggestions(transactionId: id),
                 try vault.refundCandidates(transactionId: id, query: query, limit: 200),
                 outgoing ? try vault.listTransactionSplits(transactionId: id) : [])
            }
            guard run == loads, let result else { return }
            links = result.0
            // a linked booking is shown on top only
            let linkedIDs = Set(linked.map(Self.key))
            top = result.1.filter { !linkedIDs.contains(Self.key($0.booking)) }
            candidates = result.2
            // parts count from 1 in the order of the split; only outgoing ones take a refund
            ownParts = result.3.enumerated().map { (part: UInt32($0.offset + 1), split: $0.element) }.filter { $0.split.amountCents < 0 }
            if !ownParts.contains(where: { $0.part == ownPart }) { ownPart = ownParts.first?.part ?? 0 }
            loadError = nil
        } catch {
            guard run == loads else { return }
            loadError = describe(error)
        }
        loaded = true
    }
}

/// Inspector card of a linked booking (#714): a refund shows its originals, an original its
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
                Button("Bearbeiten …") { model.openRefundPicker(for: booking) }
                    .buttonStyle(.borderless)
                    .appFont(.caption)
            }
            if let originals = links?.originals, !originals.isEmpty {
                Text(originals.count == 1 ? "Ursprüngliche Buchung" : "Ursprüngliche Buchungen (\(originals.count))")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
                ForEach(originals, id: \.self) { line($0, refundID: booking.id, originalID: $0.id) }
            }
            if let refunds = links?.refunds, !refunds.isEmpty {
                Text(refunds.count == 1 ? "Erstattung dieser Buchung" : "Erstattungen dieser Buchung (\(refunds.count))")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
                ForEach(refunds, id: \.self) { line($0, refundID: $0.id, originalID: booking.id) }
            }
            if let links {
                ForEach(RefundSum.lines(booking: booking, links: links), id: \.self) {
                    Text($0)
                        .sensitive(.amounts)
                        .appFont(.caption)
                        .foregroundStyle(Theme.secondary)
                        .fixedSize(horizontal: false, vertical: true)
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

    private func line(_ r: RefundBooking, refundID: Int64, originalID: Int64) -> some View {
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
            Button { model.unlinkRefund(refundID: refundID, originalID: originalID, part: r.part) } label: {
                Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.tertiary)
            }
            .buttonStyle(.borderless)
            .help("Verknüpfung aufheben")
            .accessibilityLabel("Verknüpfung aufheben")
        }
    }
}
