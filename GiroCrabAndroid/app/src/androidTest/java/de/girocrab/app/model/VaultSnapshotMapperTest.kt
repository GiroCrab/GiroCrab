package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import de.girocrab.app.security.NativeVaultOpener
import de.girocrab.app.security.OwnVault
import java.io.File
import java.security.SecureRandom
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Proof for #1429 (3.1): the snapshot the screens read carries the same accounts and the same sum
 * as the vault the core opened. The fixture vault lives in `test-mapper/`, never in production.
 */
@RunWith(AndroidJUnit4::class)
class VaultSnapshotMapperTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val dir = File(context.filesDir, "test-mapper")
    private val vaultPath = File(File(dir, "vaults"), "girocrab.sqlcipher")
    private val opener = NativeVaultOpener()

    @Before
    fun clean() {
        dir.deleteRecursively()
        dir.mkdirs()
    }

    @After
    fun tearDown() {
        dir.deleteRecursively()
    }

    /**
     * The counter-check the issue asks for: count and sum of the mapped accounts against the
     * accounts the fixture vault was built with. The ids come back from `createOfflineAccount`,
     * so the test compares against the core's own answer, not against a copied expectation.
     */
    @Test
    fun snapshotCarriesTheAccountsOfTheFixtureVault() {
        val key = newKey()
        val ids = withVault(key) { vault ->
            listOf(
                vault.createOfflineAccount("Girokonto Test", "CHECKING", "EUR", 123_456),
                vault.createOfflineAccount("Sparkonto Test", "SAVINGS", "EUR", 654_321),
            )
        }

        val snapshot = read(key)

        assertEquals("every account has to arrive", ids.size, snapshot.accounts.size)
        assertEquals(
            "account ids have to match the core's own ids",
            ids.map { it.toString() }.toSet(),
            snapshot.accounts.map { it.id }.toSet(),
        )
        assertEquals(
            setOf("Girokonto Test", "Sparkonto Test"),
            snapshot.accounts.map { it.name }.toSet(),
        )
        assertEquals(
            "balances have to add up to what was created",
            123_456L + 654_321L,
            snapshot.accounts.sumOf { it.balanceCents },
        )
        assertTrue("a vault always has at least one group", snapshot.groups.isNotEmpty())
        val knownGroups = snapshot.groups.map { it.id }.toSet()
        assertTrue(
            "no account may point at a group that is not in the snapshot",
            snapshot.accounts.all { it.groupId in knownGroups },
        )
        assertTrue(
            "kind has to survive the mapping",
            snapshot.accounts.any { it.kind == AccountKind.Giro } &&
                snapshot.accounts.any { it.kind == AccountKind.Savings },
        )
    }

    /** Mapping is a copy, not a query: reading the same vault twice yields the same snapshot. */
    @Test
    fun twoReadsOfTheSameVaultAgree() {
        val key = newKey()
        withVault(key) { it.createOfflineAccount("Girokonto Test", "CHECKING", "EUR", 42_00) }

        assertEquals(read(key).accounts, read(key).accounts)
    }

    /** The first page asks for everything: no date bound, both directions, no hidden limit. */
    @Test
    fun firstPageFilterAsksForEverything() {
        val filter = VaultSnapshotMapper.firstPageFilter()

        assertTrue("no account filter", filter.accountIds.isEmpty())
        assertEquals(0u, filter.offset)
        assertEquals(VaultSnapshotMapper.PAGE_SIZE, filter.limit)
        assertNull(filter.dateFrom)
        assertNull(filter.dateTo)
        assertNull(filter.checked)
    }

    // --- helpers ---------------------------------------------------------------------------

    private fun newKey() = ByteArray(NativeVaultOpener.MASTER_KEY_BYTES).also { SecureRandom().nextBytes(it) }

    private fun open(key: ByteArray) =
        opener.open(vaultPath.path, key) ?: throw AssertionError("native core is not linked")

    /** Runs [block] on a fresh handle and closes it afterwards — one place, so no test leaks. */
    private fun <T> withVault(key: ByteArray, block: (OwnVault) -> T): T {
        val vault = open(key)
        return try {
            block(vault)
        } finally {
            vault.close()
        }
    }

    private fun read(key: ByteArray): VaultSnapshot {
        val vault = open(key)
        return try {
            vault.snapshot()
        } finally {
            vault.close()
        }
    }
}
