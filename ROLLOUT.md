# Rollout & Release Plan — Greek Learning App

- **Status:** plan (no code changes; signing/CI changes specified as tasks only)
- **Date:** 2026-10-18 · **HEAD audited:** `b3425a4` (Waves 1–5 verified, 6a complete)
- **Author:** rollout subagent (Agent P4)
- **Scope:** release channels, versioning, phase gates P0→P3, security gates, ops runbook, risks

## 1. TL;DR

**"Launch" means:** (1) signed universal APK on **zapstore** (nostr-native, primary — tooling `zsp`, see §2.4 + ZAPSTORE.md) with GitHub Releases as asset host, (2) F-Droid inclusion later (real effort — see §2), (3) Play Store closed-track first, open only after sync-beta stabilizes. Current APK is **release-signed** (own key, cert SHA-256 `8802681e…`, gitignored `android/keystore/`), `versionCode 2 / versionName "0.1.0"`, ~92 MB release with the 113 MB morphology asset, device-verified on Pixel 6 / Android 17. No CI exists (`.github/` absent at HEAD — spec only in §4 P2).

Phase spine: **P0 internal soak → P1 private beta (3–5 users) → P2 public beta (GitHub tags + community posts) → P3 stable (Play open).** Every phase has exit criteria and a rollback lever (§4). Two security gates (**nsec secret storage**, **DocTicket/relay policy**) are MUST-FIX before P2 public (§5).

## 2. Release channels

| Channel | Status today | Gating work | Verdict |
|---|---|---|---|
| Direct sideload (adb) | ✅ works now (debug-signed APK) | none | Internal/P0 only |
| **Zapstore (nostr app store)** | ⏳ **channeled & ready**: `zapstore.yaml` committed at repo root (pubkey field awaits owner's npub), `zsp` 0.4.17 validated end-to-end offline against a real APK (events parse version/package/cert correctly) | Owner fills `pubkey:` in zapstore.yaml + pushes to GitHub; runs one `zsp publish` with the chosen nostr key (see ZAPSTORE.md) | **Primary public channel** — changelog §2.4 |
| GitHub Releases (signed APK) | ✅ release signing key generated (`android/keystore/`, gitignored), `signingConfigs` wired; universal `app-release.apk` builds signed (cert SHA-256 `8802681e…`) | Tag-push CI (§7) optional; manual attach fine | Asset host for zapstore + fallback downloads — P1 |
| F-Droid | ❌ not submitted | Metadata + reproducible-build proof; blocked on Rust/NDK pinning | **Defer until after P3** |
| Play Store | ❌ no account, assets half-ready | Feature graphic + screenshots + privacy URL + data-safety (drafts below); 20-tester closed-track rule | **After ≥2 public APK releases** |

### 2.1 GitHub Releases (primary)

**Required gradle change (TASK — do not hand-edit without owner):** `android/app/build.gradle.kts` currently has **no `signingConfigs`** and the `release` buildType (lines 24–31) would emit an **unsigned** APK. Spec:

```kotlin
val keyProps = java.util.Properties()
rootProject.file("key.properties").takeIf { it.exists() }?.inputStream()?.use(keyProps::load)
if (keyProps.containsKey("storeFile")) {
    signingConfigs {
        create("release") {
            storeFile = rootProject.file(keyProps["storeFile"] as String)
            storePassword = keyProps["storePassword"] as String
            keyAlias = keyProps["keyAlias"] as String
            keyPassword = keyProps["keyPassword"] as String
        }
    }
    buildTypes.named("release") { signingConfig = signingConfigs.getByName("release") }
}
```

Key generation (one-time, on owner's machine, offline):

```sh
keytool -genkeypair -v -keystore greek-release.jks -alias greek \
  -keyalg RSA -keysize 4096 -sigalg SHA256withRSA -validity 10950
# key.properties (gitignored) next to it:
# storeFile=greek-release.jks / storePassword=… / keyAlias=greek / keyPassword=…
```

`.gitignore` today ignores `*.apk` but **not** `key.properties`/`*.jks` — add both before the key lands on disk (TASK R-1). jks backup lives in the owner's password manager + offline vault; losing it permanently orphans the update path under this applicationId (`com.greekapp`) — Play/App-signing note in §8.

Verify every release: `apksigner verify --print-certs app-release.apk` — cert DN must **not** be `Android Debug` (debug APK verified 2026-10-18 to carry exactly that DN).

VersionCode rule: **monotonic, never reused, current max = 2** (`versionName "0.1.0"` — the zapstore-first build). Next beta = `versionCode 3, versionName 0.2.0`.

### 2.2 F-Droid audit (deferred)

- **Deps are FOSS-compatible**: Rust — fsrs (MIT), rusqlite+bundled sqlite (MIT/public domain), serde/uuid/thiserror (MIT/Apache), uniffi 0.28 (MPL-2.0), nostr-sdk 0.43 (MIT), hmac/sha2/hex (MIT); Android — androidx (material/compose/navigation/datastore) all Apache-2.0, JNA 5.14.0 Apache-2.0, and the gradle `packaging.excludes` on `META-INF/{AL2.0,LGPL2.1}` only strips metadata, fine. Wave-6b additions (zxing-core Apache-2.0 — **use core-only, not play-services-mlkit**; iroh Rust shim MIT/Apache) stay clean. **No Firebase/GMS/tracking deps exist.** ✅
- **Known pain**: F-Droid metadata needs pinned Rust toolchain (currently nix `flake.nix` + rust 1.93.1 per spike) **and** pinned NDK (`27.0.12077973`, clang26 per `SPIKE-IROH.md` Part C). `fdroidserver` supports `srclibs`/`prebuild` rust builds but reproducible builds across the workspace on the F-Droid build farm is unproven — expect 1–2 weeks of iteration. **Decision needed (§9):** inclusion priority vs. effort. Recommend: files `metadata/com.greekapp.yml` after P2, accept non-reproducible ("the app is built from source by F-Droid, apk signed by F-Droid") model, not upstream-signed RB, unless a maintainer volunteers.

### 2.3 Play Store (after P2, before P3-open)

| Requirement | State | Action |
|---|---|---|
| 512×512 icon | ✅ `art/ic_launcher-playstore.png` (verified 512×512 PNG) | none |
| Feature graphic 1024×500 | ❌ missing (`art/` has icon-master 2048² only) | TASK R-2: derive from `icon-master.png` + title strip via `art/prompts.md` style |
| ≥2 phone screenshots | ❌ none in repo | TASK R-3: capture on Pixel 6 (Home, Review, Reader, Curriculum) |
| Privacy policy URL | ❌ no page yet | docs site agent builds `privacy.html` at root of GitHub Pages `/docs` site; URL must be live before submission |
| Data safety form | draft below | fill at submission |
| 16 KB page-size compat (targetSdk 35) | ❌ **FAILED today** | TASK R-4, see §8 risk row |
| Closed testing, 20 testers × 14 days | — | Owner must recruit testers post-Nov-2023 personal accounts; budget real time |
| $25 + identity disclosure | — | decision §9 |

**Data-safety form (draft answers):** *Collects or shares user data?* — While the build has no `INTERNET` permission (verified: manifest declares only `POST_NOTIFICATIONS`), the honest current answer is **"No data is collected or shared"** for the no-sync build. Once sync ships: *App activity → other actions (review history, settings)* — Collected: **Yes**; Shared: **No**; Optional: **Yes** (sync disabled by default); Purpose: App functionality; Encrypted in transit: **Yes** (NIP-44 E2E + TLS); User can request deletion: **Yes** (local; relay events are user-owned ciphertext, deletable via NIP-09 on user-configured relays). *Device or other IDs*: the random `device_id` UUID — same answers, disclose. Keep both versions saved for re-submission.

Update 2026-10-18: `INTERNET` + `ACCESS_NETWORK_STATE` are now declared (sync transports need sockets — caught pre-launch by this document's audit); the "sync ships" column above is the live one.

### 2.4 Zapstore (nostr-native; chosen primary)

Full runbook in [ZAPSTORE.md](ZAPSTORE.md). Key facts (verified 2026-10-18): publishing tool is `zsp` 0.4.17 (the 2025-era Dart `zapstore-cli` flow is dead); events = kind 32267 app / 30063 release / 3063 asset; zapstore indexes from its own relay (`wss://relay.zapstore.dev`), and whitelists the publishing npub by fetching the committed `zapstore.yaml` from the GitHub repo — **order matters: push yaml to GitHub first, then run one command** (`SIGN_WITH=<key> zsp publish -y zapstore.yaml`). APK cert = update identity (back up `android/keystore/` offline — losing it means users must reinstall); nostr key = listing ownership, linked once via `zsp identity --link-key`. APK/media hosting via Blossom (`cdn.zapstore.dev`; `BLOSSOM_URL` self-host escape hatch). A validated `zsp` binary is staged at `/tmp/opencode/zapstore-setup/zsp-0.4.17-linux-amd64` — move it onto `$PATH` and delete the copy after first publish.

## 3. Versioning policy

- **App-facing semver with a `0.9.x` beta line.** Gradle today says `versionName "1.0" / versionCode 1` — a **deception** (feature-incomplete); first public beta is **`0.9.0` / code `2`**. Lead-up internal builds may use codes `1+beta index`.
- **`1.0` criteria** (all must hold): P3 gates green 30 days (crash-free, sync incident-free), LSJ-status resolved (real LSJ or permanently honest Morpheus labeling incl. store listing), backup/restore drill passes from GitHub-issued build, docs site live.
- **versionCode monotonic forever.** Never reset, never fork numbering per channel; beta/staging share one line (code = simple increment).
- **Schema promise.** Migrations v1→v2→v3 run via `PRAGMA user_version` (`crates/core/src/db/mod.rs:104-186`) with proven upgrade tests (`test_migrate_v1_to_v2_adds_source_and_preserves_rows`, idempotency test). **Rule for all future schema bumps: ship ALTERs + an upgrade test that builds the N-1 shape by hand and upgrades it** — same pattern, no exceptions. Downgrades are unsupported; pre-update backup via transport-zero export (§11 of DESIGN-device-sync) is the user's parachute once it lands in Wave 6c.
- **Morphology asset versioning is already live**: Kotlin-side `MORPHOLOGY_DB_VERSION = 2` + marker file re-copy guard (`GreekApplication.kt:127-172`). Rule: **every morphology.sqlite change bumps that constant** — it is the only re-copy trigger; the old "file exists" check was the stale-asset bug ANALYSIS.md §2 flagged (fixed by Agent E in Wave 1).
- **FSRS params**: app folds history with default params (`FSRS::new(Some(&[]))`, `sync/mod.rs:262`). `Cargo.toml` floats `fsrs = "2"`. **Pin `=2.0.4`** and record `user_settings['fsrs_params_version']` before each param/model bump (DESIGN-device-sync.md §14); a fold is parameter-dependent so silent drift = cross-device divergence. TASK R-5.

## 4. Phases & gates

### P0 — Internal (NOW; cost: none)

Owner daily-drives the debug APK on the Pixel 6; nightly two-device nostr round-trip against the owner's Pyramid relay (`wss://chat.virginiafreedom.tech`, v1.2.12, `restricted_writes=true`, NIP-42, **member-invite only** — usable only after admin invites the owner's sync npub).

- [ ] Pyramid admin: invite/member-add the sync npub (relay admin UI) — **do NOT** attempt writes before membership; NIP-42 will auth-reject and the client treats that as relay failure
- [ ] Soak script (spec — `crates/cli` bin `sync-soak` or `#[ignore]` nightly test): two in-process `greek_core::Database` instances ("deviceA", "deviceB"); seeded PRNG; per simulated day each device steps `N∈[0,25]` random ratings from its own due queue via the real submit path, then both push/fetch via `NostrSpoolTransport` to the Pyramid relay; every 7 simulated days assert convergence: identical `COUNT(*)`/ordered-hash of `review_logs`, `settings_sync`, and all `memory_states` rows. 30 simulated days. Runs headless on the gitea box (cron + `nix develop -c cargo run -p greek-cli --release --bin sync-soak`), result appended to a log file; alert = nonzero exit
- [ ] Crash-free owner use: no Rust panics / `GreekApplication` init errors for 14 consecutive days (logcat kept per device-check protocol)
- [ ] Backup/restore drill (once transport-zero lands, design §11):
  1. Settings → Export → `greek-backup-<date>.json` (degenerate SyncBatch: full log + settings)
  2. Snapshot: due-queue order + streak + counts via the §7 forensic queries
  3. `adb uninstall com.greekapp`; reinstall same build; import JSON
  4. Assert: due queue identical, streak identical, `review_logs` count identical (uuid-stable)

**Exit criteria:** 30-day soak converges every checkpoint; zero crash blockers; drill passes. **Rollback:** disable sync UI; ship P1 as no-network APK.

### P1 — Private beta (3–5 trusted users; family / classics Discord)

Signed APK via GitHub Releases (`v0.9.betaN` tags, **pre-release** flag). Feedback channel: single GitHub Discussions post + direct contact. Beta testers syncing = Pyramid members (owner invites their npubs) **or** fully local (no sync).

**Exit criteria:** zero data-loss reports; one pairing session **watched** where a tester pairs two devices unaided and correctly predicts what synced. **Rollback:** pull the release (`gh release delete`) — nothing else depends on it.

### P2 — Public beta

GitHub Releases tagged `v0.9.x` non-prerelease; docs site live (`/docs` → Pages); posts:

> **[r/ancientgreek + r/Greek, ~200 words — draft]** *Title: I built an offline Ancient Greek app (Athenaze / JACT / DCC vocab, FSRS spaced repetition, tap-to-parse reader) — public beta APK.* Body: Beta-quality, 1,140 cards across Athenaze I/II, JACT and the DCC Core list, real FSRS scheduling, a tap-to-parse reader with a Morpheus morphology database, and a built-in lemma dictionary. Everything is offline and on-device — no accounts, no ads, no analytics. The APK (~90 MB, includes the 113 MB morphology DB) is on GitHub Releases; source is open. Known limitations: dictionary glosses are Morpheus short-form, not LSJ (LSJ ingestion is planned but not in this build); no audio yet (the settings toggle was removed until real audio content ships); device-to-device sync exists as an opt-in beta using end-to-end-encrypted events through a relay you choose; Android only, 64-bit. This is beta: export backups often, report issues on the tracker. Feedback from people actually studying with Athenaze/JACT is exactly what I need.

CI (TASK R-6, **spec only — do not create `.github/workflows/release.yml`** in this pass):

- trigger: `push: tags: ["v*"]`; jobs: setup JDK17 + Android SDK + NDK `27.0.12077973` + rust stable w/ android targets
- decode `secrets.GREEK_KEYSTORE_B64` → jks, `secret`s → `key.properties`; `cargo test --workspace`; `./gradlew :app:assembleRelease`; `apksigner verify --print-certs` assert not-debug; upload asset via `gh release create` (binaries action)
- secrets live on the **github** remote only; gitea `origin` runs no CI (the soak cron above is its substitute)

**Exit criteria:** ≥1 week after P1 exit; security gates §5 all closed; LSJ-honesty everywhere (no "LSJ" strings in UI/store copy without LSJ data — current UI label "Morpheus glosses (LSJ pending)" is compliant). **Rollback:** delete tag/release, pin a known-limitations note.

### P3 — Stable (Play closed → open)

Play closed track after ≥2 public APK releases; open when: crash-free 30 days across Play Console + self-reports, **zero** sync integrity incidents in P2, ≥1 external community success report of reinstall→export-import restore. **Rollback:** Play staged rollout halt; channel rank resets to GitHub-only.

## 5. Security gates — MUST-FIX before P2

| # | Gate | Verified state (2026-10-18) | Fix spec |
|---|---|---|---|
| G1 | **Sync secret at rest** | No in-app nostr identity storage exists **yet** at HEAD (grep: no nsec anywhere in app/bindings; `sync-nostr` transports hold `Keys` in memory). Risk is **forward-looking**: Wave 6c must not persist nsec in `user_settings`/`settings_sync` SQLite (plaintext, and `settings_sync` is a *synced* table — would gossip the key to every device in plaintext-intent tables) | Android: EncryptedSharedPreferences / Keystore-wrapped AES per DESIGN-device-sync.md §8; same for the iroh author + namespace keys. Never write `nsec*` keys via `setting_put`/`apply_settings` — add an explicit `SECRET_KEY_BLOCKLIST` guard + test in `crates/core/src/sync/mod.rs`. Exclude prefs from Auto Backup (`fullBackupContent` attrs; manifest currently only sets `allowBackup=true`) |
| G2 | **DocTicket = write capability** | SYNC-WIRE §3.3: Write-share ticket embeds the namespace secret; observed 277-char QR. iroh has **no capability TTL/revocation** — rotation = new doc + re-pair | UX: pairing screen warns "this QR is a credential — don't screenshot/share"; dim-and-pair in person; roster (`sync_devices`) exposes drop/rotate. Accept TTL absence; document it |
| G3 | **Relay write policy for public users** | Owner's Pyramid: member-invite only — **not usable** for arbitrary beta users. SYNC-WIRE §2.3: public relays rate-limit fresh keys (140-chunk burst tripped damus; 500 ms pacing + 10 s backoff succeeded) | Default = bring-your-own relay; fallback curated public list with the §2.3 pacing hardcoded; UI copy states public relays see ciphertext + timing only |
| G4 | **Import-side FSRS integrity** | `apply_review_events` (`sync/mod.rs:214`) trusts incoming events: no whitelist of `card_id` (DESIGN §10 promised this — **not implemented**), no `v!=1` rejection in `sync-nostr` codec (field deserialized, never enforced), out-of-range ratings silently fold as Good (`sync/mod.rs:311-317`), HLC parse checks shape only | Tighten: codec rejects `v!=1` and oversized batches; apply rejects unknown `card_id` (except `lookup-` G-set rows per §5 design) + rating ∉1–4 + HLC future-skew >24 h; covered by new UniFFI integration tests next to `crates/uniffi-bindings/tests/wave2_integration.rs` |

## 6. Telemetry stance — none, by design

Public commitment (write into `privacy.html` on the docs site): **zero analytics, zero crash SDKs, zero network in the non-sync build** (manifest has no `INTERNET`). Health signals without telemetry:

- **GitHub issue templates** (task when `.github/` is created): *Bug* (steps, expected/actual, APK version = `versionName/code`), *Crash* (attach `adb logcat --pid=$(pidof com.greekapp)` excerpt instructions + how to find the Rust panic line), *Sync* (attach **SyncStats JSON**, see below).
- **In-app "Report a problem"** (Settings): opens `https://github.com/EthnTuttle/greek/issues/new?template=bug.yml` via `CustomTabsIntent`; **not** mailto (owner preference, filters noise to the tracker).
- **"Share sync diagnostics"** button: exports a sanitized JSON = `{appVersion, schemaVersion, morphologyDbVersion, reviewRowCount, settingsRowCount, syncDeviceCount, lastSyncHlcPerDevice}` — **no card_ids, no nsec, no relay URLs redacted-on by default?** relay URL visible is fine (user's own), user reviews the JSON before sharing.

## 7. Ops runbook

**Cutting a release** (after R-1 key + R-4 alignment fixes land):

1. `git log --oneline v<prev>..HEAD` → prepend summary to `CHANGELOG.md`
2. bump `versionCode` (+1) / `versionName` in `android/app/build.gradle.kts`; bump `MORPHOLOGY_DB_VERSION` iff morphology.sqlite changed
3. `cargo test --workspace` (81 core + integration suites green) — local substitute for CI
4. `./gradlew :app:syncAssets :app:assembleRelease`; `apksigner verify --print-certs …/app-release.apk` → assert owner cert, not `CN=Android Debug`
5. 16 KB sanity: `objdump -p android/app/src/main/jniLibs/arm64-v8a/libuniffi_bindings.so | grep align` → must read `2**14` on LOAD rows (today: `2**12` — fails; see §8 R-4)
6. Device-check on Pixel 6 (existing protocol):
   ```sh
   adb install -r app-release.apk          # update in place; do not uninstall
   adb shell monkey -p com.greekapp -c android.intent.category.LAUNCHER 1
   # manual smoke: Home counters render, one review session, Curriculum/Reader/Dictionary tabs load
   # forensic (debug variant via run-as; for release use the export → compare drill):
   adb shell run-as com.greekapp ls databases
   adb shell run-as com.greekapp cat databases/<app>.db > /dev/null   # readable check
   adb pull via run-as exec-out, then locally:
     sqlite3 app.db "PRAGMA user_version;            -- must equal SCHEMA_VERSION (3)
                     SELECT COUNT(*) FROM review_logs;
                     SELECT COUNT(*) FROM review_logs WHERE uuid IS NULL;  -- must be 0 on v3
                     SELECT COUNT(*) FROM memory_states;
                     SELECT COUNT(*) FROM settings_sync;"
   # compare all four counts against the pre-update snapshot; due queue diff must be empty
   ```
7. tag + push **both** remotes: `git tag v0.9.0; git push origin v0.9.0; git push github v0.9.0`
8. `gh release create v0.9.0 app-release.apk -t "…" -n CHANGELOG-excerpt` (P1: add `--prerelease`)

**Relay ops (Pyramid):** member add/remove via admin UI when beta testers join/leave; monthly eyeball of disk + retention; the relay only ever holds ciphertext of invited members' own events. Relay-data backup is optional (history reconstitutes from any connected device, design §14) but cheap — do it anyway.

**Docs site:** edit `/docs`, push `master` to `github` remote → Pages auto-deploys. Privacy page must ship before Play submission (§2.3).

## 8. Risk register

| # | Risk | L×I | Mitigation |
|---|---|---|---|
| 1 | Bus factor = 1 dev; release key + relay admin + Play account all owner-held | M×H | Password-manager escrow of jks + key.properties; BLOCKED-protocol for agents; this runbook |
| 2 | NDK/Rust toolchain rot breaks reproducible builds | M×M | `flake.nix` pins host; record NDK `27.0.12077973` + rust 1.93.1 in release notes; copy spike Cargo.lock into workspace pin |
| 3 | FSRS-rs param drift across versions → cross-device fold divergence (design §14) | M×H | Pin `fsrs = "=2.0.4"`; record `fsrs_params_version`; explicit re-anchor event on any bump |
| 4 | **16 KB page-size**: verified LOAD `align 2**12` on arm64 `.so`; Play (targetSdk 35) requires 16 KB since 2025; device already warned | H×M(P3 blocks) | R-4: rebuild with `RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"` for both ABIs (NDK r27 supports); verify with the §7 objdump gate |
| 5 | Public-relay rate limits degrade sync (SYNC-WIRE §2.4 evidence) | H×M | Private BYO-relay default; pacing hardcoded; chunked batches respect 64 KiB caps |
| 6 | Data-loss event early (no transport-zero yet) → reputational kill | M×H | Ship export/import before P2; force backup nudge in release notes; drill in P0 |
| 7 | License/attribution gap: no root LICENSE; morphology = Morpheus XML derived from Perseus (CC BY-SA per CLAUDE.md); card decks = textbook vocab fact-lists | M×M | R-7: add LICENSE (code) + NOTICE listing Perseus CC BY-SA attribution + LSJ public-domain + GLTP MIT, linked from app About screen |
| 8 | Play 20-tester closed-track requirement stalls P3 | H×L | Recruit via r/ancientgreek beta thread + classics Discords early; GitHub Releases remains primary |
| 9 | APK size (~90 MB) deters install | M×L | Accept for now; note in listing; future: split abi + on-demand asset |
| 10 | Sync metadata leak on public relays (pubkey, timing: accepted v1) | M×L | Documented in privacy page; topic-tag blinding candidate for v2 |

## 9. Open questions — decision needed from owner

1. **Play Console $25 + identity disclosure**: acceptable? (blocks P3) — personal accounts also need the 20-tester closed track.
2. **F-Droid priority**: file metadata after P2 (~1–2 wk iteration) or skip to focus on Play?
3. **Pyramid as beta sync relay**: owner willing to member-invite 3–5 beta npubs for P1 sync testing, or strictly keep Pyramid personal (beta = no-sync or user's own relay)?
4. **GitHub Releases signing identity**: one long-lived RSA-4096 key (proposed) vs. key rotation story if lost — confirm escrow location.
5. **LSJ for 1.0**: hold 1.0 on real LSJ ingestion (data/README future-work) or accept "Morpheus glosses" honesty permanently?
6. **Beta scope freeze**: are alphabet gate + practice loops + sync enough for the 0.9.x line, or must LSJ/audio land before P2?

## Appendix A — Facts verified for this document (2026-10-18)

| Claim | Command / evidence |
|---|---|
| versionCode 1, versionName "1.0" | `android/app/build.gradle.kts:15-16` |
| release buildType: no signingConfig, no minify | `build.gradle.kts:24-31` |
| Manifest permissions: `POST_NOTIFICATIONS` only; **no INTERNET**; `allowBackup=true`; ReminderReceiver `exported=false` | `AndroidManifest.xml` lines 4/8/25 |
| APK debug-signed | `apksigner verify --print-certs app-debug.apk` → `Signer #1 certificate DN: C=US, O=Android, CN=Android Debug` |
| APK size 90,274,772 B (~90 MB); morphology asset 113,475,584 B | `ls -l android/app/build/outputs/apk/debug/`, `android/app/src/main/assets/` (main checkout; assets are gitignored) |
| 16 KB failure | `objdump -p …/arm64-v8a/libuniffi_bindings.so` → all LOAD `align 2**12` (4 KB), not 2**14 |
| `MORPHOLOGY_DB_VERSION = 2` + marker re-copy guard | `GreekApplication.kt:127,162,172` |
| Schema chain v1→v3 + upgrade tests | `crates/core/src/db/mod.rs:8-19,104-186,230-292` |
| 81 core unit tests (curriculum 24, db 4, morphology 6, questions 13, reader 7, search 5, srs 15, sync 7) | `grep -c '#\[test\]' crates/core/src/**` |
| No `v==1` gate in codec; ratings fold-as-Good on OOR | `crates/sync-nostr/src/codec.rs` (no version check found); `sync/mod.rs:311-317` |
| No in-app nsec storage at HEAD | grep over `crates/` + `android/app/src/main` — nothing (G1 is a forward guard, not an existing leak) |
| `.github/` absent; no CHANGELOG; no root LICENSE | `ls` at repo root |
| Play icon 512×512 exists; no feature graphic; no screenshots | `file art/ic_launcher-playstore.png` → `512 x 512 PNG` |
| Remotes: `origin`=gitea@git.virginiafreedom.tech:2222, `github`=EthnTuttle/greek | `git remote -v` |
| Public-relay pacing evidence (rate-limit at burst, 500 ms pacing OK, ~2.5 min for 8-chunk / 367 KB batch) | `SYNC-WIRE.md §2.4` live test log, rerun 2026-10-18 |
| iroh ticket = write credential, 277 chars | `SPIKE-IROH.md` Part A + `SYNC-WIRE.md §3.3` |
| Licenses of shipped deps | `Cargo.toml`s (fsrs MIT, uniffi MPL-2.0, nostr-sdk MIT, rusqlite MIT); `android/app/build.gradle.kts` (JNA 5.14.0, androidx Apache-2.0); content provenance per `CLAUDE.md` + `data/README.md` |

## Appendix B — Task list extracted (owner-visible)

| ID | Task | Blocks |
|---|---|---|
| R-1 | Add `key.properties`, `*.jks`, `*.keystore` to `.gitignore` | first keygen |
| R-2 | Feature graphic 1024×500 from `art/icon-master.png` | Play listing |
| R-3 | ≥2 Pixel 6 screenshots | Play listing |
| R-4 | Rebuild native libs with `-Wl,-z,max-page-size=16384`; verify `2**14` LOAD align both ABIs | P3 Play (16 KB) + cleaner warning-free installs |
| R-5 | Pin `fsrs = "=2.0.4"`; persist `fsrs_params_version` in `user_settings` | P2 (sync correctness story) |
| R-6 | Create `.github/workflows/release.yml` per §4-P2 spec (after owner approves) | P2 automation |
| R-7 | Root LICENSE (code, owner's choice — MIT/Apache-2.0 dual suggested) + NOTICE (Perseus CC BY-SA attribution, Morpheus provenance, LSJ public-domain note once ingested) | P2 public beta |
