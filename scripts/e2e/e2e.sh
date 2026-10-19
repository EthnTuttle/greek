#!/usr/bin/env bash
#
# e2e.sh — orchestrator for Wave-8 cross-device sync E2E ("greek-sync-e2e v1").
#
# Synopsis:
#   scripts/e2e/e2e.sh iroh|nostr|edges|all  [--selfcheck | --device <serial>] [--keep]
#
# DEFAULT is --selfcheck: pure host<->host, zero adb, zero device access.
# --device REQUIRES an explicit serial and:
#   - HARD-REFUSES the owner's daily-driver Pixel 6 (1B151FDF600826), always.
#   - refuses unless `adb devices` shows exactly that serial in state "device".
# All adb invocations carry `-s $SERIAL`.
#
# Frozen protocol (see README.md §Protocol): the androidTest CrossDeviceSyncE2E
# and the host `sync-peer` example exchange evidence files. On device they live
# in files/e2e/ under the app sandbox (run-as com.greekapp — DEBUG build only).
# In selfcheck the same helpers address a shared host dir (sync-peer --dir).
#
# E2E_STUB=1 replaces every cargo-run invocation with
# ${STUBDIR:-/tmp/opencode/e2e-stubs}/{sync-peer-stub,relay-stub}.
# E2E_STUB=1 proves ORCHESTRATION LOGIC ONLY — no real sync stack runs.
#
# Tunables (env):
#   E2E_HOST_TIMEOUT_S  host peer hard timeout          (default 240)
#   E2E_POLL_TIMEOUT_S  device file-poll timeout        (default 90)
#   E2E_RELAY_WAIT_S    relay RELAY_READY wait          (default 180; first cargo build)
#   E2E_RELAY_PORT      local nostr relay port          (default 7777)
#   STUBDIR             stub binary dir                 (default /tmp/opencode/e2e-stubs)

set -Eeuo pipefail

# ---------------------------------------------------------------- constants
APP_ID="com.greekapp"
PKG="$APP_ID"
TEST_CLASS="com.greekapp.CrossDeviceSyncE2E"
TEST_RUNNER="com.greekapp.test/androidx.test.runner.AndroidJUnitRunner"
BANNED_SERIAL="1B151FDF600826"   # owner's daily-driver Pixel 6 — never target
RELAY_PORT=""          # resolved lazily by resolve_relay_port (device app port stays 7777)
RELAY_URL=""
DEVICE_RELAY_PORT=7777   # what the androidTest dials; adb reverse maps it to $RELAY_PORT

pick_free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'; }

resolve_relay_port() {
  RELAY_PORT="${E2E_RELAY_PORT:-$(pick_free_port)}"
  RELAY_URL="ws://127.0.0.1:${RELAY_PORT}"
}
E2E_ROOT="${E2E_ROOT:-/tmp/opencode/e2e}"
STUBDIR="${STUBDIR:-/tmp/opencode/e2e-stubs}"
HOST_TIMEOUT_S="${E2E_HOST_TIMEOUT_S:-240}"
POLL_TIMEOUT_S="${E2E_POLL_TIMEOUT_S:-90}"
RELAY_WAIT_S="${E2E_RELAY_WAIT_S:-180}"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd -P)"

# ---------------------------------------------------------------- state
MODE=""                 # iroh | nostr | edges | all
TARGET="selfcheck"      # selfcheck | device
SERIAL=""
KEEP=0
RUN=""                  # /tmp/opencode/e2e/<UTCts>-<mode>
WORK=""                 # $RUN/tmp
EVID=""                 # selfcheck evidence dir ($WORK/e2e) — dev_* helpers target it
BRIDGE=""               # bridge dir given to the host B peer ($WORK/bridge or $EVID)
RELAY_PID=""            # nostr relay bg pid ("" = none)
REVERSED=0              # adb reverse tcp:$RELAY_PORT currently installed
DEV_PID=""              # bg instrumentation (device) / bg serve peer (selfcheck)
FLOW=""                 # current flow name inside `all` (per-flow log naming)

# ---------------------------------------------------------------- logging
logt() { printf '[%s] %s\n' "$(date -u +%H:%M:%S)" "$*"; }
die()  { logt "FATAL: $*" >&2; exit 2; }
note() { logt "note: $*"; }

# ---------------------------------------------------------------- args
usage() {
  cat <<'EOF'
usage: e2e.sh iroh|nostr|edges|all [--selfcheck | --device <serial>] [--keep]

  iroh    DocTicket live pairing (device test = serve side, host sync-peer joins)
  nostr   local-relay spool round-trip (A pushes, B joins, B pushes back)
  edges   device-only instrumentation edge cases (no selfcheck equivalent)
  all     iroh, then nostr, then edges (selfcheck: iroh + nostr only)

  --selfcheck        host<->host only, zero adb (DEFAULT)
  --device <serial>  bench device; must be `adb devices` state "device"
  --keep             skip evidence cleanup (device files/e2e + host tmp dirs)

env: E2E_STUB=1 = STUB HARNESS — proves orchestration logic only, NOT real sync.
EOF
  exit "${1:-1}"
}

parse_args() {
  [ $# -ge 1 ] || usage
  MODE="$1"; shift
  case "$MODE" in iroh|nostr|edges|all) ;; *) usage ;; esac
  while [ $# -gt 0 ]; do
    case "$1" in
      --selfcheck) TARGET="selfcheck"; shift ;;
      --device)
        [ $# -ge 2 ] || die "--device requires an explicit serial"
        TARGET="device"; SERIAL="$2"; shift 2 ;;
      --keep) KEEP=1; shift ;;
      -h|--help) usage 0 ;;
      *) die "unknown arg: $1" ;;
    esac
  done
}

# ---------------------------------------------------------------- device guard
assert_device_allowed() {
  [ -n "$SERIAL" ] || die "--device requires an explicit serial"
  [ "$SERIAL" != "$BANNED_SERIAL" ] || \
    die "serial $BANNED_SERIAL is the owner's daily-driver Pixel 6 — HARD-REFUSED. Bench Pixel 7a only (see README.md)."
  command -v adb >/dev/null || die "adb not on PATH"
  adb devices | awk -v s="$SERIAL" '$1==s && $2=="device" {ok=1} END{exit !ok}' || \
    die "serial '$SERIAL' not present with state 'device' in \`adb devices\`. Refusing."
  note "device guard ok: $SERIAL state=device"
}

# adb wrapper: ALWAYS pinned to $SERIAL.
adb_s() { adb -s "$SERIAL" "$@"; }

# ---------------------------------------------------------------- run-as channel ("greek-sync-e2e v1" helpers)
# Device:   dev_read  = run-as com.greekapp cat files/e2e/<rel>
#           dev_writef = run-as com.greekapp sh -c 'cat > files/e2e/<rel>' (stdin)
# Selfcheck: identical helpers on $EVID (the shared --dir).
dev_read() {
  if [ "$TARGET" = "selfcheck" ]; then
    cat "$EVID/$1" 2>/dev/null
  else
    adb_s shell "run-as $APP_ID cat files/e2e/$1" 2>/dev/null | tr -d '\r'
  fi
}

dev_writef() { # stdin → files/e2e/$1
  if [ "$TARGET" = "selfcheck" ]; then
    cat > "$EVID/$1"
  else
    adb_s shell "run-as $APP_ID sh -c 'cat > files/e2e/$1'"
  fi
}

_dev_read_ok() { # non-empty; *.json additionally must parse (guards mid-write reads)
  local rel="$1" content="$2"
  [ -n "$content" ] || return 1
  case "$rel" in
    *.json) printf '%s' "$content" | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null ;;
    *) return 0 ;;
  esac
}

dev_poll_file() { # $1 rel, $2 timeout_s, $3 optional dest file. Prints content if no dest.
  local rel="$1" timeout_s="$2" dest="${3:-}"
  local deadline=$(( $(date +%s) + timeout_s )) content
  note "poll files/e2e/$rel (every 2s, up to ${timeout_s}s)"
  while :; do
    content="$(dev_read "$rel" || true)"
    if _dev_read_ok "$rel" "$content"; then
      logt "got files/e2e/$rel"
      if [ -n "$dest" ]; then printf '%s\n' "$content" > "$dest"; else printf '%s' "$content"; fi
      return 0
    fi
    [ "$(date +%s)" -lt "$deadline" ] || return 1
    sleep 2
  done
}

# ---------------------------------------------------------------- host runners (E2E_STUB aware)
run_sync_peer() { # args: serve|join [--nostr --relay U] --dir D --db DB
  if [ "${E2E_STUB:-0}" = "1" ]; then
    timeout "$HOST_TIMEOUT_S" "$STUBDIR/sync-peer-stub" "$@"
  else
    (cd "$REPO_ROOT" && timeout "$HOST_TIMEOUT_S" \
      cargo run -p uniffi-bindings --example sync-peer -- "$@")
  fi
}

run_relay() { # $1: port → sets RELAY_PID; blocks until RELAY_READY
  local logf="$RUN/host.log"
  if [ "${E2E_STUB:-0}" = "1" ]; then
    "$STUBDIR/relay-stub" "$1" >>"$logf" 2>&1 &
  else
    (cd "$REPO_ROOT" && \
      cargo run -p sync-nostr --example local_relay -- --port "$1") >>"$logf" 2>&1 &
  fi
  RELAY_PID=$!
  note "relay pid $RELAY_PID port $1 — waiting for RELAY_READY (cap ${RELAY_WAIT_S}s)"
  local deadline=$(( $(date +%s) + RELAY_WAIT_S ))
  while :; do
    grep -q "RELAY_READY" "$logf" 2>/dev/null && { logt "relay ready"; return 0; }
    kill -0 "$RELAY_PID" 2>/dev/null || die "relay exited before RELAY_READY (see $RUN/host.log)"
    [ "$(date +%s)" -lt "$deadline" ] || die "relay RELAY_READY timeout (${RELAY_WAIT_S}s)"
    sleep 1
  done
}

iroh_dns_ok() { timeout 5 getent hosts staging-euw1-1.relay.n0.iroh.network >/dev/null 2>&1; }

device_preflight() { # kill stale app, wipe old evidence
  assert_device_allowed
  if adb_s shell pidof "$PKG" 2>/dev/null | grep -q '[0-9]'; then
    note "stale $PKG process — force-stop"
    adb_s shell am force-stop "$PKG" || true
  fi
  adb_s shell "run-as $APP_ID rm -rf files/e2e" 2>/dev/null \
    && note "wiped device files/e2e" \
    || note "run-as wipe failed (app not installed yet? continuing)"
}

gradle_install() { # gradle tasks...
  [ "${E2E_STUB:-0}" != "1" ] || { note "E2E_STUB: skipping gradle $*"; return 0; }
  note "gradle: $*"
  (cd "$REPO_ROOT/android" && ./gradlew "$@")
}

dev_log_file() {
  if [ -n "$FLOW" ]; then echo "$RUN/device.${FLOW}.logcat.txt"
  else echo "$RUN/device.logcat.txt"; fi
}

instrument_bg() { # $1: test method; rest: optional extra `am instrument` args (-e k v ...)
  local method="$1"; shift
  local logf; logf="$(dev_log_file)"; : > "$logf"
  adb_s shell am instrument -w \
    -e class "${TEST_CLASS}#$method" \
    "$@" \
    "$TEST_RUNNER" >>"$logf" 2>&1 &
  DEV_PID=$!
  note "instrument #$method pid $DEV_PID → $logf"
}

# ---------------------------------------------------------------- compare (python3 — jq deliberately NOT used)
# Frozen export shape: {"uuids":["<uuid>", ...]}; a top-level JSON array is
# accepted as interop safety net. PASS requires equal AND non-empty sets.
compare_exports() { # $1 a, $2 b → echo "<|A|> <|B|> <true|false>"; rc 0 iff equal & non-empty
  python3 - "$1" "$2" <<'PY'
import json, sys
def uuids(p):
    with open(p) as f: d = json.load(f)
    if isinstance(d, list): return set(map(str, d))
    if isinstance(d, dict):
        if isinstance(d.get("uuids"), list):
            return set(map(str, d["uuids"]))
        # SyncBatch v1 (what GreekCore.sync_export_backup_json emits):
        # {"v":1, ..., "reviews":[{"uuid": ...}, ...]}
        if isinstance(d.get("reviews"), list):
            return set(str(r["uuid"]) for r in d["reviews"])
    raise SystemExit(f"compare: no uuid list in {p}")
a, b = uuids(sys.argv[1]), uuids(sys.argv[2])
print(f"{len(a)} {len(b)} {'true' if a == b else 'false'}")
sys.exit(0 if (a and a == b) else 1)
PY
}

# Shared verdict: pulls exports, compares, prints the canonical PASS/FAIL line.
# $1 label  $2 device exit  $3 host exit  → rc 0 = PASS
finalize_verdict() {
  local label="$1" dev_exit="$2" host_exit="$3" rc=1
  local exp_a="$RUN/exports/export_a.json" exp_b="$RUN/exports/export_b.json"
  local counts="? ? unknown" out="" cmp_rc=0

  # Pull final exports: A's from device evidence, B's from the bridge dir.
  dev_poll_file export_a.json 30 "$exp_a" || note "export_a.json missing"
  if [ -f "$BRIDGE/export_b.json" ]; then
    cp "$BRIDGE/export_b.json" "$exp_b"
  else
    note "export_b.json missing in $BRIDGE"
  fi

  if [ -s "$exp_a" ] && [ -s "$exp_b" ]; then
    out="$(compare_exports "$exp_a" "$exp_b" 2>/dev/null)" || cmp_rc=$?
    [ -n "$out" ] && counts="$out"
    if [ "$cmp_rc" = "0" ] && [ "$dev_exit" = "0" ] && [ "$host_exit" = "0" ]; then rc=0; fi
  fi

  local na nb eq
  IFS=' ' read -r na nb eq <<<"$counts"
  local p="FAIL"; [ "$rc" = "0" ] && p="PASS"
  echo "${label}: ${p} (device exit=${dev_exit}, host exit=${host_exit}, |uuidsA|=${na:-?}, |uuidsB|=${nb:-?}, setEqual=${eq:-unknown})"
  return "$rc"
}

# ---------------------------------------------------------------- cleanup
cleanup_nostr() {
  if [ -n "$RELAY_PID" ]; then kill "$RELAY_PID" 2>/dev/null || true; RELAY_PID=""; fi
  if [ "$REVERSED" = "1" ]; then adb_s reverse --remove "tcp:$DEVICE_RELAY_PORT" 2>/dev/null || true; REVERSED=0; fi
}

evidence_cleanup() { # end-of-run only; device flows re-wipe in device_preflight
  if [ "$KEEP" = "1" ]; then
    note "--keep: evidence left in place (device files/e2e and $WORK)"
    return 0
  fi
  if [ "$TARGET" = "device" ]; then
    # re-check the guard here too: never let the EXIT trap touch the banned serial
    [ -n "$SERIAL" ] && [ "$SERIAL" != "$BANNED_SERIAL" ] || return 0
    adb devices 2>/dev/null | awk -v s="$SERIAL" '$1==s && $2=="device" {ok=1} END{exit !ok}' \
      || return 0
    adb_s shell "run-as $APP_ID rm -rf files/e2e" 2>/dev/null || true
  elif [ -n "$EVID" ]; then
    rm -rf "$EVID"
  fi
  [ -n "$WORK" ] && rm -rf "$WORK"
  return 0
}

on_exit() {
  local rc=$?
  set +e
  cleanup_nostr
  evidence_cleanup
  logt "exit rc=$rc — logs: $RUN"
  exit $rc
}
trap on_exit EXIT

# ---------------------------------------------------------------- flow: iroh
flow_iroh() {
  local host_exit="n/a" dev_exit="n/a"
  local instr_extra=()

  # Netmode selection: selfcheck is hermetic (loopback = presets::Empty,
  # no relay, no DNS). Device keeps the production N0 path, unless host
  # DNS cannot reach the n0 relays (filtered resolver) — then require the
  # operator to opt into loopback explicitly (works phone↔host same LAN).
  if [ "$TARGET" != "device" ]; then
    export GREEK_IROH_NET=loopback
    note "selfcheck: GREEK_IROH_NET=loopback (hermetic: no WAN/DNS needed)"
  elif ! iroh_dns_ok; then
    if [ "${E2E_IROH_NET:-}" = "loopback" ]; then
      note "WARN: n0 relay DNS unreachable — E2E_IROH_NET=loopback: both sides dial direct LAN addrs"
      export GREEK_IROH_NET=loopback
      instr_extra=(-e GREEK_IROH_NET loopback)
    else
      die "host DNS cannot resolve n0 iroh relays (getent failed for staging-euw1-1.relay.n0.iroh.network). Fix the resolver/VPN, or if phone+host share a LAN rerun with E2E_IROH_NET=loopback"
    fi
  fi

  if [ "$TARGET" = "device" ]; then
    device_preflight
    gradle_install :app:installDebug :app:installDebugAndroidTest
    instrument_bg irohLivePairingE2E "${instr_extra[@]}"
  else
    echo "--- selfcheck A+B := sync-peer inproc --dir $EVID --db $WORK/idbs (bg)"
    run_sync_peer inproc --dir "$EVID" --db "$WORK/idbs" >>"$RUN/host.log" 2>&1 &
    DEV_PID=$!   # inproc: one process plays BOTH halves of the protocol
  fi

  if [ "$TARGET" = "device" ]; then
    # Frozen contract: A (device) publishes ticket.txt + seeds_a.json.
    # Materialize both into the bridge dir for host B (b_pushed.flag unused here).
    dev_poll_file ticket.txt  "$POLL_TIMEOUT_S" "$BRIDGE/ticket.txt"   || \
      die "ticket.txt never appeared (iroh endpoint up? N0/DERP relay reachable?)"
    dev_poll_file seeds_a.json "$POLL_TIMEOUT_S" "$BRIDGE/seeds_a.json" || \
      die "seeds_a.json never appeared"

    # B's db gets its OWN dir: iroh persists the endpoint key at
    # <db-parent>/iroh-sync — sharing a parent with A = same NodeId = join dials itself.
    echo "--- B := sync-peer join --dir $BRIDGE --db $WORK/host_b/greek_app.db"
    host_exit=0
    run_sync_peer join --dir "$BRIDGE" --db "$WORK/host_b/greek_app.db" >>"$RUN/host.log" 2>&1 \
      || host_exit=$?
    logt "host B exit=$host_exit"
  else
    # inproc: one process drives A+B; its exit code is the verdict for both.
    note "selfcheck: waiting on inproc pair (both halves in one process)"
    host_exit=0
  fi

  dev_exit=0
  wait "$DEV_PID" || dev_exit=$?
  logt "A/instrument exit=$dev_exit"
  if [ "$TARGET" != "device" ]; then host_exit="$dev_exit"; fi

  local rc=0
  finalize_verdict E2E-IROH "$dev_exit" "$host_exit" || rc=$?
  return "$rc"
}

# ---------------------------------------------------------------- flow: nostr
gen_nsec_hex() {
  # 32 random bytes, hex. Not strict BIP-340 keygen; invalid-curve risk ~2^-128 —
  # acceptable for throwaway test keys. (NEVER reuse for a real identity.)
  python3 -c 'import secrets; print(secrets.token_hex(32))'
}

flow_nostr() {
  local host_exit="n/a" dev_exit="n/a"

  # Relay FIRST — device ws cleartext reaches it through `adb reverse`.
  resolve_relay_port
  run_relay "$RELAY_PORT"

  local nsec_hex; nsec_hex="$(gen_nsec_hex)"

  if [ "$TARGET" = "device" ]; then
    adb_s reverse "tcp:$DEVICE_RELAY_PORT" "tcp:$RELAY_PORT" && REVERSED=1
    note "adb reverse: device tcp:$DEVICE_RELAY_PORT → host tcp:$RELAY_PORT"
    device_preflight
    gradle_install :app:installDebug :app:installDebugAndroidTest
    # Device test polls nsec_in.hex FIRST — deliver before instrumenting.
    # (Written after preflight's files/e2e wipe, which is why it sits here.)
    printf '%s\n' "$nsec_hex" | dev_writef nsec_in.hex          # device sandbox
    printf '%s\n' "$nsec_hex" > "$BRIDGE/nsec_in.hex"           # bridge copy for host B
    note "wrote nsec_in.hex (device + bridge)"
    instrument_bg nostrLocalRelayE2E
  else
    printf '%s\n' "$nsec_hex" > "$BRIDGE/nsec_in.hex"
    note "wrote nsec_in.hex (bridge)"
    echo "--- A := sync-peer serve --nostr --relay $RELAY_URL --dir $BRIDGE (bg)"
    run_sync_peer serve --nostr --relay "$RELAY_URL" \
      --dir "$BRIDGE" --db "$WORK/host_a_nostr.db" >>"$RUN/host.log" 2>&1 &
    DEV_PID=$!
  fi

  # A pushed its spool to the relay → B may fetch.
  dev_poll_file push_done.flag "$POLL_TIMEOUT_S" >/dev/null || \
    die "push_done.flag never appeared (relay ready? ws cleartext blocked? check debug networkSecurityConfig)"

  echo "--- B := sync-peer join --nostr --relay $RELAY_URL --dir $BRIDGE --db $WORK/host_b_nostr.db"
  host_exit=0
  run_sync_peer join --nostr --relay "$RELAY_URL" \
    --dir "$BRIDGE" --db "$WORK/host_b_nostr.db" >>"$RUN/host.log" 2>&1 \
    || host_exit=$?
  logt "host B exit=$host_exit"

  # Signal A that B's spool is on the relay; A fetches, merges, exports.
  printf '1\n' | dev_writef b_pushed.flag

  dev_exit=0
  wait "$DEV_PID" || dev_exit=$?
  logt "A/instrument exit=$dev_exit"

  cleanup_nostr

  local rc=0
  finalize_verdict E2E-NOSTR "$dev_exit" "$host_exit" || rc=$?
  return "$rc"
}

# ---------------------------------------------------------------- flow: edges (device only)
flow_edges() {
  if [ "$TARGET" != "device" ]; then
    note "edges has no selfcheck equivalent — skipping (exit 0)"
    return 0
  fi
  local logf; logf="$(dev_log_file)"; : > "$logf"
  device_preflight
  gradle_install :app:installDebugAndroidTest

  local rc=0
  adb_s shell am instrument -w \
    -e class "${TEST_CLASS}#syncEdgeCases" \
    "$TEST_RUNNER" >"$logf" 2>&1 || rc=$?
  logt "instrument exit=$rc"

  echo "--- raw INSTRUMENTATION_STATUS tail ($logf):"
  grep -E "INSTRUMENTATION_(STATUS|RESULT)" "$logf" | tail -n 40 || true

  if [ "$rc" = "0" ] && grep -q "INSTRUMENTATION_CODE: -1" "$logf"; then
    echo "E2E-EDGES: PASS"; return 0
  fi
  echo "E2E-EDGES: FAIL (instrument exit=$rc; see $logf)"
  return 1
}

# ---------------------------------------------------------------- main
main() {
  parse_args "$@"
  case "${E2E_STUB:-0}" in
    0) ;;
    1) [ -x "$STUBDIR/sync-peer-stub" ] || die "E2E_STUB=1 but $STUBDIR/sync-peer-stub missing" ;;
    *) die "E2E_STUB must be 0 or 1" ;;
  esac
  [ "$TARGET" != "device" ] || assert_device_allowed   # fail fast, before run-dir init

  # run dir + transcript
  local ts; ts="$(date -u +%Y%m%dT%H%M%SZ)"
  RUN="$E2E_ROOT/${ts}-${MODE}"
  while [ -e "$RUN" ]; do RUN="${RUN}-$$"; done
  WORK="$RUN/tmp"
  mkdir -p "$RUN/exports" "$WORK"
  exec > >(tee -a "$RUN/transcript.log") 2>&1

  if [ "$TARGET" = "device" ]; then
    BRIDGE="$WORK/bridge"; mkdir -p "$BRIDGE"
  else
    EVID="$WORK/e2e"; mkdir -p "$EVID"
    BRIDGE="$EVID"          # selfcheck: shared dir IS the bridge
  fi

  logt "mode=$MODE target=$TARGET serial=${SERIAL:-none} stub=${E2E_STUB:-0} keep=$KEEP run=$RUN"
  [ "${E2E_STUB:-0}" != "1" ] || note "E2E_STUB=1 — stubs from $STUBDIR (orchestration logic only, NOT real sync)"

  local rc=0
  case "$MODE" in
    iroh)  flow_iroh  || rc=$? ;;
    nostr) flow_nostr || rc=$? ;;
    edges) flow_edges || rc=$? ;;
    all)
      local fails=() f v
      for f in iroh nostr edges; do
        if [ "$f" = "edges" ] && [ "$TARGET" != "device" ]; then
          note "all/selfcheck: edges skipped (no selfcheck equivalent)"; continue
        fi
        FLOW="$f"
        echo "================ flow: $f ================"
        v=0; "flow_$f" || v=$?
        # stash per-flow artifacts under exports/<flow>/ and device.<flow>.logcat.txt
        [ -f "$RUN/exports/export_a.json" ] && mkdir -p "$RUN/exports/$f" && \
          mv -f "$RUN/exports/export_a.json" "$RUN/exports/$f/" || true
        [ -f "$RUN/exports/export_b.json" ] && mkdir -p "$RUN/exports/$f" && \
          mv -f "$RUN/exports/export_b.json" "$RUN/exports/$f/" || true
        [ "$v" = "0" ] || { rc=1; fails+=("$f"); }
      done
      FLOW=""
      if [ "${#fails[@]}" -eq 0 ]; then
        echo "E2E-ALL: PASS"
      else
        echo "E2E-ALL: FAIL (failed: ${fails[*]})"; rc=1
      fi
      ;;
  esac
  return "$rc"
}

main "$@"
