package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Wave-4 letters track identity: textbook key + expected chapter count. */
private const val LETTERS_TEXTBOOK = "letters"
private const val LETTERS_CHAPTER_COUNT = 5

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
    /** Show the "start with the alphabet" banner while letters ch.1 is unmastered. */
    val showLettersBanner: Boolean = false,
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
                // Letters onboarding banner (wave 4): shown while the letters
                // track is present (5 chapters, all with cards) but chapter 1 is
                // not yet ~80% mastered. Mirrors the core UNLOCK_THRESHOLD rule
                // in integer math: mastered < ceil(0.8 * cards) ⇔ mastered*5 < cards*4.
                val showLettersBanner = try {
                    val chapters = core.getChapters(LETTERS_TEXTBOOK)
                    val first = chapters.firstOrNull { it.chapter == 1 }
                    chapters.size == LETTERS_CHAPTER_COUNT &&
                        chapters.all { it.cardCount > 0 } &&
                        first != null &&
                        first.masteredCount * 5 < first.cardCount * 4
                } catch (e: Exception) {
                    false
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
                    showLettersBanner = showLettersBanner,
                    isLoading = false
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false)
            }
        }
    }
}
