package com.greekapp.data

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/**
 * Wave-9 in-app Greek keyboard preference store.
 *
 * DEVICE-LOCAL BY DECISION (DESIGN-greek-keyboard.md, Appendix R —
 * "Preference locality (blocker)"): `greek_input_mode` lives in Preferences
 * DataStore on this device only and is NEVER written to the synced
 * `user_settings` table. A phone without a system Greek IME and a tablet
 * with one legitimately differ by definition.
 *
 * MIGRATION NOTE: Wave-9 is the first release carrying this preference, so
 * there is no synced legacy value to migrate. If a `user_settings` row named
 * "greek_input_mode" ever appears (e.g. restored from a desktop build), this
 * repository's one-time migration path is: absent local key → read the
 * synced row once → write it here as the local default. Until then the
 * default is [GreekInputMode.IN_APP].
 */

/** How the app sources Greek text input. */
enum class GreekInputMode(val storageKey: String) {
    /** On-screen polytonic keyboard panel (default). */
    IN_APP("in_app"),

    /** No panel; rely on the user's system IME. */
    IME("ime"),

    /** Panel shown as a collapsible strip above the system IME inset. */
    BOTH("both");

    companion object {
        val DEFAULT = IN_APP
        fun fromStorage(key: String?): GreekInputMode =
            entries.firstOrNull { it.storageKey == key } ?: DEFAULT
    }
}

// Separate store file (not the app "settings" store) to keep the keyboard
// contract self-contained; both are equally device-local DataStore prefs.
private val Context.greekInputDataStore: DataStore<Preferences> by preferencesDataStore(
    name = "greek_input"
)

object GreekInputRepository {
    private val KEY_MODE = stringPreferencesKey("greek_input_mode")

    fun modeFlow(context: Context): Flow<GreekInputMode> =
        context.greekInputDataStore.data.map { GreekInputMode.fromStorage(it[KEY_MODE]) }

    suspend fun setMode(context: Context, mode: GreekInputMode) {
        context.greekInputDataStore.edit { it[KEY_MODE] = mode.storageKey }
    }
}
