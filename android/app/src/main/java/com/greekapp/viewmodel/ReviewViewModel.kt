package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import com.greekapp.ReviewSessionRepository
import com.greekapp.core.CoreProvider
import com.greekapp.core.GreekCoreApi
import com.greekapp.core.IntervalPreviewData
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
    val errorMessage: String? = null,
    /** Next-interval preview for the current card; null → plain rating labels. */
    val intervalPreview: IntervalPreviewData? = null
)

class ReviewViewModel(
    private val savedStateHandle: SavedStateHandle
) : ViewModel() {
    private val _state = MutableStateFlow(ReviewState())
    val state: StateFlow<ReviewState> = _state.asStateFlow()

    init {
        // Restore an in-progress session, preferring whichever source has it.
        // SavedStateHandle only survives system-initiated death (config changes,
        // low-memory restore with a bundle); it is lost on swipe-away / `am kill`.
        // The durable DataStore session survives full process death, so we restore
        // the SAME batch at the SAME position rather than rebuilding a new one.
        val savedCardsJson = savedStateHandle.get<String>(KEY_CARDS)
        if (savedCardsJson != null) {
            restoreFrom(
                cardsJson = savedCardsJson,
                index = savedStateHandle.get<Int>(KEY_INDEX) ?: 0,
                revealed = savedStateHandle.get<Boolean>(KEY_REVEALED) ?: false
            )
        } else {
            // No in-memory state: consult durable storage before building a fresh batch.
            viewModelScope.launch(Dispatchers.IO) {
                val persisted = try {
                    ReviewSessionRepository.load(GreekApplication.appContext)
                } catch (e: Exception) {
                    Log.e(TAG, "Failed to load persisted session", e)
                    null
                }
                // Only resume if the persisted batch actually has cards. A blank or
                // empty-array ("[]") payload — or an unparseable one — is NOT resumable;
                // treating it as a completed session would strand the user on the
                // completion screen forever. In that case, clear it and build fresh.
                val persistedCards = persisted?.let { parseCards(it.cardsJson) } ?: emptyList()
                if (persisted != null && persistedCards.isNotEmpty()) {
                    // Rehydrate SavedStateHandle too, so a subsequent config change is fast.
                    savedStateHandle[KEY_CARDS] = persisted.cardsJson
                    savedStateHandle[KEY_INDEX] = persisted.index
                    savedStateHandle[KEY_REVEALED] = persisted.revealed
                    restoreFrom(persisted.cardsJson, persisted.index, persisted.revealed)
                } else {
                    if (persisted != null) clearDurableSession()
                    loadCards()
                }
            }
        }
    }

    /** Set state from a serialized batch + position (no I/O). */
    private fun restoreFrom(cardsJson: String, index: Int, revealed: Boolean) {
        val cards = parseCards(cardsJson)
        val safeIndex = index.coerceIn(0, cards.size)
        _state.value = ReviewState(
            cards = cards,
            currentIndex = safeIndex,
            isAnswerRevealed = revealed,
            isLoading = false,
            isComplete = cards.isEmpty() || safeIndex >= cards.size
        )
        // Preload the interval preview when resuming onto an unanswered card.
        if (!revealed && safeIndex < cards.size) {
            loadIntervalPreview(cards[safeIndex].id)
        }
    }

    private fun loadCards() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current()
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
                if (cards.isEmpty()) {
                    // Nothing to review: clear any stale session and DON'T persist an
                    // empty batch. Persisting "[]" and then clearing it are two
                    // unordered coroutines; if the clear lost the race, a stale "[]"
                    // would be restored as a bogus completed session on next launch.
                    clearDurableSession()
                } else {
                    persistSession(cards, 0, false)
                }
                _state.value = ReviewState(
                    cards = cards,
                    currentIndex = 0,
                    isAnswerRevealed = false,
                    isLoading = false,
                    isComplete = cards.isEmpty()
                )
                if (cards.isNotEmpty()) {
                    loadIntervalPreview(cards.first().id)
                }
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
        core: GreekCoreApi,
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
        persistProgress(s.currentIndex, true)
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
            val core = CoreProvider.current()
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

            // In-session requeue: a card rated Again can become due again within
            // minutes. Pull anything newly due and append it to the END of the
            // session (skipping cards still waiting in the queue) so lapses are
            // seen again before the session ends. Growth is capped so a string
            // of lapses can't extend the session forever.
            var cards = s.cards
            if (cards.size < MAX_SESSION_CARDS) {
                val remainingIds = cards.drop(nextIndex).mapTo(HashSet()) { it.id }
                val dueAgain = try {
                    core.getDueCards(1)
                } catch (e: Exception) {
                    Log.e(TAG, "Requeue check failed after reviewing ${card.id}", e)
                    emptyList()
                }
                for (dc in dueAgain) {
                    if (cards.size >= MAX_SESSION_CARDS) break
                    if (dc.card.id !in remainingIds) {
                        cards = cards + Card(dc.card.id, dc.card.front, dc.card.back)
                    }
                }
            }
            val sessionGrew = cards.size != s.cards.size

            val complete = nextIndex >= cards.size
            _state.value = if (complete) {
                s.copy(cards = cards, isComplete = true, isSubmitting = false, isAnswerRevealed = false)
            } else {
                s.copy(cards = cards, currentIndex = nextIndex, isSubmitting = false, isAnswerRevealed = false, intervalPreview = null)
            }
            savedStateHandle[KEY_INDEX] = _state.value.currentIndex
            savedStateHandle[KEY_REVEALED] = false
            if (complete) {
                // Session finished: drop the durable copy so the next launch builds
                // a fresh batch instead of resuming a completed one.
                clearDurableSession()
            } else if (sessionGrew) {
                // The batch changed, so persist the whole thing (not just the
                // position) — a process-death resume must replay the SAME grown
                // session, including the requeued cards.
                persistSession(cards, _state.value.currentIndex, false)
            } else {
                persistProgress(_state.value.currentIndex, false)
            }
            if (!complete) {
                loadIntervalPreview(cards[nextIndex].id)
            }
        }
    }

    /**
     * Fetch the next-interval preview for the card that just became current
     * (while its answer is still hidden) so the rating buttons can show
     * "10m / 1h / 1d"-style hints the moment the answer is revealed.
     * Best-effort: a failure or null result leaves the buttons label-only.
     */
    private fun loadIntervalPreview(cardId: String) {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            val preview = try {
                core.previewIntervals(cardId)
            } catch (e: Exception) {
                Log.e(TAG, "previewIntervals failed for $cardId", e)
                null
            }
            // Discard the result if the user has already moved to another card.
            val s = _state.value
            val stillCurrent = s.cards.getOrNull(s.currentIndex)?.id == cardId
            if (stillCurrent) {
                _state.value = s.copy(intervalPreview = preview)
            }
        }
    }

    private fun persistSession(cards: List<Card>, index: Int, revealed: Boolean) {
        val json = serializeCards(cards)
        savedStateHandle[KEY_CARDS] = json
        savedStateHandle[KEY_INDEX] = index
        savedStateHandle[KEY_REVEALED] = revealed
        // Mirror the whole batch to durable storage so it survives process death.
        viewModelScope.launch(Dispatchers.IO) {
            try {
                ReviewSessionRepository.save(GreekApplication.appContext, json, index, revealed)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to persist review session", e)
            }
        }
    }

    /** Update only the position/revealed flag in durable storage (batch unchanged). */
    private fun persistProgress(index: Int, revealed: Boolean) {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                ReviewSessionRepository.updateProgress(GreekApplication.appContext, index, revealed)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to persist review progress", e)
            }
        }
    }

    private fun clearDurableSession() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                ReviewSessionRepository.clear(GreekApplication.appContext)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to clear review session", e)
            }
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
        /** Hard cap on session growth from in-session requeueing (Again lapses). */
        private const val MAX_SESSION_CARDS = SESSION_SIZE + 40
        private const val KEY_CARDS = "review_cards"
        private const val KEY_INDEX = "review_index"
        private const val KEY_REVEALED = "review_revealed"
    }
}
