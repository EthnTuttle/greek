# Android Project Structure

State of the Android app as of 2026-10 (HEAD): a working MVP. Compose UI is
fully wired to the Rust core over UniFFI — no mock data remains.

## Directory Structure

```
android/
├── gradle/
│   └── wrapper/
│       ├── gradle-wrapper.jar
│       └── gradle-wrapper.properties (Gradle 8.14.3)
├── gradlew (executable shell script)
├── gradlew.bat (Windows batch file)
├── settings.gradle.kts (project settings)
├── build.gradle.kts (root build file: AGP 8.7.3, Kotlin 2.0.21)
├── gradle.properties (project-wide Gradle settings)
├── local.properties (SDK path, gitignored)
├── .gitignore (Android-specific ignores)
├── README.md (project documentation)
└── app/
    ├── build.gradle.kts (app module config + syncAssets data-copy tasks)
    ├── proguard-rules.pro (ProGuard rules for release builds)
    └── src/
        └── main/
            ├── AndroidManifest.xml (POST_NOTIFICATIONS + ReminderReceiver)
            ├── assets/ (GITIGNORED — synced from ../data by Gradle preBuild:
            │     morphology.sqlite ~113 MB + cards/{athenaze-i,athenaze-ii,
            │     jact,dcc-core}.json = 1,140 cards)
            ├── jniLibs/ (GITIGNORED — libuniffi_bindings.so for
            │     arm64-v8a and x86_64 only)
            ├── java/com/greekapp/
            │   ├── GreekApplication.kt (Application class; loads Rust core,
            │   │   InitState gate, DataStores + Settings/ReviewSession repos)
            │   ├── MainActivity.kt (edge-to-edge entry, theme, InitState UI)
            │   ├── navigation/
            │   │   └── AppNavigation.kt (Navigation graph + bottom nav)
            │   ├── reminder/
            │   │   ├── ReminderReceiver.kt (posts the daily review notification)
            │   │   └── ReminderScheduler.kt (AlarmManager booking, DataStore sync)
            │   ├── ui/
            │   │   ├── theme/
            │   │   │   ├── Theme.kt (Material 3 light/dark themes)
            │   │   │   ├── Color.kt (parchment color scheme)
            │   │   │   └── Type.kt (typography with Greek serif font)
            │   │   └── screens/
            │   │       ├── HomeScreen.kt (due cards, streak, stats)
            │   │       ├── ReviewScreen.kt (flashcard review UI)
            │   │       ├── ReaderScreen.kt (tap-to-parse Greek text)
            │   │       ├── CurriculumScreen.kt (Athenaze chapters)
            │   │       ├── DictionaryScreen.kt (search + results)
            │   │       └── SettingsScreen.kt (dark mode + daily reminder)
            │   └── viewmodel/
            │       ├── HomeViewModel.kt (real stats from GreekCore)
            │       ├── ReviewViewModel.kt (FSRS review via GreekCore)
            │       ├── ReaderViewModel.kt (morphology lookups via GreekCore)
            │       ├── CurriculumViewModel.kt (chapter progress via GreekCore)
            │       └── DictionaryViewModel.kt (search via GreekCore)
            ├── java/uniffi/uniffi_bindings/
            │   └── uniffi_bindings.kt (GENERATED UniFFI Kotlin bindings —
            │       do not edit hand-written code here)
            └── res/
                ├── values/
                │   ├── strings.xml (all UI strings)
                │   ├── colors.xml (color resources)
                │   ├── themes.xml (light theme config)
                │   └── ic_launcher_colors.xml (launcher icon colors)
                ├── values-night/
                │   └── themes.xml (dark theme config)
                ├── mipmap-anydpi-v26/
                │   └── ic_launcher.xml (adaptive launcher icon)
                └── mipmap-{hdpi,mdpi,xhdpi,xxhdpi,xxxhdpi}/
                    └── ic_launcher.png
```

## Key Files Summary

### Build Configuration (5 files)
1. **settings.gradle.kts** — Root project settings, includes `:app` module
2. **build.gradle.kts** (root) — Plugin versions (AGP 8.7.3, Kotlin 2.0.21)
3. **app/build.gradle.kts** — App module config (Compose BOM 2024.12.01,
   DataStore, JNA) + `syncCardData` / `syncMorphologyDb` / `syncAssets` tasks
   that copy `../data/cards/*.json` and `../data/morphology.sqlite` into
   `assets/` on every `preBuild`
4. **gradle.properties** — JVM settings, AndroidX enabled
5. **local.properties** — Android SDK path (per-machine, gitignored)

### Application Core (2 files)
1. **GreekApplication.kt** — `System.loadLibrary("uniffi_bindings")`, copies
   `morphology.sqlite` from assets on first launch, merges the four card decks,
   and constructs `GreekCore.newWithCards(...)` on a background coroutine
   behind the observable `InitState` (Loading / Ready / Error). Also owns the
   two DataStores and repositories:
   - `settings` DataStore — keys `dark_mode` (Boolean, unset = follow system),
     `daily_reminder` (Boolean, default true), `auto_play_audio` (Boolean)
   - `review_session` DataStore — persisted in-progress review batch
     (`cards` JSON, `index`, `revealed`) so a review survives process death
2. **MainActivity.kt** — Entry point with Compose + edge-to-edge; gates the UI
   on `InitState`

### Reminder (2 files, framework-only: no androidx.work, no NotificationCompat)
1. **ReminderScheduler.kt** — `setEnabled(context, enabled)` books/cancels an
   inexact daily 19:00-local alarm via `AlarmManager.setAndAllowWhileIdle`
   (one-shot, re-booked after each firing); `syncFromPreferences(context)`
   re-arms from the `settings` DataStore key `daily_reminder`
2. **ReminderReceiver.kt** — BroadcastReceiver that ensures the `daily_review`
   NotificationChannel (IMPORTANCE_DEFAULT) and posts notification id 1001
   ("Greek review" / "Your cards are ready"), content intent → MainActivity

### Navigation (1 file)
1. **AppNavigation.kt** — Bottom nav bar + NavHost with 6 screens

### Theme (3 files)
1. **Color.kt** — Parchment light/dark + amber/olive accent colors
2. **Type.kt** — Material 3 typography with Greek serif font family
3. **Theme.kt** — Light/dark color schemes

### Screens (6 files)
1. **HomeScreen.kt** — Cards due, streak, stats, action buttons
2. **ReviewScreen.kt** — Flashcard front/back, 4 difficulty buttons, progress
3. **ReaderScreen.kt** — Clickable Greek text + modal bottom sheet analysis
4. **CurriculumScreen.kt** — Chapter progress; "Coming Soon" badges on chapters
   without cards are intentional
5. **DictionaryScreen.kt** — Search bar + scrollable results
   ("Morpheus glosses (LSJ pending)")
6. **SettingsScreen.kt** — Toggle switches (daily reminder, dark mode)

### ViewModels (5 files)
All expose StateFlow and fetch through `GreekApplication.greekCore` (Rust core
via UniFFI): due-card counts and stats (Home), FSRS due-queue + rating
(Review), word morphology (Reader), chapter unlock/progress (Curriculum),
gloss search (Dictionary).

### Resources
1. **strings.xml** — UI strings for all screens
2. **colors.xml** / **ic_launcher_colors.xml** — Theme + launcher colors
3. **themes.xml** / **values-night/themes.xml** — Light and dark theme styles
4. **mipmap-\*/ic_launcher** — Launcher icon (adaptive + density PNGs)

## Technical Specifications

- **Package**: com.greekapp
- **Kotlin**: 2.0.21 (JVM target 17)
- **Android Gradle Plugin**: 8.7.3
- **Gradle**: 8.14.3 (via wrapper)
- **Compile SDK**: 35 (Android 15)
- **Min SDK**: 26 (Android 8.0)
- **Target SDK**: 35 (Android 15)

### Dependencies
- Compose BOM: 2024.12.01
- Material 3 (`material3`, `material3-window-size-class`, icons-extended)
- Navigation Compose: 2.8.5
- Lifecycle: runtime-ktx / viewmodel-compose / runtime-compose 2.8.7
- Activity Compose: 1.9.3
- Core KTX: 1.15.0
- DataStore Preferences: 1.1.1
- JNA: 5.14.0 (UniFFI runtime)
- JUnit 4.13.2 / Espresso 3.6.1 / ui-test-junit4 (declared; no Android tests yet)

## Rust Integration (done)

- **GreekApplication.kt** calls `System.loadLibrary("uniffi_bindings")` and
  constructs `GreekCore` at startup
- **ViewModels** call `GreekCore.*` functions directly (mock data removed)
- **jniLibs** carries `libuniffi_bindings.so` for arm64-v8a + x86_64 (built
  from `../crates/uniffi-bindings`, a cdylib; artifacts gitignored)
- **assets** carries `morphology.sqlite` + card JSON (synced from `../data` by
  the `syncAssets` Gradle task; gitignored)
- **Generated bindings** are committed at
  `java/uniffi/uniffi_bindings/uniffi_bindings.kt`

## Build Command

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 ./gradlew assembleDebug
```

## Status

- ✅ All screens functional against the real Rust core + real data
- ✅ Parchment theme, bottom navigation, dark mode, review-session resume
- ✅ Daily-reminder framework (AlarmManager + NotificationChannel)
- ✅ Rust core: 33 unit tests passing; DB schema versioning scaffold in place
- ⚠️ Known gaps: Morpheus glosses instead of LSJ; LLM tutor and web/desktop
  apps unimplemented; no Android test suite; arm64-v8a + x86_64 ABIs only —
  see [../ANALYSIS.md](../ANALYSIS.md)
