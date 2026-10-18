package com.greekapp.ui.screens

import android.graphics.Bitmap
import android.graphics.Color
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.greekapp.R
import com.greekapp.core.PairedDevice
import com.greekapp.viewmodel.SyncUiMessage
import com.greekapp.viewmodel.SyncViewModel
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Wave-6 device-sync surface (beta): backup export/import via SAF, direct
 * pairing over iroh (QR code or pasted ticket), nostr relay push/pull, the
 * paired-device list, and the launch-time auto-sync toggle.
 */
@Composable
fun SyncScreen(
    onBack: () -> Unit = {},
    viewModel: SyncViewModel = viewModel()
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    // SAF launchers. Export: fetch the document from the core, then launch the
    // picker; the chosen Uri gets the prefetched payload written into it.
    var pendingExport by remember { mutableStateOf<String?>(null) }
    val exportLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.CreateDocument("application/json")
    ) { uri ->
        val json = pendingExport
        pendingExport = null
        if (uri != null && json != null) viewModel.writeBackup(uri, json)
    }
    val importLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocument()
    ) { uri ->
        if (uri != null) {
            scope.launch(Dispatchers.IO) {
                runCatching {
                    context.contentResolver.openInputStream(uri)?.use { input ->
                        input.bufferedReader().readText()
                    }
                }.getOrNull()?.let { viewModel.importBackupJson(it) }
            }
        }
    }

    // One-shot snackbar messages (copy lives in res/).
    val msgBackupWritten = stringResource(R.string.sync_backup_written)
    val msgCopied = stringResource(R.string.sync_copied)
    val msgIdentityGenerated = stringResource(R.string.sync_identity_generated)
    val msgRelaysSaved = stringResource(R.string.sync_saved)
    val msgJoined = stringResource(R.string.sync_joined)
    LaunchedEffect(state.message) {
        val text = when (val m = state.message) {
            is SyncUiMessage.BackupWritten -> msgBackupWritten
            is SyncUiMessage.BackupImported ->
                context.getString(R.string.sync_backup_merged, m.reviews, m.settings)
            is SyncUiMessage.Synced ->
                context.getString(R.string.sync_sync_result, m.received, m.sent)
            is SyncUiMessage.Joined -> msgJoined
            is SyncUiMessage.IdentityGenerated -> msgIdentityGenerated
            is SyncUiMessage.RelaysSaved -> msgRelaysSaved
            is SyncUiMessage.Error -> context.getString(R.string.sync_error, m.reason)
            null -> null
        }
        if (text != null) {
            snackbarHostState.showSnackbar(text)
            viewModel.messageShown()
        }
    }

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
                        contentDescription = stringResource(R.string.sync_back)
                    )
                }
                Text(
                    text = stringResource(R.string.sync_title),
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.Bold
                )
            }
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = stringResource(R.string.sync_beta_disclaimer),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(modifier = Modifier.height(16.dp))

            // ---- This device ----
            SyncCard {
                Text(
                    text = stringResource(R.string.sync_this_device),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = stringResource(R.string.sync_device_id, state.deviceId.take(12)),
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = {
                            // Callback arrives on the VM's IO dispatcher —
                            // hop to Main (rememberCoroutineScope) before
                            // touching the ActivityResultLauncher.
                            viewModel.exportBackupJson { json ->
                                scope.launch {
                                    if (json != null) {
                                        pendingExport = json
                                        val stamp = SimpleDateFormat("yyyyMMdd", Locale.US).format(Date())
                                        exportLauncher.launch("greek-backup-$stamp.json")
                                    }
                                }
                            }
                        }
                    ) { Text(stringResource(R.string.sync_export_backup)) }
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = {
                            importLauncher.launch(
                                arrayOf("application/json", "text/*", "application/octet-stream")
                            )
                        }
                    ) { Text(stringResource(R.string.sync_import_backup)) }
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Direct pairing (iroh) ----
            SyncCard {
                Text(
                    text = stringResource(R.string.sync_iroh_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                var joinTicket by remember { mutableStateOf("") }
                OutlinedButton(
                    enabled = !state.busy,
                    onClick = { viewModel.showPairingQr() }
                ) { Text(stringResource(R.string.sync_show_qr)) }
                OutlinedTextField(
                    value = joinTicket,
                    onValueChange = { joinTicket = it },
                    label = { Text(stringResource(R.string.sync_join_label)) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        enabled = !state.busy && joinTicket.isNotBlank(),
                        onClick = { viewModel.joinIroh(joinTicket) }
                    ) { Text(stringResource(R.string.sync_join)) }
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = { viewModel.irohSyncNow() }
                    ) { Text(stringResource(R.string.sync_sync_now)) }
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Relay sync (nostr) ----
            SyncCard {
                Text(
                    text = stringResource(R.string.sync_nostr_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                OutlinedTextField(
                    value = state.relaysCsv,
                    onValueChange = viewModel::onRelaysChange,
                    label = { Text(stringResource(R.string.sync_relays_label)) },
                    placeholder = {
                        Text(stringResource(R.string.sync_relays_placeholder))
                    },
                    minLines = 2,
                    modifier = Modifier.fillMaxWidth()
                )
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = { viewModel.saveRelays() }
                    ) { Text(stringResource(R.string.sync_save)) }
                    Text(
                        text = when {
                            state.identityNpub != null ->
                                stringResource(R.string.sync_identity_npub, state.identityNpub!!.take(16))
                            state.hasIdentity -> stringResource(R.string.sync_identity_configured)
                            else -> stringResource(R.string.sync_identity_none)
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f)
                    )
                }
                var nsec by remember { mutableStateOf("") }
                OutlinedTextField(
                    value = nsec,
                    onValueChange = { nsec = it },
                    label = { Text(stringResource(R.string.sync_nsec_label)) },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    modifier = Modifier.fillMaxWidth()
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = { viewModel.generateIdentity() }
                    ) { Text(stringResource(R.string.sync_generate_identity)) }
                    OutlinedButton(
                        enabled = !state.busy && nsec.isNotBlank(),
                        onClick = { viewModel.importNsec(nsec); nsec = "" }
                    ) { Text(stringResource(R.string.sync_import_nsec)) }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        enabled = !state.busy,
                        onClick = { viewModel.nostrPush() }
                    ) { Text(stringResource(R.string.sync_push_now)) }
                    OutlinedButton(
                        enabled = !state.busy,
                        onClick = { viewModel.nostrPull() }
                    ) { Text(stringResource(R.string.sync_pull_now)) }
                }
                Text(
                    text = stringResource(R.string.sync_nostr_info),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Paired devices ----
            SyncCard {
                Text(
                    text = stringResource(R.string.sync_paired_title),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                if (state.pairedDevices.isEmpty()) {
                    Text(
                        text = stringResource(R.string.sync_paired_empty),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                } else {
                    state.pairedDevices.forEachIndexed { i, device ->
                        if (i > 0) Divider()
                        PairedDeviceRow(device)
                    }
                }
            }

            Spacer(modifier = Modifier.height(16.dp))

            // ---- Auto-sync toggle ----
            SyncCard {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = stringResource(R.string.sync_autolaunch_title),
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.Medium
                        )
                        Text(
                            text = stringResource(R.string.sync_autolaunch_desc),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                    Switch(
                        checked = state.autolaunchEnabled,
                        onCheckedChange = { viewModel.setAutolaunch(it) }
                    )
                }
            }

            Spacer(modifier = Modifier.height(24.dp))
        }

        SnackbarHost(
            hostState = snackbarHostState,
            modifier = Modifier.align(Alignment.BottomCenter)
        )
    }

    // Pairing QR dialog: the ticket string doubles as selectable mono text
    // plus a copy button for devices that can't scan.
    state.irohTicket?.let { ticket ->
        val clipboard = LocalClipboardManager.current
        AlertDialog(
            onDismissRequest = viewModel::dismissPairingQr,
            title = { Text(stringResource(R.string.sync_show_qr)) },
            text = {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    rememberQrBitmap(ticket)?.let { bmp ->
                        Image(
                            bitmap = bmp,
                            contentDescription = stringResource(R.string.sync_show_qr),
                            modifier = Modifier.fillMaxWidth().aspectRatio(1f)
                        )
                    }
                    SelectionContainer {
                        Text(
                            text = ticket,
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    clipboard.setText(AnnotatedString(ticket))
                    scope.launch { snackbarHostState.showSnackbar(msgCopied) }
                }) { Text(stringResource(R.string.sync_copy)) }
            },
            dismissButton = {
                TextButton(onClick = viewModel::dismissPairingQr) {
                    Text(stringResource(R.string.sync_close))
                }
            }
        )
    }
}

/** Card styling shared with SettingsScreen (surfaceVariant, 16dp padding). */
@Composable
private fun SyncCard(content: @Composable ColumnScope.() -> Unit) {
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

@Composable
private fun PairedDeviceRow(device: PairedDevice) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(
            text = device.label.ifBlank { device.deviceId.take(12) },
            style = MaterialTheme.typography.titleMedium,
            fontWeight = FontWeight.Medium
        )
        Text(
            text = lastSyncLabel(device.lastSyncHlc),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

/**
 * Humanize an HLC "last sync" stamp: best-effort epoch-digit prefix → relative
 * time; blank → "never"; anything unparseable is shown verbatim.
 */
@Composable
private fun lastSyncLabel(hlc: String): String {
    if (hlc.isBlank()) return stringResource(R.string.sync_last_sync_never)
    val rawEpoch = hlc.takeWhile { it.isDigit() }.toLongOrNull()
        ?: return stringResource(R.string.sync_last_sync_at, hlc)
    val epochMillis = if (rawEpoch > 100_000_000_000L) rawEpoch else rawEpoch * 1000L
    val diff = System.currentTimeMillis() - epochMillis
    return when {
        diff < 60_000L -> stringResource(R.string.sync_just_now)
        diff < 3_600_000L -> stringResource(R.string.sync_minutes_ago, diff / 60_000L)
        diff < 86_400_000L -> stringResource(R.string.sync_hours_ago, diff / 3_600_000L)
        else -> stringResource(R.string.sync_days_ago, diff / 86_400_000L)
    }
}

/** Render [content] as a QR Code bitmap via ZXing; null while encoding fails. */
@Composable
private fun rememberQrBitmap(content: String, sizePx: Int = 512): ImageBitmap? {
    return produceState<ImageBitmap?>(initialValue = null, content) {
        value = withContext(Dispatchers.Default) {
            runCatching {
                val matrix = QRCodeWriter().encode(content, BarcodeFormat.QR_CODE, sizePx, sizePx)
                val w = matrix.width
                val h = matrix.height
                val pixels = IntArray(w * h) { i ->
                    if (matrix[i % w, i / w]) Color.BLACK else Color.WHITE
                }
                Bitmap.createBitmap(pixels, w, h, Bitmap.Config.RGB_565).asImageBitmap()
            }.getOrNull()
        }
    }.value
}
