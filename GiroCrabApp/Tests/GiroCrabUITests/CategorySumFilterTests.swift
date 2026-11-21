import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// The sidebar's category sums follow the list filters: period, status, accounts (GitHub #621).
@MainActor
final class CategorySumFilterTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        Self.resetSwitches()
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
    }

    override func tearDown() async throws {
        model = nil
        Self.resetSwitches()
    }

    /// The switch and the pin are saved; the test store lives as long as the process (#1469).
    private static func resetSwitches() {
        AppDefaults.store.removeObject(forKey: AppModel.pinnedCategoryKeysKey)
        AppDefaults.store.removeObject(forKey: AppModel.categorySumsFollowAccountFilterKey)
        AppDefaults.store.removeObject(forKey: AppModel.categorySetsKey)
    }

    /// The category the narrowing is watched on: in a category view only its own row counts by the shown filters.
    private var focus: Int64 = 0

    /// Shows the busiest category: its row is the one the filters set in this view narrow.
    private func focusCategory() throws {
        let busiest = try XCTUnwrap(model.categories.filter { $0.depth == 0 }.max { $0.transactionCount < $1.transactionCount })
        focus = busiest.id
        model.sidebarSelection = .category(focus)
    }
    private var focusCount: Int { Int(model.categories.first { $0.id == focus }?.transactionCount ?? 0) }
    private var focusTotal: Int64 { model.categories.first { $0.id == focus }?.totalCents ?? 0 }

    private var topLevelCount: Int { model.categories.filter { $0.depth == 0 }.reduce(0) { $0 + Int($1.transactionCount) } }

    func testNoFilterHasNoNoteAndNoNarrowing() {
        XCTAssertNil(model.categorySumNote)
        XCTAssertTrue(model.categorySumFilter.accounts.isEmpty)
        XCTAssertNil(model.categorySumFilter.checked)
    }

    func testStatusFilterNarrowsSums() throws {
        try focusCategory()
        let all = focusCount
        XCTAssertGreaterThan(all, 0)
        model.statusFilter = .checked
        let checked = focusCount
        model.statusFilter = .new
        let new = focusCount
        XCTAssertEqual(checked + new, all)
        XCTAssertNotNil(model.categorySumNote)
        model.statusFilter = .all
        XCTAssertEqual(focusCount, all)
        XCTAssertNil(model.categorySumNote)
    }

    func testDirectionFilterNarrowsSums() throws {
        try focusCategory()
        let all = focusTotal
        model.directionFilter = .income
        let income = focusTotal
        XCTAssertNotNil(model.categorySumNote)
        model.directionFilter = .expense
        let expense = focusTotal
        XCTAssertEqual(income + expense, all)
        model.directionFilter = .all
        XCTAssertNil(model.categorySumNote)
    }

    func testAccountFilterNarrowsSumsInCategoryViews() throws {
        try focusCategory()
        let all = focusCount
        let account = try XCTUnwrap(model.allAccounts.first)
        model.categoryAccountFilter = [account.id]
        XCTAssertLessThanOrEqual(focusCount, all)
        XCTAssertEqual(model.categorySumFilter.accounts, [account.id])
        XCTAssertNotNil(model.categorySumNote)
        model.categoryAccountFilter = []
        XCTAssertEqual(focusCount, all)
    }

    /// #357: the amount range narrows the sidebar's category sums like the list.
    func testAmountRangeNarrowsSums() throws {
        try focusCategory()
        let all = focusCount
        XCTAssertGreaterThan(all, 0)
        model.amountFilter = AmountFilter(minCents: 10_001, maxCents: nil)
        let above = focusCount
        model.amountFilter = AmountFilter(minCents: nil, maxCents: 10_000)
        let below = focusCount
        XCTAssertEqual(above + below, all)
        XCTAssertEqual(model.categorySumFilter.amount, AmountFilter(minCents: nil, maxCents: 10_000))
        XCTAssertNotNil(model.categorySumNote)
        model.amountFilter = AmountFilter()
        XCTAssertEqual(focusCount, all)
        XCTAssertNil(model.categorySumNote)
    }

    /// #1469: in „Alle Konten“ the sums ignore every filter of the account views; a category view uses its own.
    func testAccountViewFiltersDoNotReachTheSums() throws {
        model.sidebarSelection = .allAccounts
        let all = topLevelCount
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind != .loan }).id
        model.toggleCategoryAccount(account)
        model.directionFilter = .income
        model.reloadCategories()
        XCTAssertEqual(model.categorySumFilter, .unfiltered)
        XCTAssertEqual(topLevelCount, all)
        XCTAssertNil(model.categorySumNote)

        let category = try XCTUnwrap(model.categories.first { $0.depth == 0 })
        model.sidebarSelection = .category(category.id)
        XCTAssertEqual(model.categorySumFilter, CategorySumFilter(), "the category starts without the account's filters")
        model.toggleCategoryAccount(account)
        XCTAssertEqual(model.categorySumFilter.accounts, [account])
        model.sidebarSelection = .allAccounts
        model.reloadCategories()
        XCTAssertEqual(topLevelCount, all)
    }

    /// What the „Ohne Kategorie“ list shows for the current period and filters (`total_count` of its first page).
    private func uncategorizedListTotal() throws -> Int {
        let vault = try XCTUnwrap(model.vault)
        model.sidebarSelection = .uncategorized
        return Int(try vault.listTransactions(filter: model.currentFilter).totalCount)
    }

    /// #357 / #1439: the badge of „Ohne Kategorie“ counts what the period and the filters leave.
    func testUncategorizedBadgeFollowsTheFilters() throws {
        let vault = try XCTUnwrap(model.vault)
        model.sidebarSelection = .uncategorized
        model.period = .all
        model.reloadCategories()
        let plain = model.uncategorizedCount
        XCTAssertGreaterThan(plain, 0)
        XCTAssertNotNil(model.uncategorizedFilteredCount)

        func expected() throws -> Int {
            Int(try vault.countTransactions(filter: model.uncategorizedCountFilter(model.categorySumFilter)))
        }
        model.amountFilter = AmountFilter(minCents: 5_000, maxCents: nil)
        model.reloadCategories()
        XCTAssertLessThan(model.uncategorizedCount, plain)
        XCTAssertEqual(model.uncategorizedCount, try expected())
        model.amountFilter = AmountFilter()
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount, plain)

        model.directionFilter = .income
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount, try expected())
        model.directionFilter = .expense
        model.reloadCategories()
        let expense = model.uncategorizedCount
        model.directionFilter = .income
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount + expense, plain)
        model.directionFilter = .all

        let account = try XCTUnwrap(model.allAccounts.first { $0.kind != .loan && $0.uncategorizedCount > 0 })
        model.sidebarSelection = .allAccounts
        model.toggleCategoryAccount(account.id)
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount, plain) // the account's filter does not reach the sums
        model.sidebarSelection = .uncategorized
        model.toggleCategoryAccount(account.id) // the category view has its own account filter
        model.reloadCategories() // a synchronous selection change keeps the sums; the interactive one reloads them
        XCTAssertEqual(model.uncategorizedCount, try expected())
        XCTAssertLessThan(model.uncategorizedCount, plain)
    }

    /// #1439: a narrower period narrows the badge, and it equals the list's row count for the same period,
    /// with and without list filters (own transfers, #302, are left out in every branch).
    func testUncategorizedBadgeFollowsThePeriodAndEqualsTheList() throws {
        model.period = .all
        model.reloadCategories()
        let all = model.uncategorizedCount
        XCTAssertGreaterThan(all, 0)
        XCTAssertEqual(all, try uncategorizedListTotal())

        model.period = .days(30)
        model.reloadCategories()
        XCTAssertLessThan(model.uncategorizedCount, all)
        XCTAssertEqual(model.uncategorizedCount, try uncategorizedListTotal())

        // The same number whether or not a list filter is active (it used to jump).
        let unfiltered = model.uncategorizedCount
        model.statusFilter = .all
        model.directionFilter = .expense
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount, try uncategorizedListTotal())
        model.directionFilter = .all
        model.reloadCategories()
        XCTAssertEqual(model.uncategorizedCount, unfiltered)
    }

    /// #1439: the badge is loaded for the period of the selection, so a period change reloads it.
    func testUncategorizedBadgeReloadsWithThePeriod() async throws {
        model.sidebarSelection = .uncategorized
        model.period = .all
        model.reloadCategories()
        let all = model.uncategorizedCount
        model.interactivePeriod = .days(30)
        let want = try XCTUnwrap(model.vault).countTransactions(filter: model.uncategorizedCountFilter(model.categorySumFilter))
        for _ in 0..<300 where model.uncategorizedCount != Int(want) { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.uncategorizedCount, Int(want))
        XCTAssertLessThan(model.uncategorizedCount, all)
    }

    // MARK: - Pin (#1469)

    private var topCategories: [GiroCrabCore.Category] { model.categories.filter { $0.depth == 0 && $0.transactionCount > 0 } }

    /// The filters of the account views and of the category views are independent sets: nothing is taken over.
    func testAccountAndCategoryFiltersAreIndependent() throws {
        let category = try XCTUnwrap(topCategories.first)
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind != .loan }).id
        model.sidebarSelection = .allAccounts
        model.statusFilter = .checked
        model.amountFilter = AmountFilter(minCents: 100, maxCents: nil)
        model.toggleCategoryAccount(account)
        model.sidebarSelection = .category(category.id)
        XCTAssertEqual(model.currentFilterSet, ListFilterSet(), "a category view starts with its own, empty filters")
        model.directionFilter = .income
        model.sidebarSelection = .allAccounts
        XCTAssertEqual(model.statusFilter, .checked)
        XCTAssertEqual(model.directionFilter, .all, "the category's direction does not reach the account")
        XCTAssertEqual(model.amountFilter, AmountFilter(minCents: 100, maxCents: nil))
        XCTAssertEqual(model.categoryAccountFilter, [account])
        // unpinned, the category's filters are global and reset with the switch to an account view
        model.sidebarSelection = .category(category.id)
        XCTAssertEqual(model.currentFilterSet, ListFilterSet())
    }

    /// Without the pin the filters are global: another category shows the same ones, and the sidebar counts every
    /// row by them; an account view resets them.
    func testUnpinnedFiltersAreGlobal() async throws {
        let (a, b) = (try XCTUnwrap(topCategories.first), try XCTUnwrap(topCategories.dropFirst().first))
        let all = Dictionary(uniqueKeysWithValues: model.categories.map { ($0.id, $0.transactionCount) })
        model.sidebarSelection = .category(a.id)
        model.statusFilter = .new
        model.sidebarSelection = .category(b.id)
        XCTAssertEqual(model.statusFilter, .new, "the filters are global")
        XCTAssertFalse(model.shownCategoryPinned)
        try await settleCount(a.id, model.categories.first { $0.id == a.id }?.transactionCount)
        XCTAssertLessThan(try XCTUnwrap(model.categories.first { $0.id == a.id }).transactionCount, try XCTUnwrap(all[a.id]),
                          "every row counts by the global filters")
        XCTAssertLessThan(try XCTUnwrap(model.categories.first { $0.id == b.id }).transactionCount, try XCTUnwrap(all[b.id]))
        // an account view resets them
        model.sidebarSelection = .allAccounts
        XCTAssertEqual(model.globalCategoryFilters, ListFilterSet())
        try await settleCount(a.id, all[a.id])
        model.sidebarSelection = .category(a.id)
        XCTAssertEqual(model.statusFilter, .all)
    }

    /// A pinned category keeps its own filters (saved, also across account views); the others do not follow them,
    /// and its row counts only by them.
    func testPinnedCategoryKeepsItsOwnFilters() async throws {
        let (a, b) = (try XCTUnwrap(topCategories.first), try XCTUnwrap(topCategories.dropFirst().first))
        let all = Dictionary(uniqueKeysWithValues: model.categories.map { ($0.id, $0.transactionCount) })
        model.sidebarSelection = .category(a.id)
        model.statusFilter = .new
        model.toggleCategoryPin()
        XCTAssertTrue(model.shownCategoryPinned)
        model.interactivePeriod = .days(30)
        model.directionFilter = .income

        model.sidebarSelection = .category(b.id)
        XCTAssertEqual(model.statusFilter, .new, "the global filters count here")
        XCTAssertEqual(model.directionFilter, .all)
        XCTAssertEqual(model.period, .all)
        model.directionFilter = .expense
        try await settleCount(b.id, model.categories.first { $0.id == b.id }?.transactionCount)
        // the pinned row counts by its own filters, not by the global ones
        let pinnedRow = try XCTUnwrap(model.categories.first { $0.id == a.id })
        model.sidebarSelection = .category(a.id)
        XCTAssertEqual(model.statusFilter, .new)
        XCTAssertEqual(model.directionFilter, .income)
        XCTAssertEqual(model.period, .days(30))
        XCTAssertEqual(try XCTUnwrap(model.categories.first { $0.id == a.id }).transactionCount, pinnedRow.transactionCount)

        // account views reset the global filters; the pinned category keeps its own, in the sums, too
        model.sidebarSelection = .allAccounts
        XCTAssertEqual(model.globalCategoryFilters, ListFilterSet())
        try await settleCount(b.id, all[b.id])
        XCTAssertNotNil(model.categorySumNote)
        model.sidebarSelection = .category(a.id)
        XCTAssertEqual(model.statusFilter, .new)

        // saved across a restart
        let again = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertTrue(again.pinnedCategoryKeys.contains("c\(a.id)"))
        XCTAssertEqual(again.categorySets["c\(a.id)"]?.direction, .income)
        XCTAssertEqual(again.categorySets["c\(a.id)"]?.period, .days(30))

        // released: the category follows the global filters again
        model.toggleCategoryPin()
        XCTAssertFalse(model.shownCategoryPinned)
        XCTAssertEqual(model.currentFilterSet, ListFilterSet())
        XCTAssertNil(model.categorySets["c\(a.id)"])
    }

    /// The switch applies the account's filters to the sums of every category that is not pinned; a pinned one
    /// keeps counting by its own. It is saved, off by default.
    func testSwitchAppliesTheAccountFiltersExceptToPinnedCategories() async throws {
        let (a, b) = (try XCTUnwrap(topCategories.first), try XCTUnwrap(topCategories.dropFirst().first))
        let all = Dictionary(uniqueKeysWithValues: model.categories.map { ($0.id, $0.transactionCount) })
        model.sidebarSelection = .category(a.id)
        model.toggleCategoryPin() // a counts by its own (empty) filters
        model.sidebarSelection = .allAccounts
        model.statusFilter = .new
        XCTAssertEqual(model.categorySumFilter, .unfiltered, "switch off: nothing")
        try await settleCount(b.id, all[b.id])

        model.categorySumsFollowAccountFilter = true
        XCTAssertEqual(model.categorySumFilter.checked, false)
        let note = try XCTUnwrap(model.categorySumNote)
        XCTAssertTrue(note.contains("Filter des Kontos"))
        XCTAssertTrue(note.contains("Zeitraum: \(model.periodTitle)"))
        XCTAssertEqual(model.categorySumsRequest.perCategory[b.id]?.filter.checked, false, "unpinned: the account's filters")
        XCTAssertEqual(model.categorySumsRequest.perCategory[a.id]?.filter, CategorySumFilter(period: PeriodSetting(.all)),
                       "pinned: its own")
        try await settleCount(a.id, all[a.id])

        // a single account is named without naming it
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind != .loan }).id
        model.sidebarSelection = .account(account)
        XCTAssertEqual(model.categorySumFilter.accounts, [account])
        XCTAssertTrue(try XCTUnwrap(model.categorySumNote).contains("Aktuelles Konto"))

        let again = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertTrue(again.categorySumsFollowAccountFilter)
        model.categorySumsFollowAccountFilter = false
        XCTAssertFalse(AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:])).categorySumsFollowAccountFilter)
    }

    /// A main category's filters also act on its subcategories; a subcategory can be pinned and then counts by
    /// its own filters instead.
    func testMainCategoryFiltersActOnSubcategoriesUntilTheyArePinned() throws {
        let sub = try XCTUnwrap(model.categories.first { $0.parentId != nil })
        let main = try XCTUnwrap(sub.parentId)
        let (mainKey, subKey) = ("c\(main)", "c\(sub.id)")
        model.sidebarSelection = .category(main)
        model.statusFilter = .new
        model.toggleCategoryPin() // the main category's own filters: Status Neu
        model.directionFilter = .income

        model.sidebarSelection = .category(sub.id)
        XCTAssertEqual(model.governingKey(for: subKey), mainKey)
        XCTAssertEqual(model.shownFilterOrigin, .mainCategory)
        XCTAssertEqual(model.statusFilter, .new, "the main category's filters act on the subcategory")
        XCTAssertEqual(model.directionFilter, .income)
        XCTAssertTrue(try XCTUnwrap(model.categorySumNote).contains("Filter der Hauptkategorie"))
        XCTAssertEqual(model.categorySumsRequest.perCategory[sub.id]?.filter.checked, false)
        // a change in the subcategory is a change of the filters that govern it
        model.directionFilter = .expense
        XCTAssertEqual(model.categorySets[mainKey]?.direction, .expense)

        // pinned, the subcategory overrides the main category
        model.toggleCategoryPin()
        model.directionFilter = .all
        model.statusFilter = .checked
        XCTAssertEqual(model.shownFilterOrigin, .own)
        XCTAssertEqual(model.categorySets[mainKey]?.status, .new, "the main category keeps its own")
        model.sidebarSelection = .category(main)
        XCTAssertEqual(model.statusFilter, .new)
        XCTAssertEqual(model.categorySumsRequest.perCategory[sub.id]?.filter.checked, true)

        // released, the subcategory follows the main category again
        model.sidebarSelection = .category(sub.id)
        model.toggleCategoryPin()
        XCTAssertEqual(model.statusFilter, .new)
        XCTAssertEqual(model.governingKey(for: subKey), mainKey)
        XCTAssertTrue(model.pinnedFilterSummary(forKey: mainKey).hasPrefix("Eigene Filter:"))
    }

    /// The note says whose filters count.
    func testNoteNamesWhoseFiltersCount() throws {
        let category = try XCTUnwrap(topCategories.first)
        model.sidebarSelection = .category(category.id)
        model.statusFilter = .new
        XCTAssertTrue(try XCTUnwrap(model.categorySumNote).contains("Filter der Kategorien, global"))
        model.toggleCategoryPin()
        XCTAssertTrue(try XCTUnwrap(model.categorySumNote).contains("Filter dieser Kategorie, individuell"))
    }

    /// The sums of a selection change load in the background: wait for the count of one row.
    private func settleCount(_ id: Int64, _ want: UInt32?, line: UInt = #line) async throws {
        for _ in 0..<300 where model.categories.first(where: { $0.id == id })?.transactionCount != want {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(model.categories.first { $0.id == id }?.transactionCount, want, line: line)
    }

    /// A period alone is a filter, too: the note appears for it.
    func testPeriodAloneGetsANote() throws {
        model.sidebarSelection = .uncategorized
        XCTAssertNil(model.categorySumNote)
        model.interactivePeriod = .days(30)
        let note = try XCTUnwrap(model.categorySumNote)
        XCTAssertTrue(note.contains("Zeitraum: \(model.periodTitle)"))
        XCTAssertTrue(note.contains("Filter der Kategorien, global"))
        // pinned category in an account view without the switch: its period counts
        model.toggleCategoryPin()
        model.sidebarSelection = .allAccounts
        XCTAssertNotNil(model.categorySumNote)
        model.interactivePeriod = .all
        model.sidebarSelection = .uncategorized
        model.interactivePeriod = .all
        model.sidebarSelection = .allAccounts
        XCTAssertNil(model.categorySumNote)
    }

    /// The note names the period besides the criteria, in the category views too; without a filter there is none there.
    func testNoteNamesThePeriodInCategoryViews() throws {
        model.sidebarSelection = .uncategorized
        XCTAssertNil(model.categorySumNote)
        model.statusFilter = .new
        let note = try XCTUnwrap(model.categorySumNote)
        XCTAssertTrue(note.contains("Filter der Kategorien, global"))
        XCTAssertTrue(note.contains("Zeitraum: \(model.periodTitle)"))
    }

    /// Pin off: the selected account is no reason for a note.
    func testNoNoteInAnAccountWithoutPin() throws {
        let account = try XCTUnwrap(model.allAccounts.first { $0.kind != .loan }).id
        model.sidebarSelection = .account(account)
        XCTAssertEqual(model.categorySumFilter, .unfiltered)
        XCTAssertNil(model.categorySumNote)
    }

    /// Tags narrow the sums like the list: every chosen tag, or no tag at all; the badge follows.
    func testTagFilterNarrowsSumsAndBadge() throws {
        let vault = try XCTUnwrap(model.vault)
        model.period = .all
        let tag = try vault.createTag(name: "Summen-Test", colorHex: "").id
        // two bookings of one category carry the tag
        let booked = try vault.listTransactions(filter: model.currentFilter).transactions.filter { $0.categoryId != nil }
        let categoryID = try XCTUnwrap(booked.first?.categoryId)
        let ids = booked.filter { $0.categoryId == categoryID }.prefix(2).map(\.id)
        XCTAssertEqual(ids.count, 2)
        for id in ids { try vault.setBookingTags(bookingId: id, tagIds: [tag]) }
        focus = categoryID
        model.sidebarSelection = .category(categoryID)
        model.reloadCategories()
        let all = focusCount
        XCTAssertGreaterThanOrEqual(all, 2)

        model.tagFilter = BookingTagFilter(tagIDs: [tag])
        model.reloadCategories()
        XCTAssertEqual(model.categorySumFilter.tagIDs, [tag])
        let tagged = focusCount
        XCTAssertGreaterThan(tagged, 0)
        XCTAssertLessThanOrEqual(tagged, all)
        XCTAssertNotNil(model.categorySumNote)

        model.tagFilter = BookingTagFilter(untagged: true)
        model.reloadCategories()
        XCTAssertTrue(model.categorySumFilter.untaggedOnly)
        XCTAssertLessThan(focusCount, all)
        XCTAssertLessThanOrEqual(focusCount + tagged, all)

        // the badge equals the list in the category view for the same tag filter
        model.tagFilter = BookingTagFilter(tagIDs: [tag])
        XCTAssertEqual(try uncategorizedListTotal(), Int(try vault.countTransactions(filter: model.uncategorizedCountFilter(model.categorySumFilter))))
        // the account views ignore the tags of the categories
        model.sidebarSelection = .allAccounts
        XCTAssertTrue(model.categorySumFilter.tagIDs.isEmpty)
    }
}
