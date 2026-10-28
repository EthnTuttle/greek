#!/usr/bin/env bash
# lint-triage.sh — Android lint gate with the 4 known-and-triaged findings
# whitelisted. See scripts/ci/README.md.
#
# Context (production-readiness-2026-10-20.md §"Native compatibility and lint"):
# `:app:lintDebug` reports 4 errors, 57 warnings. The errors are:
#   1-3. `NewApi` (java.lang.ref.Cleaner needs API 33; minSdk=26) in GENERATED
#        android/app/src/main/java/uniffi/uniffi_bindings/uniffi_bindings.kt —
#        uniffi-bindgen output that deliberately probes Cleaner via reflection
#        and falls back at runtime; regenerating bindings recreates them, so
#        hand-patching is futile.
#   4.   `ProduceState*` in ui/screens/SyncScreen.kt — audited as a false
#        positive (explicit state assignment exists).
#
# Fail-closed policy: the build fails iff lint reports any Error/Fatal finding
# that does NOT match one of the known signatures above. New real lint errors
# therefore still break CI; the generated-file findings do not.
#
# Line numbers are deliberately NOT part of the signature: the audit cites
# uniffi_bindings.kt:1831/1834/1840 and SyncScreen.kt:445, but the latter has
# already drifted (now ~:482); a line-only baseline would silently unpin.
#
# NOTE: no `lint-baseline.xml` is used — a baseline would freeze ALL findings,
# including the 57 warnings, hiding future regressions among them.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd -P)"
APP_DIR="$REPO_ROOT/android"
REPORT="$APP_DIR/app/build/reports/lint-results-debug.xml"

cd "$APP_DIR"

echo "== ./gradlew :app:lintDebug (exit code captured; triage below) =="
lint_rc=0
./gradlew :app:lintDebug || lint_rc=$?
echo "lintDebug exit code: $lint_rc"

if [ ! -f "$REPORT" ]; then
    echo "::error::lint XML report missing at $REPORT — lint infrastructure itself failed (not a triageable finding)"
    exit 1
fi

# Whitelist + verdict in python3 (repo convention: python3, never jq —
# see scripts/e2e/e2e.sh compare_exports for the precedent).
python3 - "$REPORT" <<'PY'
import sys
import xml.etree.ElementTree as ET

report = sys.argv[1]
tree = ET.parse(report)

def is_known(issue):
    iid = issue.get("id") or ""
    # An error issue carries at least one <location file="...">.
    files = [loc.get("file") or "" for loc in issue.iter("location")]
    in_generated_bindings = any(f.endswith("uniffi_bindings.kt") for f in files)
    msg = issue.get("message") or ""
    if iid == "NewApi" and in_generated_bindings and "Cleaner" in msg:
        return True  # known: 3 generated uniffi Cleaner findings
    if iid.startswith("ProduceState") and any(f.endswith("SyncScreen.kt") for f in files):
        return True  # known: produceState false positive in SyncScreen.kt
    return False

known, unexpected = [], []
for issue in tree.getroot().iter("issue"):
    if issue.get("severity") not in ("Error", "Fatal"):
        continue  # warnings are informational in this gate (57 known as of audit)
    (known if is_known(issue) else unexpected).append(issue)

for it in known:
    loc = next(iter(it.iter("location")), None)
    print(f"known/allowed:  {it.get('id')}  {loc.get('file') if loc is not None else '?'}:{loc.get('line') if loc is not None else '?'}")
print(f"-- {len(known)} known triaged error(s) allowed")

if unexpected:
    print(f"::error::{len(unexpected)} UNEXPECTED lint error(s) (not in the known-triaged whitelist):", file=sys.stderr)
    for it in unexpected:
        loc = next(iter(it.iter("location")), None)
        print(f"  {it.get('id')} [{it.get('severity')}] {loc.get('file') if loc is not None else '?'}:{loc.get('line') if loc is not None else '?'} — {it.get('message')}", file=sys.stderr)
    sys.exit(1)

if known:
    print("lint triage: PASS (only known-and-triaged findings present)")
else:
    print("lint triage: PASS (no errors at all — the whitelist can be retired)")
PY
