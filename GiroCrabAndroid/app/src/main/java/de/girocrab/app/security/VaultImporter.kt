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

    /** No native core linked. */
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
 * Refuses to run when a vault is already set up: an import would replace it, and that has to be a
 * deliberate decision, not a side effect of tapping "importieren".
 */
class VaultImporter(
    private val keyStore: VaultKeyStore,
    private val opener: NativeVaultOpener,
    private val prompt: CipherPrompt,
    private val filesDir: File,
    private val title: String = "Tresor einspielen",
    private val subtitle: String = "Bestätige deine Identität, damit der Geräteschlüssel geschützt wird",
) {
    fun import(backupPath: String, password: String, onResult: (ImportResult) -> Unit) {
        if (keyStore.state() == VaultKeyStore.State.Ready) {
            return onResult(
                ImportResult.Failed(
                    "Auf diesem Gerät liegt schon ein Tresor. Ein Import würde ihn ersetzen — " +
                        "erst den vorhandenen Tresor entfernen.",
                ),
            )
        }
        if (!File(backupPath).isFile) {
            return onResult(ImportResult.Failed("Sicherungsdatei wurde nicht gefunden."))
        }
        if (!opener.available) return onResult(ImportResult.Unavailable)

        val vaultPath = File(opener.defaultVaultPath(filesDir))
        // Without a wrapped key nothing can open an existing file: it is a leftover from an aborted
        // attempt and would block this one with "file is not a database".
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
            return onResult(ImportResult.Failed("Geräteschlüssel nicht nutzbar: ${e.message}"))
        }

        prompt.authenticate(cipher, title, subtitle) { result ->
            when (result) {
                PromptResult.Cancelled -> {
                    key.fill(0)
                    onResult(ImportResult.RefusedByUser)
                }
                is PromptResult.Failed -> {
                    key.fill(0)
                    onResult(ImportResult.Failed(result.message ?: "Authentifizierung fehlgeschlagen."))
                }
                is PromptResult.Succeeded -> {
                    if (result.cipher !== cipher) {
                        key.fill(0)
                        return@authenticate onResult(
                            ImportResult.Failed("Der Prompt lieferte einen anderen Cipher als angefordert."),
                        )
                    }
                    restore(result.cipher, key, vaultPath, backupPath, password, onResult)
                }
            }
        }
    }

    private fun restore(
        authenticated: Cipher,
        key: ByteArray,
        vaultPath: File,
        backupPath: String,
        password: String,
        onResult: (ImportResult) -> Unit,
    ) {
        val imported: OwnVault
        try {
            val vault = opener.open(vaultPath.path, key)
                ?: return onResult(ImportResult.Unavailable)

            try {
                vault.restoreBackup(backupPath, password)
            } catch (e: Exception) {
                runCatching { vault.close() }
                throw ImportFailed(
                    "Sicherung ließ sich nicht einspielen (${e.javaClass.simpleName}: ${e.message})",
                )
            }

            // Published only after the data is in place — see the class comment.
            keyStore.storeWrapped(key, authenticated)
            imported = vault
        } catch (e: Exception) {
            keyStore.clearWrapped()
            vaultPath.delete()
            val reason = if (e is ImportFailed) e.message else "Import abgebrochen (${e.javaClass.simpleName}: ${e.message})"
            return onResult(ImportResult.Failed(reason ?: "Import abgebrochen."))
        } finally {
            key.fill(0)
        }

        // Deliberately outside the handler: a throwing caller must not roll back a good import.
        onResult(ImportResult.Imported(imported))
    }

    /** Marks a failure that is already phrased for the lock screen. */
    private class ImportFailed(override val message: String) : Exception(message)
}
