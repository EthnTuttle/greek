# Greek production-readiness review

Date: 2026-10-20. Source revision: `0b766e7` (Android `0.1.1`, version code 3).

## Verdict

**Do not ship the current revision as a production release.** There are reproduced Android screen crashes, non-atomic review writes, loss of legitimate Greek lookup data during backup/restore, replica divergence, and incorrect learning-flow behavior. The standalone desktop also lacks a complete progression and dictionary-installation path. Passing unit tests do not cover several of these integration failures.

This is a source/artifact review with automated host checks. It is not certification of a freshly signed build on a physical device. The older `ANALYSIS.md`, `ROLLOUT.md`, and website feature/test descriptions are partially stale; findings below use current source and observed results.

Priority terminology: **P1** = release blocker; **P2** = important correctness, compatibility, or operational issue. Reproduction evidence is distinguished from source-level findings.

## Verified checks

| Check | Result |
|---|---|
| `nix develop --command cargo test --workspace --locked` | 171 passed, 0 failed, 4 live-network tests ignored |
| Android `:app:testDebugUnitTest` | 27 passed, 0 failed |
| Android `:app:polytonicAudit` | Passed: 173 inventory codepoints covered |
| Android `:app:lintDebug` | Failed: 4 errors, 57 warnings; requires triage, not four demonstrated runtime defects |
| Android release manifest/native-library merge | Passed in a temporary source copy without signing files |
| Desktop `node dev/test-ui.mjs` | 51/51 mocked UI checks passed; actual rendered CSS revealed defects the mock misses |
| Original desktop locked/offline build | Failed before compilation: checked-in lockfile needs updating |
| Desktop contract tests in a disposable copy with refreshed lockfile | 8 passed, 1 failed: existing identity lookup reads the obsolete secret location |
| Desktop Linux package/smoke in disposable copy | A 20.2 MB `.deb` built; isolated fresh-profile launch, real IPC review, close/restart and persistence passed; dictionary asset absent |
| Greek keyboard class initialization | Reproduced `StackOverflowError` against current compiled classes |
| Morphology database | `PRAGMA quick_check` = `ok`; 36,165 lemmas, 958,469 forms, no orphaned form references |
| Card data | 1,183 cards; no duplicate IDs or blank fronts/backs |
| Source/Android morphology asset comparison | Identical SHA-256: `e97d66b77154ce8ac1c5380d44d8c50ba8397719297a76b1bd15569df02ba411` |
| Public site/privacy page | Both HTTP 200 |
| GitHub Releases | Public API returned an empty release list; website download links currently lead there |

Android checks reused the existing native libraries, so passing JVM/manifest checks do not establish that a current-source release APK works end to end. No device installation or physical-device instrumentation was performed in this review.

## Android release blockers

### A1 — P1: essential screens crash during keyboard initialization (reproduced)

`android/app/src/main/java/com/greekapp/ui/components/GreekKeyboard.kt:75-80` defines a local `put()` that calls itself instead of the map receiver. Initializing `GreekKeyboardKt` overflows the stack. Dictionary, Practice/Alphabet Check, and Letter Typing call `GreekKeyboardContainer`, so selecting the system IME does not avoid class initialization.

Host reproduction: `Class.forName("com.greekapp.ui.components.GreekKeyboardKt")` throws `StackOverflowError`, repeatedly at `GreekKeyboard.kt:77`.

Fix the shadowed helper/receiver call and add a screen-opening integration check that exercises the actual keyboard-containing composables.

### A2 — P1: review-save failures are acknowledged as success (source-verified)

`android/app/src/main/java/com/greekapp/viewmodel/ReviewViewModel.kt:222-235` catches exceptions but ignores the Boolean returned by `submitReview`. The Rust export explicitly converts storage errors into `false` at `crates/uniffi-bindings/src/lib.rs:439-452`. A failed new-card save, for example under full storage, can therefore advance the session without a review log being stored.

Practice has a related path at `PracticeViewModel.kt:254-263`, and Alphabet Check can report local success despite failed persistence at `:297-314`.

Check unsuccessful return values, retain the answer/card for retry, and verify failure behavior using a failing persistence implementation.

### A3 — P1: chapter taps target the wrong textbook (source-verified)

`android/app/src/main/java/com/greekapp/ui/screens/CurriculumScreen.kt:102` discards the textbook and passes only a chapter number. `viewmodel/CurriculumViewModel.kt:135-141,169-176` then finds the first curriculum containing that number. Since Letters precedes Athenaze and JACT (`crates/core/src/curriculum/mod.rs:147-174`), tapping Athenaze chapter 2 acts on Letters chapter 2 and leaves the requested chapter locked.

Carry `(textbook, chapter)` through the callback and unlock operation, and verify books with overlapping chapter numbers.

### A4 — P1: built-in keyboard edits are discarded (source-verified)

`android/app/src/main/java/com/greekapp/ui/components/GreekInputController.kt:264-293` changes internal `TextFieldValue` only. The screen's `onValueChange` callback is called exclusively by the system-field handler at `:338-340`. Recomposition resets differing internal text to the unchanged external value at `:329-331`.

This is an independent input failure after A1 is fixed. Panel edits must reach the screen's authoritative state.

### A5 — P1: APK ABI metadata allows unsupported installations (source/artifact-verified)

`android/app/build.gradle.kts:23-34` does not filter ABIs. Existing APKs and the freshly merged native-library set contain 32-bit `armeabi-v7a`/`x86` dependency libraries, but the app's Rust library is present only for `arm64-v8a` and `x86_64`. An unsupported 32-bit device can install and then fail the uncaught native load at `GreekApplication.kt:171`.

Filter the package to the intended supported ABIs and check the actual final APK contents/install eligibility.

### A6 — P1: identity re-generation overwrites the existing decryption key (source-verified)

`android/app/src/main/java/com/greekapp/ui/screens/SyncScreen.kt:258-265` offers generation even with an existing identity. `crates/uniffi-bindings/src/lib.rs:1033-1043` replaces the stored secret. If there is no other copy of the previous key, relay history encrypted with it becomes inaccessible; existing peers also retain the former identity. Local review rows are not deleted by this operation.

Treat existing-identity replacement as an explicit rotation/recovery operation and preserve the ability to recover history encrypted under the prior identity.

## Additional Android correctness issues

- **P2 — Completed-session restoration:** `ReviewViewModel.kt:262-280` marks completion without advancing the index and clears only DataStore. `SavedStateHandle` retains the batch and takes restoration precedence (`:50-56`), so restoring after process death can offer the last card again. Writes/clears are separate unawaited coroutines at `:312-345`.
- **P2 — Manual sharing flush fails reflection:** `core/RustCoreAdapter.kt:414-422` requests zero-argument `shareFlushNow`, but the generated method is suspending and has a JVM `Continuation` parameter (`uniffi_bindings.kt:2688`, verified with `javap`). The fallback can report an empty result and mark sharing unavailable (`RustCoreAdapter.kt:316-346,448-450`).
- **P2 — Backup export is lost on activity recreation:** `ui/screens/SyncScreen.kt:60-66` retains the pending JSON with `remember`. Rotation while the document picker is open loses the payload; the callback skips writing and may leave an empty document.
- **P2 — Reminders overstate enabled status:** Android 13+ notification permission is declared but never requested. `SettingsScreen.kt:109-116` schedules after a preference change, and `ReminderReceiver.kt:45-50` attempts notification without obtaining permission. There is also no boot receiver to re-arm alarms after restart.
- **P2 — Reverse selections crash editing (reproduced):** `GreekInputController.kt:270-287` forwards directional selection endpoints to helpers that use them directly in `replaceRange`/`removeRange` (`:92,136`). Both helper probes with `(3,1)` threw `IndexOutOfBoundsException`.
- **P2 — Accessibility/layout:** the non-scrollable `HomeScreen.kt:39-43` can put Start Review (`:175-187`) outside a short/large-font window. Reader lookup at `ReaderScreen.kt:239-256` uses raw pointer gestures without equivalent accessible word actions.

Paths in this section are relative to `android/app/src/main/java/com/greekapp/`, except the generated bindings, which are under `android/app/src/main/java/uniffi/uniffi_bindings/`.

## Learning-core correctness

The following probes were run using in-memory application databases and the actual checked-in decks. The morphology probe attaches the reference database read-only/immutable. The parent reviewer reran both probe binaries and confirmed the outcomes below; the intentionally caught malformed-timestamp panic is part of the probe, not a crash of the production app or modification of user data.

### C1 — P1: review writes are not atomic (fault-injection reproduced)

`crates/core/src/srs/mod.rs:225-265` performs the memory-state upsert and review-log insertion as separate autocommit operations. A trigger rejecting the log insert made submission return an error while leaving `reps=1` and zero review logs. Removing the trigger and retrying produced **two repetitions but only one log**.

This compounds A2: fixing the Android Boolean check alone does not make retries safe. Commit scheduling state, the review event, and HLC changes in one transaction, and assert that injected failures leave all three unchanged.

### C2 — P1: lookup cards use an invalid initial FSRS state (reproduced)

`crates/uniffi-bindings/src/lib.rs:817-823` inserts an unreviewed lookup card with zero stability/difficulty. `crates/core/src/srs/mod.rs:133-140,174-188` treats the presence of a row as a learned FSRS state.

All four initial lookup rating previews were 864 seconds, whereas a normal new card's Good preview was 274,147 seconds. A lookup card's first Good stored stability **0.01**, but reconstruction from that same one-review history produced **3.173**. Local state and replay therefore disagree without requiring a network fault.

Represent unreviewed lookup cards as genuinely new (`None` for FSRS), and check incremental-versus-replay equivalence.

### C3 — P1: the 10/day introduction cap admits a 20-card session (reproduced)

`crates/uniffi-bindings/src/lib.rs:403-409` caps each chapter query against the same completed-review count. Android fills across chapters at `ReviewViewModel.kt:166-186`; no shared reservation/admission budget is decremented while constructing the session.

With the alphabet declared known, the real caller pattern returned 7 Letters + 10 Athenaze + 3 JACT cards. Submitting the queue produced **`new_cards_today() == 20`**. Enforce one introduction budget across all sources and account for outstanding admissions/submission races.

### C4 — P1: ordinary Practice bypasses the alphabet gate (reproduced)

`crates/uniffi-bindings/src/lib.rs:531-566` draws from the entire loaded deck without gate, chapter, or new-card allowance filtering. At `:604-615`, practice results create ordinary memory states. The grandfather predicate in `crates/core/src/curriculum/mod.rs:291-304` then interprets the new vocabulary state as prior vocabulary history.

A fresh core's gate was false; ordinary Practice returned `ath1-001` through `ath1-010`. Saving one practice answer changed the gate to **true, method `vocab_history`**. Android's ordinary Practice route calls this API directly (`PracticeViewModel.kt:126-130`). Apply a shared eligibility policy while preserving the explicitly designed placement-test path.

### C5 — P1: ordinary Practice samples the same prefix indefinitely (reproduced)

`crates/core/src/questions/mod.rs:45-50,74-85` only shuffles caller-ordered windows of `limit` cards, stopping after enough questions. The binding preserves deck order within the reviewed tier (`crates/uniffi-bindings/src/lib.rs:546-559`). Seeds 1, 7, 42, and 1337 all selected precisely `ath1-001` through `ath1-010` from the real merged deck.

Once these cards have been practiced, they remain at the start of the reviewed tier. Shuffle/rotate the full eligible priority tier before selecting a subset, and test coverage beyond the initial window.

### C6 — P1: first-failure relearning does not meet the same-session promise (reproduced)

`crates/core/src/srs/mod.rs:190-209,469-475` uses FSRS's retention interval directly without an explicit short learning/relearning-step policy. With locked FSRS 2.0.4, a new card's Again preview is **34,780 seconds (9h 39m 40s)**.

Sub-day timestamp preservation is genuinely fixed, but this interval cannot support the documented ordinary-session recycling behavior (`docs/guide.html:109-114`). The current tests only require less than 24 hours or manually move the due date into the past. Implement the intended short-term steps and verify them with an injectable clock.

### Additional learning-core issues

- **C7 — P2, silent empty-deck initialization:** `crates/uniffi-bindings/src/lib.rs:344-349,365-384` uses `unwrap_or_default()` for card JSON. Malformed input successfully initializes with zero cards; existing due rows may be rendered with blank content. Return an initialization error and validate loaded cards.
- **C8 — P2, state read failures reset history:** `crates/core/src/srs/mod.rs:330-348` collapses SQL/conversion errors into “no state.” A row with nonnumeric difficulty, reps 8, and lapses 2 was silently replaced after a successful review with reps 1/lapses 0. Distinguish a missing row from a failed read.
- **C9 — P2, malformed timestamps panic:** `crates/core/src/srs/mod.rs:417-424,452-459` byte-slices unvalidated dates. A stored date containing a non-ASCII month caused a caught panic at line 423. Through UniFFI this occurs while mutexes are held and can poison later calls. Use validated parsing and recoverable errors.
- **C10 — P2, repeated placement prompts:** `crates/core/src/questions/mod.rs:383-405` mixes shuffled picks with an unshuffled cursor. Seed 42 generated 24 questions but only 18 unique card IDs; seed 1337 produced 17. Consume one shuffled pool per chapter and assert unique prompts.
- **C11 — P2, backend chapter mastery is advisory:** `crates/uniffi-bindings/src/lib.rs:390-436,664-673` and `crates/core/src/curriculum/mod.rs:475-482` do not enforce chapter unlock/mastery requirements. A locked Athenaze chapter 2 with 0/19 prerequisite mastery returned ten new cards and accepted an unlock call. Enforce progression at the API boundary, with explicit override semantics if needed.
- **C12 — P2, organic alphabet completion can relock:** `crates/core/src/curriculum/mod.rs:314-330` returns satisfaction without persisting a latch; the binding only records explicit completion paths (`crates/uniffi-bindings/src/lib.rs:707-730`). Twenty correct attempts over two dates satisfied the organic gate; five further incorrect attempts made it false, with zero latch rows. Persist first satisfaction.
- **C13 — P2, lookup content changes under existing history:** `crates/uniffi-bindings/src/lib.rs:793-815` keys by lemma but overwrites the front with each encountered form and changes the gloss while retaining scheduling state. Repeated additions are not the documented no-op. Use stable lexical identity/content and explicit revision behavior.
- **C14 — P2, equally correct distractors are marked wrong:** `crates/core/src/questions/mod.rs:157-185` excludes duplicate front strings, not equivalent answers. A real-data question for “it is necessary (impersonal)” offered both `χρή` and `δεῖ` but accepted only `χρή`. Add semantic/accepted-answer checks and curated fixtures.
- **C15 — P2, shipped participles are misclassified:** `crates/core/src/morphology/mod.rs:123-137,184-192` misses Morpheus's `t` participle prefix. The corpus has 222,867 such forms. The reader's `ἀρξάμενος` (`t-sa-mmn-`) omits “participle” from its analysis. Test the actual tag dialect, not only synthetic `v...p...` fixtures.
- **C16 — P2, dictionary search rejects common unaccented queries:** `crates/core/src/search/mod.rs:22-48` searches headword/definition FTS without using normalized headwords. Real `λόγος` works while `λογος` does not; the same occurs with common words such as `ἄνθρωπος`/`ανθρωπος`. Use shared Greek normalization for matching/indexing.

Lower-priority cleanup: strict typed grading should NFC-normalize canonically equivalent spellings; local submission accepts invalid ratings/unknown card IDs; negative query limits can become unbounded SQLite queries; some DCC learning units duplicate the same headword. Preserve existing IDs/history when curating content.

## Backup and sync integrity

### S1 — P1: backup/restore loses real Greek lookup data (reproduced)

`SyncBatch` carries only reviews and settings (`crates/core/src/sync/mod.rs:159-167`), omitting known words, custom lookup-card content, manual chapter unlocks, and the initial state of unreviewed lookup cards. Additionally, the app creates IDs such as `lookup-λόγος` (`crates/uniffi-bindings/src/lib.rs:793`), while import admits ASCII card IDs only (`crates/core/src/sync/mod.rs:239-248`).

The network-free API probe added and reviewed a Greek lookup card, marked it known, exported, and imported into a fresh core. The review existed in the export, but import reported **zero received reviews** and the known word was not restored. The desktop review independently reproduced missing custom cards and chapter unlocks.

Extend the backup/sync schema to cover actual user progress and custom card definitions, and validate against IDs the app legitimately creates. Include a fresh-install restore test with real Greek reader additions.

### S2 — P1: a malformed import can poison the core (reproduced)

Review ingress validates rating, card-ID charset and HLC but not timestamps (`crates/core/src/sync/mod.rs:236-265`). A timestamp such as `202α-10-09T12:00:00Z` reaches unchecked byte slicing during folding (`crates/core/src/srs/mod.rs:417-424`). The import holds the DB mutex (`crates/uniffi-bindings/src/lib.rs:1001-1003`), so the panic poisons it.

The probe confirmed both the import panic and a subsequent unrelated API call panicking. Earlier log insertion has already committed. Malformed ASCII date fallback to the current clock (`srs/mod.rs:469-475`) also undermines deterministic replay. Validate bounded canonical timestamps before mutation, return recoverable errors, and verify the core remains usable after a rejected import.

### S3 — P1: settings ingress can overwrite local identity/cursors (reproduced)

`crates/core/src/sync/mod.rs:449-480` accepts any setting key except secret-shaped keys and writes it into both synchronized and local settings tables. The probe successfully replaced `device_id` and installed a future `nostr_last_push_hlc` through backup import.

Local-only HLC state and transport cursors are also reachable this way. A future push cursor suppresses normal history uploads. Enforce an allowlist/schema for portable preferences and reject device-local identity/replication metadata at ingress.

### S4 — P1: receiving a remote clock does not advance the local HLC (reproduced)

`crates/core/src/sync/mod.rs:95-136` considers only wall time and previously local clock values. Receive paths do not observe incoming HLCs. A local review following an ahead-of-time remote review can therefore sort before the event that it followed locally.

The sync probe produced the same two-event set with stability **0.949919 on one replica versus 7.861664 on the other**. Reapplying duplicates inserts zero rows and skips the repair fold (`:268-269`). Implement receive-side HLC observation and ensure incremental scheduling agrees with canonical event ordering, including skewed-clock tests.

### S5 — P1: import, folding and migrations have partial-commit windows (reproduced/source-verified)

In addition to C1's review-write failure:

- Imported logs commit before memory-state folding (`crates/core/src/sync/mod.rs:250-269`). A crash/fold failure leaves partial state, and retrying duplicates can skip rebuilding.
- Backup import commits reviews and earlier settings before a later setting error (`crates/uniffi-bindings/src/lib.rs:1001-1003`). The probe returned an import error while retaining both the review and the earlier `theme` setting.
- Migration ALTER statements precede the version stamp without an encompassing transaction (`crates/core/src/db/mod.rs:117-146,206`). Interruption can leave a version that retries already-committed column additions. This migration case is source-verified; completed-migration tests do not test interruption.

Make each state transition atomic and test interruption/failure recovery, not only completed-operation idempotency.

### S6 — P1: a global Nostr pull cursor skips delayed batches (source-verified)

`crates/sync-nostr/src/transport.rs:125-136` tolerates incomplete/failed batches when any complete batch is available, then filters every batch at or below a single HLC cursor. UniFFI advances that cursor to the maximum returned HLC (`crates/uniffi-bindings/src/lib.rs:1188-1196`).

If batch A is incomplete while newer B succeeds, A's later final chunk cannot recover it: its completed batch is now below the cursor and is discarded. Late uploads and lagging device clocks have the same problem. Use reconciliation/overlap and UUID dedup or per-origin completeness tracking instead of assuming globally ordered arrival.

### S7 — P1: outbound progress misses older arrivals and destination changes (source-verified)

Both transports select `hlc > last_push` (`crates/core/src/sync/mod.rs:173-184`; `crates/uniffi-bindings/src/lib.rs:1133-1135,1225-1235`). Relay/identity updates and iroh joins do not scope/reset those cursors (`:1026-1054,1213-1215`).

Importing older history after a prior push adds legitimate local rows that never get sent. Switching to a new relay/identity/notebook can similarly populate only the recent tail. Track local delivery/receipt order independently of originating HLC and bind acknowledgement state to the destination.

### S8 — P1: iroh creator-side restart can stop synchronization (source-verified)

`crates/sync-iroh/src/lib.rs:300-312` persists peers learned when joining a ticket, not incoming peers on the ticket creator. Restart only opens a document (`:132-160`), while `sync_once` returns before `start_sync` if the private peer list is empty (`:423-435`). In the pinned iroh-docs version, opening alone does not enable live synchronization.

Thus the original QR-producing device can stop participating after restart while the operation appears successful, until its ticket is shared again. Failed `SyncFinished` events are also counted as completion (`:443-451`). Resume active document synchronization explicitly and validate actual outcomes with restart E2Es.

### S9 — P1: iroh does not persist the selected notebook (source-verified)

Join updates in-memory `state.doc` and peer IDs (`crates/sync-iroh/src/lib.rs:304-312`). Reopen chooses the first stored namespace (`:154-160`), not the last joined one. A device that first creates a QR/notebook and later joins another may reopen the wrong document with stale peer/cursor associations. Persist active namespace identity and namespace-scoped state.

### S10 — P1: valid Greek JSON can become undecryptable across chunk boundaries (reproduced)

`crates/sync-nostr/src/codec.rs:25-30` splits JSON at arbitrary byte boundaries, but `:73-77` decrypts each chunk through the String-returning NIP-44 API. A split within a Greek character fails UTF-8 decoding before the chunks can be reassembled.

The offline codec probe produced five valid self-encrypted chunks, two broken UTF-8 boundaries, and **zero recovered batches**. Sending can still succeed and advance the outbound cursor. Decrypt to bytes, or preserve UTF-8 boundaries deliberately, and test encrypted multichunk Greek content.

### Additional sync/sharing integration issues

- **P2 — Sharing cannot load its normally configured key (reproduced):** `crates/share-nostr/src/identity.rs:69-83` still reads `user_settings.nostr_nsec`, while generation/enabling stores it in `secrets` (`crates/uniffi-bindings/src/lib.rs:1040-1053`; `share-nostr/src/facade.rs:67-74`). The API probe showed sharing enabled, vault populated, but an empty sharing public key. Detection silently avoids enqueuing (`facade.rs:279,293`). Existing identity tests seed the obsolete location.
- **P2 — Additional Android sharing field mismatch:** the adapter requests `pubkeyHex` (`RustCoreAdapter.kt:350-356`), but the generated record exposes `sharePubkeyHex` (`uniffi_bindings.kt:3828-3834`). This is separate from A's suspend-method reflection problem.
- **P2 — Relay payload size/pacing:** a 48 KiB plaintext chunk becomes 65,628 bytes of ciphertext content, exceeding 64 KiB before event metadata (`codec.rs:13,36-54`). Production sends lack the pacing/backoff supplied manually by the live large-batch test (`transport.rs:70-81`; `tests/live.rs:101-124`). Failed publish preserves the cursor, but retries cannot repair a consistently oversized payload.
- **P2 — Credential recovery gap:** generated credentials have no supported user-facing export/recovery path; generation returns only a public key, JSON backups exclude secrets, and platform backup is disabled. Re-generation also replaces the old key (A6). Explicitly imported externally backed-up identities avoid this particular recovery gap.
- **P2 — Error text exported as a backup:** `crates/uniffi-bindings/src/lib.rs:982-986` converts export failures into an `error` JSON string, and Android can save it and announce success (`SyncViewModel.kt:88-96`). Return a typed error rather than backup-shaped failure output.
- **P2 — Network version validation differs from file import:** file import rejects unsupported versions, but Nostr reassembly/pull accepts them (`codec.rs:81-86`; bindings `:1188-1190`). The offline codec probe accepted v2. Apply version/schema validation consistently at every ingress.
- **P2 — Sharing holds the core mutex across network waits:** `crates/share-nostr/src/facade.rs:218-219` retains the shared DB lock through asynchronous flush/read-back/publish. Fixing the earlier identity/adapter defects can expose long blocking of learning operations. Copy inputs under a short lock and apply results afterward.
- **P2 — Sharing bookkeeping:** partially delivered rows remain queued without consuming the configured caps (`ops.rs:302-307`; `caps.rs:124-129`). Content-only read-back can mistake a prior chapter's identical generic preset for delivery of a new milestone (`publisher.rs:273-289`; `templates.rs:111-115`). Test partial delivery alongside multiple queued milestones and repeated template content.

## Desktop readiness

**The desktop is a working Linux shell, but is not ready as a standalone learning product.** It embeds all 1,183 cards and frontend assets, migrates to schema v4, and passed an isolated real-IPC review/restart/persistence smoke after a disposable-copy lockfile refresh.

### Desktop blockers and defects

- **D1 — P1, progression stops after the seven vowel cards (reproduced):** after reviewing them twice, the probe reported mastery 7, chapter 2 still locked, and no next card. Chapter rows are noninteractive (`desktop/src-ui/app.js:292-307`), no unlock command is exported (`desktop/src-tauri/src/lib.rs:87-101`), and the measured practice/alphabet-check alternatives are not wired locally. Add desktop progression and an explicit alphabet completion path.
- **D2 — P1, the installed dictionary is absent (package/runtime-verified):** `desktop/src-tauri/src/state.rs:68-97` checks a user data directory or developer overrides, without installed resource resolution. `tauri.conf.json:21-31` does not bundle the morphology database. The built `.deb` omitted it, and a fresh isolated launch confirmed no database was installed. Queries return `[]` and the UI says “No matches.” Bundle/install the content and distinguish missing content from a failed lookup.
- **D3 — P1, shared restore data loss:** the desktop independently reproduced S1: a Greek lookup review and unlocked chapter disappeared after a successful backup import.
- **D4 — P2, CSS overrides hidden UI state (rendered-browser reproduced):** author `display` rules override default `[hidden]` styling. Rating buttons appear before answer reveal; other probes expose completed cards, new-card badges and an idle spinner. Relevant rules: `desktop/src-ui/style.css:176,191-199,235-258,369-377`; visibility updates: `app.js:57-59,240-247`. The mock harness inspects booleans without CSS. Enforce hidden styling and test actual rendered visibility.
- **D5 — P2, identity ensure fails after first creation (existing test failure):** `desktop/src-tauri/src/commands.rs:358-377` reads the obsolete `user_settings.nostr_nsec` location. The contract fails at `tests/contract.rs:141` with `Query returned no rows`. Desktop also lacks existing-identity import, so independently generating phone/desktop Nostr keys cannot pair their self-encrypted history. Use a shared core accessor and supported identity setup.
- **D6 — P2, desktop also exceeds the daily new-card cap (reproduced):** `desktop/src-tauri/src/commands.rs:75-101` admits 20 new cards against an initial 10-card allowance; Home reports `20 / 20` (`:27-29`). Apply the shared C3 fix and report the actual configured budget.
- **D7 — P2, synchronization reports success despite failed pull (reproduced):** `desktop/src-tauri/src/commands.rs:320-348` hides a pull failure when push succeeds, including an empty push that did not connect. An invalid-relay/empty-push probe returned `viaNostr=true` with zero counts and no error. Preserve per-operation outcomes and errors.
- **D8 — P2, open review batches survive imports unchanged (reproduced):** after importing another device's completed review, reopening Review still offered the cached card as new (`desktop/src-tauri/src/commands.rs:139-145,395-403`; frontend `app.js:596-612`). Reconcile/invalidate sessions after import/sync.
- **D9 — P2, committed lockfile is stale (command-verified):** original `cargo test --release --locked --offline --test contract -- --test-threads=1` fails before compilation. The desktop lockfile's bindings dependencies omit `share-nostr` (`desktop/src-tauri/Cargo.lock:8952-8965`), which current source requires. Refresh and commit the tested lockfile/CLI tooling; scratch build success is not a successful locked build of the original checkout.

Additional hardening: desktop CSP is unset and application command permissions are not explicitly scoped. Current strings are escaped or use `textContent`, frontend assets are local, and no filesystem/shell plugins were identified. Empty capability configuration did not prevent current custom-command IPC; it is not itself a demonstrated startup failure. Cross-platform installers and live phone/desktop release pairing remain unverified.

## Native compatibility and lint

## Native compatibility and lint

All 17 native entries in each existing app APK were inspected. ZIP alignment passed for both APKs, as did `zipalign -c -P 16 -v 4`. Every ARM64 library has at least 16 KB ELF LOAD alignment; ARM64 JNA has 64 KB alignment.

However, `lib/x86_64/libjnidispatch.so` and the 32-bit JNA variants have 4 KB ELF alignment. ZIP alignment alone therefore does not establish all-library 16 KB compatibility. Update/filter the relevant JNA packaging and rerun the check. **This is not a blanket ARM64 alignment failure.**

The inspected release APK is dated October 18 and the debug APK October 20. These observations must not be presented as validation of a freshly built, signed current-source release.

Lint's four errors are three generated `Cleaner` API findings (`uniffi_bindings.kt:1831,1834,1840`) and a `produceState` finding (`SyncScreen.kt:445`). Reflective Cleaner fallback and an explicit state assignment exist, so triage tool configuration/false positives rather than assuming four further runtime bugs.

## Dependency security

An OSV query of the root lockfile's 762 registry package entries identified advisories for multiple versions. Only application-relevant findings should determine release priority; unmaintained, optional, build-only, and unreachable-feature advisories are not all equivalent to exploitable app bugs.

The runtime dependency graph confirms `nostr-relay-pool 0.43.1` -> `nostr-sdk 0.43.0` -> sync/sharing/UniFFI. `Cargo.lock:4411-4412` also pins `nostr 0.43.1`.

Relevant official advisories include:

- [RUSTSEC-2026-0224](https://rustsec.org/advisories/RUSTSEC-2026-0224.html): failed-signature verification cache poisoning; fixed in `nostr-relay-pool 0.44.2`.
- [RUSTSEC-2026-0232](https://rustsec.org/advisories/RUSTSEC-2026-0232.html): processing unverified relay events through known-ID/cache paths; fixed in pool `0.44.3`.
- [RUSTSEC-2026-0231](https://rustsec.org/advisories/RUSTSEC-2026-0231.html): unbounded relay authentication challenge queue; fixed in pool `0.44.3`.
- [RUSTSEC-2026-0216](https://rustsec.org/advisories/RUSTSEC-2026-0216.html): NIP-44 short-buffer panic after a valid conversation-key HMAC; fixed in `nostr 0.44.5`. This specifically requires conversation-key knowledge and is not an arbitrary-relay plaintext-forgery claim.
- [RUSTSEC-2026-0227](https://rustsec.org/advisories/RUSTSEC-2026-0227.html): oversized NIP-44 input allocation/authentication work; fixed in `nostr 0.44.7`.

Application-level reachability was reviewed:

- Sync decoding and sharing read-back do not independently call incoming `Event::verify()`. Verification of stored outgoing sharing events does not protect incoming read-back. NIP-44 authentication still prevents arbitrary plaintext review forgery without the conversation key.
- Sync uses `Client::new` with default automatic authentication (`crates/sync-nostr/src/transport.rs:29-30`), exposing the AUTH-queue issue. Sharing explicitly disables automatic authentication (`publisher.rs:194-195`), which mitigates that issue for the sharing client.
- Sync calls the affected NIP-44 decrypt path. The short-buffer panic still requires a valid conversation-key HMAC.
- SDK defaults cap individual messages at 5 MiB, but the app lacks aggregate fetch/batch limits. This is a partial allocation bound, not a complete application defense.
- Both clients retain default-disabled subscription filter verification; sharing read-back compares content without independently validating author/kind.

**Updating the SDK family remains a release blocker.** Current mitigations are partial. A lockfile advisory match alone does not demonstrate exploitation or a private-key leak; the app-level findings above establish which defenses are actually present.

## Release reproducibility and data

**P1 — Required content has no complete clean-checkout rebuild path.** `data/morphology.sqlite` is ignored (`.gitignore:11-12`); `data/README.md:12-18` documents missing Morpheus XML inputs. All three expected XML files were absent during this review. No pinned acquisition route for the database is documented, and GitHub currently has no release artifacts.

The builder is also unsafe to rerun casually: `tools/data-pipeline/src/main.rs:286-289` deletes the destination if it exists; `:328-332` selects a hard-coded personal checkout path and creates the database before attempting to open input XML (`:347-380`). A run with missing inputs can replace the existing reference asset with an empty database before returning an error. This behavior was source-reviewed, not executed against the real asset.

Archive/pin the usable reference artifact with its checksum, or restore a reproducible input pipeline. Validate all inputs before replacing output, write atomically, and parameterize the output directory.

There is no checked-in CI workflow for release validation. Add repeatable Rust, Android, and desktop gates once the functional failures are fixed, including final native/ABI inspection and a signed release smoke/upgrade/restore drill.

The database is structurally healthy, but 2,055 lemma definitions are blank. The documented Morpheus short-gloss and frequency-proxy limitations remain content-quality constraints; the app should continue to describe its dictionary accurately.

## Evidence locations

- Android checks/probes: `/tmp/opencode/greek-android-audit-0ekpnj8v/`
  - `checks.log`, `release-inspection.log`
  - `keyboard-host-probe.log`, `keyboard-bytecode.txt`, `core-signatures.txt`
  - `apk-native-alignment.txt`, `apk-readelf-loads.txt`, `zipalign-*.log`
- Main Rust test output: `/home/ethan/.local/share/opencode/shell/9b74b2067a04cf158eba1dec2b64b32568a7400a/sh_1590345dc001IN3i67wMTvkqeE.out`
- Core reproduction sources/binaries: `/tmp/opencode/core-readiness-review/{api_probe.rs,core_probe.rs,api_probe,core_probe}`
- Offline sync repros: `/tmp/opencode/greek-sync-review-20261020/` (including `uniffi_repro.rs` and runnable API/core/codec probes)
- Desktop scratch source, package and diagnostics: `/tmp/opencode/greek-desktop-audit-tc06cfqr/`
- Desktop installed-app smoke evidence: `/tmp/opencode/greek-installed-smoke-6ach1p7z/`

## Recommended release gate

1. Repair the deterministic Android crash and input/routing integration failures.
2. Make review/import/migration writes atomic; preserve all legitimate user data in backup/restore; reject malformed input without poisoning the core.
3. Fix scheduling/new-card/practice invariants and add end-to-end regression coverage using actual deck/lookup data.
4. Repair sync ordering, acknowledgement/cursor and restart behavior; update the vulnerable SDK family and verify Unicode multichunk transfers.
5. Finish desktop progression/content packaging if distributing desktop, resolve lint/contract/lockfile failures, and pin the required morphology artifact.
6. Gate a fresh signed release on actual screen navigation, save-failure/retry, upgrade, process-death, full reinstall/restore, and two-device restart/offline/delayed-delivery scenarios. Compare full persisted progress and schedules, not only UUID counts.

## Audit integrity

No application source changes or releases were made by this assessment; this report and isolated diagnostic artifacts are outside the repository. A concurrent edit to `.cargo/config.toml` added developer-cache notes and debug-info settings during the review. It was inspected and left intact; it does not fix the application defects above. The original desktop lockfile was preserved, and desktop build/test changes were confined to a disposable source copy. Temporary morphology WAL/SHM sidecars disappeared when readers closed.
