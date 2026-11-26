package de.girocrab.app.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDate

/** Blank patterns: a regex keeps its whitespace like the core; the string kinds treat it as missing. */
class RuleBlankPatternTest {
    private fun booking(counterparty: String, cents: Long = -1_000) = Booking(
        id = "b", accountId = "a", bookingDate = LocalDate.of(2026, 11, 2),
        amountCents = cents, counterparty = counterparty, purpose = "",
    )

    private fun rule(
        kind: MatchKind,
        pattern: String,
        min: Long? = null,
        max: Long? = null,
        category: String? = "c",
        tags: Set<String> = emptySet(),
    ) = Rule(
        id = "r", field = RuleField.Counterparty, matchKind = kind, pattern = pattern,
        categoryId = category, tagIds = tags, minCents = min, maxCents = max,
    )

    @Test
    fun whitespaceRegexMatchesALiteralSpace() {
        val r = rule(MatchKind.Regex, " ")
        assertTrue(r.evaluableHere)
        assertTrue(r.matches(booking("REWE Markt")))
        assertFalse(r.matches(booking("REWE")))
    }

    @Test
    fun blankContainsWithRangeMatchesOnTheSignedRangeOnly() {
        val r = rule(MatchKind.Contains, "  ", min = -2_000, max = -500)
        assertTrue(r.matches(booking("x", -1_000)))
        assertFalse(r.matches(booking("x", 1_000)))
    }

    @Test
    fun blankKeywordNeverMatchesEvenWithRange() {
        assertFalse(rule(MatchKind.Keyword, " ", min = -2_000, max = 0).matches(booking("x")))
    }

    @Test
    fun tagOnlyRuleUpdatesTagsButSetsNoCategory() {
        val tagOnly = rule(MatchKind.Contains, "REWE", category = null, tags = setOf("t"))
        val out = AppViewModel.applyRulesTo(listOf(tagOnly), booking("REWE"))
        assertNull(out.categoryId)
        assertEquals(setOf("t"), out.tagIds)
    }
}
