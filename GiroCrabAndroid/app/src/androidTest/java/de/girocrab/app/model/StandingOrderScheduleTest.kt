package de.girocrab.app.model

import java.time.LocalDate
import java.time.temporal.ChronoUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * 3.2b: a standing order's dates come from the core (`first_date`, `last_date`, `exec_day`,
 * `time_unit`/`interval`). The next run is derived, so a bank fetch changes the plan without a
 * migration, and a clamped month end cannot drift.
 */
class StandingOrderScheduleTest {

    private fun order(
        unit: String = "M",
        every: Int = 1,
        execDay: Int = 1,
        first: String = "2024-01-01",
        last: String? = null,
    ) = StandingOrder(
        id = "so",
        accountId = "a",
        recipient = "Hausverwaltung",
        iban = "DE44500105175407324931",
        amountCents = -98_000,
        purpose = "Miete",
        unit = unit,
        every = every,
        execDay = execDay,
        firstDate = LocalDate.parse(first),
        lastDate = last?.let(LocalDate::parse),
    )

    @Test
    fun monthlyRunFallsOnTheExecDay() {
        val o = order(execDay = 15, first = "2024-01-15")
        assertEquals(LocalDate.of(2026, 11, 15), o.nextRun(LocalDate.of(2026, 11, 1)))
        assertEquals(LocalDate.of(2026, 12, 15), o.nextRun(LocalDate.of(2026, 11, 20)))
    }

    @Test
    fun monthEndDoesNotDrift() {
        val o = order(execDay = 31, first = "2024-01-31")
        assertEquals(LocalDate.of(2026, 2, 28), o.nextRun(LocalDate.of(2026, 2, 1)))
        assertEquals(LocalDate.of(2026, 3, 31), o.nextRun(LocalDate.of(2026, 3, 1)))
    }

    @Test
    fun quarterlyRhythmCountsFromTheFirstDate() {
        val o = order(every = 3, first = "2024-01-01")
        assertEquals(LocalDate.of(2026, 10, 1), o.nextRun(LocalDate.of(2026, 9, 2)))
    }

    @Test
    fun weeklyRhythmKeepsItsStep() {
        val o = order(unit = "W", every = 2, first = "2026-01-05")
        val next = o.nextRun(LocalDate.of(2026, 11, 25))!!
        assertTrue(next.isAfter(LocalDate.of(2026, 11, 25)))
        assertEquals(LocalDate.of(2026, 1, 5).dayOfWeek, next.dayOfWeek)
        val following = o.after(next)!!
        assertEquals(14L, ChronoUnit.DAYS.between(next, following))
    }

    @Test
    fun endedOrderHasNoNextRun() {
        val o = order(last = "2026-06-01")
        assertEquals(LocalDate.of(2026, 6, 1), o.nextRun(LocalDate.of(2026, 5, 20)))
        assertNull(o.nextRun(LocalDate.of(2026, 6, 2)))
        assertNull(o.after(LocalDate.of(2026, 6, 1)))
    }

    @Test
    fun firstRunInTheFutureIsTheFirstRun() {
        assertEquals(LocalDate.of(2027, 1, 1), order(first = "2027-01-01").nextRun(LocalDate.of(2026, 11, 25)))
    }

    @Test
    fun rhythmLabelCoversTheIntervalsTheCoreStores() {
        assertEquals("wöchentlich", order(unit = "W", every = 1).rhythm)
        assertEquals("alle 2 Wochen", order(unit = "W", every = 2).rhythm)
        assertEquals("monatlich", order(every = 1).rhythm)
        assertEquals("vierteljährlich", order(every = 3).rhythm)
        assertEquals("jährlich", order(every = 12).rhythm)
        assertEquals("alle 2 Monate", order(every = 2).rhythm)
    }
}
