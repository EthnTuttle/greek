# Workstream: Release infrastructure & reproducibility

Source: production-readiness-2026-10-20.md §"Release reproducibility and data", §"Native compatibility and lint". Verified 2026-10-21 against `ebaaf74`.

## R1 — Pin the reference morphology artifact (P1, blocking)
- **Root cause:** `data/morphology.sqlite` is git-ignored; the 3 Morpheus XML inputs are unobtainable per `data/README.md`. Today the app ships an unreproducible binary asset.
- **Decision needed (cheap):** treat `morphology.sqlite` as a pinned artifact (hash-locked) rather than a built artifact.
- **Fix:**
  1. Record SHA-256 of the current known-good DB (`e97d66b7…` per report §verified) in `data/MANIFEST` with provenance notes.
  2. Publish it as a GitHub Release asset (also fixes the empty-releases download links) or commit via Git LFS.
  3. Android build + desktop packaging (D2) + CI fetch from that one pinned source and verify the hash.
- **Effort:** S. **Risk:** low. **Blocks:** D2, CI release gates.

## R2 — Make the data pipeline safe (P1, follows R1)
- **Root cause (verified):** `tools/data-pipeline/src/main.rs:286-289` deletes the destination DB before opening inputs; hard-coded personal checkout path at ~:328-332.
- **Fix:** validate all inputs first (exist + parse sanity), build to `<out>.tmp`, atomic rename over destination; take input/output dirs from CLI args/env, default to repo `data/inputs` + `data/`; error out listing missing inputs.
- **Effort:** S–M. **Risk:** low. Only needed if the pipeline is ever run again — R1 removes it from the critical path.

## R3 — ABI filtering + JNA alignment (P1, Android packaging; closes report A5)
- **Root cause (verified):** `android/app/build.gradle.kts` declares no `abiFilters` — grep finds no ndk/abi config — so dependency `.so`s land for armeabi-v7a/x86 while the Rust lib exists only for arm64-v8a/x86_64. Also x86_64 `libjnidispatch.so` is 4 KB ELF-aligned.
- **Fix:**
  1. `defaultConfig { ndk { abiFilters += listOf("arm64-v8a", "x86_64") } }`.
  2. Update the JNA artifact to a 16 KB-aligned release (or drop x86_64 JNA if unused on desktop-class emulators) — check AAR for `jnidispatch` version, bump, and re-run the APK inspection scripts in the report's evidence dir.
  3. Add a release-check task asserting APK `lib/**` matches {arm64-v8a,x86_64} only and all LOAD alignments ≥ 16 KB.
- **Effort:** S–M. **Risk:** low (narrows install base deliberately). Independent of all code fixes; can be done today.

## R4 — CI release gates (P1 process item)
No `.github/workflows` exists. Minimum gate set, added only after functional P1s start landing:
1. `cargo test --workspace --locked` (existing 171 tests).
2. `cargo test -p greek-desktop --locked --offline --test contract` (catches D9-class rot).
3. Android `:app:testDebugUnitTest` + `:app:polytonicAudit` + `:app:lintDebug` with the 4 known findings allowlisted/baselined until triaged (3 generated `Cleaner` + 1 `produceState`).
4. Native/ABI inspection script (from R3.3) on the release APK.
5. Desktop `.deb` build + existing installed-smoke script.
- **Effort:** M. **Risk:** none. Sequence after the first wave of crash fixes so CI isn't permanently red.

## R5 — Dependency security: nostr SDK family upgrade (P1, owned by ws-sync)
- `nostr-relay-pool 0.43.1 → ≥0.44.3`, `nostr → ≥0.44.7` (RUSTSEC-2026-0224/-0232/-0231/-0216/-0227). API churn likely in client/auth/pool layers touching `sync-nostr` and `share-nostr`; do the upgrade as the **first** item in ws-sync so S6/S7/S8/S10 fixes are written against the new API, not rewritten. Also add the app-level mitigations from the report: aggregate fetch/batch limits, and share read-back author/kind validation.

## Order & integration
1. **R3 ABI filter** — 30-minute change, independent, ship it early.
2. **R1 artifact pinning** — publish release asset + MANIFEST hash.
3. **R5 SDK upgrade** (in ws-sync sequence).
4. **R4 CI** after first crash-fix wave.
5. **R2 pipeline hardening** whenever convenient; not on release critical path once R1 is done.

Cross-workstream handoffs: R1→D2 (desktop packaging), R3→A5 (represents its completion; Android agent plan should reference, not duplicate), R5→ws-sync.
