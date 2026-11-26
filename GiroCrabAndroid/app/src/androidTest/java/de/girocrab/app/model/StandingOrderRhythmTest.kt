package de.girocrab.app.model

import java.time.LocalDate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * 3.2b: the rhythm label of a standing order and what the card says without a run. The execution
 * dates themselves come from the core (list_planned_payments); this class only checks how the stored
 * unit/interval are shown and that a missing run is not silently called "ended".
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

    /** A run inside the window needs no note; the card prints the date. */
    @Test
    fun aReportedRunNeedsNoNote() {
        val o = order().copy(nextDate = LocalDate.parse("2026-12-01"))
        assertNull(o.noNextRunNote(LocalDate.parse("2026-11-26")))
    }

    /** Only a bank-reported last execution in the past makes an order ended. */
    @Test
    fun onlyAReportedLastExecutionInThePastMeansEnded() {
        val today = LocalDate.parse("2026-11-26")
        val ended = order().copy(lastDate = LocalDate.parse("2026-06-01"))
        assertEquals("keine (beendet)", ended.noNextRunNote(today))
        // Last execution today is not "in the past" — the run may still come.
        assertEquals("kein Termin im Zeitfenster", ended.copy(lastDate = today).noNextRunNote(today))
    }

    /** An open order without a run in the 400-day window is not ended. */
    @Test
    fun anOpenOrderWithoutRunIsNotCalledEnded() {
        assertEquals("kein Termin im Zeitfenster", order().noNextRunNote(LocalDate.parse("2026-11-26")))
    }
}
