package com.greekapp.viewmodel

import com.greekapp.core.AlphabetGateState
import com.greekapp.core.AlphabetQuestionData
import com.greekapp.core.GreekCoreApi
import com.greekapp.core.IntervalPreviewData
import com.greekapp.core.McqData
import com.greekapp.core.NostrConfig
import com.greekapp.core.PairedDevice
import com.greekapp.core.PracticeStats
import com.greekapp.core.ShareFlushReport
import com.greekapp.core.SyncStats
import uniffi.uniffi_bindings.AnalysisView
import uniffi.uniffi_bindings.CardView
import uniffi.uniffi_bindings.CardWithStateView
import uniffi.uniffi_bindings.ChapterView
import uniffi.uniffi_bindings.CurriculumView
import uniffi.uniffi_bindings.ReviewStats
import uniffi.uniffi_bindings.SearchResultView
import uniffi.uniffi_bindings.TextSectionView

/**
 * Minimal [GreekCoreApi] stub for host-JVM unit tests: every member throws
 * [NotImplementedError] unless a handler lambda is supplied for it. Lets the
 * ViewModel decision helpers (extracted as companion functions) be exercised
 * without a Rust library, android.util.Log, or Dispatchers.Main.
 */
internal class FakeGreekCore(
    var onSubmitReview: ((cardId: String, rating: UByte) -> Boolean)? = null,
    var onSubmitPracticeResult: ((cardId: String, correct: Boolean, exact: Boolean) -> Boolean)? = null,
    var onRecordAlphabetCheckResult: ((correct: UInt, total: UInt) -> Boolean)? = null,
    var onBuildReviewSession: ((sessionSize: UInt) -> List<CardWithStateView>)? = null,
    var onShareFlushNow: (suspend () -> ShareFlushReport)? = null
) : GreekCoreApi {

    override fun buildReviewSession(sessionSize: UInt): List<CardWithStateView> =
        onBuildReviewSession?.invoke(sessionSize) ?: throw NotImplementedError()

    override fun submitReview(cardId: String, rating: UByte): Boolean =
        onSubmitReview?.invoke(cardId, rating) ?: throw NotImplementedError()

    override fun submitPracticeResult(cardId: String, correct: Boolean, exact: Boolean): Boolean =
        onSubmitPracticeResult?.invoke(cardId, correct, exact) ?: throw NotImplementedError()

    override fun recordAlphabetCheckResult(correct: UInt, total: UInt): Boolean =
        onRecordAlphabetCheckResult?.invoke(correct, total) ?: throw NotImplementedError()

    // ---- Everything below is irrelevant to the tests and simply stubs the
    // abstract members of the interface. ----

    override fun addCardFromLookup(lemma: String, form: String, definition: String): String =
        throw NotImplementedError()

    override fun getChapters(textbook: String): List<ChapterView> = throw NotImplementedError()

    override fun getCurricula(): List<CurriculumView> = throw NotImplementedError()

    override fun getDueCards(limit: Int): List<CardWithStateView> = throw NotImplementedError()

    override fun getKnownWords(): List<String> = throw NotImplementedError()

    override fun getNewCards(textbook: String, chapter: Int, limit: Int): List<CardView> =
        throw NotImplementedError()

    override fun getReviewStats(): ReviewStats = throw NotImplementedError()

    override fun getTextSections(): List<TextSectionView> = throw NotImplementedError()

    override fun isWordKnown(word: String): Boolean = throw NotImplementedError()

    override fun lookupWord(form: String): List<AnalysisView> = throw NotImplementedError()

    override fun markWordKnown(word: String, known: Boolean) = throw NotImplementedError()

    override fun searchDictionary(query: String, limit: Int): List<SearchResultView> =
        throw NotImplementedError()

    override fun unlockChapter(textbook: String, chapter: Int): Boolean = throw NotImplementedError()

    override fun previewIntervals(cardId: String): IntervalPreviewData? = throw NotImplementedError()

    override fun newCardsToday(): UInt = throw NotImplementedError()

    override fun generatePracticeMcq(limit: UInt): List<McqData> = throw NotImplementedError()

    override fun checkTypedAnswer(cardId: String, answer: String): Boolean =
        throw NotImplementedError()

    override fun typedAnswerExact(cardId: String, answer: String): Boolean =
        throw NotImplementedError()

    override fun getPracticeStats(): PracticeStats = throw NotImplementedError()

    override fun generateAlphabetCheck(): List<AlphabetQuestionData> = throw NotImplementedError()

    override fun declareAlphabetKnown(): Boolean = throw NotImplementedError()

    override fun getAlphabetGateState(): AlphabetGateState = throw NotImplementedError()

    override fun syncDeviceId(): String = throw NotImplementedError()

    override fun syncPairedDevices(): List<PairedDevice> = throw NotImplementedError()

    override fun syncExportBackupJson(): String = throw NotImplementedError()

    override fun syncImportBackupJson(json: String): SyncStats = throw NotImplementedError()

    override suspend fun syncNostrPush(): SyncStats = throw NotImplementedError()

    override suspend fun syncNostrPull(): SyncStats = throw NotImplementedError()

    override fun syncNostrConfig(): NostrConfig = throw NotImplementedError()

    override fun syncSetNostrRelays(relaysCsv: String) = throw NotImplementedError()

    override fun syncGenerateNostrIdentity(): String = throw NotImplementedError()

    override fun syncSetNostrNsec(nsec: String) = throw NotImplementedError()

    override suspend fun syncIrohTicket(): String = throw NotImplementedError()

    override suspend fun syncIrohJoin(ticket: String) = throw NotImplementedError()

    override suspend fun syncIrohNow(): SyncStats = throw NotImplementedError()

    override suspend fun shareFlushNow(): ShareFlushReport =
        onShareFlushNow?.invoke() ?: throw NotImplementedError()
}
