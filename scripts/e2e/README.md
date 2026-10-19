# scripts/e2e — cross-device sync E2E orchestrator

`e2e.sh` ties the two halves of the sync E2E together under the frozen protocol
**greek-sync-e2e v1**:

- **Device half** — `androidTest` `com.greekapp.CrossDeviceSyncE2E` (DEBUG build;
  written by the parallel Android agent) runs the real GreekCore sync stack on
  the phone and drops evidence files in `files/e2e/` inside the app sandbox.
- **Host half** — `cargo run -p uniffi-bindings --example sync-peer` (parallel
  Rust agent) is a CLI peer that runs the same GreekCore logic on this machine
  against a throwaway SQLite DB.

The orchestrator bridges the halves, waits for both exit codes, pulls the
exports, and prints a one-line verdict.

```
scripts/e2e/e2e.sh iroh|nostr|edges|all  [--selfcheck | --device <serial>] [--keep]
```

## Protocol: greek-sync-e2e v1 (frozen — do not redefine here)

Evidence-namespace files (relative to the evidence dir):

| file | writer | reader | meaning |
|---|---|---|---|
| `ticket.txt` | A (device test / `serve`) | orchestrator → B bridge | iroh DocTicket for live pairing |
| `seeds_a.json` | A | orchestrator → B bridge | `{"uuids":[...]}` A's card ids |
| `nsec_in.hex` | orchestrator | device test + B | 64-hex-chars throwaway test identity (nostr only) |
| `push_done.flag` | A | orchestrator | A's spool is on the relay; B may fetch |
| `b_pushed.flag` | orchestrator | A (device test) | B's spool is on the relay; A fetches + merges (nostr only) |
| `export_a.json` / `export_b.json` | A / B | orchestrator | post-sync `{"uuids":[...]}`; sets must be equal and non-empty |

On device the evidence dir is reached via the **run-as channel**
(`adb shell run-as com.greekapp … files/e2e/<rel>`), which only works on debug
builds — that's intentional: the E2E gates the debug APK. In `--selfcheck` the
identical helpers address a local temp dir, which is exactly the `--dir` both
`sync-peer` instances share.

## Modes

| mode | selfcheck (default) | `--device <serial>` |
|---|---|---|
| `iroh` | `sync-peer serve` (bg) + `sync-peer join` in one shared dir | installs debug+test APKs, instruments `#irohLivePairingE2E`, materializes `ticket.txt`/`seeds_a.json` into a bridge dir, runs host join |
| `nostr` | stub/local relay + `serve --nostr` (bg) + `join --nostr` over `ws://127.0.0.1:7777` | local relay first, `adb reverse tcp:7777`, pushes `nsec_in.hex` to the sandbox, instruments `#nostrLocalRelayE2E`, waits `push_done.flag` → host B joins → `b_pushed.flag` → A merges |
| `edges` | prints "no selfcheck equivalent", exit 0 | `:app:installDebugAndroidTest` + `#syncEdgeCases`; raw `INSTRUMENTATION_STATUS` tail |
| `all` | iroh + nostr | iroh → nostr → edges |

Safety rails (device mode only): the serial argument is **mandatory**, the
owner's daily-driver Pixel 6 `1B151FDF600826` is **hard-refused** in the script,
and the run refuses unless `adb devices` lists the serial with state `device`
(not `unauthorized`/absent). Every adb call is pinned with `adb -s $SERIAL`.

> **Bench device is currently the Pixel 7a, serial `35291JEHN11734`.** The
> banned Pixel 6 serial is hardcoded above and in `e2e.sh`; to move the bench,
> pass a different `--device <serial>` flag — do not edit the script.

Every run logs to `/tmp/opencode/e2e/<UTCts>-<mode>/`:

- `transcript.log` — everything you saw on stdout/stderr
- `host.log` — relay + `sync-peer` stdout/stderr
- `device.logcat.txt` — instrumentation stream (`device.<flow>.logcat.txt` in `all` mode)
- `exports/` — `export_a.json` / `export_b.json` (`exports/<flow>/` in `all` mode)

## Prerequisites

- Host: `cargo`, `python3`, `adb` (platform-tools), GNU `timeout`.
- Device mode: bench phone, USB debugging, **screen unlocked**, this repo's
  debug APK installable (`run-as com.greekapp` works ⇒ debug build). Gradle needs
  the usual Android SDK/NDK env.
- First real run builds the examples — allow several minutes
  (`E2E_RELAY_WAIT_S=180` default covers a cold `cargo` build of the relay).

## Runbook & expected wall-clock

| command | expected duration | notes |
|---|---|---|
| `scripts/e2e/e2e.sh iroh` | < 1 min | selfcheck, host↔host over iroh via loopback discovery |
| `scripts/e2e/e2e.sh nostr` | < 1 min (+ first cargo build) | selfcheck over local relay |
| `scripts/e2e/e2e.sh iroh --device 35291JEHN11734` | ~2–6 min | gradle install dominates; pairing itself ~30–90 s (`ticket.txt` poll cap 90 s) |
| `scripts/e2e/e2e.sh nostr --device 35291JEHN11734` | ~2–4 min | relay + reverse + instrument + two sync directions |
| `scripts/e2e/e2e.sh edges --device 35291JEHN11734` | ~1–3 min | no host side |
| `scripts/e2e/e2e.sh all --device 35291JEHN11734` | ~5–12 min | pre-release bench sweep |

Exit code: 0 = PASS, 1 = a flow failed, 2 = usage/guard/infra fatal.

### Verdict lines

```
E2E-IROH:  PASS (device exit=0, host exit=0, |uuidsA|=128, |uuidsB|=128, setEqual=true)
E2E-NOSTR: PASS (device exit=0, host exit=0, |uuidsA|=128, |uuidsB|=128, setEqual=true)
E2E-EDGES: PASS
E2E-ALL:   PASS
```

PASS requires: instrument exit 0 **and** host peer exit 0 **and** both exports
present + parse **and** UUID sets equal **and** non-empty. UUID set equality is
checked with `python3` (never `jq`). Timestamp-vs-content mismatches are out of
scope here — the verdict asserts *card-id set convergence* only.

## Failure triage

| symptom | likely cause |
|---|---|
| `ticket.txt never appeared` (iroh, device) | device's iroh endpoint didn't come up or can't reach the N0/DERP relay; check `device.logcat.txt` for endpoint init, test Wi-Fi egress |
| `push_done.flag never appeared` (nostr, device) | relay wasn't actually ready, or device can't use ws cleartext → confirm debug `networkSecurityConfig` allows cleartext to localhost, confirm `adb reverse tcp:7777` installed (script logs it) |
| `relay exited before RELAY_READY` | port 7777 busy (`E2E_RELAY_PORT` to change), or `sync-nostr` example missing/broken — see `host.log` |
| instrument `Process crashed` in `device.logcat.txt` | app-side crash — grep the logcat tail for the exception; evidence dir may be half-populated (rerun with `--keep`) |
| `host exit=124` | host peer hit `E2E_HOST_TIMEOUT_S` (default 240 s) — pairing stalled: iroh ticket stale/wrong, or relay events never arrived — see `host.log` |
| `setEqual=false` | genuine merge divergence — keep `--keep` artifacts, diff `exports/export_a.json` vs `export_b.json`; the symmetric difference is the lost direction of the sync |
| `run-as` errors / permission denied | APK on the bench phone is not the **debug** build, or the screen locked mid-run |
| `serial … not present with state 'device'` | phone unplugged/adb unauthorized; `adb devices` must show the exact serial with state `device` |

## E2E_STUB=1 (host-stub harness)

With `E2E_STUB=1` every cargo invocation is replaced by
`${STUBDIR:-/tmp/opencode/e2e-stubs}/{sync-peer-stub,relay-stub}` — tiny bash
stand-ins that write fake-but-wellformed contract files on the same shape
(ticket → join → union exports; push flag → join → b_pushed → union exports).

**E2E_STUB=1 proves orchestration logic only**: polling, bridging, sorting,
exit-code capture, verdict computation, cleanup. It runs no iroh, no nostr, no
GreekCore. A stubbed PASS is a prerequisite for — never evidence of — a real
device PASS.

## Field notes (learned during wave-8 integration, 2026-10-19)

1. **DB-dir collision = silent join hang.** iroh persists its endpoint secret
   key at `<db-parent>/iroh-sync/`. If two instances share a parent dir they
   share ONE node identity; a join then dials itself and hangs with zero dial
   logs (60s op timeout). BOTH peers must get their own subdirectory
   (`a/greek_app.db`, `b/greek_app.db`). Cost a full debugging session; the
   symptom is "join exceeds 60s, no dial activity, A's log shows sync rounds
   with no peer".
2. **Exports are SyncBatch v1** (`{"v":1,...,"reviews":[{"uuid":...}]}`), not
   `{"uuids":[...]}`. The comparator accepts both, but protocol docs that say
   "uuid set" mean `reviews[].uuid`.
3. **B's first nostr push sends MORE than its own 3 seeds**: pull lands A's
   events in B's DB and B's fresh push cursor re-uploads them (idempotent —
   A's pull-since cursor skips them). Assert `>=3`, never `==3`.
4. **Selfcheck topology is `inproc`** (two cores, one process, real loopback
   QUIC) — the cross-process `serve`/`join` pair remains available for manual
   bisection and is what `--device` mode uses (phone = A over `run-as`).
5. **`GREEK_IROH_NET=loopback`** (set by selfcheck automatically) selects
   `presets::Empty` in the transport: no n0 relay, no DNS discovery. If host
   DNS cannot resolve `staging-euw1-1.relay.n0.iroh.network`, device mode
   fails fast with guidance; same-LAN runs may opt into
   `E2E_IROH_NET=loopback`.
6. **Relay port is probed**, not hardcoded, since other services squat on
   7777; the androidTest always dials loopback `127.0.0.1:7777` on the PHONE
   and `adb reverse` maps it to the probed host port.
