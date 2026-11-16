#!/usr/bin/env bash
# Local release-verification runner. GitHub Actions is deliberately not used.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd -P)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd -P)"
cd "$REPO_ROOT"

usage() {
    cat <<'EOF'
Usage: scripts/ci/run-local.sh [--quick] [--only rust|desktop|android] [--release-check]

  --quick          Skip Android unit tests, polytonic audit, and lint.
  --only LEG       Run only rust, desktop, or android checks (skips artifact pin).
  --release-check  Also inspect an existing release APK for the required ABIs.
                   Build it first: (cd android && ./gradlew :app:assembleRelease)
EOF
}

quick=0
only=""
release_check=0
while (($# > 0)); do
    case "$1" in
        --quick)
            quick=1
            shift
            ;;
        --only)
            if (($# < 2)); then
                echo "ERROR: --only requires rust, desktop, or android" >&2
                usage >&2
                exit 2
            fi
            only="$2"
            shift 2
            ;;
        --release-check)
            release_check=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

case "$only" in
    ""|rust|desktop|android) ;;
    *)
        echo "ERROR: invalid --only selector: $only (want rust, desktop, or android)" >&2
        usage >&2
        exit 2
        ;;
esac
if ((quick == 1)) && [[ "$only" == "android" ]]; then
    echo "ERROR: --quick skips Android and cannot be combined with --only android" >&2
    exit 2
fi

run_rust=1
run_desktop=1
run_android=1
run_artifact=1
if [[ -n "$only" ]]; then
    run_rust=0
    run_desktop=0
    run_android=0
    run_artifact=0
    case "$only" in
        rust) run_rust=1 ;;
        desktop) run_desktop=1 ;;
        android) run_android=1 ;;
    esac
elif ((quick == 1)); then
    run_android=0
fi

pass_count=0
fail_count=0
skip_count=0

run_step() {
    local label="$1"
    shift
    printf '\n===== %s =====\n' "$label"
    if "$@"; then
        printf 'PASS: %s\n' "$label"
        pass_count=$((pass_count + 1))
    else
        local rc=$?
        printf 'FAIL: %s (exit %d)\n' "$label" "$rc"
        fail_count=$((fail_count + 1))
    fi
}

skip_step() {
    printf 'SKIP: %s (%s)\n' "$1" "$2"
    skip_count=$((skip_count + 1))
}

rust_tests() {
    cargo test --workspace --locked
}

desktop_contract_tests() {
    cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked \
        --test contract -- --test-threads=1
}

desktop_ui_tests() {
    (cd desktop/src-ui && node dev/test-ui.mjs)
}

android_tests() {
    (cd android && ./gradlew :app:testDebugUnitTest :app:polytonicAudit)
}

android_lint() {
    bash scripts/ci/lint-triage.sh
}

artifact_pins() {
    sha256sum -c data/MANIFEST --ignore-missing
}

release_abi_check() {
    local apk_dir="android/app/build/outputs/apk/release"
    local apk=""
    if [[ -d "$apk_dir" ]]; then
        apk="$(find "$apk_dir" -maxdepth 1 -type f -name '*.apk' -print -quit)"
    fi
    if [[ -z "$apk" ]]; then
        printf 'ERROR: no release APK found under %s.\n' "$apk_dir" >&2
        printf 'Build the unsigned release first (the keystore is git-ignored):\n' >&2
        printf '  (cd android && ./gradlew :app:assembleRelease)\n' >&2
        printf 'Then rerun with --release-check; this runner will not auto-assemble.\n' >&2
        return 1
    fi
    if ! command -v unzip >/dev/null 2>&1; then
        echo "ERROR: unzip is required to inspect $apk" >&2
        return 1
    fi

    printf 'Inspecting APK: %s\n' "$apk"
    local abis
    if ! abis="$(unzip -l "$apk" | awk '$4 ~ /^lib\/[^/]+\// {print $4}' \
        | cut -d/ -f2 | sort -u)"; then
        echo "ERROR: could not read APK ZIP entries from $apk" >&2
        return 1
    fi
    printf 'ABIs in APK lib/:\n'
    if [[ -n "$abis" ]]; then
        printf '%s\n' "$abis"
    else
        printf '(none)\n'
    fi

    local bad
    bad="$(printf '%s\n' "$abis" | grep -vE '^(arm64-v8a|x86_64)$' || true)"
    if [[ -n "$bad" ]]; then
        printf 'ERROR: disallowed APK ABI(s) (audit A5):\n%s\n' "$bad" >&2
        return 1
    fi

    local want
    for want in arm64-v8a x86_64; do
        if ! printf '%s\n' "$abis" | grep -qx "$want"; then
            printf 'ERROR: expected ABI %s missing from APK lib/.\n' "$want" >&2
            return 1
        fi
    done
    printf 'APK lib/ contains exactly the supported set: arm64-v8a + x86_64\n'
}

printf 'LOCAL VERIFICATION\nRepository: %s\n' "$REPO_ROOT"

run_step "Card data lint" python3 scripts/data-card-lint.py

if ((run_rust == 1)); then
    run_step "Rust workspace tests" rust_tests
else
    skip_step "Rust workspace tests" "not selected"
fi

if ((run_desktop == 1)); then
    run_step "Desktop contract tests" desktop_contract_tests
    run_step "Desktop UI harness" desktop_ui_tests
else
    skip_step "Desktop contract tests and UI harness" "not selected"
fi

if ((run_android == 1)); then
    run_step "Android unit tests and polytonic audit" android_tests
    run_step "Android lint triage" android_lint
elif ((quick == 1)); then
    skip_step "Android unit tests, polytonic audit, and lint triage" "--quick"
else
    skip_step "Android unit tests, polytonic audit, and lint triage" "not selected"
fi

if ((run_artifact == 1)); then
    run_step "Pinned artifact hashes" artifact_pins
else
    skip_step "Pinned artifact hashes" "not selected by --only"
fi

if ((release_check == 1)); then
    run_step "Release APK ABI check" release_abi_check
else
    skip_step "Release APK ABI check" "use --release-check to enable"
fi

printf '\n===== LOCAL VERIFICATION SUMMARY =====\n'
printf 'SUMMARY: PASS=%d FAIL=%d SKIP=%d\n' "$pass_count" "$fail_count" "$skip_count"
if ((fail_count > 0)); then
    printf 'RESULT: FAIL\n'
    exit 1
fi
printf 'RESULT: PASS\n'
