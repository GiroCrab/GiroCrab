import Foundation

/// The settings store. In the app it is `UserDefaults.standard` (so `@AppStorage`
/// reads the same values). Under `swift test --parallel` several test processes
/// run at once and shared `.standard`, overwriting each other's settings, so each
/// test process keeps its settings in memory instead (like the temp vaults,
/// `AppModel.tempVaultURL`). Not a file-backed suite: on the CI runners a suite at
/// a temp path lost writes or fell back to an older state now and then (a test
/// read `[]` or an earlier test's value right after writing its own).
public enum AppDefaults {
    public nonisolated static let store: UserDefaults = makeStore()

    /// `.standard` in the app, an in-memory store of its own in unit tests.
    nonisolated static func makeStore(unitTests: Bool = AppModel.runningUnitTests) -> UserDefaults {
        unitTests ? InMemoryDefaults() : .standard
    }
}

/// Settings that live only in this process: every read sees the last write, and
/// nothing reaches cfprefsd or the disk. Every getter (`bool`, `string`, `array` …)
/// goes through `object(forKey:)` and every setter through `set(_:forKey:)`. The
/// change notifications keep key-value observation working (`showRunningBalance`).
final class InMemoryDefaults: UserDefaults, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: Any] = [:]

    init() {
        // Never read or written: every access is overridden below.
        super.init(suiteName: nil)!
    }

    override func object(forKey key: String) -> Any? {
        lock.withLock { values[key] }
    }

    override func set(_ value: Any?, forKey key: String) {
        willChangeValue(forKey: key)
        lock.withLock { values[key] = value }
        didChangeValue(forKey: key)
    }

    override func removeObject(forKey key: String) {
        set(nil, forKey: key)
    }

    override func dictionaryRepresentation() -> [String: Any] {
        lock.withLock { values }
    }
}
