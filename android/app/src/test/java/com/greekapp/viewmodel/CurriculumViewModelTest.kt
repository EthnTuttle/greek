package com.greekapp.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Regression test for release blocker A3: chapter taps must resolve within
 * the tapped textbook's section, not the first section containing that
 * chapter number. Letters precedes Athenaze/JACT in the curriculum list, so
 * overlapping chapter numbers (e.g. tapping "Athenaze chapter 2") previously
 * matched Letters chapter 2 instead.
 */
class CurriculumViewModelTest {

    private val sections = listOf(
        Section(
            name = "Letters",
            textbook = "letters",
            chapters = listOf(
                Chapter(number = 1, title = "The alphabet", cardCount = 24, completedCount = 0),
                Chapter(number = 2, title = "Vowels", cardCount = 8, completedCount = 0)
            )
        ),
        Section(
            name = "Athenaze",
            textbook = "athenaze",
            chapters = listOf(
                Chapter(number = 1, title = "Ὁ Δικαιόπολις", cardCount = 30, completedCount = 0),
                Chapter(number = 2, title = "Ἡ οἰκία", cardCount = 28, completedCount = 0)
            )
        )
    )

    @Test
    fun chapterTapped_resolvesInOwnTextbook() {
        // Tapping "Athenaze chapter 2" must land in Athenaze, even though
        // Letters (listed first) also has a chapter 2.
        val (athenazeSection, athenazeChapter) =
            CurriculumViewModel.resolveChapter(sections, "athenaze", 2)!!
        assertEquals("athenaze", athenazeSection.textbook)
        assertEquals("Athenaze", athenazeSection.name)
        assertEquals(2, athenazeChapter.number)
        assertEquals("Ἡ οἰκία", athenazeChapter.title)

        // The Letters tap keeps resolving to Letters.
        val (lettersSection, lettersChapter) =
            CurriculumViewModel.resolveChapter(sections, "letters", 2)!!
        assertEquals("letters", lettersSection.textbook)
        assertEquals("Vowels", lettersChapter.title)

        // Unknown textbooks / chapter numbers are no-ops, as before.
        assertNull(CurriculumViewModel.resolveChapter(sections, "jact", 2))
        assertNull(CurriculumViewModel.resolveChapter(sections, "athenaze", 99))
    }
}
