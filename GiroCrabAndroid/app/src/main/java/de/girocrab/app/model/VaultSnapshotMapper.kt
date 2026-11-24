package de.girocrab.app.model

import java.time.LocalDate
import uniffi.girocrab_core.Account as CoreAccount
import uniffi.girocrab_core.AssetItem as CoreAssetItem
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
import uniffi.girocrab_core.FlowKind as CoreFlowKind
import uniffi.girocrab_core.Holding as CoreHolding
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
     * Rows per query, not the size of the list: [BookingsPaging] reads page after page until the
     * core's own count is covered. The size stays fixed so that memory and query cost per step
     * are predictable; a vault above [BookingsPaging.MAX_PAGES] pages says what is missing
     * instead of truncating silently.
     */
    const val PAGE_SIZE: UInt = 1000u

    /**
     * Filter for one page: every account, every date, both directions, no day split.
     * [offset] is the row offset [BookingsPaging] asks for; see there for the `wholeDays` caveat.
     */
    fun firstPageFilter(offset: Int = 0): CoreFilter = CoreFilter(
        accountIds = emptyList(),
        categoryId = null,
        includeSubcategories = true,
        uncategorizedOnly = false,
        search = null,
        dateFrom = null,
        dateTo = null,
        limit = PAGE_SIZE,
        offset = offset.toUInt(),
        scope = BookingScope.ALL,
        merchantKey = null,
        direction = BookingDirection.ALL,
        checked = null,
        // Capped, not whole days: the core may extend a page to complete the last day, which would
        // hand back more rows than PAGE_SIZE — and the truncation notice would stay silent.
        wholeDays = false,
    )

    /**
     * Single-page call. Kept so callers that hold one `BookingPage` (tests, spot reads) do not
     * have to unwrap it; the app itself goes through the list variant below.
     */
    fun map(
        groups: List<CoreGroup>,
        accounts: List<CoreAccount>,
        page: CorePage,
        categories: List<CoreCategory>,
        tags: List<CoreTag>,
        holdingsByAccount: Map<Long, List<CoreHolding>>,
        assets: List<CoreAssetItem>,
    ): VaultSnapshot = map(
        groups = groups,
        accounts = accounts,
        bookings = page.transactions,
        bookingsTotal = page.totalCount.toInt(),
        categories = categories,
        tags = tags,
        holdingsByAccount = holdingsByAccount,
        assets = assets,
    )

    fun map(
        groups: List<CoreGroup>,
        accounts: List<CoreAccount>,
        bookings: List<CoreBooking>,
        bookingsTotal: Int,
        categories: List<CoreCategory>,
        tags: List<CoreTag>,
        holdingsByAccount: Map<Long, List<CoreHolding>>,
        assets: List<CoreAssetItem>,
    ): VaultSnapshot {
        // Holdings come back in the currency of their account; that is the label for their prices.
        val quoteCurrencies = accounts.associate { it.id to it.currency.ifBlank { "EUR" } }
        return VaultSnapshot(
            groups = groups.map { group(it, accounts) },
            bookingsTotal = bookingsTotal,
            accounts = accounts.map(::account),
            bookings = bookings.map(::booking),
            categories = categories.map(::category),
            tags = tags.map(::tag),
            holdings = holdingsByAccount.flatMap { (accountId, rows) ->
                val quote = quoteCurrencies[accountId] ?: "EUR"
                rows.map { holding(accountId, it, quote) }
            },
            assets = assets.map(::asset),
        )
    }

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
        includeInTotal = row.inTotal,
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
        // Existing data has to arrive too, or a search by note finds nothing on this device.
        note = row.note,
        tagIds = row.tags.map { it.id.toString() }.toSet(),
        documentCount = row.documentCount.toInt(),
        isTransfer = row.flow == CoreFlowKind.TRANSFER,
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

    /**
     * The core keeps the EUR value of a holding; the app recomputes it as
     * `quantity * priceCents * fxRateToEur`. Instead of assuming a rate, the rate is derived, so the
     * app's own arithmetic lands on the core's value to the cent.
     */
    private fun holding(accountId: Long, row: CoreHolding, quoteCurrency: String): Holding {
        val costBasis = row.profitLossPurchaseCents?.let { row.valueCents - it } ?: row.valueCents
        val fx = if (row.quantity > 0.0 && row.currentPriceCents > 0L) {
            row.valueCents.toDouble() / (row.quantity * row.currentPriceCents)
        } else {
            1.0
        }
        return Holding(
            id = "$accountId/${row.isin.ifBlank { row.wkn.ifBlank { row.name } }}",
            accountId = accountId.toString(),
            name = row.name,
            isin = row.isin,
            quantity = row.quantity,
            priceCents = row.currentPriceCents,
            costBasisCents = costBasis,
            wkn = row.wkn,
            quoteCurrency = quoteCurrency,
            // Native code as the core reports it: "" means "the account's currency".
            currency = row.currency,
            nativePriceCents = row.nativePriceCents,
            nativePurchasePriceCents = row.nativePurchasePriceCents,
            fxRateToEur = fx,
        )
    }

    private fun asset(row: CoreAssetItem): TangibleAsset = TangibleAsset(
        id = row.id.toString(),
        accountId = row.accountId.toString(),
        name = row.setup.name,
        valueCents = row.holding.valueCents,
        acquiredDate = date(row.setup.purchaseDate),
        note = row.setup.note,
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
        // Festgeld (#1002) und Bausparen gibt es in der App-Kind-Liste nicht; beide zählen zu den
        // Sparformen.
        CoreAccountKind.TIME_DEPOSIT -> AccountKind.Savings
        CoreAccountKind.BUILDING_SAVINGS -> AccountKind.Savings
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
