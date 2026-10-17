package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.AlphabetGateState
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class HomeState(
    val dueCardsCount: Int = 0,
    val streak: Int = 0,
    val totalCards: Int = 0,
    val reviewedToday: Int = 0,
    val wordsLearned: Int = 0,
    val chaptersUnlocked: Int = 0,
    val newCardsToday: Int = 0,
    /** Practice accuracy 0–100, or null when no practice is recorded (row hidden). */
    val practiceAccuracy: Int? = null,
    /**
     * Alphabet-gate state from the core (wave 5), or null when the core call
     * failed — the screen hides the gate card for null / satisfied / empty
     * deck (lettersTotal == 0).
     */
    val alphabetGate: AlphabetGateState? = null,
    val isLoading: Boolean = true
)

class HomeViewModel : ViewModel() {
    private val _state = MutableStateFlow(HomeState())
    val state: StateFlow<HomeState> = _state.asStateFlow()

    init { refreshStats() }

    fun refreshStats() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                val stats = core.getReviewStats()
                // Words learned: count of words the user has marked known (real, Rust-backed).
                val wordsLearned = try {
                    core.getKnownWords().size
                } catch (e: Exception) {
                    0
                }
                // Chapters: number of unlocked chapters across all curricula.
                val chaptersUnlocked = try {
                    core.getCurricula().sumOf { cv ->
                        core.getChapters(cv.textbook).count { it.unlocked }
                    }
                } catch (e: Exception) {
                    0
                }
                // New cards studied today (counts against the daily new-card budget).
                val newCardsToday = try {
                    core.newCardsToday().toInt()
                } catch (e: Exception) {
                    0
                }
                // Practice accuracy; null → Home hides the row (total == 0, or
                // the wave-3 stub is still in place, or the call failed).
                val practiceAccuracy = try {
                    val ps = core.getPracticeStats()
                    if (ps.total == 0u) null else (ps.correct * 100u / ps.total).toInt()
                } catch (e: Exception) {
                    null
                }
                // Alphabet gate (wave 5): replaces the wave-4 letters banner.
                // The core decides satisfaction across all lanes; Home only
                // renders the returned counters. Null → card hidden.
                val alphabetGate = try {
                    core.getAlphabetGateState()
                } catch (e: Exception) {
                    null
                }
                _state.value = HomeState(
                    dueCardsCount = stats.dueCount,
                    streak = stats.streakDays,
                    totalCards = stats.totalCards,
                    reviewedToday = stats.reviewedToday,
                    wordsLearned = wordsLearned,
                    chaptersUnlocked = chaptersUnlocked,
                    newCardsToday = newCardsToday,
                    practiceAccuracy = practiceAccuracy,
                    alphabetGate = alphabetGate,
                    isLoading = false
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false)
            }
        }
    }
}
