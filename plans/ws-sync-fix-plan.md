# Fix plan — backup/sync release blockers (S1–S10) + coupled P2s

Source: `production-readiness-2026-10-20.md` §"Backup and sync integrity" + §"Dependency security".
All S1–S10 re-verified against current source (2026-10-21); **no stale findings** (minor line drift only: S6 = transport.rs:126-137; S2 slicing = srs/mod.rs:417-428; S8 = sync-iroh `join_from_ticket` :295-316, `first_or_no_doc` :154-162, `sync_once` :423-452).

---

### S1 — Backup/restore drops Greek lookup data + ASCII-only card-ID import reject
- **Root cause:** `SyncBatch` has only `reviews`+`settings` (core/src/sync/mod.rs:158-168); `build_batch` (uniffi-bindings/src/lib.rs:884-900) exports the same; card-ID ingress gate admits ASCII alnum+`-_.` only (sync/mod.rs:239-248) while the app creates `lookup-<lemma>` Unicode IDs (uniffi lib.rs:793). Known-words, `lookup_cards` content, chapter unlocks, unreviewed lookup initial states never travel.
- **Fix:** Wire format **v2**: add serde-defaulted optional collections to `SyncBatch` (`known_words`, `lookup_cards`, `chapter_unlocks`, `unreviewed_states`); export includes them; import applies them uuid/ID-dedup. Relax the ID gate to reject only empty/oversize/control/SQL-hostile chars, accepting NFC-normalized Unicode (`chars().all(|c| c.is_alphanumeric() || matches…)`), and validate Greek lemma IDs round-trip. Import accepts v1 (subset) and v2; export always emits v2 → friends with the version-validation P2 (single `validate_batch` shared by file import, Nostr reassembly, iroh pull).
- **Files:** core/src/sync/mod.rs; uniffi-bindings/src/lib.rs (build_batch, import); SYNC-WIRE.md.
- **Test:** fresh-install restore probe with real `lookup-λόγος` review+known word+unlock asserting full restoration.
- **Effort L, risk high (wire format).** Depends on S2/S5 (import must be safe/atomic first); coupled to version-validation P2 and C2/C13 (lookup semantics).

### S2 — Malformed import timestamp panics, poisons DB mutex
- **Root cause:** `apply_review_events` (sync/mod.rs:236-266) never validates `timestamp`; fold hits unchecked `iso[0..4]`/`[5..7]`/`[8..10]` byte slicing (srs/mod.rs:417-428 → panic on `202α…`); import holds the UniFFI DB mutex (uniffi lib.rs:1001-1003). `add_days_to_now`'s silent clock fallback (srs/mod.rs:469-475) undermines replay.
- **Fix:** strict canonical RFC3339 validator (`YYYY-MM-DDTHH:MM:SSZ`, ASCII digit/`-`/`T`/`:`/`Z` shape + range checks via the same safe parser C9 introduces) at ingress; reject (drop + count) before mutation. Convert `iso_to_days`/`iso_to_secs` to boundary-safe `.get()` parsing (shared with C9).
- **Files:** core/src/sync/mod.rs, core/src/srs/mod.rs.
- **Test:** import `202α-10-09T12:00:00Z` → recoverable error, zero rows, subsequent API call succeeds.
- **Effort S, risk low.** Depends on C9 (shared parser); prerequisite for S1/S5.

### S3 — Settings ingress overwrites local identity/cursors
- **Root cause:** `setting_merge` blocks only nsec-shaped keys and writes winners into BOTH `settings_sync` and `user_settings` (sync/mod.rs:449-483) → remote/import rows for `device_id`, `nostr_last_push_hlc`, `hlc_last_ms`, etc. overwrite local state.
- **Fix:** explicit **allowlist** of portable preference keys (`theme`, `new_per_day`, `nostr_relays`, `alphabet_gate*`, notifications prefs) enforced in `setting_merge`/`apply_settings`; drop anything else (count for observability). Keep the blocklist as defense in depth.
- **Files:** core/src/sync/mod.rs (+tests).
- **Test:** batch carrying `device_id`/`nostr_last_push_hlc` rows → not applied; `theme` still merges.
- **Effort S, risk low.** Independent; interacts with S1 (decide which new S2-era keys are portable).

### S4 — Receive path never observes remote HLC → replica divergence
- **Root cause:** `hlc_now` consults only wall time + local last values (sync/mod.rs:95-137); no receive-side `hlc_observe`. Duplicate-reapply skips the repair fold (sync/mod.rs:268-269).
- **Fix:** add `hlc_observe(db, remote_hlc)` (monotonic advance of persisted `hlc_last_ms/counter` when incoming exceeds local) called once per batch per ingress path (apply_review_events / apply_settings / uniffi pull / iroh round). Idempotent, no fold-trigger change beyond the C1-consistent rebuild.
- **Files:** core/src/sync/mod.rs (new fn + call sites); uniffi-bindings/src/lib.rs (pull paths already funnel through core fns — minimal).
- **Test:** skewed-clock two-device probe: identical event sets → byte-identical `memory_states`.
- **Effort M, risk medium (touches clock state machine).**

### S5 — Partial-commit windows (import insert/fold, batch import, migrations)
- **Root cause:** per-event autocommit inserts then separate fold (sync/mod.rs:250-269); import applies reviews then settings unenveloped (uniffi lib.rs:1001-1003); migration ALTERs precede the `user_version` stamp with no transaction (db/mod.rs:117-206).
- **Fix:** use rusqlite `unchecked_transaction` (Database exposes `&Connection`, so no `&mut` needed): one TX around import-insert+fold, one around backup import, one around each migration block incl. version stamp; retry-safe (ALTERs inside TX roll back cleanly, steps re-run).
- **Files:** core/src/sync/mod.rs, core/src/db/mod.rs, uniffi-bindings/src/lib.rs.
- **Test:** fault injection (failing FSRS fold/late setting error) → assert all-or-nothing; interrupted-migration replay test.
- **Effort M, risk medium.** Depends on C1 (shared TX helper); prerequisite for S1, S6/S7 cursor work.

### S6 — Global Nostr pull cursor drops delayed/incomplete batches
- **Root cause:** partial batches tolerated whenever any complete batch decodes, then every batch filtered at/below one HLC cursor and cursor set to max `to_hlc` (sync-nostr/src/transport.rs:126-137; uniffi lib.rs:1191-1197).
- **Fix:** per-batch-UUID applied-set (new table `sync_applied_batches`, schema v5, migration follows S5 pattern); pull applies every complete batch whose `b` tag UUID is unseen, any HLC (dedup makes replay safe); keep the HLC value only as an observability high-water mark, not an admission filter.
- **Files:** sync-nostr/src/transport.rs (expose batch UUID), uniffi-bindings/src/lib.rs, core/src/db/mod.rs.
- **Test:** late-arriving older batch after newer one applied → both merge.
- **Effort M, risk medium.** Depends on S5/S10; on S11 (transport API churn — do after upgrade).

### S7 — Outbound push misses older arrivals / destination changes
- **Root cause:** single `nostr_last_push_hlc`/`iroh_last_push_hlc` filtered by originating HLC (uniffi lib.rs:1133-1135, 1225-1235); relay/identity/notebook changes don't reset (:1026-1054, :1213-1215); imported older rows never go out.
- **Fix:** scope push cursor to a destination fingerprint (relay-set hash / identity pubkey / iroh namespace hex); reset on destination change; on import, if merged rows contain HLC ≤ current cursor, rewind that destination's cursor to the min merged HLC (receivers uuid-dedup, so resend is safe).
- **Files:** uniffi-bindings/src/lib.rs, core/src/sync/mod.rs (+ tests), minor: sync-iroh via bindings.
- **Test:** import old backup after a push, then push → old rows delivered; switch relays → full resend to new destination.
- **Effort M, risk medium.** Depends on S5; iroh side entangled with S9's namespace identity.

### S8 — iroh creator-side restart stops sync
- **Root cause:** only the joiner persists peers (sync-iroh/src/lib.rs:295-316); creator restart reopens doc but `sync_once` early-returns on empty peer list (:423-433); failed `SyncFinished` counted as success (:443-451).
- **Fix:** record peers the creator observes (`SyncFinished` peer ids, `NeighborUp`) into `peers.json`; only count successful outcomes (check the event's result payload); keep `start_sync` opportunistic but never "succeed silently" on zero peers after a join exists.
- **Files:** sync-iroh/src/lib.rs; scripts/e2e restart harness.
- **Test:** restart E2E: creator restart → `sync_iroh_now` converges without re-sharing ticket.
- **Effort M, risk medium.** Depends on S11 only insofar as dev-deps/toolchain; logically independent.

### S9 — iroh doesn't persist selected notebook
- **Root cause:** join sets in-memory `state.doc`+peers only (lib.rs:304-312); reopen takes the FIRST stored namespace (`first_or_no_doc`, :154-162).
- **Fix:** persist active namespace id (`iroh-sync/active_doc`, 0600, or device-local `user_settings.iroh_active_doc`); write on create AND join; reopen by id with fallback to first; namespace-scope the S7 push cursor.
- **Files:** sync-iroh/src/lib.rs (state), uniffi-bindings/src/lib.rs (join/create call sites).
- **Test:** create A, join B, restart → B reopened; peers/cursors match namespace.
- **Effort S, risk low.** Couple with S8 (same crate); prerequisite-leaning for S7's namespace-scoped cursor.

### S10 — NIP-44 chunking splits Greek UTF-8 across boundaries
- **Root cause:** `chunk_batch` slices JSON at raw 48 KiB byte offsets (sync-nostr/src/codec.rs:25-31) while `nip44_decrypt_self` returns `String` (:73-78) → split Greek char = UTF-8 decode failure, batch unrecoverable, outbound cursor already advanced.
- **Fix:** after S11 (0.44 exposes `decrypt_to_bytes`) decrypt to **bytes** and reassemble bytes; belt-and-braces also split chunks on UTF-8 char boundaries. Bundle coupled P2: lower `CHUNK_SIZE` to 32 KiB (48 KiB → ~65.6 KB ciphertext > relay 64 KiB limit, codec.rs:13) and add publish pacing/backoff between chunk sends (per tests/live.rs:101-124 pattern).
- **Files:** sync-nostr/src/codec.rs, sync-nostr/src/transport.rs (pacing).
- **Test:** multichunk batch with Greek lemma IDs/glosses → encrypted roundtrip ≥3 chunks, zero decode failures; assert ciphertext ≤ 48 KiB/event.
- **Effort S, risk low.** Depends on S11; feeds S6 (decode must yield complete batches).

### S11 — nostr SDK security upgrade 0.43 → 0.44.x (release blocker per report)
- **Root cause:** lockfile pins `nostr-sdk 0.43.0`/`nostr 0.43.1`; RUSTSEC-2026-0224/-0232/-0231 (pool; sync client uses default NIP-42 auto-auth, transport.rs:29-30) and -0216/-0227 (NIP-44; sync decrypt path reachable).
- **Fix:** bump `nostr-sdk` (+dev-deps `nostr-relay-builder`/`nostr-database`) to ≥0.44.3 / nostr ≥0.44.7 in sync-nostr and share-nostr Cargo.tomls; port API churn (`Client`, `send_event` output shape, `nip44` module moves, `Filter::custom_tag`, auth opts — keep sync auto-auth as-is but read defaults anew; keep share-nostr's disabled auto-auth, publisher.rs:194-195); rerun offline codec/transport tests + live harness.
- **Files:** crates/sync-nostr/{Cargo.toml,src/*}, crates/share-nostr/{Cargo.toml,src/identity.rs,publisher.rs}, uniffi `nostr_keys`/identity helpers, Cargo.lock.
- **Test:** full workspace test suite + post-upgrade live large-batch (Unicode) E2E.
- **Effort M, risk medium.** **Sequence before S6/S8/S9/S10 implementation work** to avoid porting fixes twice; after S1–S5 (which barely touch the SDK).

---

## Recommended fix ORDER
1. **S2** — smallest, makes every later ingress change safe; pairs with C9.
2. **S3** — independent one-file allowlist; parallel with S2.
3. **S5** — atomicity must precede schema/cursor changes; pairs with C1. (Can start parallel.) 
4. **S4** — convergence invariant before any cursor semantics rework; parallel with S5 after S2.
5. **S1** — wire v2 schema; gates on S2/S5 safety, locks schema + version-validation policy (fold the P2 into the same acceptance matrix PR).
6. **S11** — SDK upgrade now, BEFORE touching transport code, once core-side (1–5) churn is done; verify unchanged behavior first.
7. **S10** — chunking + relay-size P2 on the new SDK (uses `decrypt_to_bytes`).
8. **S6** — pull-cursor→UUID dedup, parallel with S7 (different crate split: sync-nostr vs uniffi/core cursors).
9. **S7** — per-destination push cursors (share namespace keys with S9's doc id if landed).
10. **S8+S9** — same crate (sync-iroh), do together sequentially after S11; restart E2E covers both.
11. **Gate** — full signed two-device restore/delayed-delivery/skewed-clock drill per release-gate §6.

Parallel plan: tracks A = {2,3}, B = {5+4}, then 6 serial, then C = {7→8} ‖ D = {9}, then E = {10+8+9 iroh pair}. SDK upgrade (6) is a hard sync point for all transport crates.