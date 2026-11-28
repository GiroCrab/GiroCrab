package de.girocrab.app.model

import java.time.LocalDate
import java.time.temporal.ChronoUnit

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
 * Match kinds of a rule. [Keyword] is the core's keyword list ("REWE|EDEKA|ALDI", also comma or one
 * per line), [Expression] its own search language — the app evaluates every kind except
 * [Expression], which only the core can evaluate.
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

    /**
     * No pattern at all. A regex keeps its whitespace like the core (`" "` matches a literal
     * space); only the string match kinds treat a blank pattern as missing.
     */
    private val hasNoPattern: Boolean
        get() = if (matchKind == MatchKind.Regex) pattern.isEmpty() else pattern.isBlank()

    /**
     * The pattern compiled once per rule. The core uses Rust's regex syntax; its named groups
     * `(?P<name>…)` are written `(?<name>…)` in Java. Null when Java still rejects the pattern.
     */
    private val compiledRegex: kotlin.text.Regex? by lazy {
        if (matchKind != MatchKind.Regex || pattern.isEmpty()) return@lazy null
        runCatching {
            Regex(
                pattern.replace("(?P<", "(?<"),
                if (caseSensitive) emptySet() else setOf(RegexOption.IGNORE_CASE),
            )
        }.getOrNull()
    }

    /**
     * False when the app cannot evaluate this rule (an [MatchKind.Expression] or a regex Java does
     * not accept). Its non-matches here say nothing about the core, so the UI has to say so.
     */
    val evaluableHere: Boolean
        get() = when (matchKind) {
            MatchKind.Expression -> false
            MatchKind.Regex -> pattern.isEmpty() || compiledRegex != null
            else -> true
        }

    fun matches(b: Booking): Boolean {
        val hasRange = minCents != null || maxCents != null
        if (!enabled || (hasNoPattern && !hasRange)) return false
        if (accountIds.isNotEmpty() && b.accountId !in accountIds) return false
        // Signed, not |amount|: a core rule with a negative range has to match here too.
        if (minCents != null && b.amountCents < minCents) return false
        if (maxCents != null && b.amountCents > maxCents) return false
        // An empty keyword list never matches in the core ("$^"), not even with an amount range.
        if (hasNoPattern) return matchKind != MatchKind.Keyword && matchKind != MatchKind.Expression
        val fields = haystacks(b)
        return when (matchKind) {
            MatchKind.Contains -> fields.any { it.contains(pattern, ignoreCase = !caseSensitive) }
            MatchKind.Equals -> fields.any { it.equals(pattern, ignoreCase = !caseSensitive) }
            MatchKind.StartsWith -> fields.any { it.startsWith(pattern, ignoreCase = !caseSensitive) }
            // Unsupported patterns are reported through [evaluableHere], not hidden here.
            MatchKind.Regex -> compiledRegex?.let { r -> fields.any { r.containsMatchIn(it) } } ?: false
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
    /** Day of the month the bank executes on; 0 is clamped to day 1, like the core. */
    val execDay: Int,
    val firstDate: LocalDate,
    /** Last execution the bank reported; `null` = open-ended. */
    val lastDate: LocalDate?,
) {
    /** "wöchentlich", "monatlich", "alle 2 Monate", … */
    val rhythm: String
        get() = when {
            unit == "W" && step == 1 -> "wöchentlich"
            unit == "W" -> "alle $step Wochen"
            step == 1 -> "monatlich"
            step == 3 -> "vierteljährlich"
            step == 12 -> "jährlich"
            else -> "alle $step Monate"
        }

    /** First run on or after [today]; `null` once [lastDate] has passed. */
    fun nextRun(today: LocalDate): LocalDate? = after(today.minusDays(1))

    /** First run strictly after [date]; `null` at the end of the order. */
    fun after(date: LocalDate): LocalDate? {
        // Start at the step just before [date], like the core: an old open order must not
        // fall off the 1000-step cap.
        val k0: Long = if (unit == "W") {
            val monday = firstDate.minusDays(firstDate.dayOfWeek.value - 1L)
            maxOf(0L, ChronoUnit.DAYS.between(monday, date) / (7L * step) - 1)
        } else {
            val firstMonths = firstDate.year * 12 + firstDate.monthValue - 1
            val fromMonths = date.year * 12 + date.monthValue - 1
            maxOf(0L, (fromMonths - firstMonths).toLong() / step - 1)
        }
        for (k in k0..k0 + 1000) {
            val d = occurrence(k) ?: return null
            if (d > date && d >= firstDate) return d
        }
        return null
    }

    /** How many [unit]s lie between two runs; a stored 0 counts as one, like the core. */
    private val step: Int get() = maxOf(every, 1)

    /**
     * The k-th run, counted from [firstDate] — not iterated, so a clamped month end (31 → 28/30)
     * cannot drift into every following month. A weekly run falls on the Monday of
     * [firstDate]'s week plus [execDay] - 1 (1 = Monday), not on [firstDate]'s weekday;
     * [execDay] is clamped like the core, so 0 is day 1.
     */
    private fun occurrence(k: Long): LocalDate? {
        val base = if (unit == "W") {
            val monday = firstDate.minusDays(firstDate.dayOfWeek.value - 1L)
            val exec = monday.plusDays((execDay.coerceIn(1, 7) - 1).toLong())
            exec.plusDays(7L * step * k)
        } else {
            val months = firstDate.year * 12 + firstDate.monthValue - 1 + step.toLong() * k
            val m = LocalDate.of((months / 12).toInt(), (months % 12).toInt() + 1, 1)
            m.withDayOfMonth(minOf(execDay.coerceIn(1, 31), m.lengthOfMonth()))
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

/**
 * A manual loan as the core stores it (GitHub #36): a loan account whose debt comes from a Stand —
 * the outstanding balance at the end of [anchorDate] — minus the repayments the core links to it,
 * with only the principal share counting once a rate is set.
 *
 * Nothing in this class is calculated here: [remainingCents] and everything below it is the core's
 * `LoanStatus`. The account itself already carries the debt as a negative balance, so every sum has
 * to count the loan once — never the account *and* [remainingCents].
 */
data class ManualLoan(
    /** The loan account; its balance is the core's own figure for the same debt. */
    val accountId: String,
    val name: String,
    /** Original amount for the progress bar; null = not set, the Stand then stands for itself. */
    val originalCents: Long?,
    /** Stand date: repayments booked after this day reduce the balance. */
    val anchorDate: LocalDate,
    /** Outstanding balance at the end of [anchorDate] (positive cents). */
    val anchorBalanceCents: Long,
    /** Nominal rate p.a. in millionths (4.99 % = 49_900); null = no interest. */
    val interestRatePpm: Long?,
    /** Rule syntax that links repayments by itself; "" = none. */
    val matchExpr: String,
    val remainingCents: Long,
    /** Date [remainingCents] refers to: the last counted payment, else [anchorDate]. */
    val asOfDate: LocalDate?,
    val paymentCount: Int,
    val paidCents: Long,
    /** Part of [paidCents] that reduced the debt. */
    val principalCents: Long,
    val interestCents: Long,
    /** Linked bookings on or before the Stand date — shown, not counted. */
    val beforeAnchorCount: Int,
    val lastPaymentCents: Long?,
    val lastPaymentDate: LocalDate?,
    /** Monthly payments still needed at the last payment's amount; null = not known. */
    val monthsLeft: Int?,
) {
    /** Share repaid, 0…1; null while no original amount is stored. */
    val repaid: Double?
        get() = originalCents
            ?.takeIf { it > 0 }
            ?.let { (1.0 - remainingCents.toDouble() / it).coerceIn(0.0, 1.0) }
}

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
