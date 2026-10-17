package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import com.greekapp.core.McqData
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject

/** Question-type alternation, 0-based: even index → typed, odd index → MCQ. */
internal fun isMcqQuestion(index: Int): Boolean = index % 2 == 1

/** One scored answer, kept in question order for the end-of-session summary. */
data class PracticeResult(
    val cardId: String,
    /** The correct full Greek form shown in feedback and the summary. */
    val greekForm: String,
    val correct: Boolean,
    /** Accent-exact bonus (typed questions only; MCQ answers always false). */
    val exact: Boolean
)

/** Scoring feedback for the question currently on screen (null = unanswered). */
data class AnswerFeedback(
    val correct: Boolean,
    val exact: Boolean
)

data class PracticeState(
    val questions: List<McqData> = emptyList(),
    val currentIndex: Int = 0,
    /** Results in question order; size == currentIndex while unanswered. */
    val results: List<PracticeResult> = emptyList(),
    val isLoading: Boolean = true,
    val isComplete: Boolean = false,
    val isSubmitting: Boolean = false,
    val errorMessage: String? = null,
    /** MCQ: index of the tapped choice (null = unanswered). */
    val selectedChoiceIndex: Int? = null,
    val feedback: AnswerFeedback? = null,
    /** Live text of the typed-answer field (screen-level editing state). */
    val typedInput: String = "",
    // ---- Alphabet Check mode (wave 5) ----
    /** True when this session is the Alphabet Check (nav arg, see AppNavigation). */
    val isAlphabetCheck: Boolean = false,
    /**
     * Check mode only: per-question direction, parallel to [questions].
     * true = glyph shown → pick the name; false = name shown → pick the glyph.
     */
    val checkDirections: List<Boolean> = emptyList(),
    /**
     * Check mode verdict once the attempt has been recorded: true = passed
     * (or gate already satisfied), false = failed, null = not computed yet
     * (in-flight, or not a check session).
     */
    val checkVerdict: Boolean? = null,
    /** Guard so [recordAlphabetCheckResult] fires exactly once per attempt. */
    val checkRecorded: Boolean = false
)

class PracticeViewModel(
    private val savedStateHandle: SavedStateHandle
) : ViewModel() {
    /**
     * Check mode comes from a Boolean nav argument declared (constant default)
     * on the "alphabet-check" destination; absent on the plain "practice"
     * destination. Known deterministically at construction, so the initial
     * state already carries it (loading screen copy depends on it).
     */
    private val isAlphabetCheck: Boolean =
        savedStateHandle.get<Boolean>(ARG_ALPHABET_CHECK) == true

    private val _state = MutableStateFlow(PracticeState(isAlphabetCheck = isAlphabetCheck))
    val state: StateFlow<PracticeState> = _state.asStateFlow()

    init {
        // Practice sessions are ephemeral: SavedStateHandle (in-memory bundle)
        // only, NO DataStore persistence. Unlike review batches there is no
        // scheduling state to protect — on full process death a fresh question
        // set is generated, and already-submitted results are safe in the core.
        val savedQuestions = savedStateHandle.get<String>(KEY_QUESTIONS)
        if (savedQuestions != null) {
            restoreFrom(savedStateHandle)
        } else {
            loadQuestions()
        }
    }

    val currentQuestion: McqData?
        get() = _state.value.questions.getOrNull(_state.value.currentIndex)

    private fun loadQuestions() {
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
                if (isAlphabetCheck) {
                    // Alphabet Check (wave 5): 24 stratified questions, ALL
                    // served as MCQ regardless of index — the typed lane is
                    // off in check mode (keyboard friction at the worst moment,
                    // per design §10).
                    val check = core.generateAlphabetCheck()
                    _state.value = PracticeState(
                        questions = check.map {
                            McqData(it.cardId, it.prompt, it.choices, it.correctIndex)
                        },
                        isLoading = false,
                        isAlphabetCheck = true,
                        checkDirections = check.map { it.glyphToName }
                        // An empty list is NOT completion — the screen shows an
                        // "unavailable" state instead.
                    )
                } else {
                    val questions = core.generatePracticeMcq(SESSION_SIZE)
                    _state.value = PracticeState(
                        questions = questions,
                        isLoading = false
                        // An empty list is NOT completion — the screen shows the
                        // "unlocks after some review" empty state instead.
                    )
                }
                saveSnapshot()
            } catch (e: Exception) {
                Log.e(TAG, "Failed to generate practice questions", e)
                _state.value = _state.value.copy(
                    isLoading = false,
                    errorMessage = if (isAlphabetCheck) {
                        "Couldn't prepare the Alphabet Check. Please try again."
                    } else {
                        "Couldn't start practice. Please try again."
                    }
                )
            }
        }
    }

    /** Retry after a load-time error. */
    fun retryLoad() {
        _state.value = _state.value.copy(isLoading = true, errorMessage = null)
        loadQuestions()
    }

    /** "Practice again" / check-mode "Try again": drop everything and regenerate. */
    fun startNewSession() {
        savedStateHandle.remove<String>(KEY_QUESTIONS)
        savedStateHandle.remove<String>(KEY_INDEX)
        savedStateHandle.remove<String>(KEY_RESULTS)
        savedStateHandle.remove<String>(KEY_SELECTED)
        savedStateHandle.remove<String>(KEY_TYPED)
        savedStateHandle.remove<Boolean>(KEY_CHECK_RECORDED)
        savedStateHandle.remove<Int>(KEY_CHECK_VERDICT)
        _state.value = PracticeState(isLoading = true, isAlphabetCheck = isAlphabetCheck)
        loadQuestions()
    }

    /** MCQ tap: immediate local scoring against [McqData.correctIndex]. */
    fun selectChoice(choiceIndex: Int) {
        val s = _state.value
        if (s.feedback != null || s.isComplete) return // already answered
        val question = currentQuestion ?: return
        if (choiceIndex !in question.choices.indices) return

        val correct = choiceIndex == question.correctIndex
        val result = PracticeResult(
            cardId = question.cardId,
            greekForm = question.correctForm,
            correct = correct,
            exact = false // MCQ answers never earn the accent-exact bonus
        )
        _state.value = s.copy(
            selectedChoiceIndex = choiceIndex,
            feedback = AnswerFeedback(correct = correct, exact = false),
            results = s.results + result,
            errorMessage = null
        )
        saveSnapshot()
        // Check mode: this is also the per-question recycling hook — every
        // answered check question is submitted immediately (misses become
        // FSRS-Again items), exactly as normal MCQ practice does.
        submitResult(question.cardId, correct, exact = false)
    }

    fun onTypedInputChange(input: String) {
        if (_state.value.feedback != null) return // locked once answered
        _state.value = _state.value.copy(typedInput = input)
        savedStateHandle[KEY_TYPED] = input
    }

    /** Typed submit: lenient check first, exact check only when correct. */
    fun submitTypedAnswer() {
        val s = _state.value
        if (s.feedback != null || s.isSubmitting || s.isComplete) return
        val question = currentQuestion ?: return
        val answer = s.typedInput.trim()
        if (answer.isEmpty()) return

        _state.value = s.copy(isSubmitting = true, errorMessage = null)
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current()
            if (core == null) {
                _state.value = _state.value.copy(
                    isSubmitting = false,
                    errorMessage = "Couldn't check your answer. Please try again."
                )
                return@launch
            }
            try {
                val correct = core.checkTypedAnswer(question.cardId, answer)
                // Skip the exact call when the answer is not even lenient-correct.
                val exact = correct && core.typedAnswerExact(question.cardId, answer)
                if (question.cardId != currentQuestion?.cardId) return@launch

                val st = _state.value
                _state.value = st.copy(
                    isSubmitting = false,
                    feedback = AnswerFeedback(correct = correct, exact = exact),
                    results = st.results + PracticeResult(
                        cardId = question.cardId,
                        greekForm = question.correctForm,
                        correct = correct,
                        exact = exact
                    )
                )
                saveSnapshot()
                submitResult(question.cardId, correct, exact)
            } catch (e: Exception) {
                // Hold the question; the user can retry the check.
                Log.e(TAG, "Answer check failed for ${question.cardId}", e)
                _state.value = _state.value.copy(
                    isSubmitting = false,
                    errorMessage = "Couldn't check your answer. Please try again."
                )
            }
        }
    }

    /**
     * Persist one outcome to the core, AFTER the feedback is on screen. A
     * failure only costs a stats tick — the session continues uninterrupted.
     */
    private fun submitResult(cardId: String, correct: Boolean, exact: Boolean) {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                if (!core.submitPracticeResult(cardId, correct, exact)) {
                    Log.w(TAG, "submitPracticeResult not persisted for $cardId")
                }
            } catch (e: Exception) {
                Log.e(TAG, "submitPracticeResult failed for $cardId", e)
            }
        }
    }

    fun continueToNext() {
        val s = _state.value
        if (s.feedback == null) return // must answer first
        val nextIndex = s.currentIndex + 1
        val complete = nextIndex >= s.questions.size
        _state.value = s.copy(
            currentIndex = nextIndex,
            isComplete = complete,
            selectedChoiceIndex = null,
            feedback = null,
            typedInput = "",
            isSubmitting = false,
            errorMessage = null
        )
        saveSnapshot()
        // Check mode: record the attempt outcome (pass/fail latch) exactly
        // once, when the last question is answered and confirmed.
        if (complete && isAlphabetCheck && !s.checkRecorded) {
            recordVerdict()
        }
    }

    /**
     * Check mode only: compute the verdict from the local score and persist it
     * via recordAlphabetCheckResult(correct, total). The returned
     * gate-satisfied-after flag ORs into the verdict, so an already-latched
     * gate (e.g. declared-known) still shows the success state. A core
     * failure degrades to the locally computed pass/fail — the attempt was
     * already fully recycled per-question via submitPracticeResult.
     */
    private fun recordVerdict() {
        val s = _state.value
        val correct = s.results.count { it.correct }
        val total = s.questions.size
        val locallyPassed = total > 0 && correct >= ALPHABET_CHECK_PASS_SCORE
        viewModelScope.launch(Dispatchers.IO) {
            val satisfied = try {
                CoreProvider.current()
                    ?.recordAlphabetCheckResult(correct.toUInt(), total.toUInt()) == true
            } catch (e: Exception) {
                Log.e(TAG, "recordAlphabetCheckResult failed", e)
                false
            }
            _state.value = _state.value.copy(
                checkVerdict = locallyPassed || satisfied,
                checkRecorded = true
            )
            saveSnapshot()
        }
    }

    /** Restore a rotated session, repairing any inconsistent invariants. */
    private fun restoreFrom(handle: SavedStateHandle) {
        val parsed = parseQuestions(handle.get<String>(KEY_QUESTIONS) ?: "[]")
        val questions = parsed.questions
        var index = (handle.get<Int>(KEY_INDEX) ?: 0).coerceIn(0, questions.size)
        var results = parseResults(handle.get<String>(KEY_RESULTS) ?: "[]")
        if (results.size > index + 1) results = results.take(index + 1)
        if (results.size < index) {
            // Missing earlier results can't be reconstructed — rewind to the
            // first unanswered question so results/questions stay aligned.
            index = results.size
        }
        val answered = results.size == index + 1
        val feedback = if (answered) {
            val r = results[index]
            AnswerFeedback(correct = r.correct, exact = r.exact)
        } else {
            null
        }
        val selected = if (answered) {
            (handle.get<Int>(KEY_SELECTED) ?: -1).takeIf { it >= 0 }
        } else {
            null
        }
        val checkRecorded = handle.get<Boolean>(KEY_CHECK_RECORDED) ?: false
        val checkVerdict = when (handle.get<Int>(KEY_CHECK_VERDICT) ?: -1) {
            1 -> true
            0 -> false
            else -> null
        }
        _state.value = PracticeState(
            questions = questions,
            currentIndex = index,
            results = results,
            isLoading = false,
            isComplete = questions.isNotEmpty() && index >= questions.size,
            selectedChoiceIndex = selected,
            feedback = feedback,
            typedInput = if (answered) "" else handle.get<String>(KEY_TYPED) ?: "",
            isAlphabetCheck = isAlphabetCheck,
            checkDirections = parsed.directions,
            checkVerdict = checkVerdict,
            checkRecorded = checkRecorded
        )
        // Death/rotation between the last answer and the verdict persist: the
        // restored attempt is complete but unrecorded → finish the recording.
        if (isAlphabetCheck && _state.value.isComplete && !checkRecorded) {
            recordVerdict()
        }
    }

    /** Snapshot the session into SavedStateHandle (rotation-safe). */
    private fun saveSnapshot() {
        val s = _state.value
        savedStateHandle[KEY_QUESTIONS] = serializeQuestions(s.questions, s.checkDirections)
        savedStateHandle[KEY_INDEX] = s.currentIndex
        savedStateHandle[KEY_RESULTS] = serializeResults(s.results)
        savedStateHandle[KEY_SELECTED] = s.selectedChoiceIndex ?: -1
        savedStateHandle[KEY_TYPED] = s.typedInput
        savedStateHandle[KEY_CHECK_RECORDED] = s.checkRecorded
        savedStateHandle[KEY_CHECK_VERDICT] = when (s.checkVerdict) {
            true -> 1
            false -> 0
            null -> -1
        }
    }

    private fun serializeQuestions(questions: List<McqData>, directions: List<Boolean>): String {
        val arr = JSONArray()
        questions.forEachIndexed { i, q ->
            val choices = JSONArray()
            for (c in q.choices) choices.put(c)
            arr.put(
                JSONObject()
                    .put("cardId", q.cardId)
                    .put("promptGloss", q.promptGloss)
                    .put("choices", choices)
                    .put("correctIndex", q.correctIndex)
                    // Check-mode direction; absent/false reads as a normal question.
                    .put("glyphToName", directions.getOrElse(i) { false })
            )
        }
        return arr.toString()
    }

    private fun parseQuestions(json: String): ParsedQuestions {
        return try {
            val arr = JSONArray(json)
            val questions = (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                val choicesArr = o.getJSONArray("choices")
                McqData(
                    cardId = o.getString("cardId"),
                    promptGloss = o.getString("promptGloss"),
                    choices = (0 until choicesArr.length()).map { choicesArr.getString(it) },
                    correctIndex = o.getInt("correctIndex")
                )
            }
            val directions = (0 until arr.length()).map { i ->
                arr.getJSONObject(i).optBoolean("glyphToName", false)
            }
            ParsedQuestions(questions, directions)
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse saved practice questions", e)
            ParsedQuestions(emptyList(), emptyList())
        }
    }

    private fun serializeResults(results: List<PracticeResult>): String {
        val arr = JSONArray()
        for (r in results) {
            arr.put(
                JSONObject()
                    .put("cardId", r.cardId)
                    .put("greekForm", r.greekForm)
                    .put("correct", r.correct)
                    .put("exact", r.exact)
            )
        }
        return arr.toString()
    }

    private fun parseResults(json: String): List<PracticeResult> {
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                PracticeResult(
                    cardId = o.getString("cardId"),
                    greekForm = o.getString("greekForm"),
                    correct = o.getBoolean("correct"),
                    exact = o.getBoolean("exact")
                )
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse saved practice results", e)
            emptyList()
        }
    }

    /** Decoded questions plus their check-mode directions (parallel arrays). */
    private data class ParsedQuestions(
        val questions: List<McqData>,
        val directions: List<Boolean>
    )

    companion object {
        private const val TAG = "PracticeViewModel"
        private val SESSION_SIZE = 10u

        /** Nav-argument key marking an Alphabet Check session (see AppNavigation). */
        const val ARG_ALPHABET_CHECK = "alphabetCheck"

        /**
         * Questions per check and the pass cutoff (≥22/24 per the frozen
         * wave-5 contract / design §3). The check length itself comes from
         * the core; only the cutoff is mirrored here for scoring + copy.
         */
        const val ALPHABET_CHECK_PASS_SCORE = 22

        private const val KEY_QUESTIONS = "practice_questions"
        private const val KEY_INDEX = "practice_index"
        private const val KEY_RESULTS = "practice_results"
        private const val KEY_SELECTED = "practice_selected"
        private const val KEY_TYPED = "practice_typed"
        private const val KEY_CHECK_RECORDED = "check_recorded"
        private const val KEY_CHECK_VERDICT = "check_verdict"
    }
}
