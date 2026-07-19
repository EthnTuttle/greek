package com.greekapp.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.ui.theme.GreekFontFamily
import com.greekapp.viewmodel.ReaderViewModel
import com.greekapp.viewmodel.SectionInfo
import com.greekapp.viewmodel.normalizeGreekWord
import uniffi.uniffi_bindings.TokenView

private val GreekTextSize = 20.sp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    viewModel: ReaderViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()
    val sheetState = rememberModalBottomSheetState()
    val snackbarHostState = remember { SnackbarHostState() }

    // Surface transient confirmations (card added, known toggled) via snackbar.
    LaunchedEffect(state.message) {
        val msg = state.message
        if (msg != null) {
            snackbarHostState.showSnackbar(msg)
            viewModel.messageShown()
        }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHostState) }
    ) { innerPadding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(innerPadding)
                .padding(16.dp)
        ) {
            Text(
                text = "Reader",
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold
            )
            Spacer(modifier = Modifier.height(16.dp))

            if (state.sections.size > 1) {
                SectionSelector(
                    sections = state.sections,
                    selectedId = state.selectedSectionId,
                    onSelect = { viewModel.selectSection(it) }
                )
                Spacer(modifier = Modifier.height(12.dp))
            }

            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surface
                )
            ) {
                // Restore scroll offset once, then persist changes for config-change /
                // process-death restore.
                val scrollState = rememberScrollState()
                LaunchedEffect(state.selectedSectionId, state.text) {
                    scrollState.scrollTo(state.initialScrollOffset)
                }
                LaunchedEffect(scrollState) {
                    snapshotFlow { scrollState.value }
                        .collect { viewModel.saveScrollOffset(it) }
                }
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(scrollState),
                    horizontalAlignment = Alignment.CenterHorizontally
                ) {
                    Box(
                        // Constrain line length so text stays readable on tablets.
                        modifier = Modifier
                            .widthIn(max = 640.dp)
                            .fillMaxWidth()
                            .padding(20.dp)
                    ) {
                        ClickableGreekText(
                            tokens = state.tokens,
                            fallbackText = state.text,
                            knownWords = state.knownWords,
                            onWordClick = { word -> viewModel.onWordTapped(word) }
                        )
                    }
                }
            }
        }
    }

    // Word analysis bottom sheet
    if (state.showAnalysis && state.selectedWord != null) {
        ModalBottomSheet(
            onDismissRequest = { viewModel.dismissAnalysis() },
            sheetState = sheetState
        ) {
            WordAnalysisSheet(
                word = state.selectedWord!!,
                onAddToSrs = { viewModel.addToSrs() },
                onSetKnown = { known -> viewModel.setWordKnown(known) },
                onDismiss = { viewModel.dismissAnalysis() }
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SectionSelector(
    sections: List<SectionInfo>,
    selectedId: String,
    onSelect: (String) -> Unit
) {
    var expanded by remember { mutableStateOf(false) }
    val selectedTitle = sections.firstOrNull { it.id == selectedId }?.title ?: "Select a text"

    ExposedDropdownMenuBox(
        expanded = expanded,
        onExpandedChange = { expanded = it }
    ) {
        OutlinedTextField(
            value = selectedTitle,
            onValueChange = {},
            readOnly = true,
            label = { Text("Passage") },
            trailingIcon = {
                Icon(Icons.Default.ArrowDropDown, contentDescription = null)
            },
            modifier = Modifier
                .menuAnchor()
                .fillMaxWidth()
        )
        ExposedDropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false }
        ) {
            sections.forEach { section ->
                DropdownMenuItem(
                    text = { Text(section.title) },
                    onClick = {
                        expanded = false
                        onSelect(section.id)
                    }
                )
            }
        }
    }
}

@Composable
private fun ClickableGreekText(
    tokens: List<TokenView>,
    fallbackText: String,
    knownWords: Set<String>,
    onWordClick: (String) -> Unit
) {
    // Prefer Rust token boundaries (they handle Greek punctuation); fall back to a
    // naive split only if tokens are unavailable.
    val effectiveTokens = remember(tokens, fallbackText) {
        if (tokens.isNotEmpty()) tokens
        else fallbackText.split(" ").mapIndexed { i, w ->
            TokenView(w, whitespaceAfter = true)
        }
    }

    val onSurface = MaterialTheme.colorScheme.onSurface
    val knownColor = MaterialTheme.colorScheme.primary

    val annotatedText = buildAnnotatedString {
        effectiveTokens.forEach { token ->
            val surface = token.surface
            val isWord = surface.any { it.isLetter() }
            if (isWord) {
                val known = normalizeGreekWord(surface) in knownWords
                pushStringAnnotation(tag = "WORD", annotation = surface)
                withStyle(
                    style = SpanStyle(
                        fontFamily = GreekFontFamily,
                        color = if (known) knownColor else onSurface,
                        fontWeight = if (known) FontWeight.Medium else FontWeight.Normal,
                        textDecoration = if (known) TextDecoration.Underline else null
                    )
                ) {
                    append(surface)
                }
                pop()
            } else {
                withStyle(
                    style = SpanStyle(fontFamily = GreekFontFamily, color = onSurface)
                ) {
                    append(surface)
                }
            }
            if (token.whitespaceAfter) append(" ")
        }
    }

    androidx.compose.foundation.text.ClickableText(
        text = annotatedText,
        style = TextStyle(
            fontFamily = GreekFontFamily,
            fontSize = GreekTextSize,
            lineHeight = 32.sp,
            color = onSurface
        ),
        onClick = { offset ->
            annotatedText.getStringAnnotations(tag = "WORD", start = offset, end = offset)
                .firstOrNull()?.let { annotation ->
                    onWordClick(annotation.item)
                }
        }
    )
}

@Composable
private fun WordAnalysisSheet(
    word: com.greekapp.viewmodel.WordAnalysis,
    onAddToSrs: () -> Unit,
    onSetKnown: (Boolean) -> Unit,
    onDismiss: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        // Word
        Text(
            text = word.word,
            style = MaterialTheme.typography.displaySmall,
            fontFamily = GreekFontFamily,
            fontWeight = FontWeight.Bold
        )

        Divider()

        // Lemma
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                text = "Lemma:",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                text = word.lemma,
                style = MaterialTheme.typography.titleMedium,
                fontFamily = GreekFontFamily,
                fontWeight = FontWeight.Medium
            )
        }

        // POS
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                text = "Part of speech:",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                text = word.pos,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Medium
            )
        }

        // Parse info
        if (word.parseInfo != null) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    text = "Parse:",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Text(
                    text = word.parseInfo,
                    style = MaterialTheme.typography.bodyMedium
                )
            }
        }

        // Additional (ambiguous) parses
        if (word.otherParses.isNotEmpty()) {
            Text(
                text = "Other possible parses:",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            word.otherParses.forEach { parse ->
                Text(
                    text = "• $parse",
                    style = MaterialTheme.typography.bodyMedium
                )
            }
        }

        Divider()

        // Definition
        Text(
            text = "Definition",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text = word.definition,
            style = MaterialTheme.typography.bodyLarge,
            lineHeight = 24.sp
        )

        Spacer(modifier = Modifier.height(8.dp))

        // Known / unknown toggle
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            FilterChip(
                selected = word.isKnown,
                onClick = { onSetKnown(!word.isKnown) },
                label = { Text(if (word.isKnown) "Known" else "Mark as known") },
                leadingIcon = if (word.isKnown) {
                    { Icon(Icons.Default.Check, contentDescription = null, modifier = Modifier.size(18.dp)) }
                } else null
            )
        }

        // Actions
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedButton(
                onClick = onDismiss,
                modifier = Modifier.weight(1f)
            ) {
                Text("Close")
            }
            Button(
                onClick = onAddToSrs,
                enabled = word.canAddToSrs,
                modifier = Modifier.weight(1f)
            ) {
                Icon(
                    imageVector = Icons.Default.Add,
                    contentDescription = null,
                    modifier = Modifier.size(18.dp)
                )
                Spacer(modifier = Modifier.width(4.dp))
                Text("Add to SRS")
            }
        }

        Spacer(modifier = Modifier.height(16.dp))
    }
}
