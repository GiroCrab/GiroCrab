import Foundation
import GiroCrabCore

/// Einstellungen › Sicherheit › „Tresor sichern …“ / „Tresor wiederherstellen …“:
/// a copy of the own vault encrypted with a password of the user's choice
/// (core `write_backup` / `restore_backup`). It opens on any device, also
/// without this device's keychain.
extension AppModel {
    /// The backup's bytes for the file exporter; nil (with a toast) when it failed.
    func makeBackup(password: String) async -> Data? {
        guard let v = ownVault else { return nil }
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("GiroCrab-Sicherung-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: url) }
        do {
            return try await Self.offMain {
                try v.writeBackup(destPath: url.path, password: password)
                return try Data(contentsOf: url)
            }
        } catch {
            toast = describe(error)
            return nil
        }
    }

    /// Replaces the own vault's contents with the backup at `url` (a picked,
    /// possibly security-scoped file) and shows it. False (with a toast) when
    /// the password is wrong or the file is no backup; then nothing changed.
    func restoreBackup(from url: URL, password: String) async -> Bool {
        guard let v = ownVault else {
            toast = String(localized: "Bitte zuerst Ihren eigenen Tresor öffnen, dann die Sicherung wiederherstellen.")
            return false
        }
        let copy = FileManager.default.temporaryDirectory.appendingPathComponent("GiroCrab-Wiederherstellung-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: copy) }
        do {
            try await Self.offMain {
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                try FileManager.default.copyItem(at: url, to: copy)
                try v.restoreBackup(backupPath: copy.path, password: password)
            }
        } catch {
            toast = describe(error)
            return false
        }
        // also from the demo an empty vault shows on a new device
        useVault(v, demo: false)
        // The restored state wins in the merge (#1090) and goes to the other
        // devices with the next sync; say so instead of syncing silently, but
        // only once that sync has started (it waits for one still running).
        toast = String(localized: "Tresor aus der Sicherung wiederhergestellt")
        Task {
            await syncRestoredVault { [weak self] in
                self?.toast = String(localized: "Tresor aus der Sicherung wiederhergestellt. Der Stand wird mit Ihren anderen Geräten abgeglichen; das Ergebnis sehen Sie unter Einstellungen › iCloud Sync.")
            }
        }
        return true
    }
}
