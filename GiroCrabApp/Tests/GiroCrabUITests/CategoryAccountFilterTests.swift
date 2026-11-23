// test: serial-defaults (the group tests switch the sidebar view, a stored choice)
import XCTest
import GiroCrabCore
@testable import GiroCrabUI

/// Category views can be narrowed to accounts (GitHub #406).
@MainActor
final class CategoryAccountFilterTests: XCTestCase {
    private var model: AppModel!

    override func setUp() async throws {
        model = AppModel(options: LaunchOptions(arguments: ["--demo-seed"], environment: [:]))
        XCTAssertEqual(model.vaultState, .open)
        model.period = .all
    }

    override func tearDown() async throws {
        AppDefaults.store.removeObject(forKey: "sidebar.accountView")
        model = nil
    }

    /// A category and an account that has bookings in it, next to bookings of other accounts.
    private func mixedCategory() throws -> (category: Int64, account: Int64) {
        let vault = try XCTUnwrap(model.vault)
        for c in model.categories {
            model.sidebarSelection = .category(c.id)
            let accounts = Set(try vault.listTransactions(filter: model.currentFilter).transactions.map(\.accountId))
            if accounts.count > 1, let first = accounts.min() { return (c.id, first) }
        }
        throw XCTSkip("The demo vault has no category with bookings of several accounts")
    }

    func testCategoryListFollowsTheChosenAccounts() throws {
        let (category, account) = try mixedCategory()
        model.sidebarSelection = .category(category)
        let all = model.bookings.count
        XCTAssertNotNil(model.headerBalance)

        model.toggleCategoryAccount(account)
        XCTAssertEqual(model.currentFilter.accountIds, [account])
        XCTAssertEqual(model.currentFilter.categoryId, category)
        XCTAssertFalse(model.bookings.isEmpty)
        XCTAssertLessThan(model.bookings.count, all)
        XCTAssertTrue(model.bookings.allSatisfy { $0.accountId == account })
        // the category total covers all accounts
        XCTAssertNil(model.headerBalance)

        // stays for the other category views …
        model.sidebarSelection = .uncategorized
        XCTAssertEqual(model.currentFilter.accountIds, [account])
        XCTAssertTrue(model.currentFilter.uncategorizedOnly)
        // … and since #357 also narrows „Alle Konten“, but never a single account
        model.sidebarSelection = .allAccounts
        XCTAssertEqual(model.currentFilter.accountIds, [account])
        model.sidebarSelection = .account(account)
        XCTAssertEqual(model.currentFilter.accountIds, [account])
        let other = try XCTUnwrap(model.allAccounts.first { $0.id != account })
        model.sidebarSelection = .account(other.id)
        XCTAssertEqual(model.currentFilter.accountIds, [other.id])

        model.sidebarSelection = .category(category)
        model.toggleCategoryAccount(account)
        XCTAssertEqual(model.currentFilter.accountIds, [])
        XCTAssertEqual(model.bookings.count, all)
    }

    func testAllAccountsChosenOrUnknownAccountsMeanNoFilter() throws {
        model.sidebarSelection = .uncategorized
        for a in model.allAccounts { model.toggleCategoryAccount(a.id) }
        XCTAssertTrue(model.categoryAccountFilter.isEmpty)
        XCTAssertEqual(model.categoryAccountFilterTitle, String(localized: "Konten: Alle"))

        // an account that no longer exists (deleted, other vault)
        model.categoryAccountFilter = [-42]
        XCTAssertEqual(model.currentFilter.accountIds, [])
    }

    /// #357: a whole group of the sidebar's view is chosen at once, and chosen again removed.
    func testAWholeGroupIsChosenAtOnce() throws {
        model.sidebarSelection = .uncategorized
        model.accountView = .byKind
        let groups = model.accountFilterGroups
        guard groups.count > 1, let group = groups.first(where: { $0.accounts.count > 1 }) else {
            throw XCTSkip("The demo vault has no account type with several accounts")
        }
        // every offered account is in exactly one group, in sidebar order
        XCTAssertEqual(Set(groups.flatMap(\.accountIDs)), Set(model.accountFilterCandidates.map(\.id)))
        XCTAssertEqual(groups.flatMap(\.accountIDs).count, model.accountFilterCandidates.count)

        XCTAssertFalse(model.isCategoryGroupChosen(group), "„Alle Konten“ chooses no group on its own")
        model.toggleCategoryGroup(group)
        XCTAssertEqual(Set(model.categoryAccountIDs), Set(group.accountIDs))
        XCTAssertTrue(model.isCategoryGroupChosen(group))
        XCTAssertEqual(Set(model.currentFilter.accountIds), Set(group.accountIDs))
        XCTAssertEqual(model.categoryAccountFilterTitle,
                       String(localized: "Konten: \(group.accounts.count) von \(model.accountFilterCandidates.count)"))

        // one account off: the group is no longer fully chosen; its entry adds the account back
        model.toggleCategoryAccount(group.accountIDs[0])
        XCTAssertFalse(model.isCategoryGroupChosen(group))
        model.toggleCategoryGroup(group)
        XCTAssertTrue(model.isCategoryGroupChosen(group))

        // chosen again: removed, nothing left = no filter
        model.toggleCategoryGroup(group)
        XCTAssertTrue(model.categoryAccountFilter.isEmpty)

        // all groups chosen one by one = all accounts = no filter
        for g in groups { model.toggleCategoryGroup(g) }
        XCTAssertTrue(model.categoryAccountFilter.isEmpty)
        XCTAssertEqual(model.categoryAccountFilterTitle, String(localized: "Konten: Alle"))
    }

    /// The menu follows the sidebar's view: by bank the groups are the banks.
    func testTheGroupsFollowTheSidebarView() throws {
        model.sidebarSelection = .allAccounts
        model.accountView = .byBank
        XCTAssertEqual(model.accountFilterGroups.map(\.name), model.bankGroups
            .filter { !$0.accounts.isEmpty }.map(\.name))
        model.accountView = .byKind
        XCTAssertEqual(model.accountFilterGroups.map(\.name), model.groups
            .filter { !$0.accounts.isEmpty }.map(\.name))
    }

    /// A sidebar group, bank or user group already limits the accounts: the menu stays flat.
    func testASelectedSidebarGroupKeepsTheMenuFlat() throws {
        model.accountView = .byBank
        guard let kind = model.groups.first(where: { !$0.accounts.isEmpty }),
              let bank = model.bankGroups.first(where: { !$0.accounts.isEmpty }) else {
            throw XCTSkip("The demo vault has no accounts")
        }
        model.sidebarSelection = .group(kind.id)
        XCTAssertTrue(model.accountFilterGroups.isEmpty)
        model.sidebarSelection = .bank(bank.id)
        XCTAssertTrue(model.accountFilterGroups.isEmpty)
        model.sidebarSelection = .allAccounts
        XCTAssertFalse(model.accountFilterGroups.isEmpty)
    }

    /// A group from a menu built before a reload may name accounts the filter no longer
    /// offers; they must not count, or "every account chosen" would clear the filter.
    func testStaleGroupAccountsDoNotClearTheFilter() throws {
        model.sidebarSelection = .uncategorized
        let candidates = model.accountFilterCandidates
        guard candidates.count > 2 else { throw XCTSkip("The demo vault has too few accounts") }
        for a in candidates.dropFirst() { model.toggleCategoryAccount(a.id) }
        let chosen = Set(model.categoryAccountIDs)
        XCTAssertEqual(chosen.count, candidates.count - 1)

        var ghost = candidates[0]
        ghost.id = -4242
        model.toggleCategoryGroup(AccountFilterGroup(id: "stale", name: "Alt", symbol: "folder", accounts: [ghost]))
        XCTAssertEqual(Set(model.categoryAccountIDs), chosen, "the stale id neither clears nor changes the filter")
    }
}
