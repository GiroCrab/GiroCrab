package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import de.girocrab.app.security.NativeVaultOpener
import de.girocrab.app.security.OwnVault
import java.io.File
import java.time.LocalDate
import uniffi.girocrab_core.AssetInterpolation
import uniffi.girocrab_core.AssetItem as CoreAssetItem
import uniffi.girocrab_core.AssetItemSetup as CoreAssetSetup
import uniffi.girocrab_core.BookingPage as CoreBookingPage
import uniffi.girocrab_core.Holding as CoreHolding
import uniffi.girocrab_core.Mandate as CoreMandate
import uniffi.girocrab_core.MandateScheme as CoreMandateScheme
import uniffi.girocrab_core.SelectionFlow as CoreFlow
import java.security.SecureRandom
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
        // Whole days let the core return more rows than the limit — the page has to stay capped.
        assertFalse("a capped page must not be extended to whole days", filter.wholeDays)
    }

    /**
     * #1429 (3.2): the core keeps the EUR value of a holding, the app recomputes it. The mapper
     * derives the factor from the core's numbers, so both sides land on the same value — the test
     * checks the app's own arithmetic, not the copied value.
     */
    @Test
    fun holdingsReproduceTheCoreValueInTheAppsOwnArithmetic() {
        val row = CoreHolding(
            isin = "US0378331005",
            wkn = "865985",
            name = "Apple Inc.",
            quantity = 10.0,
            currentPriceCents = 5_000,
            purchasePriceCents = 4_000,
            valueCents = 52_000,
            profitLossPurchaseCents = 2_000,
            currency = "USD",
            nativePriceCents = 5_200,
            nativePurchasePriceCents = 4_100,
        )

        val snapshot = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            page = emptyPage(),
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = mapOf(7L to listOf(row)),
            assets = emptyList(),
        )

        val holding = snapshot.holdings.single()
        assertEquals("the account has to come from the loop's side", "7", holding.accountId)
        assertEquals(5_000L, holding.priceCents)
        assertEquals("cost basis is what was paid: value minus gain", 50_000L, holding.costBasisCents)
        assertEquals(1.04, holding.fxRateToEur, 1e-9)
        assertEquals(
            "the app's own valueCents has to hit the core's EUR value",
            52_000L,
            holding.valueCents,
        )
        // The core reports prices in the account's currency and keeps the position's own one next
        // to it (#625) — labelling an EUR amount as "USD" was the wrong way round.
        assertEquals("quote currency is the account's", "EUR", holding.quoteCurrency)
        assertEquals("native code survives the mapping", "USD", holding.currency)
        assertEquals("the native price comes from the core", 5_200L, holding.nativePriceCents)
        assertEquals("so does the native purchase price", 4_100L, holding.nativePurchasePriceCents)
        assertEquals("printed in the native currency", "USD", holding.nativeCurrency)
    }

    /** A position without a native figure must not invent one — the price line stays bare. */
    @Test
    fun holdingWithoutNativePriceStaysWithoutOne() {
        val row = CoreHolding(
            isin = "DE0007164600",
            wkn = "716460",
            name = "SAP SE",
            quantity = 5.0,
            currentPriceCents = 10_000,
            purchasePriceCents = 9_000,
            valueCents = 50_000,
            currency = "",
        )

        val snapshot = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            page = emptyPage(),
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = mapOf(7L to listOf(row)),
            assets = emptyList(),
        )

        val holding = snapshot.holdings.single()
        assertEquals("blank native currency means the account's", "EUR", holding.nativeCurrency)
        assertNull("no native price in the core, none in the app", holding.nativePurchasePriceCents)
        assertNull(holding.nativePriceCents)
    }

    /** #1429 (3.2): a tangible asset carries the setup's name and date and the holding's value. */
    @Test
    fun tangibleAssetsCarryTheSetupAndTheHoldingValue() {
        val item = CoreAssetItem(
            id = 3,
            accountId = 9,
            setup = CoreAssetSetup(
                name = "Goldbarren",
                priceSource = "XAU",
                weightGrams = 31.1035,
                purityPermille = 999.9,
                interpolation = AssetInterpolation.STEP,
                purchaseDate = "2024-03-01",
                quantity = 1.0,
                purchasePriceCents = 210_000,
                startValueCents = null,
                note = "1 oz",
                symbol = "",
            ),
            holding = CoreHolding(
                isin = "",
                wkn = "",
                name = "Goldbarren",
                quantity = 1.0,
                currentPriceCents = 260_000,
                purchasePriceCents = 210_000,
                valueCents = 260_000,
            ),
            quantities = emptyList(),
            values = emptyList(),
            valueDate = "2026-11-22",
            missingPriceDate = null,
            sold = false,
        )

        val snapshot = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            page = emptyPage(),
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = emptyMap(),
            assets = listOf(item),
        )

        val asset = snapshot.assets.single()
        assertEquals("3", asset.id)
        assertEquals("Goldbarren", asset.name)
        assertEquals(260_000L, asset.valueCents)
        assertEquals(LocalDate.of(2024, 3, 1), asset.acquiredDate)
        assertEquals("1 oz", asset.note)
    }

    /** A truncated page has to say how much the core really holds — not how much arrived. */
    @Test
    fun truncatedPageKeepsTheCoreTotal() {
        val snapshot = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            page = emptyPage(total = 5_000u),
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = emptyMap(),
            assets = emptyList(),
        )

        assertEquals("the page may hold fewer rows, but the total must survive", 5_000, snapshot.bookingsTotal)
        assertTrue("empty page is smaller than the total", snapshot.bookings.isEmpty())
    }

    /** A derived rate lands on x.9999… easily; truncating would lose a cent per position. */
    @Test
    fun holdingValueIsRoundedNotTruncated() {
        val holding = Holding(
            id = "1/DE000",
            accountId = "1",
            name = "Test",
            isin = "DE000",
            quantity = 3.0,
            priceCents = 333,
            costBasisCents = 900,
            fxRateToEur = 1.001,
        )

        assertEquals("3 × 333 × 1,001 = 999,999 — rounded to 1000", 1_000L, holding.valueCents)
    }

    // --- helpers ---------------------------------------------------------------------------

    private fun newKey() = ByteArray(NativeVaultOpener.MASTER_KEY_BYTES).also { SecureRandom().nextBytes(it) }

    private fun open(key: ByteArray) =
        opener.open(vaultPath.path, key) ?: throw AssertionError("native core is not linked")

    private fun emptyPage(total: UInt = 0u) = CoreBookingPage(
        transactions = emptyList(),
        totalCount = total,
        incomeCents = 0,
        expenseCents = 0,
        consumptionCents = 0,
        investmentCents = 0,
        perkCents = 0,
        flow = CoreFlow(consumptionCents = 0, investedCents = 0, incomeCents = 0),
    )

    /** Runs [block] on a fresh handle and closes it afterwards — one place, so no test leaks. */
    private fun <T> withVault(key: ByteArray, block: (OwnVault) -> T): T {
        val vault = open(key)
        return try {
            block(vault)
        } finally {
            vault.close()
        }
    }

    /** 3.2b: a mandate carries the core's fields — debtor, scheme, sequence, last collection. */
    @Test
    fun mandateCarriesWhatTheCoreStores() {
        val m = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            bookings = emptyList(),
            bookingsTotal = 0,
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = emptyMap(),
            assets = emptyList(),
            mandates = listOf(coreMandate()),
        ).mandates.single()

        assertEquals("7", m.id)
        assertEquals("3", m.accountId)
        assertEquals("Vattenfall Europe Sales", m.debtorName)
        assertEquals("DE12345678901234567890", m.debtorIban)
        assertEquals("VAT-2024-0042", m.reference)
        assertEquals(LocalDate.of(2024, 3, 1), m.signedDate)
        assertEquals(MandateScheme.Core, m.scheme)
        assertFalse(m.oneOff)
        assertEquals(8_900L, m.defaultAmountCents)
        assertTrue(m.active)
        assertEquals(LocalDate.of(2026, 10, 5), m.lastCollected)
        assertEquals("RCUR", m.nextSequence)
    }

    /** A mandate without a collection keeps `null`, not a made-up date; the scheme survives. */
    @Test
    fun mandateWithoutCollectionKeepsNoDate() {
        val m = VaultSnapshotMapper.map(
            groups = emptyList(),
            accounts = emptyList(),
            bookings = emptyList(),
            bookingsTotal = 0,
            categories = emptyList(),
            tags = emptyList(),
            holdingsByAccount = emptyMap(),
            assets = emptyList(),
            mandates = listOf(coreMandate(lastCollected = null, scheme = CoreMandateScheme.B2B, oneOff = true, nextSequence = "OOFF")),
        ).mandates.single()

        assertNull(m.lastCollected)
        assertEquals(MandateScheme.B2b, m.scheme)
        assertTrue(m.oneOff)
        assertEquals("OOFF", m.nextSequence)
    }

    private fun coreMandate(
        lastCollected: String? = "2026-10-05",
        scheme: CoreMandateScheme = CoreMandateScheme.CORE,
        oneOff: Boolean = false,
        nextSequence: String = "RCUR",
    ) = CoreMandate(
        id = 7,
        accountId = 3,
        debtorName = "Vattenfall Europe Sales",
        debtorIban = "DE12345678901234567890",
        debtorBic = "",
        reference = "VAT-2024-0042",
        signedDate = "2024-03-01",
        scheme = scheme,
        oneOff = oneOff,
        defaultAmountCents = 8_900,
        defaultPurpose = "Abschlag Strom",
        note = "",
        active = true,
        lastCollected = lastCollected,
        nextSequence = nextSequence,
    )

    private fun read(key: ByteArray): VaultSnapshot {
        val vault = open(key)
        return try {
            vault.snapshot()
        } finally {
            vault.close()
        }
    }
}
