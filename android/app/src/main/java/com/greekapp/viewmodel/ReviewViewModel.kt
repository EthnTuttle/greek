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
    val back: String,
    /** Core scheduling flags retained so a restored session keeps the same lesson UI. */
    val isNew: Boolean = false,
    val reps: Int = 1
) {
    val isFirstExposure: Boolean
        get() = ReviewViewModel.isFirstExposure(reps = reps, isNew = isNew)
}

data class ScheduledReturn(
    val cardId: String,
    val returnAtMs: Long
)

/** What the learner can honestly do after finishing the current queue. */
data class ReviewCompletion(
    val remainingDue: Int = 0,
    val returningSoonCount: Int = 0,
    val minutesUntilNext: Int? = null
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
    val intervalPreview: IntervalPreviewData? = null,
    /** Honest end-of-queue guidance, populated from live core stats and rating previews. */
    val completion: ReviewCompletion = ReviewCompletion(),
    /** Epoch-ms return times predicted by the exact intervals shown on rating buttons. */
    val pendingReturns: List<ScheduledReturn> = emptyList()
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
                // C3: ONE core call builds the session — due cards first, then
                // new cards across curricula consuming a SINGLE daily new-card
                // budget snapshot. (The old per-chapter getNewCards loop below
                // re-saw the full unspent budget on every chapter, so a 10/day
                // cap admitted a 20-card multi-curriculum session.)
                val cards = sessionCards(core, SESSION_SIZE.toUInt())
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

    /** Build another due-first batch without inventing or reordering cards in the UI. */
    fun continueReview() {
        _state.value = ReviewState(isLoading = true)
        loadCards()
    }

    fun answerCard(difficulty: Int) {
        val currentState = _state.value
        if (currentState.isSubmitting) return
        val card = currentCard ?: return
        val answeredAtMs = System.currentTimeMillis()
        val returnAtMs = nextReturnAt(
            preview = currentState.intervalPreview,
            difficulty = difficulty,
            answeredAtMs = answeredAtMs
        )

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
            // The UniFFI binding converts storage errors to `false` instead of
            // throwing, so a false return and a thrown exception are the SAME
            // failure here: never silently drop or advance past the review.
            if (!submitReviewPersisted(core, card.id, rating)) {
                Log.w(TAG, "submitReview not persisted for ${card.id}; holding card for retry")
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
                        cards = cards + Card(
                            id = dc.card.id,
                            front = dc.card.front,
                            back = dc.card.back,
                            isNew = dc.isNew,
                            reps = dc.reps
                        )
                    }
                }
            }
            val sessionGrew = cards.size != s.cards.size

            val complete = nextIndex >= cards.size
            val pendingReturns = s.pendingReturns.filterNot { it.cardId == card.id } +
                listOfNotNull(returnAtMs?.let { ScheduledReturn(card.id, it) })
            val completion = if (complete) {
                val remainingDue = try {
                    core.getReviewStats().dueCount.coerceAtLeast(0)
                } catch (e: Exception) {
                    Log.w(TAG, "Could not count remaining due cards", e)
                    0
                }
                completionStatus(
                    remainingDue = remainingDue,
                    pendingReturns = pendingReturns.map { it.returnAtMs },
                    nowMs = System.currentTimeMillis()
                )
            } else {
                s.completion
            }
            _state.value = if (complete) {
                s.copy(
                    cards = cards,
                    isComplete = true,
                    isSubmitting = false,
                    isAnswerRevealed = false,
                    completion = completion,
                    pendingReturns = pendingReturns
                )
            } else {
                s.copy(
                    cards = cards,
                    currentIndex = nextIndex,
                    isSubmitting = false,
                    isAnswerRevealed = false,
                    intervalPreview = null,
                    pendingReturns = pendingReturns
                )
            }
            if (complete) {
                // Session finished. Drop BOTH session copies, in-memory FIRST:
                // the SavedStateHandle snapshot takes restoration precedence in
                // init, so leaving it behind would re-offer the last answered
                // card after process death. The durable clear is awaited here
                // (suspend fun, same coroutine) — no fire-and-forget race.
                clearSavedSession(savedStateHandle)
                clearDurableSession()
            } else {
                savedStateHandle[KEY_INDEX] = _state.value.currentIndex
                savedStateHandle[KEY_REVEALED] = false
                if (sessionGrew) {
                    // The batch changed, so persist the whole thing (not just the
                    // position) — a process-death resume must replay the SAME grown
                    // session, including the requeued cards.
                    persistSession(cards, _state.value.currentIndex, false)
                } else {
                    persistProgress(_state.value.currentIndex, false)
                }
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

    /**
     * Suspend so callers inside coroutines AWAIT the clear: fire-and-forget
     * clears race with any later persist of the same session.
     */
    private suspend fun clearDurableSession() {
        try {
            ReviewSessionRepository.clear(GreekApplication.appContext)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to clear review session", e)
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
                    .put("isNew", c.isNew)
                    .put("reps", c.reps)
            )
        }
        return arr.toString()
    }

    private fun parseCards(json: String): List<Card> {
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                Card(
                    id = o.getString("id"),
                    front = o.getString("front"),
                    back = o.getString("back"),
                    isNew = o.optBoolean("isNew", false),
                    // Older saved sessions have no scheduling flags; never
                    // reinterpret one of their cards as a first exposure.
                    reps = o.optInt("reps", 1)
                )
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

        /**
         * Map the core's ONE-CALL session plan (C3: due-first-then-new
         * ordering with a single daily new-card budget; see
         * [GreekCoreApi.buildReviewSession]) into the UI's ordered,
         * id-deduped card list. Extracted as a static, Log-free helper so
         * host-JVM unit tests can drive it without Dispatchers.Main (same
         * pattern as CurriculumViewModel.resolveChapter).
         */
        internal fun sessionCards(core: GreekCoreApi, sessionSize: UInt): List<Card> {
            val queue = LinkedHashMap<String, Card>()
            for (sc in core.buildReviewSession(sessionSize)) {
                queue.getOrPut(sc.card.id) {
                    Card(
                        id = sc.card.id,
                        front = sc.card.front,
                        back = sc.card.back,
                        isNew = sc.isNew,
                        reps = sc.reps
                    )
                }
            }
            return queue.values.toList()
        }

        /** A lesson is offered only for the core's never-reviewed, new-card state. */
        internal fun isFirstExposure(reps: Int, isNew: Boolean): Boolean = isNew && reps == 0

        /** Use the same preview the learner saw; no hand-built schedule or rating policy. */
        internal fun nextReturnAt(
            preview: IntervalPreviewData?,
            difficulty: Int,
            answeredAtMs: Long
        ): Long? {
            val seconds = when (difficulty) {
                0 -> preview?.againSecs
                1 -> preview?.hardSecs
                2 -> preview?.goodSecs
                3 -> preview?.easySecs
                else -> null
            } ?: return null
            return answeredAtMs + seconds.toLong() * 1_000L
        }

        /**
         * Prefer an actionable due-now continuation. Otherwise report only
         * returns within the early-step window, with the earliest rounded to
         * the nearest minute. Long-term scheduled cards do not produce a
         * misleading "all cards for today" claim.
         */
        internal fun completionStatus(
            remainingDue: Int,
            pendingReturns: List<Long>,
            nowMs: Long,
            earlyStepWindowMs: Long = 15L * 60L * 1_000L
        ): ReviewCompletion {
            if (remainingDue > 0) return ReviewCompletion(remainingDue = remainingDue)
            val soon = pendingReturns.filter { it > nowMs && it <= nowMs + earlyStepWindowMs }.sorted()
            if (soon.isEmpty()) return ReviewCompletion()
            val minutes = ((soon.first() - nowMs + 30_000L) / 60_000L)
                .toInt().coerceAtLeast(1)
            return ReviewCompletion(
                returningSoonCount = soon.size,
                minutesUntilNext = minutes
            )
        }

        /**
         * One submission attempt through the core. The UniFFI binding converts
         * storage errors to `false` instead of throwing, and a thrown
         * exception means the same thing to the caller — so this returns true
         * ONLY when the review was durably persisted. Extracted as a static,
         * Log-free helper so host-JVM unit tests can drive it without
         * Dispatchers.Main (same pattern as CurriculumViewModel.resolveChapter).
         */
        internal fun submitReviewPersisted(core: GreekCoreApi, cardId: String, rating: UByte): Boolean =
            try {
                core.submitReview(cardId, rating)
            } catch (e: Exception) {
                false
            }

        /**
         * Remove the in-memory session snapshot. Called on completion in the
         * same block as the durable DataStore clear: init restores from
         * SavedStateHandle FIRST, so retaining these keys after completion
         * would re-offer the last answered card after process death.
         */
        internal fun clearSavedSession(handle: SavedStateHandle) {
            handle.remove<String>(KEY_CARDS)
            handle.remove<Int>(KEY_INDEX)
            handle.remove<Boolean>(KEY_REVEALED)
        }
    }
}
