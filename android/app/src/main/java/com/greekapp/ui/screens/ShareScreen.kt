package com.greekapp.ui.screens

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.R
import com.greekapp.core.ShareHistoryEntry
import com.greekapp.core.ShareMilestone
import com.greekapp.viewmodel.ShareUiMessage
import com.greekapp.viewmodel.ShareViewModel
import kotlinx.coroutines.launch
import java.text.BreakIterator
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** Appendix R: only wss:// (plus ws://127.0.0.1 for local testing) is accepted. */
internal fun isValidShareRelay(url: String): Boolean {
    val t = url.trim()
    return t.startsWith("wss://") || t.startsWith("ws://127.0.0.1")
}

/** Pubkeys render truncated — middle-elided, monospace. */
internal fun truncatePubkey(hex: String): String =
    if (hex.length <= 12) hex else hex.take(8) + "…" + hex.takeLast(4)

/**
 * Grapheme count via BreakIterator (never String.length) per Appendix R.
 * The suffix counter uses an "el" character-instance regardless of the draft
 * locale so the budget matches the engine's own counting rule.
 */
internal fun graphemeCount(text: String): Int {
    val it = BreakIterator.getCharacterInstance(Locale("el"))
    it.setText(text)
    var n = 0
    while (it.next() != BreakIterator.DONE) n++
    return n
}

/** Cut [text] to at most [max] graphemes (BreakIterator, el rules). */
internal fun takeGraphemes(text: String, max: Int): String {
    val it = BreakIterator.getCharacterInstance(Locale("el"))
    it.setText(text)
    var n = 0
    var end = 0
    var next = it.next()
    while (next != BreakIterator.DONE) {
        n++
        if (n > max) break
        end = next
        next = it.next()
    }
    return text.substring(0, end)
}

private const val SHARE_SUFFIX_MAX_GRAPHEMES = 420

/** Day-level date — the engine treats post times as "that day", not the crossing second. */
internal fun formatDay(epochMs: Long): String =
    if (epochMs <= 0L) "" else SimpleDateFormat("yyyy-MM-dd", Locale.getDefault())
        .format(Date(epochMs))

/** typed icon for the engine's per-slug delivery state string. */
private fun stateIcon(state: String): ImageVector = when (state) {
    "posted", "accepted", "delivered" -> Icons.Filled.CheckCircle
    "queued", "pending", "deferred" -> Icons.Outlined.Schedule
    "failed", "error" -> Icons.Filled.Warning
    else -> Icons.Outlined.RadioButtonUnchecked
}

/** Best-effort pretty print of the engine's per-relay JSON; raw string on failure. */
internal fun prettyRelayJson(raw: String): String {
    if (raw.isBlank()) return raw
    return runCatching { org.json.JSONObject(raw).toString(2) }
        .recoverCatching { org.json.JSONArray(raw).toString(2) }
        .getOrDefault(raw)
}

/**
 * Wave-9 share-progress surface (DESIGN-progress-sharing.md + Appendix R):
 * relay/identity card, the exact engine-shipped milestone list with toggles
 * and the editor sheet, plus a status card with a manual flush. Everything
 * the user can read comes from the core; the screen never hardcodes milestone
 * copy.
 */
@Composable
fun ShareScreen(
    onBack: () -> Unit = {},
    viewModel: ShareViewModel = viewModel()
) {
    val context = LocalContext.current
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    ShareMessageEffect(
        message = state.message,
        snackbarHostState = snackbarHostState,
        onShown = viewModel::messageShown
    )

    // Relay changes elsewhere (or enable flow) land between resumes — refresh.
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) { viewModel.refresh() }

    var showAddRelay by remember { mutableStateOf(false) }

    Box(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) {
                    Icon(
                        imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = stringResource(R.string.share_back)
                    )
                }
                Text(
                    text = stringResource(R.string.share_title),
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold
                )
            }
            Spacer(modifier = Modifier.height(16.dp))

            if (!state.featureAvailable) {
                ShareCard {
                    Text(
                        text = stringResource(R.string.share_unavailable),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error
                    )
                }
                return@Column
            }

            if (!state.enabled) {
                ShareCard {
                    Text(
                        text = stringResource(R.string.share_off_hint),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Spacer(modifier = Modifier.height(16.dp))
            }

            // ---- Server & identity ----
            ShareCard {
                Text(
                    text = stringResource(R.string.share_server_card_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = if (state.pubkeyHex.isBlank()) {
                        stringResource(R.string.share_pubkey_none)
                    } else {
                        stringResource(R.string.share_pubkey, truncatePubkey(state.pubkeyHex))
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Divider()
                if (state.relays.isEmpty()) {
                    Text(
                        text = stringResource(R.string.share_consent_relays_label),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error
                    )
                } else {
                    // Bounded height: the card already lives inside the screen
                    // scroll container; ≤8 relays per the engine's own cap.
                    LazyColumn(
                        modifier = Modifier.heightIn(max = 280.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp)
                    ) {
                        items(state.relays, key = { it }) { relay ->
                            RelayRow(
                                url = relay,
                                onRemove = {
                                    viewModel.setRelays(state.relays - relay)
                                }
                            )
                        }
                    }
                }
                OutlinedButton(
                    enabled = !state.busy,
                    onClick = { showAddRelay = true }
                ) {
                    Icon(Icons.Outlined.Add, contentDescription = null)
                    Spacer(modifier = Modifier.width(4.dp))
                    Text(stringResource(R.string.share_relay_add))
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Milestones ----
            ShareCard {
                Text(
                    text = stringResource(R.string.share_milestones_card_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                LazyColumn(
                    modifier = Modifier.heightIn(max = 480.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(state.milestones, key = { it.slug }) { milestone ->
                        MilestoneRow(
                            milestone = milestone,
                            enabled = state.enabled && !state.busy,
                            onToggle = { viewModel.setMilestoneEnabled(milestone.slug, it) },
                            onClick = {
                                val currentLocale = if (context.resources.configuration
                                        .locales.get(0)?.language == "el"
                                ) "el" else "en"
                                viewModel.openEditor(milestone.slug, currentLocale)
                            }
                        )
                    }
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Status ----
            ShareCard {
                Text(
                    text = stringResource(R.string.share_status_card_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = stringResource(
                        R.string.share_status_counts,
                        state.pendingCount,
                        state.lastFlush?.deferred?.toInt() ?: 0,
                        state.lastFlush?.failed?.size ?: 0
                    ),
                    style = MaterialTheme.typography.bodyMedium
                )
                state.lastFlush?.let { report ->
                    Text(
                        text = stringResource(
                            R.string.share_flush_report,
                            report.posted.toInt(),
                            report.deferred.toInt(),
                            report.duplicates.toInt(),
                            report.failed.size
                        ),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    // Failure honesty (Appendix R): engine reason strings verbatim.
                    if (report.failed.isNotEmpty()) {
                        Text(
                            text = stringResource(R.string.share_status_failures_label),
                            style = MaterialTheme.typography.bodySmall,
                            fontWeight = FontWeight.Medium,
                            color = MaterialTheme.colorScheme.error
                        )
                        report.failed.forEach { line ->
                            Text(
                                text = line,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.error
                            )
                        }
                    }
                }
                Button(
                    enabled = state.enabled && !state.flushing && !state.busy,
                    onClick = viewModel::flushNow
                ) {
                    if (state.flushing) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp
                        )
                        Spacer(modifier = Modifier.width(8.dp))
                    }
                    Text(stringResource(R.string.share_flush_now))
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }

        SnackbarHost(
            hostState = snackbarHostState,
            modifier = Modifier.align(Alignment.BottomCenter)
        )
    }

    if (showAddRelay) {
        RelayAddDialog(
            onDismiss = { showAddRelay = false },
            onAdd = { url ->
                if (!state.relays.contains(url.trim())) {
                    viewModel.setRelays(state.relays + url.trim())
                }
                showAddRelay = false
            }
        )
    }

    state.editor?.let { editor ->
        val milestone = state.milestones.firstOrNull { it.slug == editor.slug }
        if (milestone != null) {
            MilestoneEditorSheet(
                milestone = milestone,
                viewModel = viewModel
            )
        }
    }
}

/** Card styling shared with SyncScreen. */
@Composable
internal fun ShareCard(content: @Composable ColumnScope.() -> Unit) {
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
            verticalArrangement = Arrangement.spacedBy(12.dp),
            content = content
        )
    }
}

/** Maps [ShareUiMessage] one-shots to snackbar text (flush failures verbatim). */
@Composable
internal fun ShareMessageEffect(
    message: ShareUiMessage?,
    snackbarHostState: SnackbarHostState,
    onShown: () -> Unit
) {
    val context = LocalContext.current
    LaunchedEffect(message) {
        val text = when (val m = message) {
            is ShareUiMessage.Enabled -> context.getString(R.string.share_enabled_msg)
            is ShareUiMessage.Disabled -> context.getString(R.string.share_disabled_msg)
            is ShareUiMessage.RelaysSaved -> context.getString(R.string.share_relays_saved)
            is ShareUiMessage.TemplateSaved -> context.getString(R.string.share_template_saved)
            is ShareUiMessage.Flushed ->
                if (m.report.failed.isEmpty()) {
                    context.getString(
                        R.string.share_flush_report,
                        m.report.posted.toInt(),
                        m.report.deferred.toInt(),
                        m.report.duplicates.toInt(),
                        0
                    )
                } else {
                    // Appendix R failure honesty: engine strings verbatim.
                    m.report.failed.joinToString("\n")
                }
            is ShareUiMessage.Error -> context.getString(R.string.share_error, m.reason)
            null -> null
        }
        if (text != null) {
            snackbarHostState.showSnackbar(text)
            onShown()
        }
    }
}

@Composable
private fun RelayRow(url: String, onRemove: () -> Unit) {
    val valid = isValidShareRelay(url)
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(
            imageVector = if (valid) Icons.Filled.CheckCircle else Icons.Filled.Warning,
            contentDescription = null,
            tint = if (valid) {
                MaterialTheme.colorScheme.primary
            } else {
                MaterialTheme.colorScheme.error
            },
            modifier = Modifier.size(18.dp)
        )
        Spacer(modifier = Modifier.width(8.dp))
        Text(
            text = url,
            style = MaterialTheme.typography.bodyMedium,
            fontFamily = FontFamily.Monospace,
            modifier = Modifier.weight(1f)
        )
        IconButton(onClick = onRemove) {
            Icon(
                imageVector = Icons.Filled.Delete,
                contentDescription = stringResource(R.string.share_relay_remove_desc, url)
            )
        }
    }
}

/** Structured add dialog — the relay list is row-edited, never CSV. */
@Composable
internal fun RelayAddDialog(
    onDismiss: () -> Unit,
    onAdd: (String) -> Unit
) {
    var url by remember { mutableStateOf("") }
    val trimmed = url.trim()
    val isValid = isValidShareRelay(trimmed)
    val showError = trimmed.isNotEmpty() && !isValid
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.share_relay_add_dialog_title)) },
        text = {
            OutlinedTextField(
                value = url,
                onValueChange = { url = it },
                label = { Text(stringResource(R.string.share_relay_custom_hint)) },
                singleLine = true,
                isError = showError,
                supportingText = {
                    if (showError) Text(stringResource(R.string.share_relay_invalid))
                },
                modifier = Modifier.fillMaxWidth()
            )
        },
        confirmButton = {
            TextButton(
                enabled = trimmed.isNotEmpty() && isValid,
                onClick = { onAdd(trimmed) }
            ) { Text(stringResource(R.string.share_relay_add)) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.share_cancel)) }
        }
    )
}

@Composable
private fun MilestoneRow(
    milestone: ShareMilestone,
    enabled: Boolean,
    onToggle: (Boolean) -> Unit,
    onClick: () -> Unit
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                text = "${milestone.titleEn} · ${milestone.titleEl}",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.weight(1f)
            )
            Icon(
                imageVector = stateIcon(milestone.lastState),
                contentDescription = milestone.lastState.ifBlank { null },
                tint = when (milestone.lastState) {
                    "failed", "error" -> MaterialTheme.colorScheme.error
                    "posted", "accepted", "delivered" -> MaterialTheme.colorScheme.primary
                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                },
                modifier = Modifier.size(18.dp)
            )
            Spacer(modifier = Modifier.width(4.dp))
            Switch(
                checked = milestone.enabled,
                onCheckedChange = onToggle,
                enabled = enabled
            )
        }
        // Combined EN+EL preview chip — exactly the shipped/edited text.
        Surface(
            color = MaterialTheme.colorScheme.surface,
            shape = MaterialTheme.shapes.small,
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(modifier = Modifier.padding(8.dp)) {
                Text(
                    text = milestone.templateFor("en"),
                    style = MaterialTheme.typography.bodySmall
                )
                Text(
                    text = milestone.templateFor("el"),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
        Text(
            text = if (milestone.lastPostedMs > 0L) {
                stringResource(R.string.share_last_posted, formatDay(milestone.lastPostedMs))
            } else {
                stringResource(R.string.share_never_posted)
            },
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

/** One curated suggestion row in the consent dialog. */
private data class SuggestedRelay(
    val url: String,
    val labelRes: Int
)

/**
 * Appendix R consent screen (binding): permanence + audience disclosure, the
 * exact engine render of gate.passed.check as the example, and an EMPTY relay
 * selection with curated checkboxes — relay.nostr.band requires explicit
 * opt-in labeled indexed/searchable. Confirm posts shareEnable(selected).
 *
 * Shown from the Settings master switch (first enable) AND re-shown whenever
 * sharing is re-enabled from a disabled state.
 */
@Composable
fun ShareEnableConsentDialog(
    viewModel: ShareViewModel,
    onDismiss: () -> Unit
) {
    val scope = rememberCoroutineScope()
    val state by viewModel.state.collectAsStateWithLifecycle()
    val suggestions = remember {
        listOf(
            SuggestedRelay("wss://relay.damus.io", R.string.share_relay_damus),
            SuggestedRelay("wss://nos.lol", R.string.share_relay_noslol),
            SuggestedRelay("wss://relay.nostr.band", R.string.share_relay_nostr_band_label)
        )
    }
    var selected by remember { mutableStateOf(emptySet<String>()) }
    var customRelay by remember { mutableStateOf("") }
    var relayTouchedError by remember { mutableStateOf(false) }
    // EXACT engine render of the gate.passed.check post (not a hand-typed copy).
    val examplePost = produceState<String?>(initialValue = null) {
        value = viewModel.examplePreview(slug = "gate.passed.check")
    }

    val customTrimmed = customRelay.trim()
    val customValid = isValidShareRelay(customTrimmed)
    val chosen = selected + if (customTrimmed.isNotEmpty() && customValid) {
        setOf(customTrimmed)
    } else emptySet()
    val mixesPublicAndCustom = selected.isNotEmpty() &&
        customTrimmed.isNotEmpty() && customValid

    // Only leave the dialog once the core actually flipped to enabled.
    LaunchedEffect(state.enabled) {
        if (state.enabled) onDismiss()
    }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false)
    ) {
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(20.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = stringResource(R.string.share_consent_title),
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.weight(1f)
                    )
                    IconButton(onClick = onDismiss) {
                        Icon(
                            imageVector = Icons.Filled.Close,
                            contentDescription = stringResource(R.string.share_cancel)
                        )
                    }
                }

                Text(
                    text = stringResource(R.string.share_consent_permanence),
                    style = MaterialTheme.typography.bodyMedium
                )

                Text(
                    text = stringResource(R.string.share_consent_example_label),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Medium
                )
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = MaterialTheme.shapes.small,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        text = examplePost.value
                            ?: stringResource(R.string.share_consent_example_unavailable),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(12.dp)
                    )
                }

                Divider()

                Text(
                    text = stringResource(R.string.share_consent_relays_label),
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Medium
                )
                suggestions.forEach { suggestion ->
                    val checked = suggestion.url in selected
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .selectable(
                                selected = checked,
                                onClick = {
                                    selected = if (checked) {
                                        selected - suggestion.url
                                    } else {
                                        selected + suggestion.url
                                    }
                                }
                            ),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Checkbox(checked = checked, onCheckedChange = null)
                        Text(
                            text = stringResource(suggestion.labelRes),
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                }

                OutlinedTextField(
                    value = customRelay,
                    onValueChange = {
                        customRelay = it
                        relayTouchedError = true
                    },
                    label = { Text(stringResource(R.string.share_relay_custom_hint)) },
                    singleLine = true,
                    isError = relayTouchedError && customTrimmed.isNotEmpty() && !customValid,
                    supportingText = {
                        if (relayTouchedError && customTrimmed.isNotEmpty() && !customValid) {
                            Text(stringResource(R.string.share_relay_invalid))
                        }
                    },
                    modifier = Modifier.fillMaxWidth()
                )

                if (mixesPublicAndCustom) {
                    Text(
                        text = stringResource(R.string.share_consent_mixing_warning),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error
                    )
                }

                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End
                ) {
                    TextButton(onClick = onDismiss) {
                        Text(stringResource(R.string.share_consent_cancel))
                    }
                    Spacer(modifier = Modifier.width(8.dp))
                    Button(
                        enabled = !state.busy && chosen.isNotEmpty(),
                        onClick = {
                            scope.launch { viewModel.enable(chosen.toList()) }
                        }
                    ) { Text(stringResource(R.string.share_consent_confirm)) }
                }

                Spacer(modifier = Modifier.height(24.dp))
            }
        }
    }
}

/**
 * Disable dialog: two honest choices per Appendix R — stop posting (keep the
 * sharing identity) vs stop + forget identity (past posts stay public;
 * kind-5 deletion is off-table for v1).
 */
@Composable
fun ShareDisableDialog(
    viewModel: ShareViewModel,
    onDismiss: () -> Unit
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    LaunchedEffect(state.enabled) {
        if (!state.enabled) onDismiss()
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.share_disable_title)) },
        text = { Text(stringResource(R.string.share_disable_permanence_note)) },
        confirmButton = {
            TextButton(
                enabled = !state.busy,
                onClick = { viewModel.disable(forgetIdentity = false) }
            ) { Text(stringResource(R.string.share_disable_keep)) }
        },
        dismissButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                TextButton(
                    enabled = !state.busy,
                    onClick = { viewModel.disable(forgetIdentity = true) }
                ) { Text(stringResource(R.string.share_disable_forget)) }
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.share_cancel)) }
            }
        }
    )
}

/**
 * Milestone editor (ModalBottomSheet). Chinrest safety: outside-tap and swipe
 * dismissal are BLOCKED via sheetState.confirmValueChange — only the explicit
 * Close button (or Save) hides the sheet, so a stray palm can't drop an edit.
 * The "test post" affordance of earlier drafts is deliberately absent: the
 * editor is local-render only (Appendix R).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MilestoneEditorSheet(
    milestone: ShareMilestone,
    viewModel: ShareViewModel
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val editor = state.editor ?: return
    val scope = rememberCoroutineScope()
    var allowDismiss by remember { mutableStateOf(false) }
    val sheetState = rememberModalBottomSheetState(
        skipPartiallyExpanded = true,
        confirmValueChange = { target ->
            target != SheetValue.Hidden || allowDismiss
        }
    )

    fun close() {
        allowDismiss = true
        scope.launch { sheetState.hide() }.invokeOnCompletion {
            viewModel.closeEditor()
        }
    }
    // System back reuses the explicit close path (blocked otherwise).
    BackHandler { close() }

    ModalBottomSheet(
        onDismissRequest = { viewModel.closeEditor() },
        sheetState = sheetState
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp)
                .padding(bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = "${milestone.titleEn} · ${milestone.titleEl}",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.weight(1f)
                )
                TextButton(onClick = { close() }) {
                    Text(stringResource(R.string.share_editor_close))
                }
            }

            // Locale toggle: each locale edits its own template slot.
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(
                    selected = editor.locale == "en",
                    onClick = { viewModel.setEditorLocale("en") },
                    label = { Text(stringResource(R.string.share_editor_locale_en)) }
                )
                FilterChip(
                    selected = editor.locale == "el",
                    onClick = { viewModel.setEditorLocale("el") },
                    label = { Text(stringResource(R.string.share_editor_locale_el)) }
                )
            }

            // Shipped preset, read-only reference for the active locale.
            Text(
                text = stringResource(R.string.share_editor_preset_label),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Surface(
                color = MaterialTheme.colorScheme.surfaceVariant,
                shape = MaterialTheme.shapes.small,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = if (editor.locale == "el") milestone.templateEl else milestone.templateEn,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(10.dp)
                )
            }

            // Live render from the engine (debounced 300 ms in the VM).
            Text(
                text = stringResource(R.string.share_editor_preview_label),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Surface(
                color = MaterialTheme.colorScheme.secondaryContainer,
                shape = MaterialTheme.shapes.small,
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = editor.preview.ifBlank { "…" },
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(10.dp)
                )
            }

            val count = graphemeCount(editor.draftText)
            OutlinedTextField(
                value = editor.draftText,
                onValueChange = {
                    viewModel.onEditorDraftChange(takeGraphemes(it, SHARE_SUFFIX_MAX_GRAPHEMES))
                },
                label = { Text(stringResource(R.string.share_editor_field_label)) },
                isError = editor.previewError != null,
                supportingText = {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween
                    ) {
                        // Engine's verbatim rejection string (Appendix R honesty).
                        editor.previewError?.let {
                            Text(
                                text = it,
                                color = MaterialTheme.colorScheme.error,
                                style = MaterialTheme.typography.bodySmall,
                                modifier = Modifier.weight(1f)
                            )
                        } ?: Spacer(modifier = Modifier.weight(1f))
                        Text(
                            text = stringResource(
                                R.string.share_editor_counter, count
                            ),
                            style = MaterialTheme.typography.bodySmall,
                            color = if (count >= SHARE_SUFFIX_MAX_GRAPHEMES) {
                                MaterialTheme.colorScheme.error
                            } else {
                                MaterialTheme.colorScheme.onSurfaceVariant
                            }
                        )
                    }
                },
                minLines = 2,
                modifier = Modifier.fillMaxWidth()
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    enabled = !state.busy && editor.previewError == null,
                    onClick = {
                        if (editor.draftText.isBlank()) {
                            viewModel.resetTemplate(editor.slug, editor.locale)
                        } else {
                            viewModel.saveTemplate(editor.slug, editor.locale, editor.draftText)
                        }
                    }
                ) { Text(stringResource(R.string.share_editor_save)) }
                // Reset affects ONLY the active locale slot.
                OutlinedButton(
                    enabled = !state.busy,
                    onClick = { viewModel.resetTemplate(editor.slug, editor.locale) }
                ) { Text(stringResource(R.string.share_editor_reset)) }
            }

            Divider()

            Text(
                text = stringResource(R.string.share_history_label),
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Medium
            )
            if (editor.history.isEmpty()) {
                Text(
                    text = stringResource(R.string.share_history_empty),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            } else {
                editor.history.forEach { entry ->
                    ShareHistoryRow(entry)
                }
            }
        }
    }
}

@Composable
private fun ShareHistoryRow(entry: ShareHistoryEntry) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                imageVector = stateIcon(entry.state),
                contentDescription = entry.state.ifBlank { null },
                modifier = Modifier.size(16.dp),
                tint = when (entry.state) {
                    "failed", "error" -> MaterialTheme.colorScheme.error
                    "posted", "accepted", "delivered" -> MaterialTheme.colorScheme.primary
                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                }
            )
            Spacer(modifier = Modifier.width(6.dp))
            Text(
                text = formatDay(entry.createdMs),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace
            )
            Spacer(modifier = Modifier.width(8.dp))
            Text(
                text = entry.state,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        if (entry.perRelayJson.isNotBlank()) {
            SelectionContainer {
                Text(
                    text = prettyRelayJson(entry.perRelayJson),
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}
