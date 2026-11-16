package com.greekapp.viewmodel

import com.greekapp.core.McqData
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Regression tests for A2-practice in PracticeViewModel: failed core saves
 * must surface a retryable state, never be silently logged away. The
 * ViewModel itself can't run on plain JUnit (Dispatchers.Main / android Log),
 * so the decision logic is extracted into static, Log-free helpers:
 *
 *  - [PracticeViewModel.practiceResultPersisted]: `false` AND thrown both
 *    mean "answer not saved" (caller queues the result for retry and sets the
 *    banner state).
 *  - [PracticeViewModel.recordAlphabetCheckOutcome] + `classifyVerdictSave`:
 *    a verdict whose save failed must end up in the "verdict not saved" state
 *    (checkRecorded stays false, verdictSaveFailed = true) — the locally
 *    computed result must never silently be treated as final.
 */
class PracticeViewModelTest {

    @Test
    fun untypeableItemsAlwaysFallBackToMcq() {
        for (index in 0..9) {
            assertTrue(isMcqQuestion(index, typeable = false))
        }
        assertFalse(isMcqQuestion(0, typeable = true))
        assertTrue(isMcqQuestion(1, typeable = true))
        assertFalse(isMcqQuestion(2, typeable = true))
        // Older restored/generated records have no metadata and use safe MCQ.
        assertFalse(McqData("card", "prompt", listOf("answer"), 0).typeable)
    }

    private val result = PracticeResult(
        cardId = "card-1",
        greekForm = "γράφω",
        correct = true,
        exact = true
    )

    @Test
    fun submitPracticeResultReturnsFalse_isNotPersisted() {
        val core = FakeGreekCore(onSubmitPracticeResult = { _, _, _ -> false })
        assertFalse(PracticeViewModel.practiceResultPersisted(core, result))
    }

    @Test
    fun submitPracticeResultThrows_isNotPersisted() {
        val core = FakeGreekCore(onSubmitPracticeResult = { _, _, _ ->
            throw RuntimeException("disk full")
        })
        assertFalse(PracticeViewModel.practiceResultPersisted(core, result))
    }

    @Test
    fun submitPracticeResultReturnsTrue_isPersisted() {
        val core = FakeGreekCore(onSubmitPracticeResult = { _, _, _ -> true })
        assertTrue(PracticeViewModel.practiceResultPersisted(core, result))
    }

    @Test
    fun failureThenRetry_countsExactlyOneSavedPracticeResult() {
        var attempts = 0
        var savedSubmissions = 0
        val core = FakeGreekCore(onSubmitPracticeResult = { _, _, _ ->
            attempts++
            if (attempts == 1) {
                false
            } else {
                savedSubmissions++
                true
            }
        })
        assertFalse(PracticeViewModel.practiceResultPersisted(core, result))
        assertTrue(PracticeViewModel.practiceResultPersisted(core, result))
        assertEquals(2, attempts)
        assertEquals(1, savedSubmissions)
    }

    @Test
    fun recordAlphabetCheckOutcome_throws_mapsToNull() {
        val core = FakeGreekCore(onRecordAlphabetCheckResult = { _, _ ->
            throw RuntimeException("gone")
        })
        assertNull(PracticeViewModel.recordAlphabetCheckOutcome(core, 23, 24))
    }

    @Test
    fun recordAlphabetCheckOutcome_returnsBooleanThrough() {
        val core = FakeGreekCore(onRecordAlphabetCheckResult = { _, _ -> true })
        assertEquals(true, PracticeViewModel.recordAlphabetCheckOutcome(core, 23, 24))
        val core2 = FakeGreekCore(onRecordAlphabetCheckResult = { _, _ -> false })
        assertEquals(false, PracticeViewModel.recordAlphabetCheckOutcome(core2, 20, 24))
    }

    @Test
    fun verdictOutcomeTrue_isPersisted() {
        assertEquals(
            VerdictSaveStatus.Persisted,
            PracticeViewModel.classifyVerdictSave(outcome = true, locallyPassed = false)
        )
        assertEquals(
            VerdictSaveStatus.Persisted,
            PracticeViewModel.classifyVerdictSave(outcome = true, locallyPassed = true)
        )
    }

    @Test
    fun verdictMissingOrFalseAfterLocalPass_isFailed() {
        // Threw / no core:
        assertEquals(
            VerdictSaveStatus.Failed,
            PracticeViewModel.classifyVerdictSave(outcome = null, locallyPassed = true)
        )
        // Returned false despite a local pass — a recorded pass would have
        // latched the gate and returned true, so this save did NOT land:
        assertEquals(
            VerdictSaveStatus.Failed,
            PracticeViewModel.classifyVerdictSave(outcome = false, locallyPassed = true)
        )
    }

    @Test
    fun verdictNullOutcome_isFailedRegardlessOfScore() {
        assertEquals(
            VerdictSaveStatus.Failed,
            PracticeViewModel.classifyVerdictSave(outcome = null, locallyPassed = false)
        )
    }

    @Test
    fun verdictFalseAfterLocalFail_isPersisted() {
        // Recorded-but-not-satisfied (or an adapter-folded error a retry
        // cannot change): the locally-computed "fail" verdict is final here.
        assertEquals(
            VerdictSaveStatus.Persisted,
            PracticeViewModel.classifyVerdictSave(outcome = false, locallyPassed = false)
        )
    }

    @Test
    fun failedCheck_listsPromptAndNameForEveryMissInQuestionOrder() {
        val questions = listOf(
            McqData("letters-001", "Α α", listOf("alpha", "beta"), 0),
            McqData("letters-002", "beta", listOf("Α α", "Β β"), 1),
            McqData("letters-003", "Γ γ", listOf("gamma", "delta"), 0)
        )
        val results = listOf(
            PracticeResult("letters-001", "alpha", correct = false, exact = false),
            PracticeResult("letters-002", "Β β", correct = true, exact = false),
            PracticeResult("letters-003", "gamma", correct = false, exact = false)
        )

        val missed = PracticeViewModel.missedCheckItems(questions, results)

        assertEquals(
            listOf(
                MissedCheckItem(prompt = "Α α", answer = "alpha"),
                MissedCheckItem(prompt = "Γ γ", answer = "gamma")
            ),
            missed
        )
    }
}
