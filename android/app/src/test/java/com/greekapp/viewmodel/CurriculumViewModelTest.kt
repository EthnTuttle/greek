package com.greekapp.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
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

    @Test
    fun chapter_showsSeparateLearningAndMasteredNumbers() {
        val chapter = Chapter(
            number = 2,
            title = "Consonants",
            cardCount = 9,
            completedCount = 0,
            learnedCount = 7
        )

        assertEquals(7, chapter.learnedCount)
        assertEquals(0, chapter.masteredCount)
        assertEquals(7f / 9f, chapter.progress)
    }

    @Test
    fun lockedChapter_isReadyOnlyWithNoOutstandingRequirement() {
        val ready = Chapter(
            number = 2,
            title = "Consonants",
            cardCount = 9,
            completedCount = 0,
            unlocked = false,
            unlockHint = ""
        )
        val blocked = ready.copy(unlockHint = "Learn 7 more cards")

        assertTrue(ready.readyToUnlock)
        assertFalse(blocked.readyToUnlock)
        assertFalse(ready.copy(unlocked = true).readyToUnlock)
        assertFalse(ready.copy(available = false).readyToUnlock)
        assertEquals("Chapter 2 unlocked", CurriculumViewModel.chapterUnlockedMessage(2))
    }
}
