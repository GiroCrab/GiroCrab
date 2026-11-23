import SwiftUI
import GiroCrabCore

/// Entry points: `docs/ERSTATTUNGEN_UI.md`.
struct RefundLinkMenu: View {
    let model: AppModel
    let booking: Booking

    var body: some View {
        if booking.splitCount > 0 || booking.amountCents != 0 {
            Button("Erstattung zuordnen …", systemImage: "arrow.uturn.backward") { model.openRefundPicker(for: booking) }
        }
    }
}

/// Layout and privacy: `docs/ERSTATTUNGEN_UI.md`.
struct RefundLinksCard: View {
    @Environment(AppModel.self) private var model
    let booking: Booking
    @State private var loaded: (key: RefundLoadKey, data: RefundCardSnapshot)?

    private var key: RefundLoadKey {
        RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: booking.id, ownPart: "", revision: model.refundRevision)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label("Erstattungen", systemImage: "arrow.uturn.backward").appFont(.subheadline, weight: .semibold)
                Spacer()
                Button("Bearbeiten …") { model.openRefundPicker(for: booking) }
                    .buttonStyle(.borderless).appFont(.caption)
            }
            if let loaded, loaded.key == key {
                ForEach([true, false], id: \.self) { paysBack in
                    let links = loaded.data.links.filter { ($0.refundId == booking.id) == paysBack }
                    if !links.isEmpty {
                        Text(paysBack
                             ? (links.count == 1 ? String(localized: "Ursprüngliche Buchung") : String(localized: "Ursprüngliche Buchungen"))
                             : (links.count == 1 ? String(localized: "Erstattung") : String(localized: "Erstattungen")))
                            .appFont(.caption).foregroundStyle(Theme.secondary)
                        ForEach(links, id: \.self) { link in
                            inspectorRow(link, data: loaded.data, paysBack: paysBack)
                        }
                        let summary = RefundAllocationSummary(links: links, paysBack: paysBack)
                        Text("Zugeordnet \(Money.format(summary.assigned, currency: booking.currency)) von \(Money.format(summary.total, currency: booking.currency)) · noch offen \(Money.format(summary.open, currency: booking.currency))")
                            .appFont(.caption).foregroundStyle(Theme.secondary)
                            .fixedSize(horizontal: false, vertical: true).sensitive(.amounts)
                    }
                }
            }
        }
        .flatSection()
        .task(id: key) {
            let requested = key
            do {
                let result = try await model.refundCard(for: requested)
                guard !Task.isCancelled, key == requested else { return }
                loaded = result.map { (requested, $0) }
            } catch {
                guard !Task.isCancelled, key == requested else { return }
                model.errorMessage = describe(error)
            }
        }
    }

    private func inspectorRow(_ link: RefundLink, data: RefundCardSnapshot, paysBack: Bool) -> some View {
        let id = paysBack ? link.originalId : link.refundId
        let counterpart = data.counterparts[id]
        let part = paysBack ? link.resolvedOriginalPart : link.resolvedRefundPart
        return HStack(alignment: .top, spacing: 6) {
            Button { model.revealBooking(id) } label: {
                HStack(spacing: 6) {
                    RefundBookingLine(booking: counterpart, partLabel: part?.isEmpty == false ? (paysBack ? link.originalLabel : link.refundLabel) : "",
                        allocated: link.allocatedCents, fallbackCurrency: booking.currency, inspector: true)
                    Image(systemName: "chevron.right").appFont(.caption2, weight: .semibold)
                        .foregroundStyle(Theme.tertiary)
                }.contentShape(Rectangle())
            }.buttonStyle(.plain).help("Verknüpfte Buchung anzeigen").disabled(counterpart == nil)
            Button("Verknüpfung aufheben", systemImage: "xmark.circle.fill") {
                Task { await model.unlinkRefund(RefundEndpointPair(link)) }
            }.labelStyle(.iconOnly).buttonStyle(.borderless).foregroundStyle(Theme.tertiary)
                .disabled(model.refundBusy)
        }
    }
}

/// Booking identity and allocation hierarchy: `docs/ERSTATTUNGEN_UI.md`.
private struct RefundBookingLine: View {
    let name: String
    let date: String
    let account: String
    var purpose = ""
    var partLabel = ""
    let cents: Int64?
    let currency: String
    var allocated: Int64?
    var open: Int64 = 0
    var inspector = false

    init(booking: Booking?, partLabel: String, allocated: Int64, open: Int64 = 0,
         fallbackCurrency: String, inspector: Bool = false) {
        name = booking.map { $0.merchantName.isEmpty ? $0.counterparty : $0.merchantName } ?? ""
        date = booking?.bookingDate ?? ""
        account = booking?.accountName ?? ""
        purpose = booking?.purpose ?? ""
        self.partLabel = partLabel
        cents = booking?.amountCents
        currency = booking?.currency ?? fallbackCurrency
        self.allocated = allocated
        self.open = open
        self.inspector = inspector
    }

    init(candidate: RefundCandidate, account: String) {
        name = candidate.merchantName.isEmpty ? candidate.counterparty : candidate.merchantName
        date = candidate.bookingDate
        self.account = account
        partLabel = candidate.part.isEmpty ? "" : candidate.partLabel
        cents = candidate.amountCents
        currency = candidate.currency
    }

    var body: some View {
        if inspector {
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    identity
                    Spacer(minLength: 0)
                    amount
                }
                context.fixedSize(horizontal: false, vertical: true)
                allocation
            }.frame(maxWidth: .infinity, alignment: .leading)
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                VStack(alignment: .leading, spacing: 2) {
                    identity
                    context.lineLimit(1)
                    if !purpose.isEmpty {
                        Text(purpose).appFont(.caption).foregroundStyle(Theme.tertiary).lineLimit(1).sensitive()
                    }
                    if open > 0 {
                        Text("Noch offen: \(Money.format(open, currency: currency))")
                            .appFont(.caption).foregroundStyle(Theme.secondary).sensitive(.amounts)
                    }
                }.frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
                VStack(alignment: .trailing, spacing: 2) {
                    amount
                    allocation
                }
            }
        }
    }

    private var identity: some View {
        Text((name.isEmpty ? String(localized: "Ohne Namen") : name) + (partLabel.isEmpty ? "" : " · " + partLabel))
            .appDefaultFont().foregroundStyle(Theme.text).lineLimit(2).sensitive([.personal, .labels, .amounts])
    }

    private var context: some View {
        Text([date.isEmpty ? "" : DateText.medium(date), account].filter { !$0.isEmpty }.joined(separator: " · "))
            .appFont(.caption).foregroundStyle(Theme.secondary).sensitive()
    }

    @ViewBuilder private var amount: some View {
        if let cents { AmountLabel(cents: cents, currency: currency).fixedSize().sensitive(.amounts) }
    }

    @ViewBuilder private var allocation: some View {
        if let allocated {
            Text("Zugeordnet: \(Money.format(allocated, currency: currency))")
                .appFont(.caption, monospacedDigit: true).foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: true, vertical: false).sensitive(.amounts)
        }
    }
}

/// Row layout: `docs/ERSTATTUNGEN_UI.md`.
struct RefundLinkMarker: View {
    let booking: Booking

    private func partial(_ role: String) -> Int64? {
        let covered = role == "original" ? booking.refundedCents : booking.refundCents
        return covered > 0 && (booking.splitCount > 0 || covered < abs(booking.amountCents)) ? covered : nil
    }

    var body: some View {
        if booking.refundMarker != "none" {
            HStack(spacing: 3) {
                ForEach(booking.refundMarker == "both" ? ["refund", "original"] : [booking.refundMarker], id: \.self) { role in
                    Image(systemName: role == "refund" ? "arrow.uturn.backward" : "arrow.uturn.forward")
                        .appFont(.caption, weight: .semibold)
                        .accessibilityLabel("Verknüpfte Erstattung")
                    if let partial = partial(role) {
                        Text(Money.format(partial, currency: booking.currency))
                            .appFont(.caption, monospacedDigit: true)
                            .fixedSize()
                            .sensitive(.amounts)
                    }
                }
            }
            .foregroundStyle(Theme.secondary)
            .help("Mit einer Erstattung verknüpft")
        }
    }
}

/// Layout and interaction: `docs/ERSTATTUNGEN_UI.md`.
struct RefundPickerSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @Environment(\.textScale) private var textScale
    let booking: Booking
    @State private var ownPart = ""
    @State private var query = ""
    @State private var state = RefundPickerState()
    @State private var parts: [TransactionSplit] = []
    @State private var loadError: String?
    @State private var candidateError: String?

    private var key: RefundLoadKey {
        RefundLoadKey(vaultRevision: model.vaultRevision, bookingID: booking.id,
                      ownPart: ownPart, revision: model.refundRevision)
    }
    private var snapshot: RefundSnapshot? { state.snapshot(for: key) }
    private var candidateKey: RefundCandidateKey? {
        snapshot.map { _ in RefundCandidateKey(context: key, query: query) }
    }
    private var paysBack: Bool { parts.first { $0.uuid == ownPart }.map { $0.amountCents > 0 } ?? (booking.amountCents > 0) }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(paysBack ? String(localized: "Ursprüngliche Buchung zuordnen") : String(localized: "Erstattung zuordnen")).appFont(.headline)
                Spacer()
                Button("Fertig") { dismiss() }.keyboardShortcut(.defaultAction)
            }
            .padding(16)
            Divider()
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(booking.merchantName.isEmpty ? booking.counterparty : booking.merchantName)
                        .appFont(.body, weight: .semibold).lineLimit(2).sensitive()
                    Spacer()
                    AmountLabel(cents: booking.amountCents, currency: booking.currency).fixedSize().sensitive(.amounts)
                }
                Text("\(DateText.medium(booking.bookingDate)) · \(model.account(for: booking.accountId)?.name ?? booking.accountName)")
                    .appFont(.caption).foregroundStyle(Theme.secondary).sensitive()
                Text(paysBack ? String(localized: "Wählen Sie die ursprünglichen Buchungen, auch mehrere Teilerstattungen.")
                     : String(localized: "Wählen Sie die Erstattungen, die diese Buchung zurückzahlen."))
                    .appFont(.callout).foregroundStyle(Theme.secondary).fixedSize(horizontal: false, vertical: true)
                Text("Die Verknüpfung lässt Buchungen und Kontostände unverändert. Auswertungen berücksichtigen die Erstattung.")
                    .appFont(.caption).foregroundStyle(Theme.secondary)
                if !parts.isEmpty {
                    Picker("Teil der Buchung", selection: $ownPart) {
                        ForEach(parts.filter { $0.amountCents != 0 }, id: \.uuid) { part in
                            Text(partTitle(part)).tag(part.uuid)
                        }
                    }
                    .sensitive([.personal, .labels, .amounts])
                }
                TextField("Name oder Verwendungszweck suchen", text: $query)
                    .textFieldStyle(.roundedBorder).sensitive()
                if let undo = model.refundUndo {
                    HStack {
                        Text("Verknüpfung aufgehoben").appFont(.caption)
                        Button("Rückgängig") { Task { await model.restoreRefund(undo) } }
                            .disabled(model.refundBusy)
                    }
                }
                if let loadError { Text(loadError).appFont(.caption).foregroundStyle(Theme.warning) }
            }
            .padding(16)
            if let snapshot {
                list(snapshot)
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(idealHeight: (snapshot.map { emptyResults($0) } == true ? 520 : 680) * textScale)
        .macFrame(minWidth: 540, idealWidth: 580, minHeight: 520)
        .task(id: key) { await reload(key) }
        .task(id: candidateKey) {
            guard let candidateKey, let snapshot else { return }
            await reloadCandidates(candidateKey, snapshot: snapshot)
        }
        #if os(macOS)
        .onExitCommand { dismiss() }
        #endif
    }

    private func partTitle(_ part: TransactionSplit) -> String {
        let category = part.categoryId.flatMap { id in model.categories.first { $0.id == id }?.name } ?? ""
        return [category, part.note, Money.format(part.amountCents, currency: booking.currency)]
            .filter { !$0.isEmpty }.joined(separator: " · ")
    }

    private func list(_ data: RefundSnapshot) -> some View {
        List {
            if !data.links.isEmpty {
                Section {
                    ForEach(data.links, id: \.self) { link in
                        Button {
                                Task { await model.unlinkRefund(RefundEndpointPair(link)) }
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "checkmark.circle.fill").appFont(.title3).foregroundStyle(Theme.accent)
                                RefundBookingLine(booking: data.counterparts[data.paysBack ? link.originalId : link.refundId],
                                    partLabel: (data.paysBack ? link.resolvedOriginalPart : link.resolvedRefundPart)?.isEmpty == false
                                        ? (data.paysBack ? link.originalLabel : link.refundLabel) : "",
                                    allocated: link.allocatedCents,
                                    open: data.paysBack ? link.originalOpenCents : link.refundOpenCents,
                                    fallbackCurrency: data.booking.currency)
                            }.contentShape(Rectangle()).padding(.vertical, 4)
                        }
                        .buttonStyle(.plain).disabled(model.refundBusy).help("Verknüpfung aufheben")
                    }
                } header: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(data.paysBack ? String(localized: "Verknüpfte ursprüngliche Buchungen (\(data.links.count))") : String(localized: "Verknüpfte Erstattungen (\(data.links.count))"))
                            .appFont(.caption, weight: .semibold)
                        Text("Zugeordnet: \(Money.format(abs(data.amount) - data.ownOpen, currency: data.booking.currency)) von \(Money.format(abs(data.amount), currency: data.booking.currency))")
                            .appFont(.caption).sensitive(.amounts)
                        if data.ownOpen > 0 {
                            Text("Noch offen: \(Money.format(data.ownOpen, currency: data.booking.currency)). Sie können weitere Buchungen zuordnen.")
                                .appFont(.caption).sensitive(.amounts)
                        }
                    }.textCase(nil)
                }
            }
            if data.ownOpen == 0 {
                Text("Dieser Teil ist vollständig zugeordnet.").appFont(.callout).foregroundStyle(Theme.secondary)
            }
            if !data.suggestions.isEmpty {
                Section {
                    ForEach(data.suggestions, id: \.candidate) { suggestion in
                        candidate(suggestion.candidate, data: data, reason: suggestion.reason)
                    }
                } header: { Text("Vorschläge") }
            }
            Section {
                if let candidateError {
                    Text(candidateError).appFont(.caption).foregroundStyle(Theme.warning)
                } else if let candidateKey, let rows = state.candidates(for: candidateKey) {
                    let suggested = Set(data.suggestions.map(\.candidate))
                    let others = rows.filter { !query.isEmpty || !suggested.contains($0) }
                    ForEach(others, id: \.self) { candidate($0, data: data) }
                    if others.isEmpty && data.suggestions.isEmpty && !data.links.isEmpty {
                        emptySearch.padding(.vertical, 20)
                    }
                } else {
                    ProgressView()
                }
            } header: { Text(query.isEmpty ? String(localized: "Weitere Buchungen") : String(localized: "Suchergebnisse")) }
        }
        .overlay {
            if emptyResults(data) && data.links.isEmpty { emptySearch }
        }
        .disabled(model.refundBusy)
    }

    private func emptyResults(_ data: RefundSnapshot) -> Bool {
        guard candidateError == nil, let candidateKey, let rows = state.candidates(for: candidateKey) else { return false }
        return rows.isEmpty && data.suggestions.isEmpty
    }

    private var emptySearch: some View {
        VStack(spacing: 10) {
            Image(systemName: "magnifyingglass").appFont(.largeTitle).foregroundStyle(Theme.tertiary)
            Text("Keine passenden Buchungen").appFont(.title2, weight: .semibold)
            Text("Prüfen Sie die Schreibweise oder versuchen Sie eine andere Suche.")
                .appFont(.callout).foregroundStyle(Theme.secondary).multilineTextAlignment(.center)
        }.padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func candidate(_ row: RefundCandidate, data: RefundSnapshot, reason: RefundMatch? = nil) -> some View {
        Button {
            let pair = RefundEndpointPair(bookingID: booking.id, ownPart: ownPart, paysBack: data.paysBack, candidate: row)
            Task { await model.linkRefund(pair) }
        } label: {
            HStack(spacing: 10) {
                Image(systemName: "circle").appFont(.title3).foregroundStyle(Theme.tertiary)
                RefundBookingLine(candidate: row, account: model.account(for: row.accountId)?.name ?? "")
                if let reason {
                    Text(Self.reason(reason)).appFont(.caption2).foregroundStyle(Theme.accent)
                        .padding(.horizontal, 5).padding(.vertical, 2)
                        .background(Theme.accent.opacity(0.08), in: Capsule())
                }
            }
            .padding(.vertical, 4).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!data.canLink(row))
        .opacity(data.canLink(row) ? 1 : 0.45)
    }

    private static func reason(_ reason: RefundMatch) -> LocalizedStringKey {
        switch reason {
        case .sameAmount: "Gleicher Betrag"
        case .sameMerchant: "Gleicher Händler"
        case .sameReference: "Gleiche Referenz"
        }
    }

    private func reload(_ requested: RefundLoadKey) async {
        guard state.snapshot(for: requested) == nil else { return }
        loadError = nil
        do {
            guard let result = try await model.refundSnapshot(for: requested, booking: booking), !Task.isCancelled, key == requested else { return }
            parts = result.parts
            ownPart = result.selectedPart
            state.loaded = (key, result)
            loadError = nil
        } catch {
            guard !Task.isCancelled, key == requested else { return }
            loadError = describe(error)
        }
    }

    private func reloadCandidates(_ requested: RefundCandidateKey, snapshot: RefundSnapshot) async {
        candidateError = nil
        do {
            try await Task.sleep(for: .milliseconds(250))
            guard let rows = try await model.refundCandidates(for: requested, snapshot: snapshot),
                  !Task.isCancelled, candidateKey == requested else { return }
            state.loadedCandidates = (requested, rows)
        } catch {
            guard !Task.isCancelled, candidateKey == requested else { return }
            candidateError = describe(error)
        }
    }
}
