package de.girocrab.app.security

import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.io.File
import javax.crypto.Cipher

/** What the lock screen has to show after asking for the device key. */
sealed interface UnlockResult {
    /** The device key was unwrapped and the vault is open. Caller owns the handle. */
    class Opened(val vault: OwnVault) : UnlockResult

    /** Nothing is stored on this device yet — the vault has to be restored from a backup (2.6). */
    data object NoVaultOnDevice : UnlockResult

    /** The Keystore key is gone or invalidated; only a restore brings the vault back. */
    data object KeyInvalidated : UnlockResult

    /**
     * Nothing to authenticate with — no biometric hardware, nothing enrolled — or no native core
     * linked. Distinct from [Refused] on purpose: retrying cannot help, importing can.
     */
    data object Unavailable : UnlockResult

    /** Aborted, failed, or unusable cipher — nothing was unwrapped. */
    data class Refused(val message: String?) : UnlockResult
}

/** Answer of a biometric prompt. Only [Succeeded] carries a cipher, and only that one may be used. */
sealed interface PromptResult {
    class Succeeded(val cipher: Cipher) : PromptResult
    data object Cancelled : PromptResult
    /** No sensor, no enrollment, hardware temporarily unavailable — retrying will not help. */
    data class Unavailable(val message: String?) : PromptResult
    data class Failed(val message: String?) : PromptResult
}

/** Injection seam: tests replace the system prompt, the app uses [AndroidCipherPrompt]. */
fun interface CipherPrompt {
    fun authenticate(cipher: Cipher, title: String, subtitle: String, onResult: (PromptResult) -> Unit)
}

/**
 * Turns "the user proved who they are" into "the vault is open", and into nothing else.
 *
 * The order matters: the wrapped key is read **only** with the cipher the prompt authenticated,
 * and only that exact cipher instance is accepted. A cancelled or failed prompt therefore ends
 * with [UnlockResult.Refused] — no key, no vault, no half-open state.
 */
class VaultUnlocker(
    private val keyStore: VaultKeyStore,
    private val opener: NativeVaultOpener,
    private val prompt: CipherPrompt,
    private val filesDir: File,
    private val title: String = "GiroCrab entsperren",
    private val subtitle: String = "Bestätige deine Identität, um den Tresor zu öffnen",
) {
    /** A second request while a prompt is open must not stack a second prompt. */
    private var prompting = false

    fun unlock(onResult: (UnlockResult) -> Unit) {
        if (prompting) return
        // Reading the wrapped file can fail (I/O). That is not an invalidated key: the user may
        // retry, so it must not travel as `KeyInvalidated`.
        val state = try {
            keyStore.state()
        } catch (e: Exception) {
            return onResult(
                UnlockResult.Refused(
                    "Tresorschlüssel nicht lesbar (${e.javaClass.simpleName}: ${e.message}) — neuer Versuch möglich.",
                ),
            )
        }
        when (state) {
            VaultKeyStore.State.NotSetUp -> return onResult(UnlockResult.NoVaultOnDevice)
            VaultKeyStore.State.Invalidated -> return onResult(UnlockResult.KeyInvalidated)
            VaultKeyStore.State.Ready -> Unit
        }
        // Android never creates a vault on its own (#1459, Import-only): without a file there is
        // nothing to open, and the only way back is a backup from the desktop.
        if (!File(opener.defaultVaultPath(filesDir)).isFile) {
            return onResult(UnlockResult.VaultMissing)
        }
        val cipher = try {
            keyStore.unlockCipher()
        } catch (_: Exception) {
            // Key invalidated between state() and here: same answer, the key is unusable.
            return onResult(UnlockResult.KeyInvalidated)
        }
        prompting = true
        try {
            prompt.authenticate(cipher, title, subtitle) { result ->
                prompting = false
                when (result) {
                    PromptResult.Cancelled -> onResult(UnlockResult.Refused(null))
                    is PromptResult.Unavailable -> onResult(UnlockResult.Unavailable)
                    is PromptResult.Failed -> onResult(UnlockResult.Refused(result.message))
                    is PromptResult.Succeeded -> onAuthenticated(cipher, result.cipher, onResult)
                }
            }
        } catch (e: Exception) {
            // A prompt that never calls back would block every later attempt.
            prompting = false
            onResult(UnlockResult.Refused("Entsperren nicht möglich (${e.javaClass.simpleName}: ${e.message})"))
        }
    }

    private fun onAuthenticated(
        expected: Cipher,
        authenticated: Cipher,
        onResult: (UnlockResult) -> Unit,
    ) {
        if (authenticated !== expected) {
            onResult(UnlockResult.Refused("Der Prompt lieferte einen anderen Cipher als angefordert."))
            return
        }
        val key = try {
            keyStore.readWrapped(authenticated)
        } catch (e: Exception) {
            // A bare `message` hides the cause: GCM tag failures and Keystore errors carry none.
            onResult(UnlockResult.Refused("Entsperren fehlgeschlagen (${e.javaClass.simpleName}: ${e.message})"))
            return
        }
        try {
            val vault = opener.open(opener.defaultVaultPath(filesDir), key)
            onResult(if (vault != null) UnlockResult.Opened(vault) else UnlockResult.Unavailable)
        } catch (e: Exception) {
            onResult(UnlockResult.Refused(e.message))
        } finally {
            // The core has copied what it needs; the plain key does not stay in the heap.
            key.fill(0)
        }
    }
}

/**
 * System prompt with the cipher attached as `CryptoObject`: the Keystore only releases the key
 * for an authenticated cipher, so a prompt that ends without authentication cannot yield one.
 */
class AndroidCipherPrompt(private val activity: FragmentActivity) : CipherPrompt {

    override fun authenticate(
        cipher: Cipher,
        title: String,
        subtitle: String,
        onResult: (PromptResult) -> Unit,
    ) {
        if (!canAuthenticate()) {
            // No sensor or nothing enrolled: retrying cannot help, so this is not a refusal.
            onResult(PromptResult.Unavailable("Auf diesem Gerät ist keine sichere Sperre eingerichtet."))
            return
        }
        val prompt = BiometricPrompt(
            activity,
            ContextCompat.getMainExecutor(activity),
            object : BiometricPrompt.AuthenticationCallback() {
                override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                    val authenticated = result.cryptoObject?.cipher
                    onResult(
                        if (authenticated != null) PromptResult.Succeeded(authenticated)
                        else PromptResult.Failed("Die Biometrie lieferte keinen Cipher zurück."),
                    )
                }

                override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                    val message = errString.toString()
                    when (errorCode) {
                        BiometricPrompt.ERROR_NEGATIVE_BUTTON,
                        BiometricPrompt.ERROR_USER_CANCELED,
                        BiometricPrompt.ERROR_CANCELED,
                        -> onResult(PromptResult.Cancelled)

                        // Hardware or enrollment gone: a retry cannot succeed, but importing can.
                        BiometricPrompt.ERROR_HW_UNAVAILABLE,
                        BiometricPrompt.ERROR_HW_NOT_PRESENT,
                        BiometricPrompt.ERROR_NO_BIOMETRICS,
                        BiometricPrompt.ERROR_NO_DEVICE_CREDENTIAL,
                        -> onResult(PromptResult.Unavailable(message))

                        else -> onResult(PromptResult.Failed(message))
                    }
                }
            },
        )
        val builder = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setAllowedAuthenticators(authenticators())
            // Biometric-only prompts require a negative button on every API level; it is also what
            // makes `Cancelled` reachable instead of ending in an error code.
            .setNegativeButtonText("Abbrechen")
        prompt.authenticate(builder.build(), BiometricPrompt.CryptoObject(cipher))
    }

    private fun canAuthenticate(): Boolean =
        BiometricManager.from(activity).canAuthenticate(authenticators()) == BiometricManager.BIOMETRIC_SUCCESS

    companion object {
        /**
         * Biometrics only, on every API level: the Keystore key is created with
         * `AUTH_BIOMETRIC_STRONG`, so a device PIN could never release it. Offering one would end
         * in a successful prompt and an unwrappable key — the worst kind of "it almost worked".
         */
        const val AUTHENTICATORS: Int = BiometricManager.Authenticators.BIOMETRIC_STRONG

        fun authenticators(): Int = AUTHENTICATORS
    }
}
