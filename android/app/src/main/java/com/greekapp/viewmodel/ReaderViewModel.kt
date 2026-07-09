package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class WordAnalysis(
    val word: String,
    val lemma: String,
    val pos: String,
    val definition: String,
    val parseInfo: String? = null
)

data class ReaderState(
    val text: String = "",
    val title: String = "",
    val selectedWord: WordAnalysis? = null,
    val showAnalysis: Boolean = false,
    val isLoading: Boolean = true
)

class ReaderViewModel : ViewModel() {
    private val _state = MutableStateFlow(ReaderState())
    val state: StateFlow<ReaderState> = _state.asStateFlow()

    init { loadText() }

    private fun loadText() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            try {
                val sections = core.getTextSections()
                if (sections.isNotEmpty()) {
                    val first = sections.first()
                    _state.value = ReaderState(
                        text = first.text,
                        title = first.title,
                        isLoading = false
                    )
                } else {
                    _state.value = ReaderState(
                        text = "No texts available yet.",
                        title = "Reader",
                        isLoading = false
                    )
                }
            } catch (e: Exception) {
                _state.value = ReaderState(
                    text = "Failed to load text.",
                    title = "Error",
                    isLoading = false
                )
            }
        }
    }

    fun onWordTapped(word: String) {
        val normalized = word.lowercase().trim { it in " .,;·?!·\"'()[]" }
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            try {
                val results = core.lookupWord(normalized)
                val analysis = if (results.isNotEmpty()) {
                    val first = results.first()
                    WordAnalysis(
                        word = word,
                        lemma = first.lemma,
                        pos = first.postagHuman.split(",").firstOrNull()?.trim() ?: "unknown",
                        definition = first.definition,
                        parseInfo = first.postagHuman
                    )
                } else {
                    WordAnalysis(
                        word = word,
                        lemma = "Unknown",
                        pos = "unknown",
                        definition = "Not found in dictionary"
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
                        definition = "Lookup failed"
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

    fun addToSrs() {
        dismissAnalysis()
    }
}
