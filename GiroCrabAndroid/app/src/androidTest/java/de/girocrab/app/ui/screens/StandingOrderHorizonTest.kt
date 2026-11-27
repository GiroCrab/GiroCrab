package de.girocrab.app.ui.screens

import de.girocrab.app.model.StandingOrder
import de.girocrab.app.model.StandingOrderRun
import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 3.2b: the planned tab lists the core's standing-order runs, cut to the chosen horizon. A run that
 * already happened must not show up as planned — a snapshot loaded before midnight still carries
 * yesterday's row.
 */
class StandingOrderHorizonTest {

    private val today = LocalDate.parse("2026-11-26")

    private fun order() = StandingOrder(
        id = "so1",
        accountId = "a1",
        recipient = "Hausverwaltung",
        iban = "DE44500105175407324931",
        amountCents = -98_000,
        purpose = "Miete",
        unit = "M",
        every = 1,
        execDay = 1,
        firstDate = LocalDate.parse("2024-01-01"),
        lastDate = null,
    )

    private fun dates(vararg runs: String) = expandStandingOrders(
        orders = listOf(order()),
        runs = runs.map { StandingOrderRun("so1", LocalDate.parse(it)) },
        today = today,
        horizon = PlannedHorizon.D30,
    ).map { it.date.toString() }

    @Test
    fun runsBeforeTodayAreNotPlanned() {
        assertEquals(listOf("2026-12-01"), dates("2026-11-25", "2026-12-01"))
    }

    @Test
    fun runsBeyondTheHorizonAreDropped() {
        assertEquals(listOf("2026-11-30", "2026-12-24"), dates("2026-11-30", "2026-12-24", "2027-02-01"))
    }

    @Test
    fun anOrderThatIsNotInTheSnapshotContributesNothing() {
        val foreign = expandStandingOrders(
            orders = emptyList(),
            runs = listOf(StandingOrderRun("so1", today)),
            today = today,
            horizon = PlannedHorizon.D30,
        )
        assertEquals(0, foreign.size)
    }
}
