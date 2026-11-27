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

    /// "Neuer Crypto-Bestand …": creates the empty depot and selects it.
    func createCryptoAccount(name: String) throws {
        guard let vault else { throw CoreError.Database(message: "vault locked") }
        let id = try vault.createCryptoAccount(name: name)
        reloadAccounts()
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
            defer { self.cryptoStore.refreshing = false }
            do {
                let r = try await pin.runAsync { try await $0.refreshCryptoPrices() }
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

    func deleteCryptoPosition(_ request: CryptoPositionRequest) {
        guard let symbol = request.previousSymbol, let vault else { return }
        runCore { try vault.deleteCryptoPosition(accountId: request.accountId, symbol: symbol) }
        cryptoChanged()
    }
}
