#!/usr/bin/env bash
# Stage the pinned morphology dictionary for Tauri desktop packaging (bug D2).
#
# Copies data/morphology.sqlite (the pinned, git-ignored reference artifact)
# into desktop/src-tauri/resources/morphology.sqlite, verifying the SHA-256
# pinned in data/MANIFEST BEFORE and AFTER the copy. Any missing artifact or
# hash mismatch fails loudly so `tauri build` cannot ship an empty dictionary.
# Wired in as tauri.conf.json's beforeBuildCommand; safe to run by hand.

set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$repo/data/morphology.sqlite"
dst_dir="$repo/desktop/src-tauri/resources"
dst="$dst_dir/morphology.sqlite"

remediation() {
  cat >&2 <<'EOF'

Remediation: fetch the pinned dictionary artifact into data/morphology.sqlite
(the pinned SHA-256 is in data/MANIFEST; provenance in data/README.md — the
file is git-ignored on purpose and distributed out-of-band, e.g. a GitHub
Release download), verify it, then re-run this step:
    sha256sum -c data/MANIFEST --ignore-missing   # must print "OK"
    bash scripts/desktop-stage-morphology.sh
EOF
}

fail() {
  echo "desktop-stage-morphology: ERROR: $*" >&2
  remediation
  exit 1
}

[ -f "$src" ] || fail "pinned artifact missing: $src"

# Verify the source against the pinned hash (canonical pin: data/MANIFEST).
(cd "$repo" && sha256sum -c --quiet --ignore-missing data/MANIFEST) \
  || fail "data/morphology.sqlite does not match the SHA-256 pinned in data/MANIFEST — refusing to stage a corrupted artifact"

mkdir -p "$dst_dir"
cp "$src" "$dst"

# Verify the staged copy too (belt and suspenders: the .deb consumes THIS
# file, so its hash is what actually matters at release-gate time).
pinned="$(grep 'data/morphology\.sqlite' "$repo/data/MANIFEST" | awk '{print $1}')"
[ -n "$pinned" ] || fail "no morphology.sqlite entry found in data/MANIFEST"
(cd "$dst_dir" && echo "$pinned  morphology.sqlite" | sha256sum -c --quiet -) \
  || fail "staged copy failed hash verification — aborting packaging"

echo "desktop-stage-morphology: staged $dst ($(stat -c%s "$dst") bytes), hash OK ($pinned)"
