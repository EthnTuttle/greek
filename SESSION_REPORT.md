# Session Report: Greek Learning App — Overnight Build

**Date**: 2026-07-08
**Duration**: ~2 hours
**Result**: Working debug APK built successfully

## What Was Built

### Rust Core Library (`crates/core/`)
- **SRS Engine**: FSRS-6 integration via `fsrs` crate, card scheduling, review submission, due date computation
- **Morphology**: SQLite lookup for 958K Greek forms → lemma + postag + definition, with normalized fallback
- **Search**: FTS5 full-text search over 36K lemmas (headword + definition)
- **Curriculum**: Textbook/chapter progression tracking (Athenaze, JACT), unlock logic
- **Reader**: Text tokenizer, sample passages (Athenaze Ch.1, Xenophon, Lysias), difficulty scoring
- **Database**: Schema with migrations, CRUD operations, WAL mode

All 29 unit tests pass.

### Data Pipeline (`tools/data-pipeline/`)
- Parses all 3 MorpheusGreekUnicode XML files (958K entries)
- Deduplicates forms, extracts lemmas with definitions
- Outputs `data/morphology.sqlite` (109 MB): forms table, lemmas table, FTS5 index

### Vocabulary Card Data (`data/cards/`)
- `athenaze-i.json`: Chapters 1-16, ~200 cards
- `athenaze-ii.json`: Chapters 17-25, ~40 cards
- `jact.json`: Chapters 1-10, ~45 cards
- `dcc-core.json`: 500 frequency-ranked vocab + 50 principal parts cards

### UniFFI Bindings (`crates/uniffi-bindings/`)
- Thread-safe wrapper (Mutex) around core library
- Exposes: SRS operations, morphology lookup, dictionary search, curriculum, reader
- Generated 2331-line Kotlin binding file via `uniffi-bindgen`
- Data types: CardView, AnalysisView, SearchResultView, CurriculumView, ChapterView, TextSectionView, TokenView, ReviewStats

### Android App (`android/`)
- **6 Compose screens** with working mock UI:
  - Home: due cards counter, streak, start review button
  - Review: flashcard flip with Again/Hard/Good/Easy buttons
  - Reader: tappable Greek text with morphology bottom sheet
  - Curriculum: expandable chapter list with progress bars
  - Dictionary: search bar with filtered results
  - Settings: toggle switches
- Material 3 theme with light/dark mode
- Bottom navigation bar
- ViewModels with realistic mock data
- JNA dependency for UniFFI interop

## APK Details

- **Location**: `android/app/build/outputs/apk/debug/app-debug.apk`
- **Size**: 56 MB (mostly the 109 MB morphology.sqlite compressed)
- **Install**: `adb install android/app/build/outputs/apk/debug/app-debug.apk`
- **Min SDK**: 26 (Android 8.0)
- **Architectures**: arm64-v8a, x86_64
- **Native library**: libuniffi_bindings.so (3.6-4.0 MB per arch)

## What's Stubbed

- **LLM Tutor**: Empty struct, no API calls (Phase 2)
- **Audio Player**: Empty struct (Phase 5)
- **ViewModels → Rust integration**: Currently mock data; the UniFFI bindings are generated and the .so is bundled, but the ViewModels don't call into Rust yet (need to wire `GreekCore.newWithCards()` in GreekApplication.kt)
- **Pronunciation**: No TTS integration
- **Sync**: Local-only, no cloud sync

## Build Instructions

### Prerequisites
- Nix (for Rust toolchain with Android targets): `nix develop /home/ethan/code/greek`
- OR: Rust with Android targets + cargo-ndk + Android SDK/NDK 27

### Build Steps
```bash
# 1. Build morphology database (first time only, ~5 min)
cargo run -p data-pipeline --release

# 2. Build Rust library for Android
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/27.0.12077973"
cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build --release -p uniffi-bindings

# 3. Generate Kotlin bindings (after building host library)
cargo build --release -p uniffi-bindings
./target/release/uniffi-bindgen generate --library target/release/libuniffi_bindings.so --language kotlin --out-dir android/app/src/main/java/

# 4. Copy assets
cp data/morphology.sqlite android/app/src/main/assets/
cp data/cards/*.json android/app/src/main/assets/cards/

# 5. Build APK
cd android
JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64 ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleDebug
```

### Run Tests
```bash
cargo test
```

## Known Issues

1. **APK size**: 56 MB due to full 109 MB morphology database. Could subset to top 50K forms for 95% text coverage.
2. **ViewModels use mock data**: The Rust .so is bundled but not wired to the UI yet.
3. **Kotlin warnings**: Deprecated APIs (Divider → HorizontalDivider, ClickableText, SearchBar overload) — cosmetic only.
4. **Unreachable pattern warnings**: In reader tokenizer's Unicode punctuation matching — harmless.
5. **No proguard/R8**: Debug build only; release would need proper minification rules for JNA.

## Recommended Next Session Priorities

1. **Wire ViewModels to Rust**: Replace mock data with actual `GreekCore` calls in each ViewModel
2. **Subset morphology DB**: Create a smaller DB (~20MB) with only high-frequency forms for faster APK download
3. **Fix deprecated Compose APIs**: Update Divider, SearchBar, ClickableText usages
4. **Add onboarding flow**: First-launch textbook selection, daily goal setting
5. **LLM integration**: Add Claude API provider in `crates/core/src/llm/`
6. **Release build**: Configure proguard rules, signing config, app icons
