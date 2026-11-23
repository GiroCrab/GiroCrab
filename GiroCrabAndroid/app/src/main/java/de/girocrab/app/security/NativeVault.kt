package de.girocrab.app.security

import java.io.File
import java.lang.reflect.InvocationTargetException
import java.lang.reflect.Method

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
                val raw = call("accountCount") as? Number
                    ?: throw IllegalStateException("accountCount did not return a number")
                return raw.toInt()
            }

        override val accountNames: List<String>
            get() {
                val list = call("listAccounts") as? List<*>
                    ?: throw IllegalStateException("listAccounts did not return a list")
                return list.map { account ->
                    val name = account?.let { invoke(it.javaClass.getMethod("getName"), it) }
                    name as? String ?: throw IllegalStateException("account without a name getter")
                }
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

        private fun call(name: String): Any? {
            // `Class.getMethods` has no defined order, so the exact name is tried explicitly first
            // and the mangled `name-<hash>` form is only a fallback.
            val method = methods.firstOrNull { it.name == name }
                ?: methods.firstOrNull { it.name.startsWith("$name-") }
                ?: throw IllegalStateException(
                    "generated binding has no '$name' (found ${methods.joinToString { it.name }}); " +
                        "a value-class return can mangle the JVM name",
                )
            return invoke(method, vault)
        }

        private val methods: List<Method> by lazy {
            vault.javaClass.methods.filter { it.parameterCount == 0 }
        }
    }

    companion object {
        /** Matches `Vault::open`: the raw master key is 32 bytes and wiped by the core. */
        const val MASTER_KEY_BYTES = 32
        private const val VAULT_CLASS = "uniffi.girocrab_core.Vault"

        private fun invoke(method: Method, receiver: Any?, vararg args: Any?): Any? =
            try {
                method.invoke(receiver, *args)
            } catch (e: InvocationTargetException) {
                throw e.targetException ?: e
            }
    }
}
