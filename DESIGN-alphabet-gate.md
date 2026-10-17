# Design: The Alphabet Gate — Force Script Fluency Before Vocabulary

- **Status:** design proposal (no code written)
- **Date:** 2026-10-17
- **Author:** research/design subagent (Kimi K3)
- **Scope:** Android app + Rust core at `/home/ethan/code/greek`

## TL;DR

Replace the implicit "letters first" nudge with an **explicit, measured Alphabet Gate**:
vocabulary *new-card introduction* (never due reviews) is blocked until the learner
proves alphabet recognition fluency. Proof = **Alphabet Check: 24 stratified MCQ
questions over the letters track, pass at ≥22/24 (~92%)**, available from minute one
as a placement test for anyone who already knows the script. A no-button **organic
lane** (coverage + 85% measured letters-practice accuracy + 2 distinct days) unlocks
automatically for learners who simply follow the letters track. Diligent beginner
unlocks vocabulary on **day 4–6**; an experienced classicist unlocks it in **3 minutes**.

---

## 1. Problem

Current state (verified in code):

- `data/cards/letters.json` — 43 cards, ch.1 vowels (7), ch.2 stops (9), ch.3
  continuants (9), ch.4 breathings & accents (7), ch.5 diphthongs (11). Registered
  first in `Curriculum::get_textbooks()` (`crates/core/src/curriculum/mod.rs:64-72`).
- Mastery = `memory_states.stability >= MASTERY_STABILITY_DAYS (30.0)`
  (`curriculum/mod.rs:8`). Intra-track unlocks need 80% mastered
  (`UNLOCK_THRESHOLD`, `curriculum/mod.rs:12`).
- `HomeViewModel.kt:74-83` shows a soft "start with the alphabet" banner only.
- Athenaze/JACT ch.1 and DCC ch.0 are default-unlocked
  (`curriculum/mod.rs:144`), so `ReviewViewModel.fillWithNewCards`
  (`ReviewViewModel.kt:156-190`) pours letters **and** vocab into a new user's
  first sessions.

### The tension

Gating vocab on the current 30d-stability rule means ~35 of 43 letter cards at
stability ≥30d. With FSRS-4.5 defaults (verified in vendored `fsrs-2.0.4`,
`inference.rs:27`: initial stabilities Again/Hard/Good/Easy = 0.40/1.18/3.17/15.69
days) a card needs 3–4 successful reviews across **2–4 weeks** to cross 30d; with
the 10/day new-card cap, 80% of the track lands at **roughly 4–7 calendar weeks**
of alphabet-only study. That is pedagogically wrong for a recognition-level skill
of 24 glyphs + digraphs (compare: the task brief's own estimate of ~5–15 h of
practice), and it would strangle motivation.

The fix is to define **alphabet mastery as a measured recognition-fluency
criterion achievable in days**, and to gate *only* vocabulary card introduction —
never review debt, never the reader.

## 2. Research notes

Websearch was non-functional in this environment (all queries returned empty);
claims below were verified by direct fetch where marked, or are labeled by
confidence.

- **Testing effect** — retrieval practice beats re-study for retention; quizzes are
  learning events, not just measurements. Roediger & Karpicke 2006 (*Psych.
  Science*); Dunlosky et al. 2013 rate practice testing "high utility".
  [Verified via en.wikipedia.org/wiki/Testing_effect, fetched 2026-10-17.]
  → A quiz-as-gate is itself good pedagogy; taking the check *teaches*.
- **Spacing effect** — Cepeda et al. 2006 (*Psych. Bulletin*): spaced beats massed
  in 259/271 reviewed cases; Bahrick et al. 1993: foreign vocabulary retention
  tracks inter-session spacing. [Verified via en.wikipedia.org/wiki/Spacing_effect.]
  → Spacing matters most for vocabulary (hundreds of items, years-long horizon);
  for a tiny closed set of perceptual items the cost of same-week compression is
  small, and the FSRS schedule still spaces the misses automatically.
- **Interleaving / induction** — Kornell & Bjork 2008 (*Psych. Science*, "Is
  Spacing the 'Enemy of Induction'?"): interleaved study beats blocked study for
  learning visual categories; replicated by Verkoeijen & Bouwmeester 2014.
  [Verified via the same page, refs 14–15.] → The check should sample **mixed
  across all letters chapters** (stratified), never chapter-by-chapter, and
  letters lessons should stay interleaved with exposure to real words
  (keep the Reader open during the gate).
- **Established-course sequencing** (moderate confidence; product/source pages
  not fetchable today — duolingo.fandom.com 403, Duolingo blog 404):
  - Athenaze and Reading Greek (JACT) both present the alphabet at the very
    start (Athenaze ch.1–2, JACT §1) **and begin connected reading immediately** —
    orthography is interleaved with real text, not followed by weeks of isolated
    drilling. (High confidence from textbook structure; consistent with the
    user's own textbook context noted in the wiki `_index.md`.)
  - Duolingo's Japanese/Korean courses front-load script (hiragana/hangeul
    sections) and let prior knowledge skip ahead via **placement test-out**.
    (Low-moderate confidence; cite as industry pattern only.)
- **User wiki** (`~/.wiki/topics/ancient-greek-learning/`) — **available; no
  alphabet/orthography-gating notes exist** (grep over `compiled/` found nothing
  for alphabet/orthography/letter). Aligned spirit: `compiled/references/
  pronunciation-systems.md` — "Don't let this choice delay learning."
- **Anti-gaming math** (computed): with n=24 and pass ≥22, a retake-spammer with
  true accuracy p=0.7 passes ≈1.2% of attempts; p=0.8 → ≈11%; a genuinely fluent
  p=0.9 user passes ≈56% per attempt and **≈80% within two attempts** — and every
  missed item is recycled into FSRS regardless, so even failed attempts remediate.

## 3. Definition of "alphabet mastery" (the criterion)

The gate is a **latched flag**, written on first satisfaction, never re-locked:

`user_settings` key `alphabet_gate` = `{method}` with `{method}` ∈
`passed-check | passed-organic | declared-known | vocab-history`, plus
`alphabet_gate_at` = ISO-8601 timestamp. (Table already exists,
`crates/core/src/db/mod.rs:56` — no migration needed.)

Gate is satisfied when the latch exists **or** any lane evaluates true (then
latch it):

**Lane A — Alphabet Check (expression lane / placement test):**
≥22 of 24 correct on a generated check (§4.3). Every check answer is persisted
via `submit_practice_result` (existing, `uniffi-bindings/src/lib.rs:520`) →
misses become FSRS-Again items immediately.

**Lane B — organic (no special button):**
all of
1. coverage: every of the 43 letters cards has a `memory_states` row
   (`SELECT COUNT(*) FROM memory_states WHERE card_id LIKE 'letters-%'` = letters_total);
2. measured accuracy: ≥85% correct over ≥20 practice attempts on letters
   (`SELECT COUNT(*), SUM(rating>=3) FROM review_logs WHERE source='practice'
    AND card_id LIKE 'letters-%'` — same convention as `practice_stats`,
   `srs/mod.rs:302`);
3. spacing hedge: letters activity on ≥2 distinct UTC days
   (`SELECT COUNT(DISTINCT substr(timestamp,1,10)) FROM review_logs
    WHERE card_id LIKE 'letters-%'` ≥ 2).

**Lane C — prior knowledge:**
- `declared-known`: explicit "I already know the alphabet" affordance —
  allowed, but logged (latch records method + timestamp).
- `vocab-history` (grandfather): a `memory_states` row exists for any
  non-letters card (`card_id NOT LIKE 'letters-%'`, includes `lookup-%`) —
  e.g. the owner's July vocab data. Auto-satisfied at evaluation time.

**Deliberately absent:** same-day prohibition on Lane A. A cold pass of a fresh
24-item stratified check *is* the evidence of fluency; requiring a second day
only taxes placement users. Lane B keeps the 2-day hedge because its accuracy
numbers can be accumulated by same-day cramming.

**Known limitation:** ch.4 (breathings/accents) items cannot be phrased as
name↔glyph MCQs from current card strings (see §4.3); they are covered by the
coverage floor + their natural occurrence as MCQ distractor glyphs, and re-taught
in-context by vocab card backs. Authoring 4–6 breathings questions is filed as
future work.

## 4. Mechanism design (exact touch-points)

### 4.1 Core gate evaluator — `crates/core/src/curriculum/mod.rs`

```rust
pub enum AlphabetGateMethod { CheckPassed, Organic, DeclaredKnown, VocabHistory }
pub struct AlphabetGateState {
    pub satisfied: bool,
    pub method: Option<AlphabetGateMethod>,
    pub letters_introduced: u32,   // memory_states rows for 'letters-%'
    pub letters_total: u32,        // loaded letters cards (0 => fail-open)
    pub letters_practice_total: u32,
    pub letters_practice_correct: u32,
    pub letters_active_days: u32,
    pub letters_chapters_met: u32, // chapters meeting the exposure bar (progress UI)
}
impl Curriculum {
    pub fn alphabet_gate_state(db: &Database, letters_total: u32) -> AlphabetGateState;
    pub fn record_alphabet_gate(db: &Database, method: AlphabetGateMethod) -> anyhow::Result<()>;
}
```

Evaluation order: latch → Lane C-grandfather → Lane A evidence (a recorded
`passed-check` latch) → Lane B. Fail-open (satisfied, no method) when
`letters_total == 0` (deck not loaded) so the gate can never brick a build.

### 4.2 Intra-letters unlocks use exposure, not 30d stability

In `Curriculum::get_chapters` (`curriculum/mod.rs:109-120`), the mastered-id set
is currently `stability >= 30`. For `textbook == "letters"` only, count a card as
unlock-progress when `reps >= 2` (introduced and seen again, any source — practice
included). Vocabulary textbooks keep the 30d rule unchanged. Effect: letters ch.1→2
unlocks the same day a learner works the chapter, instead of weeks later. Existing
tests (`test_letters_unlock_chain_with_mastered_chapter_1` uses reps=5 rows) stay
green.

### 4.3 Alphabet Check generator — `crates/core/src/questions/mod.rs`

```rust
pub enum AlphabetDirection { GlyphToName, NameToGlyph }
pub struct AlphabetQuestion { pub card_id: String, pub direction: AlphabetDirection,
    pub prompt: String, pub choices: [String; 4], pub correct_index: usize }
pub fn generate_alphabet_check(cards: &[Card], n: usize, seed: u64) -> Vec<AlphabetQuestion>;
```

- **Labels** (verified against today's `letters.json`): for ch.1–3, text of `back`
  before `" — "` ("Α α"→`alpha`); for ch.5, the romanization token after the dash,
  cut at `,` or `/` ("αι — ai, as in 'aisle'"→`ai`). Eligibility invariant:
  non-empty, ≤20 chars, ≤2 words, and (for ch.5) label ≠ front. Yields **35 of 43
  cards today** (ch.1: 7, ch.2: 9, ch.3: 8, ch.5: 11). Enforce with a unit test
  using `include_str!("../../../data/cards/letters.json")` so data drift fails CI.
- **Stratified sampling** across chapters (≥3 questions per represented chapter),
  **mixed** option order, alternating directions; distractors reuse the existing
  widening policy (`build_mcq`'s same-chapter → same-tag → same-pos chain,
  `questions/mod.rs:162-188`) within eligible labels for name-space choices.
- Wire-through: results feed `submit_practice_result` (Miss→Again, Hit→Good);
  the host passes a pass/fail verdict and, on pass, calls
  `record_alphabet_gate(CheckPassed)`.

### 4.4 Gate enforcement

1. **`GreekCore::get_new_cards`** (`uniffi-bindings/src/lib.rs:331`): at the top,
   `if textbook != "letters" && !gate.satisfied { return Vec::new(); }`.
   Refusal lives in the core so *every* current and future UI path is capped.
   `get_due_cards` (`lib.rs:306`) is **not** touched — review debt always flows.
2. **`GreekCore::unlock_chapter`** (`lib.rs:580`): refuse (return `false`) for
   non-letters tracks while the gate is unmet.
3. **`Curriculum::get_chapters`**: for non-letters textbooks, when gate unmet all
   chapters report `unlocked=false`; the default chapter (ch.1 / DCC ch.0) gets
   hint override: "Finish 'The Alphabet & Pronunciation' — or pass the Alphabet
   Check." Later chapters keep their existing prev-chapter hints.
4. **New UniFFI export**: `GreekCore::get_alphabet_gate_state() ->
   AlphabetGateStateView` + `record_alphabet_gate_result(passed: bool, correct:
   u32, total: u32)`; mirror records in `uniffi-bindings/src/lib.rs`.

### 4.5 Kotlin/UX touch-points

- `ReviewViewModel.fillWithNewCards` (`ReviewViewModel.kt:156`): **no code change
  required** — it already iterates curricula letters-first and tolerates empty
  returns, which is exactly what the refusing core yields.
- `PracticeViewModel` / `PracticeScreen.kt`: add a check mode (nav arg
  `practice?alphabetCheck=1` or separate route) that calls the generator with
  n=24 and renders pass/fail with recycling explanation.
- `HomeViewModel.kt:74-83`: replace the `showLettersBanner` heuristic with
  `get_alphabet_gate_state()`; while unmet, the banner becomes the primary
  progress card ("Letters 31/43 · accuracy 82% · [Take the Alphabet Check]").
- `CurriculumScreen.kt`: existing locked+hint rendering already works; add a
  deep-link from the gate hint snackbar to the check.
- `SettingsScreen.kt`: "I already know the Greek alphabet" row → confirmation
  dialog → `record_alphabet_gate_result` lane `declared-known`.

## 5. Time-to-unlock math

Diligent beginner, ~20 min/day, under the 10/day new-card cap, intra-letters
unlocks now exposure-based:

- **Day 1:** letters ch.1 (7 cards) + 3 of ch.2 introduced; chapter loop + a
  practice round ⇒ ch.1 exposure bar (reps≥2) met same day; ch.2 unlocks.
- **Days 2–5:** remaining cards at the cap (10/10/10/3) ⇒ all 43 introduced by
  day 4 evening–day 5; stratified check passable as soon as coverage exists.
- **Verdict:** check lane pass lands **day 4–6** (60–120 min cumulative practice).
  Organic lane lands day 5–6 (needs 20 letters-practice attempts, trivially met).
  Prior-knowledge user: **one 3-minute check**. Compare rejected 30d rule: 4–7
  weeks. Compare research norm for script recognition: same order of magnitude
  (hours, not weeks) — and misses stay on the FSRS schedule afterward, so
  consolidation continues *inside* vocab study with real words.

## 6. UX copy sketches

Gate card (Home, replaces banner):
> **Read Greek first.** Vocabulary unlocks when you can recognize the alphabet.
> Letters: 31/43 · Practice accuracy: 82% (goal 85%)
> [Continue letters]   [Take the Alphabet Check (~3 min)]
> Already know it? *Skip with the check — passing counts instantly.*

Locked vocab chapter hint:
> Vocabulary opens once you can read the alphabet — finish "The Alphabet &
> Pronunciation" or pass the Alphabet Check.

Check intro: "24 questions, mixed both ways (see the letter, name it — and back).
Pass with 22+. Anything you miss goes straight into your review deck."
Fail: "Not yet — 19/24 (need 22). The 5 you missed are already scheduled for
review. [Study letters] [Try again]"
Pass: "✅ 23/24 — the alphabet is yours. Vocabulary is unlocked."
Settings skip: "Skip the alphabet requirement? We'll only ask once. (Logged as
self-reported.)" → buttons [Take the 3-minute check instead] [Skip anyway].

## 7. Impacts summary

| Surface | Change |
|---|---|
| `ReviewViewModel.fillWithNewCards` | None (core refusal makes the loop a no-op for gated tracks; letters fill first as today) |
| `CurriculumScreen` | Vocab chapters show locked+gate hint; letters chapters' mastered counter now means "familiar (reps≥2)" — display copy only |
| `PracticeViewModel`/`Screen` | New 24-question check mode + verdict screen; results reuse `submitPracticeResult` |
| `HomeViewModel` banner | Swap `showLettersBanner` for `alphabetGateState`; banner becomes progress + CTA |
| `SettingsScreen` | Skip affordance (logged) |
| `crates/core` | New gate evaluator + check generator; letters-only exposure rule in `get_chapters`; refusals in `get_new_cards`/`unlock_chapter` |

## 8. Test plan

Core unit tests (`crates/core/src/curriculum/mod.rs`, `questions/mod.rs`):
1. `gate_fresh_db_unsatisfied` — empty DB → not satisfied, totals zeroed.
2. `gate_grandfathered_by_vocab_history` — one `ath1-001` memory_state ⇒ satisfied, method VocabHistory.
3. `gate_organic_lane` — 43 letters states + 20 practice rows at 90% across 2 dates ⇒ satisfied; drop to 1 date or 80% ⇒ unsatisfied.
4. `gate_latch_persists` — record CheckPassed, wipe underlying rows ⇒ still satisfied.
5. `letters_exposure_rule_vs_vocab_30d` — letters ch.1 with reps=2/stability=2d counts as met; athenaze ch.1 identical rows do not.
6. `alphabet_check_generator` — determinism per seed; both directions present; ≥3 per chapter; all prompts from eligible set; fails if `letters.json` eligibility drops below 30.
7. `check_misses_recycle` — wrong check answer writes `review_logs.source='practice'` rating 1 and an Again-due memory state.

Integration test (`crates/uniffi-bindings/tests/alphabet_gate_integration.rs`):
`new_with_cards` fixture (letters + athenaze) → `get_new_cards("athenaze",1,10)`
empty while `get_new_cards("letters",1,10)` non-empty; due vocab card still
returned by `get_due_cards`; simulate passing check (23/24 `submit_practice_result`
hits) + `record_alphabet_gate_result` ⇒ `get_new_cards("athenaze",…)` non-empty;
`unlock_chapter("letters",2)` succeeds while `unlock_chapter("athenaze",2)` refused
pre-gate.

## 9. Risks & mitigations

- **Gaming the check by retakes.** Mitigated: fresh stratified sample per attempt,
  ≥22/24 ⇒ ~1% pass at true p=0.7; misses recycle into FSRS so spamming teaches;
  no cooldown needed (attempts are the practice).
- **Frustration / rage-quit.** Mitigated: progress-visible banner, check available
  from minute one, 3-minute skip for prior knowledge, reader/dictionary stay open.
- **Streak death behind the gate.** Non-issue: `compute_streak_days`
  (`lib.rs:686`) counts all `review_logs` rows regardless of `source`, so
  letters-only and practice days keep streaks alive.
- **Grandfather false negatives** (old DB shapes). Ship the `vocab-history`
  predicate and log the chosen method; worst case the owner passes the check in
  3 minutes — a feature, not a bug.
- **Blind check burn of new-card budget.** Check misses create memory_states whose
  first review counts toward `new_cards_today` (`srs/mod.rs:277`). Accepted by
  design (recycle is the remediation); documented for the implementer rather than
  special-cased.
- **Data drift breaking the check.** The 35-eligible-cards invariant is enforced
  by a test against the real `letters.json`.

## 10. Decided against

- **30d-stability gate on letters before vocab** — 4–7 weeks of delay for a
  recognition skill; also contradicts how Athenaze/JACT interleave script with
  immediate reading.
- **Pure time-gate (e.g., "2 days of letters")** — measures presence, not fluency;
  punishes fast starters and protects nobody.
- **Accuracy check without coverage/stratification** — a uniform 24-of-43 sample
  can skip whole categories; stratified sampling + Lane-B coverage floor closes it.
- **Hard 2-day floor on all lanes** — kills the legitimate placement-test user;
  kept only on the organic lane.
- **Gating the Reader or Dictionary** — decoding real words *is* alphabet practice
  (interleaving); these stay open.
- **Typed Greek answers in the check** — keyboard setup friction at the worst
  moment; recognition MCQ matches the target skill (reading).
- **Blocking due vocab reviews** — never; SRS debt is sacred (`get_due_cards`
  untouched).

## 11. Open questions for the implementer

1. Author 4–6 ch.4 breathings/accents questions (needs new card-string format or
   a tiny hand-authored table) — future wave.
2. Whether to show the gate card on the Curriculum tab as well as Home (cheap;
   recommended).
3. Voice/audio check variant (hear /a/, pick the glyph) once audio ships.
