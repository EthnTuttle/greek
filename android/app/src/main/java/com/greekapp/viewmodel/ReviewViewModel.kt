package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

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
    val isLoading: Boolean = true,
    val isSubmitting: Boolean = false,
    val errorMessage: String? = null
)

class ReviewViewModel(
    private val savedStateHandle: SavedStateHandle
) : ViewModel() {
    private val _state = MutableStateFlow(ReviewState())
    val state: StateFlow<ReviewState> = _state.asStateFlow()

    init {
        // Restore an in-progress session across process death, if one was saved.
        val savedCardsJson = savedStateHandle.get<String>(KEY_CARDS)
        if (savedCardsJson != null) {
            val cards = parseCards(savedCardsJson)
            val index = (savedStateHandle.get<Int>(KEY_INDEX) ?: 0).coerceIn(0, cards.size)
            val revealed = savedStateHandle.get<Boolean>(KEY_REVEALED) ?: false
            _state.value = ReviewState(
                cards = cards,
                currentIndex = index,
                isAnswerRevealed = revealed,
                isLoading = false,
                isComplete = cards.isEmpty() || index >= cards.size
            )
        } else {
            loadCards()
        }
    }

    private fun loadCards() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore
            if (core == null) {
                _state.value = _state.value.copy(
                    isLoading = false,
                    errorMessage = "Couldn't reach the learning engine. Please reopen the app."
                )
                return@launch
            }
            try {
                // 1. Due cards come first (these may now throw).
                val queue = LinkedHashMap<String, Card>()
                for (dc in core.getDueCards(SESSION_SIZE)) {
                    queue[dc.card.id] = Card(dc.card.id, dc.card.front, dc.card.back)
                }

                // 2. Fill any remaining slots with new cards from unlocked, available
                //    chapters across every curriculum, so progression isn't stuck on ch.1.
                if (queue.size < SESSION_SIZE) {
                    fillWithNewCards(core, queue)
                }

                val cards = queue.values.toList()
                persistSession(cards, 0, false)
                _state.value = ReviewState(
                    cards = cards,
                    currentIndex = 0,
                    isAnswerRevealed = false,
                    isLoading = false,
                    isComplete = cards.isEmpty()
                )
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load review queue", e)
                _state.value = _state.value.copy(
                    isLoading = false,
                    isComplete = false,
                    errorMessage = "Couldn't load your cards. Please try again."
                )
            }
        }
    }

    private fun fillWithNewCards(
        core: uniffi.uniffi_bindings.GreekCore,
        queue: LinkedHashMap<String, Card>
    ) {
        val curricula = try {
            core.getCurricula()
        } catch (e: Exception) {
            Log.e(TAG, "getCurricula failed while filling queue", e)
            return
        }
        for (cur in curricula) {
            if (queue.size >= SESSION_SIZE) break
            val chapters = try {
                core.getChapters(cur.textbook)
            } catch (e: Exception) {
                Log.e(TAG, "getChapters failed for ${cur.textbook}", e)
                continue
            }
            for (ch in chapters) {
                if (queue.size >= SESSION_SIZE) break
                if (!ch.unlocked || !ch.available) continue
                val remaining = SESSION_SIZE - queue.size
                val newCards = try {
                    core.getNewCards(cur.textbook, ch.chapter, remaining)
                } catch (e: Exception) {
                    Log.e(TAG, "getNewCards failed for ${cur.textbook} ch ${ch.chapter}", e)
                    continue
                }
                for (nc in newCards) {
                    if (queue.size >= SESSION_SIZE) break
                    queue.getOrPut(nc.id) { Card(nc.id, nc.front, nc.back) }
                }
            }
        }
    }

    fun revealAnswer() {
        val s = _state.value
        _state.value = s.copy(isAnswerRevealed = true)
        savedStateHandle[KEY_REVEALED] = true
    }

    /** Retry loading after a load-time error / dismiss the error banner. */
    fun retryLoad() {
        _state.value = _state.value.copy(isLoading = true, errorMessage = null)
        loadCards()
    }

    fun answerCard(difficulty: Int) {
        val currentState = _state.value
        if (currentState.isSubmitting) return
        val card = currentCard ?: return

        _state.value = currentState.copy(isSubmitting = true, errorMessage = null)

        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore
            if (core == null) {
                // Hold the card; do not advance or drop the review.
                _state.value = _state.value.copy(
                    isSubmitting = false,
                    errorMessage = "Couldn't save your review. Tap a rating to retry."
                )
                return@launch
            }
            val rating = (difficulty + 1).toUByte()
            try {
                core.submitReview(card.id, rating)
            } catch (e: Exception) {
                // Never silently drop a review: keep the current card and surface the error.
                Log.e(TAG, "submitReview failed for ${card.id}", e)
                _state.value = _state.value.copy(
                    isSubmitting = false,
                    errorMessage = "Couldn't save your review. Tap a rating to retry."
                )
                return@launch
            }

            // Only advance once the review is safely persisted.
            val s = _state.value
            val nextIndex = s.currentIndex + 1
            _state.value = if (nextIndex >= s.cards.size) {
                s.copy(isComplete = true, isSubmitting = false, isAnswerRevealed = false)
            } else {
                s.copy(currentIndex = nextIndex, isSubmitting = false, isAnswerRevealed = false)
            }
            savedStateHandle[KEY_INDEX] = _state.value.currentIndex
            savedStateHandle[KEY_REVEALED] = false
        }
    }

    private fun persistSession(cards: List<Card>, index: Int, revealed: Boolean) {
        savedStateHandle[KEY_CARDS] = serializeCards(cards)
        savedStateHandle[KEY_INDEX] = index
        savedStateHandle[KEY_REVEALED] = revealed
    }

    val currentCard: Card?
        get() = _state.value.let { state ->
            if (state.currentIndex < state.cards.size) {
                state.cards[state.currentIndex]
            } else {
                null
            }
        }

    private fun serializeCards(cards: List<Card>): String {
        val arr = JSONArray()
        for (c in cards) {
            arr.put(
                JSONObject()
                    .put("id", c.id)
                    .put("front", c.front)
                    .put("back", c.back)
            )
        }
        return arr.toString()
    }

    private fun parseCards(json: String): List<Card> {
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                Card(o.getString("id"), o.getString("front"), o.getString("back"))
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse saved session", e)
            emptyList()
        }
    }

    companion object {
        private const val TAG = "ReviewViewModel"
        private const val SESSION_SIZE = 20
        private const val KEY_CARDS = "review_cards"
        private const val KEY_INDEX = "review_index"
        private const val KEY_REVEALED = "review_revealed"
    }
}
