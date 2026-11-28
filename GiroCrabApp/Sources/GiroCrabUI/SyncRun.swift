import Foundation

/// One login of a sync run and what became of it (window: `SyncRunSheet`, docs/ABRUF_UI.md).
struct SyncRunEntry: Identifiable, Equatable {
    enum Status: Equatable {
        case waiting
        /// Being fetched. What it waits for (PIN, TAN, approval) comes from the open prompt.
        case running
        /// `imported` is nil when the login was fetched in its own browser window.
        case done(imported: Int?, detail: String?)
        case skipped(reason: String)
        case failed(message: String)
    }

    let connectionID: Int64
    /// Name when the run started: shown if the login is deleted meanwhile.
    let name: String
    var status: Status = .waiting

    var id: Int64 { connectionID }
    /// Still to come or being fetched.
    var isOpen: Bool { status == .waiting || status == .running }
}

/// "Alle abrufen" / a group's "Konten abrufen" over several logins (#1041): every login
/// of the run with its status. Pure state; `AppModel+Sync` moves it on, the window shows it
/// until the user closes it.
struct SyncRun: Equatable {
    private(set) var entries: [SyncRunEntry]

    init(logins: [(id: Int64, name: String)]) {
        var seen = Set<Int64>()
        entries = logins.filter { seen.insert($0.id).inserted }
            .map { SyncRunEntry(connectionID: $0.id, name: $0.name) }
    }

    var total: Int { entries.count }
    /// Logins still to come, the running one included.
    var remaining: Int { entries.filter(\.isOpen).count }
    /// Logins that are through (fetched, skipped or failed).
    var completed: Int { total - remaining }
    /// Nothing waits or runs any more: the whole run is over.
    var isFinished: Bool { remaining == 0 }
    var running: SyncRunEntry? { entries.first { $0.status == .running } }
    /// Position of the running login, e.g. 2 for "Bankzugang 2 von 5".
    var position: Int { min(completed + 1, total) }

    var doneCount: Int { entries.filter { if case .done = $0.status { true } else { false } }.count }
    var skippedCount: Int { entries.filter { if case .skipped = $0.status { true } else { false } }.count }
    var failedCount: Int { entries.filter { if case .failed = $0.status { true } else { false } }.count }

    func contains(_ id: Int64) -> Bool { entries.contains { $0.connectionID == id } }
    func status(of id: Int64) -> SyncRunEntry.Status? { entries.first { $0.connectionID == id }?.status }

    /// The login's fetch begins (also again after a retry or a prompt).
    mutating func start(_ id: Int64) {
        update(id) { $0 = .running }
    }

    mutating func finish(_ id: Int64, imported: Int?, detail: String?) {
        update(id) { $0 = .done(imported: imported, detail: detail) }
    }

    mutating func skip(_ id: Int64, reason: String) {
        update(id) { if $0 == .waiting || $0 == .running { $0 = .skipped(reason: reason) } }
    }

    mutating func fail(_ id: Int64, message: String) {
        update(id) { $0 = .failed(message: message) }
    }

    /// "Erneut versuchen" of a failed or skipped login: it waits again.
    /// False when it is still open or not in this run.
    @discardableResult
    mutating func retry(_ id: Int64) -> Bool {
        guard let s = status(of: id), s != .waiting, s != .running else { return false }
        update(id) { $0 = .waiting }
        return true
    }

    /// "Alle abbrechen" or a lock for all logins: whatever is still open is skipped.
    mutating func skipRemaining(reason: String) {
        for i in entries.indices where entries[i].isOpen {
            entries[i].status = .skipped(reason: reason)
        }
    }

    private mutating func update(_ id: Int64, _ change: (inout SyncRunEntry.Status) -> Void) {
        guard let i = entries.firstIndex(where: { $0.connectionID == id }) else { return }
        change(&entries[i].status)
    }
}
