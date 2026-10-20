package com.greekapp.typing

import com.greekapp.ui.components.GreekTypingGrader
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.text.Normalizer

/**
 * Polytonic battery for the typed-Greek grader (Wave 9, Appendix R grading
 * policy): STRICT breathings / iota subscript / diaeresis / final-sigma
 * folding, LENIENT pitch accents. Includes NFD (decomposed) answer forms and
 * final-sigma edge cases.
 */
class GreekTypingGraderTest {

    private fun nfd(s: String): String =
        Normalizer.normalize(s, Normalizer.Form.NFD)

    // ---- LENIENT: pitch accents ----

    @Test
    fun `pitch accents are lenient`() {
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος", "λογος"))
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος", "λογόσ".let { nfd(it) }))
        assertTrue(GreekTypingGrader.isCloseMatch("καιρός", "καὶρός")) // acute vs grave
        assertTrue(GreekTypingGrader.isCloseMatch("ἄνθρωπος", "ἂνθρωπος")) // acute vs grave
        assertTrue(GreekTypingGrader.isCloseMatch("δοῦλος", "δουλος")) // circumflex dropped
    }

    // ---- STRICT: breathings ----

    @Test
    fun `breathings are strict`() {
        // Smooth vs rough breathing must NOT match (rough = /h/, phonemic).
        assertFalse(GreekTypingGrader.isCloseMatch("ἁ", "ἀ"))
        assertFalse(GreekTypingGrader.isCloseMatch("ὁ", "ὀ"))
        // Breathing-present vs breathing-absent must NOT match.
        assertFalse(GreekTypingGrader.isCloseMatch("ἄ", "ά"))
        assertFalse(GreekTypingGrader.isCloseMatch("ἑλίκη", "ελικη"))
        // Same breathing, different pitch accent DOES match.
        assertTrue(GreekTypingGrader.isCloseMatch("ἄ", "ἂ"))
        assertTrue(GreekTypingGrader.isCloseMatch("ἅ", "ἃ"))
    }

    // ---- STRICT: iota subscript and diaeresis ----

    @Test
    fun `iota subscript and diaeresis are strict`() {
        assertFalse(GreekTypingGrader.isCloseMatch("ᾳ", "α"))
        assertFalse(GreekTypingGrader.isCloseMatch("χώρᾳ", "χωρα"))
        assertFalse(GreekTypingGrader.isCloseMatch("ϊ", "ι"))
        // But accents ON a subscripted vowel are lenient.
        assertTrue(GreekTypingGrader.isCloseMatch("ᾴ", "ᾳ"))
        assertTrue(GreekTypingGrader.isCloseMatch("ᾷ", "ᾳ"))
        // And ᾳ vs ά (accent-only) still differs by the subscript.
        assertFalse(GreekTypingGrader.isCloseMatch("ᾳ", "ά"))
    }

    // ---- Final sigma folding (word-final σ→ς on both sides) ----

    @Test
    fun `final sigma edges`() {
        // Learner types medial sigma at word end: still correct.
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος", "λογοσ"))
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος", "λόγοσ"))
        // Bare letter targets: σ and ς are the same letter for the drill.
        assertTrue(GreekTypingGrader.isCloseMatch("σ", "ς"))
        assertTrue(GreekTypingGrader.isCloseMatch("ς", "σ"))
        // Mid-word sigma stays strict: expected word-final ς cannot appear mid-word.
        assertFalse(GreekTypingGrader.isCloseMatch("σος", "ςοσ"))
        // Multi-word answers fold per word.
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος θεοῦ", "λογοσ θεου"))
    }

    // ---- Case folding ----

    @Test
    fun `case is folded`() {
        assertTrue(GreekTypingGrader.isCloseMatch("α", "Α"))
        assertTrue(GreekTypingGrader.isCloseMatch("ἁ", "Ἁ"))
    }

    // ---- NFD / normalization robustness ----

    @Test
    fun `nfd answers match nfc expectations`() {
        val expectedNfc = "ἅ"
        val nfdAnswer = nfd(expectedNfc)
        assertTrue(expectedNfc != nfdAnswer) // sanity: actually decomposed
        assertTrue(GreekTypingGrader.isCloseMatch(expectedNfc, nfdAnswer))
        assertTrue(GreekTypingGrader.isCloseMatch("ᾷ", nfd("ᾷ")))
        // Whitespace collapse on both sides.
        assertTrue(GreekTypingGrader.isCloseMatch("λόγος θεοῦ", "  λόγος   θεοῦ  "))
    }

    @Test
    fun `wrong letters stay wrong`() {
        assertFalse(GreekTypingGrader.isCloseMatch("α", "β"))
        assertFalse(GreekTypingGrader.isCloseMatch("λόγος", "λογο"))
    }

    // ---- Exact flag (accent-exact bonus) ----

    @Test
    fun `exact match honors every mark`() {
        assertTrue(GreekTypingGrader.isExactMatch("λόγος", "λόγος"))
        assertTrue(GreekTypingGrader.isExactMatch("λόγος", nfd("λόγος"))) // NFC-insensitive
        assertFalse(GreekTypingGrader.isExactMatch("λόγος", "λογοσ")) // sigma not folded
        assertFalse(GreekTypingGrader.isExactMatch("λόγος", "λογος")) // accents exact
    }
}
