# ZAPSTORE.md — Publishing `Greek` (com.greekapp) to Zapstore

Prepared 2026-10-18 by subagent Z1. Tested with **zsp v0.4.17** (prebuilt linux-amd64 binary).
Sources cited per section. Companion file: `zapstore.yaml` (same directory — orchestrator merges it to the repo root).

## 1. What the current (2026) flow actually is
Source: https://zapstore.dev/docs/publish · https://github.com/zapstore/zsp

| Topic | Answer |
|---|---|
| CLI | **`zsp`** (Go). The old Dart `zapstore-cli` is superseded; do NOT `pub global activate zapstore_cli` — that flow is 2025-era and rotted. |
| Install | `go install github.com/zapstore/zsp@latest` **or** prebuilt binary from https://github.com/zapstore/zsp/releases (verified working here: `zsp-0.4.17-linux-amd64`) |
| Config | `zapstore.yaml` at **repo root, committed** (relay fetches it for whitelisting) |
| Event kinds | **32267** app metadata · **30063** software release · **3063** software asset (NIP-82), plus a one-time **NIP-C1** identity cert-linking event |
| Artifact hosting | zsp uploads APK + icon + screenshots to **Blossom** CDN (`https://cdn.zapstore.dev` default, override `BLOSSOM_URL`). Remote-URL artifacts (GitHub Releases) also supported — asset event can reference the GitHub URL directly. |
| Signing | `SIGN_WITH` env: `nsec1…`, hex key, `bunker://…` (NIP-46), `npub1…` (unsigned output), or `browser` (NIP-07). Bunker is the CI-friendly option. |
| Indexing / relays | Events go to **`wss://relay.zapstore.dev`** by default (`RELAY_URLS` to override). Zapstore indexes ITS OWN relay. |

**⚠ Pyramid relay answer (chat.virginiafreedom.tech):** zapstore's default catalog indexes `relay.zapstore.dev`, NOT your relay. Your nostr key needs **no membership on the Pyramid relay** for the listing itself. Your relay only matters (a) if you set `RELAY_URLS` to publish an extra copy there, or (b) as the relay in a `bunker://` URL if you run NIP-46 remote signing through it. Reusing your Pyramid-authorized key as the zapstore identity is safe but see §3.

## 2. Key separation — what signs what
Source: https://zapstore.dev/docs/trust-model · zsp README §Nostr Events

| Key | Role | Rotation cost |
|---|---|---|
| APK signing cert (`android/keystore/release.jks`, alias `greek`) | **Update identity.** Android requires all updates of `com.greekapp` to be signed by the same cert. zsp records `apk_certificate_hash` in every kind-3063 event. | Lose it = users must uninstall/reinstall. Back it up **independently** of nostr keys. |
| Nostr key (your nsec) | **Listing ownership.** Signs kinds 32267/30063/3063. Whitelisted against `pubkey:` in the committed `zapstore.yaml`. | Rotatable: change key, update `pubkey:` in repo, relay re-verifies. Reputation (Vertex scores, user follows) resets. |
| NIP-C1 link event | One-time cryptographic proof binding APK cert → npub, so Zapstore can show "publisher == APK signer." Run `zsp identity --link-key`. | Re-run if either side rotates. |

**Recommendation:** use a **zapstore-dedicated nostr keypair** for publishing (keep nsec offline, use bunker for releases). Your existing Pyramid key can optionally post a kind-32267/30063 **endorsement** linking personas, but keeping publishing identity separate limits blast radius of either compromise. Either way works technically.

## 3. Prerequisites (one-time)
| Step | Command | Notes |
|---|---|---|
| Install zsp | `curl -sL -o ~/.local/bin/zsp https://github.com/zapstore/zsp/releases/download/v0.4.17/zsp-0.4.17-linux-amd64 && chmod +x ~/.local/bin/zsp` | or `go install github.com/zapstore/zsp@latest` |
| Build release APK | `cd android && ./gradlew assembleRelease` | requires `~/.gradle/gradle.properties` to point at `keystore/release.jks`; output = `android/app/build/outputs/apk/release/app-release.apk` |
| Generate keystore (if absent) | `keytool -genkeypair -v -keystore android/keystore/release.jks -keyalg RSA -keysize 4096 -validity 10000 -alias greek -storetype PKCS12` | PKCS12 already as planned |
| Merge this setup | copy `zapstore.yaml` (with real `pubkey:`) + this file to repo root; **commit & push to github.com/EthnTuttle/greek** | relay fetches `zapstore.yaml` from GitHub — must exist on the mirror's default branch BEFORE first publish |
| Fill in pubkey | edit `pubkey:` in `zapstore.yaml` (`nostr-key-tools`-style: derive npub from your nsec) | placeholder invalid until replaced |
| Cert linking | `SIGN_WITH=nsec1… zsp identity --link-key android/keystore/release.jks` | NIP-C1 proof; first `zsp publish` will prompt for it anyway if skipped |

## 4. First publish (one-time)
```bash
# from repo root, zapstore.yaml committed & pushed to GitHub:
SIGN_WITH=nsec1<your-publish-key> zsp publish --wizard
```
Wizard previews events in browser, uploads APK+icon+screenshots to `cdn.zapstore.dev`, publishes the 3 events, and NIP-C1 links your keystore if not already done. The relay whitelists your npub by fetching the committed `zapstore.yaml` and matching `pubkey:`.

## 5. Per-release (steady state)
| # | Step | Command |
|---|---|---|
| 1 | Bump version | `versionCode+1` & `versionName` in `android/app/build.gradle.kts` (currently 1 / "1.0") — version is parsed FROM the APK, not the yaml |
| 2 | Build | `cd android && ./gradlew assembleRelease` |
| 3 | Publish | `SIGN_WITH=nsec1… zsp publish -y zapstore.yaml` |
| 4 | (opt) mirror to your relay | `RELAY_URLS=wss://relay.zapstore.dev,wss://chat.virginiafreedom.tech zsp publish -y zapstore.yaml` |

**Final ready-to-run command for this repo (after §3+§4 prerequisites):**

```bash
cd /home/ethan/code/greek && SIGN_WITH="$ZAPSTORE_NSEC" zsp publish -y zapstore.yaml
```

**Preview without publishing** (tested working):
```bash
SIGN_WITH=nsec1<key> zsp publish -q --offline zapstore.yaml > events.json   # signed events → stdout, upload manifest → stderr
```

## 6. Update semantics & guarantees
Source: https://zapstore.dev/docs/trust-model §Verification on install

| Mechanism | Guarantee |
|---|---|
| Same `d` tag (`com.greekapp`) on kind-32267 | App listing continuity |
| Higher `version_code` in kind-3063 | Client offers update |
| Same `apk_certificate_hash` across releases | Android OS enforces install-as-update; Zapstore blocks install on mismatch |
| SHA256 `x` tag == downloaded APK hash | Client blocks tampered downloads |

## 7. Troubleshooting
Source: zsp README + trust-model doc

| Symptom | Cause | Fix |
|---|---|---|
| Event rejected by relay, new npub | Not whitelisted | Commit `zapstore.yaml` with correct `pubkey:` to the GitHub mirror, push, retry |
| Rejected despite committed yaml | Pubkey mismatch between yaml and `SIGN_WITH` | npub of your nsec must EXACTLY equal the yaml value |
| Whitelist fetch fails | Repo host unsupported / default branch not mirrored | GitHub/GitLab/Codeberg/Gitea only; verify mirror push |
| GitHub API rate limits during fetch/enrich | Anonymous API calls | set `GITHUB_TOKEN` |
| 117MB APK upload slow/fails | Blossom CDN upload of large asset | retry; zsp caches (`--overwrite-release` forces); `BLOSSOM_URL` to self-hosted Blossom if cdn.zapstore.dev chokes |
| Relay propagation lag | zapstore.dev indexer polls relay | wait minutes; verify with `nak req -k 32267 --tag d com.greekapp wss://relay.zapstore.dev` |
| Rebuild same version, no changes | Cache suppresses re-publish | expected; use `--overwrite-release` only if truly needed (creates duplicate release events) |
| Config question without keys | — | `zsp publish --check zapstore.yaml` (needs fetchable APK); `--offline` + `SIGN_WITH=npub1…` for full dry-run (this is how `zapstore.yaml` was validated) |

## 8. Merge checklist for the orchestrator / owner
Order matters — items 1–4 must land before the first `zsp publish`.

| # | Action | Where |
|---|---|---|
| 1 | Copy `zapstore.yaml` to repo root; replace `pubkey:` placeholder with the real npub | `/home/ethan/code/greek/zapstore.yaml` |
| 2 | Copy this file to repo root (or `docs/`) | `/home/ethan/code/greek/ZAPSTORE.md` |
| 3 | Commit + push so GitHub mirror has `zapstore.yaml` on default branch | origin = github.com/EthnTuttle/greek |
| 4 | Create & wire `release.jks` (keytool command in §3); build release APK | `android/keystore/`, §3 |
| 5 | First publish per §4; verify listing at `https://zapstore.dev/apps/com.greekapp` | browser |
| 6 | Delete the staging binary `/tmp/opencode/zapstore-setup/zsp-0.4.17-linux-amd64` (install per §3 instead) | — |
| 7 | (opt) Announce release from any nostr account tagging the npub; bots/users may add it to community catalogs | nostr |

## 9. Known-rotten 2025-era assumptions — DO NOT USE
| Claim | Verdict |
|---|---|
| `dart pub global activate zapstore_cli` | ❌ Dart CLI superseded by Go `zsp` |
| `zapstore.yaml` with `identifier:` / `artifacts:` fields | ❌ Package id & version parsed from the APK; artifacts replaced by `release_source` + Blossom upload |
| 1063 file-metadata events | ❌ Kind 3063 + Blossom now |
| Manual GitHub-Releases URL w/ `apkIdentifierHash` | ❌ Obsolete; zsp auto-selects best APK from `-r` repo or uploads local builds |

## 10. Reality check performed
- Downloaded `zsp-0.4.17-linux-amd64`; `--version`/`--help` OK.
- Ran full dry-run against the existing debug APK with offline mode: **3 valid unsigned events generated**, all 7 media paths accepted, APK parsed (`com.greekapp`, `1.0`/1, minSdk 26, targetSdk 35, 117,737,065 bytes, cert hash extracted).
- No release APK and no GitHub Release exist yet → §3 build step is mandatory before §4.
- Debug-APK event showed the universal-ABI `f` tag list (armeabi/mips/… placeholders) — cosmetic, zsp defaults; the real release APK contains only arm64-v8a + x86_64 native libs.
