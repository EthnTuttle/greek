package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import com.greekapp.SettingsRepository
import com.greekapp.core.AlphabetGateState
import com.greekapp.core.CoreProvider
import com.greekapp.core.GreekCoreApi
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

enum class HomeHeading { WELCOME_BACK, START_ALPHABET }
enum class HomeReviewLabel { START_REVIEW, LEARN_LETTERS }

data class HomeDecision(
    val heading: HomeHeading = HomeHeading.WELCOME_BACK,
    val reviewLabel: HomeReviewLabel = HomeReviewLabel.START_REVIEW,
    val availableNewLetters: Int = 0
)

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
    val decision: HomeDecision = HomeDecision(),
    val isLoading: Boolean = true
)

class HomeViewModel : ViewModel() {
    private val _state = MutableStateFlow(HomeState())
    val state: StateFlow<HomeState> = _state.asStateFlow()

    init {
        refreshStats()
        syncOnLaunch()
    }

    /**
     * Wave-6 launch-time auto-sync: silent pull → iroh sync → push, gated on
     * the user's toggle AND at least one configured transport (nostr identity,
     * or an iroh join recorded via [SettingsKeys.SYNC_IROH_JOINED]). Failures
     * are log-only; a successful round refreshes the home stats so merged
     * history shows up immediately.
     */
    private fun syncOnLaunch() {
        viewModelScope.launch(Dispatchers.IO) {
            var ran = false
            runCatching {
                val context = GreekApplication.appContext
                if (!SettingsRepository.syncAutolaunchFlow(context).first()) return@runCatching
                val core = CoreProvider.current() ?: return@runCatching
                val nostrReady = core.syncNostrConfig().hasIdentity
                val irohReady = SettingsRepository.irohJoinedFlow(context).first()
                if (!nostrReady && !irohReady) return@runCatching
                if (nostrReady) core.syncNostrPull()
                if (irohReady) core.syncIrohNow()
                if (nostrReady) core.syncNostrPush()
                ran = true
            }.onFailure { Log.w(TAG, "launch auto-sync failed", it) }
            if (ran) refreshStats()
        }
    }

    fun refreshStats() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                val stats = core.getReviewStats()
                // The gate's introduced counter is authoritative even when
                // check answers have already spent today's new-card budget.
                val alphabetGate = try {
                    core.getAlphabetGateState()
                } catch (e: Exception) {
                    null
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
                val decision = homeDecision(
                    gateSatisfied = alphabetGate?.satisfied,
                    lettersTotal = alphabetGate?.lettersTotal?.toInt() ?: 0,
                    availableNewLetters = availableNewLetters(core, alphabetGate, newCardsToday)
                )
                // Core learning_count counts introduced review states, never
                // Reader known-word marks or the total size of bundled decks.
                val wordsLearned = stats.learningCount
                // Practice accuracy; null → Home hides the row (total == 0, or
                // the wave-3 stub is still in place, or the call failed).
                val practiceAccuracy = try {
                    val ps = core.getPracticeStats()
                    if (ps.total == 0u) null else (ps.correct * 100u / ps.total).toInt()
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
                    decision = decision,
                    isLoading = false
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false)
            }
        }
    }

    companion object {
        private const val DAILY_NEW_CARD_LIMIT = 10
        private const val SESSION_SIZE = 20u

        /** Home's primary routing/copy decision is independent of Android and I/O. */
        internal fun homeDecision(
            gateSatisfied: Boolean?,
            lettersTotal: Int,
            availableNewLetters: Int
        ): HomeDecision {
            val learningAlphabet = gateSatisfied == false && lettersTotal > 0
            return HomeDecision(
                heading = if (learningAlphabet) HomeHeading.START_ALPHABET else HomeHeading.WELCOME_BACK,
                reviewLabel = if (learningAlphabet) HomeReviewLabel.LEARN_LETTERS else HomeReviewLabel.START_REVIEW,
                availableNewLetters = availableNewLetters.coerceAtLeast(0)
            )
        }

        /** Ask the core for the actual available batch; never fabricate a session in Home. */
        private fun availableNewLetters(
            core: GreekCoreApi,
            gate: AlphabetGateState?,
            newCardsToday: Int
        ): Int {
            if (gate == null || gate.satisfied || gate.lettersTotal == 0u) return 0
            return try {
                core.buildReviewSession(SESSION_SIZE).count {
                    it.isNew && it.reps == 0 && it.card.id.startsWith("letters-")
                }
            } catch (e: Exception) {
                // Conservative fallback for a stale binding: remaining deck
                // and daily allowance only. The real batch remains core-built.
                val deckRemaining = (gate.lettersTotal.toLong() - gate.lettersIntroduced.toLong())
                    .coerceAtLeast(0L).toInt()
                minOf(deckRemaining, (DAILY_NEW_CARD_LIMIT - newCardsToday).coerceAtLeast(0))
                    .coerceAtLeast(0)
            }
        }

        private const val TAG = "HomeViewModel"
    }
}
