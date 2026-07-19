package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class Chapter(
    val number: Int,
    val title: String,
    val cardCount: Int,
    val completedCount: Int,
    // A chapter with no vocabulary yet is "unavailable" and should be surfaced
    // as coming-soon rather than a normal 0-card entry.
    val available: Boolean = true
) {
    val progress: Float get() = if (cardCount > 0) completedCount.toFloat() / cardCount else 0f
}

data class Section(
    val name: String,
    val textbook: String,
    val chapters: List<Chapter>,
    val isExpanded: Boolean = false
)

data class CurriculumState(
    val sections: List<Section> = emptyList(),
    val isLoading: Boolean = true
)

class CurriculumViewModel : ViewModel() {
    private val _state = MutableStateFlow(CurriculumState())
    val state: StateFlow<CurriculumState> = _state.asStateFlow()

    init { loadCurriculum() }

    private fun loadCurriculum() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            try {
                val curricula = core.getCurricula()
                val sections = curricula.mapIndexed { index, cv ->
                    val chapters = core.getChapters(cv.textbook)
                    Section(
                        name = cv.displayName,
                        textbook = cv.textbook,
                        chapters = chapters.map { ch ->
                            Chapter(
                                number = ch.chapter,
                                title = ch.title,
                                cardCount = ch.cardCount,
                                completedCount = ch.masteredCount,
                                available = ch.available
                            )
                        },
                        isExpanded = (index == 0)
                    )
                }
                _state.value = CurriculumState(sections = sections, isLoading = false)
            } catch (e: Exception) {
                _state.value = _state.value.copy(isLoading = false)
            }
        }
    }

    fun toggleSection(sectionName: String) {
        _state.value = _state.value.copy(
            sections = _state.value.sections.map { section ->
                if (section.name == sectionName) {
                    section.copy(isExpanded = !section.isExpanded)
                } else {
                    section
                }
            }
        )
    }

    fun startChapter(chapterNumber: Int) {
        val section = _state.value.sections.firstOrNull { s ->
            s.chapters.any { it.number == chapterNumber }
        } ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val core = GreekApplication.greekCore ?: return@launch
            try {
                core.unlockChapter(section.textbook, chapterNumber)
            } catch (_: Exception) {}
        }
    }
}
