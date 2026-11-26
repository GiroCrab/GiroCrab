package de.girocrab.app.model

import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * 3.2b: the rhythm label of a standing order. The execution dates themselves come from the core
 * (list_planned_payments); this class only checks how the stored unit/interval are shown.
 */
class StandingOrderRhythmTest {

    private fun order(unit: String = "M", every: Int = 1) = StandingOrder(
        id = "so",
        accountId = "a",
        recipient = "Hausverwaltung",
        iban = "DE44500105175407324931",
        amountCents = -98_000,
        purpose = "Miete",
        unit = unit,
        every = every,
        execDay = 1,
        firstDate = LocalDate.parse("2024-01-01"),
        lastDate = null,
    )

    @Test
    fun rhythmLabelCoversTheIntervalsTheCoreStores() {
        assertEquals("wöchentlich", order(unit = "W", every = 1).rhythm)
        assertEquals("alle 2 Wochen", order(unit = "W", every = 2).rhythm)
        assertEquals("monatlich", order(every = 1).rhythm)
        assertEquals("vierteljährlich", order(every = 3).rhythm)
        assertEquals("jährlich", order(every = 12).rhythm)
        assertEquals("alle 2 Monate", order(every = 2).rhythm)
    }

    @Test
    fun zeroEveryCountsAsOneMonth() {
        assertEquals("monatlich", order(every = 0).rhythm)
    }
}
