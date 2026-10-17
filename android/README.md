# Greek Learning App - Android

Android shell for the Greek Learning App: Jetpack Compose UI over the Rust core
(`crates/uniffi-bindings`) via UniFFI + JNA.

## Architecture

- **Jetpack Compose UI** with Material 3 theming (parchment aesthetic for Ancient Greek)
- **Navigation Compose** bottom navigation between the five main screens + Settings
- **ViewModels** with StateFlow — all five are wired to real Rust calls
  (`uniffi_bindings.GreekCore`); there is no mock data left in the app
- **Rust core** loaded at startup in `GreekApplication.kt` via
  `System.loadLibrary("uniffi_bindings")`; the 113 MB asset copy + core init
  run off the main thread behind an `InitState` gate (Loading / Ready / Error)
- **Native libs** live in `app/src/main/jniLibs/` for arm64-v8a and x86_64 only
  (gitignored build artifacts produced from `../crates/uniffi-bindings`, a
  cdylib named `libuniffi_bindings.so`)

## Bundled data

Synced from `../data/` into `app/src/main/assets/` by the `syncAssets` Gradle
task (wired into `preBuild`); the assets themselves are gitignored.

- `morphology.sqlite` — 113 MB **Morpheus-derived** morphology database
  (attached as `morph` in SQLite). This is NOT LSJ: dictionary glosses in the
  UI are labelled "Morpheus glosses (LSJ pending)"
- `cards/` — 4 decks, 1,140 cards total:
  `athenaze-i` (340), `athenaze-ii` (119), `jact` (131), `dcc-core` (550)

At first launch the morphology DB is copied to internal storage
(`getDatabasePath("morphology.sqlite")`) and the four decks are merged
in-memory and handed to `GreekCore.newWithCards`.

## Screens

- **Home**: due-cards count, streak, quick actions
- **Review**: SRS flashcard review (FSRS); the in-progress session is persisted
  to DataStore and survives full process death
- **Reader**: tap-to-parse Greek text with morphological analysis
- **Curriculum**: Athenaze chapter progress — "Coming Soon" badges on chapters
  without cards are intentional
- **Dictionary**: morphology gloss search (Morpheus; LSJ pending)
- **Settings**: dark-mode (system-follow default) and daily-reminder toggles,
  persisted to DataStore

## Daily review reminder

Framework-only (no androidx.work / NotificationCompat): `AlarmManager` +
`NotificationChannel` in `com.greekapp.reminder` (`ReminderScheduler` +
`ReminderReceiver`). The Settings toggle flows through the same `settings`
DataStore the scheduler reads; the alarm is an inexact `setAndAllowWhileIdle`
booking for 19:00 local, re-booked after each firing. Notification channel:
`daily_review`.

## Building

Requires:

- JDK 17+
- Android SDK (compileSdk 35)
- Gradle 8.14.3 (via wrapper)
- Rust target toolchain for the native libs, if rebuilding the `.so` files

```bash
cd android
./gradlew assembleDebug
```

`preBuild` automatically syncs `../data/cards/*.json` and
`../data/morphology.sqlite` into app assets (see `app/build.gradle.kts`).

## Testing

- Rust core: `cargo test -p greek-core` — 33 unit tests
- Android: no unit or instrumentation tests yet (JUnit/Espresso deps are
  declared in `app/build.gradle.kts`)

## Known gaps

Pedagogy and content fixes in flight are tracked in
[ANALYSIS.md](../ANALYSIS.md) and [PLAN-PARALLEL-AGENTS.md](../PLAN-PARALLEL-AGENTS.md). In brief:
dictionary glosses are Morpheus-derived (LSJ pending); the LLM tutor and the
web/desktop apps remain unimplemented (see repo `CLAUDE.md` Phases); native
libs ship for arm64-v8a + x86_64 only (no armeabi-v7a / x86).

## Tech Stack

- Kotlin 2.0.21 (JVM target 17)
- Android Gradle Plugin 8.7.3
- Jetpack Compose BOM 2024.12.01 (Material 3)
- Navigation Compose 2.8.5
- Lifecycle ViewModel Compose 2.8.7
- DataStore Preferences 1.1.1
- JNA 5.14.0 (UniFFI runtime)
- Min SDK 26 (Android 8.0) / Target SDK 35 (Android 15)
