import Foundation
import SwiftUI
import GiroCrabCore

extension AppModel {
    // MARK: - Loading

    func reloadAll() {
        let all = Perf.signposter.beginInterval("reloadAll")
        defer { Perf.signposter.endInterval("reloadAll", all) }
        reloadAccounts()
        refreshSalaryCycle()
        refreshRecurring()
        mirrorDocumentsIfEnabled()
        reloadHistory()
        reloadCategories()
        reloadBookings()
        reloadRules()
        reloadSettings()
        reloadDashboard()
    }

    func reloadAccounts() {
        guard let vault else { return }
        run {
            applyAccountGroups(try vault.listAccountGroups(), try vault.listUserGroups())
            connections = try vault.listBankConnections()
            connectionKeys = Dictionary(uniqueKeysWithValues: connections.compactMap { c in
                (try? vault.connectionKey(connectionId: c.id)).map { (c.id, $0) }
            })
        }
    }

    func reloadCategories() {
        settlePendingSelectionLoad()
        guard let vault else { return }
        let (from, to) = periodRange
        run {
            categories = try vault.listCategories(dateFrom: from, dateTo: to)
            categoryTree = Self.buildTree(categories)
        }
    }

    func reloadDashboard() {
        guard let vault else { return }
        dashboardTask?.cancel()
        dashboardStale = false
        run { dashboard = try vault.investmentDashboard(months: 6) }
    }

    /// After an edit that can change a merchant's history: drop the cache, load again.
    func reloadMerchantStats() {
        merchantStatsCache = [:]
        loadMerchantStats()
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
        var next = currentFilter
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
                self.bookings.append(contentsOf: next.transactions)
            }
            // an edit reloads exactly what is on screen
            self.listLimit = UInt32(self.bookings.count)
        }
    }

    /// Raises the list limit to everything matching the current filter.
    func loadAllBookings() {
        listLimit = max(listLimit, page?.totalCount ?? listLimit)
        reloadBookings()
    }

    var currentFilter: BookingFilter {
        let (from, to) = periodRange
        var accountIDs: [Int64] = []
        var category: Int64?
        var uncategorized = false
        switch sidebarSelection {
        case .account(let id): accountIDs = [id]
        case .group(let gid): accountIDs = groups.first(where: { $0.id == gid })?.accounts.map(\.id) ?? []
        case .userGroup(let uuid): accountIDs = userGroupAccountIDs(uuid)
        case .category(let id): category = id
        case .uncategorized: uncategorized = true
        case .allAccounts, .documents, .none: break
        }
        return BookingFilter(
            accountIds: accountIDs,
            categoryId: category,
            includeSubcategories: true,
            uncategorizedOnly: uncategorized,
            search: searchText.isEmpty ? nil : searchText,
            dateFrom: from,
            dateTo: to,
            limit: listLimit,
            offset: 0,
            scope: scope,
            merchantKey: nil,
            direction: directionFilter,
            checked: statusFilter == .all ? nil : statusFilter == .checked,
            wholeDays: true
        )
    }

    /// Exports every booking matching the current list filter as Excel-friendly CSV.
    func exportFilteredBookingsCSV() {
        guard let vault else { return }
        do {
            let csv = try vault.exportBookingsCsv(filter: currentFilter)
            csvExport = CSVFileDocument(data: Data(csv.utf8))
            showCSVExporter = true
        } catch {
            showError(String(localized: "Export fehlgeschlagen"), error.localizedDescription)
        }
    }

    // MARK: - Booking list, KPIs & history

    func reloadBookings() {
        // A synchronous refresh (account, period, edit, vault change) supersedes
        // any scope request still running in the background.
        settlePendingSelectionLoad()
        scopeLoadTask?.cancel()
        dropPendingPage()
        guard let vault else { return }
        run {
            let filter = currentFilter
            let p = try Perf.interval("listTransactions") { try vault.listTransactions(filter: filter) }
            Perf.interval("publishBookings") { publish(p) }
        }
        reloadSelectionKPIs()
        // Every edit, sync and import ends up here: cached merchant histories may be
        // stale, and so may a scan still running from before the change. Restart it,
        // also when a history is on screen (it would otherwise keep the old numbers).
        merchantStatsCache = [:]
        loadMerchantStats(keepShown: true)
    }

    /// After a reload: the multiple selection keeps only bookings the list still
    /// shows (plus the inspector's own booking, which may be sticky).
    func pruneMultiSelection() {
        guard !selectedBookingIDs.isEmpty else { return }
        let visible = Set(visibleBookings.map(\.id))
        let kept = selectedBookingIDs.filter { visible.contains($0) || $0 == selectedBookingID }
        if kept.count != selectedBookingIDs.count { selectedBookingIDs = kept }
    }

    /// Let the segmented control finish updating immediately. Database work
    /// and UniFFI conversion must not run inside its selection setter.
    func reloadScopeBookings() {
        scopeLoadTask?.cancel()
        dropPendingPage()
        guard let vault else { return }
        let filter = currentFilter
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

    /// KPI tiles: same flow rules as the dashboard, but for the selected
    /// period and account/group (no account selected = all accounts).
    func reloadSelectionKPIs() {
        guard let vault else { selectionFlow = nil; return }
        let (from, to) = periodRange
        let filter = currentFilter
        // The list query already summed the same bookings unless it is narrowed further.
        if let page, filter.listHasFlow {
            selectionFlow = (page.flow.consumptionCents, page.flow.investedCents, page.flow.incomeCents)
            return
        }
        let ids = filter.accountIds
        run {
            let f = try vault.selectionFlow(accountIds: ids, dateFrom: from, dateTo: to)
            selectionFlow = (f.consumptionCents, f.investedCents, f.incomeCents)
        }
    }

    func reloadHistory() {
        settlePendingSelectionLoad()
        guard let vault else { return }
        let (ids, days) = historyRequest
        let (from, to) = periodRange
        run { history = Self.trimmedHistory(try vault.balanceHistory(accountIds: ids, days: days), from: from, to: to) }
    }

    func reloadRules() {
        guard let vault else { return }
        rulesStale = false
        run { rules = try vault.listRules() }
    }

    func reloadSettings() {
        guard let vault else { return }
        run {
            settings = try vault.listSettings()
            logoProvider = try vault.onlineLogoProvider()
        }
        reloadProductIDState(vault)
    }

    // MARK: - Derived

    /// The loaded bookings; every filter of the list is applied by the core.
    var visibleBookings: [Booking] { bookings }

    /// Jump to `date` in the booking stream: that day, or the nearest earlier
    /// day with bookings (the balance of a quiet day comes from there).
    func jumpToDay(_ date: Date) {
        let day = DateText.iso(date)
        loadBookings(through: day)
        let groups = dayGroups // newest first
        guard let target = groups.first(where: { $0.date <= day }) ?? groups.last else { return }
        scrollToDay = DayJump(day: target.date)
    }

    func rebuildBookingIndex() {
        var index: [Booking.ID: Int] = [:]
        index.reserveCapacity(bookings.count)
        for (i, b) in bookings.enumerated() { index[b.id] = i }
        bookingIndexByID = index
        rebuildDayGroups()
    }

    private func rebuildDayGroups() {
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
    var uncategorizedCount: Int { allAccounts.filter { $0.kind != .loan }.reduce(0) { $0 + Int($1.uncategorizedCount) } }

    /// Everyday spending this month (never includes ETF plans / perks).
    /// Selected account/group and period; falls back to the dashboard month.
    var consumptionThisMonth: Int64 { selectionFlow?.consumption ?? dashboard?.monthly.last?.consumptionCents ?? 0 }
    var investedThisMonth: Int64 { selectionFlow?.invested ?? dashboard?.monthly.last?.investmentCents ?? 0 }
    var incomeThisMonth: Int64 { selectionFlow?.income ?? dashboard?.monthly.last?.incomeCents ?? 0 }

    /// Number of bookings matching the list filter (the list itself loads them page by page).
    var bookingCountLabel: Int {
        Int(page?.totalCount ?? UInt32(bookings.count))
    }

    var title: String {
        switch sidebarSelection {
        case .allAccounts, .none: return String(localized: "Übersicht")
        case .group(let id): return groups.first { $0.id == id }?.name ?? String(localized: "Gruppe")
        case .userGroup(let uuid): return userGroups.first { $0.uuid == uuid }?.name ?? String(localized: "Gruppe")
        case .account(let id): return allAccounts.first { $0.id == id }?.name ?? String(localized: "Konto")
        case .category(let id): return categories.first { $0.id == id }?.path ?? String(localized: "Kategorie")
        case .uncategorized: return String(localized: "Ohne Kategorie")
        case .documents: return String(localized: "Belege & PostBox")
        }
    }

    var headerBalance: (label: String, cents: Int64)? {
        switch sidebarSelection {
        case .account(let id): return allAccounts.first { $0.id == id }.map { (String(localized: "Kontostand"), $0.balanceCents) }
        case .group(let id): return groups.first { $0.id == id }.map { (String(localized: "Summe"), $0.balanceCents) }
        case .userGroup(let uuid): return userGroups.first { $0.uuid == uuid }.map { (String(localized: "Summe"), $0.balanceCents) }
        case .allAccounts, .none: return (String(localized: "Verfügbar (Giro, Tagesgeld, Karten)"), liquidBalance)
        case .category(let id): return categories.first { $0.id == id }.map { (String(localized: "Summe im Zeitraum"), $0.totalCents) }
        case .uncategorized, .documents: return nil
        }
    }

    var lastSyncText: String {
        let latest = connections.compactMap(\.lastSyncUnix).max()
        guard let latest else { return String(localized: "Noch nie aktualisiert") }
        let f = RelativeDateTimeFormatter()
        f.locale = .autoupdatingCurrent
        f.unitsStyle = .full
        let relative = f.localizedString(for: Date(timeIntervalSince1970: TimeInterval(latest)), relativeTo: Date())
        return String(localized: "Aktualisiert \(relative)")
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
    var showsAccountMarkers: Bool {
        switch sidebarSelection {
        case .account: false
        case .group(let id): (groups.first { $0.id == id }?.accounts.count ?? 0) > 1
        case .userGroup(let uuid): userGroupAccountIDs(uuid).count > 1
        default: accountMarkers.count > 1
        }
    }

    func setAccountColor(_ accountId: Int64, hex: String?) {
        guard let vault else { return }
        run { try vault.setAccountColor(accountId: accountId, colorHex: hex) }
        reloadAccounts()
    }

    /// Back to the symbol of the account kind; a bank sync keeps a chosen symbol.
    func resetAccountSymbol(_ accountId: Int64) {
        guard let vault else { return }
        run { try vault.resetAccountSymbol(accountId: accountId) }
        reloadAccounts()
    }

    func setAccountSymbol(_ accountId: Int64, symbol: String) {
        guard let vault else { return }
        run { try vault.setAccountSymbol(accountId: accountId, sfSymbol: symbol) }
        reloadAccounts()
    }

    func rebuildAccountIndex() {
        reloadLoans()
        let accounts = allAccounts
        accountsByID = Dictionary(accounts.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        func base(_ a: Account) -> String { a.name.replacingOccurrences(of: " (MoneyMoney)", with: "") }
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
