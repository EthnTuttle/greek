# WS-core fix plan — learning-core release blockers (C1–C6 + coupled P2s)

Triage of `production-readiness-2026-10-20.md` "Learning-core correctness" against `0b766e7` sources. All six P1s reproduced **as documented**; verification notes inline (stale bits flagged).

**Stale-finding flags from verification:**
- C1: line window 225-265 → actual `crates/core/src/srs/mod.rs:226-265`. Cosmetic drift only; finding stands.
- C9 (coupled P2): **partially stale.** `iso_to_secs` time-part slicing is already hardened (`srs/mod.rs:436-439`, `get()` + comment). The **date part is not**: `iso_to_days` still raw byte-slices `iso[0..4]/[5..7]/[8..10]` (`srs/mod.rs:422-424`), so a multibyte char inside the first 10 bytes (e.g. the S2 probe's `202α-…`) still panics mid-char-boundary. Also `ymd_to_days` (`:452-459`) can underflow-panic on year 0 with month ≤ 2. C9 remains valid, scoped to `iso_to_days`/`ymd_to_days` and to the sync fold path (`sync/mod.rs:336` → `days_between` → `iso_to_days`).
- Everything else matches cited lines exactly.

---

## C1 — non-atomic review writes
- **Root cause:** `srs/mod.rs:226-245` upserts `memory_states`, `:251-265` inserts `review_logs` (plus `sync::device_id`/`hlc_now` reads/writes inline at :262-263) as separate autocommit statements. `crates/core/src/db/mod.rs:100` exposes only `&Connection`; no tx helper exists.
- **Fix:** add a `Database::with_tx(..)` (or `Transaction<'_>` borrow) wrapper; do upsert + log insert + HLC advance inside one `BEGIN IMMEDIATE`/`COMMIT`, auto-rollback on error.
- **Files:** `crates/core/src/db/mod.rs`, `crates/core/src/srs/mod.rs`.
- **Regression test:** trigger rejecting the `review_logs` insert; assert `submit_review` errors AND leaves `memory_states`, `review_logs`, and HLC setting untouched; retry succeeds exactly once.
- **Effort:** S-M. **Risk:** low (same statement text, new scope).
- **Deps:** blocks safe Android retry for A2; the tx wrapper is shared by S2/S5 (import/fold/migration atomicity) — land it first.

## C8 (coupled P2) — state read failures reset history
- **Root cause:** `srs/mod.rs:330-348`: `load_memory_state` ends in `.ok()`, collapsing genuine SQL/conversion errors into "never reviewed."
- **Fix:** return `Result<Option<MemoryStateRow>>`; propagate errors from `submit_review_with_source` and `preview_intervals` (currently `Option`; make it return `Result` internally, bindings map to `None`/typed error).
- **Files:** `crates/core/src/srs/mod.rs`, `crates/uniffi-bindings/src/lib.rs` (signature fallout).
- **Regression test:** row with non-numeric `difficulty`, then submit a review → must return Err and leave the row (reps 8/lapses 2) untouched.
- **Effort:** S. **Risk:** low. **Deps:** none; same function C1/C2 touch — bundle with C1 to avoid thrash.

## C9 (coupled P2, partial) — malformed timestamps panic
- **Root cause:** `srs/mod.rs:417-428` (`iso_to_days` unchecked byte slicing), `ymd_to_days` underflow at `:453`; reachable from incremental paths (`days_between` :410) and the S2 fold (`sync/mod.rs:336`).
- **Fix:** replace slicing with a validated parser (`parse_iso_ymd(&str) -> Option<(u64,u64,u64)>` checking length, ASCII digits, separators, ranges; reject year 0). Reuse it at sync ingress (S2's validation) so parser is written once.
- **Files:** `crates/core/src/srs/mod.rs` (parser), `crates/core/src/sync/mod.rs` (ingress validation is S2 proper — coordinate).
- **Regression test:** `days_between("202α-10-09T12:00:00Z", now)` and `submit_review`/fold inputs with malformed stored dates return error/degrade, never panic; run through `catch_unwind` to assert.
- **Effort:** S. **Risk:** low. **Deps:** prerequisite for S2 hardening; independent of C1.

## C2 — lookup cards invalid initial FSRS state
- **Root cause:** `uniffi-bindings/src/lib.rs:817-824` inserts `memory_states` with stability/difficulty 0.0 and `reps 0`; `srs/mod.rs:185-193` then feeds `Some(MemoryState{0,0})` into FSRS instead of `None` (new). Fold in `sync/mod.rs:326+` replays from `None`, hence the 0.01-vs-3.173 divergence.
- **Fix:** delete the zero-state insert; an unreviewed lookup card has **no** `memory_states` row until its first review (due-ness for lookup cards then comes from the reader/deck side, not due_date). `get_new_cards`' state-set scan (`:419-433`) unaffected (lookup ids are not in the deck).
- **Files:** `crates/uniffi-bindings/src/lib.rs` only (plus migration concern: existing 0/0 rows in users' DBs — fold or `rebuild_memory_states` won't clear rows lacking logs; add a one-time cleanup deleting `memory_states` rows with reps=0, no review_logs, and no last_review, or leave harmless).
- **Regression test:** add lookup card → `preview_intervals` equals a fresh new-card preview; after one Good review, compare `memory_states` against `rebuild_memory_states` output (incremental≡replay).
- **Effort:** S. **Risk:** low-moderate (existing-install cleanup). **Deps:** none; S4's fold-consistency work consumes the new test.

## C6 — first-failure relearning ~9.7h, not same-session
- **Root cause:** `srs/mod.rs:190-209` uses FSRS `chosen.interval` unmodified (new-card Again = 0.4028d ≈ 9h40m); `add_days_to_now` (:469-476) correctly preserves sub-day — the policy, not the clock math, is missing. Docs promise same-session recycling (`docs/guide.html:109-114`).
- **Fix:** explicit short-step policy layered on FSRS: when the card is in learning/relearning (few reps or post-lapse), override due date with a fixed step ladder (e.g., Again→1min, Hard→5min, Good→10min first time) while still storing FSRS stability/difficulty from the chosen rating. Steps as constants in core. **Must mirror identically in `rebuild_memory_states`** (`sync/mod.rs:333-359`) or S4's divergence returns. Deterministic steps make mirroring trivial.
- **Files:** `crates/core/src/srs/mod.rs`, `crates/core/src/sync/mod.rs` (fold mirror).
- **Regression test:** with injectable clock: new card → Again → due in ~1min and appears in `get_due_cards`; second step advances; replay fold after simulated sync produces the identical due_date. Update `test_submit_review_again_schedules_subday_due` bound from 86400 to the step max.
- **Effort:** M. **Risk:** moderate (scheduling behavior change; ecosystem/UI labels read the same `IntervalPreview`, so previews must show the step too). **Deps:** injectable clock (shared infra); coordinate with S4.

## C5 + C4 — practice samples same prefix / bypasses alphabet gate (same function, fix together)
- **Root cause (C5):** `questions/mod.rs:74-87` only shuffles caller-ordered windows of `limit`; `uniffi-bindings:546-559` passes the reviewed tier in deck order, so the first `limit` reviewed cards are always the pool.
- **Root cause (C4):** `generate_practice_mcq` (`uniffi-bindings:531-566`) applies no gate, chapter, or new-card-allowance filter, and `submit_practice_result` (:604-615) then creates memory states that satisfy the vocab-history grandfather (`curriculum/mod.rs:293-300`), flipping the gate.
- **Fix:**
  - C5: shuffle each **full priority tier** once with the seeded RNG, then take `limit` (change `generate_mcq_set` to shuffle the whole eligible slice before windowing, or shuffle tier in binding — core-side is cleaner: keep window semantics but shuffle `eligible` across the entire slice, not windows).
  - C4: add one shared eligibility policy in core (e.g., `curriculum::practice_eligible(db, cards, card) -> bool`: `letters-*` always fine; vocabulary cards only when gate satisfied — `alphabet_gate_state().satisfied`; unseen vocab cards additionally respect `new_cards_remaining_today` when practice would create states). Apply in `generate_practice_mcq`. Preserve the placement-test path (C10's generator at `questions/mod.rs:383-405`) by keeping a separate explicit API (do not route placement through the practice gate).
- **Files:** `crates/core/src/questions/mod.rs`, `crates/core/src/curriculum/mod.rs` (policy helper), `crates/uniffi-bindings/src/lib.rs` (`generate_practice_mcq`, `submit_practice_result` unchanged).
- **Regression tests:** seeds {1,7,42,1337} on the real deck must produce differing, non-prefix question sets covering beyond `ath1-001..010`; fresh core practice returns letters-only (or empty), and a practice answer must NOT flip `alphabet_gate_state().satisfied`.
- **Effort:** M (two tests need the real deck fixture). **Risk:** moderate (practice behavior change; Android route unchanged). **Deps:** gate policy shares `alphabet_gate_state` latch with C12; new-cap budget from C3's refactor should be reused rather than duplicated.

## C3 — new-card cap admits 20/day
- **Root cause:** `uniffi-bindings:405-409` computes `new_cards_remaining_today` independently per `get_new_cards(textbook, chapter, ..)` call; Android loops chapters (`ReviewViewModel.kt:166-186`), each call seeing the same unspent budget. No admission reservation.
- **Fix:** add a single session-builder API in bindings/core, e.g. `build_review_session(limit) -> SessionPlan` that internally walks due cards then new cards across chapters in textbook order with one budget passed down — remove per-chapter capping responsibility from callers. If multi-call composition must be retained, add a lightweight per-day reservation row updated inside C1's transaction on submit; the session-builder approach is smaller and race-safe for a single-threaded UI.
- **Files:** `crates/uniffi-bindings/src/lib.rs` (new API), `crates/core/src/srs/mod.rs` (budget plumbing); Android caller update (A-side coordination), desktop `commands.rs:75-101` (D6) consumes same API.
- **Regression test:** built deck, gates satisfied: session build across Letters+Athenaze+JACT yields ≤10 new cards; submitting the queue leaves `new_cards_today() <= 10`.
- **Effort:** M. **Risk:** moderate (caller signatures change; Android + desktop call sites). **Deps:** C1's submit path (counter moves on successful submit only); D6; Android session code (A2-adjacent but not blocked by it).

## C12 (coupled P2) — organic alphabet completion can relock
- **Root cause:** organic lane (`curriculum/mod.rs:314-331`) returns satisfied without persisting via `record_alphabet_gate` (:337+); only check-passed/declared paths latch (`uniffi-bindings:707-725`).
- **Fix:** when organic recomputation yields satisfied and no latch row exists, write the latch (`AlphabetGateMethod::Organic`) before returning — inside the same function so every reader latches. No binding change needed.
- **Files:** `crates/core/src/curriculum/mod.rs`.
- **Regression test:** reach organic thresholds → latch row exists; degrade letters accuracy below threshold → gate still satisfied with `Organic`.
- **Effort:** S. **Risk:** low. **Deps:** same gate state/type used by C4 policy; order after/alongside C4.

---

## Shared infrastructure
1. **Transaction wrapper** on `Database` (C1; reused by S2/S5 import/fold/migration atomicity). One place, ~30 lines.
2. **Validated ISO parser** (C9; reused at S2 ingress validation).
3. **Injectable clock**: `chrono_now()` (`srs/mod.rs:362`) is a free function reading `SystemTime`. Introduce a `Clock` trait (default `SystemClock`) threaded into `SrsEngine` and fold so C6's step policy and C1's fault-injection tests are deterministic without wall-clock sleeps. Also stabilizes C3's day-boundary tests.
4. **Eligibility/gate policy helper** in `curriculum` (C4; reused by C3's session builder and C12's latch). Don't grow three copies of "gate satisfied + alphabet lanes."

## Recommended fix order
1. **C9 parser** — tiny, pure, no callers' signatures change; unblocks S2 immediately.
2. **Tx wrapper + C1 (+ C8 bundled)** — one file, one review; creates the wrapper S2/S5 need before sync work starts.
3. **C2** — independent of scheduling policy; small; its incremental≡replay test becomes the template C6 reuses.
4. **Injectable clock + C6** — after C1 so `submit_review` isn't re-touched; fold mirror lands with S4 coordination.
5. **C3 session builder** — API addition; parallel-safe with C4/C5 once step 4 settles, since different files (bindings vs core/questions).
6. **C4+C5 together** — same function; reuses C12's latch path, so pair with step 7.
7. **C12** — latch write; trivial once `record_alphabet_gate` semantics confirmed by C4.

**Parallelism:** two lanes after step 2 — Lane A: C2 → C6 (scheduling/fold); Lane B: C3 → C4/C5 → C12 (eligibility/practice). Lanes touch disjoint modules except `curriculum/mod.rs` (C4+C12) and the shared infra list, which should land in steps 1–2 to avoid merge churn.
