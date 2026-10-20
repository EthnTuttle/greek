package com.greekapp.ui.screens

import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.R
import com.greekapp.SettingsRepository
import com.greekapp.data.GreekInputMode
import com.greekapp.data.GreekInputRepository
import com.greekapp.reminder.ReminderScheduler
import com.greekapp.viewmodel.SettingsViewModel
import com.greekapp.viewmodel.ShareViewModel
import kotlinx.coroutines.launch

@Composable
fun SettingsScreen(
    /** Wave-5: navigate to the Alphabet Check destination. */
    onTakeAlphabetCheck: () -> Unit = {},
    /** Wave-6: navigate to the Device Sync destination. */
    onOpenSync: () -> Unit = {},
    /** Wave-9: navigate to the Share progress destination. */
    onOpenShare: () -> Unit = {},
    viewModel: SettingsViewModel = viewModel(),
    /** Wave-9: owns the share card state; separate instance from ShareScreen's. */
    shareViewModel: ShareViewModel = viewModel()
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val notificationsEnabled by SettingsRepository.dailyReminderFlow(context)
        .collectAsStateWithLifecycle(initialValue = true)
    // Dark mode: null == follow system; the switch reflects the effective state.
    val darkModePref by SettingsRepository.darkModeFlow(context)
        .collectAsStateWithLifecycle(initialValue = null)
    val darkModeEnabled = darkModePref ?: isSystemInDarkTheme()

    val state by viewModel.state.collectAsStateWithLifecycle()
    val shareState by shareViewModel.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    var showSkipDialog by remember { mutableStateOf(false) }
    var showShareConsent by remember { mutableStateOf(false) }
    var showShareDisable by remember { mutableStateOf(false) }

    // Gate state can change on a check entered from this screen — refresh on
    // every return so the row hides immediately after a pass. Same for the
    // share badge count after a manual flush on the Share screen.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        viewModel.refreshGate()
        shareViewModel.refresh()
    }

    // Share one-shot messages (enabled/disabled/errors) ride the same snackbar.
    ShareMessageEffect(
        message = shareState.message,
        snackbarHostState = snackbarHostState,
        onShown = shareViewModel::messageShown
    )

    // One-shot snackbar after a successful skip declaration.
    val skipMessage = stringResource(R.string.settings_alphabet_skipped)
    LaunchedEffect(state.skipConfirmed) {
        if (state.skipConfirmed) {
            snackbarHostState.showSnackbar(skipMessage)
            viewModel.skipConfirmationShown()
        }
    }

    Box(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp)
        ) {
            Text(
                text = "Settings",
                style = MaterialTheme.typography.headlineMedium,
                fontWeight = FontWeight.Bold
            )
            Spacer(modifier = Modifier.height(16.dp))

            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant
                )
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    // Daily reminder
                    SettingItem(
                        title = "Daily reminder",
                        description = "Get notified when you have cards due",
                        checked = notificationsEnabled,
                        onCheckedChange = {
                            scope.launch { SettingsRepository.setDailyReminder(context, it) }
                            ReminderScheduler.setEnabled(context, it)
                        }
                    )

                    Divider()

                    // Dark mode
                    SettingItem(
                        title = "Dark mode",
                        description = "Use dark theme for the app",
                        checked = darkModeEnabled,
                        onCheckedChange = { scope.launch { SettingsRepository.setDarkMode(context, it) } }
                    )

                    Divider()

                    // Wave-9: Greek input mode — DEVICE-LOCAL tri-state
                    // (DataStore, never synced; see GreekInputRepository).
                    GreekInputModeRow()

                    // Alphabet gate skip lane — only while the gate is unmet.
                    if (state.alphabetGateUnmet) {
                        Divider()
                        SettingActionItem(
                            title = stringResource(R.string.settings_alphabet_row_title),
                            description = stringResource(R.string.settings_alphabet_row_description),
                            onClick = { showSkipDialog = true }
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(24.dp))

            // Wave-6: entry point to the dedicated Device Sync screen.
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant
                )
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp)
                ) {
                    SettingActionItem(
                        title = stringResource(R.string.settings_sync_row_title),
                        description = stringResource(R.string.settings_sync_row_description),
                        onClick = onOpenSync
                    )
                }
            }

            Spacer(modifier = Modifier.height(24.dp))

            // Wave-9: Share progress (nostr) — off by default, opt-in only.
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant
                )
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    SettingItem(
                        title = stringResource(R.string.settings_share_row_title),
                        description = stringResource(
                            if (shareState.featureAvailable) {
                                R.string.settings_share_row_description
                            } else {
                                R.string.share_unavailable
                            }
                        ),
                        checked = shareState.enabled,
                        onCheckedChange = { on ->
                            // Enable is gated on the Appendix R consent screen;
                            // disable offers keep-vs-forget-identity choices.
                            if (on) showShareConsent = true else showShareDisable = true
                        }
                    )
                    // Quiet persistent badge whenever anything is waiting (App R).
                    if (shareState.pendingCount > 0) {
                        AssistChip(
                            onClick = onOpenShare,
                            label = {
                                Text(
                                    stringResource(
                                        R.string.settings_share_pending_badge,
                                        shareState.pendingCount
                                    )
                                )
                            }
                        )
                    }
                    SettingActionItem(
                        title = stringResource(R.string.settings_share_manage),
                        description = stringResource(R.string.settings_share_manage_description),
                        onClick = onOpenShare
                    )
                }
            }

            Spacer(modifier = Modifier.height(24.dp))

            // About section
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant
                )
            ) {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Text(
                        text = "About",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold
                    )
                    Text(
                        text = "Greek Learning App",
                        style = MaterialTheme.typography.bodyLarge
                    )
                    Text(
                        text = "Version 1.0.0",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        }

        SnackbarHost(
            hostState = snackbarHostState,
            modifier = Modifier.align(Alignment.BottomCenter)
        )
    }

    // Skip confirmation: check lane is offered first; skipping is logged
    // core-side as self-reported.
    if (showSkipDialog) {
        AlertDialog(
            onDismissRequest = { showSkipDialog = false },
            text = { Text(stringResource(R.string.settings_alphabet_dialog_text)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        showSkipDialog = false
                        viewModel.declareAlphabetKnown()
                    }
                ) {
                    Text(stringResource(R.string.settings_alphabet_dialog_skip))
                }
            },
            dismissButton = {
                TextButton(
                    onClick = {
                        showSkipDialog = false
                        onTakeAlphabetCheck()
                    }
                ) {
                    Text(stringResource(R.string.settings_alphabet_dialog_check))
                }
            }
        )
    }

    // Wave-9: first-enable consent (permanence disclosure + empty relay picker).
    if (showShareConsent) {
        ShareEnableConsentDialog(
            viewModel = shareViewModel,
            onDismiss = { showShareConsent = false }
        )
    }

    // Wave-9: disable = stop posting (keep identity) vs forget identity.
    if (showShareDisable) {
        ShareDisableDialog(
            viewModel = shareViewModel,
            onDismiss = { showShareDisable = false }
        )
    }
}

@Composable
private fun SettingItem(
    title: String,
    description: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(
            modifier = Modifier.weight(1f)
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Medium
            )
            Text(
                text = description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange
        )
    }
}

/**
 * Wave-9 "Greek input" row: tri-state in-app keyboard / system IME / both.
 * Device-local (a phone without an IME and a tablet with one differ by
 * definition) — reads/writes [GreekInputRepository], never user_settings.
 */
@Composable
private fun GreekInputModeRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val mode by GreekInputRepository.modeFlow(context)
        .collectAsStateWithLifecycle(initialValue = GreekInputMode.DEFAULT)
    var showDialog by remember { mutableStateOf(false) }

    fun label(m: GreekInputMode): String = when (m) {
        GreekInputMode.IN_APP -> "In-app keyboard"
        GreekInputMode.IME -> "System IME"
        GreekInputMode.BOTH -> "Both (strip above IME)"
    }

    SettingActionItem(
        title = "Greek input",
        description = "Current: ${label(mode)}. Applies only to this device.",
        onClick = { showDialog = true }
    )

    if (showDialog) {
        AlertDialog(
            onDismissRequest = { showDialog = false },
            title = { Text("Greek input") },
            text = {
                Column {
                    GreekInputMode.entries.forEach { option ->
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable {
                                    scope.launch {
                                        GreekInputRepository.setMode(context, option)
                                    }
                                    showDialog = false
                                }
                                .padding(vertical = 4.dp)
                        ) {
                            RadioButton(selected = mode == option, onClick = null)
                            Spacer(Modifier.width(8.dp))
                            Text(label(option))
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showDialog = false }) { Text("Cancel") }
            }
        )
    }
}

/** Non-toggle settings row: title + description, whole row taps to an action. */
@Composable
private fun SettingActionItem(
    title: String,
    description: String,
    onClick: () -> Unit
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(
            modifier = Modifier.weight(1f)
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Medium
            )
            Text(
                text = description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}
