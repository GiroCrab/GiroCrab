package de.girocrab.app.ui.screens

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Proof that the analytics screen states an incomplete booking set instead of summing it silently.
 */
@RunWith(AndroidJUnit4::class)
class IncompleteBookingsNoteTest {

    @Test
    fun completeListProducesNoNote() {
        assertNull(incompleteBookingsNote(loaded = 1_000, total = 1_000))
    }

    @Test
    fun emptyVaultProducesNoNote() {
        assertNull(incompleteBookingsNote(loaded = 0, total = 0))
    }

    @Test
    fun pageCapProducesNoteWithBothNumbers() {
        val note = incompleteBookingsNote(loaded = 100_000, total = 120_345)
        assertTrue(note != null)
        assertTrue(note!!.contains("100000 von 120345"))
        assertTrue(note.contains("nur auf die geladenen Buchungen"))
    }

    @Test
    fun totalBelowLoadedRowsProducesNoNote() {
        assertNull(incompleteBookingsNote(loaded = 8, total = 5))
    }
}
