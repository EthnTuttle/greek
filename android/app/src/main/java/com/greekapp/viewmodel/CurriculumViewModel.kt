package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

data class Chapter(
    val number: Int,
    val title: String,
    val cardCount: Int,
    val completedCount: Int
) {
    val progress: Float get() = if (cardCount > 0) completedCount.toFloat() / cardCount else 0f
}

data class Section(
    val name: String,
    val chapters: List<Chapter>,
    val isExpanded: Boolean = false
)

data class CurriculumState(
    val sections: List<Section> = emptyList()
)

class CurriculumViewModel : ViewModel() {
    private val _state = MutableStateFlow(
        CurriculumState(
            sections = listOf(
                Section(
                    name = "Athenaze Book I",
                    chapters = listOf(
                        Chapter(1, "Ο ΔΙΚΑΙΟΠΟΛΙΣ (α)", 45, 45),
                        Chapter(2, "Ο ΔΙΚΑΙΟΠΟΛΙΣ (β)", 52, 38),
                        Chapter(3, "Ο ΑΡΟΤΟΣ (α)", 48, 12),
                        Chapter(4, "Ο ΑΡΟΤΟΣ (β)", 55, 0),
                        Chapter(5, "Ο ΛΥΚΟΣ (α)", 61, 0),
                        Chapter(6, "Ο ΛΥΚΟΣ (β)", 47, 0),
                        Chapter(7, "Ο ΚΥΝΗΓΟΣ (α)", 58, 0),
                        Chapter(8, "Ο ΚΥΝΗΓΟΣ (β)", 53, 0),
                        Chapter(9, "Ο ΠΑΡΘΕΝΟΣ (α)", 49, 0),
                        Chapter(10, "Ο ΠΑΡΘΕΝΟΣ (β)", 56, 0)
                    ),
                    isExpanded = true
                ),
                Section(
                    name = "Athenaze Book II",
                    chapters = (11..20).map { num ->
                        Chapter(num, "Chapter $num", 50, 0)
                    },
                    isExpanded = false
                )
            )
        )
    )
    val state: StateFlow<CurriculumState> = _state.asStateFlow()

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
        // In real app: navigate to chapter-specific review session
        // GreekCore.loadChapterCards(chapterNumber)
    }
}
