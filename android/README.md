# Greek Learning App - Android

Android shell for the Greek Learning App, built with Jetpack Compose.

## Structure

- **Jetpack Compose UI** with Material 3 theming (parchment aesthetic for Ancient Greek)
- **Navigation Compose** for bottom navigation between screens
- **ViewModels** with StateFlow for state management
- **Mock data** for all screens (ready to wire to Rust core via UniFFI)

## Screens

- **Home**: Due cards count, streak, quick actions
- **Review**: SRS flashcard review with FSRS-style difficulty buttons
- **Reader**: Tap-to-parse Greek text with morphological analysis
- **Curriculum**: Athenaze chapter progress tracking
- **Dictionary**: Search LSJ dictionary entries
- **Settings**: App preferences

## Building

Requires:
- JDK 17+
- Android SDK (compileSdk 35)
- Gradle 8.11.1+ (included via wrapper)

```bash
./gradlew assembleDebug
```

## Next Steps

1. Build Rust library via UniFFI
2. Add `.so` files to `app/src/main/jniLibs/{armeabi-v7a,arm64-v8a,x86,x86_64}/`
3. Add morphology database to `app/src/main/assets/`
4. Wire ViewModels to call Rust functions
5. Enable `System.loadLibrary("uniffi_bindings")` in `GreekApplication.kt`

## Tech Stack

- Kotlin 2.0.21
- Jetpack Compose BOM 2024.12.01
- Material 3
- Navigation Compose 2.8.5
- Lifecycle ViewModel Compose 2.8.7
- Min SDK 26 (Android 8.0)
- Target SDK 35 (Android 15)
