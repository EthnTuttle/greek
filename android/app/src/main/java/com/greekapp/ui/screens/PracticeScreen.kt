package com.greekapp.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.R
import com.greekapp.core.McqData
import com.greekapp.ui.theme.GreekFontFamily
import com.greekapp.viewmodel.PracticeState
import com.greekapp.viewmodel.PracticeViewModel
import com.greekapp.viewmodel.isMcqQuestion

@Composable
fun PracticeScreen(
    onBack: () -> Unit,
    /**
     * Alphabet Check (wave-5) fail verdict only: navigate to the letters
     * curriculum tab. Unused during normal practice sessions.
     */
    onStudyLetters: () -> Unit = {},
    viewModel: PracticeViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()
    val question = viewModel.currentQuestion

    if (state.isLoading) {
        PracticeMessageScreen(
            title = null,
            message = stringResource(
                if (state.isAlphabetCheck) R.string.alphabet_check_loading
                else R.string.practice_loading
            ),
            buttonText = null,
            onButton = {}
        )
        return
    }

    if (state.isComplete) {
        if (state.isAlphabetCheck) {
            // Verdict replaces the normal summary. Null verdict = recording is
            // still in flight (fast path) → show the loading surface briefly.
            val verdict = state.checkVerdict
            if (verdict == null) {
                PracticeMessageScreen(
                    title = null,
                    message = stringResource(R.string.alphabet_check_loading),
                    buttonText = null,
                    onButton = {}
                )
            } else {
                AlphabetCheckVerdictScreen(
                    state = state,
                    passed = verdict,
                    onStudyLetters = onStudyLetters,
                    onTryAgain = { viewModel.startNewSession() },
                    onDone = onBack
                )
            }
        } else {
            PracticeSummaryScreen(
                state = state,
                onPracticeAgain = { viewModel.startNewSession() },
                onBack = onBack
            )
        }
        return
    }

    // No current question (empty pool or load-time failure): terminal states.
    if (question == null) {
        if (state.errorMessage != null) {
            PracticeMessageScreen(
                title = stringResource(R.string.practice_error_title),
                message = state.errorMessage!!,
                buttonText = stringResource(R.string.practice_try_again),
                onButton = { viewModel.retryLoad() }
            )
        } else if (state.isAlphabetCheck) {
            // Check generator returned nothing (letters deck not loaded, or a
            // pre-integration stub core).
            PracticeMessageScreen(
                title = stringResource(R.string.alphabet_check_unavailable_title),
                message = stringResource(R.string.alphabet_check_unavailable_message),
                buttonText = stringResource(R.string.practice_back),
                onButton = onBack
            )
        } else {
            // Empty question pool: rated-card pool too small (or wave-3 stub core).
            PracticeMessageScreen(
                title = stringResource(R.string.practice_empty_title),
                message = stringResource(R.string.practice_empty_message),
                buttonText = stringResource(R.string.practice_back),
                onButton = onBack
            )
        }
        return
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.SpaceBetween
    ) {
        // Progress header
        Column(modifier = Modifier.fillMaxWidth()) {
            LinearProgressIndicator(
                progress = { (state.currentIndex.toFloat() + 1) / state.questions.size },
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = stringResource(
                    R.string.practice_progress,
                    state.currentIndex + 1,
                    state.questions.size
                ),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        // Inline error banner (e.g. an answer check failed) — question stays put.
        if (state.errorMessage != null) {
            Text(
                text = state.errorMessage!!,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp)
            )
        }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .padding(vertical = 24.dp),
            verticalArrangement = Arrangement.Center,
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            if (state.isAlphabetCheck) {
                // Check mode: every question is a 4-choice MCQ. A subtitle
                // names the direction; the prompt is the glyph pair or the
                // letter name (glyph prompts get the Greek serif).
                val glyphToName = state.checkDirections
                    .getOrElse(state.currentIndex) { true }
                Text(
                    text = stringResource(
                        if (glyphToName) R.string.alphabet_check_direction_glyph_to_name
                        else R.string.alphabet_check_direction_name_to_glyph
                    ),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center
                )
                Spacer(modifier = Modifier.height(8.dp))
                Text(
                    text = question.promptGloss,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold,
                    fontFamily = if (glyphToName) GreekFontFamily else null,
                    textAlign = TextAlign.Center,
                    overflow = TextOverflow.Ellipsis,
                    maxLines = 3
                )
                Spacer(modifier = Modifier.height(32.dp))
                McqChoices(state = state, question = question, onSelect = viewModel::selectChoice)
            } else {
                // Gloss prompt on top (shared by both question types).
                Text(
                    text = question.promptGloss,
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold,
                    textAlign = TextAlign.Center,
                    overflow = TextOverflow.Ellipsis,
                    maxLines = 3
                )
                Spacer(modifier = Modifier.height(32.dp))

                if (isMcqQuestion(state.currentIndex)) {
                    McqChoices(state = state, question = question, onSelect = viewModel::selectChoice)
                } else {
                    TypedAnswer(
                        state = state,
                        question = question,
                        onInputChange = viewModel::onTypedInputChange,
                        onSubmit = viewModel::submitTypedAnswer
                    )
                }
            }
        }

        if (state.feedback != null) {
            Button(
                onClick = { viewModel.continueToNext() },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = stringResource(R.string.practice_continue),
                    style = MaterialTheme.typography.titleMedium
                )
            }
        }
    }
}

/** MCQ body: Greek choices as large buttons with immediate right/wrong tinting. */
@Composable
private fun ColumnScope.McqChoices(
    state: PracticeState,
    question: McqData,
    onSelect: (Int) -> Unit
) {
    val feedback = state.feedback
    Column(
        modifier = Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        question.choices.forEachIndexed { index, choice ->
            // After feedback: correct choice goes green-ish (theme secondary),
            // the tapped wrong choice goes error-colored; taps become no-ops
            // (PracticeViewModel.selectChoice ignores them once answered).
            val colors = when {
                feedback == null -> ButtonDefaults.buttonColors()
                index == question.correctIndex -> ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.secondary,
                    contentColor = MaterialTheme.colorScheme.onSecondary
                )
                index == state.selectedChoiceIndex -> ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.error,
                    contentColor = MaterialTheme.colorScheme.onError
                )
                else -> ButtonDefaults.buttonColors()
            }
            Button(
                onClick = { onSelect(index) },
                colors = colors,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(64.dp)
            ) {
                Text(
                    text = choice,
                    style = MaterialTheme.typography.titleLarge,
                    fontFamily = GreekFontFamily,
                    fontWeight = FontWeight.Normal,
                    textAlign = TextAlign.Center,
                    overflow = TextOverflow.Ellipsis,
                    maxLines = 2
                )
            }
        }
    }
}

/** Typed-answer body: field + Check affordance, then correct-form feedback. */
@Composable
private fun ColumnScope.TypedAnswer(
    state: PracticeState,
    question: McqData,
    onInputChange: (String) -> Unit,
    onSubmit: () -> Unit
) {
    val feedback = state.feedback
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        OutlinedTextField(
            value = state.typedInput,
            onValueChange = onInputChange,
            enabled = feedback == null,
            singleLine = true,
            label = { Text(stringResource(R.string.practice_typed_label)) },
            supportingText = { Text(stringResource(R.string.practice_typed_supporting)) },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { onSubmit() }),
            modifier = Modifier.fillMaxWidth()
        )
        if (feedback == null) {
            Button(
                onClick = onSubmit,
                enabled = state.typedInput.isNotBlank() && !state.isSubmitting,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = stringResource(R.string.practice_check),
                    style = MaterialTheme.typography.titleMedium
                )
            }
        } else {
            // The correct full Greek form (choices[correctIndex]) is shown for
            // EVERY outcome; only the message changes.
            Text(
                text = stringResource(
                    when {
                        !feedback.correct -> R.string.practice_feedback_wrong
                        feedback.exact -> R.string.practice_feedback_exact
                        else -> R.string.practice_feedback_correct
                    }
                ),
                style = MaterialTheme.typography.titleMedium,
                color = if (feedback.correct) {
                    MaterialTheme.colorScheme.secondary
                } else {
                    MaterialTheme.colorScheme.error
                },
                textAlign = TextAlign.Center
            )
            Text(
                text = question.correctForm,
                style = MaterialTheme.typography.headlineMedium,
                fontFamily = GreekFontFamily,
                fontWeight = FontWeight.Normal,
                textAlign = TextAlign.Center,
                overflow = TextOverflow.Ellipsis,
                maxLines = 2
            )
        }
    }
}

/**
 * Alphabet Check verdict (wave 5): replaces the normal practice summary.
 * Copy per DESIGN-alphabet-gate.md §6 — pass unlocks vocabulary and exits;
 * fail points back at the letters track or a fresh attempt (a new stratified
 * sample is generated each time).
 */
@Composable
private fun AlphabetCheckVerdictScreen(
    state: PracticeState,
    passed: Boolean,
    onStudyLetters: () -> Unit,
    onTryAgain: () -> Unit,
    onDone: () -> Unit
) {
    val score = state.results.count { it.correct }
    val total = state.questions.size
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = if (passed) {
                stringResource(R.string.alphabet_check_pass, score, total)
            } else {
                stringResource(
                    R.string.alphabet_check_fail,
                    score,
                    total,
                    PracticeViewModel.ALPHABET_CHECK_PASS_SCORE
                )
            },
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center
        )
        Spacer(modifier = Modifier.height(32.dp))
        if (passed) {
            Button(
                onClick = onDone,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = stringResource(R.string.alphabet_check_done),
                    style = MaterialTheme.typography.titleMedium
                )
            }
        } else {
            Button(
                onClick = onStudyLetters,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = stringResource(R.string.alphabet_check_study_letters),
                    style = MaterialTheme.typography.titleMedium
                )
            }
            Spacer(modifier = Modifier.height(8.dp))
            OutlinedButton(
                onClick = onTryAgain,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = stringResource(R.string.alphabet_check_try_again),
                    style = MaterialTheme.typography.titleMedium
                )
            }
        }
    }
}

@Composable
private fun PracticeSummaryScreen(
    state: PracticeState,
    onPracticeAgain: () -> Unit,
    onBack: () -> Unit
) {
    val score = state.results.count { it.correct }
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = stringResource(R.string.practice_score, score, state.questions.size),
            style = MaterialTheme.typography.displayMedium,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center,
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 24.dp)
        )
        LazyColumn(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            items(state.results) { result ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = result.greekForm,
                        style = MaterialTheme.typography.titleMedium,
                        fontFamily = GreekFontFamily,
                        overflow = TextOverflow.Ellipsis,
                        maxLines = 1,
                        modifier = Modifier.weight(1f)
                    )
                    if (result.exact) {
                        Text(
                            text = stringResource(R.string.practice_star_exact),
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.secondary,
                            modifier = Modifier.padding(end = 8.dp)
                        )
                    }
                    Text(
                        text = stringResource(
                            if (result.correct) R.string.practice_mark_correct
                            else R.string.practice_mark_wrong
                        ),
                        style = MaterialTheme.typography.titleMedium,
                        color = if (result.correct) {
                            MaterialTheme.colorScheme.secondary
                        } else {
                            MaterialTheme.colorScheme.error
                        }
                    )
                }
            }
        }
        Spacer(modifier = Modifier.height(16.dp))
        Button(
            onClick = onPracticeAgain,
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            Text(
                text = stringResource(R.string.practice_again),
                style = MaterialTheme.typography.titleMedium
            )
        }
        Spacer(modifier = Modifier.height(8.dp))
        OutlinedButton(
            onClick = onBack,
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            Text(
                text = stringResource(R.string.practice_back),
                style = MaterialTheme.typography.titleMedium
            )
        }
    }
}

/** Loading / empty / error message page, mirroring ReviewMessageScreen. Title
 *  and button are optional so the loading state can be spinner-only. */
@Composable
private fun PracticeMessageScreen(
    title: String?,
    message: String,
    buttonText: String?,
    onButton: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        if (title == null) {
            CircularProgressIndicator()
            Spacer(modifier = Modifier.height(16.dp))
        } else {
            Text(
                text = title,
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center
            )
            Spacer(modifier = Modifier.height(16.dp))
        }
        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
        if (buttonText != null) {
            Spacer(modifier = Modifier.height(32.dp))
            Button(
                onClick = onButton,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(buttonText)
            }
        }
    }
}
