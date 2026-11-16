package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.GreekApplication
import com.greekapp.SettingsRepository
import com.greekapp.core.CoreProvider
import com.greekapp.core.NostrConfig
import com.greekapp.core.PairedDevice
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** One-shot snackbar payloads; the screen maps them to string resources so
 *  all user-visible copy lives in res/. */
sealed interface SyncUiMessage {
    data object BackupWritten : SyncUiMessage
    data class BackupImported(val reviews: Int, val settings: Int) : SyncUiMessage
    data class Synced(val received: Int, val sent: Int) : SyncUiMessage
    data object Joined : SyncUiMessage
    data object IdentityGenerated : SyncUiMessage
    data object RelaysSaved : SyncUiMessage
    data class Error(val reason: String) : SyncUiMessage
}

data class SyncState(
    /** Full device sync identity — the screen renders the first 12 chars. */
    val deviceId: String = "",
    val pairedDevices: List<PairedDevice> = emptyList(),
    /** Draft of the relays field; initialized from the core config. */
    val relaysCsv: String = "",
    val hasIdentity: Boolean = false,
    /** npub returned by Generate Identity; the core config exposes only the
     *  boolean, so the npub is displayable only in the session that made it. */
    val identityNpub: String? = null,
    val autolaunchEnabled: Boolean = true,
    /** Non-null while the QR/join-ticket dialog should be shown. */
    val irohTicket: String? = null,
    /**
     * Backup payload staged for the SAF CreateDocument picker. Lives in
     * ViewModel state (not `remember`) so a configuration change while the
     * picker is open can't drop it and leave the callback writing nothing.
     * Cleared by [SyncViewModel.consumePendingExport].
     */
    val pendingExport: String? = null,
    /** True while the explicit rotate-identity confirmation dialog is shown
     *  (only reachable when [hasIdentity]; see Issue-A6 gating). */
    val rotationConfirmPending: Boolean = false,
    /** Identity-secret export is a separate deliberate recovery flow. The
     *  secret is absent until the warning is confirmed and the core returns. */
    val identityExportConfirmPending: Boolean = false,
    val exportedNsec: String? = null,
    /** Copy is one-shot; it never re-reads or re-exports the secret. */
    val identityExportCopied: Boolean = false,
    val busy: Boolean = false,
    val message: SyncUiMessage? = null
)

class SyncViewModel : ViewModel() {
    private val _state = MutableStateFlow(SyncState())
    val state: StateFlow<SyncState> = _state.asStateFlow()

    private val appContext get() = GreekApplication.appContext

    init {
        refresh()
        viewModelScope.launch(Dispatchers.IO) {
            val enabled = SettingsRepository.syncAutolaunchFlow(appContext).first()
            _state.value = _state.value.copy(autolaunchEnabled = enabled)
        }
    }

    /** Reload device id, paired devices, and the nostr config from the core. */
    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            runCatching {
                val config: NostrConfig = core.syncNostrConfig()
                _state.value = _state.value.copy(
                    deviceId = core.syncDeviceId(),
                    pairedDevices = core.syncPairedDevices(),
                    relaysCsv = config.relaysCsv,
                    hasIdentity = config.hasIdentity
                )
            }.onFailure { Log.w(TAG, "refresh failed", it) }
        }
    }

    /** Fetch the backup document; [onJson] runs on the caller's scope with the
     *  payload (or null on failure) — the screen feeds it to its SAF launcher. */
    fun exportBackupJson(onJson: (String?) -> Unit) {
        busyCall(
            block = { CoreProvider.current()?.syncExportBackupJson() },
            onSuccess = { it?.let(onJson) ?: failMessage("export") },
            onFailure = { failMessage(it) }
        )
    }

    /**
     * Validate and stage a fetched backup payload for the SAF picker (the
     * picker callback retrieves it via [consumePendingExport]). The core can
     * return an `{"error": ...}`-shaped string on export failure, so anything
     * that fails [backupPayloadError] becomes an error snackbar and the picker
     * is NOT launched. Returns true when the payload was staged.
     */
    fun stagePendingExport(json: String?): Boolean {
        val reason = backupPayloadError(json)
        if (reason != null) {
            message(SyncUiMessage.Error("invalid backup: $reason"))
            return false
        }
        _state.value = _state.value.copy(pendingExport = json)
        return true
    }

    /** Take (and clear) the staged backup payload, if any. Called by the SAF
     *  picker callback, including the cancel path so nothing stale lingers. */
    fun consumePendingExport(): String? {
        val json = _state.value.pendingExport
        if (json != null) _state.value = _state.value.copy(pendingExport = null)
        return json
    }

    /** Write the fetched backup payload to the user-picked SAF document. */
    fun writeBackup(uri: android.net.Uri, json: String) {
        busyCall(
            block = {
                appContext.contentResolver.openOutputStream(uri)?.use { out ->
                    out.write(json.toByteArray(Charsets.UTF_8))
                } ?: error("openOutputStream returned null")
            },
            onSuccess = { message(SyncUiMessage.BackupWritten) },
            onFailure = { failMessage(it) }
        )
    }

    /** Merge an import document read from the SAF picker. */
    fun importBackupJson(json: String) {
        busyCall(
            block = { CoreProvider.current()?.syncImportBackupJson(json) },
            onSuccess = { stats ->
                if (stats == null) failMessage("import") else {
                    message(SyncUiMessage.BackupImported(stats.received, stats.settings))
                    refresh()
                }
            },
            onFailure = { failMessage(it) }
        )
    }

    /** Local edit of the relays draft field (not persisted until saveRelays). */
    fun onRelaysChange(csv: String) {
        _state.value = _state.value.copy(relaysCsv = csv)
    }

    fun saveRelays() {
        val csv = _state.value.relaysCsv
        busyCall(
            block = { CoreProvider.current()?.syncSetNostrRelays(csv) },
            onSuccess = { message(SyncUiMessage.RelaysSaved) },
            onFailure = { failMessage(it) }
        )
    }

    /**
     * Entry point for the "Generate identity" button. With no identity the
     * core call is harmless and runs immediately; with an identity already in
     * place the core overwrites the stored secret unconditionally (destroying
     * access to history encrypted under the old key), so the action is gated
     * behind the explicit rotation dialog ([confirmIdentityRotation]) and the
     * core is NEVER called from the no-confirm path.
     */
    fun requestGenerateIdentity() {
        _state.value = _state.value.copy(
            identityExportConfirmPending = false,
            exportedNsec = null,
            identityExportCopied = false
        )
        when (generateAction(_state.value.hasIdentity)) {
            GenerateAction.GenerateNow -> generateIdentity()
            GenerateAction.RequireConfirmation ->
                _state.value = _state.value.copy(rotationConfirmPending = true)
        }
    }

    /** User confirmed the rotation warning in the dialog: proceed to the core. */
    fun confirmIdentityRotation() {
        _state.value = _state.value.copy(rotationConfirmPending = false)
        generateIdentity()
    }

    /** Dialog dismissed without confirming: nothing reaches the core. */
    fun dismissIdentityRotation() {
        _state.value = _state.value.copy(rotationConfirmPending = false)
    }

    /** Open the explicit credential-export warning. Requesting a fresh export
     *  also clears any previously revealed key (including after rotation). */
    fun requestIdentityExport() {
        if (!_state.value.hasIdentity) return
        _state.value = _state.value.copy(
            rotationConfirmPending = false,
            identityExportConfirmPending = true,
            exportedNsec = null,
            identityExportCopied = false
        )
    }

    /** Reveal only if this specific warning is still open when I/O returns;
     *  dismissing it while busy cannot make a late secret appear. */
    fun confirmIdentityExport() {
        if (!_state.value.identityExportConfirmPending) return
        busyCall(
            block = {
                // The config accessor performs legacy vault migration. The
                // checked-in Kotlin binding already exposes secretGet; the
                // narrower Rust syncExportNostrNsec export uses this exact
                // migration-aware source and deliberately returns no Option.
                CoreProvider.current()?.syncNostrConfig()
                GreekApplication.greekCore?.secretGet("nostr_nsec")
            },
            onSuccess = { nsec ->
                val next = identityExportRevealState(
                    _state.value,
                    confirmed = _state.value.identityExportConfirmPending,
                    nsec = nsec
                )
                if (next.exportedNsec != null) _state.value = next
                else failMessage(appContext.getString(com.greekapp.R.string.sync_backup_identity_unavailable))
            },
            onFailure = {
                _state.value = _state.value.copy(identityExportConfirmPending = false)
                failMessage(it)
            }
        )
    }

    fun dismissIdentityExport() {
        _state.value = _state.value.copy(
            identityExportConfirmPending = false,
            exportedNsec = null,
            identityExportCopied = false
        )
    }

    /** Return the already-revealed value at most once for clipboard copy. */
    fun takeIdentityExportForCopy(): String? {
        val (next, value) = identityExportCopyState(_state.value)
        _state.value = next
        return value
    }

    /** Actually calls the core. Private entry is [requestGenerateIdentity] /
     *  [confirmIdentityRotation] so existing-identity overwrites always pass
     *  the confirmation gate. */
    private fun generateIdentity() {
        busyCall(
            block = { CoreProvider.current()?.syncGenerateNostrIdentity() },
            onSuccess = { npub ->
                npub?.takeIf { it.isNotBlank() }?.let {
                    _state.value = _state.value.copy(identityNpub = it)
                    message(SyncUiMessage.IdentityGenerated)
                } ?: failMessage("generate identity")
                refresh()
            },
            onFailure = { failMessage(it) }
        )
    }

    fun importNsec(nsec: String) {
        if (nsec.isBlank()) return
        _state.value = _state.value.copy(
            identityExportConfirmPending = false,
            exportedNsec = null,
            identityExportCopied = false
        )
        busyCall(
            block = { CoreProvider.current()?.syncSetNostrNsec(nsec.trim()) },
            // No exception == accepted (the stub is a no-op; the real export
            // throws on invalid keys).
            onSuccess = { message(SyncUiMessage.IdentityGenerated); refresh() },
            onFailure = { failMessage(it) }
        )
    }

    fun nostrPush() = nostrTransfer(push = true)
    fun nostrPull() = nostrTransfer(push = false)

    private fun nostrTransfer(push: Boolean) {
        busyCall(
            block = {
                val core = CoreProvider.current()
                if (push) core?.syncNostrPush() else core?.syncNostrPull()
            },
            onSuccess = { stats ->
                if (stats == null) failMessage("sync") else {
                    message(SyncUiMessage.Synced(stats.received, stats.sent))
                    refresh()
                }
            },
            onFailure = { failMessage(it) }
        )
    }

    /** Fetch this device's iroh ticket and open the QR dialog. */
    fun showPairingQr() {
        busyCall(
            block = { CoreProvider.current()?.syncIrohTicket() },
            onSuccess = { ticket ->
                ticket?.takeIf { it.isNotBlank() }?.let {
                    _state.value = _state.value.copy(irohTicket = it)
                } ?: failMessage("iroh ticket")
            },
            onFailure = { failMessage(it) }
        )
    }

    fun dismissPairingQr() {
        _state.value = _state.value.copy(irohTicket = null)
    }

    fun joinIroh(ticket: String) {
        if (ticket.isBlank()) return
        busyCall(
            block = { CoreProvider.current()?.syncIrohJoin(ticket.trim()) },
            onSuccess = {
                message(SyncUiMessage.Joined)
                // Mark the iroh transport configured so launch-time auto-sync arms.
                viewModelScope.launch(Dispatchers.IO) {
                    SettingsRepository.setIrohJoined(appContext, true)
                }
                refresh()
            },
            onFailure = { failMessage(it) }
        )
    }

    fun irohSyncNow() {
        busyCall(
            block = { CoreProvider.current()?.syncIrohNow() },
            onSuccess = { stats ->
                if (stats == null) failMessage("sync") else {
                    message(SyncUiMessage.Synced(stats.received, stats.sent))
                    refresh()
                }
            },
            onFailure = { failMessage(it) }
        )
    }

    fun setAutolaunch(enabled: Boolean) {
        _state.value = _state.value.copy(autolaunchEnabled = enabled)
        viewModelScope.launch(Dispatchers.IO) {
            SettingsRepository.setSyncAutolaunch(appContext, enabled)
        }
    }

    /** Called by the UI once the snackbar has been displayed. */
    fun messageShown() {
        _state.value = _state.value.copy(message = null)
    }

    // -- helpers ---------------------------------------------------------------

    /** Run [block] on IO with the busy flag set; [onSuccess]/[onFailure] also
     *  run on IO ([Dispatchers.Main] confinement is not required anywhere above). */
    private inline fun <T> busyCall(
        crossinline block: suspend () -> T,
        crossinline onSuccess: (T) -> Unit,
        crossinline onFailure: (Throwable) -> Unit
    ) {
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(busy = true)
            try {
                onSuccess(block())
            } catch (t: Throwable) {
                Log.w(TAG, "sync action failed", t)
                onFailure(t)
            } finally {
                _state.value = _state.value.copy(busy = false)
            }
        }
    }

    private fun message(message: SyncUiMessage) {
        _state.value = _state.value.copy(message = message)
    }

    private fun failMessage(reason: Any) {
        message(SyncUiMessage.Error(reason.toString()))
    }

    companion object {
        private const val TAG = "SyncViewModel"

        /** Pure confirmation gate for host-JVM tests and the async success
         *  path alike. A dismissed dialog, a missing confirmation, or an
         *  empty core result can never put a secret into UI state. */
        internal fun identityExportRevealState(
            state: SyncState,
            confirmed: Boolean,
            nsec: String?
        ): SyncState {
            val reveal = confirmed && state.identityExportConfirmPending && !nsec.isNullOrBlank()
            return state.copy(
                identityExportConfirmPending = false,
                exportedNsec = if (reveal) nsec else null,
                identityExportCopied = false
            )
        }

        /** Pure one-shot clipboard handoff; the value remains manually
         *  selectable after copying and no second core read is involved. */
        internal fun identityExportCopyState(state: SyncState): Pair<SyncState, String?> {
            val nsec = state.exportedNsec
            if (state.identityExportCopied || nsec == null) return state to null
            return state.copy(identityExportCopied = true) to nsec
        }

        /** How the generate-identity action may proceed for a given state. */
        enum class GenerateAction { GenerateNow, RequireConfirmation }

        /**
         * Identity-overwrite gate (A6): generating while an identity exists
         * overwrites the stored secret, so the UI must route through the
         * rotation-confirmation dialog instead of calling the core directly.
         * Static and Log-free so host-JVM tests can drive the decision.
         */
        internal fun generateAction(hasIdentity: Boolean): GenerateAction =
            if (hasIdentity) GenerateAction.RequireConfirmation
            else GenerateAction.GenerateNow

        /**
         * Minimal structural gate for a core-exported backup payload: must
         * parse as a flat JSON object, must carry a `version` field, and must
         * NOT carry an `error` key (the core signals export failure as an
         * `{"error": ...}`-shaped string, which used to be written to the
         * picked file and announced as success). Full schema validation is
         * deliberately out of scope. Returns a short reason on rejection,
         * null when the payload is acceptable. Static and Log-free for tests.
         */
        internal fun backupPayloadError(json: String?): String? {
            if (json == null) return "empty payload"
            val keys = topLevelObjectKeys(json) ?: return "not a JSON object"
            if ("error" in keys) return "core reported an export error"
            if ("version" !in keys) return "missing \"version\" field"
            return null
        }

        /** Keys of a single flat JSON object, or null when [json] is not a
         *  well-formed top-level object. Tiny hand-rolled walker (org.json is
         *  unavailable in host-JVM unit tests); string contents are skipped
         *  correctly so braces/quotes inside values can't confuse it. */
        private fun topLevelObjectKeys(json: String): Set<String>? {
            val s = json.trim()
            if (s.length < 2 || s[0] != '{') return null
            val keys = LinkedHashSet<String>()
            var i = skipWs(s, 1)
            if (i < s.length && s[i] == '}') {
                return if (skipWs(s, i + 1) == s.length) keys else null
            }
            while (i < s.length) {
                if (s[i] != '"') return null
                val (key, afterKey) = parseString(s, i) ?: return null
                keys.add(key)
                i = skipWs(s, afterKey)
                if (i >= s.length || s[i] != ':') return null
                i = skipWs(s, skipValue(s, skipWs(s, i + 1)) ?: return null)
                if (i >= s.length) return null
                when (s[i]) {
                    ',' -> i = skipWs(s, i + 1)
                    '}' -> return if (skipWs(s, i + 1) == s.length) keys else null
                    else -> return null
                }
            }
            return null
        }

        /** Parse a JSON string at [start] (must be `"`); returns the raw
         *  contents (escapes NOT decoded — only structural accuracy matters
         *  for key matching) and the index just past the closing quote. */
        private fun parseString(s: String, start: Int): Pair<String, Int>? {
            val sb = StringBuilder()
            var i = start + 1
            while (i < s.length) {
                when (val c = s[i]) {
                    '"' -> return sb.toString() to i + 1
                    '\\' -> {
                        if (i + 1 >= s.length) return null
                        sb.append(s[i + 1]) // keep escaped char past the check
                        i += 2
                    }
                    else -> { sb.append(c); i++ }
                }
            }
            return null
        }

        /** Skip one JSON value starting at [start]; returns the index just
         *  past it, or null when malformed. */
        private fun skipValue(s: String, start: Int): Int? {
            if (start >= s.length) return null
            return when (s[start]) {
                '"' -> parseString(s, start)?.second
                '{', '[' -> skipComposite(s, start)
                else -> {
                    // number / true / false / null: run to the next delimiter
                    var i = start
                    while (i < s.length && s[i] !in ",}]\" \t\n\r") i++
                    if (i > start) i else null
                }
            }
        }

        /** Skip a balanced `{...}`/`[...]` composite (strings respected). */
        private fun skipComposite(s: String, start: Int): Int? {
            var depth = 0
            var i = start
            while (i < s.length) {
                when (s[i]) {
                    '"' -> i = parseString(s, i)?.second ?: return null
                    '{', '[' -> { depth++; i++ }
                    '}', ']' -> {
                        depth--; i++
                        if (depth == 0) return i
                        if (depth < 0) return null
                    }
                    else -> i++
                }
            }
            return null
        }

        private fun skipWs(s: String, start: Int): Int {
            var i = start
            while (i < s.length && s[i].isWhitespace()) i++
            return i
        }
    }
}
