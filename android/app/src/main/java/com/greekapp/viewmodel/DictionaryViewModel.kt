package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class DictionaryEntry(
    val headword: String,
    val pos: String,
    val definition: String,
    val lemma: String = headword
)

data class DictionaryState(
    val query: String = "",
    val results: List<DictionaryEntry> = emptyList(),
    val isSearching: Boolean = false,
    val error: String? = null
)

class DictionaryViewModel : ViewModel() {
    private val _state = MutableStateFlow(DictionaryState())
    val state: StateFlow<DictionaryState> = _state.asStateFlow()

    private var searchJob: Job? = null

    fun onQueryChanged(query: String) {
        _state.value = _state.value.copy(query = query)
        search(query)
    }

    private fun search(query: String) {
        searchJob?.cancel()
        if (query.isBlank()) {
            _state.value = _state.value.copy(results = emptyList(), isSearching = false, error = null)
            return
        }
        _state.value = _state.value.copy(isSearching = true, error = null)
        searchJob = viewModelScope.launch(Dispatchers.IO) {
            delay(150)
            val core = CoreProvider.current() ?: return@launch
            try {
                val results = core.searchDictionary(query, 50)
                _state.value = _state.value.copy(
                    results = results.map {
                        DictionaryEntry(
                            headword = it.headword,
                            pos = it.pos,
                            definition = it.definition
                        )
                    },
                    isSearching = false,
                    error = null
                )
            } catch (e: Exception) {
                _state.value = _state.value.copy(
                    results = emptyList(),
                    isSearching = false,
                    error = e.message ?: "Search failed"
                )
            }
        }
    }

    fun clearSearch() {
        searchJob?.cancel()
        _state.value = DictionaryState()
    }
}
