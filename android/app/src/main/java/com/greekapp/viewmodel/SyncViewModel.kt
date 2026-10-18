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

    fun generateIdentity() {
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

    private companion object {
        const val TAG = "SyncViewModel"
    }
}
