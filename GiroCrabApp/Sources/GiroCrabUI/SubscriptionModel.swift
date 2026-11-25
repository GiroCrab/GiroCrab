import Foundation
import GiroCrabCore

// Abo-Übersicht (GitHub #1787): pure types and calculations, no UI and no vault.

/// The rhythms a subscription is listed in. A detected series with fixed calendar days
/// (`RecurrenceInterval.custom`) gets its own group.
enum SubscriptionGroup: Int, CaseIterable, Identifiable {
    case monthly, quarterly, halfYearly, yearly, custom

    var id: Int { rawValue }

    init(_ interval: RecurrenceInterval) {
        switch interval {
        case .monthly: self = .monthly
        case .quarterly: self = .quarterly
        case .halfYearly: self = .halfYearly
        case .yearly: self = .yearly
        case .custom: self = .custom
        }
    }

    var title: String {
        switch self {
        case .monthly: String(localized: "Monatlich")
        case .quarterly: String(localized: "Vierteljährlich")
        case .halfYearly: String(localized: "Halbjährlich")
        case .yearly: String(localized: "Jährlich")
        case .custom: String(localized: "Individuell")
        }
    }

    /// "pro Monat", used behind an amount that is booked once per rhythm.
    var perPeriod: String {
        switch self {
        case .monthly: String(localized: "Monat")
        case .quarterly: String(localized: "Quartal")
        case .halfYearly: String(localized: "Halbjahr")
        case .yearly: String(localized: "Jahr")
        case .custom: String(localized: "Zahlung")
        }
    }

    /// Months between two payments (nil for fixed days).
    var months: Int? {
        switch self {
        case .monthly: 1
        case .quarterly: 3
        case .halfYearly: 6
        case .yearly: 12
        case .custom: nil
        }
    }
}

extension RecurrenceInterval {
    /// The interval a manually entered subscription can have.
    static func manual(_ group: SubscriptionGroup) -> RecurrenceInterval {
        switch group {
        case .monthly: .monthly
        case .quarterly: .quarterly
        case .halfYearly: .halfYearly
        case .yearly: .yearly
        case .custom: .yearly
        }
    }
}

enum NoticeUnit: String, Codable, CaseIterable, Identifiable {
    case days, weeks, months
    var id: String { rawValue }
    var title: String {
        switch self {
        case .days: String(localized: "Tage")
        case .weeks: String(localized: "Wochen")
        case .months: String(localized: "Monate")
        }
    }
}

/// What the user knows about the contract. Every field is optional on purpose: nobody has all of them.
struct SubscriptionDetails: Codable, Equatable {
    var note = ""
    var noticeValue: Int?
    var noticeUnit: NoticeUnit = .months
    /// ISO day (yyyy-MM-dd) the current term ends.
    var contractEnd: String?
    /// Months the contract renews by when it is not cancelled.
    var renewalMonths: Int?
    var cancelled = false
    /// ISO day the cancellation was sent; tells whether it came in time.
    var cancelledOn: String?
    /// ISO day a free trial ends; the payments start after it.
    var trialEnd: String?
    /// What identified the series when it was last seen. The series key follows the amount, so a price
    /// change gives it a new key; these three find the entry again (see `reconcileSubscriptionKeys`).
    var baseKey: String?
    var lastAmountCents: Int64?
    var accountId: Int64?

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        note = try c.decodeIfPresent(String.self, forKey: .note) ?? ""
        noticeValue = try c.decodeIfPresent(Int.self, forKey: .noticeValue)
        noticeUnit = try c.decodeIfPresent(NoticeUnit.self, forKey: .noticeUnit) ?? .months
        contractEnd = try c.decodeIfPresent(String.self, forKey: .contractEnd)
        renewalMonths = try c.decodeIfPresent(Int.self, forKey: .renewalMonths)
        cancelled = try c.decodeIfPresent(Bool.self, forKey: .cancelled) ?? false
        cancelledOn = try c.decodeIfPresent(String.self, forKey: .cancelledOn)
        trialEnd = try c.decodeIfPresent(String.self, forKey: .trialEnd)
        baseKey = try c.decodeIfPresent(String.self, forKey: .baseKey)
        lastAmountCents = try c.decodeIfPresent(Int64.self, forKey: .lastAmountCents)
        accountId = try c.decodeIfPresent(Int64.self, forKey: .accountId)
    }

    private enum Keys: String, CodingKey {
        case note, noticeValue, noticeUnit, contractEnd, renewalMonths, cancelled, cancelledOn, trialEnd, baseKey, lastAmountCents, accountId
    }
}

/// A subscription that is not (yet) a detected series: paid in cash, by card abroad, not yet
/// booked, … The user enters name, amount and rhythm; the start date anchors the next payment.
struct ManualSubscription: Codable, Equatable, Identifiable {
    var id: String
    var name: String
    /// Negative: an expense, like a booking.
    var amountCents: Int64
    var group: Int
    var currency: String
    /// ISO day of a payment (first or any); the next one is derived from it.
    var startDate: String
    /// SF Symbol chosen for the tile; nil: the standard one.
    var symbol: String?
    var details = SubscriptionDetails()

    var subscriptionGroup: SubscriptionGroup { SubscriptionGroup(rawValue: group) ?? .monthly }
}

struct SubscriptionData: Codable, Equatable {
    var entries: [String: SubscriptionDetails] = [:]
    var manual: [ManualSubscription] = []

    init(entries: [String: SubscriptionDetails] = [:], manual: [ManualSubscription] = []) {
        self.entries = entries
        self.manual = manual
    }

    /// A missing key is an empty list, not an unreadable value (a later version may add or drop one).
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: Keys.self)
        entries = try c.decodeIfPresent([String: SubscriptionDetails].self, forKey: .entries) ?? [:]
        manual = try c.decodeIfPresent([ManualSubscription].self, forKey: .manual) ?? []
    }

    private enum Keys: String, CodingKey { case entries, manual }
}

enum SubscriptionStore {
    static let settingKey = "subscriptions.v2"
    /// First version: only a note per series.
    static let legacySettingKey = "subscriptions.v1"

    /// nil when a stored value exists but cannot be read: the caller must not treat that as "no
    /// subscriptions" and write over it. No stored value at all is an empty, valid state.
    static func parse(_ json: String?, legacy: String? = nil) -> SubscriptionData? {
        if let json {
            guard let data = json.data(using: .utf8),
                  let value = try? JSONDecoder().decode(SubscriptionData.self, from: data) else { return nil }
            return value
        }
        if let legacy {
            guard let data = legacy.data(using: .utf8),
                  let notes = try? JSONDecoder().decode([String: String].self, from: data) else { return nil }
            return SubscriptionData(entries: notes.mapValues { note in
                var d = SubscriptionDetails()
                d.note = note
                return d
            })
        }
        return SubscriptionData()
    }

    /// Lenient variant: unreadable values become empty data (display only, never to be stored back).
    static func decode(_ json: String?, legacy: String? = nil) -> SubscriptionData {
        parse(json, legacy: legacy) ?? SubscriptionData()
    }

    static func encode(_ value: SubscriptionData) -> String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return String(decoding: (try? encoder.encode(value)) ?? Data("{}".utf8), as: UTF8.self)
    }
}

enum SubscriptionMath {
    static func paymentsPerYear(_ interval: RecurrenceInterval, customCount: Int) -> Int {
        switch interval {
        case .monthly: 12
        case .quarterly: 4
        case .halfYearly: 2
        case .yearly: 1
        case .custom: max(customCount, 1)
        }
    }

    static func yearlyCents(amountCents: Int64, interval: RecurrenceInterval, customCount: Int = 0) -> Int64 {
        amountCents * Int64(paymentsPerYear(interval, customCount: customCount))
    }

    /// A yearly amount for a period of `months` months, rounded to the nearest cent
    /// (a yearly 99.99 € is 8.33 € per month).
    static func cents(yearly: Int64, months: Int) -> Int64 {
        let scaled = yearly * Int64(months)
        return (scaled + (scaled < 0 ? -6 : 6)) / 12
    }

    /// Per month, rounded to the nearest cent.
    static func monthlyCents(amountCents: Int64, interval: RecurrenceInterval, customCount: Int = 0) -> Int64 {
        cents(yearly: yearlyCents(amountCents: amountCents, interval: interval, customCount: customCount), months: 1)
    }

    static func monthlyCents(_ s: RecurringSeries) -> Int64 {
        monthlyCents(amountCents: s.amountCents, interval: s.interval, customCount: s.customDates.count)
    }
}

/// Where a contract stands today, from the fields the user filled in.
struct ContractStatus: Equatable {
    /// End of the current term. While the contract renews, this is the end of the term it runs in now.
    var end: Date
    /// Last day to cancel for that end; nil without a notice period.
    var deadline: Date?
    var daysToDeadline: Int?
    var daysToEnd: Int
    /// The term is over and the user gave no renewal: the contract has run out.
    var expired: Bool
    /// The deadline for the term has passed and the contract does not renew (nothing to cancel any more).
    var deadlinePassed: Bool
    /// Cancelled and the end is reached: nothing is paid any more.
    var ended: Bool
    /// Cancelled after the notice deadline of the entered term: the deadline that was missed (the contract
    /// runs another term, `end` already is its end). nil when cancelled in time or unknown.
    var missedDeadline: Date? = nil
}

enum SubscriptionContract {
    private static var calendar: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = .autoupdatingCurrent
        return c
    }

    static func deadline(end: Date, details d: SubscriptionDetails) -> Date? {
        guard let n = d.noticeValue, n > 0 else { return nil }
        let cal = calendar
        switch d.noticeUnit {
        case .days: return cal.date(byAdding: .day, value: -n, to: end)
        case .weeks: return cal.date(byAdding: .day, value: -7 * n, to: end)
        case .months: return cal.date(byAdding: .month, value: -n, to: end)
        }
    }

    /// A cancelled contract keeps its end: it does not renew. Otherwise, with a renewal period, the
    /// term is rolled forward while it is over or while its notice deadline has passed (not cancelled in
    /// time, so it runs another term); without one, a past end means the contract has run out.
    static func status(_ d: SubscriptionDetails, today: Date = Date()) -> ContractStatus? {
        guard let endText = d.contractEnd, var end = DateText.parseISODay(endText) else { return nil }
        let cal = calendar
        let now = cal.startOfDay(for: today)
        func days(_ from: Date, _ to: Date) -> Int { cal.dateComponents([.day], from: from, to: to).day ?? 0 }
        var expired = false
        if d.cancelled {
            // Cancelled after the deadline: the term runs on, the cancellation counts for the next one.
            var missed: Date?
            if let onText = d.cancelledOn, let on = DateText.parseISODay(onText), let months = d.renewalMonths, months > 0 {
                var n = 0
                while n < 1200, let dl = deadline(end: end, details: d), dl < on {
                    missed = missed ?? dl
                    end = cal.date(byAdding: .month, value: months, to: end) ?? end.addingTimeInterval(86_400 * 30)
                    n += 1
                }
            }
            let dl = deadline(end: end, details: d)
            return ContractStatus(end: end, deadline: dl, daysToDeadline: dl.map { days(now, $0) },
                                  daysToEnd: days(now, end), expired: false, deadlinePassed: false, ended: end < now,
                                  missedDeadline: missed)
        }
        if let months = d.renewalMonths, months > 0 {
            var guardCount = 0
            while guardCount < 1200 {
                let dl = deadline(end: end, details: d)
                guard end < now || (dl.map { $0 < now } ?? false) else { break }
                end = cal.date(byAdding: .month, value: months, to: end) ?? end.addingTimeInterval(86_400 * 30)
                guardCount += 1
            }
        } else if end < now {
            expired = true
        }
        let dl = deadline(end: end, details: d)
        let daysToDeadline = dl.map { days(now, $0) }
        return ContractStatus(end: end, deadline: dl, daysToDeadline: daysToDeadline, daysToEnd: days(now, end),
                              expired: expired, deadlinePassed: (daysToDeadline ?? 0) < 0, ended: false)
    }

    /// The next payment on or after `today` for a payment `anchor` and a rhythm in months.
    static func nextPayment(anchor: String, months: Int, today: Date = Date()) -> String? {
        guard months > 0, let start = DateText.parseISODay(anchor) else { return nil }
        let cal = calendar
        let now = cal.startOfDay(for: today)
        var next = start
        var i = 0
        while next < now, i < 2400 {
            i += months
            next = cal.date(byAdding: .month, value: i, to: start) ?? now
        }
        return DateText.iso(next)
    }
}

/// What the amounts of the list are shown in. „Original“ keeps every subscription in its own rhythm.
enum SubscriptionAmountBasis: String, CaseIterable, Identifiable {
    case original, monthly, quarterly, halfYearly, yearly
    var id: String { rawValue }

    var title: String {
        switch self {
        case .original: String(localized: "Original (eigener Turnus)")
        case .monthly: String(localized: "Pro Monat")
        case .quarterly: String(localized: "Pro Quartal")
        case .halfYearly: String(localized: "Pro Halbjahr")
        case .yearly: String(localized: "Pro Jahr")
        }
    }

    /// Short form for the menu label.
    var shortTitle: String {
        switch self {
        case .original: String(localized: "Original")
        case .monthly: String(localized: "€ / Monat")
        case .quarterly: String(localized: "€ / Quartal")
        case .halfYearly: String(localized: "€ / Halbjahr")
        case .yearly: String(localized: "€ / Jahr")
        }
    }

    var months: Int? {
        switch self {
        case .original: nil
        case .monthly: 1
        case .quarterly: 3
        case .halfYearly: 6
        case .yearly: 12
        }
    }

    var period: String? {
        switch self {
        case .original: nil
        case .monthly: String(localized: "Monat")
        case .quarterly: String(localized: "Quartal")
        case .halfYearly: String(localized: "Halbjahr")
        case .yearly: String(localized: "Jahr")
        }
    }
}

/// Why a subscription gets a colour in the list.
enum SubscriptionAttention: Equatable {
    case none
    /// The notice deadline of the running term is within 30 days.
    case deadlineSoon
    /// Cancelled, and the contract still runs until its end.
    case cancelledRunning
    /// Cancelled and over.
    case ended
}

/// One line of the list: a detected series or a manual entry, with its details.
struct SubscriptionItem: Identifiable {
    let id: String
    let name: String
    /// Per payment; negative.
    let amountCents: Int64
    let group: SubscriptionGroup
    let interval: RecurrenceInterval
    let customCount: Int
    let currency: String
    let accountId: Int64?
    let categorySymbol: String
    let nextDue: String?
    let lastDate: String?
    let series: RecurringSeries?
    let manual: ManualSubscription?
    let details: SubscriptionDetails

    var isManual: Bool { manual != nil }
    var isIncome: Bool { amountCents > 0 }
    var yearlyCents: Int64 { SubscriptionMath.yearlyCents(amountCents: amountCents, interval: interval, customCount: customCount) }
    var monthlyCents: Int64 { SubscriptionMath.monthlyCents(amountCents: amountCents, interval: interval, customCount: customCount) }
    /// The fields the status is worked out from. Without a contract end (and a notice period to count
    /// back from), the next payment stands in for it: that is when the next term would start, and it
    /// renews by the rhythm unless a renewal is entered. A cancelled contract keeps what the user entered.
    var effectiveDetails: SubscriptionDetails {
        var d = details
        if d.cancelled, d.renewalMonths == nil { d.renewalMonths = group.months }
        guard d.contractEnd == nil, d.noticeValue != nil, let due = nextDue else { return d }
        d.contractEnd = due
        if d.renewalMonths == nil { d.renewalMonths = group.months }
        return d
    }
    var status: ContractStatus? { SubscriptionContract.status(effectiveDetails) }

    /// Cancelled and the end reached. A cancelled contract without a known end counts as over: there
    /// is nothing to wait for.
    var isEnded: Bool { details.cancelled && (status?.ended ?? true) }
    var isActive: Bool { !isEnded }

    var attention: SubscriptionAttention {
        if isEnded { return .ended }
        if details.cancelled { return .cancelledRunning }
        if let days = status?.daysToDeadline, (0...30).contains(days) { return .deadlineSoon }
        return .none
    }

    /// The trial end while it still lies ahead (today counts).
    var trialEndDate: Date? {
        guard let t = details.trialEnd, let d = DateText.parseISODay(t),
              d >= Calendar.current.startOfDay(for: Date()) else { return nil }
        return d
    }

    /// Whether the cancellation came in before the notice deadline. nil when unknown.
    var cancelledInTime: Bool? {
        guard details.cancelled, let s = status, details.cancelledOn != nil, s.deadline != nil else { return nil }
        return s.missedDeadline == nil
    }

    /// The amount in the chosen basis (the own rhythm for „Original“).
    func cents(in basis: SubscriptionAmountBasis) -> Int64 {
        guard let months = basis.months else { return amountCents }
        return SubscriptionMath.cents(yearly: yearlyCents, months: months)
    }

    func period(in basis: SubscriptionAmountBasis) -> String { basis.period ?? group.perPeriod }
}

enum SubscriptionSort: String, CaseIterable, Identifiable {
    case name, yearlyAmount, nextPayment, deadline
    var id: String { rawValue }
    var title: String {
        switch self {
        case .name: String(localized: "Name")
        case .yearlyAmount: String(localized: "Betrag pro Jahr")
        case .nextPayment: String(localized: "Nächste Zahlung")
        case .deadline: String(localized: "Kündigungsfrist")
        }
    }

    func sorted(_ items: [SubscriptionItem]) -> [SubscriptionItem] {
        func byName(_ a: SubscriptionItem, _ b: SubscriptionItem) -> Bool {
            a.name.localizedCaseInsensitiveCompare(b.name) == .orderedAscending
        }
        return items.sorted { a, b in
            switch self {
            case .name: return byName(a, b)
            case .yearlyAmount:
                // currencies are never compared: the amounts of one currency decide, EUR first
                if a.currency != b.currency { return a.currency == "EUR" ? true : (b.currency == "EUR" ? false : a.currency < b.currency) }
                let x = abs(a.yearlyCents), y = abs(b.yearlyCents)
                return x != y ? x > y : byName(a, b)
            case .nextPayment:
                switch (a.nextDue, b.nextDue) {
                case let (x?, y?): return x != y ? x < y : byName(a, b)
                case (_?, nil): return true
                case (nil, _?): return false
                case (nil, nil): return byName(a, b)
                }
            case .deadline:
                // soonest deadline first; open deadlines and cancelled contracts last
                func key(_ i: SubscriptionItem) -> Date? { i.details.cancelled ? nil : i.status?.deadline }
                switch (key(a), key(b)) {
                case let (x?, y?): return x != y ? x < y : byName(a, b)
                case (_?, nil): return true
                case (nil, _?): return false
                case (nil, nil): return byName(a, b)
                }
            }
        }
    }
}

enum SubscriptionStatusFilter: String, CaseIterable, Identifiable {
    case all, active, inactive, cancelled, deadlineSoon
    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: String(localized: "Alle")
        case .active: String(localized: "Aktiv")
        case .inactive: String(localized: "Inaktiv")
        case .cancelled: String(localized: "Gekündigt")
        case .deadlineSoon: String(localized: "Frist in 30 Tagen")
        }
    }

    func matches(_ i: SubscriptionItem) -> Bool {
        switch self {
        case .all: true
        case .active: i.isActive
        case .inactive: i.isEnded
        case .cancelled: i.details.cancelled
        case .deadlineSoon: i.attention == .deadlineSoon
        }
    }
}

enum SubscriptionKindFilter: String, CaseIterable, Identifiable {
    case all, expenses, income
    var id: String { rawValue }
    var title: String {
        switch self {
        case .all: String(localized: "Ausgaben und Einnahmen")
        case .expenses: String(localized: "Ausgaben")
        case .income: String(localized: "Einnahmen")
        }
    }

    func matches(_ i: SubscriptionItem) -> Bool {
        switch self {
        case .all: true
        case .expenses: !i.isIncome
        case .income: i.isIncome
        }
    }
}

/// How the list is divided below „Ausgaben“ / „Einnahmen“.
enum SubscriptionGrouping: String, CaseIterable, Identifiable {
    case rhythm, activity, none
    var id: String { rawValue }
    var title: String {
        switch self {
        case .rhythm: String(localized: "Nach Turnus")
        case .activity: String(localized: "Aktiv / Inaktiv")
        case .none: String(localized: "Nicht gruppieren")
        }
    }
}


/// A price rise: the newest payment is higher than the ones before, which were all the same.
/// A bill that varies every month (electricity) never counts.
struct SubscriptionPriceChange: Equatable {
    let old: Int64
    let new: Int64
    var delta: Int64 { new - old }

    /// `amounts`: newest first.
    static func detect(_ amounts: [Int64]) -> SubscriptionPriceChange? {
        guard amounts.count >= 3, amounts[1] == amounts[2], abs(amounts[0]) > abs(amounts[1]) else { return nil }
        return SubscriptionPriceChange(old: amounts[1], new: amounts[0])
    }
}
