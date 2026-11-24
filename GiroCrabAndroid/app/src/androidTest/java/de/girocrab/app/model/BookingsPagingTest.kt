package de.girocrab.app.model

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Proof for #1429 (3.3): the booking list is read to the end, the page cap reports what is
 * missing, and a counter that disagrees with the data cannot produce an endless loop.
 *
 * Pure logic on purpose — no vault, no core: the helper decides where the next page starts, and
 * that decision is what the vault-facing test cannot check cheaply.
 */
@RunWith(AndroidJUnit4::class)
class BookingsPagingTest {

    private fun page(from: Int, count: Int, total: Int) =
        BookingsPaging.Page((from until from + count).toList(), total)

    @Test
    fun readsEveryPageUntilTheCoresCountIsCovered() {
        val offsets = mutableListOf<Int>()
        val coverage = BookingsPaging.readAll(pageSize = 10) { offset ->
            offsets += offset
            when (offset) {
                0 -> page(0, 10, 25)
                10 -> page(10, 10, 25)
                else -> page(20, 5, 25)
            }
        }
        assertEquals("every row of the list has to arrive", (0 until 25).toList(), coverage.rows)
        assertEquals(25, coverage.totalCount)
        assertFalse("25 of 25 is not truncated", coverage.truncated)
        assertEquals("pages are asked for back to back", listOf(0, 10, 20), offsets)
    }

    @Test
    fun stopsAtThePageCapAndSaysWhatIsMissing() {
        val coverage = BookingsPaging.readAll(pageSize = 10, maxPages = 2) { offset ->
            page(offset, 10, 500)
        }
        assertEquals("two pages, no more", 20, coverage.rows.size)
        assertEquals(500, coverage.totalCount)
        assertTrue("the list has to be able to say 20 of 500", coverage.truncated)
    }

    @Test
    fun anEmptyPageEndsTheReadEvenWhenTheCounterClaimsMore() {
        var calls = 0
        val coverage = BookingsPaging.readAll(pageSize = 10) { offset ->
            calls++
            if (offset == 0) page(0, 10, 500) else BookingsPaging.Page(emptyList(), 500)
        }
        assertEquals("an empty page stops the loop", 2, calls)
        assertEquals(10, coverage.rows.size)
        assertTrue(coverage.truncated)
    }

    @Test
    fun oneHeadIsEnoughWhenTheCountMatchesTheRows() {
        var calls = 0
        val coverage = BookingsPaging.readAll(pageSize = 10) { offset ->
            calls++
            page(offset, 3, 3)
        }
        assertEquals(1, calls)
        assertEquals(3, coverage.rows.size)
        assertFalse(coverage.truncated)
    }

    @Test
    fun aCounterBelowTheRowsCannotProduceANegativeGap() {
        val coverage = BookingsPaging.readAll(pageSize = 10) { offset ->
            page(offset, 5, 2)
        }
        assertEquals("what arrived is the lower bound", 5, coverage.totalCount)
        assertFalse(coverage.truncated)
    }
}
