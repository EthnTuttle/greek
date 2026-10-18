# SPIKE-IROH — iroh 1.0 + iroh-docs sync feasibility

Date: 2026-10-17. Head: 4c49da6 (worktree wt/sync-iroh). No workspace changes;
standalone crate at `spikes/iroh/` ([workspace] isolated, own target dir).

## Versions locked

Selected by `"1"` / `"0.101"` / `"0.103"` semver ranges (resolved 2026-10-17):

- iroh 1.3.0, iroh-base 1.3.0, iroh-relay 1.3.0, iroh-dns 1.3.0
- iroh-docs 0.101.0 (fs-store/redb default on)
- iroh-blobs 0.103.1, iroh-gossip 0.101.0, iroh-tickets 1.0.0
- rustls 0.23.46 with **ring** provider, rust 1.93.1

## PART A — in-process convergence: PASS

Two endpoints (preset `Empty` = no relay/DNS, loopback QUIC), each a full
Router accepting BLOBS+GOSSIP+DOCS ALPNs, Docs::memory:

- X: doc + author, 50 entries, keys `reviews/<hlc-ms:013>-dev-<uuid8>`,
  ~300 B JSON payloads.
- Ticket: `doc.share(ShareMode::Write, AddrInfoOptions::Addresses)` →
  string (kind `doc`, postcard), observed **277 chars**, round-trips via
  `FromStr`. **Write-capable ticket embeds the namespace secret key in
  `capability`; Read tickets embed only the public key** — multi-writer
  requires Write tickets (treat the ticket as a credential).
- Y: `import(ticket)` + `start_sync` → converged to exactly 50 entries.
  **Measured initial sync: 722.7 ms and 731.8 ms (two runs)** in-process on
  this dev box. Todo scale: ~1.4 ms/entry pair (gossip + blob transfer
  included).
- Y writes 10 entries under its own author; X `start_sync` → X converges to
  exactly 60. Bidirectional multi-author verified.
- Key order: `get_many(Query::all())` returned entries in sorted key order in
  all runs (HLC key prefix strategy holds).

## PART B — persistence (redb): PASS

Same flow with `Docs::persistent(dir)` (writes `docs.redb` + author store),
routers shut down, fresh process-like reopen of node X: `docs.list()` returns
the doc, all 60 entries intact.

## PART C — Android cross-build: PASS

Toolchain exactly as app config: NDK 27.0.12077973, clang
`*-linux-android26-clang`, `llvm-ar`, link via clang. Baseline project uses
no cross-compile scripts (checked) — env var approach only.

- `aarch64-linux-android`: release build **clean on first attempt, 1 m 13 s**
  wall (13 m user, parallel). `libspike_iroh.so`: **2,551,632 B unstripped →
  1,173,544 B after `llvm-strip -s`**.
- `x86_64-linux-android`: release build clean, **1 m 08 s** wall.
  2,038,600 B unstripped → **1,198,792 B stripped**.
- Breakages encountered: **none**. ring (not aws-lc-rs) → no cmake needed;
  getrandom/quinn-udp compiled unpatched; `.cargo/config.toml` unnecessary —
  `CARGO_TARGET_*_LINKER` + PATH was sufficient.
- Prerequisite found on host: rustls crypto provider must be passed explicitly
  (`Endpoint::builder(..).crypto_provider(ring::default_provider())`) — used
  from the start, not Android-specific.

Binary weight vs app: current
`android/app/src/main/jniLibs/arm64-v8a/libuniffi_bindings.so` = **4,825,840 B
(4.6 MiB)** (actual measurement — the "~20 MB per ABI" figure in the brief is
stale). Adding the iroh sync layer as a second cdylib ≈ **+1.1 MiB stripped
per ABI (~+24 %)** — well under budget.

Note: spike .so includes tokio full + serde_json + redb but not yet UniFFI
glue; shim glue is pure Rust and negligible vs the 1.1 MiB of iroh itself.

## PART D — wire format section: DONE

See `SYNC-WIRE.md` → `## 3. iroh-docs encoding`.

## OPEN RISKS

1. **API pinning**: iroh-docs 0.101 / iroh 1.x compat is a semver match but
   the docs API (`Docs::memory/persistent`, `api::Doc`) still moves between
   0.x releases; pin exact versions + vendored lockfile, expect porting work
   on any bump past 0.101.
2. **Battery**: keeping an endpoint + gossip swarm alive on Android idle is
   unmeasured; checklist prescribes shutdown-on-background + foreground
   service for active sync only.
3. **NAT traversal success rate unknown**: spike proved loopback/direct
   address mode (LAN); relay-mediated sync across real WAN networks (n0
   relays reachable from sandbox? untested) and IPv6-only carriers unproven
   at sync level even though iroh's hole-punching is production-proven
   elsewhere.
4. **Ticket freshness**: tickets carry direct addresses that age; peers on
   roam networks need fresh tickets or relay in tickets
   (`AddrInfoOptions::Relay`/N0 preset).
5. redb store on Android filesystem (encrypted app dir) — performance
   unmeasured on-device, expected fine.

## GO/NO-GO for Wave 6b Android integration

**GO.** All four parts PASS on locked versions. Composer of Wave 6b should
pin the exact Cargo.lock produced here (commit `spikes/iroh/Cargo.lock`),
reuse the env-var NDK build recipe, and schedule one WAN/relay follow-up
spike before claiming end-to-end field reliability.

## Commands that worked

```sh
# host run (parts A+B)
cd spikes/iroh && cargo run --quiet

# android builds
NDK=~/Android/Sdk/ndk/27.0.12077973/toolchains/llvm/prebuilt/linux-x86_64/bin
export PATH=$NDK:$PATH
export CC_aarch64_linux_android=aarch64-linux-android26-clang
CXX_aarch64_linux_android=aarch64-linux-android26-clang++
AR_aarch64_linux_android=llvm-ar
CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android26-clang
cargo build --release --target aarch64-linux-android   # 1m13s
# (x86_64-linux-android analogous)                      # 1m08s
$NDK/llvm-strip -s target/<triple>/release/libspike_iroh.so -o out.so
```

## Deviations from brief

- `libuniffi_bindings.so` measured 4.83 MB, not "~20 MB" — brief figure stale.
- `docs` memory API spelled `iroh_docs::protocol::Docs::memory()` (not
  `Docs::in-memory`); persistent via `Docs::persistent(path)`.
- Ticket measured with `AddrInfoOptions::Addresses` (LAN), not `Id`
  (requires DNS discovery we disabled via `presets::Empty`).
- rustls **ring** chosen over aws-lc-rs from the start; zero Android pain.
- Host `.cargo` registry GC emits a benign "Permission denied" warning on
  every cargo invocation here (read-only Nix-managed dir owned elsewhere);
  unrelated to the spike.
