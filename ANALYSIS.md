# Agentic Codebase Analysis — Greek Learning App

**Date:** 2026-10-16 · **HEAD:** `4eaef45` · **Method:** parallel subagent deep-dives (Rust workspace, Android app) + first-hand verification (test run, schema/data inspection, scheduling-code audit)

---

## 1. Executive summary

Phase-1 MVP of an Ancient Greek learning app: Rust core (`greek-core` + UniFFI bindings, ~2,600 LOC) driving a **fully-wired Android app** (Jetpack Compose, 6 screens), with a 109 MB morphology database (958,469 forms / 36,165 lemmas) and 1,140 flashcards (Athenaze I/II, JACT, DCC Core).

- **All 32 Rust unit tests pass**; debug APK (56 MB) builds and ships in `android/app/build/outputs/apk/debug/`.
- Contrary to the stale `android/README.md` and `PROJECT_STRUCTURE.md`, **zero mock data remains** — every screen calls the real Rust backend via UniFFI (enabled `System.loadLibrary`, arm64-v8a + x86_64 `.so`s present).
- Overall: ~60% of the CLAUDE.md vision. Solid engineering skeleton; pedagogy has two critical flaws and several missing loops (§4).

## 2. Completeness vs. CLAUDE.md vision

| CLAUDE.md promise | Status | Reality |
|---|---|---|
| Textbook-aligned SRS (Athenaze + FSRS) | ✅ Real | Official `fsrs` 2.0.4 crate; chapter-tagged cards; review logs |
| Tap-to-parse reader (Perseus morphology) | ⚠️ Partial | Real Morpheus-derived DB lookup; but corpus = **3 hardcoded texts** (`crates/core/src/reader/mod.rs:22-46`); no Perseus corpus ingestion |
| LSJ dictionary | ❌ Mislabeled | Dictionary is Morpheus lemmatizer glosses, **not LSJ**. Data-quality bug: `ἄνθρωπος` → "manfaced"; `λόγος` → "the word" |
| LLM grammar tutor (Claude API) | ❌ Absent | `crates/core/src/llm/mod.rs` = 21-line empty struct; no HTTP client in workspace |
| Reading-readiness progression (Steadman) | ❌ Absent | No graded reader path |
| Web SPA / Tauri desktop | ❌ Absent | Android only; `crates/cli` is a 3-line TODO stub |
| Audio | ❌ Absent | `crates/core/src/audio/mod.rs` stub; Settings "Auto-play audio" toggle reads into nothing |
| Offline-first | ✅ Good | SQLite + WAL; morphology DB + card JSONs bundled as APK assets |

### Engineering gaps (verified)

- No DB migrations — `CREATE TABLE IF NOT EXISTS` only, no `PRAGMA user_version` (`crates/core/src/db/mod.rs`).
- Cards live in RAM via one giant JSON string over FFI; parse failure silently yields an empty deck (`crates/uniffi-bindings/src/lib.rs:267`, `unwrap_or_default()`).
- SQL errors throughout degrade silently to empty results (`unwrap_or(0)`, `rows.flatten()`).
- Zero Android/instrumentation tests despite declared JUnit/Espresso/Compose-test deps in `app/build.gradle.kts`.
- Asset copy has no version/size check — a stale `morphology.sqlite` on device is never re-synced (`GreekApplication.kt:152-159`).
- Init-error screen has no retry path (requires app restart).
- No 32-bit ABIs (armeabi-v7a/x86 missing); Gradle wrapper is 8.14.3 (docs claim 8.11.1).
- `android/README.md` + `android/PROJECT_STRUCTURE.md` describe the scaffold commit `3c9ce4c`, not HEAD.
- Data pipeline not currently re-runnable (Morpheus XML inputs were in `/tmp/morpheus-data`, now gone); `frequency_rank` = forms-per-lemma proxy, not corpus frequency.
- Two zombie Settings toggles: "Daily reminder" (no scheduler) and "Auto-play audio" (no player) persist to DataStore, read by nothing.

## 3. What works end-to-end (verified in code)

- **Review loop:** due-first queue (20-card sessions) → new-card backfill → FSRS rating 1-4 persisted before advancing; mid-session resume survives process death (SavedStateHandle + durable DataStore mirror); no silent review drops.
- **Reader loop:** 3 texts → tokenizer (Greek-punctuation aware) → exact-form then accent-stripped morphology lookup → full AGDT 9-position postag decode → mark known / add-to-SRS (idempotent `lookup-{lemma}` cards persisted in `lookup_cards`).
- **Curriculum:** 3 textbooks; card counts + unlock flags from SQLite; intentional "Coming Soon" badge for chapters with no cards.
- **Dictionary:** FTS5 + BM25 over 36k lemmas with query sanitization, 150 ms debounce.
- **Home:** stats/streak from `review_logs`; refresh on `ON_RESUME`.

## 4. Pedagogy assessment

### Sound ✅
- Cards follow textbook chapter order; principal-parts cards appear only late (Athenaze ch.14+); DCC deck is frequency-banded (bands 1-5).
- Real FSRS (not SM-2 cosplay): full `next_states` per rating, DR=0.9, rich review logging — the data to *measure* learning is captured.
- Nouns carry their article (`ὁ ἀγρός`) — gender taught with the noun; declension metadata in tags.
- Reader gives on-demand morphology + tap-to-add-SRS: a real form-focused loop.

### Critical flaws ❌ (ranked)

1. **Intra-session relearning destroyed by day-rounding.** `add_days_to_now()` does `days.round().max(1.0)` and stamps `T00:00:00Z` (`crates/core/src/srs/mod.rs:316-321`). FSRS sub-day learning steps (~1-10 min for "Again") collapse to "tomorrow midnight UTC." Failed cards recur 15-24 h later — the most evidence-backed mechanism in spaced repetition is effectively disabled.
2. **Mastery = `reps >= 1`** (`crates/core/src/curriculum/mod.rs:80`). See a card once → Curriculum shows it mastered. FSRS `stability` (the real signal) sits unused in the same table.
3. **Recognition-only self-grading.** Flip card → self-rate Again/Hard/Good/Easy. No production (EN→GR), no typed answers, no cloze, no MCQ, no paradigm drills. Cannot distinguish recognizing `λόγος` from being able to decline it; self-rating bias inflates everything.
4. **No progression gating.** Ch.1 of every textbook unlocked by default (Athenaze-1 + JACT-1 + DCC mixed from day one); `unlock_chapter` is a free tap — a day-one user can open ch.16 principal-parts. Digestible sequencing exists in the data but is not enforced in progression.
5. **New-card firehose.** Sessions top up to 20 with *new* cards and there is no daily new-card cap (`ReviewViewModel.kt:112-178`, `SESSION_SIZE=20`).
6. **Reader path is not a progression.** 3 texts total, handwritten difficulty floats, Anabasis 1.1.1/Lysias are hard for Athenaze-ch.3 readers; no link between vocab coverage and text difficulty.

### Secondary issues
- No next-interval preview on rating buttons (learners can't calibrate).
- Due dates at midnight **UTC**, not local midnight.
- No explanation layer (LLM tutor absent): the entire corrective response to "Again" is the card back.
- Dictionary gloss artifacts + proxy `frequency_rank`; UI/README claim LSJ.

## 5. Priority fixes (input to PLAN-PARALLEL-AGENTS.md)

| # | Fix | Locus | Effort |
|---|---|---|---|
| 1 | Preserve sub-day intervals; recycle Again-cards in-session | `core/srs`, `ReviewViewModel` | S |
| 2 | Mastery from FSRS stability (≥30d), not reps≥1 | `core/curriculum` | S |
| 3 | Daily new-card cap + chapter gating (N+1 needs ≥80% mastery of N) | `core/srs`+`core/curriculum`, Android | S-M |
| 4 | Dictionary truth: fix gloss artifacts (curated overrides), relabel UI, restore LSJ claim only with real LSJ data | `tools/data-pipeline`, `data/`, strings | S-M |
| 5 | Non-self-report assessment: auto-generated MCQ/typed drills from morphology DB | new `core/questions`, Android | M-L |
| 6 | Reader corpus growth (GLTP, MIT) with coverage-aware difficulty from `known_words` | new pipeline tool, `core/reader` | L |
| 7 | Housekeeping: docs refresh, zombie toggles, init retry, migration scaffolding, Android tests, interval previews | cross-cutting | S |

Deferred beyond this plan (per CLAUDE.md phases): LLM tutor (Phase 1 remainder), Tauri desktop + web SPA (Phase 2), Steadman progression, sync.
