package de.girocrab.app.security

import de.girocrab.app.model.BookingsPaging
import de.girocrab.app.model.VaultSnapshot
import de.girocrab.app.model.VaultSnapshotMapper
import java.io.File
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Method
import java.time.LocalDate
import uniffi.girocrab_core.Account as CoreAccount
import uniffi.girocrab_core.AssetItem as CoreAssetItem
import uniffi.girocrab_core.Mandate as CoreMandate
import uniffi.girocrab_core.StandingOrderInfo as CoreStandingOrder
import uniffi.girocrab_core.PlannedPayment as CorePlannedPayment
import uniffi.girocrab_core.Booking as CoreBooking
import uniffi.girocrab_core.AccountGroup as CoreGroup
import uniffi.girocrab_core.BookingPage as CorePage
import uniffi.girocrab_core.Category as CoreCategory
import uniffi.girocrab_core.Holding as CoreHolding
import uniffi.girocrab_core.Rule as CoreRule
import uniffi.girocrab_core.Tag as CoreTag

/**
 * Handle on an open vault of `girocrab-core`, reached through the UniFFI bindings.
 *
 * Reading data stays in Rust: this type only exposes what the Kotlin side needs to prove it
 * really opened the vault (account count and names) until the snapshot mapper from #1429 (3.1)
 * arrives. Nothing here re-implements core logic.
 *
 * Every member throws when the call cannot be made. Returning `0` or an empty list on a failed
 * reflection call would make "no accounts" and "the binding could not be reached" look identical —
 * and a test asserting zero would then always pass.
 */
interface OwnVault : AutoCloseable {
    val accountCount: Int
    val accountNames: List<String>

    /** Writes a password-protected copy of this vault — the same format the desktop uses. */
    fun writeBackup(destPath: String, password: String)

    /** Replaces this vault's contents with a backup (`Vault::restore_backup`). */
    fun restoreBackup(backupPath: String, password: String)

    /**
     * Creates a local account — needed to prove an import really brought data along (#1428, 2.6)
     * and the entry point for manual accounts later. [kind] is one of `CHECKING`, `SAVINGS`,
     * `CREDIT_CARD`, `PAYMENT_SERVICE`, `BROKERAGE`, `CASH`, `LOAN`.
     */
    fun createOfflineAccount(name: String, kind: String, currency: String, openingCents: Long): Long

    /**
     * Reads the whole vault into the app's snapshot format (#1429, 3.1).
     *
     * The handle may be closed right after: the snapshot is a plain Kotlin copy and the core keeps
     * nothing back. Every call throws when the binding or the core cannot answer — an empty list
     * would make "this vault has no accounts" indistinguishable from "the mapper never ran".
     */
    fun snapshot(): VaultSnapshot
}

/**
 * Opens vaults through the generated Kotlin bindings (`uniffi.girocrab_core.Vault`).
 *
 * The bindings are reached by reflection, exactly like `CorePaymentParser`: the app has to
 * build and run without the native library, and every call then simply reports "no core"
 * instead of failing at class load time.
 */
class NativeVaultOpener {

    val available: Boolean get() = entry != null

    /**
     * Opens (or creates) the vault at [path] with the raw 32-byte master key.
     *
     * Returns null when the core is not linked; a key of the wrong length is refused before
     * anything is touched. A failure inside the core (wrong key, corrupt file) is rethrown as the
     * core's own exception — `Method.invoke` would otherwise wrap it in
     * [InvocationTargetException] and hide what actually went wrong.
     */
    fun open(path: String, key: ByteArray): OwnVault? {
        require(key.size == MASTER_KEY_BYTES) {
            "master key must be $MASTER_KEY_BYTES bytes, was ${key.size}"
        }
        val resolved = entry ?: return null
        // The core opens a file, it does not create directories: `filesDir/vaults` is missing on a
        // fresh install, and the core then reports a database error without any message.
        File(path).parentFile?.let { dir ->
            if (!dir.isDirectory && !dir.mkdirs()) {
                throw IllegalStateException("cannot create ${dir.path}")
            }
        }
        val handle = invoke(resolved.openMethod, resolved.receiver, path, key) ?: return null
        return Handle(handle)
    }

    /** Path the app keeps its own vault at; `vaults/` is runtime data, never in the repository. */
    fun defaultVaultPath(filesDir: File): String =
        File(File(filesDir, "vaults"), "girocrab.sqlcipher").path

    // --- reflection plumbing -------------------------------------------------------------

    /**
     * The method is resolved together with its receiver: Kotlin companion functions are only
     * `static` on the class with `@JvmStatic`, which UniFFI does not emit, so looking the method
     * up on the class while calling it on the `Companion` throws `NoSuchMethodException`.
     */
    private class Entry(val openMethod: Method, val receiver: Any?)

    private val entry: Entry? by lazy {
        val cls = runCatching { Class.forName(VAULT_CLASS) }.getOrNull() ?: return@lazy null
        findOpen(cls)?.let { return@lazy Entry(it, null) }
        val companion = runCatching { cls.getField("Companion").get(null) }.getOrNull()
            ?: return@lazy null
        findOpen(companion.javaClass)?.let { Entry(it, companion) }
    }

    private fun findOpen(target: Class<*>): Method? =
        runCatching { target.getMethod("open", String::class.java, ByteArray::class.java) }.getOrNull()

    private class Handle(private val vault: Any) : OwnVault {
        override val accountCount: Int
            get() {
                val raw = callNoArgs("accountCount") as? Number
                    ?: throw IllegalStateException("accountCount did not return a number")
                return raw.toInt()
            }

        override val accountNames: List<String>
            get() {
                val list = callNoArgs("listAccounts") as? List<*>
                    ?: throw IllegalStateException("listAccounts did not return a list")
                return list.map { account ->
                    val name = account?.let { invoke(it.javaClass.getMethod("getName"), it) }
                    name as? String ?: throw IllegalStateException("account without a name getter")
                }
            }

        override fun writeBackup(destPath: String, password: String) {
            callWith("writeBackup", destPath, password)
        }

        override fun restoreBackup(backupPath: String, password: String) {
            callWith("restoreBackup", backupPath, password)
        }

        override fun createOfflineAccount(
            name: String,
            kind: String,
            currency: String,
            openingCents: Long,
        ): Long {
            val kindClass = Class.forName(ACCOUNT_KIND_CLASS)
            val value = kindClass.enumConstants?.firstOrNull { (it as Enum<*>).name == kind }
                ?: throw IllegalArgumentException("unknown account kind '$kind'")
            val raw = callWith("createOfflineAccount", name, value, currency, openingCents)
            return (raw as? Number)?.toLong()
                ?: throw IllegalStateException("createOfflineAccount did not return an id")
        }

        @Suppress("UNCHECKED_CAST")
        override fun snapshot(): VaultSnapshot {
            val groups = callNoArgs("listAccountGroups") as? List<CoreGroup>
                ?: throw IllegalStateException("listAccountGroups did not return a list")
            val accounts = callNoArgs("listAccounts") as? List<CoreAccount>
                ?: throw IllegalStateException("listAccounts did not return a list")
            // Every page, not just the first: the analytics screen sums whatever arrived, so a
            // truncated page made its totals too small without saying so (#1429, 3.3).
            val coverage = BookingsPaging.readAll<CoreBooking> { offset, limit ->
                val page = callWith(
                    "listTransactions",
                    VaultSnapshotMapper.firstPageFilter(offset, limit.toUInt()),
                ) as? CorePage
                    ?: throw IllegalStateException("listTransactions did not return a page")
                BookingsPaging.Page(page.transactions, page.totalCount.toInt())
            }
            // The core wants both bounds explicitly; `null` means "no limit", not "today".
            val noDate: String? = null
            val categories = callWith("listCategories", noDate, noDate) as? List<CoreCategory>
                ?: throw IllegalStateException("listCategories did not return a list")
            val tags = callNoArgs("listTags") as? List<CoreTag>
                ?: throw IllegalStateException("listTags did not return a list")
            // Core rules. The app applies them for preview and apply too, so the mapper has to
            // keep the fields its matcher reads (range, accounts, case sensitivity).
            val rules = callNoArgs("listRules") as? List<CoreRule>
                ?: throw IllegalStateException("listRules did not return a list")
            // The core asks per account here; the flat list is assembled on this side.
            val holdings = accounts.associate { account ->
                val rows = callWith("listHoldings", account.id) as? List<CoreHolding>
                    ?: throw IllegalStateException("listHoldings did not return a list")
                account.id to rows
            }
            val assets = callNoArgs("listAllAssetItems") as? List<CoreAssetItem>
                ?: throw IllegalStateException("listAllAssetItems did not return a list")
            // `null` = every account, as in the core's `list_mandates(Option<i64>)`.
            val mandates = callWith("listMandates", null as Any?) as? List<CoreMandate>
                ?: throw IllegalStateException("listMandates did not return a list")
            // Empty list = every account, as the core's `list_standing_orders(account_ids)` does.
            val standingOrders = callWith("listStandingOrders", emptyList<Long>()) as? List<CoreStandingOrder>
                ?: throw IllegalStateException("listStandingOrders did not return a list")
            // The core computes the schedule; the window covers the app's longest horizon (365 days).
            val today = LocalDate.now()
            val planned = callWith(
                "listPlannedPayments",
                today.toString(),
                today.plusDays(PLANNED_WINDOW_DAYS).toString(),
                emptyList<Long>(),
            ) as? List<CorePlannedPayment>
                ?: throw IllegalStateException("listPlannedPayments did not return a list")
            return VaultSnapshotMapper.map(
                groups = groups,
                accounts = accounts,
                bookings = coverage.rows,
                bookingsTotal = coverage.totalCount,
                categories = categories,
                tags = tags,
                rules = rules,
                holdingsByAccount = holdings,
                assets = assets,
                mandates = mandates,
                standingOrders = standingOrders,
                planned = planned,
            )
        }

        /**
         * UniFFI objects implement `AutoCloseable`; older bindings only had `destroy`.
         *
         * Only the *lookup* is guarded: a core failure while closing has to reach the caller,
         * otherwise a leaked handle looks exactly like a clean one.
         */
        override fun close() {
            val close = runCatching { vault.javaClass.getMethod("close") }.getOrNull()
            invoke(close ?: vault.javaClass.getMethod("destroy"), vault)
        }

        private fun callNoArgs(name: String): Any? = callWith(name)

        /**
         * Resolves by name and argument count; `Class.getMethods` has no defined order, so the
         * exact name is tried first and the mangled `name-<hash>` form only as a fallback.
         */
        private fun callWith(name: String, vararg args: Any?): Any? {
            val candidates = vault.javaClass.methods.filter { it.parameterCount == args.size }
            val method = candidates.firstOrNull { it.name == name }
                ?: candidates.firstOrNull { it.name.startsWith("$name-") }
                ?: throw IllegalStateException(
                    "generated binding has no '$name' with ${args.size} arguments " +
                        "(found ${candidates.joinToString { it.name }}); a value-class return can mangle the JVM name",
                )
            return invoke(method, vault, *args)
        }
    }

    companion object {
        /** Matches `Vault::open`: the raw master key is 32 bytes and wiped by the core. */
        const val MASTER_KEY_BYTES = 32
        private const val VAULT_CLASS = "uniffi.girocrab_core.Vault"
        private const val ACCOUNT_KIND_CLASS = "uniffi.girocrab_core.AccountKind"

        /** Planned-payment window a snapshot carries: the app's longest horizon plus a month. */
        const val PLANNED_WINDOW_DAYS = 400L

        private fun invoke(method: Method, receiver: Any?, vararg args: Any?): Any? =
            try {
                method.invoke(receiver, *args)
            } catch (e: InvocationTargetException) {
                throw e.targetException ?: e
            }
    }
}
