package de.girocrab.app.security

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.KeyStore
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Covers the rule this step is about: a biometric prompt that does not authenticate yields no
 * cipher, and therefore no vault. The prompt is replaced by a fake — an emulator has no enrolled
 * biometric to hand a real `CryptoObject` to — but the Keystore, the wrapped key and the core are
 * the real thing.
 */
@RunWith(AndroidJUnit4::class)
class VaultUnlockerTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    /**
     * The app's data directory is the production vault location. Both `@Before` and `@After` wipe
     * state, so the test gets its own directory and never touches a real vault.
     */
    private val testDir = File(context.filesDir, "test-unlock")
    private val alias = "girocrab.test.unlock"
    private val wrappedFile = File(testDir, "device-key.bin")
    private val vaultFile = File(File(testDir, "vaults"), "girocrab.sqlcipher")
    private val opener = NativeVaultOpener()

    private fun keyStore() = VaultKeyStore(wrappedFile, alias, requireUserAuthentication = false)

    @Before
    fun clean() {
        deleteAlias()
        testDir.deleteRecursively()
        testDir.mkdirs()
    }

    @After
    fun tearDown() {
        testDir.deleteRecursively()
        deleteAlias()
    }

    @Test
    fun nothingStoredMeansNoVaultOnDevice() {
        var prompted = false
        val unlocker = unlocker { cipher, _, _, onResult ->
            prompted = true
            onResult(PromptResult.Succeeded(cipher))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertEquals(UnlockResult.NoVaultOnDevice, result)
        assertFalse("no prompt without a wrapped key", prompted)
        assertFalse("no vault may be created", vaultFile.exists())
    }

    @Test
    fun cancelledPromptUnlocksNothing() {
        storeWrappedKey()
        val sizeBefore = vaultBytes()
        var prompted = false
        val unlocker = unlocker { _, _, _, onResult ->
            prompted = true
            onResult(PromptResult.Cancelled)
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertTrue("the prompt has to be shown", prompted)
        assertTrue("cancellation must not open anything, was $result", result is UnlockResult.Refused)
        assertEquals("a cancelled prompt must not write to the vault", sizeBefore, vaultBytes())
    }

    @Test
    fun failedPromptUnlocksNothing() {
        storeWrappedKey()
        val sizeBefore = vaultBytes()
        val unlocker = unlocker { _, _, _, onResult ->
            onResult(PromptResult.Failed("zu viele Versuche"))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertEquals(UnlockResult.Refused("zu viele Versuche"), result)
        assertEquals("a failed prompt must not write to the vault", sizeBefore, vaultBytes())
    }

    /**
     * A cipher that was not handed to the prompt must never be able to unwrap the key — otherwise
     * a caller could pass an unauthenticated cipher through the prompt result.
     */
    @Test
    fun foreignCipherIsRefused() {
        storeWrappedKey()
        val sizeBefore = vaultBytes()
        // An *initialized* cipher from the same key: only the identity check can refuse it, so the
        // test cannot pass just because an uninitialized cipher would fail anyway.
        val foreign = keyStore().unlockCipher()
        val unlocker = unlocker { _, _, _, onResult ->
            onResult(PromptResult.Succeeded(foreign))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertTrue("a foreign cipher must be refused, was $result", result is UnlockResult.Refused)
        assertEquals("a foreign cipher must not write to the vault", sizeBefore, vaultBytes())
    }

    /**
     * Android never creates a vault (#1459, Import-only). With a key but no file, opening would
     * create an empty vault and make "the data is gone" look like "there was never data".
     */
    @Test
    fun missingVaultFileIsReportedWithoutPrompt() {
        storeWrappedKey(createVault = false)
        var prompted = false
        val unlocker = unlocker { cipher, _, _, onResult ->
            prompted = true
            onResult(PromptResult.Succeeded(cipher))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertEquals(UnlockResult.VaultMissing, result)
        assertFalse("a missing vault must not prompt", prompted)
        assertFalse("an empty vault must not be created", vaultFile.exists())
    }

    @Test
    fun authenticatedPromptOpensTheVault() {
        val key = storeWrappedKey()
        createVaultFile(key)
        val unlocker = unlocker { cipher, _, _, onResult ->
            onResult(PromptResult.Succeeded(cipher))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        val opened = result as? UnlockResult.Opened
        assertTrue("expected an opened vault, was $result", opened != null)
        try {
            assertEquals(0, opened!!.vault.accountCount)
        } finally {
            opened!!.vault.close()
        }
        assertTrue("the vault file has to exist now", vaultFile.length() > 0)
    }

    @Test
    fun invalidatedKeystoreKeyIsReportedWithoutPrompt() {
        storeWrappedKey()
        deleteAlias()
        var prompted = false
        val unlocker = unlocker { cipher, _, _, onResult ->
            prompted = true
            onResult(PromptResult.Succeeded(cipher))
        }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertEquals(UnlockResult.KeyInvalidated, result)
        assertFalse("an invalidated key must not prompt", prompted)
    }

    /**
     * The catch that guards a prompt which never calls back must not answer on top of an answer:
     * a caller that throws while handling the result would otherwise receive a refusal instead of
     * the opened vault, and the lock screen would show the wrong reason.
     */
    @Test
    fun throwingCallerIsAnsweredOnlyOnce() {
        val key = storeWrappedKey()
        createVaultFile(key)
        val unlocker = unlocker { cipher, _, _, onResult ->
            onResult(PromptResult.Succeeded(cipher))
        }

        val results = mutableListOf<UnlockResult>()
        unlocker.unlock { result ->
            results += result
            throw IllegalStateException("Oberfläche scheitert beim Lesen der Kontozahl")
        }

        assertEquals("exactly one answer, was $results", 1, results.size)
        val opened = results.single() as? UnlockResult.Opened
        assertTrue("expected the opened vault, was ${results.single()}", opened != null)
        opened!!.vault.close()
    }

    /**
     * #1428 (2.5): the soft lock only takes the window away — the vault is still open. Releasing it
     * has to prove the key without opening a second vault and without writing anything.
     */
    @Test
    fun softReleaseProvesTheKeyWithoutOpeningTheVault() {
        storeWrappedKey(createVault = false)
        var prompted = false
        val unlocker = unlocker { cipher, _, _, onResult ->
            prompted = true
            onResult(PromptResult.Succeeded(cipher))
        }

        var result: UnlockResult? = null
        unlocker.releaseSoftLock { result = it }

        assertTrue("the prompt has to be shown", prompted)
        assertEquals("expected the window to open again, was $result", UnlockResult.SoftReleased, result)
        assertEquals("the soft lock must not create a vault", 0L, vaultBytes())
    }

    /** A refused prompt leaves the window locked — the vault stays untouched either way. */
    @Test
    fun cancelledSoftReleaseKeepsTheWindowLocked() {
        storeWrappedKey(createVault = true)
        val before = vaultBytes()
        val unlocker = unlocker { _, _, _, onResult -> onResult(PromptResult.Cancelled) }

        var result: UnlockResult? = null
        unlocker.releaseSoftLock { result = it }

        assertTrue("a cancelled prompt is a refusal, was $result", result is UnlockResult.Refused)
        assertEquals("the vault file must stay as it was", before, vaultBytes())
    }

    /** A stored key without its vault is not a vault: nothing may be created behind the prompt. */
    @Test
    fun storedKeyWithoutVaultFileIsNoVaultOnDevice() {
        storeWrappedKey(createVault = false)
        val unlocker = unlocker { cipher, _, _, onResult -> onResult(PromptResult.Succeeded(cipher)) }

        var result: UnlockResult? = null
        unlocker.unlock { result = it }

        assertEquals(UnlockResult.NoVaultOnDevice, result)
        assertFalse("no vault may be created", vaultFile.exists())
    }

    // --- helpers ---------------------------------------------------------------------------

    private fun createVaultFile(key: ByteArray) {
        val vault = opener.open(opener.defaultVaultPath(testDir), key.copyOf())
        checkNotNull(vault) { "native core not linked" }.close()
    }

    private fun unlocker(prompt: CipherPrompt) = VaultUnlocker(
        keyStore = keyStore(),
        opener = opener,
        prompt = prompt,
        filesDir = testDir,
    )

    /**
     * Stores a key and proves the plain round-trip first. If the unlocker test fails afterwards,
     * the defect is in the unlocker, not in the store — otherwise both look identical in the log.
     */
    private fun storeWrappedKey(createVault: Boolean = true): ByteArray {
        val key = ByteArray(VaultKeyStore.VAULT_KEY_BYTES) { (it + 11).toByte() }
        val store = keyStore()
        store.storeWrapped(key, store.setupCipher())
        assertEquals(VaultKeyStore.State.Ready, store.state())
        assertArrayEquals(
            "wrapped key must read back before the unlocker is involved",
            key,
            store.readWrapped(store.unlockCipher()),
        )
        // On a device the import created the file; without it the unlocker reports VaultMissing.
        if (createVault) handleOrFail(opener.open(vaultFile.path, key)).close()
        return key
    }

    private fun vaultBytes(): Long = if (vaultFile.exists()) vaultFile.length() else 0L

    private fun handleOrFail(vault: OwnVault?): OwnVault =
        vault ?: throw AssertionError("core did not return a handle (not linked?)")

    private fun deleteAlias() {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (keyStore.containsAlias(alias)) keyStore.deleteEntry(alias)
    }
}
