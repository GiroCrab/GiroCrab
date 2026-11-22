import Foundation

/// Bitcoin Core's data directory as Bitcoin-Qt remembers it (`strDataDir` in its
/// preferences): a node kept on another drive is found without typing the path.
enum BitcoinCoreDatadir {
    static let key = "bitcoin_core.datadir"

    /// The directory Bitcoin-Qt uses, nil when it has none stored (or isn't installed).
    static func detected(_ defaults: UserDefaults? = UserDefaults(suiteName: "org.bitcoin.Bitcoin-Qt")) -> String? {
        guard let dir = defaults?.string(forKey: "strDataDir")?.trimmingCharacters(in: .whitespaces),
              !dir.isEmpty else { return nil }
        return dir
    }

    /// The node's default data directory on the Mac; none on iPhone and iPad.
    static var standard: String {
        #if os(macOS)
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/Bitcoin").path
        #else
        ""
        #endif
    }

    /// Whether this Mac has a node: one of its folders (set, Bitcoin-Qt's, default) exists.
    /// A vault that came over iCloud to a Mac without one is not fetched there.
    static func nodeOnThisMac(configured: String?, detected: String? = detected(), standard: String = standard) -> Bool {
        [configured, detected, standard].contains { dir in
            var isDir: ObjCBool = false
            guard let dir, !dir.isEmpty else { return false }
            return FileManager.default.fileExists(atPath: dir, isDirectory: &isDir) && isDir.boolValue
        }
    }
}
