package com.greekapp.viewmodel

import androidx.lifecycle.SavedStateHandle
import com.greekapp.core.IntervalPreviewData
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.uniffi_bindings.CardView
import uniffi.uniffi_bindings.CardWithStateView

/**
 * Regression tests for audit items A2 and the completed-session-restoration
 * P2 in ReviewViewModel.
 *
 * The ViewModel itself can't run on plain JUnit (viewModelScope needs
 * Dispatchers.Main, android.util.Log is stubbed), so — following the
 * CurriculumViewModelTest precedent — the decision logic is extracted into
 * static, Log-free companion helpers and tested here:
 *
 *  - A2: the UniFFI binding converts storage errors to `false` instead of
 *    throwing. [ReviewViewModel.submitReviewPersisted] must treat BOTH as
 *    "not saved", so the caller holds the card in the session, resets
 *    isSubmitting, and shows the retryable error (never advances silently).
 *
 *  - Session P2: completion must remove the SavedStateHandle snapshot —
 *    init() restores from it FIRST, so leaving it behind re-offers the last
 *    answered card after process death.
 */
class ReviewViewModelTest {

    @Test
    fun submitReviewReturnsFalse_isNotPersisted() {
        // Binding-level storage failure: no exception, but nothing was saved.
        val core = FakeGreekCore(onSubmitReview = { _, _ -> false })
        assertFalse(ReviewViewModel.submitReviewPersisted(core, "card-1", 3.toUByte()))
    }

    @Test
    fun submitReviewThrows_isNotPersisted() {
        val core = FakeGreekCore(onSubmitReview = { _, _ ->
            throw RuntimeException("database is locked")
        })
        assertFalse(ReviewViewModel.submitReviewPersisted(core, "card-1", 3.toUByte()))
    }

    @Test
    fun submitReviewReturnsTrue_isPersisted() {
        val core = FakeGreekCore(onSubmitReview = { _, _ -> true })
        assertTrue(ReviewViewModel.submitReviewPersisted(core, "card-1", 3.toUByte()))
    }

    @Test
    fun failureThenRetry_countsExactlyOneSavedSubmission() {
        // The caller contract: on "not saved" the card stays current (not
        // advanced, not completed) with a retryable error, so the user re-taps
        // a rating and the SAME card is submitted again. Across the two
        // attempts the core must see exactly one SAVED submission.
        var attempts = 0
        var savedSubmissions = 0
        val core = FakeGreekCore(onSubmitReview = { _, _ ->
            attempts++
            if (attempts == 1) {
                false // first attempt: binding-level save failure
            } else {
                savedSubmissions++
                true
            }
        })
        // Attempt 1 → held for retry.
        assertFalse(ReviewViewModel.submitReviewPersisted(core, "card-1", 3.toUByte()))
        // Attempt 2 (the retry of the same card) → saved.
        assertTrue(ReviewViewModel.submitReviewPersisted(core, "card-1", 3.toUByte()))
        assertEquals(2, attempts)
        assertEquals(1, savedSubmissions)
    }

    @Test
    fun completionClearsSavedStateHandleSessionKeys() {
        // Seed the exact snapshot a completed session would leave behind
        // (key literals mirror ReviewViewModel's private KEY_* constants).
        val handle = SavedStateHandle()
        handle["review_cards"] = "[{\"id\":\"card-1\",\"front\":\"γράφω\",\"back\":\"I write\"}]"
        handle["review_index"] = 1
        handle["review_revealed"] = true

        ReviewViewModel.clearSavedSession(handle)

        // All three keys gone — init's restore branch keys off review_cards,
        // so a post-death revive can no longer re-offer the finished card.
        assertNull(handle.get<String>("review_cards"))
        assertNull(handle.get<Int>("review_index"))
        assertNull(handle.get<Boolean>("review_revealed"))
    }

    /**
     * C3: the ViewModel consumes the ONE-CALL core session plan as-is:
     * order preserved (due first, then new — the core's guarantee) and ids
     * deduped keeping the first occurrence, so a due card also supplied by
     * the new-card fill can never appear twice.
     */
    @Test
    fun sessionCards_preservesCoreOrderAndDedupsById() {
        fun view(id: String, isNew: Boolean) = CardWithStateView(
            card = CardView(id, "front-$id", "back-$id", 1, "Vocab", "noun", emptyList()),
            stability = if (isNew) 0f else 3f,
            difficulty = if (isNew) 0f else 5f,
            dueDate = if (isNew) "" else "2020-01-02T00:00:00Z",
            reps = if (isNew) 0 else 1,
            lapses = 0,
            isNew = isNew
        )
        val core = FakeGreekCore(
            onBuildReviewSession = { size ->
                assertEquals(20u, size)
                listOf(
                    view("due-1", isNew = false),
                    view("due-2", isNew = false),
                    view("new-1", isNew = true),
                    view("due-1", isNew = true) // impossible double-listing: deduped
                )
            }
        )

        val cards = ReviewViewModel.sessionCards(core, 20u)

        assertEquals(listOf("due-1", "due-2", "new-1"), cards.map { it.id })
        // "first listing wins" — the due-side listing survives the duplicate.
        assertEquals("front-due-1", cards[0].front)
        assertEquals("back-due-1", cards[0].back)
        assertFalse(cards[0].isFirstExposure)
        assertTrue(cards[2].isFirstExposure)
    }

    @Test
    fun firstExposure_onlyNeverReviewedNewCardsGetTwoButtonLesson() {
        assertTrue(ReviewViewModel.isFirstExposure(reps = 0, isNew = true))
        assertFalse(ReviewViewModel.isFirstExposure(reps = 1, isNew = false))
        assertFalse(ReviewViewModel.isFirstExposure(reps = 1, isNew = true))
        // A malformed flag cannot turn a reviewed card into a new lesson.
        assertFalse(ReviewViewModel.isFirstExposure(reps = 2, isNew = true))
        assertFalse(ReviewViewModel.isFirstExposure(reps = 0, isNew = false))
    }

    @Test
    fun nextReturnAt_usesPreviewForChosenRatingOnly() {
        val preview = IntervalPreviewData(
            againSecs = 60u,
            hardSecs = 300u,
            goodSecs = 600u,
            easySecs = 86400u
        )

        assertEquals(66_000L, ReviewViewModel.nextReturnAt(preview, 0, 6_000L))
        assertEquals(606_000L, ReviewViewModel.nextReturnAt(preview, 2, 6_000L))
        assertEquals(86_406_000L, ReviewViewModel.nextReturnAt(preview, 3, 6_000L))
        assertNull(ReviewViewModel.nextReturnAt(null, 2, 6_000L))
    }

    @Test
    fun completion_dueNowContinuationTakesPriorityOverEarlyReturns() {
        val completion = ReviewViewModel.completionStatus(
            remainingDue = 7,
            pendingReturns = listOf(120_000L),
            nowMs = 0L
        )

        assertEquals(7, completion.remainingDue)
        assertEquals(0, completion.returningSoonCount)
        assertNull(completion.minutesUntilNext)
    }

    @Test
    fun completion_countsEarlyStepCardsAndRoundsEarliestMinutes() {
        val now = 1_000_000L
        val completion = ReviewViewModel.completionStatus(
            remainingDue = 0,
            pendingReturns = listOf(
                now + 8L * 60_000L - 10_000L, // about 8 minutes
                now + 10L * 60_000L,
                now + 16L * 60_000L,           // outside the early-step window
                now - 1L                        // already due, not pending
            ),
            nowMs = now
        )

        assertEquals(0, completion.remainingDue)
        assertEquals(2, completion.returningSoonCount)
        assertEquals(8, completion.minutesUntilNext)
    }

    @Test
    fun completion_onlyLongTermCards_hasHonestGenericBatchEnding() {
        val completion = ReviewViewModel.completionStatus(
            remainingDue = 0,
            pendingReturns = listOf(16L * 60_000L),
            nowMs = 0L
        )

        assertEquals(ReviewCompletion(), completion)
    }
}
