package com.greekapp.viewmodel

import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.uniffi_bindings.TokenView

data class WordAnalysis(
    val word: String,
    val lemma: String,
    val pos: String,
    val definition: String,
    val parseInfo: String? = null,
    val otherParses: List<String> = emptyList(),
    val isKnown: Boolean = false,
    val canAddToSrs: Boolean = true
)

/** Lightweight descriptor for the section selector UI. */
data class SectionInfo(
    val id: String,
    val title: String
)

data class ReaderState(
    val sections: List<SectionInfo> = emptyList(),
    val selectedSectionId: String = "",
    val text: String = "",
    val title: String = "",
    val tokens: List<TokenView> = emptyList(),
    val knownWords: Set<String> = emptySet(),
    val selectedWord: WordAnalysis? = null,
    val showAnalysis: Boolean = false,
    val initialScrollOffset: Int = 0,
    val isLoading: Boolean = true,
    /** Transient message to surface in a snackbar; cleared via [messageShown]. */
    val message: String? = null
)

/**
 * Strips surrounding Greek/ASCII punctuation and lowercases so that lookup,
 * known-state marking, and the highlight comparison all use the same key.
 */
internal fun normalizeGreekWord(word: String): String =
    word.lowercase().trim { it in " .,;·?!·\"'()[]»«…—–-·;·" }

class ReaderViewModel(
    private val savedStateHandle: SavedStateHandle
) : ViewModel() {
    private val _state = MutableStateFlow(ReaderState())
    val state: StateFlow<ReaderState> = _state.asStateFlow()

    // Full section data (incl. tokens) kept in memory; UI only needs id/title/tokens.
    private var sections: List<uniffi.uniffi_bindings.TextSectionView> = emptyList()

    init { loadText() }

    private fun loadText() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: run {
                _state.value = _state.value.copy(
                    text = "Reader is unavailable.",
                    title = "Reader",
                    isLoading = false
                )
                return@launch
            }
            try {
                val loaded = core.getTextSections()
                sections = loaded
                if (loaded.isEmpty()) {
                    _state.value = ReaderState(
                        text = "No texts available yet.",
                        title = "Reader",
                        isLoading = false
                    )
                    return@launch
                }
                val savedId = savedStateHandle.get<String>(KEY_SECTION_ID)
                val selected = loaded.firstOrNull { it.id == savedId } ?: loaded.first()
                val known = runCatching {
                    core.getKnownWords().map { normalizeGreekWord(it) }.toSet()
                }.getOrDefault(emptySet())
                _state.value = ReaderState(
                    sections = loaded.map { SectionInfo(it.id, it.title) },
                    selectedSectionId = selected.id,
                    text = selected.text,
                    title = selected.title,
                    tokens = selected.tokens,
                    knownWords = known,
                    initialScrollOffset = savedStateHandle.get<Int>(KEY_SCROLL) ?: 0,
                    isLoading = false
                )
            } catch (e: Exception) {
                _state.value = ReaderState(
                    text = "Failed to load text.",
                    title = "Error",
                    isLoading = false
                )
            }
        }
    }

    fun selectSection(id: String) {
        val section = sections.firstOrNull { it.id == id } ?: return
        savedStateHandle[KEY_SECTION_ID] = id
        savedStateHandle[KEY_SCROLL] = 0
        _state.value = _state.value.copy(
            selectedSectionId = id,
            text = section.text,
            title = section.title,
            tokens = section.tokens,
            initialScrollOffset = 0
        )
    }

    /** Persist the current scroll position for config-change / process-death restore. */
    fun saveScrollOffset(offset: Int) {
        savedStateHandle[KEY_SCROLL] = offset
    }

    fun onWordTapped(word: String) {
        val normalized = normalizeGreekWord(word)
        if (normalized.isEmpty()) return
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            val known = runCatching { core.isWordKnown(normalized) }.getOrDefault(false)
            try {
                val results = core.lookupWord(normalized)
                val analysis = if (results.isNotEmpty()) {
                    val first = results.first()
                    WordAnalysis(
                        word = word,
                        lemma = first.lemma,
                        pos = first.postagHuman.split(",").firstOrNull()?.trim() ?: "unknown",
                        definition = first.definition,
                        parseInfo = first.postagHuman,
                        otherParses = results.drop(1).map { it.postagHuman }.distinct(),
                        isKnown = known,
                        canAddToSrs = true
                    )
                } else {
                    WordAnalysis(
                        word = word,
                        lemma = "Unknown",
                        pos = "unknown",
                        definition = "Not found in dictionary",
                        isKnown = known,
                        canAddToSrs = false
                    )
                }
                _state.value = _state.value.copy(
                    selectedWord = analysis,
                    showAnalysis = true
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    selectedWord = WordAnalysis(
                        word = word,
                        lemma = "Error",
                        pos = "unknown",
                        definition = "Lookup failed",
                        isKnown = known,
                        canAddToSrs = false
                    ),
                    showAnalysis = true
                )
            }
        }
    }

    fun dismissAnalysis() {
        _state.value = _state.value.copy(
            showAnalysis = false,
            selectedWord = null
        )
    }

    /** Create an SRS card for the selected word via the Rust lookup helper. */
    fun addToSrs() {
        val analysis = _state.value.selectedWord
        if (analysis == null || !analysis.canAddToSrs) {
            _state.value = _state.value.copy(
                message = "Can't add an unrecognized word.",
                showAnalysis = false,
                selectedWord = null
            )
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                core.addCardFromLookup(analysis.lemma, analysis.word, analysis.definition)
                _state.value = _state.value.copy(
                    message = "Added “${analysis.lemma}” to your review deck.",
                    showAnalysis = false,
                    selectedWord = null
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    message = "Couldn't add card.",
                    showAnalysis = false,
                    selectedWord = null
                )
            }
        }
    }

    /** Mark the selected word known/unknown and reflect it in the highlight set. */
    fun setWordKnown(known: Boolean) {
        val analysis = _state.value.selectedWord ?: return
        val key = normalizeGreekWord(analysis.word)
        if (key.isEmpty()) return
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                core.markWordKnown(key, known)
                val updated = if (known) _state.value.knownWords + key
                    else _state.value.knownWords - key
                _state.value = _state.value.copy(
                    knownWords = updated,
                    selectedWord = analysis.copy(isKnown = known)
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(message = "Couldn't update known status.")
            }
        }
    }

    fun messageShown() {
        _state.value = _state.value.copy(message = null)
    }

    private companion object {
        const val KEY_SECTION_ID = "reader_section_id"
        const val KEY_SCROLL = "reader_scroll_offset"
    }
}
