import SwiftUI
import GiroCrabCore

/// Which depot position gets a purchase price set by hand (#1987).
struct PurchasePriceRequest: Identifiable {
    let accountId: Int64
    let isin: String
    let wkn: String
    let name: String
    let currency: String
    /// How the price is quoted: per unit, or (bonds) in percent of the nominal amount.
    let basis: PriceBasis
    /// As stored (per unit in the account currency, or percent × 100); 0 = none stored.
    let currentCents: Int64
    var id: String { "\(accountId)-\(isin)-\(wkn)" }
}

/// Sets the purchase price of a depot position (`docs/DEPOT_UND_VERMOEGEN_UI.md`): per unit, for
/// bonds in percent of the nominal amount (the way their price is stored). An empty field removes
/// it; the depot's gain is computed from it when the bank reports none.
struct PurchasePriceSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let request: PurchasePriceRequest
    let onSaved: () -> Void
    @State private var text = ""
    @State private var busy = false
    @State private var error: String?
    /// The vault the sheet was opened for: a save never reaches another one.
    @State private var pin: VaultPin?

    private var label: String {
        request.basis == .percentOfNominal
            ? String(localized: "Einstandskurs in Prozent des Nennwerts")
            : String(localized: "Einstandskurs pro Stück (\(request.currency))")
    }
    private var trimmed: String { text.trimmingCharacters(in: .whitespaces) }
    /// nil while the field holds no valid amount; an empty field is 0 (remove).
    private var cents: Int64? {
        if trimmed.isEmpty { return 0 }
        guard let c = Money.parseCents(trimmed), c >= 0 else { return nil }
        return c
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(request.name).sensitive(.labels).appFont(.headline)
            Form {
                TextField(label, text: $text,
                          prompt: Text("z. B. \(Money.plain(request.basis == .percentOfNominal ? 98_50 : 44_49))"))
                    .sensitive(.amounts)
                    .onSubmit(save)
            }
            Text("Gilt, solange die Bank keinen eigenen Einstandskurs liefert. Ein leeres Feld entfernt den Wert.")
                .appFont(.caption).foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if cents == nil {
                Text("Bitte einen Betrag von 0 oder mehr eingeben.").foregroundStyle(Theme.negative).appFont(.caption)
            }
            if let error { Text(error).foregroundStyle(Theme.negative).appFont(.caption) }
            HStack {
                Button("Abbrechen", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction).disabled(busy)
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Speichern", action: save).buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction).disabled(busy || cents == nil || pin == nil)
            }
        }
        .padding(20).macFrame(width: 460)
        .interactiveDismissDisabled(busy)
        .onAppear {
            text = request.currentCents > 0 ? Money.plain(request.currentCents) : ""
            pin = model.pinVault()
        }
    }

    private func save() {
        guard let cents, let pin, !busy else { return }
        guard model.isCurrent(pin) else { dismiss(); return }
        let r = request
        busy = true
        Task {
            do {
                try await pin.run {
                    try $0.setHoldingPurchasePrice(accountId: r.accountId, isin: r.isin, wkn: r.wkn, purchasePriceCents: cents)
                }
                busy = false
                if model.isCurrent(pin) { onSaved() }
                dismiss()
            } catch {
                busy = false
                self.error = describe(error)
            }
        }
    }
}
