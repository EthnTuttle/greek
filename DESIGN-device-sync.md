# Design: Device Sync — CRDT State, Pluggable Transports (iroh-docs + nostr spool)

- **Status:** design (specification; core merge layer implemented in parallel, Wave 6a)
- **Date:** 2026-10-17
- **Author:** design subagent (Kimi K3)
- **Scope:** `greek-core` sync layer + Android (phone) ↔ future Tauri desktop, `/home/ethan/code/greek`

## 1. TL;DR

Sync the learner's state across an Android phone and a future desktop with **one
irreversible fact as the shared truth: the review log**. `review_logs` becomes a
**G-set (grow-only set) keyed by UUID** — merge is set-union, so convergence,
idempotency, and commutativity are free. `memory_states` are **deliberately not
synced**: each device recomputes them as a deterministic FSRS fold of the merged
history, ordered by `(hlc, uuid)` per card, which eliminates the hardest CRDT
problem (converging floating-point scheduler state) entirely. Settings —
including the alphabet-gate latch, so passing the check on desktop unlocks the
phone — are **LWW-per-key under a Hybrid Logical Clock**, with a monotonicity
rule that makes the gate latch un-losable. Transports are pluggable behind a
`SyncTransport` trait: **(a) iroh-docs** (iroh 1.0 + iroh-docs 0.101, official
UniFFI Kotlin bindings) for direct paired-device sync with DocTicket QR pairing,
and **(b) a nostr NIP-78/NIP-44 encrypted spool** on the user's self-hosted
strfry relay for store-and-forward when devices are not online together.

## 2. Problem

What breaks today:

- **Single-device state.** `memory_states`, `review_logs`, `curriculum_progress`,
  and `user_settings` (all in the app's SQLite, schema v2,
  `crates/core/src/db/mod.rs:30-75`) live only on the phone. The desktop shell
  planned in PLAN-PARALLEL-AGENTS.md §6-deferred vision (CLAUDE.md Phase 2) would
  start from a blank slate; progress on one device is invisible to the other.
- **No backup or export at all.** ANALYSIS.md §2 lists every persistence surface
  and none of them has an off-device copy. One lost/broken phone destroys the
  entire FSRS history — months of learning data with no recovery path. Sync
  doubles as disaster recovery: `transport zero` (§11) is a one-file JSON export.
- **State is scheduler-derived, not log-derived.** `memory_states` rows are the
  output of FSRS replay; the raw material (`review_logs`) is already append-only
  in spirit (`submit_review_with_source`, `crates/core/src/srs/mod.rs:164-254`)
  but carries no globally unique id, no device attribution, and no logical clock
  to merge by. Schema v3 adds these (§5).

## 3. Options considered

| Option | Verdict | Why it fails (or wins) |
|---|---|---|
| Roll-your-own G-set log + rebuild | **CHOSEN** | `review_logs` is already an append-only fact table; the app owns both ends; merge semantics are trivially provable (§6). No third-party state model to contort SQLite into. |
| cr-sqlite (sqlite-ext CRR tables) | Rejected | Turns every table into a causal-length CRR with its own connection, VFS, and C extension that must be bundled per-ABI on Android and re-linked under UniFFI. **Maintenance of the project is uncertain** (the core author's post-vlite funding/maintenance status has been publicly unclear; vlite was archived); building the app's whole future on an uncertain-dep is not defensible. Also CRRs target whole-DB replication, but we must *not* replicate `memory_states` (it is a fold — folding state that is itself replicated from a fold is incoherent). |
| Automerge / Yrs document sy­nc | Rejected | Both model shared *documents* (text/JSON), not a relational fact table feeding a floating-point scheduler. Mapping `memory_states` floats through a document CRDT reintroduces the merge-of-derived-values problem; binary blobs over FFI would have to replace our transparent SQL. Overkill machinery for one G-set + LWW map. |
| ElectricSQL / PowerSync / Zero | Rejected | All three are client↔server replicated-postgres/database products targeting multi-user SaaS with a hosted sync service. They require a running Postgres-compatible backend and an auth model for multiple *users*; our topology is same-owner devices, fully offline-first. |
| Syncthing / rsync of the sqlite file | Rejected | SQLite files do not merge — two devices that reviewed different cards concurrently lose one side's history wholesale (WAL semantics make corruption plausible). File-sync of a live single-writer database between two writers is the canonical foot-gun. |

The decisive insight: **FSRS state is a pure function of the review history.**
If the history is a G-set, the scheduler output is a deterministic fold of it —
so the only things that ever need merging are a set and a small key-value map.

## 4. Chosen architecture

```
                 ┌─────────────────────────┐         ┌─────────────────────────┐
                 │   Android (phone)       │         │   Tauri desktop (W7)    │
                 │  greek-core + SyncTransport impls │  greek-core + iroh impl │
                 └───────┬─────────────────┘         └──────────┬──────────────┘
                         │ merge API lives in greek-core         │
                         │ (transport-agnostic): events_since,   │
                         │ settings_since, apply_*, rebuild_*    │
           ┌─────────────┴───────────────────────────┴───────────┐
           │                 SyncTransport trait                 │
           └───────────────┬───────────────────────┬─────────────┘
                           │                       │
                 ┌─────────▼─────────┐   ┌─────────▼───────────┐
                 │ IrohDocsTransport │   │ NostrSpoolTransport │
                 │ paired DocTicket  │   │ NIP-78 batch events │
                 │ direct P2P / n0   │   │ NIP-44 self-encrypt │
                 └───────────────────┘   │  strfry relay       │
                                         │  (user's own host)  │
                                         └─────────────────────┘
```

Layers, bottom up:

1. **Merge semantics in `greek-core`, transport-agnostic.** SQLite schema v3
   (§5), HLC logic (§7), history merge, FSRS replay
   (`rebuild_memory_states`), and the settings LWW rules (§6) are pure Rust
   unit-testable functions over `Database`. No network code.
2. **`SyncTransport` trait.** `push(SyncBatch)`, `pull(since_hlc) -> SyncBatch`,
   `pair(...)`, `status()`. Format on the wire (`SyncBatch` JSON; §Wire in
   SYNC-WIRE.md, authored by the transport agents — referenced, not duplicated).
3. **Two implementations:** `iroh-docs` (direct pairing, requires simultaneous
   online time), nostr spool (store-and-forward, eventual asynchronous sync).

> Key insight: because `memory_states` is a fold of the log, the problem *
> *"merge two divergent floating-point scheduler states"** never needs to be
> solved. The hard CRDT case collapses to "replay the merged log" — which also
> gives crash recovery, auditability, and parameter migration for free (§14).

## 5. Data model & schema v3

Schema v3 migration (`user_version` 2 → 3), implemented verbatim by the
parallel core agent:

```sql
-- review_logs additions (user_version 2 -> 3)
ALTER TABLE review_logs ADD COLUMN uuid TEXT;
ALTER TABLE review_logs ADD COLUMN device_id TEXT;
ALTER TABLE review_logs ADD COLUMN hlc TEXT;
CREATE UNIQUE INDEX idx_review_logs_uuid ON review_logs(uuid);
CREATE TABLE settings_sync(key TEXT PRIMARY KEY, value TEXT NOT NULL, hlc TEXT NOT NULL);
CREATE TABLE sync_devices(device_id TEXT PRIMARY KEY, label TEXT NOT NULL DEFAULT '', paired_at TEXT NOT NULL, last_sync_hlc TEXT);
```

Backfill for existing rows: `uuid` = random v4 per legacy row; `device_id` =
this device's bootstrapped id (§8); `hlc` = derived from the row's existing
`timestamp` (`{ms:013}-{counter:0000}-{this-node:8hex}`), so legacy rows sort
before any post-migration row — correct, since they genuinely happened first.

| Table / data | Synced? | Semantics |
|---|---|---|
| `review_logs` (with `uuid`, `device_id`, `hlc`) | ✅ Synced | G-set, union by `uuid` |
| `user_settings` + `settings_sync` | ✅ Synced | LWW-per-key by `hlc`; `settings_sync` is the sync-coherent ledger of the same values |
| `sync_devices` | ✅ Meta | Device roster; pairing metadata, not user state |
| `memory_states` | ❌ Derived | Recomputed by `rebuild_memory_states` on each device from merged `review_logs` |
| `curriculum_progress`, `known_words`, `lookup_cards` | ✅ Synced (as derived/evaluated over merged history; `lookup_cards` rows join the G-set via their review logs or as LWW content rows — same trait) | Rule: anything a user action writes lands in either the G-set or the LWW map; there is no third kind |
| `morphology.sqlite` (attached read-only as `morph`, `db/mod.rs:82-88`) | ❌ Never | 109 MB asset shipped with the app; identical on every install |
| Card JSON (`data/cards/*.json`) | ❌ Never | Ships with the app; deck versioning handled by app releases |

## 6. Merge semantics

The merge layer owes these numbered proof obligations (each maps to a test in
§13):

1. **Convergence under shuffle.** Any permutation of the union of devices'
   `review_logs` rows, applied via `apply_review_events`, converges to
   identical `(memory_states, settings)` on every device. Holds because inserts
   are keyed by `uuid` (set-union) and replay order is a total order (§7).
2. **Idempotency.** Re-applying any prefix of an already-applied batch is a
   no-op: `apply_review_events` dedupes on `idx_review_logs_uuid`, returns
   count of *new* rows. `SyncBatch` from the same peer may resend freely.
3. **Commutativity.** Batch order between peers does not matter: set-union and
   LWW both commute.
4. **Replay determinism under clock skew.** `rebuild_memory_states` orders per
   card by `(hlc, uuid)`, not by local insertion order or raw wall time. HLC
   gives a total order even when device clocks disagree (skew is bounded by
   HLC's `max(own_ms, max_seen_ms)` rule); the `uuid` tail tie-breaks rows the
   HLC cannot separate (e.g. legacy backfill at counter 0 or a burst in one
   millisecond). Devices with genuinely different event *causality* therefore
   still agree on replay order, hence on FSRS output.
5. **Practice-source rows are portable.** Rows with `source='practice'`
   (schema v2, `db/mod.rs:109-116`; semantics in `srs/mod.rs:299-310`) merge
   like any other — so letters practice done on desktop feeds the phone's
   organic-lane accuracy (`curriculum/mod.rs` Lane B evidence queries) and
   practice accuracy is measured over the *merged* history on any device.
6. **Daily new-card cap holds globally.** `new_cards_today`
   (`srs/mod.rs:277`) counts first-ever reviews in the log, so introductions
   derive from the *merged* log; a card introduced on desktop cannot be
   re-introduced as "new" on the phone, and the 10/day cap
   (`DEFAULT_DAILY_NEW_CAP`) is enforced against the union, not per-device.
   (Boundary note: two devices that each burn the cap before their next sync
   can briefly total 20/day; accepted — the cap is a pacing guardrail, not a
   correctness invariant.)
7. **Alphabet-gate latch never reverts.** Gate satisfaction is *monotone*
   (once passable, always passable). Plain LWW could lose a `satisfied` latch
   if one device wrote it and the other synced an older clock with the key
   absent. **Rule:** merge *only* writes when incoming `hlc` is *strictly
   greater* than local — and **absence is not a value**: an incoming batch
   that simply lacks key `k` cannot delete or downgrade `k`. All latch rows
   (`alphabet_gate`, `alphabet_gate_at`) and every settings key obey this.
8. **Cross-device chapter unlock.** Unlock state is derived from merged
   `memory_states.stability` (`MASTERY_STABILITY_DAYS`, PLAN §1 / `curriculum/
   mod.rs`), so a chapter unlocked by desktop progress reads unlocked on the
   phone after one sync, and `unlock_chapter` intent rows are LWW settings.

## 7. Hybrid Logical Clock

Text form, stored in `review_logs.hlc` and `settings_sync.hlc`, lexicographically
sortable:

```
{ms:013}-{counter:04}-{node:8hex}
```

- `ms`: 13-digit zero-padded Unix epoch milliseconds of the local clock,
  always `max(local_wall_ms, last_ms)` (HLC rule) — so a jumped-back clock
  never breaks ordering, and a jumped-forward clock only inflates `ms` once.
- `counter`: 4-digit zero-padded, incremented when `ms` did not advance;
  0000 on first event of a millisecond.
- `node`: 8 hex chars derived from `device_id` (§8), making distinct devices
  totally ordered even at identical `(ms, counter)`.

**Per-device monotonicity is persisted**: `user_settings['hlc_last_ms']` holds
the last emitted `ms`; `hlc_now(db)` clamps the wall clock up to it (and
advances counter) after restarts, so a reboot or an Android clock-set-back can
never produce a smaller HLC. Backfill rows use `counter=0000`, this device as
node (§5) — strictly lower than any live event keyed on a real device id only
if ids differ, which pairing guarantees (§8).

## 8. Identity & pairing

- **`device_id`**: random UUID v4, bootstrapped on first run after migration,
  persisted in `user_settings['device_id']`; inserted into `sync_devices` with
  a human label ("Pixel 9"). Never reused after wipe.
- **iroh pairing (direct devices).** The desktop generates an iroh-docs
  `DocTicket` — a single self-contained string carrying doc id + all secret
  keys needed to read and write — because we use one private doc per
  device-pair document. A QR encodes `greeksync://ticket/<ticket>`. The phone
  scans it: pairing and authorization happen in one gesture. Per-device iroh
  node/author keys are random ed25519, persisted: on Android the iroh secret
  key goes to `EncryptedSharedPreferences`; on desktop, the Tauri keyring.
  The doc namespace write-capability lives in the pair ticket, so possession
  of the ticket *is* the write capability; it is never sent anywhere else,
  and is exported to a newly-added device only via the same QR flow.
- **nostr pairing.** A single `nsec` is the sync identity shared (as a *user*-
  controlled secret) by all the user's devices. Stored in Android Keystore-
  backed `EncryptedSharedPreferences`, entered on desktop via QR (`nostr:nsec…`)
  or manual entry. Payloads are **NIP-44 self-encrypted** (versioned payload,
  conversation key derived against *the user's own* pubkey), so the relay and
  all network observers see only ciphertext. Checkpoint events (§9b) use
  **d-tag blinding** — the `d` label is `HMAC(secret, "checkpoint")` rather
  than a fixed literal — because relays index and can enumerate `d` labels;
  blinding hides even the *existence* and cadence of sync metadata from the
  relay operator (§10).

## 9. Transports

### 9a. iroh-docs (direct paired sync)

- Versions: **iroh 1.0 + iroh-docs 0.101** (both released June 2026), official
  UniFFI Kotlin bindings on Maven as `computer.iroh:iroh:1.0.0`.
- **Entry key layout:** `b"reviews/<hlc>-<uuid8>"` → content is one JSON
  `ReviewEvent`. Ordering falls out of the lexicographic key; uniqueness of
  `uuid8` breaks ties. Content per entry is well under ~1 KB, so no blob work
  is needed — but the implementer must verify the current small-content/inline
  threshold in the iroh-docs 0.101 API before assuming entries avoid the blob
  layer (§15).
- Docs engine: `Docs::persistent(redb)` storage; discovery = n0 preset
  (DNS + n0 relays, used for connectivity off-LAN — the relays are pipes, not
  storage).
- **Simultaneous-online constraint:** iroh-doc sync is live P2P; if the phone
  and desktop are not both online, nothing syncs. UX consequence: sync runs on
  app foreground (plus a manual **Sync now** button in the Sync settings
  screen, §12). The nostr spool exists exactly for the asynchronous case.
- **Android lifecycle:** backgrounded apps may be suspended; the iroh node is
  shut down on `onStop` and re-bound on foreground using the persisted secret
  key. Optional foreground service for long first-sync. (Per iroh Android
  notes in the official docs; see §15 for the open item on exact bind cost.)

### 9b. nostr spool (store-and-forward)

- Plain `SyncBatch` JSON is NIP-44 self-encrypted and published as a **regular
  kind 1078 event** (NIP-78 family; kind 1078 chosen as an unclaimed custom
  batch kind in the 1000–9999 regular range so every strfry/nostr-rs-relay
  retains it until prune policy). Batches are chunked so **ciphertext stays
  ≤ 48 KiB** per event.
- Tags: `[["t","greek-sync"],["d",<device>],["b",<batch>],["i",<idx>],["n",<count>]]`
  — `t` for fetch filtering, `d` for provenance, `b/i/n` for chunk reassembly.
- Checkpoint: **kind 30078 (addressable, `d`-tagged `HMAC`-blinded)** published
  to-self holding `{from_hlc, state}` so any device can pull "everything after
  the last checkpoint" without scanning all history events — the relay serves
  the newest addressable event per tag.
- Fetch on the other device: `filter { authors: [self-pubkey], "#t":
  ["greek-sync"] }` → signature-verify → NIP-44 decrypt (drop on failure, §10)
  → reassemble → `apply_review_events` / `setting_merge` → `rebuild_memory_states`.
- **Deployment (appendix-target:§12 6c):** the relay is a **strfry** (or
  nostr-rs-relay) binary/Docker container colocated on the host already serving
  the user's self-hosted Jumble nostr client at
  `https://jumble.virginiafreedom.tech` — verified today as a static Svelte PWA
  over HTTPS (200), *not* a relay: no WS upgrade honored, and no `relay.`/
  `nostr.`/`wss.` subdomains exist. `git.virginiafreedom.tech` runs gitea, so
  self-hosting behind nginx with existing certs is the established pattern.
  Runbook: add a `wss.virginiafreedom.tech` (or path-gated `/sync-relay`)
  nginx vhost proxying WS→strfry with TLS from the existing certs, enable
  NIP-42 auth in strfry, and set a **pubkey write-whitelist** to the sync
  pubkey only so the relay cannot be used as public storage.

## 10. Security & privacy vs. adversaries

| Adversary | Mitigation |
|---|---|
| **Relay operator / malicious public relay** | All payloads NIP-44 self-encrypted; relay sees only ciphertext + blinded tags. Relay is untrusted storage. Forged or replayed events: every event signature-verified against the user's own pubkey before decryption; decrypt-failure = silent drop (do not error, do not apply). |
| **Network observer** | iroh traffic is QUIC/TLS; nostr over WSS. Metadata leakage is limited to device-pair timing and payload sizes; checkpoint d-tags are blinded (§8). |
| **Stolen/lost device** | The nostr nsec and iroh pair ticket are on-device secrets; if a device is lost, generate a new `DocTicket`/rotate nsec on the surviving device and re-pair — `sync_devices` lets you see and drop the lost `device_id` from the roster. Android secrets under Keystore; desktop under keyring. |
| **Garbage into the G-set** | `apply_review_events` rejects rows whose `card_id` is unknown to the merged deck or whose `rating` is out of range; malformed batches fail JSON parse and never touch SQL. The UNIQUE uuid index makes replays harmless. |

## 11. Backup/export: "transport zero"

A third `SyncTransport` writes the whole state to **one JSON file**: full
`review_logs` (with uuid/device_id/hlc) + `settings_sync` + `sync_devices` —
a degenerate `SyncBatch` with `from_hlc = 0000000000000-0000-00000000`.
Import on a fresh install is `apply_review_events` + `setting_merge` +
`rebuild_memory_states`. Requires no network and no peer; doubles as the
disaster-recovery path ANALYSIS.md §2 shows we lack today.

## 12. Phased roadmap

| Phase | Content |
|---|---|
| **6a — core merge layer** (in parallel with this doc) | Schema v3 exactly as §5; `ReviewEvent{uuid,card_id,timestamp(RFC3339),rating u32,elapsed_days f64,scheduled_days f64,source,device_id,hlc}`; `device_id(db)`; `hlc_now(db)`; `events_since(db,since_hlc)->Vec<ReviewEvent>`; `settings_since(db,since)`; `apply_review_events(db,&[ReviewEvent])->u32` (uuid-idempotent, batches trigger rebuild); `rebuild_memory_states(db)->u32` (deterministic fold per card ordered `(hlc,uuid)`); `setting_put`/`setting_merge` writing **both** `user_settings` and `settings_sync` under strict-greater LWW + absence-is-not-a-value; `pair_device`/`note_last_sync` over `sync_devices`. Hero, transport-agnostic, fully unit-tested (§13). |
| **6b — iroh transport + Android Sync settings screen** | `IrohDocsTransport` on `computer.iroh:iroh:1.0.0`; DocTicket QR generate/scan (`greeksync://`); Sync settings screen listing paired devices, `last_sync_hlc`/human last-sync time, and a **Sync now** button; foreground-service optional. |
| **6c — nostr spool + strfry runbook executed** | `NostrSpoolTransport` (kind 1078 chunked ≤48 KiB, blinded 30078 checkpoint); §9b runbook performed on the Jumble host; write-whitelist + NIP-42 verified; round-trip tested phone↔relay↔phone. |
| **Wave 7 — Tauri desktop shell** | Desktop app reuses `crates/core` unchanged plus the native iroh stack (no UniFFI needed); same merge layer, same database, same SyncBatch. |
| **Future — deck sharing between users** | Same transports, different privacy model (public G-set of cards rather than encrypted self-spool). Out of scope here. |

Wire format `SyncBatch` JSON — `{"v":1,"device_id","from_hlc","to_hlc",
"reviews":[ReviewEvent…],"settings":[{key,value,hlc}…]}` — encoding,
compression, and chunking details live in **SYNC-WIRE.md** (transport agents,
Waves 6b/6c); this document deliberately does not duplicate them.

## 13. Test matrix

- **Core unit tests (Wave 6a):** migration 2→3 idempotency and backfill shapes;
  HLC monotonicity across reboot (simulated `hlc_last_ms` > wall clock);
  strict-greater LWW; absence-not-a-value; uuid-idempotent
  `apply_review_events`; `rebuild_memory_states` equivalence to incremental
  `submit_review_with_source` on the same history (property: fold(update*) ==
  rebuild).
- **Core property tests:** random shuffle/replay of a generated two-device
  history converges (obligation 1); re-applying prefixes is no-op (2); batch
  order irrelevant (3); adversarial clock skew still orders deterministically
  (4); practice rows from device B feed lane-B state on device A (5); merged
  cap accounting (6); latch survives "old clock, key absent" batch (7–8).
- **Transport round-trips:** loopback two `IrohDocsTransport` instances in one
  process; nostr against a local strfry container (chunking, checkpoint
  resume, decrypt-fail drop on injected garbage).
- **Soak:** scripted 30-day two-device simulation — alternating online/
  offline intervals, both transports exercised, assert final
  `(memory_states, settings)` equality and row counts on both devices.

## 14. Risks & mitigations

| Risk | Mitigation |
|---|---|
| **aws-lc-rs / ring Android NDK build risk** (quinn/iroh TLS stack cross-compile to arm64) | Spike agent verifies the `computer.iroh:iroh:1.0.0` artifacts and an NDK build *before* 6b is scheduled; fallback to nostr-only transport if the native dep is unbuildable. |
| **iroh binary size on Android** | Measure after 6b spike; strip features; accept only if APK delta is tens of MB, not more. Nostr transport has a much smaller footprint and can ship first if needed. |
| **Relay retention / prune policy** | We use regular kind 1078 events so strfry keeps them; strfry runbook disables aggressive pruning for the whitelisted pubkey; checkpoints (30078) bound worst-case rescan anyway; full history re-pull always reconstructs. |
| **Wall-clock jumps** | HLC clamps via persisted `hlc_last_ms` (§7); backfill rows use zero counters; replay order is `(hlc, uuid)` — never raw wall time, so skew degrades ordering granularity, not correctness. |
| **FSRS parameter updates changing replay results** | `user_settings['fsrs_params_version']` (fsrs-rs version) is recorded; when parameters change ship them as explicit *events* or re-anchor the fold at a documented break point, because a fold's output is parameter-dependent. |
| **Database corruption during rebuild** | `rebuild_memory_states` runs in a single transaction and is idempotent; export (§11) exists independent of transports. |

## 15. Open questions

1. Exact small-content/inline blob threshold in iroh-docs 0.101 — determines
   whether `reviews/…` entries sit in redb inline or need a blob layer (§9a).
2. Whether `lookup_cards` / `known_words` / `curriculum_progress` ship as
   their own LWW content records or piggy-back exclusively on review-log
   evidence (§5 row marked "same trait" — decide with the 6a implementer).
3. Foreground-service policy for first sync of a large log on Android (
   user-visible notification + WorkManager? measured in 6b spike).
4. Whether `Sync now` should opportunistically try nostr first, iroh first, or
   race both (currently: iroh when a peer ticket exists, nostr always).
5. Multi-device (>2) nostr clock contention: nothing in the design *limits* it
   to two devices, but UX copy and tests so far assume phone+desktop.
