package de.girocrab.app.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.LocalDate

/** Rule application and matching must give the core's result (`girocrab-core/src/rules.rs`). */
class RuleApplyTest {
    private fun booking(counterparty: String = "REWE Markt", amount: Long = -1_250, categoryId: String? = null) =
        Booking(
            id = "b1", accountId = "a1", bookingDate = LocalDate.of(2026, 3, 1),
            amountCents = amount, counterparty = counterparty, purpose = "", categoryId = categoryId,
        )

    private fun rule(
        id: String,
        pattern: String,
        categoryId: String?,
        tags: Set<String> = emptySet(),
        kind: MatchKind = MatchKind.Keyword,
        min: Long? = null,
        max: Long? = null,
    ) = Rule(id, RuleField.Counterparty, kind, pattern, categoryId, tags, minCents = min, maxCents = max)

    @Test
    fun tagOnlyRuleFirstDoesNotHideTheCategoryRule() {
        val out = AppViewModel.applyRulesTo(
            listOf(rule("r1", "REWE", null, setOf("t1")), rule("r2", "REWE", "c_food")),
            booking(),
        )
        assertEquals("c_food", out.categoryId)
        assertTrue(out.categoryByRule)
        assertEquals(setOf("t1"), out.tagIds)
    }

    @Test
    fun tagsComeFromEveryMatchingRuleAndKeepManualCategory() {
        val b = booking(categoryId = "c_manual")
        val out = AppViewModel.applyRulesTo(
            listOf(rule("r1", "REWE", "c_food", setOf("t3")), rule("r2", "Markt", null, setOf("t9"))),
            b,
        )
        assertEquals("c_manual", out.categoryId)
        assertFalse(out.categoryByRule)
        assertEquals(setOf("t3", "t9"), out.tagIds)
    }

    @Test
    fun tagOnlyRuleDoesNotMarkUncategorizedBookingAsRuleCategorized() {
        val out = AppViewModel.applyRulesTo(listOf(rule("r1", "REWE", null, setOf("t1"))), booking())
        assertNull(out.categoryId)
        assertFalse(out.categoryByRule)
        assertEquals(setOf("t1"), out.tagIds)
    }

    @Test
    fun unchangedBookingIsReturnedAsIs() {
        val b = booking(categoryId = "c_manual")
        assertSame(b, AppViewModel.applyRulesTo(listOf(rule("r1", "REWE", "c_food")), b))
        val untagged = booking(counterparty = "Bäckerei")
        assertSame(untagged, AppViewModel.applyRulesTo(listOf(rule("r1", "REWE", null, setOf("t1"))), untagged))
    }

    @Test
    fun emptyKeywordListNeverMatchesEvenWithRange() {
        val b = booking()
        assertFalse(rule("r1", "", "c", min = -5_000, max = 0).matches(b))
        assertFalse(rule("r1", " | , ", "c", min = -5_000, max = 0).matches(b))
        // The app's own kinds keep matching on the range alone.
        assertTrue(rule("r1", "", "c", kind = MatchKind.Contains, min = -5_000, max = 0).matches(b))
    }

    @Test
    fun rustNamedGroupsMatchAndInvalidPatternsAreReported() {
        val named = rule("r1", "(?P<shop>rewe)", "c", kind = MatchKind.Regex)
        assertTrue(named.evaluableHere)
        assertTrue(named.matches(booking()))

        val broken = rule("r2", "(REWE", "c", kind = MatchKind.Regex)
        assertFalse(broken.evaluableHere)
        assertFalse(broken.matches(booking()))

        assertFalse(rule("r3", "betrag > 10", "c", kind = MatchKind.Expression).evaluableHere)
        assertTrue(rule("r4", "REWE", "c").evaluableHere)
    }
}
