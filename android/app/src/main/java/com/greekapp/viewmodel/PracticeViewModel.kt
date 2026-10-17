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
    val typedInput: String = ""
)

class PracticeViewModel(
    private val savedStateHandle: SavedStateHandle
) : ViewModel() {
    private val _state = MutableStateFlow(PracticeState())
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
                val questions = core.generatePracticeMcq(SESSION_SIZE)
                _state.value = PracticeState(
                    questions = questions,
                    isLoading = false
                    // An empty list is NOT completion — the screen shows the
                    // "unlocks after some review" empty state instead.
                )
                saveSnapshot()
            } catch (e: Exception) {
                Log.e(TAG, "Failed to generate practice questions", e)
                _state.value = _state.value.copy(
                    isLoading = false,
                    errorMessage = "Couldn't start practice. Please try again."
                )
            }
        }
    }

    /** Retry after a load-time error. */
    fun retryLoad() {
        _state.value = _state.value.copy(isLoading = true, errorMessage = null)
        loadQuestions()
    }

    /** "Practice again" on the summary screen: drop everything and regenerate. */
    fun startNewSession() {
        savedStateHandle.remove<String>(KEY_QUESTIONS)
        savedStateHandle.remove<String>(KEY_INDEX)
        savedStateHandle.remove<String>(KEY_RESULTS)
        savedStateHandle.remove<String>(KEY_SELECTED)
        savedStateHandle.remove<String>(KEY_TYPED)
        _state.value = PracticeState(isLoading = true)
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
    }

    /** Restore a rotated session, repairing any inconsistent invariants. */
    private fun restoreFrom(handle: SavedStateHandle) {
        val questions = parseQuestions(handle.get<String>(KEY_QUESTIONS) ?: "[]")
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
        _state.value = PracticeState(
            questions = questions,
            currentIndex = index,
            results = results,
            isLoading = false,
            isComplete = questions.isNotEmpty() && index >= questions.size,
            selectedChoiceIndex = selected,
            feedback = feedback,
            typedInput = if (answered) "" else handle.get<String>(KEY_TYPED) ?: ""
        )
    }

    /** Snapshot the session into SavedStateHandle (rotation-safe). */
    private fun saveSnapshot() {
        val s = _state.value
        savedStateHandle[KEY_QUESTIONS] = serializeQuestions(s.questions)
        savedStateHandle[KEY_INDEX] = s.currentIndex
        savedStateHandle[KEY_RESULTS] = serializeResults(s.results)
        savedStateHandle[KEY_SELECTED] = s.selectedChoiceIndex ?: -1
        savedStateHandle[KEY_TYPED] = s.typedInput
    }

    private fun serializeQuestions(questions: List<McqData>): String {
        val arr = JSONArray()
        for (q in questions) {
            val choices = JSONArray()
            for (c in q.choices) choices.put(c)
            arr.put(
                JSONObject()
                    .put("cardId", q.cardId)
                    .put("promptGloss", q.promptGloss)
                    .put("choices", choices)
                    .put("correctIndex", q.correctIndex)
            )
        }
        return arr.toString()
    }

    private fun parseQuestions(json: String): List<McqData> {
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                val choicesArr = o.getJSONArray("choices")
                McqData(
                    cardId = o.getString("cardId"),
                    promptGloss = o.getString("promptGloss"),
                    choices = (0 until choicesArr.length()).map { choicesArr.getString(it) },
                    correctIndex = o.getInt("correctIndex")
                )
            }
        } catch (e: Exception) {
            Log.e(TAG, "Failed to parse saved practice questions", e)
            emptyList()
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

    companion object {
        private const val TAG = "PracticeViewModel"
        private val SESSION_SIZE = 10u
        private const val KEY_QUESTIONS = "practice_questions"
        private const val KEY_INDEX = "practice_index"
        private const val KEY_RESULTS = "practice_results"
        private const val KEY_SELECTED = "practice_selected"
        private const val KEY_TYPED = "practice_typed"
    }
}
