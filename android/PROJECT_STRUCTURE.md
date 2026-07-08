# Android Project Structure

Complete Android Gradle project scaffold created for the Greek Learning App.

## Directory Structure

```
android/
├── gradle/
│   └── wrapper/
│       ├── gradle-wrapper.jar (48KB, downloaded from GitHub)
│       └── gradle-wrapper.properties (Gradle 8.11.1)
├── gradlew (executable shell script)
├── gradlew.bat (Windows batch file)
├── settings.gradle.kts (project settings)
├── build.gradle.kts (root build file)
├── gradle.properties (project-wide Gradle settings)
├── local.properties (SDK path: /home/ethan/Android/Sdk)
├── .gitignore (Android-specific ignores)
├── README.md (project documentation)
└── app/
    ├── build.gradle.kts (app module build configuration)
    ├── proguard-rules.pro (ProGuard rules for release builds)
    └── src/
        └── main/
            ├── AndroidManifest.xml
            ├── assets/ (empty, ready for morphology.sqlite)
            ├── java/com/greekapp/
            │   ├── GreekApplication.kt (Application class)
            │   ├── MainActivity.kt (main activity with Compose)
            │   ├── navigation/
            │   │   └── AppNavigation.kt (Navigation graph + bottom nav)
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
            │   │       └── SettingsScreen.kt (preferences)
            │   └── viewmodel/
            │       ├── HomeViewModel.kt (home state + mock data)
            │       ├── ReviewViewModel.kt (SRS review logic + mock cards)
            │       ├── ReaderViewModel.kt (text parsing + mock dictionary)
            │       ├── CurriculumViewModel.kt (chapter progress + mock)
            │       └── DictionaryViewModel.kt (search + mock entries)
            └── res/
                ├── values/
                │   ├── strings.xml (all UI strings)
                │   ├── colors.xml (color resources)
                │   └── themes.xml (light theme config)
                └── values-night/
                    └── themes.xml (dark theme config)
```

## Key Files Summary

### Build Configuration (5 files)
1. **settings.gradle.kts** - Root project settings, includes `:app` module
2. **build.gradle.kts** (root) - Plugin versions (AGP 8.7.3, Kotlin 2.0.21)
3. **app/build.gradle.kts** - App module config with Compose BOM 2024.12.01
4. **gradle.properties** - JVM settings, AndroidX enabled
5. **local.properties** - Android SDK path

### Application Core (2 files)
1. **GreekApplication.kt** - App initialization (ready for Rust library loading)
2. **MainActivity.kt** - Entry point with Compose + edge-to-edge

### Navigation (1 file)
1. **AppNavigation.kt** - Bottom nav bar + NavHost with 6 screens

### Theme (3 files)
1. **Color.kt** - Parchment light/dark + amber/olive accent colors
2. **Type.kt** - Material 3 typography with Greek serif font family
3. **Theme.kt** - Light/dark color schemes + system bar colors

### Screens (6 files)
1. **HomeScreen.kt** - Cards due, streak card, stats, action buttons
2. **ReviewScreen.kt** - Flashcard front/back, 4 difficulty buttons, progress
3. **ReaderScreen.kt** - Clickable Greek text + modal bottom sheet analysis
4. **CurriculumScreen.kt** - Expandable sections, chapter progress bars
5. **DictionaryScreen.kt** - Search bar + scrollable results
6. **SettingsScreen.kt** - Toggle switches for preferences

### ViewModels (5 files)
All use StateFlow and contain realistic mock data:
1. **HomeViewModel.kt** - 12 due cards, 7-day streak
2. **ReviewViewModel.kt** - 12 Greek vocabulary cards from Athenaze
3. **ReaderViewModel.kt** - Sample Athenaze text + 20 word dictionary entries
4. **CurriculumViewModel.kt** - 10 chapters from Athenaze Book I + Book II stub
5. **DictionaryViewModel.kt** - 40 common Greek words with definitions

### Resources (4 files)
1. **strings.xml** - 30+ UI strings for all screens
2. **colors.xml** - Theme color definitions
3. **themes.xml** - Light theme style
4. **values-night/themes.xml** - Dark theme style

## Technical Specifications

- **Package**: com.greekapp
- **Kotlin**: 2.0.21 (JVM target 17)
- **Android Gradle Plugin**: 8.7.3
- **Gradle**: 8.11.1 (via wrapper)
- **Compile SDK**: 35 (Android 15)
- **Min SDK**: 26 (Android 8.0)
- **Target SDK**: 35 (Android 15)

### Dependencies
- Compose BOM: 2024.12.01
- Material 3
- Navigation Compose: 2.8.5
- Lifecycle ViewModel Compose: 2.8.7
- Activity Compose: 1.9.3
- Core KTX: 1.15.0

## Mock Data Summary

### Home Screen
- 12 cards due
- 7-day streak
- 127 words learned, 3 chapters, 12.5 hours

### Review Screen
12 Greek vocabulary cards:
- ὁ ἄνθρωπος → the person, human
- ἡ γῆ → the earth, land
- τὸ ἔργον → the work, deed
- λέγω → I say, I speak
- καλός → beautiful, good
- (plus 7 more)

### Reader Screen
Sample Athenaze text (Chapter 1α):
"Ὁ Δικαιόπολις ἐν τοῖς ἀγροῖς πονεῖ..."
With 20 clickable words that show morphological analysis

### Curriculum Screen
- Athenaze Book I: 10 chapters (45-61 cards each)
- Chapter 1: 100% complete (45/45)
- Chapter 2: 73% complete (38/52)
- Chapter 3: 25% complete (12/48)
- Chapters 4-10: 0% complete
- Athenaze Book II: 10 chapters (stub)

### Dictionary Screen
40 common Greek words including:
- ἄγω, ἀγρός, ἀδελφός, αἱρέω, ἀκούω
- ἀληθής, ἄλλος, ἀνήρ, ἄνθρωπος, ἀπό
- (30 more entries with full definitions)

## Build Command

```bash
cd android
JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 ./gradlew assembleDebug
```

## Integration Points (Future)

Ready for Rust UniFFI integration:

1. **GreekApplication.kt**: Uncomment `System.loadLibrary("uniffi_bindings")`
2. **ViewModels**: Replace mock data calls with `GreekCore.*` functions
3. **jniLibs**: Add `.so` files for each ABI
4. **assets**: Add `morphology.sqlite` database

All ViewModels have placeholder comments showing where Rust calls will go.

## Status

✅ Complete and self-contained
✅ All screens have working UI
✅ Mock data provides realistic preview
✅ Theme with parchment aesthetic
✅ Bottom navigation between screens
✅ Ready for Rust integration
