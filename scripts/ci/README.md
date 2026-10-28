# scripts/ci

CI glue for `.github/workflows/`. Anything nontrivial that a workflow job
needs lives here as a small script so the YAML stays declarative and the
logic stays testable locally.

## lint-triage.sh

Wraps `./gradlew :app:lintDebug` and parses
`android/app/build/reports/lint-results-debug.xml`, allowing exactly the 4
lint errors the 2026-10-20 production-readiness audit triaged:

- 3× `NewApi` about `java.lang.ref.Cleaner` in generated
  `uniffi_bindings.kt` (uniffi-bindgen output; reflection-guarded by design)
- 1× `ProduceState*` in `SyncScreen.kt` (audited false positive)

Any other Error/Fatal-severity finding fails the job, so new lint
regressions still break CI. Signatures match issue ID + file, not line
numbers (audit line refs have already drifted; also why no lint baseline
file: a baseline would freeze the 57 warnings too).

## Job → audit-gate mapping

| CI job | Gate enforced (production-readiness-2026-10-20.md item) |
|---|---|
| `rust-test` (ci.yml) | General C-series/S-series regression net: `cargo test --workspace --locked` (171 host tests); live-network probes stay `#[ignore]`d |
| `desktop-contract` (ci.yml) | D-series shape gates: frozen desktop command JSON contracts, D9 lockfile freshness (`--locked` on the separate desktop lockfile), D5 identity access via the secrets vault |
| `android` unit tests (ci.yml) | A-series regression net at JVM level (`:app:testDebugUnitTest`, 27 tests as of audit) |
| `android` polytonicAudit (ci.yml) | DESIGN-greek-keyboard.md Appendix R — every deck/morphology Greek codepoint must be keyboard-emittable |
| `android` lint triage (ci.yml) | "Native compatibility and lint" — known-4 whitelisted, everything else fails |
| `artifact-check` (ci.yml) | "Release reproducibility and data" P1 — `data/MANIFEST` well-formed; any pinned artifact present must hash-match (the git-ignored DB itself is absent in CI by design) |
| `abi-check` (release-check.yml, manual) | A5 — released APK `lib/` restricted to `arm64-v8a` + `x86_64` |

## Not wired into CI (deliberately)

- `scripts/e2e/e2e.sh` — device targets need physical bench hardware; the
  `--selfcheck` host↔host mode still compiles and runs the real iroh/nostr
  stack (heavy, and the flows assume operator-provided timing budgets tuned
  for the bench). Kept manual/pre-release; revisit if it is ever hardened
  into a deterministic selfcheck.
- Signed-APK verification — keystore is git-ignored; release-check.yml
  verifies the unsigned release APK (abiFilters apply regardless of signing).
