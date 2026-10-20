package com.greekapp.typing

import com.greekapp.ui.components.KeyboardVariants
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.text.Normalizer

/**
 * Variant-map completeness + coherence tests (Wave 9). The full corpus audit
 * lives in the `polytonicAudit` Gradle task; these lock the popup contract:
 * every vowel family must expose breathing/accent combos, iota-subscript and
 * uppercase-breathing forms, and diaeresis on ι/υ.
 */
class KeyboardVariantsTest {

    @Test
    fun `emittable entries are single NFC codepoints in the Greek ranges`() {
        for (s in KeyboardVariants.EMITTABLE) {
            assertEquals("not a single codepoint: $s", 1, s.codePointCount(0, s.length))
            val cp = s.codePointAt(0)
            assertTrue(
                "out of range: ${Integer.toHexString(cp)}",
                cp in 0x0370..0x03FF || cp in 0x1F00..0x1FFF
            )
            assertEquals(
                "not NFC: $s",
                s, Normalizer.normalize(s, Normalizer.Form.NFC)
            )
        }
    }

    @Test
    fun `every plain letter upper and lower is emittable`() {
        val set = KeyboardVariants.EMITTABLE.toSet()
        for (ch in "αβγδεζηθικλμνξοπρστυφχψω") {
            assertTrue("missing $ch", ch.toString() in set)
            assertTrue("missing ${ch.uppercaseChar()}", ch.uppercaseChar().toString() in set)
        }
        assertTrue("missing ς", "ς" in set)
    }

    @Test
    fun `vowel popups include breathing and accent combos`() {
        for ((lower, samples) in mapOf(
            'α' to listOf('ά', 'ὰ', 'ᾶ', 'ἀ', 'ἁ', 'ἄ', 'ἅ', 'ἂ', 'ἃ', 'ἆ', 'ἇ'),
            'ε' to listOf('έ', 'ἐ', 'ἑ', 'ἔ', 'ἕ'),
            'η' to listOf('ή', 'ἠ', 'ἡ', 'ἦ', 'ἧ'),
            'ι' to listOf('ί', 'ἰ', 'ἱ', 'ἶ', 'ἷ'),
            'ο' to listOf('ό', 'ὀ', 'ὁ', 'ὄ', 'ὅ'),
            'υ' to listOf('ύ', 'ὐ', 'ὑ', 'ὖ', 'ὗ'),
            'ω' to listOf('ώ', 'ὠ', 'ὡ', 'ὦ', 'ὧ')
        )) {
            val variants = KeyboardVariants.variantsFor(lower).map { it[0] }.toSet()
            for (expected in samples) {
                assertTrue("variantsFor($lower) missing $expected", expected in variants)
            }
        }
    }

    @Test
    fun `iota subscript forms are emittable`() {
        for (cp in listOf('ᾳ', 'ῃ', 'ῳ', 'ᾴ', 'ᾲ', 'ᾷ', 'ᾄ')) {
            assertTrue(
                "missing $cp",
                KeyboardVariants.EMITTABLE.contains(cp.toString())
            )
        }
        assertTrue('ᾳ' in KeyboardVariants.variantsFor('α').map { it[0] })
    }

    @Test
    fun `uppercase breathing forms are emittable`() {
        for (cp in listOf('Ἀ', 'Ἁ', 'Ἄ', 'Ἅ', 'Ἐ', 'Ἑ', 'Ἠ', 'Ἡ', 'Ὀ', 'Ὁ', 'ᾼ')) {
            assertTrue(
                "missing $cp",
                KeyboardVariants.EMITTABLE.contains(cp.toString())
            )
        }
        assertTrue('Ἀ' in KeyboardVariants.variantsFor('Α').map { it[0] })
    }

    @Test
    fun `diaeresis forms are emittable for iota and upsilon`() {
        val iota = KeyboardVariants.variantsFor('ι').map { it[0] }.toSet()
        val upsilon = KeyboardVariants.variantsFor('υ').map { it[0] }.toSet()
        for (cp in listOf('ϊ', 'ΐ', 'ῒ', 'ῗ')) assertTrue("$cp", cp in iota)
        for (cp in listOf('ϋ', 'ΰ', 'ῢ', 'ῧ')) assertTrue("$cp", cp in upsilon)
    }

    @Test
    fun `rho breathing variants are emittable`() {
        val rho = KeyboardVariants.variantsFor('ρ').map { it[0] }.toSet()
        assertTrue('ῤ' in rho)
        assertTrue('ῥ' in rho)
    }

    @Test
    fun `consonants without diacritic forms have empty popups`() {
        assertTrue(KeyboardVariants.variantsFor('β').isEmpty())
        assertTrue(KeyboardVariants.variantsFor('π').isEmpty())
    }
}
