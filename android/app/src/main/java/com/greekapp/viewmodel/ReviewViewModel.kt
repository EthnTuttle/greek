package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class Card(
    val id: String,
    val front: String,
    val back: String
)

data class ReviewState(
    val cards: List<Card> = emptyList(),
    val currentIndex: Int = 0,
    val isAnswerRevealed: Boolean = false,
    val isComplete: Boolean = false,
    val isLoading: Boolean = true
)

class ReviewViewModel : ViewModel() {
    private val _state = MutableStateFlow(ReviewState())
    val state: StateFlow<ReviewState> = _state.asStateFlow()

    init { loadCards() }

    private fun loadCards() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            try {
                val dueCards = core.getDueCards(20)
                val cards = if (dueCards.isNotEmpty()) {
                    dueCards.map { Card(id = it.card.id, front = it.card.front, back = it.card.back) }
                } else {
                    val newCards = core.getNewCards("athenaze", 1, 10)
                    newCards.map { Card(id = it.id, front = it.front, back = it.back) }
                }
                _state.value = ReviewState(
                    cards = cards,
                    isLoading = false,
                    isComplete = cards.isEmpty()
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false, isComplete = true)
            }
        }
    }

    fun revealAnswer() {
        _state.value = _state.value.copy(isAnswerRevealed = true)
    }

    fun answerCard(difficulty: Int) {
        val currentState = _state.value
        val card = currentCard ?: return

        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            val rating = (difficulty + 1).toUByte()
            try {
                core.submitReview(card.id, rating)
            } catch (_: Exception) {}
        }

        val nextIndex = currentState.currentIndex + 1
        if (nextIndex >= currentState.cards.size) {
            _state.value = currentState.copy(isComplete = true)
        } else {
            _state.value = currentState.copy(
                currentIndex = nextIndex,
                isAnswerRevealed = false
            )
        }
    }

    val currentCard: Card?
        get() = _state.value.let { state ->
            if (state.currentIndex < state.cards.size) {
                state.cards[state.currentIndex]
            } else {
                null
            }
        }
}
