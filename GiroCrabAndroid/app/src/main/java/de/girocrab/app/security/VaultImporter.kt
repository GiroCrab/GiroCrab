package de.girocrab.app.security

import java.io.File
import java.security.SecureRandom
import javax.crypto.Cipher

/** Outcome of importing a backup. */
sealed interface ImportResult {
    /** The backup was written into a fresh local vault; caller owns the handle. */
    class Imported(val vault: OwnVault) : ImportResult

    /** The user aborted the prompt — nothing was written. */
    data object RefusedByUser : ImportResult

    /** No biometric hardware, no enrollment or no native core — importing cannot work here. */
    data object Unavailable : ImportResult

    /** Anything that ends without a usable vault; the message is meant for the lock screen. */
    data class Failed(val message: String) : ImportResult
}

/**
 * Brings a desktop backup onto the device (#1428, 2.6).
 *
 * Android never learns the desktop's vault key: the backup is password-encrypted and this device
 * creates its own key, wraps it with the Keystore and then lets the core replace the vault's
 * contents (`Vault::restore_backup`). The password therefore has to be right, and the device key
 * still needs a biometric prompt before it is stored.
 *
 * Order matters. The wrapped key is written **last**, only after the restore succeeded: if the
 * process dies in between, `state()` still reports `NotSetUp`, so the next attempt starts clean
 * instead of refusing because a half-restored vault claims to be ready.
 *
 * An import only replaces something that is already unusable: a dead Keystore key (invalidated by
 * new biometrics or a reset screen lock) or a vault file without a key to open it. A usable key
 * **with** its vault file is never touched — that would be data loss dressed up as recovery.
 */
class VaultImporter(
    private val keyStore: VaultKeyStore,
    private val opener: NativeVaultOpener,
    private val prompt: CipherPrompt,
    private val filesDir: File,
    private val title: String = "Tresor einspielen",
    private val subtitle: String = "Bestätige deine Identität, damit der Geräteschlüssel geschützt wird",
) {
    /** One import at a time: two overlapping runs would fight over the same vault file. */
    private var running = false

    fun import(backupPath: String, password: String, onResult: (ImportResult) -> Unit) {
        if (running) {
            return onResult(ImportResult.Failed("Es läuft bereits ein Import."))
        }
        if (!File(backupPath).isFile) {
            return onResult(ImportResult.Failed("Sicherungsdatei wurde nicht gefunden."))
        }
        if (!opener.available) return onResult(ImportResult.Unavailable)

        val vaultPath = File(opener.defaultVaultPath(filesDir))
        val state = try {
            keyStore.state()
        } catch (e: Exception) {
            return onResult(
                ImportResult.Failed("Geräteschlüssel nicht lesbar (${e.javaClass.simpleName}: ${e.message})."),
            )
        }
        // There is only something to lose when both parts exist: a usable key and its vault.
        if (state == VaultKeyStore.State.Ready && vaultPath.exists()) {
            return onResult(
                ImportResult.Failed(
                    "Auf diesem Gerät liegt schon ein Tresor. Ein Import würde ihn ersetzen — " +
                        "erst den vorhandenen Tresor entfernen.",
                ),
            )
        }
        // Anything else is unusable: a key that the Keystore no longer honours, or a vault file
        // nobody can open. Both are replaced rather than kept as a trap.
        if (state != VaultKeyStore.State.NotSetUp) {
            keyStore.resetForNewSetup()
        }
        if (vaultPath.exists() && !vaultPath.delete()) {
            return onResult(
                ImportResult.Failed("Reste eines früheren Imports ließen sich nicht entfernen."),
            )
        }

        val key = ByteArray(NativeVaultOpener.MASTER_KEY_BYTES)
        SecureRandom().nextBytes(key)

        val cipher = try {
            keyStore.setupCipher()
        } catch (e: Exception) {
            key.fill(0)
            return onResult(
                ImportResult.Failed("Geräteschlüssel ließ sich nicht anlegen (${e.javaClass.simpleName}: ${e.message})."),
            )
        }

        running = true
        var finished = false
        fun finish(result: ImportResult) {
            if (finished) return
            finished = true
            running = false
            onResult(result)
        }

        try {
            prompt.authenticate(cipher, title, subtitle) { result ->
                when (result) {
                    PromptResult.Cancelled -> {
                        key.fill(0)
                        finish(ImportResult.RefusedByUser)
                    }
                    is PromptResult.Unavailable -> {
                        key.fill(0)
                        finish(ImportResult.Unavailable)
                    }
                    is PromptResult.Failed -> {
                        key.fill(0)
                        finish(ImportResult.Failed(result.message ?: "Authentifizierung fehlgeschlagen."))
                    }
                    is PromptResult.Succeeded -> {
                        if (result.cipher !== cipher) {
                            key.fill(0)
                            finish(ImportResult.Failed("Der Prompt lieferte einen anderen Cipher als angefordert."))
                            return@authenticate
                        }
                        restore(result.cipher, key, vaultPath, backupPath, password) { finish(it) }
                    }
                }
            }
        } catch (e: Exception) {
            key.fill(0)
            finish(ImportResult.Failed("Import nicht möglich (${e.javaClass.simpleName}: ${e.message})."))
        }
    }

    private fun restore(
        authenticated: Cipher,
        key: ByteArray,
        vaultPath: File,
        backupPath: String,
        password: String,
        finish: (ImportResult) -> Unit,
    ) {
        var vault: OwnVault? = null
        try {
            val opened = opener.open(vaultPath.path, key)
                ?: return finish(ImportResult.Unavailable)
            vault = opened

            try {
                opened.restoreBackup(backupPath, password)
            } catch (e: Exception) {
                throw ImportFailed(
                    "Sicherung ließ sich nicht einspielen (${e.javaClass.simpleName}: ${e.message})",
                )
            }

            // Published only after the data is in place — see the class comment.
            keyStore.storeWrapped(key, authenticated)
            vault = null // ownership moves to the caller
            // Deliberately after the handler: a throwing caller must not roll back a good import.
            finish(ImportResult.Imported(opened))
        } catch (e: Exception) {
            runCatching { vault?.close() }
            keyStore.clearWrapped()
            vaultPath.delete()
            val reason = if (e is ImportFailed) {
                e.message
            } else {
                "Import abgebrochen (${e.javaClass.simpleName}: ${e.message})"
            }
            finish(ImportResult.Failed(reason ?: "Import abgebrochen."))
        } finally {
            key.fill(0)
        }
    }

    /** Marks a failure that is already phrased for the lock screen. */
    private class ImportFailed(override val message: String) : Exception(message)
}
