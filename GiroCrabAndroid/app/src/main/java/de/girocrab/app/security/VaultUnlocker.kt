package de.girocrab.app.security

import android.os.Build
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

    /** No biometric hardware, nothing enrolled, or no core linked. */
    data object Unavailable : UnlockResult

    /** Aborted, failed, or unusable cipher — nothing was unwrapped. */
    data class Refused(val message: String?) : UnlockResult
}

/** Answer of a biometric prompt. Only [Succeeded] carries a cipher, and only that one may be used. */
sealed interface PromptResult {
    class Succeeded(val cipher: Cipher) : PromptResult
    data object Cancelled : PromptResult
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
    fun unlock(onResult: (UnlockResult) -> Unit) {
        when (keyStore.state()) {
            VaultKeyStore.State.NotSetUp -> return onResult(UnlockResult.NoVaultOnDevice)
            VaultKeyStore.State.Invalidated -> return onResult(UnlockResult.KeyInvalidated)
            VaultKeyStore.State.Ready -> Unit
        }
        val cipher = try {
            keyStore.unlockCipher()
        } catch (_: Exception) {
            // Key invalidated between state() and here: same answer, the key is unusable.
            return onResult(UnlockResult.KeyInvalidated)
        }
        prompt.authenticate(cipher, title, subtitle) { result ->
            when (result) {
                PromptResult.Cancelled -> onResult(UnlockResult.Refused(null))
                is PromptResult.Failed -> onResult(UnlockResult.Refused(result.message))
                is PromptResult.Succeeded -> onAuthenticated(cipher, result.cipher, onResult)
            }
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
            onResult(UnlockResult.Refused(e.message))
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
            onResult(PromptResult.Failed("Auf diesem Gerät ist keine sichere Sperre eingerichtet."))
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
                    val cancelled = errorCode == BiometricPrompt.ERROR_NEGATIVE_BUTTON ||
                        errorCode == BiometricPrompt.ERROR_USER_CANCELED ||
                        errorCode == BiometricPrompt.ERROR_CANCELED
                    onResult(if (cancelled) PromptResult.Cancelled else PromptResult.Failed(errString.toString()))
                }
            },
        )
        val builder = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setAllowedAuthenticators(authenticators())
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) builder.setNegativeButtonText("Abbrechen")
        prompt.authenticate(builder.build(), BiometricPrompt.CryptoObject(cipher))
    }

    private fun canAuthenticate(): Boolean =
        BiometricManager.from(activity).canAuthenticate(authenticators()) == BiometricManager.BIOMETRIC_SUCCESS

    companion object {
        /**
         * Device credential is only combinable with biometrics from API 30; below that the prompt
         * needs a negative button and asks for biometrics only.
         */
        fun authenticators(): Int =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL
            } else {
                BiometricManager.Authenticators.BIOMETRIC_STRONG
            }
    }
}
