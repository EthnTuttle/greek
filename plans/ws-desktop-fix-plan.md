# Workstream: Desktop P1 fixes (and coupled P2s)

Source: production-readiness-2026-10-20.md, verified 2026-10-21 against `ebaaf74`.
Desktop is a Tauri v2 Linux app (`desktop/src-tauri`, Rust) + vanilla JS UI (`desktop/src-ui`) wrapping `GreekCore` from `crates/uniffi-bindings`.

## D2 — Bundle morphology dictionary (P1)
- **Root cause (verified):** `desktop/src-tauri/tauri.conf.json` `bundle` block has no `resources`; `desktop/src-tauri/src/state.rs:68-97` only resolves `GREEK_MORPH_PATH` env, `<data_dir>/morphology.sqlite`, or the `GREEK_REPO_ASSETS` dev override. Built `.deb` contains no DB; fresh install shows empty dictionary.
- **Fix:**
  1. Copy the pinned dictionary (see ws-release plan for the artifact pinning) to `desktop/src-tauri/resources/morphology.sqlite` at build time (or check in via Git LFS / download step in CI).
  2. Add to `tauri.conf.json` bundle: `"resources": ["resources/morphology.sqlite"]`.
  3. In `state.rs`, resolve `path_resolver().resource_dir()/morphology.sqlite` before falling back to data dir; when absent, surface "dictionary not installed" (new typed state) so UI can distinguish missing-content from no-match (`dict_search` currently just returns `[]`; frontend message lives in app.js dict panel).
- **Files:** `desktop/src-tauri/tauri.conf.json`, `desktop/src-tauri/src/state.rs`, `desktop/src-tauri/build.rs` (or CI copy step), `desktop/src-ui/app.js` (missing-content message).
- **Test:** extend the existing installed-smoke flow: fresh-profile launch of the built `.deb`, `dict_search("λόγος", 5)` returns rows; absence case shows "not installed".
- **Effort:** M. **Risk:** low (packaging path only). **Depends on:** ws-release R1 (pinned artifact source).
- **DONE (2026-10-27):** `scripts/desktop-stage-morphology.sh` copies `data/morphology.sqlite` → `desktop/src-tauri/resources/morphology.sqlite`, verifying the SHA-256 pinned in `data/MANIFEST` before AND after the copy (fails loudly, with remediation text, on missing/mismatch). Wired as `beforeBuildCommand` in `tauri.conf.json` so every `tauri build` stages + verifies; `"resources": ["resources/morphology.sqlite"]` added under `bundle`, and `desktop/src-tauri/.gitignore` now ignores `/resources` (binary never committed). `state.rs` resolution order is now: `GREEK_MORPH_PATH` env → `<resource_dir>/morphology.sqlite` (passed in from lib.rs's `Builder::setup` via `app.path().resource_dir()`, before `.manage`) → `<data_dir>/morphology.sqlite` (with the old `GREEK_REPO_ASSETS` dev copy-in last). New `dict_status` command returns `{"installed":bool,"path":...}`; app.js calls it on dictionary-tab load and renders "Dictionary not installed" instead of "No matches" when the lexicon is absent. Tests: 3 `state.rs` unit tests cover resolution order against scratch dirs; new `dict_status_reports_installation` contract test (12/12 contract green); `dev/test-ui.mjs` covers installed/not-installed/tab-switch paths (75/75). **Release-gate note:** after `cargo tauri build`, verify the artifact with `dpkg -c <pkg>.deb | grep morphology.sqlite` (must list one entry), then fresh-profile launch → search "λόγος" shows rows; the beforeBuildCommand fails the build before bundling if the pinned artifact is missing or corrupted.

## D1 — Progression dead-ends after 7 vowel cards (P1)
- **Root cause (verified):** `desktop/src-tauri/src/lib.rs:87-101` `invoke_handler!` exports review/home/dict/sync commands only — no chapter-unlock, no practice, no alphabet-check command. `desktop/src-ui/app.js:267-320` chapter rows are render-only (`<div class="chapter-row">`, no click handler).
- **Fix (minimal viable progression):**
  1. Export the existing binding fns as Tauri commands: `alphabet_check_*` (the designed completion path) and `curriculum_mark_chapters_up_to` (or equivalent unlock binding used by Android — mirror its semantics exactly to avoid inventing desktop-only policy).
  2. Make chapter rows buttons: when the binding reports the alphabet gate satisfied, allow tapping to unlock; keep locked-state hints otherwise.
  3. Longer-term desktop practice is out of scope for the release gate.
- **Files:** `desktop/src-tauri/src/commands.rs`, `desktop/src-tauri/src/lib.rs`, `desktop/src-ui/app.js`, `desktop/src-ui/style.css` (interactive states).
- **Test:** contract test (fresh profile → complete alphabet check via real IPC → Letters ch.2+ unlockable → next review session yields non-vowel cards). This replaces the repro probe in the report.
- **Effort:** M–L. **Risk:** medium (new IPC surface; must not diverge from Android progression semantics). **Depends on:** ws-core C12 (organic-completion latch) if desktop reuses the gate; otherwise independent. Must not implement a desktop-local gate bypass — that would repeat C4.

## D3 — Shared restore data loss (P1)
- Not desktop-local: desktop reproduces S1 because it calls the same `backup_import` binding.
- **Fix:** covered entirely by ws-sync S1 (extended SyncBatch schema + ID validation accepting e.g. `lookup-λόγος`). No desktop code changes beyond picking up the fixed bindings.
- **Action here:** after S1 lands, add desktop regression to `tests/contract.rs`: restore a backup containing a Greek lookup review + unlocked chapter into a fresh profile; assert both survive.
- **Effort:** S (test only). **Depends on:** S1.

## Coupled P2s worth bundling
- **D9 — stale lockfile (S):** `desktop/src-tauri/Cargo.lock` bindings entry omits `share-nostr`. Fix = refresh lockfile in-tree and commit; gate CI on `cargo test --locked --offline -p greek-desktop --test contract`. Do this **first** — every other desktop change needs a buildable locked checkout.
- **D6 — 20/10 new-card cap (S):** `commands.rs:75-101` fills `SESSION_SIZE` across chapters while each `get_new_cards` re-evaluates the same allowance. **Do not invent a desktop-local budget** — the fix is the shared core admission budget from ws-core C3; desktop just stops passing a per-call limit and reports the configured cap on Home (`commands.rs:27-29`). Depends on C3.
- **D7 — sync_now masks pull failure (S):** `commands.rs:320-348` returns `viaNostr=true` on push success even when pull failed. Fix = structured result `{push: Result, pull: Result}` + UI error display. Independent.
- **D5 — identity ensure reads obsolete key location (S):** `commands.rs:358-377` reads `user_settings.nostr_nsec`; real location is the `secrets` table. Fix = delegate to the shared accessor used by share-nostr (ws-sync SDK/identity work will touch this; coordinate). Existing failing test `tests/contract.rs:141` passes when fixed.
- **D8 — open review batch survives import (M):** invalidate the in-memory `ReviewSession` on `backup_import`/sync-now success. Small, but add the test while fixing.
- **D4 — `[hidden]` overridden by author CSS (S):** in `style.css` add `[hidden]{display:none!important}` ahead of component display rules (lines 176,191-199,235-258,369-377); make the mock harness assert computed style or switch assertions to `aria-hidden`/class-based. Independent.

## Recommended order
1. **D9 lockfile refresh** — unblocks all desktop verification. (S)
2. **D2 resources + resolution** (M) — needs R1 artifact source decided; can start with the current data/morphology.sqlite.
3. **D5 shared identity accessor** (S) — flips an already-failing test green.
4. **D7 structured sync result** (S).
5. **D1 alphabet-check + unlock commands, interactive rows** (M–L) — the one real feature build.
6. **D8 session invalidation** (S, ride along with D5/D7 file touches).
7. **D4 CSS hidden fix** (S).
8. **D6** once ws-core C3 lands; **D3 regression test** once S1 lands.

Parallelizable: {2,4,7} and {3,6} are independent of each other; 5 is the only L-item and owns app.js's curriculum section (others touch different regions — minor merge care with 7).
