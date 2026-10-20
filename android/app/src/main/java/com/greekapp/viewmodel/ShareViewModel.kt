package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import com.greekapp.core.ShareBindStatus
import com.greekapp.core.ShareFlushReport
import com.greekapp.core.ShareHistoryEntry
import com.greekapp.core.ShareMilestone
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** One-shot snackbar payloads; the screens map them to string resources. */
sealed interface ShareUiMessage {
    data object Enabled : ShareUiMessage
    data object Disabled : ShareUiMessage
    data object RelaysSaved : ShareUiMessage
    data object TemplateSaved : ShareUiMessage
    data class Flushed(val report: ShareFlushReport) : ShareUiMessage
    data class Error(val reason: String) : ShareUiMessage
}

/** Editor-sheet draft state for one milestone (active-locale scoped text). */
data class ShareEditorState(
    val slug: String,
    /** Active template locale slot ("en" | "el"). */
    val locale: String,
    /** Draft of the per-locale custom text ("" = shipped preset). */
    val draftText: String,
    /** Engine-rendered preview of draftText appended to the template line. */
    val preview: String = "",
    /** Engine's verbatim rejection reason for the current draft (null = OK). */
    val previewError: String? = null,
    val history: List<ShareHistoryEntry> = emptyList()
)

data class ShareState(
    /** False when the loaded native bindings predate wave-9 share exports. */
    val featureAvailable: Boolean = true,
    val enabled: Boolean = false,
    /** Derived sharing pubkey hex ("" while no identity). UI truncates. */
    val pubkeyHex: String = "",
    val relays: List<String> = emptyList(),
    /** Rendered exactly as shipped by the engine — never a hardcoded set. */
    val milestones: List<ShareMilestone> = emptyList(),
    val pendingCount: Int = 0,
    /** Last flush result from THIS session; feeds the status card. */
    val lastFlush: ShareFlushReport? = null,
    val flushing: Boolean = false,
    val busy: Boolean = false,
    /** Non-null while the milestone editor sheet is open for this slug. */
    val editor: ShareEditorState? = null,
    val message: ShareUiMessage? = null
)

@OptIn(FlowPreview::class)
class ShareViewModel : ViewModel() {
    private val _state = MutableStateFlow(ShareState())
    val state: StateFlow<ShareState> = _state.asStateFlow()

    /** Debounced (slug, suffix) preview requests from the editor field. */
    private val previewRequests = MutableStateFlow<Pair<String, String>?>(null)

    init {
        refresh()
        viewModelScope.launch {
            previewRequests.debounce(300L).collect { request ->
                request?.let { (slug, suffix) -> runPreview(slug, suffix) }
            }
        }
    }

    /** Reload config, milestones, and the badge count from the core. */
    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            runCatching {
                val config = core.shareConfigGet()
                val milestones = core.shareMilestones()
                    // The declared-known gate lane never posts (self-declaration
                    // is not evidence): filter defensively even though the
                    // engine omits it (Appendix R).
                    .filter { it.slug != "gate.passed.declared-known" }
                _state.value = _state.value.copy(
                    featureAvailable = ShareBindStatus.available,
                    enabled = config.enabled,
                    pubkeyHex = config.pubkeyHex,
                    relays = config.relays,
                    milestones = milestones,
                    pendingCount = core.sharePendingCount().toInt()
                )
            }.onFailure { Log.w(TAG, "refresh failed", it) }
        }
    }

    /** Consent confirmed: enable with exactly the relays the user picked. */
    fun enable(relays: List<String>) {
        busyCall(
            block = { CoreProvider.current()?.shareEnable(relays) },
            onSuccess = { message(ShareUiMessage.Enabled); refresh() },
            onFailure = { failMessage(it) }
        )
    }

    /** [forgetIdentity] = "stop and forget sharing identity" disable lane. */
    fun disable(forgetIdentity: Boolean) {
        busyCall(
            block = { CoreProvider.current()?.shareDisable(forgetIdentity) },
            onSuccess = { message(ShareUiMessage.Disabled); refresh() },
            onFailure = { failMessage(it) }
        )
    }

    fun setRelays(relays: List<String>) {
        busyCall(
            block = { CoreProvider.current()?.shareRelaysSet(relays) },
            onSuccess = { message(ShareUiMessage.RelaysSaved); refresh() },
            onFailure = { failMessage(it) }
        )
    }

    fun setMilestoneEnabled(slug: String, enabled: Boolean) {
        busyCall(
            block = { CoreProvider.current()?.shareSetMilestoneEnabled(slug, enabled) },
            onSuccess = { refresh() },
            onFailure = { failMessage(it) }
        )
    }

    fun saveTemplate(slug: String, locale: String, text: String) {
        busyCall(
            block = { CoreProvider.current()?.shareSetTemplate(slug, locale, text) },
            onSuccess = {
                message(ShareUiMessage.TemplateSaved)
                refresh()
                refreshEditor(slug, locale, text)
            },
            onFailure = { failMessage(it) }
        )
    }

    fun resetTemplate(slug: String, locale: String) {
        busyCall(
            block = { CoreProvider.current()?.shareResetTemplate(slug, locale) },
            onSuccess = {
                message(ShareUiMessage.TemplateSaved)
                refresh()
                refreshEditor(slug, locale, "")
            },
            onFailure = { failMessage(it) }
        )
    }

    /** Engine-rendered example post for the consent dialog (gate.passed.check). */
    suspend fun examplePreview(slug: String = "gate.passed.check", locale: String = "en"): String? =
        withContext(Dispatchers.IO) {
            runCatching { CoreProvider.current()?.sharePreview(slug, "") }.getOrNull()
        }

    fun flushNow() {
        if (_state.value.flushing) return
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(flushing = true)
            try {
                val core = CoreProvider.current()
                    ?: error("core not ready")
                val report = core.shareFlushNow()
                _state.value = _state.value.copy(
                    lastFlush = report,
                    message = ShareUiMessage.Flushed(report)
                )
                refresh()
            } catch (t: Throwable) {
                Log.w(TAG, "shareFlushNow failed", t)
                failMessage(t)
            } finally {
                _state.value = _state.value.copy(flushing = false)
            }
        }
    }

    // ---- Milestone editor sheet ----

    fun openEditor(slug: String, initialLocale: String) {
        val milestone = _state.value.milestones.firstOrNull { it.slug == slug } ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val history = runCatching { CoreProvider.current()?.shareHistory(slug) }
                .getOrNull() ?: emptyList()
            val locale = initialLocale
            _state.value = _state.value.copy(
                editor = ShareEditorState(
                    slug = slug,
                    locale = locale,
                    draftText = if (locale == "el") {
                        milestone.customTemplateEl ?: ""
                    } else {
                        milestone.customTemplateEn ?: ""
                    },
                    history = history
                )
            )
            _state.value.editor?.let { runPreview(it.slug, it.draftText) }
        }
    }

    fun closeEditor() {
        _state.value = _state.value.copy(editor = null)
        previewRequests.value = null
    }

    fun setEditorLocale(locale: String) {
        val editor = _state.value.editor ?: return
        val milestone = _state.value.milestones.firstOrNull { it.slug == editor.slug } ?: return
        val draft = if (locale == "el") {
            milestone.customTemplateEl ?: ""
        } else {
            milestone.customTemplateEn ?: ""
        }
        _state.value = _state.value.copy(
            editor = editor.copy(locale = locale, draftText = draft, previewError = null)
        )
        previewRequests.value = editor.slug to draft
    }

    fun onEditorDraftChange(text: String) {
        val editor = _state.value.editor ?: return
        _state.value = _state.value.copy(editor = editor.copy(draftText = text))
        previewRequests.value = editor.slug to text
    }

    /** Immediate (non-debounced) render of slug+draft into the editor state. */
    private suspend fun runPreview(slug: String, suffix: String) {
        val editor = _state.value.editor ?: return
        if (editor.slug != slug) return
        val result = withContext(Dispatchers.IO) {
            runCatching { CoreProvider.current()?.sharePreview(slug, suffix) ?: "" }
        }
        val current = _state.value.editor ?: return
        if (current.slug != slug || current.draftText != suffix) return
        _state.value = _state.value.copy(
            editor = current.copy(
                preview = result.getOrDefault(""),
                previewError = result.exceptionOrNull()?.message
            )
        )
    }

    private fun refreshEditor(slug: String, locale: String, draft: String) {
        val editor = _state.value.editor ?: return
        if (editor.slug != slug) return
        viewModelScope.launch(Dispatchers.IO) {
            val history = runCatching { CoreProvider.current()?.shareHistory(slug) }
                .getOrNull() ?: emptyList()
            val current = _state.value.editor ?: return@launch
            if (current.slug != slug) return@launch
            _state.value = _state.value.copy(
                editor = current.copy(
                    locale = locale,
                    draftText = draft,
                    history = history,
                    previewError = null
                )
            )
            runPreview(slug, draft)
        }
    }

    /** Called by the UI once the snackbar has been displayed. */
    fun messageShown() {
        _state.value = _state.value.copy(message = null)
    }

    // -- helpers ---------------------------------------------------------------

    private inline fun <T> busyCall(
        crossinline block: suspend () -> T,
        crossinline onSuccess: (T) -> Unit,
        crossinline onFailure: (Throwable) -> Unit
    ) {
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(busy = true)
            try {
                onSuccess(block())
            } catch (t: Throwable) {
                Log.w(TAG, "share action failed", t)
                onFailure(t)
            } finally {
                _state.value = _state.value.copy(busy = false)
            }
        }
    }

    private fun message(message: ShareUiMessage) {
        _state.value = _state.value.copy(message = message)
    }

    private fun failMessage(reason: Throwable) {
        message(ShareUiMessage.Error(reason.message ?: reason.toString()))
    }

    private companion object {
        const val TAG = "ShareViewModel"
    }
}
