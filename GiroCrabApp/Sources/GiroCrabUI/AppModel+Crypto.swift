import SwiftUI
import GiroCrabCore

// Crypto holdings (#1836): AppModel side, see CryptoHoldingsUI.swift.

extension AppModel {
    func reloadCryptoAccounts() {
        guard let vault else { cryptoStore.accountIDs = []; return }
        do {
            cryptoStore.accountIDs = Set(try vault.listCryptoAccountIds())
        } catch {
            errorMessage = describe(error)
        }
    }

    func isCrypto(_ accountId: Int64) -> Bool {
        cryptoStore.accountIDs.contains(accountId)
    }

    /// "Neuer Crypto-Bestand …" created the empty depot `id` in the open vault: show and select it.
    /// The tab is shared by all depots: a new holding opens on its positions, where
    /// "Position hinzufügen …" is.
    func cryptoAccountCreated(_ id: Int64) {
        reloadAccounts()
        depotTab = .positions
        sidebarSelection = .account(id)
    }

    /// A position was added, changed or removed: the depot view and every total that shows it reload.
    func cryptoChanged() {
        cryptoStore.revision += 1
        reloadAccounts(); reloadHistory()
    }

    /// "Kurse aktualisieren": the EUR prices of every ticker held; the outcome shows as a toast.
    func refreshCryptoPrices() {
        guard let pin = pinVault(), !cryptoStore.refreshing else { return }
        cryptoStore.refreshing = true
        Task {
            // the flag belongs to this vault: a switch resets it (`cryptoHoldingSheets`), and a refresh
            // of the previous vault finishing later must not clear one running in the new vault
            defer { if self.isCurrent(pin) { self.cryptoStore.refreshing = false } }
            do {
                let r = try await pin.runAsync { try await $0.refreshCryptoPrices() }
                self.cryptoPricesLogged()
                guard self.isCurrent(pin) else { return }
                self.cryptoChanged()
                if !r.unknown.isEmpty {
                    self.toast = String(localized: "Kein Euro-Kurs für \(r.unknown.joined(separator: ", "))")
                } else if let e = r.error {
                    self.toast = String(localized: "Kurse konnten nicht geladen werden: \(e)")
                } else {
                    self.toast = String(localized: "Kurse aktualisiert")
                }
            } catch {
                if self.isCurrent(pin) { self.toast = describe(error) }
            }
        }
    }

    /// A price fetch wrote to the Abrufprotokoll (the core logs it): an open protocol window reloads.
    func cryptoPricesLogged() {
        syncLogVersion &+= 1
    }

    func deleteCryptoPosition(_ request: CryptoPositionRequest) {
        guard let symbol = request.previousSymbol, let vault else { return }
        do {
            try vault.deleteCryptoPosition(accountId: request.accountId, symbol: symbol)
            cryptoChanged()
        } catch {
            errorMessage = describe(error)
        }
    }
}
