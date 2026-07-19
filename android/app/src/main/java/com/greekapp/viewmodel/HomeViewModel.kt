package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
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
    val isLoading: Boolean = true
)

class HomeViewModel : ViewModel() {
    private val _state = MutableStateFlow(HomeState())
    val state: StateFlow<HomeState> = _state.asStateFlow()

    init { refreshStats() }

    fun refreshStats() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
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
                _state.value = HomeState(
                    dueCardsCount = stats.dueCount,
                    streak = stats.streakDays,
                    totalCards = stats.totalCards,
                    reviewedToday = stats.reviewedToday,
                    wordsLearned = wordsLearned,
                    chaptersUnlocked = chaptersUnlocked,
                    isLoading = false
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false)
            }
        }
    }
}
