package de.girocrab.app.security

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.security.KeyStore
import java.security.SecureRandom
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Proves the import path from #1428 (2.6): a desktop backup brings accounts back onto a device
 * that has no vault, without the desktop's key ever being transferred.
 *
 * The backup is produced by the core itself (`writeBackup`), so the test also pins the round trip
 * the desktop uses. The prompt is faked — an emulator has no enrolled biometric — while Keystore,
 * wrapped key and core are real.
 */
@RunWith(AndroidJUnit4::class)
class VaultImporterTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    /**
     * Everything lives inside `test-import/`: the app's own `vaults/` directory is production
     * state, and this test deletes recursively in `@Before`/`@After`.
     */
    private val dir = File(context.filesDir, "test-import")
    private val alias = "girocrab.test.import"
    private val wrappedFile = File(dir, "device-key.bin")
    private val vaultPath = File(File(dir, "vaults"), "girocrab.sqlcipher")
    private val backupFile = File(dir, "export.gcbackup")
    private val opener = NativeVaultOpener()

    private fun keyStore() = VaultKeyStore(wrappedFile, alias, requireUserAuthentication = false)

    @Before
    fun clean() {
        deleteAlias()
        dir.deleteRecursively()
        dir.mkdirs()
    }

    @After
    fun tearDown() {
        dir.deleteRecursively()
        deleteAlias()
    }

    @Test
    fun importsAccountsFromABackup() {
        createVaultWithAccountsAndBackup(wipeDevice = true)

        assertFalse("device key has to be gone before the import", wrappedFile.exists())
        assertFalse("vault file has to be gone before the import", vaultPath.exists())

        var result: ImportResult? = null
        result = runImport(importerThatSucceeds(), backupFile.path, BACKUP_PASSWORD)

        val imported = importedOrFail(result)
        try {
            assertEquals("accounts have to come back", 2, imported.vault.accountCount)
            assertTrue(imported.vault.accountNames.contains("Girokonto Test"))
            assertTrue(imported.vault.accountNames.contains("Sparkonto Test"))
        } finally {
            imported.vault.close()
        }

        assertTrue("vault file has to exist", vaultPath.length() > 0)
        assertEquals(VaultKeyStore.State.Ready, keyStore().state())
    }

    @Test
    fun wrongPasswordLeavesNoVaultBehind() {
        createVaultWithAccountsAndBackup(wipeDevice = true)

        var result: ImportResult? = null
        result = runImport(importerThatSucceeds(), backupFile.path, "falsches-passwort")

        assertTrue("import must fail for a wrong password, was $result", result is ImportResult.Failed)
        assertEquals("no usable state may be left", VaultKeyStore.State.NotSetUp, keyStore().state())
        assertFalse("no half-imported vault file may survive", vaultPath.exists())
    }

    @Test
    fun cancelledPromptWritesNothing() {
        createVaultWithAccountsAndBackup(wipeDevice = true)

        var result: ImportResult? = null
        val importer = VaultImporter(
            keyStore = keyStore(),
            opener = opener,
            prompt = { _, _, _, onResult -> onResult(PromptResult.Cancelled) },
            filesDir = dir,
        )
        result = runImport(importer, backupFile.path, BACKUP_PASSWORD)

        assertEquals(ImportResult.RefusedByUser, result)
        assertFalse("nothing wrapped after a cancelled prompt", wrappedFile.exists())
        assertFalse(vaultPath.exists())
    }

    /**
     * The state the lock screen reports as `VaultMissing`: a usable key whose vault file is gone.
     * Import has to work here — that is exactly the case 2.4 sends the user into.
     */
    @Test
    fun importsWhenTheVaultFileIsMissingButTheKeyIsUsable() {
        createVaultWithAccountsAndBackup(wipeDevice = false)
        vaultPath.delete() // key stays usable, the file is gone
        assertEquals(VaultKeyStore.State.Ready, keyStore().state())

        var result: ImportResult? = null
        result = runImport(importerThatSucceeds(), backupFile.path, BACKUP_PASSWORD)

        val imported = importedOrFail(result)
        try {
            assertEquals(2, imported.vault.accountCount)
        } finally {
            imported.vault.close()
        }
    }

    /** Biometric enrollment invalidated the key: the import replaces the dead key instead of failing. */
    @Test
    fun importsOverAnInvalidatedKey() {
        createVaultWithAccountsAndBackup(wipeDevice = false)
        deleteAlias() // key gone, vault file still there
        assertEquals(VaultKeyStore.State.Invalidated, keyStore().state())

        var result: ImportResult? = null
        result = runImport(importerThatSucceeds(), backupFile.path, BACKUP_PASSWORD)

        val imported = importedOrFail(result)
        try {
            assertEquals(2, imported.vault.accountCount)
        } finally {
            imported.vault.close()
        }
        assertEquals(VaultKeyStore.State.Ready, keyStore().state())
    }

    @Test
    fun refusesToReplaceAnExistingVault() {
        // Device keeps its key and vault here: an import must not overwrite them silently.
        createVaultWithAccountsAndBackup(wipeDevice = false)
        assertEquals(VaultKeyStore.State.Ready, keyStore().state())

        var prompted = false
        var result: ImportResult? = null
        val importer = VaultImporter(
            keyStore = keyStore(),
            opener = opener,
            prompt = { cipher, _, _, onResult ->
                prompted = true
                onResult(PromptResult.Succeeded(cipher))
            },
            filesDir = dir,
        )
        result = runImport(importer, backupFile.path, BACKUP_PASSWORD)

        assertTrue("must refuse, was $result", result is ImportResult.Failed)
        assertFalse("no prompt for a refused import", prompted)
        assertEquals("existing vault stays usable", VaultKeyStore.State.Ready, keyStore().state())
    }

    @Test
    fun missingBackupFileIsReported() {
        var result: ImportResult? = null
        result = runImport(importerThatSucceeds(), File(dir, "gibts-nicht.gcbackup").path, BACKUP_PASSWORD)

        assertTrue("missing file must be reported, was $result", result is ImportResult.Failed)
    }

    /**
     * An `Error` on the prompt path — in practice `UnsatisfiedLinkError` when the core is missing —
     * must still end the import. A result that never arrives would leave the lock screen waiting
     * and `running` on `true` for the rest of the process.
     */
    @Test
    fun errorFromThePromptStillEndsTheImport() {
        createVaultWithAccountsAndBackup(wipeDevice = true)

        val importer = VaultImporter(
            keyStore = keyStore(),
            opener = opener,
            prompt = { _, _, _, _ -> throw AssertionError("kein nativer Kern geladen") },
            filesDir = dir,
        )
        val result = runImport(importer, backupFile.path, BACKUP_PASSWORD)

        assertTrue("an Error must end the import, was $result", result is ImportResult.Failed)
    }

    // --- helpers ---------------------------------------------------------------------------

    private fun importerThatSucceeds() = VaultImporter(
        keyStore = keyStore(),
        opener = opener,
        prompt = { cipher, _, _, onResult -> onResult(PromptResult.Succeeded(cipher)) },
        filesDir = dir,
    )

    /**
     * Builds a vault with two accounts and writes the backup. With [wipeDevice] the device key and
     * the vault file are removed afterwards, which is the state a freshly installed app is in.
     */
    private fun createVaultWithAccountsAndBackup(wipeDevice: Boolean) {
        val store = keyStore()
        val key = ByteArray(NativeVaultOpener.MASTER_KEY_BYTES).also { SecureRandom().nextBytes(it) }
        store.storeWrapped(key, store.setupCipher())

        val vault = handleOrFail(opener.open(vaultPath.path, key))
        try {
            vault.createOfflineAccount("Girokonto Test", "CHECKING", "EUR", 123_456)
            vault.createOfflineAccount("Sparkonto Test", "SAVINGS", "EUR", 654_321)
            assertEquals(2, vault.accountCount)
            vault.writeBackup(backupFile.path, BACKUP_PASSWORD)
        } finally {
            vault.close()
        }
        assertTrue("backup file has to exist", backupFile.length() > 0)

        if (wipeDevice) {
            deleteAlias()
            vaultPath.delete()
            wrappedFile.delete()
        }
    }

    /**
     * The importer delivers on Main and does the native work on its own thread, so the test has to
     * wait for the result instead of reading it right after the call.
     */
    private fun runImport(importer: VaultImporter, path: String, password: String): ImportResult? {
        val latch = CountDownLatch(1)
        var result: ImportResult? = null
        importer.import(path, password) {
            result = it
            latch.countDown()
        }
        assertTrue("Import lieferte kein Ergebnis", latch.await(60, TimeUnit.SECONDS))
        return result
    }

    private fun importedOrFail(result: ImportResult?): ImportResult.Imported =
        result as? ImportResult.Imported
            ?: throw AssertionError("expected an imported vault, was $result")

    private fun handleOrFail(vault: OwnVault?): OwnVault =
        vault ?: throw AssertionError("could not create the source vault (core not linked?)")

    private fun deleteAlias() {
        val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (keyStore.containsAlias(alias)) keyStore.deleteEntry(alias)
    }

    companion object {
        private const val BACKUP_PASSWORD = "streng-geheim-42"
    }
}
