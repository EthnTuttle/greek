package com.greekapp.core

import uniffi.uniffi_bindings.AnalysisView
import uniffi.uniffi_bindings.CardView
import uniffi.uniffi_bindings.CardWithStateView
import uniffi.uniffi_bindings.ChapterView
import uniffi.uniffi_bindings.CurriculumView
import uniffi.uniffi_bindings.ReviewStats
import uniffi.uniffi_bindings.SearchResultView
import uniffi.uniffi_bindings.TextSectionView

/**
 * App-side seam over the UniFFI-generated `GreekCore`.
 *
 * Every ViewModel talks to this interface (via [CoreProvider]) instead of
 * `GreekApplication.greekCore` directly, so wave-2 contract additions can be
 * stubbed on the Kotlin side until the UniFFI bindings are regenerated.
 *
 * The first group of methods mirrors the generated
 * `uniffi.uniffi_bindings.GreekCoreInterface` signatures EXACTLY — keep them
 * in sync when the bindings are regenerated.
 */
interface GreekCoreApi {

    // ---- Existing UniFFI exports (must match GreekCoreInterface exactly) ----

    fun addCardFromLookup(lemma: String, form: String, definition: String): String

    fun getChapters(textbook: String): List<ChapterView>

    fun getCurricula(): List<CurriculumView>

    fun getDueCards(limit: Int): List<CardWithStateView>

    fun getKnownWords(): List<String>

    fun getNewCards(textbook: String, chapter: Int, limit: Int): List<CardView>

    fun getReviewStats(): ReviewStats

    fun getTextSections(): List<TextSectionView>

    fun isWordKnown(word: String): Boolean

    fun lookupWord(form: String): List<AnalysisView>

    fun markWordKnown(word: String, known: Boolean)

    fun searchDictionary(query: String, limit: Int): List<SearchResultView>

    fun submitReview(cardId: String, rating: UByte): Boolean

    fun unlockChapter(textbook: String, chapter: Int): Boolean

    // ---- Wave-2 contract additions ----
    // These UniFFI exports do NOT exist in the generated bindings yet.
    // RustCoreAdapter serves them from ContractStubs until regen; see
    // WAVE2-NOTES.md at the repo root.

    /**
     * Preview the next review interval (in seconds) for each rating of a card.
     * Null when the card has no scheduling state yet or lookups are unavailable.
     */
    fun previewIntervals(cardId: String): IntervalPreviewData?

    /** Number of new (never-reviewed) cards the user has studied today. */
    fun newCardsToday(): UInt

    /**
     * Human-readable requirement for unlocking [chapter] (from the wave-2
     * `ChapterView.unlockHint` field), or "" when there is no hint. An empty
     * result means callers should fall back to the legacy behavior of calling
     * [unlockChapter] directly.
     */
    fun unlockHintFor(chapter: ChapterView): String = "" // WAVE2-WIRE: override in RustCoreAdapter to return chapter.unlockHint once the binding exists

    // ---- Wave-3 contract additions (Practice mode) ----
    // These UniFFI exports do NOT exist in the generated bindings yet.
    // RustCoreAdapter serves safe stubs until regen; see WAVE3-NOTES.md at
    // the repo root for the expected export/view names.

    /**
     * Build [limit] auto-generated multiple-choice questions from rated cards.
     * May return fewer (or none) when the question pool is too small — callers
     * must handle an empty list with an "unlocks after some review" empty state.
     */
    fun generatePracticeMcq(limit: UInt): List<McqData>

    /** Accent-insensitive correctness check for a typed answer to [cardId]. */
    fun checkTypedAnswer(cardId: String, answer: String): Boolean

    /** Strict check — the typed answer must match the stored form, accents included. */
    fun typedAnswerExact(cardId: String, answer: String): Boolean

    /**
     * Record one practice outcome for [cardId] (correct = lenient pass,
     * exact = accent-exact bonus). Returns true when the result was persisted.
     */
    fun submitPracticeResult(cardId: String, correct: Boolean, exact: Boolean): Boolean

    /** Lifetime practice totals driving the Home accuracy line. */
    fun getPracticeStats(): PracticeStats

    // ---- Wave-5 contract additions (Alphabet Gate) ----
    // These UniFFI exports do NOT exist in the generated bindings yet.
    // RustCoreAdapter serves safe stubs until regen; see WAVE5-NOTES.md at
    // the repo root for the expected export/view names.

    /**
     * Build the 24-question Alphabet Check (stratified MCQ over the letters
     * track, mixed glyph→name / name→glyph). May return empty when the
     * letters deck isn't loaded — callers show an "unavailable" state.
     */
    fun generateAlphabetCheck(): List<AlphabetQuestionData>

    /**
     * Record the outcome of one completed Alphabet Check attempt.
     * Returns the gate-satisfied state after recording (true on pass, or when
     * the gate was already latched satisfied). Misses are expected to have
     * been recycled into FSRS via [submitPracticeResult] per question.
     */
    fun recordAlphabetCheckResult(correct: UInt, total: UInt): Boolean

    /**
     * "I already know the alphabet" lane: explicitly satisfy the gate.
     * Logged core-side as self-reported. Returns the resulting gate state.
     */
    fun declareAlphabetKnown(): Boolean

    /**
     * Current gate evaluation: satisfied flag, satisfying [AlphabetGateState.method]
     * ("" when none), and live letters-track counters for the progress UI.
     */
    fun getAlphabetGateState(): AlphabetGateState
}

/**
 * Kotlin-side mirror of the wave-2 `IntervalPreviewView` UniFFI record.
 * Field names and types (UInt seconds) match the planned export exactly.
 */
data class IntervalPreviewData(
    val againSecs: UInt,
    val hardSecs: UInt,
    val goodSecs: UInt,
    val easySecs: UInt
)

/**
 * Kotlin-side mirror of the wave-3 `McqView` UniFFI record. Field names and
 * types match the planned export (generated records expose camelCase fields).
 * Self-contained in the seam package; mapping from generated views happens in
 * RustCoreAdapter at integration time.
 *
 * An MCQ question AND a typed-answer question can both be served from one
 * McqData: typed mode ignores [choices] except that
 * `choices[correctIndex]` is always the correct full Greek form to display.
 */
data class McqData(
    val cardId: String,
    val promptGloss: String,
    val choices: List<String>,
    val correctIndex: Int
) {
    /** The correct full Greek form (always present at [correctIndex]). */
    val correctForm: String
        get() = choices.getOrElse(correctIndex) { "" }
}

/**
 * Kotlin-side mirror of the wave-3 `PracticeStatsView` UniFFI record
 * (counts arrive as u32 → UInt).
 */
data class PracticeStats(
    val total: UInt,
    val correct: UInt
)

/**
 * Kotlin-side mirror of the wave-5 `AlphabetQuestionView` UniFFI record.
 * Field names match the frozen contract (generated records expose camelCase);
 * `correctIndex` arrives as UByte in the generated view and is mapped to Int
 * here (see WAVE5-NOTES.md).
 *
 * [prompt] holds the presented side (glyph pair or letter name) and
 * [glyphToName] says which direction this question runs, so the UI can label
 * it ("Name this letter" / "Pick the matching letter"). `choices[correctIndex]`
 * is always the correct answer.
 */
data class AlphabetQuestionData(
    val cardId: String,
    val glyphToName: Boolean,
    val prompt: String,
    val choices: List<String>,
    val correctIndex: Int
) {
    /** The correct choice text (always present at [correctIndex]). */
    val correctChoice: String
        get() = choices.getOrElse(correctIndex) { "" }
}

/**
 * Kotlin-side mirror of the wave-5 `AlphabetGateStateView` UniFFI record
 * (counters arrive as u32 → UInt). [method] is "" while the gate is unsatisfied
 * by any lane; otherwise one of `passed-check` / `passed-organic` /
 * `declared-known` / `vocab-history`.
 */
data class AlphabetGateState(
    val satisfied: Boolean,
    val method: String,
    val lettersIntroduced: UInt,
    val lettersTotal: UInt,
    val lettersPracticeTotal: UInt,
    val lettersPracticeCorrect: UInt,
    val lettersActiveDays: UInt,
    val lettersChaptersMet: UInt
)
