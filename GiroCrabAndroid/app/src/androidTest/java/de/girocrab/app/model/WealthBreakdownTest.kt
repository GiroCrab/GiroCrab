package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import de.girocrab.app.ui.screens.wealthBreakdown
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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
        currency: String = "EUR",
    ) = Account(
        id = id,
        groupId = "g1",
        bankName = "Testbank",
        name = id,
        iban = null,
        kind = kind,
        balanceCents = cents,
        includeInTotal = includeInTotal,
        currency = currency,
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

    /** A position in an account the core excluded must not sneak into the depot sum. */
    @Test
    fun holdingsOfExcludedAccountsStayOut() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(account("depot", AccountKind.Depot, 10_000, includeInTotal = false)),
                holdings = listOf(
                    Holding(
                        id = "h1",
                        accountId = "depot",
                        name = "SAP SE",
                        isin = "DE0007164600",
                        quantity = 5.0,
                        priceCents = 2_000,
                        costBasisCents = 9_000,
                        quoteCurrency = "EUR",
                    ),
                ),
            ),
        )

        assertEquals(0L, wealthBreakdown(state).depot)
        assertEquals(0L, wealthBreakdown(state).assets)
    }

    /** A hidden account is excluded the same way — and its asset must not appear as a standalone one. */
    @Test
    fun assetOfAnExcludedAccountStaysOut() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    account("giro", AccountKind.Giro, 100_00),
                    account("versteckt", AccountKind.Giro, 0, includeInTotal = false),
                ),
                assets = listOf(
                    TangibleAsset(
                        id = "a1",
                        name = "Goldbarren",
                        valueCents = 50_00,
                        acquiredDate = LocalDate.of(2024, 3, 1),
                        accountId = "versteckt",
                    ),
                ),
            ),
        )

        assertEquals("only the included account counts", 100_00L, wealthBreakdown(state).assets)
        assertEquals("its asset is not added on top", 0L, wealthBreakdown(state).tangibles)
    }

    /** Two currencies are not one number: the sum stays in one currency and names the other. */
    @Test
    fun foreignCurrencyIsNotSummedButReported() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    account("eur", AccountKind.Giro, 100_00),
                    account("usd", AccountKind.Giro, 100_00, currency = "USD"),
                ),
            ),
        )

        val wealth = wealthBreakdown(state)
        assertEquals("the sum stays in one currency", 100_00L, wealth.cash)
        assertEquals("EUR", wealth.currency)
        assertEquals(setOf("USD"), wealth.foreignCurrencies)
    }

    /**
     * An account the user excluded keeps its debt out of the totals too: counting the loan record
     * instead would overrule `includeInTotal` behind the user's back.
     */
    @Test
    fun loanOfAnExcludedAccountStaysOutOfEverySum() {
        fun loan(accountId: String, remaining: Long) = ManualLoan(
            accountId = accountId,
            name = "Autokredit",
            originalCents = 1_800_000,
            anchorDate = LocalDate.parse("2026-01-01"),
            anchorBalanceCents = remaining,
            interestRatePpm = 39_000,
            matchExpr = "",
            remainingCents = remaining,
            asOfDate = null,
            paymentCount = 0,
            paidCents = 0,
            principalCents = 0,
            interestCents = 0,
            beforeAnchorCount = 0,
            lastPaymentCents = null,
            lastPaymentDate = null,
            monthsLeft = null,
        )
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    account("giro", AccountKind.Giro, 100_000),
                    // Excluded from every total: the debt has to come from the loan record now.
                    account("kredit", AccountKind.Loan, -500_000, includeInTotal = false),
                ),
                loans = listOf(loan("kredit", 500_000)),
            ),
        )
        val breakdown = wealthBreakdown(state)
        assertEquals("the excluded account keeps its debt out of the bucket", 0L, breakdown.loans)
        assertEquals("and not in cash", 100_000L, breakdown.cash)
        val vm = AppViewModel()
        vm.adoptOwnVault(state.data)
        assertEquals("net worth and the buckets must agree", breakdown.net, vm.netWorthCents("EUR"))
        assertEquals(100_000L, vm.netWorthCents("EUR"))
    }

    /** No conversion happens anywhere, so a USD debt must not be subtracted from a EUR total. */
    @Test
    fun foreignCurrencyLoanStaysOutOfTheEuroTotal() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    // The euro account has to carry the primary currency, otherwise the breakdown
                    // would be built in USD and this test would prove nothing.
                    account("giro", AccountKind.Giro, 2_000_000),
                    account("kredit", AccountKind.Loan, -500_000, currency = "USD"),
                ),
                loans = listOf(
                    ManualLoan(
                        accountId = "kredit",
                        name = "Auto loan",
                        originalCents = null,
                        anchorDate = LocalDate.parse("2026-01-01"),
                        anchorBalanceCents = 500_000,
                        interestRatePpm = null,
                        matchExpr = "",
                        remainingCents = 500_000,
                        asOfDate = null,
                        paymentCount = 0,
                        paidCents = 0,
                        principalCents = 0,
                        interestCents = 0,
                        beforeAnchorCount = 0,
                        lastPaymentCents = null,
                        lastPaymentDate = null,
                        monthsLeft = null,
                    ),
                ),
            ),
        )
        val breakdown = wealthBreakdown(state)
        assertEquals("EUR", breakdown.currency)
        assertEquals(0L, breakdown.loans)
        assertEquals(2_000_000L, breakdown.net)
        assertTrue("the screen has to report the other currency", "USD" in breakdown.foreignCurrencies)
        val vm = AppViewModel()
        vm.adoptOwnVault(state.data)
        assertEquals(2_000_000L, vm.netWorthCents("EUR"))
    }

    /** A counted loan account carries the debt itself; its record must not be subtracted again. */
    @Test
    fun loanOfACountedAccountIsNotCountedTwice() {
        val state = AppUiState(
            data = VaultSnapshot(
                accounts = listOf(
                    account("giro", AccountKind.Giro, 1_000_000),
                    account("kredit", AccountKind.Loan, -500_000),
                ),
                loans = listOf(
                    ManualLoan(
                        accountId = "kredit",
                        name = "Autokredit",
                        originalCents = null,
                        anchorDate = LocalDate.parse("2026-01-01"),
                        anchorBalanceCents = 500_000,
                        interestRatePpm = null,
                        matchExpr = "",
                        remainingCents = 500_000,
                        asOfDate = null,
                        paymentCount = 0,
                        paidCents = 0,
                        principalCents = 0,
                        interestCents = 0,
                        beforeAnchorCount = 0,
                        lastPaymentCents = null,
                        lastPaymentDate = null,
                        monthsLeft = null,
                    ),
                ),
            ),
        )
        val breakdown = wealthBreakdown(state)
        assertEquals(500_000L, breakdown.loans)
        assertEquals(500_000L, breakdown.net)
        val vm = AppViewModel()
        vm.adoptOwnVault(state.data)
        assertEquals(500_000L, vm.netWorthCents("EUR"))
    }

    /** The demo's loan account carries exactly the debt of its record — no gap, no double count. */
    @Test
    fun theDemoLoanAccountCarriesExactlyItsDebt() {
        val d = DemoData.build()
        val loan = d.loans.single()
        val acc = d.accounts.single { it.kind == AccountKind.Loan }
        assertEquals(loan.accountId, acc.id)
        assertEquals(-loan.remainingCents, acc.balanceCents)
    }
}
