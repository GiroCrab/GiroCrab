import SwiftUI
import GiroCrabCore

/// Which depot position gets a purchase price set by hand (#1987).
struct PurchasePriceRequest: Identifiable {
    let accountId: Int64
    let isin: String
    let wkn: String
    let name: String
    let currency: String
    /// Per unit in the account currency; 0 = none stored.
    let currentCents: Int64
    var id: String { "\(accountId)-\(isin)-\(wkn)" }
}

/// Sets the purchase price per unit of a depot position (`docs/DEPOT_UND_VERMOEGEN_UI.md`).
/// An empty field removes it; the depot's gain is computed from it when the bank reports none.
struct PurchasePriceSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let request: PurchasePriceRequest
    let onSaved: () -> Void
    @State private var text = ""
    @State private var busy = false
    @State private var error: String?

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
                TextField("Einstandskurs pro Stück (\(request.currency))", text: $text,
                          prompt: Text("z. B. \(Money.plain(44_49))"))
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
                Button("Abbrechen", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction)
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Speichern", action: save).buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction).disabled(busy || cents == nil)
            }
        }
        .padding(20).macFrame(width: 460)
        .interactiveDismissDisabled(busy)
        .onAppear { text = request.currentCents > 0 ? Money.plain(request.currentCents) : "" }
    }

    private func save() {
        guard let cents, !busy else { return }
        let r = request
        busy = true
        Task {
            do {
                let done: Void? = try await model.vaultOffMain {
                    try $0.setHoldingPurchasePrice(accountId: r.accountId, isin: r.isin, wkn: r.wkn, purchasePriceCents: cents)
                }
                busy = false
                if done != nil { onSaved(); dismiss() }
            } catch {
                busy = false
                self.error = describe(error)
            }
        }
    }
}
