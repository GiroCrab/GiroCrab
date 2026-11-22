package de.girocrab.app.model

import java.time.LocalDate
import uniffi.girocrab_core.Account as CoreAccount
import uniffi.girocrab_core.AccountGroup as CoreGroup
import uniffi.girocrab_core.AccountKind as CoreAccountKind
import uniffi.girocrab_core.Booking as CoreBooking
import uniffi.girocrab_core.BookingDirection
import uniffi.girocrab_core.BookingFilter as CoreFilter
import uniffi.girocrab_core.BookingPage as CorePage
import uniffi.girocrab_core.BookingScope
import uniffi.girocrab_core.Category as CoreCategory
import uniffi.girocrab_core.CategorySource as CoreCategorySource
import uniffi.girocrab_core.ConnectorKind as CoreConnector
import uniffi.girocrab_core.Tag as CoreTag

/**
 * Translates the core's own types into the app's [VaultSnapshot] (#1429, 3.1).
 *
 * Only names and units are translated here: categorisation, rule application, balances and sums
 * stay in Rust. Nothing in this file decides what a booking is — it copies what the core says.
 *
 * The pages are read in one go for now; cursor paging over `listTransactions` is 3.3, and the
 * analysis sums from the core's balance history are 3.4. Until then this mapper must not grow a
 * second opinion about money.
 */
object VaultSnapshotMapper {

    /**
     * One page, not the whole list: 3.3 replaces this with a cursor. A vault with more bookings
     * than this shows the newest ones and says so — silently truncating without a word would be
     * worse than a visible limit.
     */
    const val PAGE_SIZE: UInt = 1000u

    /** Filter for the first page: every account, every date, both directions, no day split. */
    fun firstPageFilter(): CoreFilter = CoreFilter(
        accountIds = emptyList(),
        categoryId = null,
        includeSubcategories = true,
        uncategorizedOnly = false,
        search = null,
        dateFrom = null,
        dateTo = null,
        limit = PAGE_SIZE,
        offset = 0u,
        scope = BookingScope.ALL,
        merchantKey = null,
        direction = BookingDirection.ALL,
        checked = null,
        wholeDays = true,
    )

    fun map(
        groups: List<CoreGroup>,
        accounts: List<CoreAccount>,
        page: CorePage,
        categories: List<CoreCategory>,
        tags: List<CoreTag>,
    ): VaultSnapshot = VaultSnapshot(
        groups = groups.map { group(it, accounts) },
        accounts = accounts.map(::account),
        bookings = page.transactions.map(::booking),
        categories = categories.map(::category),
        tags = tags.map(::tag),
    )

    /** True when the core held back bookings: the list is a page, not the vault. */
    fun isTruncated(page: CorePage): Boolean = page.totalCount.toLong() > page.transactions.size

    private fun group(group: CoreGroup, accounts: List<CoreAccount>): AccountGroup {
        // The core's groups carry no connector; the kind is taken from their accounts so that the
        // icon in the list matches what the accounts below it really are.
        val connector = accounts
            .filter { it.groupId == group.id }
            .mapNotNull { it.connector }
            .map(::connector)
            .distinct()
            .singleOrNull()
            ?: ConnectorKind.Manual
        return AccountGroup(
            id = group.id.toString(),
            name = group.name,
            connector = connector,
        )
    }

    private fun account(row: CoreAccount): Account = Account(
        id = row.id.toString(),
        groupId = row.groupId.toString(),
        bankName = row.bankName,
        name = row.name,
        iban = row.iban.ifBlank { null },
        kind = accountKind(row.kind),
        balanceCents = row.balanceCents,
        currency = row.currency,
        holder = row.owner,
    )

    private fun booking(row: CoreBooking): Booking = Booking(
        id = row.id.toString(),
        accountId = row.accountId.toString(),
        bookingDate = date(row.bookingDate),
        valueDate = date(row.valueDate.ifBlank { row.bookingDate }),
        amountCents = row.amountCents,
        counterparty = row.counterparty,
        purpose = row.purpose,
        categoryId = row.categoryId?.toString(),
        categoryByRule = row.categorySource == CoreCategorySource.RULE,
        currency = row.currency,
        checked = row.checked,
        counterpartyIban = row.counterpartyIban,
    )

    private fun category(row: CoreCategory): Category = Category(
        id = row.id.toString(),
        name = row.name,
        parentId = row.parentId?.toString(),
        colorHex = color(row.colorHex, 0xFF607D8B),
        icon = row.sfSymbol.ifBlank { "label" },
        monthlyBudgetCents = row.budgetCents,
    )

    private fun tag(row: CoreTag): Tag = Tag(
        id = row.id.toString(),
        name = row.name,
        colorHex = color(row.colorHex, 0xFF1976D2),
    )

    private fun date(raw: String): LocalDate = LocalDate.parse(raw)

    /** The core sends `#RRGGBB`; the app models carry ARGB as a Long. */
    private fun color(hex: String, fallback: Long): Long {
        val digits = hex.removePrefix("#")
        if (digits.length != 6) return fallback
        val rgb = digits.toLongOrNull(16) ?: return fallback
        return 0xFF000000L or rgb
    }

    private fun accountKind(kind: CoreAccountKind): AccountKind = when (kind) {
        CoreAccountKind.CHECKING -> AccountKind.Giro
        CoreAccountKind.SAVINGS -> AccountKind.Savings
        // Festgeld (#1002) gibt es in der App-Kind-Liste nicht; es zählt zu den Sparformen.
        CoreAccountKind.TIME_DEPOSIT -> AccountKind.Savings
        CoreAccountKind.CREDIT_CARD -> AccountKind.CreditCard
        CoreAccountKind.BROKERAGE -> AccountKind.Depot
        CoreAccountKind.LOAN -> AccountKind.Loan
        CoreAccountKind.CASH -> AccountKind.Cash
        CoreAccountKind.PAYMENT_SERVICE -> AccountKind.Wallet
    }

    private fun connector(kind: CoreConnector): ConnectorKind = when (kind) {
        CoreConnector.PAY_PAL -> ConnectorKind.PayPal
        CoreConnector.TRADE_REPUBLIC -> ConnectorKind.TradeRepublic
        CoreConnector.COMDIRECT -> ConnectorKind.Comdirect
        CoreConnector.VOLKSBANK_KASSEL_GOETTINGEN,
        CoreConnector.FIN_TS_GENERIC,
        CoreConnector.DIREKT1822,
        -> ConnectorKind.FinTS
        else -> ConnectorKind.EnableBanking
    }
}
