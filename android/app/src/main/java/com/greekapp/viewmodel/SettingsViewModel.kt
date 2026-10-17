package com.greekapp.viewmodel

import android.util.Log
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.greekapp.core.CoreProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class SettingsState(
    /**
     * Wave-5 alphabet gate is unmet → show the "I already know the Greek
     * alphabet" row. Default false = row hidden until the core says unmet
     * (also hidden when the deck is empty / the call fails).
     */
    val alphabetGateUnmet: Boolean = false,
    /** One-shot event: a skip was recorded → the screen shows the snackbar. */
    val skipConfirmed: Boolean = false
)

class SettingsViewModel : ViewModel() {
    private val _state = MutableStateFlow(SettingsState())
    val state: StateFlow<SettingsState> = _state.asStateFlow()

    init { refreshGate() }

    /**
     * Re-evaluate the gate (call on ON_RESUME): the row disappears as soon as
     * any lane satisfies it — a passed check entered from this very screen
     * included.
     */
    fun refreshGate() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            val unmet = try {
                val gate = core.getAlphabetGateState()
                !gate.satisfied && gate.lettersTotal > 0u
            } catch (e: Exception) {
                Log.w(TAG, "getAlphabetGateState failed", e)
                false
            }
            _state.value = _state.value.copy(alphabetGateUnmet = unmet)
        }
    }

    /**
     * "I already know the alphabet" lane (logged core-side as self-reported).
     * On success the row hides and the snackbar event fires; on failure the
     * gate is re-evaluated and the row simply stays.
     */
    fun declareAlphabetKnown() {
        viewModelScope.launch(Dispatchers.IO) {
            val core = CoreProvider.current() ?: return@launch
            val declared = try {
                core.declareAlphabetKnown()
            } catch (e: Exception) {
                Log.e(TAG, "declareAlphabetKnown failed", e)
                false
            }
            if (declared) {
                _state.value = _state.value.copy(
                    alphabetGateUnmet = false,
                    skipConfirmed = true
                )
            } else {
                refreshGate()
            }
        }
    }

    /** Called by the UI once the skip snackbar has been displayed. */
    fun skipConfirmationShown() {
        _state.value = _state.value.copy(skipConfirmed = false)
    }

    private companion object {
        const val TAG = "SettingsViewModel"
    }
}
