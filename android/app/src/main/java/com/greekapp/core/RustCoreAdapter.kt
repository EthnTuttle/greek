package com.greekapp.core

import com.greekapp.GreekApplication
import uniffi.uniffi_bindings.AnalysisView
import uniffi.uniffi_bindings.CardView
import uniffi.uniffi_bindings.CardWithStateView
import uniffi.uniffi_bindings.ChapterView
import uniffi.uniffi_bindings.CurriculumView
import uniffi.uniffi_bindings.GreekCore
import uniffi.uniffi_bindings.ReviewStats
import uniffi.uniffi_bindings.SearchResultView
import uniffi.uniffi_bindings.TextSectionView

/**
 * [GreekCoreApi] implementation that delegates to the UniFFI-generated
 * [GreekCore]. Wave-2 exports ([previewIntervals], [newCardsToday],
 * [unlockHintFor]) call the real bindings, falling back to [ContractStubs]
 * permanently if the loaded native library predates them.
 */
class RustCoreAdapter(private val core: GreekCore) : GreekCoreApi {

    override fun addCardFromLookup(lemma: String, form: String, definition: String): String =
        core.addCardFromLookup(lemma, form, definition)

    override fun getChapters(textbook: String): List<ChapterView> =
        core.getChapters(textbook)

    override fun getCurricula(): List<CurriculumView> =
        core.getCurricula()

    override fun getDueCards(limit: Int): List<CardWithStateView> =
        core.getDueCards(limit)

    override fun getKnownWords(): List<String> =
        core.getKnownWords()

    override fun getNewCards(textbook: String, chapter: Int, limit: Int): List<CardView> =
        core.getNewCards(textbook, chapter, limit)

    override fun getReviewStats(): ReviewStats =
        core.getReviewStats()

    override fun getTextSections(): List<TextSectionView> =
        core.getTextSections()

    override fun isWordKnown(word: String): Boolean =
        core.isWordKnown(word)

    override fun lookupWord(form: String): List<AnalysisView> =
        core.lookupWord(form)

    override fun markWordKnown(word: String, known: Boolean) =
        core.markWordKnown(word, known)

    override fun searchDictionary(query: String, limit: Int): List<SearchResultView> =
        core.searchDictionary(query, limit)

    override fun submitReview(cardId: String, rating: UByte): Boolean =
        core.submitReview(cardId, rating)

    override fun unlockChapter(textbook: String, chapter: Int): Boolean =
        core.unlockChapter(textbook, chapter)

    override fun previewIntervals(cardId: String): IntervalPreviewData? {
        if (!newApiAvailable) return ContractStubs.previewIntervals(cardId)
        return runCatching {
            core.previewIntervals(cardId)?.let {
                IntervalPreviewData(it.againSecs, it.hardSecs, it.goodSecs, it.easySecs)
            }
        }.getOrElse { t ->
            markApiUnavailable("previewIntervals", t)
            ContractStubs.previewIntervals(cardId)
        }
    }

    override fun newCardsToday(): UInt {
        if (!newApiAvailable) return ContractStubs.newCardsToday()
        return runCatching { core.newCardsToday() }
            .getOrElse { markApiUnavailable("newCardsToday", it); ContractStubs.newCardsToday() }
    }

    override fun unlockHintFor(chapter: ChapterView): String {
        if (!newApiAvailable) return ""
        return runCatching { chapter.unlockHint }
            .getOrElse { markApiUnavailable("unlockHintFor", it); "" }
    }

    private companion object {
        /**
         * Set to false permanently the first time a wave-2 export throws
         * (e.g. an older on-device libuniffi_bindings.so lacks the symbol and
         * raising UnsatisfiedLinkError at call time). Subsequent calls serve
         * the stub values, never attempting the native call again.
         */
        @Volatile
        var newApiAvailable: Boolean = true

        fun markApiUnavailable(api: String, t: Throwable) {
            newApiAvailable = false
            android.util.Log.w("RustCoreAdapter", "wave-2 export $api unavailable; using stub fallback", t)
        }
    }
}

/**
 * Fallback implementations for the wave-2 UniFFI exports, used only when the
 * loaded native library predates binding regeneration (see
 * [RustCoreAdapter.newApiAvailable]). Values must stay harmless placeholders.
 */
internal object ContractStubs {

    fun previewIntervals(cardId: String): IntervalPreviewData? {
        // Plausible placeholder ladder: 10m / 1h / 1d / 4d (hard < good < easy).
        return IntervalPreviewData(
            againSecs = 600u,
            hardSecs = 3600u,
            goodSecs = 86400u,
            easySecs = 345600u
        )
    }

    fun newCardsToday(): UInt = 0u
}

/**
 * App-wide access point for the current [GreekCoreApi].
 *
 * The integration wave sets [core] once the Rust core is ready
 * (see WAVE2-NOTES.md). Until then, [current] lazily wraps
 * `GreekApplication.greekCore` so all existing call sites keep working.
 */
object CoreProvider {

    @Volatile
    var core: GreekCoreApi? = null

    /**
     * The explicitly provided core, or — when none was set — a [RustCoreAdapter]
     * wrapping `GreekApplication.greekCore` if the Rust core is initialized.
     * Null when the core isn't ready; callers keep their existing null-core
     * error paths.
     */
    fun current(): GreekCoreApi? {
        core?.let { return it }
        val raw = GreekApplication.greekCore ?: return null
        return RustCoreAdapter(raw)
    }
}
