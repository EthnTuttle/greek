package com.greekapp.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.uniffi_bindings.ChapterView

data class Chapter(
    val number: Int,
    val title: String,
    val cardCount: Int,
    val completedCount: Int,
    // A chapter with no vocabulary yet is "unavailable" and should be surfaced
    // as coming-soon rather than a normal 0-card entry.
    val available: Boolean = true,
    // Chapters start locked; tapping a locked one may reveal an unlock hint.
    val unlocked: Boolean = true
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
    val isLoading: Boolean = true,
    /** Transient unlock hint to surface in a snackbar; cleared via [hintShown]. */
    val unlockHint: String? = null,
    /**
     * Wave-5 alphabet gate unmet → the screen shows a compact gate card above
     * the textbook list with a check CTA. False while unknown (load failure).
     */
    val alphabetGateUnmet: Boolean = false
)

class CurriculumViewModel : ViewModel() {
    private val _state = MutableStateFlow(CurriculumState())
    val state: StateFlow<CurriculumState> = _state.asStateFlow()

    // Raw chapter views keyed by (textbook, chapter) so unlockHintFor can hand
    // the exact ChapterView back to the core adapter.
    private var chapterViews: Map<Pair<String, Int>, ChapterView> = emptyMap()

    init { refresh() }

    /**
     * Public so the screen can re-pull on ON_RESUME: the gate card must
     * disappear the moment a check passed elsewhere (or a skip lane ran)
     * even when this ViewModel instance was restored from the tab's
     * saved-state back stack.
     */
    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                // Alphabet gate (wave 5): one shared evaluation, kept local to
                // this ViewModel; any failure fails open (card hidden).
                val gateUnmet = try {
                    val gate = core.getAlphabetGateState()
                    !gate.satisfied && gate.lettersTotal > 0u
                } catch (e: Exception) {
                    false
                }
                val curricula = core.getCurricula()
                val viewMap = mutableMapOf<Pair<String, Int>, ChapterView>()
                val sections = curricula.mapIndexed { index, cv ->
                    val chapters = core.getChapters(cv.textbook)
                    for (ch in chapters) {
                        viewMap[cv.textbook to ch.chapter] = ch
                    }
                    Section(
                        name = cv.displayName,
                        textbook = cv.textbook,
                        chapters = chapters.map { ch ->
                            Chapter(
                                number = ch.chapter,
                                title = ch.title,
                                cardCount = ch.cardCount,
                                completedCount = ch.masteredCount,
                                available = ch.available,
                                unlocked = ch.unlocked
                            )
                        },
                        isExpanded = (index == 0)
                    )
                }
                chapterViews = viewMap
                _state.value = CurriculumState(
                    sections = sections,
                    isLoading = false,
                    alphabetGateUnmet = gateUnmet
                )
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

    /**
     * Tap handler for a chapter row. Locked chapters consult the core for a
     * human-readable unlock requirement first: when a hint exists it is shown
     * (no unlock attempt); otherwise the legacy unlock path runs unchanged.
     */
    fun onChapterTapped(chapterNumber: Int) {
        val section = _state.value.sections.firstOrNull { s ->
            s.chapters.any { it.number == chapterNumber }
        } ?: return
        val chapter = section.chapters.firstOrNull { it.number == chapterNumber } ?: return
        if (chapter.unlocked) {
            startChapter(chapterNumber)
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            val chapterView = chapterViews[section.textbook to chapterNumber]
            val hint = chapterView?.let { cv ->
                try {
                    core.unlockHintFor(cv)
                } catch (e: Exception) {
                    ""
                }
            } ?: ""
            if (hint.isNotEmpty()) {
                // Requirement known: tell the user, do NOT unlock.
                _state.value = _state.value.copy(unlockHint = hint)
            } else {
                // No hint (pre-wave-2): unlock exactly as startChapter does today.
                startChapter(chapterNumber)
            }
        }
    }

    /** Called by the UI once the unlock-hint snackbar has been displayed. */
    fun hintShown() {
        _state.value = _state.value.copy(unlockHint = null)
    }

    fun startChapter(chapterNumber: Int) {
        val section = _state.value.sections.firstOrNull { s ->
            s.chapters.any { it.number == chapterNumber }
        } ?: return
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            try {
                core.unlockChapter(section.textbook, chapterNumber)
            } catch (_: Exception) {}
        }
    }
}
