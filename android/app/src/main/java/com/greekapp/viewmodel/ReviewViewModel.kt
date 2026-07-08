package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class Card(
    val id: Int,
    val front: String,
    val back: String
)

data class ReviewState(
    val cards: List<Card> = emptyList(),
    val currentIndex: Int = 0,
    val isAnswerRevealed: Boolean = false,
    val isComplete: Boolean = false
)

class ReviewViewModel : ViewModel() {
    private val _state = MutableStateFlow(
        ReviewState(
            cards = listOf(
                Card(1, "ὁ ἄνθρωπος", "the person, human"),
                Card(2, "ἡ γῆ", "the earth, land"),
                Card(3, "τὸ ἔργον", "the work, deed"),
                Card(4, "λέγω", "I say, I speak"),
                Card(5, "καλός", "beautiful, good"),
                Card(6, "ἐν", "in, on, among (+ dat)"),
                Card(7, "πρός", "to, toward (+ acc)"),
                Card(8, "ἀλλά", "but, however"),
                Card(9, "καί", "and, also, even"),
                Card(10, "οὐ", "not"),
                Card(11, "ὁ λόγος", "the word, speech, reason"),
                Card(12, "ἡ ὁδός", "the road, way, journey")
            )
        )
    )
    val state: StateFlow<ReviewState> = _state.asStateFlow()

    fun revealAnswer() {
        _state.value = _state.value.copy(isAnswerRevealed = true)
    }

    fun answerCard(difficulty: Int) {
        val currentState = _state.value

        // In real app: GreekCore.recordAnswer(card.id, difficulty)

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
