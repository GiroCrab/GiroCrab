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
    /** Separate from [purpose] in the core; rules can match it on its own. */
    val bookingText: String = "",
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

enum class RuleField { Counterparty, Purpose, BookingText, Iban, Amount, Any }

/**
 * Match kinds of a rule. [Keyword] is the core's pipe list ("REWE|EDEKA|ALDI"), [Expression] its
 * own search language — the app shows both but only evaluates the kinds the demo path produces.
 */
enum class MatchKind { Contains, Equals, StartsWith, Regex, Keyword, Expression }

data class Rule(
    val id: String,
    val field: RuleField,
    val matchKind: MatchKind,
    val pattern: String,
    val categoryId: String?,
    val tagIds: Set<String> = emptySet(),
    val enabled: Boolean = true,
    /** Range on the signed amount in cents, like the core: expenses are negative. */
    val minCents: Long? = null,
    val maxCents: Long? = null,
    /** The core's `case_sensitive`; its default is case-insensitive. */
    val caseSensitive: Boolean = false,
    /** Empty means every account, as in the core. */
    val accountIds: Set<String> = emptySet(),
) {
    private fun haystacks(b: Booking): List<String> = when (field) {
        RuleField.Counterparty -> listOf(b.counterparty)
        RuleField.Purpose -> listOf(b.purpose)
        RuleField.BookingText -> listOf(b.bookingText)
        RuleField.Amount -> listOf((b.amountCents / 100.0).toString())
        // The core matches counterparty, purpose and booking text separately.
        RuleField.Any -> listOf(b.counterparty, b.purpose, b.bookingText)
        RuleField.Iban -> listOf(b.counterpartyIban.filterNot { it.isWhitespace() })
    }

    fun matches(b: Booking): Boolean {
        val hasRange = minCents != null || maxCents != null
        if (!enabled || (pattern.isBlank() && !hasRange)) return false
        if (accountIds.isNotEmpty() && b.accountId !in accountIds) return false
        // Signed, not |amount|: a core rule with a negative range has to match here too.
        if (minCents != null && b.amountCents < minCents) return false
        if (maxCents != null && b.amountCents > maxCents) return false
        if (pattern.isBlank()) return true
        val fields = haystacks(b)
        return when (matchKind) {
            MatchKind.Contains -> fields.any { it.contains(pattern, ignoreCase = !caseSensitive) }
            MatchKind.Equals -> fields.any { it.equals(pattern, ignoreCase = !caseSensitive) }
            MatchKind.StartsWith -> fields.any { it.startsWith(pattern, ignoreCase = !caseSensitive) }
            MatchKind.Regex -> fields.any {
                runCatching {
                    Regex(pattern, if (caseSensitive) emptySet() else setOf(RegexOption.IGNORE_CASE))
                        .containsMatchIn(it)
                }.getOrDefault(false)
            }
            // One keyword per line or separated by | or ,; empty terms are dropped, as in the core.
            MatchKind.Keyword -> {
                val words = pattern.split('|', ',', '\n', '\r')
                    .map { it.trim() }
                    .filter { it.isNotEmpty() }
                words.isNotEmpty() &&
                    fields.any { f -> words.any { w -> f.contains(w, ignoreCase = !caseSensitive) } }
            }
            // The core's search language (AND/OR/NOT, field prefixes, amount comparisons). The app
            // cannot evaluate it; the editor does not offer it either, so this only guards rules
            // that came from a vault.
            MatchKind.Expression -> false
        }
    }
}

enum class Interval { Weekly, Monthly, Quarterly, Yearly }

data class StandingOrder(
    val id: String,
    val accountId: String,
    val recipient: String,
    val iban: String,
    val amountCents: Long,
    val interval: Interval,
    val nextDate: LocalDate,
    val purpose: String,
)

data class DirectDebitMandate(
    val id: String,
    val accountId: String,
    val creditor: String,
    val creditorId: String,
    val mandateRef: String,
    val signedDate: LocalDate,
    val lastUsed: LocalDate?,
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
