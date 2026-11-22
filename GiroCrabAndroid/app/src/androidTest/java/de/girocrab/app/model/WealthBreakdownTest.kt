package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import de.girocrab.app.ui.screens.wealthBreakdown
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The wealth figures the screen shows. Two rules are easy to get wrong and invisible until someone
 * adds up the numbers by hand: an account the core excluded from its totals must not appear in
 * these sums, and a tangible asset sits in an account that already carries its value.
 */
@RunWith(AndroidJUnit4::class)
class WealthBreakdownTest {

    private fun account(
        id: String,
        kind: AccountKind,
        cents: Long,
        includeInTotal: Boolean = true,
    ) = Account(
        id = id,
        groupId = "g1",
        bankName = "Testbank",
        name = id,
        iban = null,
        kind = kind,
        balanceCents = cents,
        includeInTotal = includeInTotal,
    )

    @Test
    fun excludedAccountsStayOutOfTheSums() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    account("giro", AccountKind.Giro, 100_00),
                    account("ausgeschlossen", AccountKind.Giro, 42_00, includeInTotal = false),
                ),
            ),
        )

        assertEquals("only the included account counts", 100_00L, wealthBreakdown(state).cash)
        assertEquals(100_00L, wealthBreakdown(state).net)
    }

    @Test
    fun tangibleAssetIsCountedOnce() {
        val state = AppUiState(
            data = VaultSnapshot(
                // The depot account's balance already contains the asset.
                accounts = listOf(account("depot", AccountKind.Depot, 100_00)),
                assets = listOf(
                    TangibleAsset(
                        id = "a1",
                        name = "Goldbarren",
                        valueCents = 100_00,
                        acquiredDate = LocalDate.of(2024, 3, 1),
                        accountId = "depot",
                    ),
                ),
            ),
        )

        assertEquals(
            "the asset must not be counted twice (once as account, once as asset)",
            100_00L,
            wealthBreakdown(state).assets,
        )
    }

    @Test
    fun positionOutsideATangibleAccountStillCounts() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(account("depot", AccountKind.Depot, 0)),
                holdings = listOf(
                    Holding(
                        id = "h1",
                        accountId = "depot",
                        name = "SAP SE",
                        isin = "DE0007164600",
                        quantity = 5.0,
                        priceCents = 10_000,
                        costBasisCents = 45_000,
                        quoteCurrency = "EUR",
                    ),
                ),
            ),
        )

        assertEquals("an ordinary holding stays in the depot sum", 50_000L, wealthBreakdown(state).depot)
    }
}
