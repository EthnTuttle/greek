package com.greekapp.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.ui.theme.GreekFontFamily
import com.greekapp.viewmodel.ReviewViewModel

@Composable
fun ReviewScreen(
    onComplete: () -> Unit,
    viewModel: ReviewViewModel = viewModel()
) {
    val state by viewModel.state.collectAsState()
    val card = viewModel.currentCard

    if (state.isLoading) {
        LoadingScreen()
        return
    }

    // Load-time failure with no cards to show: offer a retry instead of a blank screen.
    if (state.errorMessage != null && card == null) {
        ReviewMessageScreen(
            title = "Something went wrong",
            message = state.errorMessage!!,
            buttonText = "Try Again",
            onButton = { viewModel.retryLoad() }
        )
        return
    }

    if (state.isComplete) {
        ReviewCompleteScreen(onDone = onComplete)
        return
    }

    // Queue is empty (nothing due, no new cards available): friendly empty state.
    if (card == null) {
        ReviewMessageScreen(
            title = "No cards due",
            message = "Great job — you're all caught up! Unlock a new chapter to keep learning.",
            buttonText = "Done",
            onButton = onComplete
        )
        return
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.SpaceBetween
    ) {
        // Progress indicator
        Column(
            modifier = Modifier.fillMaxWidth()
        ) {
            LinearProgressIndicator(
                progress = { (state.currentIndex.toFloat() + 1) / state.cards.size },
                modifier = Modifier.fillMaxWidth()
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = "${state.currentIndex + 1} of ${state.cards.size}",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }

        // Card content
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .weight(1f)
                .padding(vertical = 32.dp),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surface
            ),
            elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(24.dp),
                verticalArrangement = Arrangement.Center,
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                // Front (Greek) — pick a smaller style for longer text so it never
                // clips on narrow screens or with large system font scaling.
                val frontStyle = when {
                    card.front.length > 24 -> MaterialTheme.typography.headlineMedium
                    card.front.length > 14 -> MaterialTheme.typography.headlineLarge
                    else -> MaterialTheme.typography.displayMedium
                }
                Text(
                    text = card.front,
                    style = frontStyle,
                    fontFamily = GreekFontFamily,
                    fontWeight = FontWeight.Normal,
                    textAlign = TextAlign.Center,
                    overflow = TextOverflow.Ellipsis,
                    maxLines = 4,
                    color = MaterialTheme.colorScheme.onSurface
                )

                if (state.isAnswerRevealed) {
                    Spacer(modifier = Modifier.height(32.dp))
                    Divider(
                        modifier = Modifier.width(100.dp),
                        color = MaterialTheme.colorScheme.outline
                    )
                    Spacer(modifier = Modifier.height(32.dp))

                    // Back (English) — the letters deck has longer backs (~140
                    // chars), so step the style down as length grows and allow
                    // enough lines that the text wraps instead of clipping.
                    val backStyle = when {
                        card.back.length > 100 -> MaterialTheme.typography.titleMedium
                        card.back.length > 60 -> MaterialTheme.typography.titleLarge
                        else -> MaterialTheme.typography.headlineMedium
                    }
                    Text(
                        text = card.back,
                        style = backStyle,
                        fontWeight = FontWeight.Normal,
                        textAlign = TextAlign.Center,
                        overflow = TextOverflow.Ellipsis,
                        maxLines = 12,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        // Inline error banner (e.g. a review failed to save) — the card stays put.
        if (state.errorMessage != null) {
            Text(
                text = state.errorMessage!!,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.error,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 8.dp)
            )
        }

        // Action buttons
        if (!state.isAnswerRevealed) {
            Button(
                onClick = { viewModel.revealAnswer() },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
            ) {
                Text(
                    text = "Show Answer",
                    style = MaterialTheme.typography.titleMedium
                )
            }
        } else {
            // Interval hints under each rating (loaded while the answer was
            // hidden); when the preview is unavailable the buttons are plain.
            val preview = state.intervalPreview
            Column(
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    OutlinedButton(
                        onClick = { viewModel.answerCard(0) },
                        enabled = !state.isSubmitting,
                        modifier = Modifier
                            .weight(1f)
                            .height(56.dp),
                        colors = ButtonDefaults.outlinedButtonColors(
                            contentColor = MaterialTheme.colorScheme.error
                        )
                    ) {
                        RatingLabel(
                            label = "Again",
                            interval = preview?.let { formatIntervalLabel(it.againSecs) }
                        )
                    }
                    OutlinedButton(
                        onClick = { viewModel.answerCard(1) },
                        enabled = !state.isSubmitting,
                        modifier = Modifier
                            .weight(1f)
                            .height(56.dp)
                    ) {
                        RatingLabel(
                            label = "Hard",
                            interval = preview?.let { formatIntervalLabel(it.hardSecs) }
                        )
                    }
                }
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Button(
                        onClick = { viewModel.answerCard(2) },
                        enabled = !state.isSubmitting,
                        modifier = Modifier
                            .weight(1f)
                            .height(56.dp)
                    ) {
                        RatingLabel(
                            label = "Good",
                            interval = preview?.let { formatIntervalLabel(it.goodSecs) }
                        )
                    }
                    Button(
                        onClick = { viewModel.answerCard(3) },
                        enabled = !state.isSubmitting,
                        modifier = Modifier
                            .weight(1f)
                            .height(56.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.secondary
                        )
                    ) {
                        RatingLabel(
                            label = "Easy",
                            interval = preview?.let { formatIntervalLabel(it.easySecs) }
                        )
                    }
                }
            }
        }
    }
}

/**
 * Rating button content: the rating name, with the scheduled interval
 * ("10m", "1h", "4d", …) on a smaller second line when a preview is available.
 */
@Composable
private fun RatingLabel(label: String, interval: String?) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(label)
        if (interval != null) {
            Text(
                text = interval,
                style = MaterialTheme.typography.labelSmall
            )
        }
    }
}

/**
 * Format an interval in seconds as a compact label: minutes below an hour,
 * hours below two days, rounded days beyond that.
 */
private fun formatIntervalLabel(secs: UInt): String = when {
    secs < 3600u -> "${(secs + 30u) / 60u}m"
    secs < 172800u -> "${(secs + 1800u) / 3600u}h"
    else -> "${(secs + 43200u) / 86400u}d"
}

@Composable
private fun LoadingScreen() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        CircularProgressIndicator()
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = "Loading your cards...",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun ReviewMessageScreen(
    title: String,
    message: String,
    buttonText: String,
    onButton: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.headlineMedium,
            fontWeight = FontWeight.Bold,
            textAlign = TextAlign.Center
        )
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
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

@Composable
private fun ReviewCompleteScreen(onDone: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Great work!",
            style = MaterialTheme.typography.displayMedium,
            fontWeight = FontWeight.Bold
        )
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = "You've completed all cards for today",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
        Spacer(modifier = Modifier.height(32.dp))
        Button(
            onClick = onDone,
            modifier = Modifier
                .fillMaxWidth()
                .height(56.dp)
        ) {
            Text("Done")
        }
    }
}
