package com.greekapp.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.ui.theme.GreekFontFamily
import com.greekapp.viewmodel.ReaderViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ReaderScreen(
    viewModel: ReaderViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()
    val sheetState = rememberModalBottomSheetState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Reader",
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold
        )
        Spacer(modifier = Modifier.height(16.dp))

        Card(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surface
            )
        ) {
            val scrollState = rememberScrollState()
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(scrollState)
                    .padding(20.dp)
            ) {
                ClickableGreekText(
                    text = state.text,
                    onWordClick = { word -> viewModel.onWordTapped(word) }
                )
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
                onDismiss = { viewModel.dismissAnalysis() }
            )
        }
    }
}

@Composable
private fun ClickableGreekText(
    text: String,
    onWordClick: (String) -> Unit
) {
    val words = text.split(" ")
    val annotatedText = buildAnnotatedString {
        words.forEachIndexed { index, word ->
            pushStringAnnotation(tag = "WORD", annotation = word)
            withStyle(
                style = SpanStyle(
                    fontFamily = GreekFontFamily,
                    fontSize = 20.sp,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 1f)
                )
            ) {
                append(word)
            }
            pop()
            if (index < words.lastIndex) {
                append(" ")
            }
        }
    }

    androidx.compose.foundation.text.ClickableText(
        text = annotatedText,
        style = MaterialTheme.typography.bodyLarge.copy(
            fontFamily = GreekFontFamily,
            fontSize = 20.sp,
            lineHeight = 32.sp
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
