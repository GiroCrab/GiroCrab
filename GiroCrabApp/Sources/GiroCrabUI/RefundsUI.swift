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
                Button("Bearbeiten …") { model.openRefundPicker(for: booking) }.appFont(.caption)
            }
            if let loaded, loaded.key == key {
                ForEach(loaded.data.links, id: \.self) { link in
                    HStack(alignment: .top) {
                        let id = link.refundId == booking.id ? link.originalId : link.refundId
                        if let counterpart = loaded.data.counterparts[id] {
                            Button { model.revealBooking(id) } label: {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(counterpart.merchantName.isEmpty ? counterpart.counterparty : counterpart.merchantName)
                                        .appFont(.body).lineLimit(2).sensitive(.personal)
                                    Text(DateText.medium(counterpart.bookingDate)).appFont(.caption)
                                    Text(counterpart.accountName)
                                        .appFont(.caption).lineLimit(2).sensitive(.personal)
                                    let part = link.refundId == booking.id ? link.originalPart : link.refundPart
                                    if !part.isEmpty {
                                        Text(link.refundId == booking.id ? link.originalLabel : link.refundLabel)
                                            .appFont(.caption).sensitive([.personal, .labels, .amounts])
                                    }
                                    Text(Money.format(link.allocatedCents, currency: counterpart.currency))
                                        .appFont(.caption, monospacedDigit: true).sensitive(.amounts)
                                }
                            }.buttonStyle(.plain).help("Verknüpfte Buchung anzeigen")
                        }
                        Spacer()
                        Button("Verknüpfung aufheben", systemImage: "xmark") {
                            Task { await model.unlinkRefund(RefundEndpointPair(link)) }
                        }.labelStyle(.iconOnly).disabled(model.refundBusy)
                    }
                }
            }
        }
        .card()
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
}

/// Link-only display: `docs/ERSTATTUNGEN_UI.md`.
private struct RefundLinkSummary: View {
    let link: RefundLink
    let paysBack: Bool
    let currency: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(paysBack ? link.originalLabel : link.refundLabel)
                .appFont(.body).lineLimit(2).sensitive([.personal, .labels])
            LabeledContent("Zugeordnet") { amount(link.allocatedCents) }
            LabeledContent("Erstattung offen") { amount(link.refundOpenCents) }
            LabeledContent("Ursprüngliche Buchung offen") { amount(link.originalOpenCents) }
        }
        .appFont(.caption)
    }

    private func amount(_ cents: Int64) -> some View {
        Text(Money.format(cents, currency: currency))
            .appFont(.caption, monospacedDigit: true).fixedSize().sensitive(.amounts)
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
                Text(paysBack ? "Ursprüngliche Buchung zuordnen" : "Erstattung zuordnen").appFont(.headline)
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
                Text(paysBack ? "Wählen Sie die ursprünglichen Buchungen, auch mehrere Teilerstattungen."
                     : "Wählen Sie die Erstattungen, die diese Buchung zurückzahlen.")
                    .appFont(.callout).foregroundStyle(Theme.secondary).fixedSize(horizontal: false, vertical: true)
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
        .macFrame(minWidth: 560, idealWidth: 620, minHeight: 680)
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
                        HStack(alignment: .top) {
                            RefundLinkSummary(link: link, paysBack: data.paysBack, currency: data.booking.currency)
                            Spacer()
                            Button("Verknüpfung aufheben", systemImage: "xmark") {
                                Task { await model.unlinkRefund(RefundEndpointPair(link)) }
                            }
                            .labelStyle(.iconOnly)
                            .disabled(model.refundBusy)
                        }
                        .padding(.vertical, 4)
                    }
                } header: { Text("Verknüpfte Buchungen") }
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
                    if others.isEmpty && data.suggestions.isEmpty {
                        Text("Keine passenden Buchungen").appFont(.callout).foregroundStyle(Theme.secondary)
                    }
                } else {
                    ProgressView()
                }
            } header: { Text("Weitere Buchungen") }
        }
        .disabled(model.refundBusy)
    }

    private func candidate(_ row: RefundCandidate, data: RefundSnapshot, reason: RefundMatch? = nil) -> some View {
        Button {
            let pair = RefundEndpointPair(bookingID: booking.id, ownPart: ownPart, paysBack: data.paysBack, candidate: row)
            Task { await model.linkRefund(pair) }
        } label: {
            HStack(alignment: .firstTextBaseline) {
                Image(systemName: "plus.circle").foregroundStyle(Theme.accent)
                VStack(alignment: .leading, spacing: 3) {
                    Text(row.merchantName.isEmpty ? row.counterparty : row.merchantName)
                        .appFont(.body).foregroundStyle(Theme.text).lineLimit(2).sensitive()
                    Text(DateText.medium(row.bookingDate)).appFont(.caption).foregroundStyle(Theme.secondary)
                    if !row.part.isEmpty {
                        Text(row.partLabel).appFont(.caption).lineLimit(2).sensitive([.personal, .labels])
                    }
                    if let reason { Text(Self.reason(reason)).appFont(.caption2).foregroundStyle(Theme.accent) }
                }
                Spacer(minLength: 8)
                AmountLabel(cents: row.amountCents, currency: row.currency).fixedSize().sensitive(.amounts)
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
