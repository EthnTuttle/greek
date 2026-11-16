# Deferred follow-ups

Small items discovered during wave execution that were out of agent scope at the
time. Groom these into the next applicable wave.

## From Wave 2 (desktop D5/D7)
1. ~~Add a core-side `sync_nostr_pubkey()` export~~ — DONE (nostr-SDK-upgrade
   wave): `GreekCore::sync_nostr_pubkey()` returns the identity pubkey hex via
   `nostr_keys` → `load_nostr_nsec` (same migration-aware path as push/pull);
   desktop can now drop its `nostr-sdk` derive.
2. ~~Legacy `user_settings.nostr_nsec` migration edge~~ — DONE: getters route
   through `load_nostr_nsec` (`sync_nostr_config().has_identity` and
   `sync_nostr_pubkey()` both trigger the legacy→vault migration on read), and
   the share-nostr ensure/load path (`ShareKeys::from_db` +
   `facade::ensure_sync_identity`) now migrates a pre-v4 legacy key into the
   vault BEFORE the presence check, so `enable` reuses it instead of shadowing
   it with a fresh identity.

## From Fable close-out (non-blocking polish)
11. **Headword-normalization residue:** principal-parts lists vary formatting so
    ~19 repeated lists are not deduped; spelling variants `οὕτω(ς)`/`οὕτως` and
    `ζάω` versus `ζάω/ζῶ` likewise.
12. **Chapter display totals use raw card counts**, not the dedupe-aware
    admissible subset (a slow-schedule completed chapter may read 6/21
    indefinitely). Surface admissible counts if worth polishing.
13. **Reader dictionary missing entries:** σύ resolves to a wrong homograph,
    ὑμῖν has no row. Ship article/pronoun/Attic -ττ-/deponent overrides with a
    future content pass.

## From Wave 4 (S8/S9)
10. **iroh content-vs-restart edge (pre-existing upstream behavior):** if a node
    restarts after entry metadata replicated but before blob content finished
    downloading, pending downloads aren't resumed (runtime-only in iroh-docs
    0.101) and no new `InsertRemote` refires on restart. Content is eventually
    reconciled on the next live event; assess whether manual polling on open
    is worth it. Low priority.

## From Wave 3 (S11)
7. **Desktop: switch `nostr_identity_ensure` to the new `sync_nostr_pubkey()` export
   and drop desktop's `nostr-sdk` dep** (it now only supports `Keys::parse` for
   deriving the pubkey — core exports it directly). Update the Cargo.toml comment.
8. **Verify the optional advisory mapping in CI/release notes:** sdk stayed 0.44.1
   (0.44.3 of the *sdk* doesn't exist; the advisories pin nostr-relay-pool ≥0.44.3 ✓
   and nostr ≥0.44.7 ✓ — both landed). Confirm OSV clean on next audit pass.
9. **WAVE9-WIRE cleanup:** the remaining reflective share seams in
   `RustCoreAdapter` (`shareEnable`, `shareRelaysSet`, `shareMilestones`, …) still
   work post-regen and degrade safely on stale `.so`s, but can move to direct
   compile-checked calls whenever convenient. Low priority.

## From Wave 3 (C4/C5/C12)
6. **Share-hook gap:** organic alphabet-gate self-latch now happens inside core's
   `alphabet_gate_state` (every reader latches), so `share_hook_gate_latched`
   only fires on the explicit check-passed binding — decide whether organic
   unlocks should announce too, and wire it in the share region if so.
   → Owner: whoever next touches `crates/share-nostr` (S11 or Wave-4 sharing audit).

## From Wave 2 (C3)
5. ~~Regenerate checked-in UniFFI Kotlin bindings + direct call in RustCoreAdapter~~ — DONE (Wave-2 closeout): bindings regenerated from `libuniffi_bindings.so` (checksums updated for C2/C3/S1 changes), adapter calls directly with a stale-.so-safe fallback, `:app:compileDebugKotlin` clean.

## From Wave 1
3. ~~PracticeScreen retry affordance for `verdictSaveFailed`~~ — DONE (folded into
   the Wave-2 Android agent's scope).
4. ~~S6 must renumber its `sync_applied_batches` table from schema v5 to v6~~ —
   HANDLED: the S6 agent prompt specifies v6 directly.

## Release-gate manual items accumulating (Android, need a device)
1. **A6 rotation flow**: with identity configured, button reads "Rotate identity"; Cancel/outside-tap must not change identity; confirm shows new npub.
2. **A6 dialog copy**: the no-export limitation sentence is present until the credential-recovery P2 ships.
3. Dialog survives device rotation (state in VM).
4. **Backup export + rotation**: SAF picker open, rotate, pick file → full JSON written + "Backup written" snackbar.
5. **Export-failure path** (stub core returning `{"error":…}`): picker must NOT open; invalid-backup snackbar shows.
6. **Practice retry**: offline core → verdict screen shows error + "Retry saving result"; answer-save failure shows banner + "Retry saving answers (N pending)".

## Release-gate manual items accumulating (desktop)
1. **D2 `.deb` contents**: after `cargo tauri build`, run `dpkg -c <pkg>.deb | grep morphology.sqlite` — the bundled lexicon must be listed (staging script `scripts/desktop-stage-morphology.sh` fails the build beforehand, but inspect the artifact anyway); then a fresh-profile launch → Dictionary search for "λόγος" returns rows, and a profile with an unbundled install shows "Dictionary not installed".
