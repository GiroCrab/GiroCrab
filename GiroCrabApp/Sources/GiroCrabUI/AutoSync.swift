import Foundation

/// Interval logic for the scheduled fetch of each bank connection.
enum AutoSync {
    /// The former global interval, inherited by connections without their own schedule.
    static let hoursKey = "autoSync.hours"
    /// Last automatic attempt per connection (`autoSync.attempt.<id>`, Unix seconds).
    static let attemptKeyPrefix = "autoSync.attempt."
    /// Last scheduled fetch that failed because the bank demanded a fresh
    /// approval (`autoSync.sca_block.<id>`, Unix seconds, #1997).
    static let scaBlockKeyPrefix = "autoSync.sca_block."
    /// Offered in the picker; 0 = manual only.
    static let hourOptions = [0, 1, 2, 4, 6, 12, 24]

    /// Schedule stored in the vault under the connection's local ID.
    /// Positive values are hours, zero is manual only, and -1 excludes the connection.
    static let never = -1
    static let scheduleOptions = hourOptions + [never]

    static func scheduleKey(_ connectionID: Int64) -> String { "sync.schedule.\(connectionID)" }

    enum Decision: Equatable {
        /// Interval off.
        case off
        /// Due or not, vault locked: nothing runs, the next tick after the
        /// unlock catches up (nothing is queued, the last fetch time is unchanged).
        case skippedLocked
        case notDue
        case run
    }

    /// Stored hours: a missing, negative or unparsable value means off.
    static func hours(stored: Int?) -> Int { max(0, stored ?? 0) }

    /// A bank that demands a fresh approval (TAN / app confirmation) for every
    /// fetch can never be fetched quietly (#1997): the scheduled sync stops
    /// before any booking is read, and `sca_seen` keeps the connector skipping
    /// those jobs until a manual sync approved them. Retrying at every interval
    /// would only log into the bank over and over, so the next attempt waits a
    /// day.
    static let scaBackOffHours = 24

    static func scaBackOffKey(_ connectionID: Int64) -> String { scaBlockKeyPrefix + String(connectionID) }

    /// Still backing off after the scheduled fetch of `connectionID` was
    /// blocked by a demanded approval (within `scaBackOffHours`).
    static func scaBackOffActive(connectionID: Int64, defaults: UserDefaults, now: Date) -> Bool {
        let blocked = defaults.double(forKey: scaBackOffKey(connectionID))
        guard blocked > 0 else { return false }
        return now.timeIntervalSince1970 - blocked < TimeInterval(scaBackOffHours) * 3600
    }

    /// Remembers a scheduled fetch blocked by a demanded approval; the next
    /// automatic attempt for this connection waits `scaBackOffHours`.
    static func recordScaBlock(connectionID: Int64, defaults: UserDefaults, now: Date) {
        defaults.set(now.timeIntervalSince1970, forKey: scaBackOffKey(connectionID))
    }

    /// A scheduled fetch succeeded: the bank no longer blocks quiet fetches.
    static func clearScaBlock(connectionID: Int64, defaults: UserDefaults) {
        defaults.removeObject(forKey: scaBackOffKey(connectionID))
    }

    /// Due when no fetch or attempt is known, or the newer of the two is at
    /// least `hours` ago. The attempt counts too: a login that needs a TAN
    /// would otherwise be retried at every tick.
    static func isDue(lastSyncUnix: Int64?, lastAttemptUnix: Int64?, hours: Int, now: Date) -> Bool {
        guard hours > 0 else { return false }
        guard let last = [lastSyncUnix, lastAttemptUnix].compactMap({ $0 }).max() else { return true }
        return now.timeIntervalSince1970 - TimeInterval(last) >= TimeInterval(hours) * 3600
    }

    static func decide(hours: Int, isUnlocked: Bool, lastSyncUnix: Int64?, lastAttemptUnix: Int64?, now: Date) -> Decision {
        guard hours > 0 else { return .off }
        guard isUnlocked else { return .skippedLocked }
        return isDue(lastSyncUnix: lastSyncUnix, lastAttemptUnix: lastAttemptUnix, hours: hours, now: now) ? .run : .notDue
    }

    static func attemptKey(_ connectionID: Int64) -> String { attemptKeyPrefix + String(connectionID) }

    /// Picker / menu wording of an interval.
    static func label(_ hours: Int) -> String {
        switch hours {
        case never: return String(localized: "Nie")
        case 0: return String(localized: "Nur manuell")
        case 1: return String(localized: "Jede Stunde")
        default: return String(localized: "Alle \(hours) Stunden")
        }
    }
}

/// What the menu bar popover may say about the automatic fetch. The failure
/// detail can name a bank or an account, so the popover shows it only while
/// nothing is hidden (`MenuBarSummary.Visibility.visible`).
struct AutoSyncStatus: Equatable {
    var lastRun: Date?
    var failureDetail: String?
    var failed: Bool { failureDetail != nil }

    /// Notice lines for the popover; empty when there is nothing to say. Locked:
    /// only that the fetch is paused. Private: only that and when, no detail.
    func notice(_ visibility: MenuBarSummary.Visibility, intervalOn: Bool) -> [String] {
        switch visibility {
        case .locked:
            return intervalOn ? [String(localized: "Automatischer Abruf pausiert, bis der Tresor entsperrt ist.")] : []
        case .covered:
            return failed ? [String(localized: "Der letzte automatische Abruf ist fehlgeschlagen.")] : []
        case .visible:
            guard let detail = failureDetail else { return [] }
            return [String(localized: "Der letzte automatische Abruf ist fehlgeschlagen: \(detail)")]
        }
    }
}
