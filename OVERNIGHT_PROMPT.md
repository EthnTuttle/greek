# Overnight Build: Greek Learning App → Android APK

## Your Role

You are an **orchestrator**. You do NOT write code directly except for small glue/fixes. Instead, you:

1. Read the plan and break it into discrete tasks
2. Spawn subagents (via the Agent tool) to execute tasks in parallel where possible
3. Review their output (check files exist, run tests, verify builds)
4. Fix integration issues between modules
5. Spawn more agents for the next phase
6. Repeat until the APK builds

## Context

- Full implementation plan: `~/.wiki/topics/ancient-greek-learning/output/plan-greek-app-2026-07-07.md` — READ THIS FIRST
- Project CLAUDE.md: `/home/ethan/code/greek/CLAUDE.md`
- Working directory: `/home/ethan/code/greek`
- Dev environment: `nix develop /home/ethan/code/greek` provides Rust (with Android targets), cargo-ndk, JDK 17. Android SDK/NDK is at `~/Android/Sdk` (NDK 27, build-tools 35/36, platform android-35).
- Morphology data: clone `https://github.com/gcelano/MorpheusGreekUnicode.git` to `/tmp/morpheus-data` if not already present
- Lexicon data: NOT NEEDED for MVP. The MorpheusGreekUnicode XML already contains English glosses per entry. Skip LSJ for now.

## Orchestration Plan

Execute these phases sequentially. Within each phase, spawn agents in parallel.

### Phase A: Scaffold + Data Pipeline (parallel agents)

**Agent 1 — Workspace scaffold:**
- Create Cargo workspace at `/home/ethan/code/greek`
- Crate structure: `crates/core`, `crates/uniffi-bindings`, `crates/cli`
- Set up `crates/core/src/` with module files (lib.rs, srs/, morphology/, reader/, curriculum/, search/, llm/, audio/, db/)
- Add dependencies to Cargo.toml: fsrs, rusqlite (with bundled + FTS5 features), serde, serde_json, uuid, quick-xml, tokio, uniffi
- Ensure `cargo check` passes (stub all modules with TODO)

**Agent 2 — Data pipeline (morphology):**
- Write `tools/data-pipeline/` binary crate that parses MorpheusGreekUnicode XML from `/tmp/morpheus-data`
- Parse the XML: extract form, lemma, postag, and English gloss for each entry (the XML includes `<mean>` elements with definitions)
- Output: a SQLite database at `data/morphology.sqlite` with tables:
  - `forms(form TEXT, form_normalized TEXT, lemma_id INTEGER, postag TEXT)`
  - `lemmas(id INTEGER PRIMARY KEY, headword TEXT, pos TEXT, definition TEXT, frequency_rank INTEGER)`
  - FTS5 virtual table over lemmas (headword + definition)
- If full 958K entries take too long, cap at 150K entries and log a warning
- Add a normalized form column (strip diacritics/accents for fuzzy matching)
- LSJ lexicon is NOT available — derive definitions from the `<mean>` elements in MorpheusGreekUnicode XML only

**Agent 3 — Vocabulary card data:**
- Create `data/cards/` directory with JSON files: `athenaze-i.json`, `athenaze-ii.json`, `jact.json`, `dcc-core.json`
- Each file: array of card objects `{ id, front, back, chapter, card_type, pos, tags[] }`
- Athenaze I: chapters 1-16, at least 15-20 cards per chapter (use real Athenaze vocabulary — common beginning Greek words organized by typical textbook progression)
- Athenaze II: chapters 17-25, at least 10 cards per chapter
- JACT: chapters 1-10, at least 10 cards per chapter
- DCC Core: top 500 most frequent lemmas, no chapter organization (tags: "frequency-band-1" through "frequency-band-5")
- Card types: Vocab (Greek→English), ReverseVocab (English→Greek), PrincipalParts (for verbs)

### Phase B: Core Library Implementation (parallel agents)

After Phase A agents complete and `cargo check` passes:

**Agent 4 — SRS engine (`crates/core/src/srs/`):**
- Integrate `fsrs` crate for scheduling
- `SrsEngine` struct: init with default FSRS params, schedule reviews, compute next interval
- `SessionManager`: get due cards (ordered by overdue days), get new cards (by chapter order), submit review, update memory state
- Card storage: load from JSON card files, store memory state in SQLite
- Write tests: new card scheduling, review updates stability, due date computation

**Agent 5 — Morphology + Search (`crates/core/src/morphology/` and `crates/core/src/search/`):**
- `MorphologyDb`: open SQLite, lookup by surface form (exact match first, then normalized), return `Vec<Analysis>`
- `Analysis` struct: lemma, headword, postag, definition, confidence score
- Parse the 9-position postag into human-readable labels (e.g., "n-s---mg-" → "noun, singular, masculine, genitive")
- `SearchEngine`: FTS5 query over lemmas table, return ranked results
- Write tests: lookup known forms, search by English definition

**Agent 6 — Curriculum + Reader (`crates/core/src/curriculum/` and `crates/core/src/reader/`):**
- `Curriculum` struct: textbook enum, chapters with metadata (title, vocab count), unlock state, mastery % (computed from SRS memory states for that chapter's cards)
- `Reader`: load tokenized text sections, compute difficulty score (% of tokens whose lemma is in user's "known" set — stability > 30 days), provide `TextSection` and `Token` data
- Sample texts: include 2-3 short Greek passages (Athenaze Ch.1 style + one Xenophon excerpt) as embedded test data
- Write tests: chapter progression, difficulty computation

**Agent 7 — Database layer (`crates/core/src/db/`):**
- Schema: tables for memory_states, review_logs, curriculum_progress, user_settings
- Migrations (rusqlite migration pattern)
- CRUD operations for all entities
- Init function: create tables, load card data from JSON, attach morphology.sqlite
- Write tests: round-trip CRUD

### Phase C: UniFFI + Android (parallel agents)

After Phase B agents complete and `cargo test` passes:

**Agent 8 — UniFFI bindings (`crates/uniffi-bindings/`):**
- Define UDL or proc-macro exports for all operations listed in the plan
- Expose: init_database, get_due_cards, get_new_cards, submit_review, lookup_word, search_dictionary, get_curricula, get_chapters, get_text_sections
- Data types: CardView, AnalysisView, LemmaView, CurriculumView, ChapterView, SectionView, TokenView
- Verify: `cargo build -p uniffi-bindings` succeeds
- Generate Kotlin bindings: `uniffi-bindgen generate --library ... --language kotlin --out-dir android/app/src/main/java/com/greekapp/core/`

**Agent 9 — Android app (Jetpack Compose):**
- Create `android/` Gradle project: settings.gradle.kts, build.gradle.kts, app module
- Dependencies: Compose BOM (latest stable), Material 3, Navigation Compose, Lifecycle ViewModel
- Screens (Jetpack Compose):
  - `HomeScreen` — daily review count badge, "Start Review" button, "Read" button, streak counter
  - `ReviewScreen` — card front (large Greek text), tap-to-reveal back, rating buttons (Again/Hard/Good/Easy), progress bar
  - `ReaderScreen` — Greek text display (selectable), tap word → BottomSheet with Analysis (lemma, POS, full parsing, definition, "Add to SRS" button)
  - `CurriculumScreen` — LazyColumn of textbooks, expandable chapters with progress bars
  - `DictionaryScreen` — SearchBar + results LazyColumn
  - `SettingsScreen` — placeholder switches
- Navigation: BottomNavigation bar (Home, Read, Dictionary, Progress, Settings)
- ViewModel layer: calls into Rust via generated Kotlin bindings
- Theme: Material 3, support dark mode, Greek-appropriate typography (serif for Greek text)
- `System.loadLibrary("uniffi_bindings")` in Application class
- Gradle config: compileSdk 35, minSdk 26, Kotlin 2.0+, compose compiler plugin

### Phase D: Integration + Build (orchestrator does this directly)

After Phases A-C complete:

1. Build the Rust library for Android: `nix develop /home/ethan/code/greek --command bash -c 'cd /home/ethan/code/greek && cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build --release -p uniffi-bindings'`
2. Copy generated Kotlin bindings to the Android project
3. Copy `data/morphology.sqlite` and card JSON files to `android/app/src/main/assets/`
4. Fix any compilation errors (Kotlin bindings mismatches, missing imports, Gradle config)
5. Build APK: `cd android && ./gradlew assembleDebug`
6. If build fails, debug and fix. Spawn a focused agent if the fix is complex.
7. Verify APK exists at `android/app/build/outputs/apk/debug/app-debug.apk`

### Phase E: Report + Commit

1. Run `cargo test` and note results
2. Git init (if needed), add all files, commit with descriptive message
3. Write `SESSION_REPORT.md` at project root:
   - Features working (with screenshots/descriptions of each screen)
   - What was stubbed (LLM, audio)
   - APK location + install instructions
   - Known issues
   - Recommended next session priorities

## Rules

- Do NOT ask the user questions. Make decisions and document them.
- ALWAYS prefix shell commands with `nix develop /home/ethan/code/greek --command bash -c '...'` to ensure Rust toolchain and env vars are available.
- When spawning agents, give them COMPLETE context — they cannot see this conversation. Include file paths, expected interfaces, and the architectural decisions from the plan.
- Run agents in PARALLEL within a phase. Wait for all agents in a phase to complete before starting the next phase.
- After each phase, VERIFY: check that files exist, run `cargo check` or `cargo test`, confirm no obvious errors before proceeding.
- If an agent's output has issues, spawn a focused fix-it agent rather than rewriting everything yourself.
- Prefer working code over perfect code. The goal is a testable APK.
- If morphology XML parsing is slow, subset to 150K entries.
- Target arm64-v8a and x86_64 only (real devices + emulator).
- Git commit after each successful phase.
