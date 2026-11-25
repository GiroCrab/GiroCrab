package de.girocrab.app.model

import java.time.LocalDate

/** Domain models mirroring `girocrab-core` (models.rs). All money values are cents (Long). */

enum class ConnectorKind { FinTS, Comdirect, TradeRepublic, EnableBanking, PayPal, Import, Manual }

enum class AccountKind { Giro, Savings, CreditCard, Depot, Loan, Wallet, Cash }

data class AccountGroup(
    val id: String,
    val name: String,
    val connector: ConnectorKind,
    val colorHex: Long = 0xFF2E7D32,
)

data class Account(
    val id: String,
    val groupId: String,
    val bankName: String,
    val name: String,
    val iban: String?,
    val kind: AccountKind,
    val balanceCents: Long,
    val currency: String = "EUR",
    val hidden: Boolean = false,
    /** The core's `inTotal`: false = the account is excluded from overall sums. */
    val includeInTotal: Boolean = true,
    /** Account holder (#587). */
    val holder: String = "",
)

data class SplitPart(
    val categoryId: String?,
    val amountCents: Long,
    val note: String = "",
)

data class Booking(
    val id: String,
    val accountId: String,
    val bookingDate: LocalDate,
    val valueDate: LocalDate = bookingDate,
    val amountCents: Long,
    val counterparty: String,
    val purpose: String,
    val reference: String? = null,
    val categoryId: String? = null,
    val categoryByRule: Boolean = false,
    val tagIds: Set<String> = emptySet(),
    val note: String = "",
    val splits: List<SplitPart> = emptyList(),
    val currency: String = "EUR",
    val checked: Boolean = true,
    val documentCount: Int = 0,
    val isTransfer: Boolean = false,
    val transferReason: String = "",
    val counterpartyIban: String = "",
) {
    val isIncome get() = amountCents > 0
}

data class Category(
    val id: String,
    val name: String,
    val parentId: String? = null,
    val colorHex: Long = 0xFF607D8B,
    val icon: String = "label",
    val monthlyBudgetCents: Long? = null,
)

data class Tag(val id: String, val name: String, val colorHex: Long = 0xFF1976D2)

enum class RuleField { Counterparty, Purpose, Iban, Amount, Any }

enum class MatchKind { Contains, Equals, StartsWith, Regex }

data class Rule(
    val id: String,
    val field: RuleField,
    val matchKind: MatchKind,
    val pattern: String,
    val categoryId: String?,
    val tagIds: Set<String> = emptySet(),
    val enabled: Boolean = true,
    /** Optional range on |amount| in cents (#759). */
    val minCents: Long? = null,
    val maxCents: Long? = null,
) {
    fun matches(b: Booking): Boolean {
        val hasRange = minCents != null || maxCents != null
        if (!enabled || (pattern.isBlank() && !hasRange)) return false
        val abs = kotlin.math.abs(b.amountCents)
        if (minCents != null && abs < minCents) return false
        if (maxCents != null && abs > maxCents) return false
        if (pattern.isBlank()) return true
        val haystack = when (field) {
            RuleField.Counterparty -> b.counterparty
            RuleField.Purpose -> b.purpose
            RuleField.Iban -> b.reference.orEmpty()
            RuleField.Amount -> (b.amountCents / 100.0).toString()
            RuleField.Any -> "${b.counterparty} ${b.purpose}"
        }
        return when (matchKind) {
            MatchKind.Contains -> haystack.contains(pattern, ignoreCase = true)
            MatchKind.Equals -> haystack.equals(pattern, ignoreCase = true)
            MatchKind.StartsWith -> haystack.startsWith(pattern, ignoreCase = true)
            MatchKind.Regex -> runCatching { Regex(pattern, RegexOption.IGNORE_CASE).containsMatchIn(haystack) }
                .getOrDefault(false)
        }
    }
}

/**
 * A standing order as the core stores it. There is no "next date" field in the core: the dates are
 * [firstDate], [lastDate], [execDay] plus [unit]/[every]. Everything shown as the next run is
 * derived by [nextRun]/[after] and never stored, so a stale value cannot survive a bank fetch.
 */
data class StandingOrder(
    val id: String,
    val accountId: String,
    val recipient: String,
    val iban: String,
    val amountCents: Long,
    val purpose: String,
    /** `W` = weeks, `M` = months — the core's `time_unit`. */
    val unit: String,
    /** How many [unit]s lie between two runs (1 = monthly or weekly, 3 = quarterly). */
    val every: Int,
    /** Day of the month the bank executes on; 0 = the day of [firstDate]. */
    val execDay: Int,
    val firstDate: LocalDate,
    /** Last execution the bank reported; `null` = open-ended. */
    val lastDate: LocalDate?,
) {
    /** "wöchentlich", "monatlich", "alle 2 Monate", … */
    val rhythm: String
        get() = when {
            unit == "W" && every == 1 -> "wöchentlich"
            unit == "W" -> "alle $every Wochen"
            every == 1 -> "monatlich"
            every == 3 -> "vierteljährlich"
            every == 12 -> "jährlich"
            else -> "alle $every Monate"
        }

    /** First run on or after [today]; `null` once [lastDate] has passed. */
    fun nextRun(today: LocalDate): LocalDate? = after(today.minusDays(1))

    /** First run strictly after [date]; `null` at the end of the order. */
    fun after(date: LocalDate): LocalDate? {
        if (firstDate > date) return firstDate.takeIf { lastDate == null || it <= lastDate }
        var k = 1
        while (k <= 1200) {
            val d = occurrence(k) ?: return null
            if (d > date) return d
            k++
        }
        return null
    }

    /**
     * The k-th run, counted from [firstDate] — not iterated, so a clamped month end (31 → 28/30)
     * cannot drift into every following month. [execDay] then sets the day within that month.
     */
    private fun occurrence(k: Int): LocalDate? {
        val base = if (unit == "W") {
            firstDate.plusWeeks(every.toLong() * k)
        } else {
            val m = firstDate.plusMonths(every.toLong() * k)
            if (execDay in 1..31) m.withDayOfMonth(minOf(execDay, m.lengthOfMonth())) else m
        }
        return if (lastDate != null && base > lastDate) null else base
    }
}

/** SEPA scheme of a mandate, as the core stores it (`MandateScheme`). */
enum class MandateScheme(val label: String) { Core("CORE"), Cor1("COR1"), B2b("B2B") }

/**
 * A SEPA direct-debit mandate. The fields are the core's `Mandate`: the debtor is the counterpart
 * the money is collected from, the creditor id is the account's own and lives in the collection
 * dialog, not on the mandate. Category `0`/empty has no meaning here — nothing is invented.
 */
data class DirectDebitMandate(
    val id: String,
    val accountId: String,
    val debtorName: String,
    val debtorIban: String,
    val reference: String,
    val signedDate: LocalDate,
    val scheme: MandateScheme,
    val oneOff: Boolean,
    val defaultAmountCents: Long,
    val defaultPurpose: String,
    val note: String,
    val active: Boolean,
    val lastCollected: LocalDate?,
    /** `FRST`, `RCUR` or `OOFF` — what the next collection gets. */
    val nextSequence: String,
)

data class Holding(
    val id: String,
    val accountId: String,
    val name: String,
    val isin: String,
    val quantity: Double,
    val priceCents: Long,
    /** Total purchase cost in EUR cents. */
    val costBasisCents: Long,
    val wkn: String = "",
    /**
     * Currency of [priceCents] and [costBasisCents]. The core reports holdings in the currency of
     * their account, never in the position's own one (#625) — so a EUR amount must not be labelled
     * with the share's native code.
     */
    val quoteCurrency: String = "EUR",
    /** Native currency of the position; "" = same as [quoteCurrency]. */
    val currency: String = "",
    /** Price per unit in [currency]; null when the core has no native figure. */
    val nativePriceCents: Long? = null,
    /** Purchase price per unit in [currency]; null when the core has no native figure. */
    val nativePurchasePriceCents: Long? = null,
    /** Factor from the position's own currency to [quoteCurrency]. */
    val fxRateToEur: Double = 1.0,
) {
    /** Current value in cents of [quoteCurrency]. */
    // Rounded, not truncated: a derived rate (see the mapper) lands on x.9999… often enough that
    // truncating would lose a cent per position (#1429).
    val valueCents get() = kotlin.math.round(quantity * priceCents * fxRateToEur).toLong()
    val gainCents get() = valueCents - costBasisCents
    val gainPct get() = if (costBasisCents > 0) gainCents * 100.0 / costBasisCents else 0.0
    /** Currency to print the native figures in; the account currency when the core has none. */
    val nativeCurrency get() = currency.ifEmpty { quoteCurrency }
}

data class ManualLoan(
    val id: String,
    val name: String,
    val principalCents: Long,
    val remainingCents: Long,
    val interestRatePct: Double,
    val monthlyPaymentCents: Long,
    val startDate: LocalDate,
)

/**
 * The currency most of the money sits in — the one the app adds up in.
 *
 * Account balances arrive in their own currencies and the vault carries no exchange rates for
 * whole accounts, so a total across currencies would be a number without a unit. Everything that
 * sums money uses this currency and reports the rest as not converted instead.
 */
fun VaultSnapshot.primaryCurrency(): String =
    accounts
        .groupBy { it.currency.ifBlank { "EUR" } }
        .mapValues { (_, rows) -> rows.sumOf { kotlin.math.abs(it.balanceCents) } }
        .maxByOrNull { it.value }
        ?.key
        ?: "EUR"

data class TangibleAsset(
    val id: String,
    val name: String,
    val valueCents: Long,
    val acquiredDate: LocalDate,
    val note: String = "",
    /** The account the asset sits in — its balance already contains the value. */
    val accountId: String = "",
)

data class Announcement(
    val id: String,
    val title: String,
    val body: String,
    val date: LocalDate,
    val url: String? = null,
)

/** What the booking list / dashboard is scoped to (sidebar selection). */
sealed interface Scope {
    data object All : Scope
    data class Group(val groupId: String) : Scope
    data class OneAccount(val accountId: String) : Scope
}

enum class Direction { Any, Income, Expense }

data class BookingFilter(
    val direction: Direction = Direction.Any,
    val minCents: Long? = null,
    val maxCents: Long? = null, // compared against |amount|
    val categoryId: String? = null,
    val tagId: String? = null,
    val uncategorizedOnly: Boolean = false,
    val from: LocalDate? = null,
    val to: LocalDate? = null,
) {
    val isActive get() = this != BookingFilter()
}

/** Everything a vault (demo or own) provides. */
data class VaultSnapshot(
    val groups: List<AccountGroup> = emptyList(),
    /** How many bookings the core holds in total; more than [bookings] means a truncated page. */
    val bookingsTotal: Int = 0,
    val accounts: List<Account> = emptyList(),
    val bookings: List<Booking> = emptyList(),
    val categories: List<Category> = emptyList(),
    val tags: List<Tag> = emptyList(),
    val rules: List<Rule> = emptyList(),
    val standingOrders: List<StandingOrder> = emptyList(),
    val mandates: List<DirectDebitMandate> = emptyList(),
    val holdings: List<Holding> = emptyList(),
    val loans: List<ManualLoan> = emptyList(),
    val assets: List<TangibleAsset> = emptyList(),
    val announcements: List<Announcement> = emptyList(),
)
