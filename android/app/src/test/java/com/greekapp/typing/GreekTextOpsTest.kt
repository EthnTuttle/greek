package com.greekapp.typing

import com.greekapp.ui.components.GreekTextOps
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.text.Normalizer

/**
 * JVM tests for the keyboard's text surgery + sigma context rules (Wave 9,
 * Appendix R input contract). These use GreekTextOps' plain-string
 * primitives, so no Compose classes are needed.
 */
class GreekTextOpsTest {

    private fun nfd(s: String) = Normalizer.normalize(s, Normalizer.Form.NFD)

    @Test
    fun `insert at collapsed cursor`() {
        val r = GreekTextOps.insert("λογος", 5, 5, -1, -1, "!", false)
        assertEquals("λογος!", r.text)
        assertEquals(6, r.cursor)
    }

    @Test
    fun `insert replaces selection`() {
        val r = GreekTextOps.insert("λόγος", 1, 3, -1, -1, "AA", false)
        assertEquals("λAAος", r.text)
        assertEquals(3, r.cursor)
    }

    @Test
    fun `insert into active composition extends it when not replacing`() {
        // Text "αβ" with composition over both chars; inserting at end of the
        // composition must NOT clobber it (binding spec).
        val r = GreekTextOps.insert("αβ", 2, 2, 0, 2, "γ", false)
        assertEquals("αβγ", r.text)
        assertEquals(0, r.compStart)
        assertEquals(3, r.compEnd)
    }

    @Test
    fun `insert with replaceComposing commits over the span`() {
        val r = GreekTextOps.insert("αβ", 0, 0, 0, 2, "χ", true)
        assertEquals("χ", r.text)
        assertEquals(1, r.cursor)
        assertEquals(-1, r.compStart)
    }

    @Test
    fun `insert before composition shifts it intact`() {
        val r = GreekTextOps.insert("αβ", 0, 0, 0, 2, "χ", false)
        assertEquals("χαβ", r.text)
        assertEquals(1, r.compStart)
        assertEquals(3, r.compEnd)
    }

    @Test
    fun `deleteBackward removes a whole grapheme cluster`() {
        // ἄ as base + combining marks (NFD-ish cluster): one backspace removes
        // the base plus its combining marks, not one mark at a time.
        val cluster = nfd("ἄ")
        assertTrue(cluster.length > 1) // sanity: decomposed
        val r = GreekTextOps.deleteBackward(cluster, cluster.length, cluster.length, -1, -1)
        assertEquals("", r.text)
        assertEquals(0, r.cursor)
    }

    @Test
    fun `deleteBackward on selection removes the selection`() {
        val r = GreekTextOps.deleteBackward("αβγ", 0, 2, -1, -1)
        assertEquals("γ", r.text)
        assertEquals(0, r.cursor)
    }

    @Test
    fun `deleteBackward at zero is a no-op`() {
        val r = GreekTextOps.deleteBackward("α", 0, 0, -1, -1)
        assertEquals("α", r.text)
    }

    @Test
    fun `graphemeCount counts clusters`() {
        assertEquals(0, GreekTextOps.graphemeCount(""))
        assertEquals(1, GreekTextOps.graphemeCount("α"))
        assertEquals(1, GreekTextOps.graphemeCount(nfd("ἄ")))
        assertEquals(2, GreekTextOps.graphemeCount("αι"))
        assertEquals(3, GreekTextOps.graphemeCount("ᾶ β"))
    }

    @Test
    fun `sigma resolves final at end and after space or punctuation`() {
        assertEquals("\u03C2", GreekTextOps.resolveSigma("λογο", 4, 4)) // end
        assertEquals("\u03C2", GreekTextOps.resolveSigma("λογο ς", 4, 4)) // before space => final
        val beforeSpace = GreekTextOps.resolveSigma("λογο α", 4, 4)
        assertEquals("\u03C2", beforeSpace)
        assertEquals("\u03C2", GreekTextOps.resolveSigma("λογο,α", 4, 4)) // punct after
        assertEquals("\u03C3", GreekTextOps.resolveSigma("λογοκοσμος", 4, 4)) // mid-word
        assertEquals("\u03C3", GreekTextOps.resolveSigma("", 0, 0)) // word start
        assertEquals("\u03C3", GreekTextOps.resolveSigma("", 0, 0))
    }

    @Test
    fun `sigma skips combining marks left of cursor`() {
        // Cursor right after "ἄ" (α + combining rough + acute): the variant
        // marks must not fool the context scan — still word-final.
        val s = "δο" + nfd("ἅ")
        assertEquals("\u03C2", GreekTextOps.resolveSigma(s, s.length, s.length))
    }
}
