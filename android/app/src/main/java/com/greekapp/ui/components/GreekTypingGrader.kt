package com.greekapp.ui.components

import java.text.Normalizer

/**
 * Grading equivalence classes for learner-typed Greek (Wave 9,
 * DESIGN-greek-keyboard.md Appendix R — "Grading policy").
 *
 * Mirrors the will-be-Rust rule so client-side single-grapheme drills grade
 * identically ahead of the core port:
 *
 * STRICT (mismatch ⇒ wrong):
 *  - smooth/rough breathing (phonemic: rough = /h/)
 *  - iota subscript (ypogegrammeni)
 *  - diaeresis
 *
 * LENIENT (ignored until accent is formally taught):
 *  - pitch accents: acute, grave, circumflex (Erasmian does not pronounce them)
 *
 * FOLDED BEFORE COMPARE:
 *  - final sigma: σ → ς at word end, applied to BOTH sides (typing "λογοσ"
 *  for "λόγος" is correct, and a bare "σ" matches a target "ς")
 *  - case (typed_letter accepts Α for a lowercase target)
 *
 * This is deliberately distinct from dictionary search normalization
 * (case-fold + strip-accents, accent-AND-breathing-insensitive), which is
 * looser on purpose: grading must keep phonemic contrast.
 */
object GreekTypingGrader {

    // Pitch accents: LENIENT — stripped before compare.
    private const val VARIA = '\u0300'
    private const val OXIA = '\u0301'
    private const val PERISPOMENI = '\u0342'

    // Breathing, diaeresis, iota subscript, vrachy/macron: STRICT — retained.
    // (U+0313 psili, U+0314 dasia, U+0308 dialytika, U+0345 ypogegrammeni,
    //  U+0304 macron, U+0306 vrachy)

    private fun nfc(s: String): String = Normalizer.normalize(s, Normalizer.Form.NFC)
    private fun nfd(s: String): String = Normalizer.normalize(s, Normalizer.Form.NFD)

    private fun collapseWhitespace(s: String): String =
        s.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")

    private fun Char.isPitchAccent(): Boolean =
        this == VARIA || this == OXIA || this == PERISPOMENI

    /**
     * Fold medial sigma to final sigma when nothing but whitespace /
     * punctuation / combining marks follows it in the same word. Applied to
     * the NFC form; both sides go through it, so either σ or ς from the
     * learner matches the card's word-final sigma.
     */
    private fun foldFinalSigma(s: String): String {
        val sb = StringBuilder(s)
        for (i in sb.indices) {
            if (sb[i] != 'σ') continue
            var j = i + 1
            // Skip combining marks attached to the sigma.
            while (j < sb.length && Character.getType(sb[j]) == Character.NON_SPACING_MARK.toInt()) j++
            val next = sb.getOrNull(j)
            if (next == null || next.isWhitespace() ||
                Character.getType(next).let {
                    it == Character.CONNECTOR_PUNCTUATION.toInt() ||
                        it == Character.DASH_PUNCTUATION.toInt() ||
                        it == Character.START_PUNCTUATION.toInt() ||
                        it == Character.END_PUNCTUATION.toInt() ||
                        it == Character.OTHER_PUNCTUATION.toInt() ||
                        it == Character.INITIAL_QUOTE_PUNCTUATION.toInt() ||
                        it == Character.FINAL_QUOTE_PUNCTUATION.toInt()
                }
            ) {
                sb.setCharAt(i, 'ς')
            }
        }
        return sb.toString()
    }

    /** Canonical comparison form per the equivalence classes above. */
    fun canon(s: String): String {
        val folded = foldFinalSigma(nfc(collapseWhitespace(s)))
        val stripped = nfd(folded).filter { !it.isPitchAccent() }
        return nfc(stripped.lowercase())
    }

    /** Lenient-over-pitch, strict-over-breathing/subscript/diaeresis match. */
    fun isCloseMatch(expected: String, answer: String): Boolean =
        canon(answer) == canon(expected)

    /** Exact-match check (for the accent-exact bonus flag): NFC equality
     *  after whitespace collapse, no accent leniency, no sigma folding. */
    fun isExactMatch(expected: String, answer: String): Boolean =
        nfc(collapseWhitespace(answer)) == nfc(collapseWhitespace(expected))
}
