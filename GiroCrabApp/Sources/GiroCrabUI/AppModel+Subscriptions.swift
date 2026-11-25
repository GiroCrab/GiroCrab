import Foundation
import GiroCrabCore

/// Abo-Übersicht (GitHub #1787). The mark and the contract fields belong to the series
/// (`seriesKey`), not to the payee: two contracts with one payee (two streaming plans, several
/// in-app subscriptions) are separate series as long as their amounts differ. Manual entries are
/// stored next to them. All of it is one JSON setting in the vault: no schema step.
extension AppModel {
    /// Nothing of the previous vault stays (demo ↔ own vault, lock): data, bookings, selection, waiting reloads.
    func resetSubscriptionState() {
        subscriptionReloadTask?.cancel()
        subscriptionReloadTask = nil
        pendingSubscriptionReload = nil
        reloadsAllSubscriptionAccounts = false
        subscriptionDataLoaded = false
        subscriptionData = SubscriptionData()
        subscriptionBookings = [:]
        selectedSubscriptionID = nil
    }

    /// A failed read keeps what is there and blocks writing: an unreadable setting must never be
    /// replaced by an empty snapshot that the next edit would then store over the real contracts.
    func loadSubscriptionData() {
        guard let vault else { return }
        do {
            let current = try vault.getSetting(key: SubscriptionStore.settingKey)
            let legacy = current == nil ? try vault.getSetting(key: SubscriptionStore.legacySettingKey) : nil
            guard let loaded = SubscriptionStore.parse(current, legacy: legacy) else {
                // stored but unreadable: keep what is shown and block writing, else the next edit would
                // replace the real contracts by an empty snapshot
                subscriptionDataLoaded = false
                errorMessage = String(localized: "Die gespeicherten Abo-Daten sind nicht lesbar. Änderungen werden nicht gespeichert.")
                return
            }
            if loaded != subscriptionData { subscriptionData = loaded }
            subscriptionDataLoaded = true
        } catch {
            // an unreadable setting blocks writing, even when an earlier read had worked:
            // the snapshot in memory may be older than what is stored
            subscriptionDataLoaded = false
            errorMessage = describe(error)
        }
    }

    /// Mutations copy `subscriptionData` first, so it must be loaded before that copy is taken.
    private func ensureSubscriptionDataLoaded() -> Bool {
        if !subscriptionDataLoaded { loadSubscriptionData() }
        return subscriptionDataLoaded
    }

    /// Publishes the new value only after it is stored; false when nothing was written.
    @discardableResult
    private func storeSubscriptionData(_ value: SubscriptionData) -> Bool {
        guard let vault else { return false }
        guard ensureSubscriptionDataLoaded() else { return false }
        do {
            try vault.setSetting(key: SubscriptionStore.settingKey, value: SubscriptionStore.encode(value))
        } catch {
            errorMessage = describe(error)
            return false
        }
        subscriptionData = value
        return true
    }

    private static func remember(_ s: RecurringSeries, in d: inout SubscriptionDetails) {
        d.baseKey = s.baseKey
        d.lastAmountCents = s.amountCents
        d.accountId = s.accountId
    }

    /// The base key an entry belongs to: stored, else the series key without its amount suffix (`…#1299`).
    private static func baseKey(of key: String, _ d: SubscriptionDetails) -> String {
        if let b = d.baseKey { return b }
        guard let r = key.range(of: #"#-?\d+$"#, options: .regularExpression) else { return key }
        return String(key[..<r.lowerBound])
    }

    /// The series key follows the typical amount, so a price rise or a second contract of the same payee
    /// gives a marked series a new key: the entry would drop out of the list with its contract fields.
    /// Entries whose key is no longer there move to the series with the same base key, when that is
    /// unambiguous (one unmarked candidate, or the one clearly nearest to the last known amount).
    /// Also keeps the identity fields of the live entries current. Only touches what is visible now:
    /// `recurring` covers the selected accounts, so an entry of another account is left alone.
    func reconcileSubscriptionKeys() {
        guard subscriptionDataLoaded, !recurring.isEmpty else { return }
        var data = subscriptionData
        let live = Set(recurring.map(\.seriesKey))
        for (key, d) in subscriptionData.entries where !live.contains(key) {
            let base = Self.baseKey(of: key, d)
            let free = recurring.filter {
                $0.baseKey == base && data.entries[$0.seriesKey] == nil
                    && (d.accountId == nil || d.accountId == $0.accountId)
            }
            var target: RecurringSeries?
            if free.count == 1 { target = free.first } else if free.count > 1, let last = d.lastAmountCents {
                let near = free.filter { abs($0.amountCents - last) * 100 <= abs(last) * 15 }
                if near.count == 1 { target = near.first }
            }
            guard let target else { continue }
            var moved = d
            Self.remember(target, in: &moved)
            data.entries[key] = nil
            data.entries[target.seriesKey] = moved
            if selectedSubscriptionID == key { selectedSubscriptionID = target.seriesKey }
        }
        for s in recurring {
            guard var d = data.entries[s.seriesKey] else { continue }
            let before = d
            Self.remember(s, in: &d)
            if d != before { data.entries[s.seriesKey] = d }
        }
        guard data != subscriptionData else { return }
        let accounts = Set(recurring.compactMap { data.entries[$0.seriesKey] != nil ? $0.accountId : nil })
        if storeSubscriptionData(data) { scheduleSubscriptionBookingsReload(accounts: accounts) }
    }

    func isSubscription(_ s: RecurringSeries) -> Bool { subscriptionData.entries[s.seriesKey] != nil }

    /// Recurring expenses and incomes that can still be marked.
    var subscriptionCandidates: [RecurringSeries] {
        recurring.filter { !isSubscription($0) }
    }

    func setSubscription(_ s: RecurringSeries, marked: Bool) {
        guard ensureSubscriptionDataLoaded() else { return }
        var data = subscriptionData
        if marked {
            var d = data.entries[s.seriesKey] ?? SubscriptionDetails()
            Self.remember(s, in: &d)
            data.entries[s.seriesKey] = d
        } else { data.entries[s.seriesKey] = nil }
        storeSubscriptionData(data)
        if marked { scheduleSubscriptionBookingsReload(accounts: [s.accountId]) }
    }

    func setSubscriptionDetails(_ item: SubscriptionItem, _ details: SubscriptionDetails) {
        guard ensureSubscriptionDataLoaded() else { return }
        var data = subscriptionData
        if let s = item.series {
            guard data.entries[s.seriesKey] != nil, data.entries[s.seriesKey] != details else { return }
            data.entries[s.seriesKey] = details
        } else if let m = item.manual, let i = data.manual.firstIndex(where: { $0.id == m.id }) {
            guard data.manual[i].details != details else { return }
            data.manual[i].details = details
        } else { return }
        storeSubscriptionData(data)
    }

    /// `amountCents` carries the sign: negative is an expense, positive an income.
    func addManualSubscription(name: String, amountCents: Int64, group: SubscriptionGroup, currency: String, startDate: String,
                               symbol: String? = nil, details: SubscriptionDetails = SubscriptionDetails()) {
        guard ensureSubscriptionDataLoaded() else { return }
        var data = subscriptionData
        data.manual.append(ManualSubscription(
            id: UUID().uuidString, name: name, amountCents: amountCents, group: group.rawValue,
            currency: currency, startDate: startDate, symbol: symbol, details: details))
        storeSubscriptionData(data)
    }

    func updateManualSubscription(_ m: ManualSubscription) {
        guard ensureSubscriptionDataLoaded() else { return }
        var data = subscriptionData
        guard let i = data.manual.firstIndex(where: { $0.id == m.id }), data.manual[i] != m else { return }
        data.manual[i] = m
        storeSubscriptionData(data)
    }

    func removeManualSubscription(id: String) {
        guard ensureSubscriptionDataLoaded() else { return }
        var data = subscriptionData
        data.manual.removeAll { $0.id == id }
        storeSubscriptionData(data)
    }

    /// A new rhythm can give the series another key. The subscription mark and its fields follow it,
    /// else the subscription would silently vanish and could only be added again without its fields.
    func carrySubscription(from old: RecurringSeries) {
        // The series is the same payee/contract (same base key); a manual rhythm also drops the amount from
        // the key, so the latest booking and the key suffix both change. Among several contracts of one
        // payee the one with the nearest amount is the same contract.
        guard ensureSubscriptionDataLoaded(),
              let details = subscriptionData.entries[old.seriesKey],
              let now = recurring.filter({ $0.baseKey == old.baseKey })
                .min(by: { abs($0.amountCents - old.amountCents) < abs($1.amountCents - old.amountCents) }),
              now.seriesKey != old.seriesKey else { return }
        var data = subscriptionData
        // Two contracts of one payee can merge into one series when a rhythm changes: if the target is
        // already marked, the old entry stays where it is (not shown, but its fields are not lost).
        if data.entries[now.seriesKey] == nil {
            data.entries[old.seriesKey] = nil
            data.entries[now.seriesKey] = details
        }
        storeSubscriptionData(data)
        if selectedSubscriptionID == old.seriesKey { selectedSubscriptionID = now.seriesKey }
        scheduleSubscriptionBookingsReload(accounts: [now.accountId])
    }

    func changeSubscriptionRhythm(_ s: RecurringSeries, to interval: RecurrenceInterval) {
        setRecurringInterval(s, interval)
    }

    /// Demo vault only: fills the view with a few subscriptions the first time it opens, so there is
    /// something to look at (a real vault never gets entries it did not ask for).
    func seedDemoSubscriptions() {
        guard options.demoSeed, demoActive, subscriptionData == SubscriptionData() else { return }
        var data = SubscriptionData()
        let expenses = recurring.filter { $0.amountCents < 0 }
        let incomes = recurring.filter { $0.amountCents > 0 }
        for s in incomes.prefix(1) { data.entries[s.seriesKey] = SubscriptionDetails() }
        for (i, s) in expenses.prefix(6).enumerated() {
            var d = SubscriptionDetails()
            if i == 0 { d.noticeValue = 1; d.noticeUnit = .months; d.contractEnd = DateText.iso(Date().addingTimeInterval(86_400 * 50)); d.renewalMonths = 12 }
            if i == 1 { d.cancelled = true; d.note = "Zum Jahresende gekündigt" }
            if i == 2 { d.note = "Familientarif" }
            data.entries[s.seriesKey] = d
        }
        var gym = ManualSubscription(id: UUID().uuidString, name: "Fitnessstudio", amountCents: -8_970,
                                     group: SubscriptionGroup.quarterly.rawValue, currency: "EUR", startDate: "2026-01-10")
        gym.details.noticeValue = 3; gym.details.noticeUnit = .weeks
        gym.details.contractEnd = DateText.iso(Date().addingTimeInterval(86_400 * 140)); gym.details.renewalMonths = 6
        data.manual = [gym, ManualSubscription(id: UUID().uuidString, name: "Haftpflichtversicherung", amountCents: -7_480,
                                               group: SubscriptionGroup.yearly.rawValue, currency: "EUR", startDate: "2026-03-01")]
        storeSubscriptionData(data)
        selectedSubscriptionID = subscriptionItems.first?.id
    }

    private func currency(ofAccount id: Int64) -> String {
        allAccounts.first { $0.id == id }?.currency ?? "EUR"
    }

    /// Everything the list shows: marked series that are still detected, plus the manual entries.
    /// A marked series that is no longer recurring drops out but keeps its fields for when it is back.
    var subscriptionItems: [SubscriptionItem] {
        var items: [SubscriptionItem] = recurring.compactMap { s in
            guard let details = subscriptionData.entries[s.seriesKey] else { return nil }
            return SubscriptionItem(
                id: s.seriesKey, name: s.name, amountCents: s.amountCents, group: SubscriptionGroup(s.interval),
                interval: s.interval, customCount: s.customDates.count, currency: currency(ofAccount: s.accountId),
                accountId: s.accountId, categorySymbol: s.categorySymbol, nextDue: s.nextDue, lastDate: s.lastDate,
                series: s, manual: nil, details: details)
        }
        items += subscriptionData.manual.map { m in
            let group = m.subscriptionGroup
            return SubscriptionItem(
                id: "manual:" + m.id, name: m.name, amountCents: m.amountCents, group: group,
                interval: RecurrenceInterval.manual(group), customCount: 0, currency: m.currency, accountId: nil,
                categorySymbol: m.symbol ?? "", nextDue: group.months.flatMap { SubscriptionContract.nextPayment(anchor: m.startDate, months: $0) },
                lastDate: nil, series: nil, manual: m, details: m.details)
        }
        return items
    }

    /// Whether `b` belongs to the series (same rule as `recurringSeries(for:)`).
    private nonisolated static func belongs(_ b: Booking, to s: RecurringSeries) -> Bool {
        b.recurrenceKey == s.baseKey
            && (!s.amountSpecific || abs(b.amountCents - s.amountCents) * 100 <= abs(s.amountCents) * 15)
    }

    /// The bookings of one series. A series whose amount varies (a phone bill) keeps its range, but a
    /// one-off with the same payee (a 2,059 € purchase next to a 0.99 € subscription) is not part of it:
    /// amounts further than a factor of 4 from the series' typical amount drop out.
    nonisolated static func seriesBookings(_ all: [Booking], of s: RecurringSeries) -> [Booking] {
        let own = all.filter { belongs($0, to: s) }
        let typical = max(abs(s.amountCents), 1)
        return own.filter { abs($0.amountCents) * 4 >= typical && abs($0.amountCents) <= typical * 4 }
    }

    /// Loads the bookings of the marked series, one query per account, off the main thread. The core
    /// enriches every row of such a query, so this runs when the view appears or the marks change, not per row.
    /// Runs one reload after the other: clicking "Hinzufügen" several times in a row must not start a
    /// query per click (each holds the vault for its whole run). Only the running one and the last one execute.
    func scheduleSubscriptionBookingsReload(accounts: Set<Int64>? = nil) {
        // requests that wait collect their accounts: a cancelled one must not take its account with it
        if let accounts { pendingSubscriptionReload = (pendingSubscriptionReload ?? []).union(accounts) }
        else { reloadsAllSubscriptionAccounts = true }
        let previous = subscriptionReloadTask
        previous?.cancel()
        subscriptionReloadTask = Task { [weak self] in
            _ = await previous?.value
            guard !Task.isCancelled, let self else { return }
            let scope = reloadsAllSubscriptionAccounts ? nil : pendingSubscriptionReload
            pendingSubscriptionReload = nil
            reloadsAllSubscriptionAccounts = false
            await reloadSubscriptionBookings(accounts: scope)
        }
    }

    /// `accounts`: only these accounts are queried again, the others keep what they have.
    func reloadSubscriptionBookings(accounts only: Set<Int64>? = nil) async {
        let allMarked = recurring.filter { subscriptionData.entries[$0.seriesKey] != nil }
        guard !allMarked.isEmpty else { subscriptionBookings = [:]; return }
        let marked = only.map { keep in allMarked.filter { keep.contains($0.accountId) } } ?? allMarked
        guard !marked.isEmpty else { return }
        let from = Calendar.current.date(byAdding: .year, value: -4, to: Date()).map(DateText.iso)
        let byAccount = Dictionary(grouping: marked, by: \.accountId)
        do {
            let result = try await vaultOffMain { vault -> [String: [Booking]] in
                var out: [String: [Booking]] = [:]
                for (account, series) in byAccount {
                    let filter = BookingFilter(
                        accountIds: [account], categoryId: nil, includeSubcategories: true, uncategorizedOnly: false,
                        search: nil, dateFrom: from, dateTo: nil, limit: 5000, offset: 0, scope: .all,
                        merchantKey: nil, direction: .all, checked: nil, wholeDays: false,
                        amountMinCents: nil, amountMaxCents: nil)
                    let bookings = try vault.listTransactions(filter: filter).transactions
                    for s in series { out[s.seriesKey] = Self.seriesBookings(bookings, of: s) }
                }
                return out
            }
            if let result {
                let live = Set(allMarked.map(\.seriesKey))
                subscriptionBookings = only == nil ? result
                    : subscriptionBookings.filter { live.contains($0.key) }.merging(result) { _, new in new }
            }
        } catch {
            errorMessage = describe(error)
        }
    }

    /// Totals per currency of the given items (#569: never one sum across currencies); EUR first.
    nonisolated static func subscriptionTotals(_ items: [SubscriptionItem], cents: (SubscriptionItem) -> Int64) -> [(currency: String, cents: Int64)] {
        var sums: [String: Int64] = [:]
        for i in items { sums[i.currency, default: 0] += cents(i) }
        return sums.map { (currency: $0.key, cents: $0.value) }
            .sorted { ($0.currency == "EUR" ? "" : $0.currency) < ($1.currency == "EUR" ? "" : $1.currency) }
    }
}
