package com.greekapp

import android.app.Application
import android.content.Context
import android.util.Log
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import uniffi.uniffi_bindings.GreekCore

/** Observable lifecycle of the Rust core initialization. */
sealed interface InitState {
    data object Loading : InitState
    data object Ready : InitState
    data class Error(val message: String) : InitState
}

// App-wide preferences store (used by Settings + theme).
val Context.settingsDataStore: DataStore<Preferences> by preferencesDataStore(name = "settings")

object SettingsKeys {
    val DARK_MODE = booleanPreferencesKey("dark_mode")
    val DAILY_REMINDER = booleanPreferencesKey("daily_reminder")
    val AUTO_PLAY_AUDIO = booleanPreferencesKey("auto_play_audio")
}

/** Thin repository over DataStore. Dark mode is nullable: null == follow system. */
object SettingsRepository {
    fun darkModeFlow(context: Context): Flow<Boolean?> =
        context.settingsDataStore.data.map { it[SettingsKeys.DARK_MODE] }

    fun dailyReminderFlow(context: Context): Flow<Boolean> =
        context.settingsDataStore.data.map { it[SettingsKeys.DAILY_REMINDER] ?: true }

    fun autoPlayAudioFlow(context: Context): Flow<Boolean> =
        context.settingsDataStore.data.map { it[SettingsKeys.AUTO_PLAY_AUDIO] ?: true }

    suspend fun setDarkMode(context: Context, value: Boolean) {
        context.settingsDataStore.edit { it[SettingsKeys.DARK_MODE] = value }
    }

    suspend fun setDailyReminder(context: Context, value: Boolean) {
        context.settingsDataStore.edit { it[SettingsKeys.DAILY_REMINDER] = value }
    }

    suspend fun setAutoPlayAudio(context: Context, value: Boolean) {
        context.settingsDataStore.edit { it[SettingsKeys.AUTO_PLAY_AUDIO] = value }
    }
}

class GreekApplication : Application() {

    companion object {
        var greekCore: GreekCore? = null
            private set
        var isInitialized: Boolean = false
            private set

        private val _initState = MutableStateFlow<InitState>(InitState.Loading)
        val initState: StateFlow<InitState> = _initState.asStateFlow()
    }

    private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        System.loadLibrary("uniffi_bindings")

        // Heavy work (113MB asset copy + JSON merge + Rust core init) must run off the
        // main thread to avoid an ANR. UI observes [initState] and gates on it.
        appScope.launch {
            try {
                // Copy morphology.sqlite from assets to internal storage if not already present.
                val morphFile = getDatabasePath("morphology.sqlite")
                if (!morphFile.exists()) {
                    morphFile.parentFile?.mkdirs()
                    assets.open("morphology.sqlite").use { input ->
                        morphFile.outputStream().use { output ->
                            input.copyTo(output)
                        }
                    }
                }

                // Merge the card JSON arrays without a parse/re-serialize round trip:
                // strip each file's surrounding brackets and concatenate the raw contents.
                val cardFiles = listOf("athenaze-i.json", "athenaze-ii.json", "jact.json", "dcc-core.json")
                val merged = StringBuilder(256 * 1024)
                merged.append('[')
                var first = true
                for (fileName in cardFiles) {
                    val raw = assets.open("cards/$fileName").bufferedReader().use { it.readText() }.trim()
                    val inner = raw.removePrefix("[").removeSuffix("]").trim()
                    if (inner.isEmpty()) continue
                    if (!first) merged.append(',')
                    merged.append(inner)
                    first = false
                }
                merged.append(']')
                val allCardsJson = merged.toString()

                val dbPath = getDatabasePath("greek_app.db").absolutePath

                // GreekCore.newWithCards now throws GreekException on failure.
                val core = GreekCore.newWithCards(dbPath, morphFile.absolutePath, allCardsJson)
                greekCore = core
                isInitialized = true
                _initState.value = InitState.Ready

                Log.d("GreekApp", "GreekCore initialized successfully")
            } catch (e: Exception) {
                Log.e("GreekApp", "Failed to initialize GreekCore", e)
                _initState.value = InitState.Error(e.message ?: "Failed to initialize the Greek core.")
            }
        }
    }
}
