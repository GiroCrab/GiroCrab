import Foundation
import SwiftUI
import GiroCrabCore

extension AppModel {
    // MARK: - Loading

    /// A totals reload supersedes category snapshots and refreshes the cashflow
    /// card even when the proof list keeps exactly the same bookings.
    func invalidateTotals() {
        categoryRefreshTask?.cancel()
        categoryRefreshTask = nil
        totalsVersion += 1
    }

    func reloadAll() {
        invalidateTotals()
        assetClassificationVersion += 1
        merchantSettingsRevision += 1
        let all = Perf.signposter.beginInterval("reloadAll")
        defer { Perf.signposter.endInterval("reloadAll", all) }
        reloadAccounts()
        refreshSidebarCounts()
        refreshSalaryCycle()
        refreshRecurring()
        mirrorDocumentsIfEnabled()
        reloadHistory()
        reloadCategories()
        reloadBookings()
        reloadRules()
        reloadTags()
        reloadSettings()
        reloadDashboard()
    }

    /// `reloadAll` for changes nobody asked for in the UI (iCloud merge, scheduled sync): the
    /// small lookups reload at once, everything that scans the bookings loads in the
    /// background and replaces the shown data in one go.
    func reloadAllInBackground() {
        guard isUnlocked else { return }
        // a sync can change the effective total set: supersede edit snapshots
        // that still carry the pre-sync totals (#337), like `reloadAll` does
        invalidateTotals()
        assetClassificationVersion += 1
        merchantSettingsRevision += 1
        reloadAccounts()
        refreshSidebarCounts()
        mirrorDocumentsIfEnabled()
        reloadRules()
        reloadTags()
        reloadSettings()
        reloadDashboard()
        merchantStatsCache = [:]
        loadMerchantStats(keepShown: true)
        startSelectionLoad(keepingShown: true)
        // merchant settings from another device may leave the merchant filter empty (#1373)
        if merchantFocus != nil { Task { [weak self] in await self?.leaveStaleMerchantFocus() } }
    }

    func reloadAccounts() {
        guard let vault else { return }
        runCore {
            applyAccountGroups(try vault.listAccountGroups(), try vault.listUserGroups())
            connections = try vault.listBankConnections()
            connectionKeys = Dictionary(uniqueKeysWithValues: connections.compactMap { c in
                (try? vault.connectionKey(connectionId: c.id)).map { (c.id, $0) }
            })
        }
    }

    /// Badge counts of „Geplant“, „Lastschriften“ and the unread documents: `COUNT(*)`s off the main thread.
    func refreshSidebarCounts() {
        guard isUnlocked else { return }
        sidebarCountsGeneration &+= 1
        let generation = sidebarCountsGeneration
        Task { [weak self] in
            guard let self,
                  let r = try? await self.vaultOffMain({ v in
                      (Int(try v.countPlannedPayments()), Int(try v.countActiveMandates()), Int(try v.unseenDocumentCount()))
                  }) else { return }
            guard self.sidebarCountsGeneration == generation else { return }
            if self.plannedCount != r.0 { self.plannedCount = r.0 }
            if self.mandateCount != r.1 { self.mandateCount = r.1 }
            if self.unseenDocumentCount != r.2 { self.unseenDocumentCount = r.2 }
        }
    }

    func reloadCategories() {
        settlePendingSelectionLoad()
        guard let vault else { return }
        // `categories` are the rows the individual queries name; they are read for the request first
        let request = categorySumsRequest
        runCore {
            categories = try vault.categorySums(request)
            categoryTree = Self.buildTree(categories)
            uncategorizedFilteredCount = Int(try vault.countTransactions(filter: uncategorizedCountFilter(request)))
        }
    }

    func applyAccountGroups(_ groups: [AccountGroup], _ userGroups: [UserGroup]) {
        self.groups = groups
        self.userGroups = userGroups
        // a group removed or combined (also via sync) can't stay selected
        if case .userGroup(let u) = sidebarSelection, !userGroups.contains(where: { $0.uuid == u }) {
            sidebarSelection = .allAccounts
        }
        if case .bank(let key) = sidebarSelection, bankGroup(for: key) == nil {
            sidebarSelection = .allAccounts
        }
        rebuildAccountIndex()
    }

    func reloadDashboard() {
        guard vault != nil else { return }
        // Only the Investments scope shows it (~180 ms on 30k bookings): load it in
        // the background there, otherwise when the scope is opened (`dashboardStale`).
        // The KPI tiles fall back to it while `selectionFlow` is nil, so a first load is not skipped.
        if scope == .investments || dashboard == nil { loadDashboard() } else { dashboardTask?.cancel(); dashboardStale = true }
    }

    /// The investment dashboard reads every booking (~150 ms on 30k): off the
    /// main thread when it only follows an edit or a scope switch.
    func loadDashboard() {
        dashboardTask?.cancel()
        guard let vault else { return }
        dashboardStale = false
        dashboardTask = Task { [weak self] in
            do {
                let d = try await Self.offMain { try vault.investmentDashboard(months: 6) }
                guard !Task.isCancelled, let self, self.vault === vault else { return }
                self.dashboard = d
            } catch {
                guard !Task.isCancelled, !(error is CancellationError), let self else { return }
                self.errorMessage = describe(error)
            }
        }
    }

    /// After an edit that can change a merchant's history: drop the cache, load again.
    func reloadMerchantStats() {
        merchantStatsCache = [:]
        loadMerchantStats()
    }

    /// Merchant history of the selected booking. The core scans every booking
    /// once for it (total and own account together), so it runs off the main
    /// thread and is cached per merchant; the short delay keeps arrow-key
    /// navigation from queueing one scan per row.
    /// `keepShown`: after a data change the shown history of the same merchant
    /// stays until the fresh one arrives (no flicker).
    func loadMerchantStats(keepShown: Bool = false) {
        merchantStatsTask?.cancel()
        guard let vault, let booking = selectedBooking else {
            merchantStats = nil; merchantAccountStats = nil; merchantStatsKey = nil; return
        }
        let key = booking.merchant.key, accountId = booking.accountId
        // the account's own history lives in the same cache under a composite key
        let ownKey = "\(accountId)|\(key)"
        if let cached = merchantStatsCache[key], let own = merchantStatsCache[ownKey] {
            merchantStats = cached; merchantAccountStats = (accountId, own); merchantStatsKey = key; return
        }
        if !(keepShown && merchantStatsKey == key) { merchantStats = nil; merchantAccountStats = nil; merchantStatsKey = nil }
        merchantStatsTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(80))
            guard !Task.isCancelled else { return }
            let stats = await Task.detached(priority: .userInitiated) {
                Perf.interval("merchantStats") {
                    guard let h = try? vault.merchantHistory(merchantKey: key, accountId: accountId)
                    else { return nil as (MerchantStats, MerchantStats)? }
                    return (h.all, h.account)
                }
            }.value
            // A vault switch or data change in the meantime cancels this task.
            guard !Task.isCancelled, let self, let (all, own) = stats else { return }
            self.merchantStatsCache[key] = all
            self.merchantStatsCache[ownKey] = own
            if let now = self.selectedBooking, now.merchant.key == key {
                self.merchantStats = all; self.merchantStatsKey = key
                self.merchantAccountStats = now.accountId == accountId ? (accountId, own) : nil
            }
        }
    }

    /// Category suggestion from the history (#664) for the selected booking; the
    /// core decides which bookings get one (as in "Ohne Kategorie"). It keeps the
    /// history until the data changes; the first call after a change scans it, so
    /// off the main thread and after the same short delay as the merchant history.
    /// `keepShown`: as in `loadMerchantStats`.
    func loadCategorySuggestion(keepShown: Bool = false) {
        categorySuggestionTask?.cancel()
        // a categorised booking needs no call (and loses a shown suggestion at once)
        guard let booking = selectedBooking, booking.categoryId == nil else {
            categorySuggestion = nil; return
        }
        let id = booking.id
        if !(keepShown && categorySuggestion?.bookingId == id) { categorySuggestion = nil }
        categorySuggestionTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(80))
            guard !Task.isCancelled, let self else { return }
            // nil when the call failed or the vault changed: no stale suggestion stays
            let found = try? await self.vaultOffMain { vault in
                try Perf.interval("categorySuggestion") { try vault.suggestCategories(bookingIds: [id]).first }
            }
            guard !Task.isCancelled, self.selectedBooking?.id == id else { return }
            self.categorySuggestion = found ?? nil
        }
    }

    /// Suggestions for the uncategorised rows of the list (#664): one background
    /// call after each change of `bookings` (reload, paging, an edit's patch); the
    /// shown ones stay until the new set arrives. The core decides which rows get one
    /// and keeps the history until the data changes, so an unchanged vault costs
    /// about 1.5 ms per 100 rows.
    func loadListSuggestions() {
        listSuggestionsTask?.cancel()
        let ids = bookings.filter { $0.categoryId == nil }.map(\.id)
        guard !ids.isEmpty else {
            if !listSuggestions.isEmpty { listSuggestions = [:] }
            return
        }
        listSuggestionsTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(80))
            guard !Task.isCancelled, let self else { return }
            let found = try? await self.vaultOffMain { vault in
                try Perf.interval("listSuggestions") { try vault.suggestCategories(bookingIds: ids) }
            }
            guard !Task.isCancelled else { return }
            // a failed call or a swapped vault leaves no stale suggestions
            self.listSuggestions = Dictionary(((found ?? nil) ?? []).map { ($0.bookingId, $0) }, uniquingKeysWith: { a, _ in a })
        }
    }

    static func buildTree(_ flat: [GiroCrabCore.Category]) -> [CategoryNode] {
        func children(of parent: Int64?) -> [CategoryNode] {
            flat.filter { $0.parentId == parent }.map { c in
                let kids = children(of: c.id)
                return CategoryNode(category: c, children: kids.isEmpty ? nil : kids)
            }
        }
        return children(of: nil)
    }

    /// The stream starts with the newest bookings (one page, rounded up to whole
    /// days) and appends further pages while the user scrolls. A click on an
    /// older day in the balance chart raises the limit until that day is loaded.
    static let defaultListLimit: UInt32 = 300

    /// Every full reload calls this first: a page still on its way belongs to the old list.
    func dropPendingPage() {
        listGeneration += 1
        loadMoreTask?.cancel()
        loadMoreTask = nil
        isLoadingMoreBookings = false
    }

    /// More bookings match the current filter than are loaded (list limit reached).
    var hasMoreBookings: Bool {
        // the previous selection's page may still show during a selection load
        guard selectionLoadTask == nil, let total = page?.totalCount else { return false }
        return UInt32(bookings.count) < total
    }

    var remainingBookingCount: Int {
        max(0, Int(page?.totalCount ?? 0) - bookings.count)
    }

    /// Fetches the next page off the main thread and appends it. Nothing loaded
    /// so far is fetched again, so scrolling stays smooth.
    func loadMoreBookings() {
        guard let vault, hasMoreBookings, !isLoadingMoreBookings, selectionLoadTask == nil else { return }
        var next = listFilter
        next.offset = UInt32(bookings.count)
        next.limit = appendPageSize
        let filter = next
        let generation = listGeneration
        isLoadingMoreBookings = true
        loadMoreTask = Task { [weak self] in
            let next = try? await Self.offMain {
                try Perf.interval("listTransactions") { try vault.listTransactions(filter: filter) }
            }
            // A full reload in the meantime has replaced the list this page belongs to.
            guard let self, !Task.isCancelled, generation == self.listGeneration else { return }
            self.loadMoreTask = nil
            self.isLoadingMoreBookings = false
            guard let next, !next.transactions.isEmpty else { return }
            Perf.interval("publishBookings") {
                self.listBalances.merge(next)
                self.bookings.append(contentsOf: next.transactions)
            }
            // an edit reloads exactly what is on screen
            self.listLimit = UInt32(self.bookings.count)
        }
    }

    /// Loads the list far enough back to contain `day` (plus some older context).
    func loadBookings(through day: String) {
        guard let vault, let total = page?.totalCount, UInt32(bookings.count) < total else { return }
        if let oldest = dayGroups.last?.date, day >= oldest { return }
        // count the bookings from that day on
        var f = currentFilter
        if f.dateFrom.map({ $0 < day }) ?? true { f.dateFrom = day }
        if let newer = try? vault.countTransactions(filter: f) {
            listLimit = max(listLimit, newer + 200)
            reloadBookings()
        }
    }

    /// Raises the list limit to everything matching the current filter.
    func loadAllBookings() {
        listLimit = max(listLimit, page?.totalCount ?? listLimit)
        reloadBookings()
    }

    /// The list offers the account filter (#357): the category views, „Alle Konten“, account
    /// groups and „Neue Umsätze“. A single account has nothing to choose from.
    var showsAccountFilter: Bool { Self.offersAccountFilter(shownSelection) }

    /// The switch „Filter auch in den Kategorien anwenden“ (#1469) is offered wherever the sums do not follow the
    /// filters by themselves: „Alle Konten“, groups, banks, accounts and „Neue Umsätze“.
    var showsCategorySumsSwitch: Bool {
        switch shownSelection {
        case .account, .allAccounts, .group, .bank, .userGroup, .newBookings: true
        default: false
        }
    }

    /// The pin (#1469) sits in the filter row of the category views.
    var showsCategorySumsPin: Bool { Self.isCategoryView(shownSelection) }

    static func offersAccountFilter(_ selection: SidebarItem?) -> Bool {
        switch selection {
        case .category, .uncategorized, .allAccounts, .group, .bank, .userGroup, .newBookings: true
        case .account, .documents, .planned, .directDebits, .wealth, .none: false
        }
    }

    /// The tiles of a category view („Ohne Kategorie“, a category) count what the list shows: the
    /// category and every list filter (accounts, status, type, amount, search) narrow them. Elsewhere
    /// they keep summing the whole selection and period.
    var kpiFollowsList: Bool {
        switch sidebarSelection {
        case .category, .uncategorized: true
        default: false
        }
    }

    /// The accounts the filter chooses from: those of the selected group, otherwise all.
    var accountFilterCandidates: [Account] {
        let ids: [Int64]?
        switch sidebarSelection {
        case .group(let gid): ids = groups.first(where: { $0.id == gid })?.accounts.map(\.id)
        case .bank(let key): ids = bankGroup(for: key)?.accounts.map(\.id)
        case .userGroup(let uuid): ids = userGroupAccountIDs(uuid)
        default: ids = nil
        }
        guard let ids else { return allAccounts }
        return allAccounts.filter { ids.contains($0.id) }
    }

    /// The chosen accounts that still exist and belong to the selection, in sidebar order;
    /// empty = all of them.
    var categoryAccountIDs: [Int64] {
        guard !categoryAccountFilter.isEmpty else { return [] }
        return accountFilterCandidates.map(\.id).filter(categoryAccountFilter.contains)
    }

    /// Adds or removes one account; choosing every account is the same as no filter.
    func toggleCategoryAccount(_ id: Int64) {
        var next = Set(categoryAccountIDs)
        if !next.insert(id).inserted { next.remove(id) }
        categoryAccountFilter = next.count == accountFilterCandidates.count ? [] : next
    }

    static let categorySumsFollowAccountFilterKey = "sidebar.categorySumsFollowAccountFilter"
    static let pinnedCategoryKeysKey = "sidebar.pinnedCategoryKeys"

    /// The list filters as they are now: status, type, amount and tags, and the chosen accounts (a single account
    /// counts as its own filter). Search, merchant focus and scope never narrow the sums (#1469).
    var liveSumFilter: CategorySumFilter {
        var accounts = categoryAccountIDs
        if case .account(let id)? = sidebarSelection { accounts = [id] }
        return CategorySumFilter(accounts: accounts, status: statusFilter, direction: directionFilter, amount: amountFilter, tags: tagFilter)
    }

    /// The filters the sums of the selected row count by (#621, #1469): in the category views the current filters
    /// of the list, in the account views none (with the pin each category counts by its own, see `categorySumsRequest`).
    var categorySumFilter: CategorySumFilter {
        showsCategoryView || categorySumsFollowAccountFilter ? liveSumFilter : .unfiltered
    }

    /// The switch applies the account's filters to the sums: in the account views while it is on.
    var sumsFollowAccountFilter: Bool { !showsCategoryView && categorySumsFollowAccountFilter }

    /// The list filters as one set.
    var currentFilterSet: ListFilterSet {
        ListFilterSet(status: statusFilter, direction: directionFilter, amount: amountFilter, tags: tagFilter,
                      accounts: categoryAccountFilter, period: period,
                      customFrom: period == .custom ? customPeriodFrom : nil, customTo: period == .custom ? customPeriodTo : nil)
    }

    /// Date range the sums count: the period of the filters, else the period shown now.
    func sumsRange(_ sums: CategorySumFilter) -> (String?, String?) {
        (sums.period ?? requestedPeriod).range(salaryCycle: salaryCycle)
    }

    /// Key of a category view in the saved filters: „Ohne Kategorie“ or one category; nil for every other row.
    static func categoryFilterKey(_ selection: SidebarItem?) -> String? {
        switch selection {
        case .category(let id): "c\(id)"
        case .uncategorized: "u"
        default: nil
        }
    }

    /// The filters of each category, saved (#1469); the shown category's live ones are in `currentFilterSet`.
    static let categorySetsKey = "sidebar.categoryFilterSets"

    static func loadCategorySets() -> [String: ListFilterSet] {
        guard let data = AppDefaults.store.data(forKey: categorySetsKey),
              let stored = try? JSONDecoder().decode([String: StoredFilterSet].self, from: data) else { return [:] }
        return stored.mapValues(\.set)
    }

    /// The shown category is pinned: its filters are its own (#1469).
    var shownCategoryPinned: Bool {
        Self.categoryFilterKey(sidebarSelection).map(pinnedCategoryKeys.contains) ?? false
    }

    /// The pinned category whose filters count for `key`: the category itself, else the nearest pinned main category
    /// (a main category's filters also act on its subcategories until one is pinned itself); nil = the global ones.
    func governingKey(for key: String) -> String? {
        if pinnedCategoryKeys.contains(key) { return key }
        guard key.hasPrefix("c"), var id = Int64(key.dropFirst()) else { return nil }
        let parents = Dictionary(uniqueKeysWithValues: categories.map { ($0.id, $0.parentId) })
        while let parent = parents[id] ?? nil {
            if pinnedCategoryKeys.contains("c\(parent)") { return "c\(parent)" }
            id = parent
        }
        return nil
    }

    /// Where the shown category's filters come from: its own pin, a pinned main category, or the global ones.
    enum FilterOrigin { case own, mainCategory, global }
    var shownFilterOrigin: FilterOrigin {
        guard let key = Self.categoryFilterKey(sidebarSelection) else { return .global }
        guard let governing = governingKey(for: key) else { return .global }
        return governing == key ? .own : .mainCategory
    }

    /// Keeps the shown category's filters (called after each change in a category view): in the set that governs
    /// the category (its own, its main category's or the global one).
    func persistCategorySet() {
        guard !suspendsFilterReloads, !suspendsPeriodReload, let key = Self.categoryFilterKey(sidebarSelection) else { return }
        store(currentFilterSet, forKey: key)
    }

    private func store(_ set: ListFilterSet, forKey key: String) {
        if let governing = governingKey(for: key) {
            categorySets[governing] = set
            saveCategorySets()
        } else {
            globalCategoryFilters = set
        }
    }

    func saveCategorySets() {
        let stored = categorySets.mapValues(StoredFilterSet.init)
        if let data = try? JSONEncoder().encode(stored) { AppDefaults.store.set(data, forKey: Self.categorySetsKey) }
    }

    /// The filters of a category: those of the pin that governs it, else the global ones.
    func filterSet(forKey key: String) -> ListFilterSet {
        governingKey(for: key).map { categorySets[$0] ?? ListFilterSet() } ?? globalCategoryFilters
    }

    /// The criteria of a filter set for hints: status, type, amount, tags, accounts and the period.
    func filterCriteria(_ sums: CategorySumFilter) -> [String] {
        var parts: [String] = []
        if let checked = sums.checked { parts.append(String(localized: "Status: \((checked ? StatusFilter.checked : StatusFilter.new).title)")) }
        if sums.direction != .all { parts.append(sums.direction == .income ? String(localized: "Einnahmen") : String(localized: "Ausgaben")) }
        if sums.amount.isActive { parts.append(String(localized: "Betrag")) }
        if sums.untaggedOnly || !sums.tagIDs.isEmpty { parts.append(String(localized: "Tags")) }
        if !sums.accounts.isEmpty { parts.append(String(localized: "Konten: \(sums.accounts.count) von \(allAccounts.count)")) }
        return parts
    }

    /// The query counts by a criterion or by a period other than „Gesamter Zeitraum“.
    func isNarrowed(_ query: CategorySumQuery) -> Bool {
        !filterCriteria(query.filter).isEmpty || (query.filter.period ?? requestedPeriod).period != .all
    }

    /// Hover text of the pin symbol in the sidebar: the criteria of the category's own filters.
    func pinnedFilterSummary(forKey key: String) -> String {
        let set = Self.categoryFilterKey(sidebarSelection) == key ? currentFilterSet : filterSet(forKey: key)
        let sums = sumFilter(of: set)
        let parts = filterCriteria(sums) + [String(localized: "Zeitraum: \((sums.period ?? requestedPeriod).title)")]
        return String(localized: "Eigene Filter: \(parts.joined(separator: " · "))")
    }

    /// The pin of the shown category (#1469): pinned, its filters become its own (they start as the ones shown);
    /// released, it follows its main category's or the global filters again.
    func toggleCategoryPin() {
        guard let key = Self.categoryFilterKey(sidebarSelection) else { return }
        if pinnedCategoryKeys.contains(key) {
            pinnedCategoryKeys.remove(key)
            categorySets[key] = nil
            saveCategorySets()
            apply(filterSet(forKey: key))
        } else {
            let shown = currentFilterSet
            pinnedCategoryKeys.insert(key)
            categorySets[key] = shown
            saveCategorySets()
        }
        reloadCategorySums()
    }

    /// Shows a filter set in the list header; one reload covers all of it.
    private func apply(_ next: ListFilterSet) {
        let changesPeriod = next.period != period
            || (next.period == .custom && (next.customFrom != customPeriodFrom || next.customTo != customPeriodTo))
        suspendsFilterReloads = true
        statusFilter = next.status
        directionFilter = next.direction
        amountFilter = next.amount
        tagFilter = next.tags
        categoryAccountFilter = next.accounts
        suspendsFilterReloads = false
        if let from = next.customFrom, let to = next.customTo { customPeriodFrom = from; customPeriodTo = to }
        listLimit = Self.defaultListLimit
        if changesPeriod { period = next.period } else { reloadAfterFilterChange() }
    }

    /// The sum filters of a saved filter set (accounts in sidebar order, its own period).
    func sumFilter(of set: ListFilterSet) -> CategorySumFilter {
        var sums = CategorySumFilter(accounts: allAccounts.map(\.id).filter(set.accounts.contains), status: set.status,
                                     direction: set.direction, amount: set.amount, tags: set.tags)
        sums.period = PeriodSetting(set.period, customFrom: set.customFrom, customTo: set.customTo)
        return sums
    }

    /// Shows the filters that `selection` belongs to (#1469): the account views have their own set; a pinned category
    /// has its own; every other category shares the global ones, which are reset when an account view is chosen. The
    /// caller's selection change reloads the list and the sums.
    func switchFilterSet(to selection: SidebarItem?) {
        let oldKey = Self.categoryFilterKey(sidebarSelection)
        let newKey = Self.categoryFilterKey(selection)
        guard oldKey != newKey else { return }
        let shown = currentFilterSet
        if let oldKey {
            store(shown, forKey: oldKey)
            if newKey == nil { globalCategoryFilters = ListFilterSet() }
        } else {
            parkedAccountFilters = shown
        }
        let next = newKey.map(filterSet) ?? parkedAccountFilters
        suspendsFilterReloads = true
        defer { suspendsFilterReloads = false }
        statusFilter = next.status
        directionFilter = next.direction
        amountFilter = next.amount
        tagFilter = next.tags
        categoryAccountFilter = next.accounts
        // the selection change that follows reloads with the period of the set
        suspendsPeriodReload = true
        if let from = next.customFrom, let to = next.customTo { customPeriodFrom = from; customPeriodTo = to }
        period = next.period
        suspendsPeriodReload = false
    }

    /// „Ohne Kategorie“ or a category is chosen (the sidebar's category views).
    var showsCategoryView: Bool { Self.isCategoryView(sidebarSelection) }

    static func isCategoryView(_ selection: SidebarItem?) -> Bool {
        switch selection {
        case .category, .uncategorized: true
        default: false
        }
    }

    /// Reloads the sidebar's sums after a switch changed which filters apply (#1469).
    func reloadCategorySums() {
        guard vault != nil, categoriesRequest != categorySumsRequest else { return }
        reloadCategories()
    }

    /// What the sidebar sums load for now: `categorySumFilter` for the selected row; every other row counts by the
    /// filters of its category: its own when pinned, else the global ones (empty in the account views, #1469).
    var categorySumsRequest: CategorySumsRequest {
        let sums = categorySumFilter
        let (from, to) = sumsRange(sums)
        var request = CategorySumsRequest(base: CategorySumQuery(from: from, to: to, filter: sums))
        let shownKey = Self.categoryFilterKey(sidebarSelection)
        func query(_ key: String) -> CategorySumQuery {
            if key == shownKey { return request.base }
            // the account's filters count for every category without pinned filters of its own or its main category's (#1469)
            if sumsFollowAccountFilter, governingKey(for: key) == nil { return request.base }
            let filter = sumFilter(of: filterSet(forKey: key))
            let (f, t) = sumsRange(filter)
            return CategorySumQuery(from: f, to: t, filter: filter)
        }
        for category in categories { request.perCategory[category.id] = query("c\(category.id)") }
        request.uncategorized = query("u")
        return request
    }

    /// Hint under the sidebar's „Kategorien“ header while a filter narrows the sums; nil = none. Names the
    /// criteria and whose filter they are (the account's or the category's, #1469), never an account itself
    /// (the sidebar is part of bug report screenshots).
    var categorySumNote: String? {
        // account views without the switch: only pinned categories count by filters, their own
        if !showsCategoryView, !sumsFollowAccountFilter {
            let request = categorySumsRequest
            let own = pinnedCategoryKeys.contains { key in
                let query = key == "u" ? request.uncategorized : Int64(key.dropFirst()).flatMap { request.perCategory[$0] }
                return query.map(isNarrowed) ?? false
            }
            return own ? String(localized: "Summen gefiltert (Filter der Kategorien) – je Kategorie individuell") : nil
        }
        let sums = categorySumFilter
        var parts: [String]
        if sumsFollowAccountFilter, case .account? = sidebarSelection {
            // the selected account is no filter the user chose: it is named as such
            var named = sums
            named.accounts = []
            parts = filterCriteria(named) + [String(localized: "Aktuelles Konto")]
        } else {
            parts = filterCriteria(sums)
        }
        // with the switch on, the note shows even without a filter: it tells the account's filters reach the sums
        // a period alone is a filter, too
        guard !parts.isEmpty || sumsFollowAccountFilter || (sums.period ?? requestedPeriod).period != .all else { return nil }
        parts.append(String(localized: "Zeitraum: \((sums.period ?? requestedPeriod).title)"))
        let origin: String
        if sumsFollowAccountFilter {
            origin = String(localized: "Filter des Kontos")
        } else {
            switch shownFilterOrigin {
            case .own: origin = String(localized: "Filter dieser Kategorie, individuell")
            case .mainCategory: origin = String(localized: "Filter der Hauptkategorie")
            case .global: origin = String(localized: "Filter der Kategorien, global")
            }
        }
        return String(localized: "Summen gefiltert (\(origin)) – \(parts.joined(separator: " · "))")
    }

    /// Label of the account menu; never names an account (the header is part of bug report screenshots).
    var categoryAccountFilterTitle: String {
        let chosen = categoryAccountIDs.count
        return chosen == 0
            ? String(localized: "Konten: Alle")
            : String(localized: "Konten: \(chosen) von \(accountFilterCandidates.count)")
    }

    var currentFilter: BookingFilter {
        let (from, to) = periodRange
        var accountIDs: [Int64] = []
        var category: Int64?
        var uncategorized = false
        // Chosen accounts (#357); within a group they narrow its accounts, empty = all of them.
        let chosen = categoryAccountIDs
        switch sidebarSelection {
        case .account(let id): accountIDs = [id]
        case .group(let gid): accountIDs = chosen.isEmpty ? groups.first(where: { $0.id == gid })?.accounts.map(\.id) ?? [] : chosen
        case .bank(let key): accountIDs = chosen.isEmpty ? bankGroup(for: key)?.accounts.map(\.id) ?? [] : chosen
        case .userGroup(let uuid): accountIDs = chosen.isEmpty ? userGroupAccountIDs(uuid) : chosen
        case .category(let id): category = id; accountIDs = chosen
        case .uncategorized: uncategorized = true; accountIDs = chosen
        case .allAccounts: accountIDs = chosen
        case .documents, .newBookings, .planned, .directDebits, .wealth, .none: break
        }
        // Everything new, whatever the other list filters say; only the account and amount filters narrow it.
        if sidebarSelection == .newBookings {
            return BookingFilter(
                accountIds: chosen, categoryId: nil, includeSubcategories: true, uncategorizedOnly: false,
                search: nil, dateFrom: nil, dateTo: nil, limit: listLimit, offset: 0, scope: .all,
                merchantKey: nil, direction: .all, checked: false, wholeDays: true,
                amountMinCents: amountFilter.minCents, amountMaxCents: amountFilter.maxCents
            )
        }
        return BookingFilter(
            accountIds: accountIDs,
            categoryId: category,
            includeSubcategories: category == nil || includeSubcategories,
            uncategorizedOnly: uncategorized,
            search: coreSearchText.isEmpty ? nil : coreSearchText,
            dateFrom: from,
            dateTo: to,
            limit: listLimit,
            offset: 0,
            scope: scope,
            merchantKey: merchantFocus?.key,
            direction: directionFilter,
            checked: statusFilter == .all ? nil : statusFilter == .checked,
            wholeDays: true,
            tagIds: tagFilter.untagged ? [] : tagFilter.tagIDs.sorted(),
            untaggedOnly: tagFilter.untagged,
            amountMinCents: amountFilter.minCents,
            amountMaxCents: amountFilter.maxCents,
            // Overall money contexts (#337) — overview, kind / bank / user
            // group, also with exactly one account — sum the effective total
            // set; single account, category and proof views keep their sums.
            respectInTotal: sidebarSelection?.usesTotalsContext ?? true
        )
    }

    /// `currentFilter` for the list's own pages: with the running balances it shows (#577).
    /// Other callers (print, counts, exports) use `currentFilter` and skip that work.
    var listFilter: BookingFilter {
        var f = currentFilter
        f.withBalances = listHasBalances
        return f
    }

    /// Opens the compact export dialog (PDF / CSV / XML).
    /// - Parameter selection: use the current multi-/single selection (context menu).
    func openBookingExport(selection: Bool = false) {
        if selection {
            if selectedBookingIDs.isEmpty, let id = selectedBookingID {
                exportBookingIDs = [id]
            } else {
                exportBookingIDs = selectedBookingIDs
            }
            exportPreferSelection = !(exportBookingIDs ?? []).isEmpty
        } else {
            exportBookingIDs = nil
            exportPreferSelection = false
        }
        showExportSheet = true
    }

    // MARK: - Booking list, KPIs & history

    func reloadBookings() {
        // A synchronous refresh (account, period, edit, vault change) supersedes
        // any scope request still running in the background.
        settlePendingSelectionLoad()
        scopeLoadTask?.cancel()
        dropPendingPage()
        guard let vault else { return }
        runCore {
            let filter = listFilter
            let p = try Perf.interval("listTransactions") { try vault.listTransactions(filter: filter) }
            Perf.interval("publishBookings") { publish(p) }
        }
        reloadSelectionKPIs()
        reloadCategoryHistory()
        // Every edit, sync and import ends up here: cached merchant histories may be
        // stale, and so may a scan still running from before the change. Restart it,
        // also when a history is on screen (it would otherwise keep the old numbers).
        refreshSelectedBookingHistory()
    }

    /// After a data change: the selected booking's merchant history and category
    /// suggestion again, keeping the shown ones until the fresh ones arrive.
    func refreshSelectedBookingHistory() {
        merchantStatsCache = [:]
        loadMerchantStats(keepShown: true)
        loadCategorySuggestion(keepShown: true)
    }

    /// After a reload: the multiple selection keeps only bookings the list still
    /// shows (plus the inspector's own booking, which may be sticky).
    func pruneMultiSelection() {
        guard !selectedBookingIDs.isEmpty else { return }
        let visible = Set(visibleBookings.map(\.id))
        let kept = selectedBookingIDs.filter { visible.contains($0) || $0 == selectedBookingID }
        if kept.count != selectedBookingIDs.count { selectedBookingIDs = kept }
    }

    /// Shows a freshly loaded list page. A selected booking the page no longer
    /// contains falls back to the first row, unless it is sticky (kept in the
    /// inspector after an edit moved it out of the filter).
    func publish(_ p: BookingPage) {
        page = p
        var balances = ListBalances(currency: balanceCurrency)
        balances.merge(p)
        listBalances = balances
        bookings = keepingLingeringRow(p.transactions)
        if let id = selectedBookingID, booking(withID: id) == nil, stickyBooking?.id != id {
            // „Neue Umsätze“: no booking selected unasked, auto review would check it
            selectedBookingID = sidebarSelection == .newBookings ? nil : p.transactions.first?.id
        }
        pruneMultiSelection()
    }

    /// Running balances (#577) fit only a selection of accounts (one account, a
    /// group, all accounts), not a category or „Neue Umsätze“, whose rows come
    /// from accounts the view does not name. The list and the CSV export share it.
    var selectionHasBalances: Bool {
        switch sidebarSelection {
        case .account, .group, .bank, .userGroup, .allAccounts: true
        default: false
        }
    }

    /// The list shows running balances: Ansicht › Saldo anzeigen and a selection of accounts.
    var listHasBalances: Bool {
        AppDefaults.store.bool(forKey: ViewLayout.runningBalanceKey) && selectionHasBalances
    }

    /// Currency the core sums a selection of accounts in (#1049, as `counts_in_sums`): exactly one
    /// account its own, otherwise (none, several, all) EUR only.
    func sumCurrency(forAccounts ids: [Int64]) -> String {
        if ids.count == 1, let a = account(for: ids[0]), !a.currency.isEmpty { return a.currency.uppercased() }
        return "EUR"
    }

    /// Currency of the sidebar's category sums: they follow the account filter of the category views.
    var categorySumCurrency: String { sumCurrency(forAccounts: categorySumFilter.accounts) }

    /// Currency of the running balances, KPIs and chart: that of the accounts the list shows
    /// (one account, also a group or filter that narrows to one, its own; else the EUR sum).
    var balanceCurrency: String {
        switch sidebarSelection {
        case .documents, .planned, .directDebits, .wealth, .none: return "EUR"
        default: return sumCurrency(forAccounts: currentFilter.accountIds)
        }
    }

    /// Ansicht › Saldo anzeigen changed (also from the settings window): switching it on
    /// reloads a list that can show balances; switching it off only drops them.
    func runningBalanceSettingChanged(on: Bool) {
        if on, selectionHasBalances {
            reloadBookings()
        } else {
            listBalances = ListBalances()
        }
    }

    /// The lingering booking (checked while selected) at its old place in a
    /// new page that no longer contains it: before the row that followed it.
    private func keepingLingeringRow(_ rows: [Booking]) -> [Booking] {
        guard let id = lingeringBookingID, let kept = stickyBooking, kept.id == id,
              !rows.contains(where: { $0.id == id }),
              let old = bookings.firstIndex(where: { $0.id == id }) else { return rows }
        let ids = Set(rows.map(\.id))
        let next = bookings[(old + 1)...].first { ids.contains($0.id) }
        var out = rows
        out.insert(kept, at: next.flatMap { n in out.firstIndex { $0.id == n.id } } ?? out.endIndex)
        return out
    }

    /// A status / direction change: from a header menu the list, KPIs and sidebar sums load in
    /// the background (the menu closes at once, the old list stays until the new one is in);
    /// set from code it stays synchronous, callers read the list right after.
    func reloadAfterFilterChange() {
        if loadsFilterInBackground, vault != nil {
            startSelectionLoad(keepingShown: true)
        } else {
            reloadBookings()
            reloadCategories()
        }
    }

    /// The search text only narrows the list: the KPIs, history and merchant data do not
    /// depend on it, so just the page reloads, in the background.
    func reloadListInBackground() {
        if selectionLoadTask != nil { startSelectionLoad() } else { reloadScopeBookings() }
    }

    /// Let the segmented control finish updating immediately. Database work
    /// and UniFFI conversion must not run inside its selection setter.
    func reloadScopeBookings() {
        scopeLoadTask?.cancel()
        dropPendingPage()
        guard let vault else { return }
        let filter = listFilter
        scopeLoadTask = Task { [weak self] in
            guard !Task.isCancelled else { return }
            let load = Task.detached(priority: .userInitiated) {
                try Task.checkCancellation()
                return try vault.listTransactions(filter: filter)
            }
            do {
                let p = try await withTaskCancellationHandler {
                    try await load.value
                } onCancel: {
                    load.cancel()
                }
                // Rapid switches or a different vault/filter must never publish
                // a result from an older request, including an older error.
                guard !Task.isCancelled, let self else { return }
                // a page requested while this scope was loading belongs to no list
                self.dropPendingPage()
                self.publish(p)
                // selectionFlow always uses scope .all and is unchanged here.
            } catch {
                guard !Task.isCancelled, let self else { return }
                self.errorMessage = describe(error)
            }
        }
    }

    /// Reads what the EUR-only sums leave out (#1049) for the current selection: the KPI tiles per
    /// further currency and the sidebar totals per category. Runs after every reload of the sums; a newer
    /// call cancels an older one, and both queries return at once while no booking has another currency.
    func refreshOtherCurrencies() {
        otherCurrenciesTask?.cancel()
        guard vault != nil else {
            if !otherCurrencyFlows.isEmpty { otherCurrencyFlows = [] }
            if !categoryOtherTotals.isEmpty { categoryOtherTotals = [:] }
            return
        }
        let (from, to) = periodRange
        let filter = currentFilter
        let followsList = filter.listHasFlow || kpiFollowsList
        let sums = categorySumFilter
        let (sumsFrom, sumsTo) = sumsRange(sums)
        otherCurrenciesTask = Task { [weak self] in
            guard let self else { return }
            do {
                let loaded = try await self.vaultOffMain { vault -> ([CurrencyFlow], [CategoryCurrencyAmount]) in
                    let flows = followsList && !filter.listHasFlow
                        ? try vault.flowOtherCurrencies(filter: filter)
                        : try vault.selectionFlowOtherCurrencies(accountIds: filter.accountIds, dateFrom: from, dateTo: to, respectInTotal: filter.respectInTotal)
                    let totals = try vault.categoryOtherSums(from: sumsFrom, to: sumsTo, filter: sums)
                    return (flows, totals)
                }
                guard !Task.isCancelled, let loaded else { return }
                if self.otherCurrencyFlows != loaded.0 { self.otherCurrencyFlows = loaded.0 }
                let grouped = Dictionary(grouping: loaded.1, by: \.categoryId).mapValues { $0.map { CurrencyAmount(currency: $0.currency, cents: $0.cents) } }
                if self.categoryOtherTotals != grouped { self.categoryOtherTotals = grouped }
            } catch {
                // a missing "Zusätzlich" line is not worth an error alert
            }
        }
    }

    /// "Zusätzlich: −30,00 $ · …" for one KPI tile (nil: nothing in other currencies); `pick` reads the tile's value.
    func otherCurrencyText(_ pick: (CurrencyFlow) -> Int64) -> String? {
        let parts = otherCurrencyFlows.compactMap { f -> String? in
            let cents = pick(f)
            return cents == 0 ? nil : Money.format(cents, currency: f.currency)
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// KPI tiles: same flow rules as the dashboard, but for the selected
    /// period and account/group (no account selected = all accounts).
    func reloadSelectionKPIs() {
        guard let vault else { selectionFlow = nil; return }
        let (from, to) = periodRange
        let filter = currentFilter
        // The list query already summed the same bookings unless it is narrowed further.
        if let page, filter.listHasFlow || kpiFollowsList {
            selectionFlow = (page.flow.consumptionCents, page.flow.investedCents, page.flow.incomeCents)
            return
        }
        let ids = filter.accountIds
        let forTotals = filter.respectInTotal
        runCore {
            // the fallback must run in the filter's own calculation context
            // (#337): overall figures respect the effective total set
            let f = try (forTotals
                ? vault.selectionFlowForTotals(accountIds: ids, dateFrom: from, dateTo: to)
                : vault.selectionFlow(accountIds: ids, dateFrom: from, dateTo: to))
            selectionFlow = (f.consumptionCents, f.investedCents, f.incomeCents)
        }
    }

    /// Bar width actually used: own choice (also quarters, #634), else the global setting, else months.
    var effectiveCategoryBarUnit: BalanceBarUnit {
        if categoryBarUnit != .automatic { return categoryBarUnit }
        let global = BalanceBarUnit(rawValue: AppDefaults.store.string(forKey: BalanceBarUnit.defaultsKey) ?? "") ?? .automatic
        return global == .automatic ? .month : global
    }

    /// Request for the category chart: nil unless a category is selected.
    var categoryHistoryRequest: (id: Int64, accounts: [Int64], from: String?, to: String?, keyLength: UInt32)? {
        guard case .category(let id) = sidebarSelection else { return nil }
        let (from, to) = periodRange
        return (id, categoryAccountIDs, from, to, UInt32(effectiveCategoryBarUnit.keyLength))
    }

    /// Reloads the monthly sums of the selected category (cheap SQL aggregate).
    func reloadCategoryHistory() {
        guard let vault, let r = categoryHistoryRequest else {
            if !categoryHistory.isEmpty { categoryHistory = [] }
            if !categoryOtherCurrencies.isEmpty { categoryOtherCurrencies = [] }
            return
        }
        runCore {
            categoryHistory = try vault.categoryHistoryByUnit(categoryId: r.id, accountIds: r.accounts, dateFrom: r.from, dateTo: r.to, keyLength: r.keyLength)
            categoryOtherCurrencies = try vault.categoryOtherCurrencies(categoryId: r.id, accountIds: r.accounts, dateFrom: r.from, dateTo: r.to)
        }
    }

    func reloadHistory() {
        reloadCategoryHistory()
        settlePendingSelectionLoad()
        guard let vault else { return }
        let (ids, days) = historyRequest
        let (from, to) = periodRange
        runCore { history = Self.trimmedHistory(try vault.balanceHistory(accountIds: ids, days: days), from: from, to: to) }
    }

    /// Accounts and day count of the balance chart for the current selection and period.
    var historyRequest: (ids: [Int64], days: UInt32) {
        let ids = chartAccountIDs(sidebarSelection)
        // The chart follows the selected period. "Gesamter Zeitraum": up to ~15 years back,
        // then the flat lead-in before the first booking is cut off.
        let today = Calendar.current.startOfDay(for: Date())
        let start = periodRange.0.flatMap(DateText.date) ?? Calendar.current.date(byAdding: .year, value: -15, to: today)!
        let days = max(2, (Calendar.current.dateComponents([.day], from: start, to: today).day ?? 120) + 1)
        return (ids, UInt32(days))
    }

    /// Accounts behind the header chart (balance history, expense pie #634) of a selection.
    private func chartAccountIDs(_ selection: SidebarItem?) -> [Int64] {
        let ids: [Int64]
        switch selection {
        case .account(let id): ids = [id]
        // same accounts as the group's sum in the header (#208), and like that
        // sum only the effectively included ones (#337; `inTotal` also covers
        // archiving and the account's own switch)
        case .group(let gid): ids = groups.first(where: { $0.id == gid })?.accounts.filter(\.inTotal).map(\.id) ?? []
        case .bank(let key): ids = bankGroup(for: key)?.accounts.filter(\.inTotal).map(\.id) ?? []
        case .userGroup(let uuid): ids = userGroupBalanceAccountIDs(uuid)
        default: ids = liquidAccounts.map(\.id)
        }
        return ids.isEmpty ? [-1] : ids
    }

    /// Accounts behind the header's expense pie: those of the selection shown (#1413).
    var headerChartAccountIDs: [Int64] { chartAccountIDs(shownSelection) }

    static func trimmedHistory(_ all: [DailyBalance], from: String?, to: String?) -> [DailyBalance] {
        var points = all
        if let to { points = points.filter { $0.date <= to } }
        if from == nil,
           let firstChange = points.indices.dropFirst().first(where: { points[$0].balanceCents != points[$0 - 1].balanceCents }) {
            // keep one point before the first change
            points = Array(points[(firstChange - 1)...])
        } else if from == nil, points.count > 366 {
            // nothing ever changed (e.g. a depot without bookings): the last year is enough
            points = Array(points.suffix(366))
        }
        return points
    }

    func reloadRules() {
        guard let vault else { return }
        rulesStale = false
        runCore { rules = try vault.listRules() }
    }

    func reloadSettings() {
        guard let vault else { return }
        runCore {
            settings = try vault.listSettings()
            logoProvider = try vault.onlineLogoProvider()
            merchantRecognitionEnabled = try vault.merchantRecognitionEnabled()
        }
        reloadProductIDState(vault)
    }

    // MARK: - Derived

    /// The loaded bookings; every filter of the list is applied by the core.
    /// „Neue Umsätze“ shows the bookings per account: ↑/↓ and ranges follow the cards.
    var visibleBookings: [Booking] {
        shownSelection == .newBookings ? newBookingSections.flatMap(\.bookings) : bookings
    }

    /// „Neue Umsätze“: the loaded bookings as one card per account.
    var newBookingSections: [NewBookingsSection] { NewBookings.sections(bookings: bookings, accounts: allAccounts, keeping: lingeringBookingID) }

    /// Jump to `date` in the booking stream: that day, or the nearest earlier
    /// day with bookings (the balance of a quiet day comes from there).
    func jumpToDay(_ date: Date) {
        let day = DateText.iso(date)
        loadBookings(through: day)
        let groups = dayGroups // newest first
        guard let target = groups.first(where: { $0.date <= day }) ?? groups.last else { return }
        scrollToDay = DayJump(day: target.date)
    }

    /// The stream day the chart marker points to: the hovered chart day (or the nearest earlier day
    /// with bookings), else the day at the top of the scrolled list.
    func updateStreamHighlight() {
        let target = (chartHoverDay ?? scrollFocusDay).flatMap { day in
            dayGroups.first(where: { $0.date <= day })?.date // newest first
        }
        if streamHighlightDay != target { streamHighlightDay = target }
    }

    func rebuildBookingIndex() {
        var index: [Booking.ID: Int] = [:]
        index.reserveCapacity(bookings.count)
        for (i, b) in bookings.enumerated() { index[b.id] = i }
        bookingIndexByID = index
        rebuildDayGroups()
    }

    func rebuildDayGroups() {
        var out: [DayGroup] = []
        var current: [Booking] = []
        var day = ""
        for b in visibleBookings {
            if b.bookingDate != day, !current.isEmpty {
                out.append(DayGroup(date: day, bookings: current))
                current = []
            }
            day = b.bookingDate
            current.append(b)
        }
        if !current.isEmpty { out.append(DayGroup(date: day, bookings: current)) }
        dayGroups = out
    }

    func booking(withID id: Booking.ID) -> Booking? {
        bookingIndexByID[id].map { bookings[$0] }
    }

    var selectedBooking: Booking? {
        guard let id = selectedBookingID else { return nil }
        return booking(withID: id) ?? (stickyBooking?.id == id ? stickyBooking : nil)
    }

    var allAccounts: [Account] { groups.flatMap(\.accounts) }
    var hasArchivedAccounts: Bool { allAccounts.contains(where: \.isArchived) }
    /// Everyday accounts counted in "Verfügbar" (skips groups excluded from the total).
    var liquidAccounts: [Account] { allAccounts.filter { $0.inTotal && [.checking, .savings, .paymentService, .creditCard, .cash].contains($0.kind) } }
    var liquidBalance: Int64 { liquidAccounts.reduce(0) { $0 + $1.balanceCents } }
    var newCount: Int { allAccounts.reduce(0) { $0 + Int($1.newCount) } }
    /// Bookings without a category (badge of „Ohne Kategorie“): the number the „Ohne Kategorie“ list shows for the
    /// selected period and the same status, type, amount and account filters as the category sums (#1439).
    /// Until the first count has loaded, the accounts' own counts (all time) stand in.
    var uncategorizedCount: Int {
        uncategorizedFilteredCount ?? allAccounts.filter { $0.kind != .loan }.reduce(0) { $0 + Int($1.uncategorizedCount) }
    }

    /// The query behind the badge: what the „Ohne Kategorie“ list loads (`currentFilter`, which leaves out detected own
    /// transfers like the list, #302) for the sidebar's period and `categorySumFilter`, counted in SQL (limit 1).
    /// No chosen accounts = all of them, as in the list.
    func uncategorizedCountFilter(_ sums: CategorySumFilter) -> BookingFilter {
        let (from, to) = sumsRange(sums)
        return uncategorizedCountFilter(CategorySumQuery(from: from, to: to, filter: sums))
    }

    /// The badge's query for the request: „Ohne Kategorie“ counts by its own filters where the categories do.
    func uncategorizedCountFilter(_ request: CategorySumsRequest) -> BookingFilter {
        uncategorizedCountFilter(request.uncategorized ?? request.base)
    }

    func uncategorizedCountFilter(_ query: CategorySumQuery) -> BookingFilter {
        let (from, to, sums) = (query.from, query.to, query.filter)
        return BookingFilter(
            accountIds: sums.accounts, categoryId: nil, includeSubcategories: true, uncategorizedOnly: true,
            search: nil, dateFrom: from, dateTo: to, limit: 1, offset: 0, scope: .all,
            merchantKey: nil, direction: sums.direction, checked: sums.checked, wholeDays: true,
            tagIds: sums.tagIDs, untaggedOnly: sums.untaggedOnly,
            amountMinCents: sums.amount.minCents, amountMaxCents: sums.amount.maxCents)
    }

    /// Everyday spending this month (never includes ETF plans / perks).
    /// The tiles have no figures for the shown selection: it is loading, or its load failed
    /// (`selectionFlow` may still hold the previous selection's).
    var selectionFiguresMissing: Bool { isLoadingSelection || page == nil }
    /// Selected account/group and period; falls back to the dashboard month.
    var consumptionThisMonth: Int64 { selectionFlow?.consumption ?? dashboard?.monthly.last?.consumptionCents ?? 0 }
    var investedThisMonth: Int64 { selectionFlow?.invested ?? dashboard?.monthly.last?.investmentCents ?? 0 }
    var incomeThisMonth: Int64 { selectionFlow?.income ?? dashboard?.monthly.last?.incomeCents ?? 0 }

    /// Number of bookings matching the list filter (the list itself loads them page by page).
    var bookingCountLabel: Int {
        Int(page?.totalCount ?? UInt32(bookings.count))
    }

    var title: String {
        switch shownSelection {
        case .allAccounts, .none: return String(localized: "Alle Konten")
        case .group(let id): return groups.first { $0.id == id }?.name ?? String(localized: "Gruppe")
        case .bank(let key): return bankGroup(for: key)?.name ?? String(localized: "Bank")
        case .userGroup(let uuid): return userGroups.first { $0.uuid == uuid }?.name ?? String(localized: "Gruppe")
        case .account(let id): return allAccounts.first { $0.id == id }?.name ?? String(localized: "Konto")
        case .category(let id): return categories.first { $0.id == id }?.path ?? String(localized: "Kategorie")
        case .uncategorized: return uncategorizedLabel
        case .documents: return String(localized: "Belege & Postbox")
        case .newBookings: return String(localized: "Neue Umsätze")
        case .planned: return String(localized: "Geplant")
        case .directDebits: return String(localized: "Lastschriften")
        case .wealth: return String(localized: "Vermögen")
        }
    }

    /// Accounts behind the header total of the current selection (nil: single account / not a total).
    /// Like the sum it shows: only effectively included accounts (#337).
    private var headerTotalAccounts: [Account]? {
        switch shownSelection {
        case .allAccounts, .none: return liquidAccounts
        case .group(let id): return groups.first { $0.id == id }?.accounts.filter(\.inTotal)
        case .bank(let key): return bankGroup(for: key)?.accounts.filter(\.inTotal)
        case .userGroup(let uuid):
            let ids = Set(userGroupAccountIDs(uuid))
            return allAccounts.filter { ids.contains($0.id) && $0.inTotal }
        default: return nil
        }
    }

    /// The header total split into base currency + further currencies (#569).
    var headerTotalSplit: HeaderTotal.Split? {
        guard let accounts = headerTotalAccounts else { return nil }
        let split = HeaderTotal.split(accounts.map { ($0.currency, $0.balanceCents) })
        return split.others.isEmpty ? nil : split
    }

    var headerBalance: (label: String, cents: Int64)? {
        if let split = headerTotalSplit, let hb = headerBalanceUnsplit { return (hb.label, split.cents) }
        return headerBalanceUnsplit
    }

    private var headerBalanceUnsplit: (label: String, cents: Int64)? {
        // A group's or all accounts' total covers every account: no figure while accounts are filtered (#357).
        switch shownSelection {
        case .group, .bank, .userGroup, .allAccounts: if !categoryAccountIDs.isEmpty { return nil }
        default: break
        }
        switch shownSelection {
        case .account(let id): return allAccounts.first { $0.id == id }.map { (isTangible(id) ? String(localized: "Wert heute") : depotKind(of: id) == .securities ? String(localized: "Depotwert") : String(localized: "Kontostand"), $0.balanceCents) }
        case .group(let id): return groups.first { $0.id == id }.map { (String(localized: "Summe"), $0.balanceCents) }
        case .bank(let key): return bankGroup(for: key).map { (String(localized: "Summe"), $0.balanceCents) }
        case .userGroup(let uuid): return userGroups.first { $0.uuid == uuid }.map { (String(localized: "Summe"), $0.balanceCents) }
        case .allAccounts, .none: return (String(localized: "Verfügbar (Giro, Tagesgeld, Karten)"), liquidBalance)
        // The category total covers all accounts: no figure while accounts are filtered (#406).
        case .category(let id): return categoryAccountIDs.isEmpty ? categories.first { $0.id == id }.map { (String(localized: "Summe im Zeitraum"), $0.totalCents) } : nil
        case .uncategorized, .documents, .newBookings, .planned, .directDebits, .wealth: return nil
        }
    }

    var lastSyncText: String {
        let latest = connections.compactMap(\.lastSyncUnix).max()
        guard let latest else { return String(localized: "Noch nie aktualisiert") }
        let relative = Self.relativeSyncText(latest)
        return String(localized: "Aktualisiert \(relative)")
    }

    /// Last fetch of the account's bank connection (#815). The core stores the time per connection, not per
    /// account, so accounts of one bank show the same value. `nil` for offline accounts (they are never fetched).
    func lastFetchText(for account: Account) -> (text: String, help: String?)? {
        guard !isOffline(account.id) else { return nil }
        guard let latest = connections.first(where: { $0.id == account.connectionId })?.lastSyncUnix else {
            return (String(localized: "Noch nie abgerufen"), nil)
        }
        let relative = Self.relativeSyncText(latest)
        let exact = Date(timeIntervalSince1970: TimeInterval(latest)).formatted(date: .long, time: .shortened)
        return (String(localized: "Abgerufen \(relative)"), exact)
    }

    private static func relativeSyncText(_ unix: Int64) -> String {
        let f = RelativeDateTimeFormatter()
        f.locale = .autoupdatingCurrent
        f.unitsStyle = .full
        return f.localizedString(for: Date(timeIntervalSince1970: TimeInterval(unix)), relativeTo: Date())
    }

    func account(for id: Int64) -> Account? { accountsByID[id] }

    // MARK: - Account markers (which account a booking belongs to)

    /// The account's colour (stored in the core: the user's choice, else a
    /// default from its sync identity – the same on every device). Looked up
    /// in `accountMarkers`, built once per account reload.
    func accountColor(_ accountId: Int64) -> Color {
        accountMarkers[accountId]?.color ?? Theme.tertiary
    }

    /// Short account label for the booking list, e.g. "Girokonto" – with
    /// "··3000" only when another account has the same name.
    func accountSymbol(_ accountId: Int64) -> String {
        accountMarkers[accountId]?.symbol ?? "building.columns"
    }

    func accountShortLabel(_ accountId: Int64) -> String {
        accountMarkers[accountId]?.label ?? ""
    }

    /// Markers only where the list mixes bookings of several accounts.
    func refreshAccountMarkerVisibility() {
        showsAccountMarkers = listShowsSeveralAccounts // equal values do not notify
    }

    private var listShowsSeveralAccounts: Bool {
        switch shownSelection {
        case .account: false
        case .group(let id): (groups.first { $0.id == id }?.accounts.count ?? 0) > 1
        case .bank(let key): (bankGroup(for: key)?.accounts.count ?? 0) > 1
        case .userGroup(let uuid): userGroupAccountIDs(uuid).count > 1
        case .newBookings: false // the card header names the account
        default: accountMarkers.count > 1
        }
    }

    func setAccountColor(_ accountId: Int64, hex: String?) {
        guard let vault else { return }
        runCore { try vault.setAccountColor(accountId: accountId, colorHex: hex) }
        reloadAccounts()
    }

    /// Back to the symbol of the account kind; a bank sync keeps a chosen symbol.
    func resetAccountSymbol(_ accountId: Int64) {
        guard let vault else { return }
        runCore { try vault.resetAccountSymbol(accountId: accountId) }
        reloadAccounts()
    }

    func setAccountSymbol(_ accountId: Int64, symbol: String) {
        guard let vault else { return }
        runCore { try vault.setAccountSymbol(accountId: accountId, sfSymbol: symbol) }
        reloadAccounts()
    }

    func rebuildAccountIndex() {
        reloadLoans()
        reloadAssets()
        reloadOfflineAccounts()
        let accounts = allAccounts
        accountsByID = Dictionary(accounts.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        if let vault {
            uncategorizedName = (((try? vault.getSetting(key: "ui.uncategorized_name")) ?? nil) ?? "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        // " (MoneyMoney)": suffix that earlier versions appended to accounts created by a file import.
        func base(_ a: Account) -> String {
            a.name.replacingOccurrences(of: " (Import)", with: "").replacingOccurrences(of: " (MoneyMoney)", with: "")
        }
        let nameCount = Dictionary(accounts.map { (base($0).lowercased(), 1) }, uniquingKeysWith: +)
        accountMarkers = Dictionary(accounts.map { a in
            var label = base(a)
            if nameCount[label.lowercased(), default: 0] > 1 {
                let digits = String((a.iban.isEmpty ? a.accountNumber : a.iban).filter(\.isNumber).suffix(4))
                if !digits.isEmpty && !label.contains(digits) { label += " ··\(digits)" }
            }
            return (a.id, AccountMarker(label: label, color: Color.readable(hex: a.colorHex), symbol: a.sfSymbol))
        }, uniquingKeysWith: { a, _ in a })
    }
}
