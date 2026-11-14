import Foundation

/// Which iCloud Drive folder holds the sync snapshots. Older versions used a
/// different folder name; its data is COPIED (never moved or deleted) into
/// "GiroCrab" once, and only when that folder is still empty.
enum ICloudFolderMigration {
    static let folderName = "GiroCrab"
    /// Name of the former folder. Internal only, never shown in the UI.
    static let legacyFolderName = "moneymoney"

    struct Resolution: Equatable {
        /// Folder to sync with.
        var url: URL
        /// "GiroCrab" is occupied by foreign files, so the old folder stays in use: show a hint.
        var conflict: Bool
    }

    /// Decides the folder inside `cloudDocs` (iCloud Drive root) and migrates if due.
    /// With `create`, a new "GiroCrab" folder is created when neither exists.
    static func resolve(cloudDocs: URL, create: Bool = true, fm: FileManager = .default) -> Resolution {
        let new = cloudDocs.appendingPathComponent(folderName, isDirectory: true)
        let old = cloudDocs.appendingPathComponent(legacyFolderName, isDirectory: true)
        guard SyncFolder.containsVault(old) else {
            if create { try? fm.createDirectory(at: new, withIntermediateDirectories: true) }
            return Resolution(url: new, conflict: false)
        }
        if SyncFolder.containsVault(new) { return Resolution(url: new, conflict: false) }
        if !isEmpty(new, fm) { return Resolution(url: old, conflict: true) }
        guard create else { return Resolution(url: old, conflict: false) }
        return copy(from: old, to: new, cloudDocs: cloudDocs, fm: fm)
            ? Resolution(url: new, conflict: false)
            : Resolution(url: old, conflict: false)
    }

    /// Finder metadata only counts as empty.
    private static func isEmpty(_ dir: URL, _ fm: FileManager) -> Bool {
        let names = (try? fm.contentsOfDirectory(atPath: dir.path)) ?? []
        return names.allSatisfy { $0.hasPrefix(".") && !$0.hasSuffix(".icloud") }
    }

    /// Copies via a staging folder so "GiroCrab" is either complete or untouched.
    /// Not-yet-downloaded iCloud placeholders are requested; the copy is retried at the next start.
    private static func copy(from old: URL, to new: URL, cloudDocs: URL, fm: FileManager) -> Bool {
        guard let names = try? fm.contentsOfDirectory(atPath: old.path) else { return false }
        let placeholders = names.filter { SyncFolder.placeholderTarget($0) != nil }
        if !placeholders.isEmpty {
            for name in placeholders { try? fm.startDownloadingUbiquitousItem(at: old.appendingPathComponent(name)) }
            return false
        }
        let staging = cloudDocs.appendingPathComponent(".GiroCrab-migration-\(UUID().uuidString)", isDirectory: true)
        do {
            try fm.createDirectory(at: staging, withIntermediateDirectories: true)
            for name in names where name != ".DS_Store" {
                try fm.copyItem(at: old.appendingPathComponent(name), to: staging.appendingPathComponent(name))
            }
            // Only an (effectively) empty target folder is replaced.
            if fm.fileExists(atPath: new.path) { try fm.removeItem(at: new) }
            try fm.moveItem(at: staging, to: new)
            return true
        } catch {
            try? fm.removeItem(at: staging)
            return false
        }
    }

    /// Folder name for the UI; the old name is never shown.
    static func displayName(for url: URL) -> String {
        url.lastPathComponent == legacyFolderName ? "iCloud Drive" : url.lastPathComponent
    }
}
