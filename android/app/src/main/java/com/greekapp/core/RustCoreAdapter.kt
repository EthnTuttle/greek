package com.greekapp.core

import com.greekapp.GreekApplication
import uniffi.uniffi_bindings.AnalysisView
import uniffi.uniffi_bindings.CardView
import uniffi.uniffi_bindings.CardWithStateView
import uniffi.uniffi_bindings.ChapterView
import uniffi.uniffi_bindings.CurriculumView
import uniffi.uniffi_bindings.GreekCore
import uniffi.uniffi_bindings.NostrConfigView
import uniffi.uniffi_bindings.PairedDeviceView
import uniffi.uniffi_bindings.ReviewStats
import uniffi.uniffi_bindings.SearchResultView
import uniffi.uniffi_bindings.SyncStatsView
import uniffi.uniffi_bindings.TextSectionView

/**
 * [GreekCoreApi] implementation that delegates to the UniFFI-generated
 * [GreekCore]. Wave-2/3 exports call the real bindings, falling back to
 * safe defaults permanently if the loaded native library predates them.
 * Wave-5 (alphabet gate) exports do not exist in the checked-in bindings
 * yet, so those overrides are pure stubs marked `WAVE5-WIRE` — integration
 * rewires them using the same runCatching + [newApiAvailable] pattern.
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

    // ---- Wave-3 (Practice mode) ----
    // Real UniFFI exports with the same newApiAvailable fallback as wave-2:
    // an older on-device .so lacking these symbols throws at call time once,
    // then we permanently serve safe defaults.

    override fun generatePracticeMcq(limit: UInt): List<McqData> {
        if (!newApiAvailable) return emptyList()
        return runCatching {
            core.generatePracticeMcq(limit).map {
                McqData(it.cardId, it.promptGloss, it.choices, it.correctIndex.toInt())
            }
        }.getOrElse { markApiUnavailable("generatePracticeMcq", it); emptyList() }
    }

    override fun checkTypedAnswer(cardId: String, answer: String): Boolean {
        if (!newApiAvailable) return false
        return runCatching { core.checkTypedAnswer(cardId, answer) }
            .getOrElse { markApiUnavailable("checkTypedAnswer", it); false }
    }

    override fun typedAnswerExact(cardId: String, answer: String): Boolean {
        if (!newApiAvailable) return false
        return runCatching { core.typedAnswerExact(cardId, answer) }
            .getOrElse { markApiUnavailable("typedAnswerExact", it); false }
    }

    override fun submitPracticeResult(cardId: String, correct: Boolean, exact: Boolean): Boolean {
        if (!newApiAvailable) return false
        return runCatching { core.submitPracticeResult(cardId, correct, exact) }
            .getOrElse { markApiUnavailable("submitPracticeResult", it); false }
    }

    override fun getPracticeStats(): PracticeStats {
        if (!newApiAvailable) return PracticeStats(0u, 0u)
        return runCatching { core.getPracticeStats().let { PracticeStats(it.total, it.correct) } }
            .getOrElse { markApiUnavailable("getPracticeStats", it); PracticeStats(0u, 0u) }
    }

    // ---- Wave-5 (Alphabet Gate) ----
    // Real exports, same runCatching/newApiAvailable fallback as wave-2/3.
    // Fallback default is fail-closed-but-renderable: unmet gate over the
    // standard 43-card deck so the gate surfaces stay visible if an older
    // on-device .so lacks these symbols.

    override fun generateAlphabetCheck(): List<AlphabetQuestionData> {
        if (!newApiAvailable) return emptyList()
        return runCatching {
            core.generateAlphabetCheck().map {
                AlphabetQuestionData(it.cardId, it.glyphToName, it.prompt, it.choices, it.correctIndex.toInt())
            }
        }.getOrElse { markApiUnavailable("generateAlphabetCheck", it); emptyList() }
    }

    override fun recordAlphabetCheckResult(correct: UInt, total: UInt): Boolean {
        if (!newApiAvailable) return false
        return runCatching { core.recordAlphabetCheckResult(correct, total) }
            .getOrElse { markApiUnavailable("recordAlphabetCheckResult", it); false }
    }

    override fun declareAlphabetKnown(): Boolean {
        if (!newApiAvailable) return false
        return runCatching { core.declareAlphabetKnown() }
            .getOrElse { markApiUnavailable("declareAlphabetKnown", it); false }
    }

    override fun getAlphabetGateState(): AlphabetGateState {
        if (!newApiAvailable) return gateStubFallback
        return runCatching {
            core.getAlphabetGateState().let {
                AlphabetGateState(it.satisfied, it.method, it.lettersIntroduced,
                    it.lettersTotal, it.lettersPracticeTotal, it.lettersPracticeCorrect,
                    it.lettersActiveDays, it.lettersChaptersMet)
            }
        }.getOrElse { markApiUnavailable("getAlphabetGateState", it); gateStubFallback }
    }

    // ---- Wave-6 (Device Sync) ----
    // Real exports. Fail-soft getters use runCatching + newApiAvailable;
    // actions (import/push/pull/join/nsec/ticket) let exceptions propagate
    // so the UI can surface real sync errors, catching only the
    // missing-.so-symbol case to mark the library stale first.

    override fun syncDeviceId(): String {
        if (!newApiAvailable) return "unknown"
        return runCatching { core.syncDeviceId() }
            .getOrElse { markApiUnavailable("syncDeviceId", it); "unknown" }
    }

    override fun syncPairedDevices(): List<PairedDevice> {
        if (!newApiAvailable) return emptyList()
        return runCatching {
            core.syncPairedDevices().map {
                PairedDevice(it.deviceId, it.label, it.pairedAt, it.lastSyncHlc)
            }
        }.getOrElse { markApiUnavailable("syncPairedDevices", it); emptyList() }
    }

    override fun syncExportBackupJson(): String =
        syncAction("syncExportBackupJson") { core.syncExportBackupJson() }

    /** Sync actions delegate to real exports; only a missing native symbol degrades. */
    private fun <T> syncAction(name: String, block: () -> T): T {
        return try {
            block()
        } catch (e: UnsatisfiedLinkError) {
            markApiUnavailable(name, e)
            throw UnsupportedOperationException("Sync unavailable: stale native library", e)
        }
    }

    /** Suspend variant of [syncAction] for UniFFI async exports. */
    private suspend fun <T> syncCall(name: String, block: suspend () -> T): T {
        return try {
            block()
        } catch (e: UnsatisfiedLinkError) {
            markApiUnavailable(name, e)
            throw UnsupportedOperationException("Sync unavailable: stale native library", e)
        }
    }

    private fun SyncStatsView.toSyncStats() =
        SyncStats(sent = reviewsSent.toInt(), received = reviewsReceived.toInt(), settings = settingsApplied.toInt())

    override fun syncImportBackupJson(json: String): SyncStats =
        syncAction("syncImportBackupJson") { core.syncImportBackupJson(json).toSyncStats() }

    override suspend fun syncNostrPush(): SyncStats =
        syncCall("syncNostrPush") { core.syncNostrPush().toSyncStats() }

    override suspend fun syncNostrPull(): SyncStats =
        syncCall("syncNostrPull") { core.syncNostrPull().toSyncStats() }

    override fun syncNostrConfig(): NostrConfig {
        if (!newApiAvailable) return NostrConfig(relaysCsv = "", hasIdentity = false)
        return runCatching { core.syncNostrConfig().let { NostrConfig(it.relaysCsv, it.hasIdentity) } }
            .getOrElse { markApiUnavailable("syncNostrConfig", it); NostrConfig(relaysCsv = "", hasIdentity = false) }
    }

    override fun syncSetNostrRelays(relaysCsv: String) {
        syncAction("syncSetNostrRelays") { core.syncSetNostrRelays(relaysCsv) }
    }

    override fun syncGenerateNostrIdentity(): String {
        return syncAction("syncGenerateNostrIdentity") { core.syncGenerateNostrIdentity() }
    }

    override fun syncSetNostrNsec(nsec: String) {
        syncAction("syncSetNostrNsec") { core.syncSetNostrNsec(nsec) }
    }

    override suspend fun syncIrohTicket(): String {
        return syncCall("syncIrohTicket") { core.syncIrohTicket() }
    }

    override suspend fun syncIrohJoin(ticket: String) {
        syncCall("syncIrohJoin") { core.syncIrohJoin(ticket) }
    }

    override suspend fun syncIrohNow(): SyncStats {
        return syncCall("syncIrohNow") { core.syncIrohNow().toSyncStats() }
    }

    // ---- Wave-9 (Progress sharing over nostr) ----
    // The share-nostr exports don't exist in the checked-in generated
    // bindings yet, so unlike waves 2–6 there is no static symbol to call:
    // each method is looked up reflectively on the loaded GreekCore class —
    // a NoSuchMethodException is this wave's UnsatisfiedLinkError, tripping
    // the same one-shot [shareApiAvailable] latch as the sync helpers. After
    // bindings regen these bodies keep working unchanged; replacing the
    // reflection with direct calls later is a mechanical cleanup (WAVE9-WIRE).
    // The latch is SEPARATE from newApiAvailable: a stale share surface must
    // never degrade the review/sync surfaces.
    //
    // Getters mirror the fail-soft sync getters (safe defaults, never throw).
    // Actions mirror syncAction: only the stale-bindings case degrades; real
    // engine errors (relay validation, content policy, identity missing)
    // propagate — unwrapped from InvocationTargetException — so the UI can
    // show the engine's own reason strings verbatim (Appendix R honesty).
    //
    // WAVE9-WIRE note for integration: if the Rust side declares an export as
    // async (e.g. shareFlushNow), UniFFI generates a Kotlin suspend fun whose
    // JVM signature gains a trailing kotlin.coroutines.Continuation param —
    // extend the lookup below with the Continuation-aware variant that the
    // sync suspend exports would use, and make the seam method suspend.

    /** JNI-visible getter name for a camelCase record field (get + Capitalized). */
    private fun javaGetter(field: String) =
        "get" + field.replaceFirstChar { it.uppercaseChar() }

    /**
     * Reflectively read one property off a generated UniFFI record view.
     * Unsigned fields (UInt/ULong/… ) compile to inline classes, so their
     * JVM accessors are NAME-MANGLED (`getPosted-pVg5ArA` style) — fall back
     * to a prefix match that ignores the mangling suffix.
     */
    private fun propOf(view: Any, field: String): Any? {
        val getter = javaGetter(field)
        val method = view.javaClass.methods.firstOrNull { m ->
            m.parameterCount == 0 &&
                (m.name == getter || m.name.startsWith("$getter-"))
        } ?: throw NoSuchMethodException("$getter on ${view.javaClass.simpleName}")
        return method.invoke(view)
    }

    private fun propBool(view: Any, field: String): Boolean =
        propOf(view, field) as? Boolean ?: false

    private fun propStr(view: Any, field: String): String =
        propOf(view, field) as? String ?: ""

    private fun propStrList(view: Any, field: String): List<String> =
        (propOf(view, field) as? List<*>)?.mapNotNull { it as? String } ?: emptyList()

    /** UniFFI longs arrive as ULong for u64 / Long for i64; accept any Number. */
    private fun propLong(view: Any, field: String): Long =
        (propOf(view, field) as? Number)?.toLong() ?: 0L

    /**
     * Look up [name] on the generated core and invoke it. Throws
     * [UnsupportedOperationException] (after latching share unavailable) when
     * the bindings predate wave-9; unwraps InvocationTargetException so real
     * engine exceptions reach the caller untouched.
     */
    private fun shareInvoke(name: String, paramTypes: Array<Class<*>>, vararg args: Any?): Any? {
        var method = try {
            core.javaClass.getMethod(name, *paramTypes)
        } catch (e: NoSuchMethodException) {
            // Unsigned-returning functions (e.g. sharePendingCount(): UInt)
            // are JVM name-mangled; retry with a prefix match.
            val mangled = core.javaClass.methods.firstOrNull { m ->
                m.name.startsWith("$name-") && m.parameterTypes.contentEquals(paramTypes)
            }
            mangled ?: run {
                markShareUnavailable(name, e)
                throw UnsupportedOperationException("Share unavailable: stale native library", e)
            }
        }
        return try {
            method.invoke(core, *args)
        } catch (e: java.lang.reflect.InvocationTargetException) {
            throw e.targetException ?: e
        }
    }

    /**
     * Fail-soft getter path: catches both the stale-bindings case and real
     * engine throws, latching the share surface off on any failure.
     */
    private fun <T> shareGet(name: String, paramTypes: Array<Class<*>>, fallback: T, vararg args: Any?, map: (Any?) -> T): T {
        if (!ShareBindStatus.available) return fallback
        return runCatching { map(shareInvoke(name, paramTypes, *args)) }
            .getOrElse { t ->
                if (t !is UnsupportedOperationException) markShareUnavailable(name, t)
                fallback
            }
    }

    override fun shareConfigGet(): ShareConfig =
        shareGet("shareConfigGet", emptyArray(), ShareConfig.DISABLED) { view ->
            if (view == null) ShareConfig.DISABLED else ShareConfig(
                enabled = propBool(view, "enabled"),
                pubkeyHex = propStr(view, "pubkeyHex"),
                relays = propStrList(view, "relays")
            )
        }

    override fun shareEnable(relays: List<String>) {
        shareInvoke("shareEnable", arrayOf<Class<*>>(List::class.java), relays)
    }

    override fun shareDisable(forgetIdentity: Boolean) {
        val types = arrayOf<Class<*>>(Boolean::class.javaPrimitiveType!!)
        shareInvoke("shareDisable", types, forgetIdentity)
    }

    override fun shareRelaysSet(relays: List<String>) {
        shareInvoke("shareRelaysSet", arrayOf<Class<*>>(List::class.java), relays)
    }

    override fun shareMilestones(): List<ShareMilestone> =
        shareGet("shareMilestones", emptyArray(), emptyList()) { views ->
            (views as? List<*>)?.mapNotNull { view ->
                view?.let {
                    ShareMilestone(
                        slug = propStr(it, "slug"),
                        titleEn = propStr(it, "titleEn"),
                        titleEl = propStr(it, "titleEl"),
                        templateEn = propStr(it, "templateEn"),
                        templateEl = propStr(it, "templateEl"),
                        customTemplateEn = propOf(it, "customTemplateEn") as? String,
                        customTemplateEl = propOf(it, "customTemplateEl") as? String,
                        enabled = propBool(it, "enabled"),
                        lastPostedMs = propLong(it, "lastPostedMs"),
                        lastState = propStr(it, "lastState")
                    )
                }
            } ?: emptyList()
        }

    override fun shareSetTemplate(slug: String, locale: String, text: String) {
        val types = arrayOf<Class<*>>(String::class.java, String::class.java, String::class.java)
        shareInvoke("shareSetTemplate", types, slug, locale, text)
    }

    override fun shareResetTemplate(slug: String, locale: String) {
        val types = arrayOf<Class<*>>(String::class.java, String::class.java)
        shareInvoke("shareResetTemplate", types, slug, locale)
    }

    override fun shareSetMilestoneEnabled(slug: String, enabled: Boolean) {
        val types = arrayOf<Class<*>>(String::class.java, Boolean::class.javaPrimitiveType!!)
        shareInvoke("shareSetMilestoneEnabled", types, slug, enabled)
    }

    override fun sharePreview(slug: String, suffix: String): String {
        val types = arrayOf<Class<*>>(String::class.java, String::class.java)
        // Not fail-soft by design: the editor needs the engine's verbatim
        // rejection string, and the enable flow needs a real render.
        return shareInvoke("sharePreview", types, slug, suffix) as? String ?: ""
    }

    override fun shareFlushNow(): ShareFlushReport =
        shareGet("shareFlushNow", emptyArray(), ShareFlushReport.EMPTY) { view ->
            if (view == null) ShareFlushReport.EMPTY else ShareFlushReport(
                posted = propLong(view, "posted").toInt().coerceAtLeast(0).toUInt(),
                deferred = propLong(view, "deferred").toInt().coerceAtLeast(0).toUInt(),
                duplicates = propLong(view, "duplicates").toInt().coerceAtLeast(0).toUInt(),
                failed = propStrList(view, "failed")
            )
        }

    override fun sharePendingCount(): UInt =
        shareGet("sharePendingCount", emptyArray(), 0u) { count ->
            (count as? Number)?.toLong()?.coerceAtLeast(0L)?.toUInt() ?: 0u
        }

    override fun shareHistory(slug: String): List<ShareHistoryEntry> =
        shareGet("shareHistory", arrayOf<Class<*>>(String::class.java), emptyList(), slug) { views ->
            (views as? List<*>)?.mapNotNull { view ->
                view?.let {
                    ShareHistoryEntry(
                        eventId = propStr(it, "eventId"),
                        createdMs = propLong(it, "createdMs"),
                        state = propStr(it, "state"),
                        perRelayJson = propStr(it, "perRelayJson")
                    )
                }
            } ?: emptyList()
        }

    /**
     * One-shot latch check for the wave-9 share surface ONLY (separate from
     * [newApiAvailable] — a stale share export must not degrade review or
     * sync). See [ShareBindStatus].
     */
    private fun markShareUnavailable(api: String, t: Throwable) {
        ShareBindStatus.available = false
        android.util.Log.w("RustCoreAdapter", "wave-9 share export $api unavailable; serving disabled-state fallback", t)
    }

    private companion object {
        /**
         * Set to false permanently the first time a wave-2+ export throws
         * (e.g. an older on-device libuniffi_bindings.so lacks the symbol and
         * raising UnsatisfiedLinkError at call time). Subsequent calls serve
         * the stub values, never attempting the native call again.
         */
        @Volatile
        var newApiAvailable: Boolean = true

        /** Fail-closed-but-renderable wave-5 fallback (see getAlphabetGateState). */
        val gateStubFallback = AlphabetGateState(
            satisfied = false,
            method = "",
            lettersIntroduced = 0u,
            lettersTotal = 43u,
            lettersPracticeTotal = 0u,
            lettersPracticeCorrect = 0u,
            lettersActiveDays = 0u,
            lettersChaptersMet = 0u
        )

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
/**
 * Process-wide latch for the wave-9 share surface ONLY: flips false forever
 * when any share export lookup/invoke fails on the loaded native library, so
 * the UI can render an explicit "unavailable" banner instead of a confusing
 * always-off state. Separate from the wave-2 latch — a stale share surface
 * must never degrade review or sync.
 */
object ShareBindStatus {
    @Volatile
    var available: Boolean = true
        internal set
}

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
