package com.greekapp.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Test

class HomeViewModelTest {
    @Test
    fun unmetGate_routesHomeCopyToAlphabetLesson() {
        val decision = HomeViewModel.homeDecision(
            gateSatisfied = false,
            lettersTotal = 43,
            availableNewLetters = 7
        )

        assertEquals(HomeHeading.START_ALPHABET, decision.heading)
        assertEquals(HomeReviewLabel.LEARN_LETTERS, decision.reviewLabel)
        assertEquals(7, decision.availableNewLetters)
    }

    @Test
    fun satisfiedOrUnavailableGate_usesCustomaryHomeCopy() {
        for (satisfied in listOf(true, null)) {
            val decision = HomeViewModel.homeDecision(
                gateSatisfied = satisfied,
                lettersTotal = 43,
                availableNewLetters = 10
            )
            assertEquals(HomeHeading.WELCOME_BACK, decision.heading)
            assertEquals(HomeReviewLabel.START_REVIEW, decision.reviewLabel)
        }
    }

    @Test
    fun emptyLettersDeck_doesNotForceFirstRunGateCopy() {
        val decision = HomeViewModel.homeDecision(
            gateSatisfied = false,
            lettersTotal = 0,
            availableNewLetters = -1
        )

        assertEquals(HomeHeading.WELCOME_BACK, decision.heading)
        assertEquals(HomeReviewLabel.START_REVIEW, decision.reviewLabel)
        assertEquals(0, decision.availableNewLetters)
    }
}
