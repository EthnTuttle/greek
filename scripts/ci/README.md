# Local verification (`scripts/ci`)

The owner deliberately removed the GitHub Actions workflows; this repository
does not use GitHub CI. The equivalent release gates run locally through
`scripts/ci/run-local.sh`, with nontrivial checks kept in small, testable
scripts.

## run-local.sh

From the repository root, run the full local gate:

```sh
./scripts/ci/run-local.sh
```

This runs the Rust workspace tests, desktop contract tests and UI harness,
Android unit tests/polytonic audit/lint triage, and pinned-artifact hash check.
It prints a `PASS`/`FAIL` verdict for each section and a final `RESULT` line;
the process exits non-zero if any selected check fails. It does not clean build
outputs or install prerequisites. In particular, the desktop contract test
needs the Tauri Linux development libraries and Android needs JDK 17.

For faster iteration:

```sh
./scripts/ci/run-local.sh --quick           # everything except Android
./scripts/ci/run-local.sh --only rust       # workspace tests only
./scripts/ci/run-local.sh --only desktop    # contract tests + UI harness
./scripts/ci/run-local.sh --only android    # unit tests + audit + lint
```

`--only` also skips the artifact-pin check. `--quick` cannot be combined with
`--only android`.

### Release APK ABI check (`--release-check`)

After building a release APK, add `--release-check` to any selection:

```sh
(cd android && ./gradlew :app:assembleRelease)
./scripts/ci/run-local.sh --quick --release-check
```

The runner finds an existing APK under
`android/app/build/outputs/apk/release/` and asserts that `lib/` contains both
`arm64-v8a` and `x86_64`, and no other ABI. Because the keystore is git-ignored,
the usual local precursor produces an unsigned APK; ABI filtering is the same
before and after signing. The check fails with build instructions if no APK is
present—it never auto-assembles one. Use the full command without `--quick`
when the complete local gate plus the release check is desired.

## lint-triage.sh

`run-local.sh` invokes this script for the Android leg. It wraps
`./gradlew :app:lintDebug` and parses
`android/app/build/reports/lint-results-debug.xml`, allowing exactly the 4
lint errors the 2026-10-20 production-readiness audit triaged:

- 3× `NewApi` about `java.lang.ref.Cleaner` in generated
  `uniffi_bindings.kt` (uniffi-bindgen output; reflection-guarded by design)
- 1× `ProduceState*` in `SyncScreen.kt` (audited false positive)

Any other Error/Fatal-severity finding fails the local gate, so new lint
regressions are still caught. Signatures match issue ID + file, not line
numbers (audit line refs have already drifted; also why no lint baseline file:
a baseline would freeze the 57 warnings too). The script resolves the repository
root itself and can also be run directly from any directory.

## Former job → audit-gate mapping

The CI job names are retained below as audit references; they now correspond to
sections of the local runner rather than GitHub Actions jobs.

| Former CI job / local section | Gate enforced (production-readiness-2026-10-20.md item) |
|---|---|
| `rust-test` / `Rust workspace tests` | General C-series/S-series regression net: `cargo test --workspace --locked` (171 host tests at audit time); live-network probes stay `#[ignore]`d |
| `desktop-contract` / `Desktop contract tests` | D-series shape gates: frozen desktop command JSON contracts, D9 lockfile freshness (`--locked` on the separate desktop lockfile), D5 identity access via the secrets vault |
| `Desktop UI harness` | Headless all-screen/error-path smoke and frozen invoke-contract coverage via `node dev/test-ui.mjs` from `desktop/src-ui` |
| `android` unit tests / `Android unit tests and polytonic audit` | A-series regression net at JVM level (`:app:testDebugUnitTest`, 27 tests as of audit) |
| `android` polytonicAudit / same Android section | DESIGN-greek-keyboard.md Appendix R — every deck/morphology Greek codepoint must be keyboard-emittable |
| `android` lint triage / `Android lint triage` | "Native compatibility and lint" — known-4 whitelisted, everything else fails |
| `artifact-check` / `Pinned artifact hashes` | "Release reproducibility and data" P1 — `data/MANIFEST` must be readable by `sha256sum`; any pinned artifact present must hash-match (missing artifacts are ignored) |
| `abi-check` / `Release APK ABI check` | A5 — existing release APK `lib/` restricted to exactly `arm64-v8a` + `x86_64`, with both present |

## Not wired into the local runner (deliberately)

- `scripts/e2e/e2e.sh` — device targets need physical bench hardware; the
  `--selfcheck` host↔host mode still compiles and runs the real iroh/nostr
  stack (heavy, and the flows assume operator-provided timing budgets tuned
  for the bench). Kept manual/pre-release; revisit if it is ever hardened
  into a deterministic selfcheck.
- APK assembly and signing — keystore and signing material stay local and
  git-ignored. Build explicitly with `:app:assembleRelease` before
  `--release-check`; the runner only inspects the resulting APK.
