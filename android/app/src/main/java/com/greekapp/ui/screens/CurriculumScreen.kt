package com.greekapp.ui.screens

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.R
import com.greekapp.viewmodel.Chapter
import com.greekapp.viewmodel.CurriculumViewModel
import com.greekapp.viewmodel.Section

@Composable
fun CurriculumScreen(
    /** Wave-5: navigate to the Alphabet Check destination. */
    onTakeAlphabetCheck: () -> Unit = {},
    /** Wave-9: navigate to the typed_letter drill destination. */
    onOpenLetterTyping: () -> Unit = {},
    viewModel: CurriculumViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()
    val snackbarHostState = remember { SnackbarHostState() }

    // Re-pull on every resume: a passed Alphabet Check (or a Settings skip)
    // must hide the gate card immediately even when this tab was restored
    // from the bottom-nav back stack.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        viewModel.refresh()
    }

    // Surface unlock-hint messages (tapped a locked chapter with a known
    // requirement) as a transient snackbar.
    LaunchedEffect(state.unlockHint) {
        val hint = state.unlockHint
        if (hint != null) {
            snackbarHostState.showSnackbar(hint)
            viewModel.hintShown()
        }
    }

    Box(
        modifier = Modifier.fillMaxSize()
    ) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(16.dp)
        ) {
            Text(
                text = "Curriculum",
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold
            )
            Spacer(modifier = Modifier.height(16.dp))

            LazyColumn(
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                // Wave-5: compact alphabet-gate row above the textbook list
                // while the gate is unmet (same card as Home, reduced form).
                if (state.alphabetGateUnmet) {
                    item(key = "alphabet-gate") {
                        AlphabetGateRow(onTakeAlphabetCheck = onTakeAlphabetCheck)
                    }
                }
                // Wave-9: letters-root row for the typed_letter drill. The
                // letters track has no dedicated chapter screen (letters flow
                // through Practice + this curriculum tab, which is the letters
                // root), so the drill is a standalone pushed route entered here.
                if (state.lettersAvailable) {
                    item(key = "letter-typing-drill") {
                        OutlinedButton(
                            onClick = onOpenLetterTyping,
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Text("Type the letters (Ελληνικά)")
                        }
                    }
                }
                state.sections.forEach { section ->
                    item(key = section.name) {
                        SectionCard(
                            section = section,
                            onToggle = { viewModel.toggleSection(section.name) },
                            onChapterClick = { chapter -> viewModel.onChapterTapped(chapter.number) }
                        )
                    }
                }
            }
        }

        SnackbarHost(
            hostState = snackbarHostState,
            modifier = Modifier.align(Alignment.BottomCenter)
        )
    }
}

/** Compact wave-5 gate card (Curriculum tab): title + a single check CTA. */
@Composable
private fun AlphabetGateRow(
    onTakeAlphabetCheck: () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = stringResource(R.string.home_gate_title),
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.onTertiaryContainer,
                modifier = Modifier.weight(1f)
            )
            TextButton(onClick = onTakeAlphabetCheck) {
                Text(
                    text = stringResource(R.string.gate_take_check_short),
                    color = MaterialTheme.colorScheme.onTertiaryContainer,
                    maxLines = 2
                )
            }
        }
    }
}

@Composable
private fun SectionCard(
    section: Section,
    onToggle: () -> Unit,
    onChapterClick: (Chapter) -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        )
    ) {
        Column(
            modifier = Modifier.fillMaxWidth()
        ) {
            // Section header
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onToggle)
                    .padding(16.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = section.name,
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.Bold
                    )
                    Text(
                        text = "${section.chapters.size} chapters",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Icon(
                    imageVector = if (section.isExpanded) Icons.Default.ExpandLess else Icons.Default.ExpandMore,
                    contentDescription = if (section.isExpanded) "Collapse" else "Expand"
                )
            }

            // Chapters list
            AnimatedVisibility(visible = section.isExpanded) {
                Column(
                    modifier = Modifier.padding(bottom = 8.dp)
                ) {
                    section.chapters.forEach { chapter ->
                        ChapterItem(
                            chapter = chapter,
                            onClick = { onChapterClick(chapter) }
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ChapterItem(
    chapter: Chapter,
    onClick: () -> Unit
) {
    // Chapters with no vocabulary yet are shown as "Coming Soon": dimmed,
    // non-clickable, and without a progress bar (0/0 progress is meaningless).
    val cardModifier = Modifier
        .fillMaxWidth()
        .padding(horizontal = 16.dp, vertical = 4.dp)
        .let { if (chapter.available) it.clickable(onClick = onClick) else it }

    Card(
        modifier = cardModifier,
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surface
        )
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            // Chapter title
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    text = "Chapter ${chapter.number}",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    color = if (chapter.available) {
                        MaterialTheme.colorScheme.onSurface
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    }
                )
                if (chapter.available) {
                    Text(
                        text = "${chapter.cardCount} cards",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                } else {
                    // Coming Soon badge
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        shape = MaterialTheme.shapes.small
                    ) {
                        Text(
                            text = "Coming Soon",
                            style = MaterialTheme.typography.labelSmall,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
                        )
                    }
                }
            }

            Text(
                text = chapter.title,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            // Progress bar — only meaningful when the chapter has cards.
            if (chapter.available) {
                Column(
                    verticalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    LinearProgressIndicator(
                        progress = { chapter.progress },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        text = "${(chapter.progress * 100).toInt()}% complete (${chapter.completedCount}/${chapter.cardCount})",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }
    }
}
