package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import com.greekapp.core.CoreProvider
import com.greekapp.ui.components.GreekTextOps
import com.greekapp.ui.components.GreekTypingGrader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray

/**
 * Wave-9 typed_letter drill (DESIGN-greek-keyboard.md, Appendix R —
 * "Drill modality").
 *
 * Prompt by letter NAME / Erasmian sound description, never the target
 * glyph (audio→type is production; glyph→glyph is copywork). The letters
 * deck ships no audio assets, so the prompt is the text name — no TTS is
 * synthesized behind the learner's back. Answers are single graphemes and
 * auto-submit on the first complete grapheme.
 *
 * Grading: STRICT on breathings / iota subscript / diaeresis / final-sigma
 * folding, LENIENT on pitch accents — via [GreekTypingGrader], mirroring
 * the will-be-Rust rule.
 *
 * Outcomes recycle through `submitPracticeResult` (same sink as Practice),
 * which keeps letters-track FSRS state and gate counters honest.
 */
data class LetterQuestion(
    val cardId: String,
    /** Text prompt derived from the card's back (name / sound description),
     *  with the target glyph stripped out. */
    val prompt: String,
    /** The lowercase single-grapheme target the learner must produce. */
    val target: String
)

data class LetterDrillState(
    val questions: List<LetterQuestion> = emptyList(),
    val index: Int = 0,
    val input: String = "",
    /** null = unanswered; true = graded correct. */
    val correct: Boolean? = null,
    val score: Int = 0,
    val isLoading: Boolean = true,
    val isComplete: Boolean = false,
    val unavailable: Boolean = false
)

class LetterDrillViewModel : ViewModel() {
    private val _state = MutableStateFlow(LetterDrillState())
    val state: StateFlow<LetterDrillState> = _state.asStateFlow()

    init { loadQuestions() }

    private fun loadQuestions() {
        viewModelScope.launch(Dispatchers.IO) {
            val questions = try {
                loadLetterQuestions().shuffled().take(SESSION_SIZE)
            } catch (e: Exception) {
                Log.e(TAG, "Failed to load letters deck for typing drill", e)
                emptyList()
            }
            _state.value = if (questions.isEmpty()) {
                LetterDrillState(isLoading = false, unavailable = true)
            } else {
                LetterDrillState(questions = questions, isLoading = false)
            }
        }
    }

    /** Typed-letter drill is unlocked per-letter the moment the letters deck
     *  is present (alphabet gate handles curriculum gating separately). */
    private fun loadLetterQuestions(): List<LetterQuestion> {
        val json = GreekApplication.appContext.assets
            .open("cards/letters.json").bufferedReader().use { it.readText() }
        val arr = JSONArray(json)
        val out = mutableListOf<LetterQuestion>()
        for (i in 0 until arr.length()) {
            val o = arr.getJSONObject(i)
            if (o.optString("card_type") != "Letter") continue
            val front = o.getString("front").trim()
            // Fronts with more than one space (the multi-glyph "ζ ξ ψ" recall
            // card) aren't single-letter drills.
            if (front.count { it == ' ' } > 1) continue
            // Target: the lowercase half of an "Α α" front (or the whole front
            // for the diacritic exemplars, which are already lowercase). Sigma
            // card is "Σ σ/ς": the bare sigma is typed; final-sigma folding in
            // the grader accepts either form.
            val target = if (' ' in front) {
                front.substringAfterLast(' ').substringBefore('/')
            } else {
                front
            }
            // Single-grapheme drills only: diphthong cards (αι, ευ, …) and the
            // multi-glyph "ζ ξ ψ" card are excluded.
            if (GreekTextOps.graphemeCount(target) != 1) continue
            val rawBack = o.getString("back")
            val name = rawBack.substringBefore('—').substringBefore('.').trim()
                // Never leak the target glyph in the prompt.
                .replace(Regex("[\\u0370-\\u03FF\\u1F00-\\u1FFF]+"), "")
                .replace(Regex("[\\s,:;=/()''-]+$"), "")
                .replace(Regex("\\s+"), " ")
                .trim()
            if (name.isEmpty()) continue
            out.add(LetterQuestion(o.getString("id"), name, target))
        }
        return out
    }

    /**
     * Field edits; a single complete grapheme auto-submits immediately
     * (Appendix R: single-grapheme auto-submit, dedicated field).
     */
    fun onInputChange(input: String) {
        val s = _state.value
        if (s.correct != null || s.isComplete) return
        _state.value = s.copy(input = input)
        if (GreekTextOps.graphemeCount(input.trim()) == 1 && input.isNotBlank()) {
            submit()
        }
    }

    private fun submit() {
        val s = _state.value
        if (s.correct != null) return
        val question = s.questions.getOrNull(s.index) ?: return
        val answer = s.input.trim()
        if (answer.isEmpty()) return
        val correct = GreekTypingGrader.isCloseMatch(question.target, answer)
        val exact = GreekTypingGrader.isExactMatch(question.target, answer)
        _state.value = s.copy(
            correct = correct,
            score = s.score + (if (correct) 1 else 0)
        )
        // Same persistence sink as Practice (additive; fire-and-forget).
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                core.submitPracticeResult(question.cardId, correct, exact)
            } catch (e: Exception) {
                Log.w(TAG, "submitPracticeResult failed for ${question.cardId}", e)
            }
        }
    }

    fun continueToNext() {
        val s = _state.value
        if (s.correct == null) return
        val next = s.index + 1
        _state.value = s.copy(
            index = next,
            input = "",
            correct = null,
            isComplete = next >= s.questions.size
        )
    }

    fun restart() {
        _state.value = LetterDrillState(isLoading = true)
        loadQuestions()
    }

    private companion object {
        const val TAG = "LetterDrillViewModel"
        const val SESSION_SIZE = 10
    }
}
