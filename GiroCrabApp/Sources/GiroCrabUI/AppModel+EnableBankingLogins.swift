import Foundation
import GiroCrabCore

// MARK: Enable Banking application per bank login via iCloud Keychain (#962)

extension AppModel {
    /// What to do with the logins' assignments.
    enum EnableBankingLoginShare {
        /// The vault opened: take over what other devices changed, share what changed here.
        case open
        /// Something was saved here: only share, never take over (the store still holds the old state).
        case shareOnly
        /// The user saved this login's application: share it even over a change from another device.
        case force(Int64)
    }

    /// Same condition as the default application (`sharesEnableBankingKey`).
    private var sharesEnableBankingLogins: Bool {
        VaultKeyManager.shared.syncViaICloudKeychain && !demoActive && vault != nil
    }

    /// Reconciles the logins' applications with the store. Runs after the
    /// default application was reconciled: taking over a new default pins the
    /// logins to the old one first (`pin_default_app`), and that is what gets shared.
    ///
    /// One sync at a time, from loading the store to marking what was shared:
    /// an older snapshot must never be applied over a newer sync's result.
    func syncEnableBankingLogins(_ mode: EnableBankingLoginShare) {
        guard sharesEnableBankingLogins, let vault else { return }
        let store = enableBankingLoginStore
        let previous = enableBankingLoginTask
        // the keychain query can block (iCloud Keychain): never on the main thread
        enableBankingLoginTask = Task { [weak self] in
            await previous?.value
            let loaded = await Task.detached(priority: .utility) { () -> ([String: String], [String: EnableBankingApp])? in
                // a failed read is no empty store: uploading over it could replace another device's choice
                guard let logins = store.loadLogins() else { return nil }
                var apps: [String: EnableBankingApp] = [:]
                for id in Set(logins.values) where !id.isEmpty {
                    if let app = store.loadApp(id: id), app.isComplete { apps[id] = app }
                }
                return (logins, apps)
            }.value
            guard let (stored, apps) = loaded, !Task.isCancelled,
                  let self, self.vault === vault, self.sharesEnableBankingLogins else { return }
            await self.applyEnableBankingLogins(mode, stored: stored, apps: apps, vault: vault)
        }
    }

    private func applyEnableBankingLogins(_ mode: EnableBankingLoginShare, stored: [String: String],
                                          apps: [String: EnableBankingApp], vault: Vault) async {
        guard let logins = try? vault.listEnableBankingLogins() else { return }
        // marked as shared only once the store has them (see `markUploadedEnableBankingLogins`)
        var uploads: [(id: Int64, syncKey: String, app: String, key: EnableBankingApp?)] = []
        var assigned: [(bank: String, applicationID: String)] = []
        var conflicts: [String] = []
        for login in logins {
            let forced: Bool
            if case .force(let id) = mode { forced = id == login.connectionId } else { forced = false }
            var adopting = false
            if case .open = mode { adopting = true }
            let raw = EnableBankingLoginSync.reconcile(here: login.applicationId, synced: login.syncedApplicationId,
                                                       store: stored[login.syncKey])
            // remembered so that saving the kept application again resolves it; also when
            // forced: it stays until its upload succeeded (`markUploadedEnableBankingLogins`),
            // so a failed upload can be retried by saving again
            if raw == .conflict {
                enableBankingLoginConflicts.insert(login.connectionId)
            } else {
                enableBankingLoginConflicts.remove(login.connectionId)
            }
            let action = EnableBankingLoginSync.filter(raw, here: login.applicationId, forced: forced, adopting: adopting)
            let bank = connections.first { $0.id == login.connectionId }?.displayName ?? ""
            switch action {
            case .none:
                continue
            case .conflict:
                conflicts.append(bank)
            case .markSynced(let app):
                markEnableBankingLogin(login.connectionId, app, vault: vault)
            case .upload(let app):
                var key: EnableBankingApp?
                if !app.isEmpty {
                    let pem = ((try? vault.connectionEnableBankingKey(connectionId: login.connectionId)) ?? nil) ?? ""
                    let full = EnableBankingApp(applicationID: app, pem: pem)
                    guard full.isComplete else { continue }
                    key = full
                }
                uploads.append((login.connectionId, login.syncKey, app, key))
            case .adopt(let app):
                guard adoptEnableBankingLogin(login.connectionId, app, apps: apps, vault: vault) else { continue }
                markEnableBankingLogin(login.connectionId, app, vault: vault)
                assigned.append((bank, app))
            }
        }
        if !assigned.isEmpty { reloadSettings() }
        if !AppDefaults.store.bool(forKey: EnableBankingKeySync.noticeSuppressedKey),
           let notice = EnableBankingLoginSync.notice(assigned: assigned, conflicts: conflicts) {
            enableBankingLoginNotice = notice
        }
        guard !uploads.isEmpty else { return }
        let store = enableBankingLoginStore
        let gate = enableBankingUploadGate
        let pending = uploads
        let saved = await Task.detached(priority: .utility) { () -> [(id: Int64, app: String)] in
            var saved: [(id: Int64, app: String)] = []
            for upload in pending {
                do {
                    // locked or reset meanwhile: write nothing more
                    let written = try gate.run {
                        // the key first: a device that sees the assignment must find its key
                        if let key = upload.key { try store.saveApp(key) }
                        try store.saveLogin(syncKey: upload.syncKey, applicationID: upload.app)
                    }
                    guard written else { break }
                    saved.append((upload.id, upload.app))
                } catch {
                    // no iCloud Keychain here: the login keeps its application anyway. Not
                    // marked as shared, so the next sync uploads it again instead of taking
                    // over the store's older state.
                    enableBankingKeyLog.info("login upload failed: \((error as NSError).code, privacy: .public)")
                }
            }
            return saved
        }.value
        markUploadedEnableBankingLogins(saved, vault: vault)
    }

    /// The login's application was changed here and on another device: saving
    /// it again (even unchanged) keeps this one everywhere.
    func enableBankingLoginHasConflict(_ id: Int64) -> Bool {
        enableBankingLoginConflicts.contains(id)
    }

    /// The settings of a login were saved without changing its application: if it
    /// is in conflict with another device, the kept application now wins there too.
    func keepEnableBankingLoginApp(_ id: Int64) {
        guard enableBankingLoginConflicts.contains(id) else { return }
        syncEnableBankingLogins(.force(id))
    }

    /// Gives the login the application another device chose; false when its
    /// key has not arrived yet (iCloud lags) or does not sign.
    private func adoptEnableBankingLogin(_ id: Int64, _ app: String, apps: [String: EnableBankingApp], vault: Vault) -> Bool {
        var pem = ""
        if !app.isEmpty {
            guard let stored = apps[app] else { return false }
            pem = stored.pem
        }
        do {
            _ = try vault.setConnectionEnableBankingApp(connectionId: id, applicationId: app, privateKeyPem: pem)
            return true
        } catch {
            Self.debugLog("enablebanking login adopt failed: \(error)")
            return false
        }
    }

    /// After the store took the uploads: remembers them as shared, but only for
    /// logins that still use the uploaded application (changed meanwhile = the
    /// next sync shares the newer one).
    private func markUploadedEnableBankingLogins(_ marks: [(id: Int64, app: String)], vault: Vault) {
        guard !marks.isEmpty, self.vault === vault, let current = try? vault.listEnableBankingLogins() else { return }
        for mark in marks where current.first(where: { $0.connectionId == mark.id })?.applicationId == mark.app {
            markEnableBankingLogin(mark.id, mark.app, vault: vault)
            enableBankingLoginConflicts.remove(mark.id)
        }
    }

    private func markEnableBankingLogin(_ id: Int64, _ app: String, vault: Vault) {
        do {
            try vault.markEnableBankingLoginSynced(connectionId: id, applicationId: app)
        } catch {
            Self.debugLog("enablebanking login mark failed: \(error)")
        }
    }
}
