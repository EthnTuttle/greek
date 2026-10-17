# Execution Plan — Parallel-Agent Pedagogy & Completeness Fixes

**Date:** 2026-10-16 · **Input:** `ANALYSIS.md` §5 · **Execution model:** git worktrees + parallel subagents + single integration gate
**Status (2026-10-17, final): Waves 1–4 + Alphabet Gate (research×design×2 impl agents) COMPLETE. 76 core + 6 integration tests green; APK rebuilt. Remaining: on-device verification pass pending, then §6 epics (LLM tutor, corpus, LSJ).**

## Design rules

1. **Interface contract first.** All cross-layer signatures are fixed in §1 below. Parallel agents code against them; nobody improvises APIs.
2. **Exclusive file ownership.** Each agent owns its files outright (§2). Shared files (`Cargo.toml`s, `crates/core/src/lib.rs`, `uniffi-bindings/src/lib.rs`, generated Kotlin, `AndroidManifest.xml`) have exactly one named owner.
3. **Single bindings owner.** `crates/uniffi-bindings/src/lib.rs` and `android/.../uniffi_bindings.kt` are edited ONLY by the Wave-2 integration agent, who regenerates bindings once against merged core code. Wave-1 Rust agents expose plain `pub fn`s and never touch UniFFI macros.
4. **No git mutations without user approval.** Agents work in worktrees and produce diffs; merges/commits happen only on explicit approval.

## 1. Interface contract (binding on all agents)

```rust
// crates/core/src/srs/mod.rs  (Agent A)
pub const DEFAULT_DAILY_NEW_CAP: u32 = 10;
pub struct IntervalPreview { pub again_secs: u32, pub hard_secs: u32,
                             pub good_secs: u32, pub easy_secs: u32 }
impl SrsEngine {
    pub fn preview_intervals(&self, db: &Database, card_id: &str) -> Option<IntervalPreview>;
    pub fn new_cards_today(db: &Database) -> u32;   // first-ever reviews dated today (local day = UTC date for now)
}
// Behavior: due_date stores full ISO-8601 timestamps (no day-rounding, no midnight-stamping).
// get_due_cards signature UNCHANGED; now also returns sub-day-due learning cards.
// Legacy `T00:00:00Z` rows stay valid: lexicographic compare + 10-char parse both work.
// Rating "Again" on any card targets ~5–15 min from now; elapsed_days passed to FSRS saturates at 0 same-day (documented).

// crates/core/src/curriculum/mod.rs  (Agent B)
pub const MASTERY_STABILITY_DAYS: f32 = 30.0;
pub const UNLOCK_THRESHOLD: f32 = 0.8;
// mastered := memory_states.stability >= 30.0 (NOT reps>=1)
// ChapterInfo += pub unlock_hint: String  ("" when unlocked/available-by-default)
// Rule: chapter N (N>1, athenaze/jact) unlock-eligible when >=80% of chapter N-1's cards are mastered.
// dcc ch0 always open; unlock_chapter UNCHANGED (UI tap still allowed but records intent; hint explains requirement).

// crates/core/src/db/mod.rs  (Agent E)
// Add PRAGMA user_version + migration hook (no-op v0→v1 stub). No schema change required by A/B (format-compatible).
```

UniFFI exports the integration agent will add (Wave 2): `previewIntervals(cardId)`, `newCardsToday()`, extended `ChapterView.unlockHint`. `getNewCards` semantics change: returns `[]` when the daily cap is reached.

Android consumption contract (Agent D): after every rating, re-poll `getDueCards(1)` and append any now-due cards to the session; rating buttons show localized interval previews; Home shows "New today: X / 10"; locked Curriculum chapters show `unlock_hint` as a snackbar on tap.

## 2. Wave 1 — five parallel agents (disjoint ownership)

| Agent | Owns (exclusive) | Tasks | Acceptance |
|---|---|---|---|
| **A — SRS pacing** | `crates/core/src/srs/**` | Sub-day timestamps end-to-end; kill `days.round().max(1.0)` + midnight stamp; `preview_intervals`, `new_cards_today`, `DEFAULT_DAILY_NEW_CAP`; learning-step policy for Again (~10 min) | New tests: sub-day due filtering, cap counter, preview sanity. `cargo test -p greek-core` green; old-format rows still parse |
| **B — Mastery & gating** | `crates/core/src/curriculum/**` | Stability-based `mastered_count`; `unlock_hint` on ChapterInfo; 80%-of-prior-chapter rule | Tests: mastery threshold, per-textbook gating, hint strings. Green |
| **C — Lexicon data** | `tools/data-pipeline/**`, `data/**`, `android/app/src/main/assets/**` | Curated gloss-override table (`data-overrides/lemma_glosses.tsv`, start: top 500 lemmas incl. ἄνθρωπος, λόγος); rebuild `morphology.sqlite` with overrides; document Morpheus-vs-LSJ in a new `data/README.md`; relabel pipeline notes | Sampled glosses read correctly; DB row counts unchanged (36,165/958,469); assets byte-synced |
| **D — Android UX** | `android/app/src/main/java/com/greekapp/**` (except Manifest), `res/**` | In-session requeue loop per contract; interval previews on rating buttons; "New today" on Home; unlock-hint snackbar; Dictionary label "Morpheus glosses (LSJ pending)"; extracts a `GreekCoreApi` interface seam for testability | `./gradlew :app:assembleDebug` succeeds; no new lint errors; builds against a HAND-WRITTEN stub of the new UniFFI API (bindings not regenerated until Wave 2) |
| **E — Docs, DB harness, settings zombies** | `CLAUDE.md`, `ANALYSIS.md`, all `**/README.md`, `android/PROJECT_STRUCTURE.md`, `crates/core/src/db/**`, `AndroidManifest.xml`, `android/app/build.gradle.kts` | Rewrite stale docs to match HEAD; `PRAGMA user_version` scaffold; implement Daily-reminder via WorkManager (or remove toggle); remove/hide Auto-play-audio toggle; init-error retry button | Docs contain no false claims; reminder schedules/cancels from Settings; gradle build green |

Wave-1 do-not-touch list: `Cargo.toml` (any), `crates/core/src/lib.rs` (except A/B/E adding their own pre-agreed nothing — module set is unchanged), `crates/uniffi-bindings/**`, generated `uniffi_bindings.kt`, `GreekApplication.kt` (shared → owned by integration agent in Wave 2 to merge D's seam + E's reminder).

## 3. Wave 2 — one integration agent (sequential gate)

Runs in the main checkout after all Wave-1 diffs are approved and merged (order: E → A → B → C → D; textual conflicts expected to be near-zero by ownership design).

1. Merge/rebase all worktree branches; fix fallout.
2. Sole owner: `crates/uniffi-bindings/src/lib.rs`, regenerate `uniffi_bindings.kt` + `.so`s (arm64-v8a, x86_64).
3. Add contract exports: `previewIntervals`, `newCardsToday`, `ChapterView.unlockHint`; enforce daily cap inside exported `getNewCards`.
4. Swap D's hand-written stub for real bindings; reconcile `GreekApplication.kt` (D's interface seam + E's WorkManager wiring).
5. Full gate: `cargo test --workspace`, `./gradlew assembleDebug`, APK smoke checklist (§5).

## 4. Wave 3 — assessment mode (two parallel agents; start after Wave 2)

| Agent | Owns | Tasks |
|---|---|---|
| **G — Question engine** | new `crates/core/src/questions/**` + bindings additions | Auto-generate MCQ: stem = card front; 3 distractors sampled from same POS + same chapter/frequency-band (morphology DB available for paradigm variants); typed-answer mode (EN→GR with accent-tolerant matcher reusing `normalize_greek`); expose via UniFFI |
| **H — Practice UI** | `android/**` screens/viewmodels | Practice tab/entry point, MCQ + typed flows, results persisted through `submit_review` mapping (correct→Good etc.), session summary with measured accuracy (first real non-self-report metric on Home) |

## 5. Validation gate (integration agent + user)

- All Rust tests green incl. new suites; first Android JVM unit tests run in CI-local.
- Fresh-install smoke: day-1 session ≤10 new cards; rate a card Again → it reappears same session (~10 min, simulated clock OK); Curriculum shows 0% mastered; locked ch.2 explains requirement; Home counters update.
- Dictionary spot-check: ἄνθρωπος, λόγος, χώρα, φιλέω read sensibly.
- APK diff vs. previous build reviewed (size delta expected ~0 from Rust-only changes; bigger only if C's DB changed materially).

## 6. Explicit non-goals this cycle

LLM tutor; reader corpus expansion (GLTP) + coverage-aware difficulty (next epic); real LSJ ingestion (C delivers a feasibility note, stretch only); web/desktop shells; 32-bit ABIs; sync/social.

## 7. Operating mechanics

- Worktrees: `wt/srs-pacing`, `wt/mastery-gating`, `wt/lexicon-data`, `wt/android-ux`, `wt/docs-harness` via `tools.opencode.worktree_create`.
- Each agent's brief = its §2 row + §1 contract + "run tests, produce diff, no commits without approval."
- Failure protocol: any agent that cannot satisfy the contract writes a blocking note into its worktree root (`BLOCKED.md`) instead of improvising.
