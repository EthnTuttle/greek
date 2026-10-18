package com.greekapp

import android.app.Application
import android.content.Context
import android.util.Log
import java.io.File
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import com.greekapp.core.CoreProvider
import com.greekapp.core.RustCoreAdapter
import com.greekapp.reminder.ReminderScheduler
import uniffi.uniffi_bindings.GreekCore

/** Observable lifecycle of the Rust core initialization. */
sealed interface InitState {
    data object Loading : InitState
    data object Ready : InitState
    data class Error(val message: String) : InitState
}

// App-wide preferences store (used by Settings + theme).
val Context.settingsDataStore: DataStore<Preferences> by preferencesDataStore(name = "settings")

// Durable store for an in-progress review session, so it survives full process
// death (swipe-away / `am kill`), not just config changes. SavedStateHandle only
// survives system-initiated death, which is why a killed session used to restart
// at "1 of N" with a freshly-built batch.
val Context.reviewSessionDataStore: DataStore<Preferences> by preferencesDataStore(name = "review_session")

object SettingsKeys {
    val DARK_MODE = booleanPreferencesKey("dark_mode")
    val DAILY_REMINDER = booleanPreferencesKey("daily_reminder")
}

object ReviewSessionKeys {
    // The full batch, serialized as a JSON array of {id, front, back}.
    val CARDS = stringPreferencesKey("cards")
    val INDEX = intPreferencesKey("index")
    val REVEALED = booleanPreferencesKey("revealed")
}

/** A persisted review session: the original batch plus where the user left off. */
data class PersistedSession(
    val cardsJson: String,
    val index: Int,
    val revealed: Boolean
)

/**
 * Durable persistence for the active review session. The batch is written once
 * when the session loads and the index/revealed flags are updated as the user
 * progresses, so a cold start restores the SAME batch at the SAME position
 * rather than rebuilding a new one.
 */
object ReviewSessionRepository {
    suspend fun load(context: Context): PersistedSession? {
        val prefs = context.reviewSessionDataStore.data.first()
        val cardsJson = prefs[ReviewSessionKeys.CARDS] ?: return null
        return PersistedSession(
            cardsJson = cardsJson,
            index = prefs[ReviewSessionKeys.INDEX] ?: 0,
            revealed = prefs[ReviewSessionKeys.REVEALED] ?: false
        )
    }

    suspend fun save(context: Context, cardsJson: String, index: Int, revealed: Boolean) {
        context.reviewSessionDataStore.edit {
            it[ReviewSessionKeys.CARDS] = cardsJson
            it[ReviewSessionKeys.INDEX] = index
            it[ReviewSessionKeys.REVEALED] = revealed
        }
    }

    suspend fun updateProgress(context: Context, index: Int, revealed: Boolean) {
        context.reviewSessionDataStore.edit {
            it[ReviewSessionKeys.INDEX] = index
            it[ReviewSessionKeys.REVEALED] = revealed
        }
    }

    suspend fun clear(context: Context) {
        context.reviewSessionDataStore.edit { it.clear() }
    }
}

/** Thin repository over DataStore. Dark mode is nullable: null == follow system. */
object SettingsRepository {
    fun darkModeFlow(context: Context): Flow<Boolean?> =
        context.settingsDataStore.data.map { it[SettingsKeys.DARK_MODE] }

    fun dailyReminderFlow(context: Context): Flow<Boolean> =
        context.settingsDataStore.data.map { it[SettingsKeys.DAILY_REMINDER] ?: true }

    suspend fun setDarkMode(context: Context, value: Boolean) {
        context.settingsDataStore.edit { it[SettingsKeys.DARK_MODE] = value }
    }

    suspend fun setDailyReminder(context: Context, value: Boolean) {
        context.settingsDataStore.edit { it[SettingsKeys.DAILY_REMINDER] = value }
    }
}

class GreekApplication : Application() {

    companion object {
        /**
         * Version of the bundled morphology.sqlite lexicon asset.
         * Bump whenever the asset changes (curated gloss overrides, schema
         * fixes) so existing installs re-copy on upgrade. v1 = Morpheus
         * baseline (July); v2 = +88 curated gloss fixes (Oct 2026).
         */
        private const val MORPHOLOGY_DB_VERSION = 2

        var greekCore: GreekCore? = null
            private set
        var isInitialized: Boolean = false
            private set

        // Application context for repositories that need DataStore access outside
        // of a composable (e.g. ReviewViewModel persisting its session).
        lateinit var appContext: Context
            private set

        private val _initState = MutableStateFlow<InitState>(InitState.Loading)
        val initState: StateFlow<InitState> = _initState.asStateFlow()
    }

    private val appScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    override fun onCreate() {
        super.onCreate()
        appContext = applicationContext
        System.loadLibrary("uniffi_bindings")

        // Heavy work (113MB asset copy + JSON merge + Rust core init) must run off the
        // main thread to avoid an ANR. UI observes [initState] and gates on it.
        appScope.launch {
            try {
                // Copy morphology.sqlite from assets to internal storage. A version
                // marker guards the 113MB copy: bundled lexicon updates (e.g. the
                // curated gloss overrides) MUST reach existing installs, so we
                // re-copy whenever the bundled version moves past the stored one.
                val morphFile = getDatabasePath("morphology.sqlite")
                val markerFile = File(filesDir, "morphology.version")
                val storedVersion = markerFile.takeIf { it.exists() }
                    ?.readText()?.trim()?.removePrefix("v")?.toIntOrNull() ?: 0
                if (!morphFile.exists() || storedVersion < MORPHOLOGY_DB_VERSION) {
                    morphFile.parentFile?.mkdirs()
                    morphFile.delete() // clean slate; WAL/SHM cannot outlive the process
                    File(morphFile.path + "-wal").delete()
                    File(morphFile.path + "-shm").delete()
                    assets.open("morphology.sqlite").use { input ->
                        morphFile.outputStream().use { output ->
                            input.copyTo(output)
                        }
                    }
                    markerFile.writeText("v" + MORPHOLOGY_DB_VERSION)
                }

                // Merge the card JSON arrays without a parse/re-serialize round trip:
                // strip each file's surrounding brackets and concatenate the raw contents.
                // Decks are discovered from assets rather than hardcoded so a new
                // deck JSON (e.g. letters.json) ships with no code change; sorted
                // for a deterministic merge order.
                val cardFiles = (assets.list("cards") ?: emptyArray())
                    .filter { it.endsWith(".json") }
                    .sorted()
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
                CoreProvider.core = RustCoreAdapter(core)
                isInitialized = true
                _initState.value = InitState.Ready

                Log.d("GreekApp", "GreekCore initialized successfully")
            } catch (e: Exception) {
                Log.e("GreekApp", "Failed to initialize GreekCore", e)
                _initState.value = InitState.Error(e.message ?: "Failed to initialize the Greek core.")
            }
        }

        // Re-arm (or cancel) the daily-reminder alarm from the persisted
        // preference on every cold start.
        ReminderScheduler.syncFromPreferences(this)
    }
}
