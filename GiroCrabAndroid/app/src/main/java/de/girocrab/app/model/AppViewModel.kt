package de.girocrab.app.model

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import de.girocrab.app.ui.screens.SepaTransferDraft
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

enum class VaultState { Locked, Demo, Own }

data class AppUiState(
    val vaultState: VaultState = VaultState.Locked,
    /** True once the own vault may contain real data; demo must then never overwrite it. */
    val vaultMayHoldRealData: Boolean = false,
    val loading: Boolean = false,
    val privacyMode: Boolean = false,
    /**
     * #1428 (2.5): the app left the foreground. The vault stays open and the data stays loaded —
     * only the window has to be unlocked again. Distinct from [VaultState.Locked], which means
     * there is no vault content at all.
     */
    val backgroundLocked: Boolean = false,
    val data: VaultSnapshot = VaultSnapshot(),
    val scope: Scope = Scope.All,
    val search: String = "",
    val filter: BookingFilter = BookingFilter(),
    val error: String? = null,
    val language: String = "de",
    /** #789: false = GiroCrab's built-in FinTS registration (no product ID asked). */
    val useOwnFintsId: Boolean = false,
    val fintsProductId: String = "",
) {
    val demoActive get() = vaultState == VaultState.Demo
    val isUnlocked get() = vaultState != VaultState.Locked

    val selectedAccountId get() = (scope as? Scope.OneAccount)?.accountId

    val scopedAccountIds: Set<String>
        get() = when (val s = scope) {
            Scope.All -> data.accounts.map { it.id }.toSet()
            is Scope.Group -> data.accounts.filter { it.groupId == s.groupId }.map { it.id }.toSet()
            is Scope.OneAccount -> setOf(s.accountId)
        }

    val totalBalanceCents get() = data.accounts.filter { it.id in scopedAccountIds }.sumOf { it.balanceCents }
}

class AppViewModel(private val core: CoreBridge = DefaultCoreBridge()) : ViewModel() {
    private val _state = MutableStateFlow(AppUiState())
    val state: StateFlow<AppUiState> = _state.asStateFlow()

    /** Bookings after scope, search and filter, newest first. */
    val visibleBookings: StateFlow<List<Booking>> = _state
        .map { filterBookings(it) }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    // Own vault snapshot is kept while demo is shown so switching back loses nothing.
    private var ownSnapshot: VaultSnapshot? = null
    /** A background lock that arrived before the vault did (import still running). */
    private var pendingBackgroundLock = false

    init { bootstrap() }

    // ---- Vault lifecycle ----

    fun bootstrap() {
        viewModelScope.launch {
            _state.update { it.copy(loading = true) }
            // No vault is opened here anymore: the own vault needs the device key, and that only
            // comes back from the biometric gate (#1428, 2.2/2.4). Until then the app starts in
            // demo mode, which is exactly what it did while `openOwnVault()` returned null.
            showDemo()
        }
    }

    private fun adoptOwn(snapshot: VaultSnapshot) {
        ownSnapshot = snapshot
        // Two ways a lock arrives here: it was waiting for the vault (import in the background) or
        // it is already set (`showOwnVault` hands back the cached snapshot). Both have to survive —
        // only a successful prompt clears it.
        val locked = pendingBackgroundLock
        pendingBackgroundLock = false
        _state.update {
            it.copy(
                vaultState = VaultState.Own, vaultMayHoldRealData = true, loading = false,
                data = snapshot, scope = Scope.All, filter = BookingFilter(), search = "", error = null,
                backgroundLocked = locked || it.backgroundLocked,
            )
        }
    }

    fun showDemo() {
        viewModelScope.launch {
            val demo = withContext(Dispatchers.Default) { core.demoSnapshot() }
            _state.update {
                it.copy(
                    vaultState = VaultState.Demo, loading = false, data = demo,
                    scope = Scope.All, filter = BookingFilter(), search = "", error = null,
                )
            }
        }
    }

    /**
     * Adopts a snapshot read from the own vault (#1429, 3.1). The native handle stays with the
     * caller and is closed there — the ViewModel keeps data, never a handle.
     */
    fun adoptOwnVault(snapshot: VaultSnapshot) = adoptOwn(snapshot)

    fun showOwnVault() {
        val own = ownSnapshot
        if (own != null) {
            adoptOwn(own)
        } else {
            // Nothing read yet: back to the unlock screen. Falling into the demo here looked like
            // the own vault had been opened, only to reload demo data (#1429).
            _state.update { it.copy(vaultState = VaultState.Locked, loading = false, error = null) }
        }
    }

    /**
     * #1428 (2.5): leaving the foreground locks the window, not the vault. Dropping the snapshot
     * here would mean reading the whole vault again on every app switch for nothing.
     */
    fun lockForBackground() {
        if (_state.value.vaultState != VaultState.Own) {
            // A running import: the vault arrives after the app is gone. Remember the lock, or the
            // imported data would be readable on return without a new prompt.
            pendingBackgroundLock = true
            return
        }
        _state.update { it.copy(backgroundLocked = true) }
    }

    fun clearBackgroundLock() {
        pendingBackgroundLock = false
        _state.update { it.copy(backgroundLocked = false) }
    }

    fun lock() {
        ownSnapshot = null
        pendingBackgroundLock = false
        _state.update { AppUiState(privacyMode = it.privacyMode, vaultMayHoldRealData = it.vaultMayHoldRealData) }
    }

    fun clearError() = _state.update { it.copy(error = null) }

    // ---- Selection / search / filters ----

    fun togglePrivacy() = _state.update { it.copy(privacyMode = !it.privacyMode) }
    fun selectAll() = _state.update { it.copy(scope = Scope.All) }
    fun selectGroup(groupId: String) = _state.update { it.copy(scope = Scope.Group(groupId)) }
    fun selectAccount(accountId: String) = _state.update { it.copy(scope = Scope.OneAccount(accountId)) }
    fun setSearch(text: String) = _state.update { it.copy(search = text) }
    fun setFilter(filter: BookingFilter) = _state.update { it.copy(filter = filter) }
    fun resetFilter() = _state.update { it.copy(filter = BookingFilter(), search = "") }

    // ---- Editing (categories, tags, notes, rules) ----

    private fun editData(f: (VaultSnapshot) -> VaultSnapshot) {
        _state.update { s ->
            val next = f(s.data)
            if (s.vaultState == VaultState.Own) ownSnapshot = next
            s.copy(data = next)
        }
    }

    private fun editBooking(id: String, f: (Booking) -> Booking) =
        editData { d -> d.copy(bookings = d.bookings.map { if (it.id == id) f(it) else it }) }

    fun setCategory(bookingId: String, categoryId: String?) =
        editBooking(bookingId) { it.copy(categoryId = categoryId, categoryByRule = false) }

    fun toggleTag(bookingId: String, tagId: String) =
        editBooking(bookingId) { it.copy(tagIds = if (tagId in it.tagIds) it.tagIds - tagId else it.tagIds + tagId) }

    fun toggleBookingChecked(id: String) = editBooking(id) { it.copy(checked = !it.checked) }

    /** Books a SEPA transfer as an outgoing booking and reduces the account balance (Demo/Own). */
    fun submitTransfer(draft: SepaTransferDraft) {
        val date = draft.executionDate ?: java.time.LocalDate.now()
        val amount = -kotlin.math.abs(draft.amountCents)
        editData { d ->
            val booking = Booking(
                id = "b_tr_${System.nanoTime()}", accountId = draft.accountId, bookingDate = date,
                amountCents = amount, counterparty = draft.recipient, purpose = draft.purpose,
                isTransfer = true, transferReason = draft.purpose, counterpartyIban = draft.iban,
            )
            d.copy(
                bookings = (d.bookings + booking).sortedByDescending { it.bookingDate },
                accounts = d.accounts.map { if (it.id == draft.accountId) it.copy(balanceCents = it.balanceCents + amount) else it },
            )
        }
    }

    fun setNote(bookingId: String, note: String) = editBooking(bookingId) { it.copy(note = note) }

    fun setSplits(bookingId: String, splits: List<SplitPart>) = editBooking(bookingId) { it.copy(splits = splits) }

    fun addTag(name: String, colorHex: Long = 0xFF1976D2) {
        if (name.isBlank()) return
        editData { it.copy(tags = it.tags + Tag("t_${System.nanoTime()}", name.trim(), colorHex)) }
    }

    fun deleteTag(tagId: String) = editData { d ->
        d.copy(
            tags = d.tags.filterNot { it.id == tagId },
            bookings = d.bookings.map { it.copy(tagIds = it.tagIds - tagId) },
            rules = d.rules.map { it.copy(tagIds = it.tagIds - tagId) },
        )
    }

    fun addCategory(name: String, colorHex: Long = 0xFF607D8B, parentId: String? = null) {
        if (name.isBlank()) return
        editData { it.copy(categories = it.categories + Category("c_${System.nanoTime()}", name.trim(), parentId, colorHex)) }
    }

    fun deleteCategory(categoryId: String) = editData { d ->
        d.copy(
            categories = d.categories.filterNot { it.id == categoryId }.map { if (it.parentId == categoryId) it.copy(parentId = null) else it },
            bookings = d.bookings.map { if (it.categoryId == categoryId) it.copy(categoryId = null) else it },
            rules = d.rules.filterNot { it.categoryId == categoryId },
        )
    }

    fun upsertRule(rule: Rule) = editData { d ->
        val exists = d.rules.any { it.id == rule.id }
        d.copy(rules = if (exists) d.rules.map { if (it.id == rule.id) rule else it } else d.rules + rule)
    }

    /** Demo only: a real loan is written through the core (Stufe 2, #1430). */
    fun upsertLoan(loan: ManualLoan) = editData { d ->
        val others = d.loans.filterNot { it.accountId == loan.accountId }
        d.copy(loans = others + loan)
    }

    fun deleteLoan(accountId: String) = editData { d -> d.copy(loans = d.loans.filterNot { it.accountId == accountId }) }

    fun upsertAsset(asset: TangibleAsset) = editData { d ->
        d.copy(assets = if (d.assets.any { it.id == asset.id }) d.assets.map { if (it.id == asset.id) asset else it } else d.assets + asset)
    }

    fun deleteAsset(id: String) = editData { d -> d.copy(assets = d.assets.filterNot { it.id == id }) }

    fun deleteRule(ruleId: String) = editData { d -> d.copy(rules = d.rules.filterNot { it.id == ruleId }) }

    /**
     * Applies enabled rules: a category only to uncategorized bookings (never overrides manual
     * categories), tags from every matching rule. Returns count changed.
     */
    fun applyRules(): Int {
        var changed = 0
        editData { d ->
            val active = d.rules.filter { it.enabled }
            d.copy(bookings = d.bookings.map { b ->
                applyRulesTo(active, b).also { if (it !== b) changed++ }
            })
        }
        return changed
    }

    /** Scoped apply (#759): only this rule, only uncategorized bookings in the current scope. Returns count. */
    fun applyRuleToUncategorized(rule: Rule): Int {
        var changed = 0
        val scoped = _state.value.scopedAccountIds
        editData { d ->
            d.copy(bookings = d.bookings.map { b ->
                if (b.categoryId != null || b.accountId !in scoped) return@map b
                applyRulesTo(listOf(rule), b).also { if (it !== b) changed++ }
            })
        }
        return changed
    }

    // ---- Settings ----

    fun setLanguage(lang: String) = _state.update { it.copy(language = lang) }
    fun setOwnFintsId(own: Boolean) = _state.update { it.copy(useOwnFintsId = own) }
    fun setFintsProductId(id: String) = _state.update { it.copy(fintsProductId = id.trim()) }

    // ---- Derived values ----

    fun category(id: String?): Category? = id?.let { i -> _state.value.data.categories.firstOrNull { it.id == i } }
    fun account(id: String): Account? = _state.value.data.accounts.firstOrNull { it.id == id }

    /** Sum of expenses per category (cents, positive) for the current scope + filter. */
    fun spendingByCategory(): Map<String?, Long> =
        filterBookings(_state.value).filter { it.amountCents < 0 }
            .groupBy { it.categoryId }
            .mapValues { (_, v) -> -v.sumOf { it.amountCents } }

    /**
     * Net worth in [currency] — only that currency, because balances of different currencies are
     * not converted anywhere in the vault. Adding them would produce a number without a unit.
     */
    fun netWorthCents(currency: String = _state.value.data.primaryCurrency()): Long {
        val d = _state.value.data
        val counted = d.accounts.filter {
            !it.hidden && it.includeInTotal && it.currency.ifBlank { "EUR" } == currency
        }
        val countedIds = counted.map { it.id }.toSet()
        // A tangible asset whose account is counted is already inside that account's balance; only
        // assets of **uncounted** accounts (excluded, hidden or absent) never were, and only those
        // without any account at all are added on top (#1429).
        val allIds = d.accounts.map { it.id }.toSet()
        val looseAssets = d.assets.filter { it.accountId.isBlank() || it.accountId !in allIds }
        // A manual loan is an account whose balance already is the outstanding debt. Its record is
        // the progress behind that figure, so only a loan whose account is missing counts again.
        val looseLoans = d.loans.filter { it.accountId !in allIds }
        return counted.sumOf { it.balanceCents } + looseAssets.sumOf { it.valueCents } -
            looseLoans.sumOf { it.remainingCents }
    }

    companion object {
        /**
         * The core's split (`RuleSet::categorise` / `tags_for`): the category comes from the first
         * matching rule that sets one, and only for an uncategorized booking; tag-only rules never
         * categorise. Tags are the union of every matching rule. Returns [b] itself when nothing changes.
         */
        fun applyRulesTo(rules: List<Rule>, b: Booking): Booking {
            val matching = rules.filter { it.matches(b) }
            val tags = b.tagIds + matching.flatMap { it.tagIds }
            val category = if (b.categoryId == null) matching.firstOrNull { it.categoryId != null }?.categoryId else null
            return when {
                category != null -> b.copy(categoryId = category, categoryByRule = true, tagIds = tags)
                tags != b.tagIds -> b.copy(tagIds = tags)
                else -> b
            }
        }

        fun filterBookings(s: AppUiState): List<Booking> {
            val ids = s.scopedAccountIds
            val f = s.filter
            val q = s.search.trim()
            return s.data.bookings.asSequence()
                .filter { it.accountId in ids }
                .filter { b ->
                    when (f.direction) {
                        Direction.Any -> true
                        Direction.Income -> b.amountCents > 0
                        Direction.Expense -> b.amountCents < 0
                    }
                }
                .filter { f.minCents == null || kotlin.math.abs(it.amountCents) >= f.minCents }
                .filter { f.maxCents == null || kotlin.math.abs(it.amountCents) <= f.maxCents }
                .filter { f.categoryId == null || it.categoryId == f.categoryId }
                .filter { f.tagId == null || f.tagId in it.tagIds }
                .filter { !f.uncategorizedOnly || it.categoryId == null }
                .filter { f.from == null || !it.bookingDate.isBefore(f.from) }
                .filter { f.to == null || !it.bookingDate.isAfter(f.to) }
                .filter { b ->
                    q.isEmpty() || listOf(b.counterparty, b.purpose, b.note, b.reference.orEmpty())
                        .any { it.contains(q, ignoreCase = true) }
                }
                .sortedByDescending { it.bookingDate }
                .toList()
        }
    }
}
