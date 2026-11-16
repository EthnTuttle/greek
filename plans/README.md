# Production-readiness fix program — master plan

**STATUS (2026-10-27): all four waves EXECUTED and green.** 22/22 audit P1s +
coupled P2s fixed with regression tests; workspace 228+ tests passing,
desktop contract 15/15, UI mock 91/91, Android unit suites green, local
verification runner added (R4), release drills in
`plans/release-gate-checklist.md`. Remaining work is
physical-device gates (release-gate-checklist), the grooming list in
`plans/followups.md`, and committing/tagging. Historical planning content
follows.

---

Synthesized 2026-10-21 from the five workstream plans in this directory, triaged against
`production-readiness-2026-10-20.md` (rev `0b766e7`). **26/26 verified findings held;
one narrowed (C9 partially fixed upstream already); zero stale.** Workstream plans own the
per-item detail; this file owns sequencing.

## Hard dependencies that shape the schedule

```
C9 ISO parser ──► S2 ingress validation ──► S1 (v2 wire format) ──► D3 regression
tx wrapper (C1) ──► S5 atomicity ──► S1, S6, S7
C1 ──► safe A2 retry path
S11 SDK upgrade ──► S10, S6, S8, S9  (hard sync point: no transport fixes before it)
R1 pinned artifact ──► D2 desktop dictionary ──► R4 CI packaging gate
injectable clock ──► C6 (fold mirror in S4's file)
C12 latch ──► C4 gate policy; C3 session-builder API consumed by Android + D6
```

## Waves (items in a wave are parallel-safe)

### Wave 0 — quick wins, no risk, do immediately (~1 day)
| Item | What | Owner plan |
|---|---|---|
| A1 | Keyboard `put()` recursion — delete shadowed helper | ws-android |
| A3 | Carry textbook through chapter taps | ws-android |
| A5/R3 | `abiFilters` {arm64-v8a, x86_64} + APK lib inspection check | ws-release (closes A5) |
| C9 | Validated ISO date parser (kills S2 panic surface) | ws-core |
| S3 | Settings ingress allowlist | ws-sync |
| D9 | Refresh+commit desktop lockfile | ws-desktop |
| R1 | Pin morphology artifact (MANIFEST hash + release asset) | ws-release |

### Wave 1 — foundations (~3-5 days)
| Item | What | Blocks |
|---|---|---|
| C1+C8 | `Database::with_tx`, atomic review submit, distinct read-failure errors | S2, S5, A2-retry |
| S2 | RFC3339 ingress validation (uses C9 parser) | S1, import safety |
| S5 | TX-envelope import/fold/migrations | S1, S6, S7 |
| Injectable clock | `Clock` trait into SrsEngine + fold | C6 |
| A2+session-P2 | Honor `submitReview` bool; fix SavedStateHandle completion | — (retry-safe once C1 lands) |
| A4+selection-P2 | Panel commits propagate to screen state | — |
| C6 | Short learning-step policy, mirrored in fold (coordinate w/ S4 touchpoints) | — |
| C2 | No zero-FSRS-row for unreviewed lookups (+cleanup of existing rows) | S1 lookup semantics |

### Wave 2 — data integrity (~3-5 days)
| Item | What |
|---|---|
| S4 | Receive-side `hlc_observe` on all ingress paths |
| S1 | SyncBatch v2 (known words, lookup cards, unlocks, unreviewed states) + Unicode ID gate + shared `validate_batch` (folds version-validation P2); **schema freeze here** |
| C3 | Session-builder API with single admission budget |
| A6 | Identity-rotation guard UI (interim; core recovery semantics follow with S1) |
| backup-rotation P2 | Hoist pendingExport to ViewModel + payload validation |
| D7, D5 | Structured sync result; shared identity accessor (flip failing contract test) |

### Wave 3 — SDK + transport (~1 week; S11 is a hard sync point)
| Item | What |
|---|---|
| S11 | nostr SDK 0.43→0.44.x upgrade; verify unchanged behavior first |
| S10 | Byte-safe NIP-44 chunking (`decrypt_to_bytes`) + 32 KiB chunks + pacing |
| S6 | UUID-dedup applied-batch table replaces HLC admission filter (schema v5, S5 migration pattern) |
| S7 | Per-destination push cursors + import rewind |
| C4+C5+C12 | Practice gate policy + full-tier shuffle + organic latch (one PR, core/questions+curriculum) |
| D1 | Desktop alphabet-check/unlock commands + interactive rows (after C12) |
| D2 | Bundle morphology via tauri.conf.json resources (after R1) |
| Sharing-flush P2 | Suspend binding, `sharePubkeyHex`, drop reflection (after SDK settles) |

### Wave 4 — iroh + release gate (~1 week)
| Item | What |
|---|---|
| S8+S9 | Persist creator peers + active namespace; restart E2E |
| D6, D3-test, D8, D4 | Consume C3 API; S1 restore regression; session invalidation; CSS `[hidden]` |
| R4 | CI gates (workspace tests, desktop contract w/ `--locked`, Android unit+audit+lint baseline, ABI check, deb smoke) |
| R2 | Data-pipeline safety (any time; off critical path) |
| **Gate drill** | Signed build: navigation, save-failure/retry, process death, upgrade, full reinstall/restore, two-device skewed-clock + delayed-delivery; diff full persisted progress, not UUID counts |

## Lane assignment for parallel work
- **Rust core/scheduling:** Wave-0 C9 → Wave-1 {C1+C8, clock, C6, C2} → Wave-3 {C4+C5+C12}
- **Rust sync:** Wave-0 S3 → Wave-1 {S2, S5} → Wave-2 {S4, S1} → Wave-3 {S11 → S10, S6, S7} → Wave-4 {S8+S9}
- **Android:** Wave-0 {A1, A3} → Wave-1 {A2-set, A4-set} → Wave-2 {A6, backup P2} → Wave-3 {sharing P2}
- **Desktop:** Wave-0 D9 → Wave-2 {D7, D5} → Wave-3 {D1, D2} → Wave-4 {D6, D3-test, D8, D4}
- **Infra:** Wave-0 {R3, R1} → Wave-4 {R4, gate drill}; R2 opportunistic

## Risk watchlist
1. **S1 wire-format v2** — the only schema break; freeze after Wave 2, test v1-import compat.
2. **S11 API churn** — verify behavior-neutral before S10/S6 build on it.
3. **A4 recomposition loop** — external-push guard must track self-originated commits.
4. **C6 scheduling change** — previews/UI labels must show step intervals; mirror exactly in `rebuild_memory_states`.
5. **C2 existing installs** — cleanup migration for zero-state lookup rows.
