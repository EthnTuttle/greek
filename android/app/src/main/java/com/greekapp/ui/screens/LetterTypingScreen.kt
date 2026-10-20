package com.greekapp.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.ui.components.GreekKeyboardContainer
import com.greekapp.ui.components.GreekTextField
import com.greekapp.ui.theme.GreekFontFamily
import com.greekapp.viewmodel.LetterDrillViewModel

/**
 * Wave-9 typed_letter drill ("Type the letters"). Prompt by letter NAME /
 * Erasmian sound description, never the glyph; single-grapheme field with
 * auto-submit; grading strict on breathing/subscript/diaeresis/final-sigma
 * and lenient on pitch accents. Reached from the Curriculum (letters) tab.
 *
 * Root screen wraps in [GreekKeyboardContainer] per the binding embedding
 * spec, so the polytonic panel docks per the device-local input mode.
 */
@Composable
fun LetterTypingScreen(
    onBack: () -> Unit = {},
    viewModel: LetterDrillViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()

    GreekKeyboardContainer { contentModifier ->
        Column(
            modifier = contentModifier.padding(16.dp),
            verticalArrangement = Arrangement.SpaceBetween
        ) {
            Column(modifier = Modifier.fillMaxWidth()) {
                Text(
                    text = "Type the letters",
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold
                )
                Spacer(modifier = Modifier.height(8.dp))
                if (!state.isLoading && !state.unavailable && !state.isComplete) {
                    LinearProgressIndicator(
                        progress = {
                            (state.index.toFloat() + 1) / state.questions.size
                        },
                        modifier = Modifier.fillMaxWidth()
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = "Question ${state.index + 1} of ${state.questions.size}",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
                    .padding(vertical = 24.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                when {
                    state.isLoading -> Text(
                        "Loading letters…",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    state.unavailable -> Text(
                        "The letters deck isn't installed on this device.",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center
                    )
                    state.isComplete -> {
                        Text(
                            text = "${state.score} / ${state.questions.size}",
                            style = MaterialTheme.typography.displayMedium,
                            fontWeight = FontWeight.Bold
                        )
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            "Typed-letter accuracy (pitch accents lenient)",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    else -> {
                        val question = state.questions[state.index]
                        Text(
                            text = "Type: ${question.prompt}",
                            style = MaterialTheme.typography.headlineSmall,
                            textAlign = TextAlign.Center
                        )
                        Spacer(modifier = Modifier.height(24.dp))
                        GreekTextField(
                            value = state.input,
                            onValueChange = viewModel::onInputChange,
                            enabled = state.correct == null,
                            singleLine = true,
                            label = { Text("One letter") },
                            modifier = Modifier.fillMaxWidth()
                        )
                        Spacer(modifier = Modifier.height(16.dp))
                        state.correct?.let { correct ->
                            Text(
                                text = if (correct) "Correct" else "Not quite",
                                style = MaterialTheme.typography.titleMedium,
                                color = if (correct) MaterialTheme.colorScheme.secondary
                                else MaterialTheme.colorScheme.error
                            )
                            Text(
                                text = question.target,
                                style = MaterialTheme.typography.headlineLarge,
                                fontFamily = GreekFontFamily
                            )
                        }
                    }
                }
            }

            Column(modifier = Modifier.fillMaxWidth()) {
                if (state.isComplete) {
                    Button(
                        onClick = viewModel::restart,
                        modifier = Modifier.fillMaxWidth().height(56.dp)
                    ) {
                        Text("Practice again", style = MaterialTheme.typography.titleMedium)
                    }
                    Spacer(modifier = Modifier.height(8.dp))
                } else if (state.correct != null) {
                    Button(
                        onClick = viewModel::continueToNext,
                        modifier = Modifier.fillMaxWidth().height(56.dp)
                    ) {
                        Text("Continue", style = MaterialTheme.typography.titleMedium)
                    }
                    Spacer(modifier = Modifier.height(8.dp))
                }
                Button(
                    onClick = onBack,
                    modifier = Modifier.fillMaxWidth().height(56.dp)
                ) {
                    Text("Back", style = MaterialTheme.typography.titleMedium)
                }
            }
        }
    }
}
