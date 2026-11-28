import SwiftUI
import GiroCrabCore

// Crypto holdings (#1836, design: docs/DEPOT_UND_VERMOEGEN_UI.md): a depot without a bank whose
// positions (ticker, quantity, purchase price per unit) the user keeps by hand. The depot view shows
// it like any depot; this file adds the dialogs and the bar with "Position hinzufügen …" and
// "Kurse aktualisieren".

@Observable final class CryptoStore {
    var accountIDs: Set<Int64> = []
    var showNewAccount = false
    /// The position sheet on screen.
    var editor: CryptoPositionRequest?
    /// The position whose removal is being asked.
    var deleting: CryptoPositionRequest?
    var refreshing = false
    /// Counts the changes, so the depot view reloads even when the balance stays the same.
    var revision = 0
}

/// A new position (`previousSymbol` nil) or the one held under `previousSymbol`.
struct CryptoPositionRequest: Identifiable {
    let id = UUID()
    let accountId: Int64
    var previousSymbol: String?
    var quantity: Double = 0
    /// Per unit in EUR cents.
    var purchaseCents: Int64?

    static func edit(accountId: Int64, _ p: DepotPosition) -> CryptoPositionRequest {
        CryptoPositionRequest(accountId: accountId, previousSymbol: p.holding.wkn, quantity: p.holding.quantity,
                              purchaseCents: p.holding.purchasePriceCents > 0 ? p.holding.purchasePriceCents : nil)
    }
}

/// Above the positions of a crypto holding.
struct CryptoHoldingBar: View {
    @Environment(AppModel.self) private var model
    let accountId: Int64

    var body: some View {
        HStack(spacing: 12) {
            Button("Position hinzufügen …", systemImage: "plus") {
                model.cryptoStore.editor = CryptoPositionRequest(accountId: accountId)
            }
            Button("Kurse aktualisieren", systemImage: "arrow.clockwise") { model.refreshCryptoPrices() }
                .disabled(model.cryptoStore.refreshing)
            if model.cryptoStore.refreshing { ProgressView().controlSize(.small) }
            Spacer(minLength: 0)
            Text("Kurse in Euro, nur auf Anforderung abgerufen")
                .appFont(.caption).foregroundStyle(Theme.secondary)
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 4)
    }
}

extension View {
    /// The dialogs of the crypto holdings, driven by `model.cryptoStore`.
    func cryptoHoldingSheets(_ model: AppModel) -> some View {
        modifier(CryptoHoldingSheetsModifier(model: model))
    }
}

private struct CryptoHoldingSheetsModifier: ViewModifier {
    @Bindable var model: AppModel

    func body(content: Content) -> some View {
        content
            .capturableSheet(isPresented: Binding(get: { model.cryptoStore.showNewAccount },
                                                  set: { model.cryptoStore.showNewAccount = $0 })) {
                NewCryptoAccountSheet().environment(model)
            }
            .capturableSheet(item: Binding(get: { model.cryptoStore.editor }, set: { model.cryptoStore.editor = $0 })) { request in
                CryptoPositionSheet(request: request).environment(model)
            }
            .alert("Position löschen?", isPresented: Binding(
                get: { model.cryptoStore.deleting != nil },
                set: { if !$0 { model.cryptoStore.deleting = nil } }
            )) {
                Button("Löschen", role: .destructive) {
                    if let r = model.cryptoStore.deleting { model.deleteCryptoPosition(r) }
                    model.cryptoStore.deleting = nil
                }
                Button("Abbrechen", role: .cancel) { model.cryptoStore.deleting = nil }
            } message: {
                Text("Die Position wird aus dem Bestand entfernt.")
            }
            // the requests name accounts of this vault: another vault (demo / own) closes them. The
            // new-holding sheet names none and stays: `runInOwnVault` switches the vault, then opens it.
            .onChange(of: model.vaultRevision) {
                model.cryptoStore.editor = nil
                model.cryptoStore.deleting = nil
                model.cryptoStore.refreshing = false
            }
    }
}

/// "Neuer Crypto-Bestand …": a name, then an empty depot.
struct NewCryptoAccountSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var error: String?
    @State private var busy = false
    /// The vault the sheet was opened for (`runInOwnVault` switches first, then opens it): a later
    /// switch to the demo must not receive the new holding.
    @State private var pin: VaultPin?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Neuer Crypto-Bestand").appFont(.headline)
            Form {
                TextField("Name", text: $name, prompt: Text("z. B. Hardware-Wallet")).sensitive().onSubmit(create)
            }
            // create() works with the name it started with
            .disabled(busy)
            Text("Ein Depot ohne Bank. Die Positionen tragen Sie selbst ein; die Kurse kommen auf Anforderung.")
                .appFont(.caption).foregroundStyle(Theme.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let error { Text(error).foregroundStyle(Theme.negative).appFont(.caption) }
            HStack {
                Button("Abbrechen", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction).disabled(busy)
                Spacer()
                Button("Anlegen", action: create).buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                    .disabled(busy || pin == nil)
            }
        }
        .padding(20).macFrame(width: 420)
        .interactiveDismissDisabled(busy)
        .onAppear { pin = model.pinVault() }
    }

    private func create() {
        guard let pin, !busy else { return }
        let name = name
        busy = true
        Task {
            do {
                let id = try await pin.run { try $0.createCryptoAccount(name: name) }
                busy = false
                if model.isCurrent(pin) {
                    model.cryptoAccountCreated(id)
                } else {
                    model.toast = String(localized: "Der Crypto-Bestand wurde im zuvor geöffneten Tresor angelegt.")
                }
                dismiss()
            } catch {
                busy = false
                self.error = describe(error)
            }
        }
    }
}

/// Adds or changes a position. The ticker's price is fetched first: an unknown ticker is refused,
/// a network problem saves the position without a price and says so.
struct CryptoPositionSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let request: CryptoPositionRequest
    @State private var symbol = ""
    @State private var quantityText = ""
    /// The quantity as prefilled: unchanged, the stored (unrounded) quantity is saved.
    @State private var prefilledQuantity = ""
    @State private var purchaseText = ""
    @State private var busy = false
    @State private var error: String?
    /// The vault the sheet was opened for: a save never reaches another one.
    @State private var pin: VaultPin?

    private var ticker: String { symbol.trimmingCharacters(in: .whitespaces).uppercased() }
    private var quantity: Double? {
        if request.quantity > 0, quantityText == prefilledQuantity { return request.quantity }
        return NumberText.double(quantityText).flatMap { $0 > 0 ? $0 : nil }
    }
    /// `.some(nil)` = no purchase price; nil = not a valid amount.
    private var purchase: Int64?? {
        let t = purchaseText.trimmingCharacters(in: .whitespaces)
        if t.isEmpty { return .some(nil) }
        guard let c = Money.parseCents(t), c >= 0 else { return nil }
        return .some(c)
    }
    private var valid: Bool { !ticker.isEmpty && quantity != nil && purchase != nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(request.previousSymbol == nil ? "Position hinzufügen" : "Position bearbeiten").appFont(.headline)
            Form {
                TextField("Kürzel", text: $symbol, prompt: Text("z. B. BTC"))
                    .sensitive(.labels)
                TextField("Stückzahl", text: $quantityText, prompt: Text("z. B. 0,25"))
                    .sensitive(.amounts)
                TextField("Einstandskurs pro Stück (EUR, optional)", text: $purchaseText, prompt: Text("z. B. 30.000,00"))
                    .sensitive(.amounts)
                    .onSubmit(save)
            }
            // save() works with the values it started with: no edits while it runs
            .disabled(busy)
            if !quantityText.isEmpty, quantity == nil {
                Text("Bitte eine Stückzahl größer als 0 eingeben.").foregroundStyle(Theme.negative).appFont(.caption)
            }
            if purchase == nil {
                Text("Bitte einen Betrag von 0 oder mehr eingeben.").foregroundStyle(Theme.negative).appFont(.caption)
            }
            // the message can name the ticker, like the field above
            if let error { Text(error).foregroundStyle(Theme.negative).appFont(.caption).sensitive(.labels) }
            HStack {
                Button("Abbrechen", role: .cancel) { dismiss() }.keyboardShortcut(.cancelAction).disabled(busy)
                Spacer()
                if busy { ProgressView().controlSize(.small) }
                Button("Speichern", action: save).buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction).disabled(busy || !valid || pin == nil)
            }
        }
        .padding(20).macFrame(width: 460)
        .interactiveDismissDisabled(busy)
        .onAppear {
            symbol = request.previousSymbol ?? ""
            if request.quantity > 0 {
                quantityText = NumberText.format(request.quantity, digits: 8)
                prefilledQuantity = quantityText
            }
            if let c = request.purchaseCents { purchaseText = Money.plain(c) }
            pin = model.pinVault()
        }
    }

    private func save() {
        guard valid, let quantity, let purchase, let pin, !busy else { return }
        guard model.isCurrent(pin) else { dismiss(); return }
        let r = request, ticker = ticker
        busy = true
        error = nil
        Task {
            var priceMissing = false
            do {
                _ = try await pin.runAsync { try await $0.fetchCryptoPrice(symbol: ticker) }
                model.cryptoPricesLogged()
            } catch CoreError.InvalidInput(let message) {
                model.cryptoPricesLogged()
                busy = false
                error = message
                return
            } catch {
                model.cryptoPricesLogged()
                priceMissing = true
            }
            do {
                try await pin.run {
                    try $0.saveCryptoPosition(accountId: r.accountId, previousSymbol: r.previousSymbol, symbol: ticker,
                                              quantity: quantity, purchasePriceCents: purchase)
                }
                busy = false
                guard model.isCurrent(pin) else {
                    // saved in the vault the sheet was opened for, which is no longer shown
                    model.toast = String(localized: "Die Position wurde im zuvor geöffneten Tresor gespeichert.")
                    dismiss()
                    return
                }
                model.cryptoChanged()
                if priceMissing {
                    model.toast = String(localized: "Gespeichert, aber der Kurs für \(ticker) konnte nicht geladen werden.")
                }
                dismiss()
            } catch {
                busy = false
                self.error = describe(error)
                // the price fetched first is stored already: show it
                if !priceMissing, model.isCurrent(pin) { model.cryptoChanged() }
            }
        }
    }
}
