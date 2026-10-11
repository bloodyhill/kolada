#!/usr/bin/env bash
# "What is behind, and does it pass?" for one platform, in one command:
# the port queue (what the platform has not ported yet), then the shared
# crates' tests, then the platform's own shell tests. Exits non-zero if any
# suite fails. See docs/porting.md.
#
#   tools/port-check.sh <mac|ios|linux|win>
#
# Tests build into $CARGO_TARGET_DIR, or target/test, so a running app is not
# locked. notsy-ai (the writing model) needs cmake and the llama.cpp
# submodule: add it with NOTSY_CHECK_AI=1.
set -uo pipefail

usage() { echo "usage: tools/port-check.sh <mac|ios|linux|win>" >&2; exit 2; }
platform=${1:-}
case "$platform" in mac | ios | linux | win) ;; *) usage ;; esac

cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-target/test}

shared="notsy-core notsy-markdown notsy-editor notsy-text notsy-ui notsy-convert notsy-assist notsy-app"
[ -n "${NOTSY_CHECK_AI:-}" ] && shared="$shared notsy-ai"
case "$platform" in
    win) shell=notsy-win ;;
    mac) shell=notsy-mac ;;
    *) shell= ;; # notsy-ios and notsy-linux do not exist yet
esac

echo "== Port queue: $platform"
tools/port-queue.sh "$platform" || echo "(no queue for $platform yet)"

failed=""
run() { # run <label> <cargo test args...>
    local label=$1
    shift
    echo
    echo "== $label"
    # --no-fail-fast: one failing test binary does not hide the others.
    if cargo test --release --no-fail-fast "$@"; then
        results="$results\n  pass  $label"
    else
        results="$results\n  FAIL  $label"
        failed=1
    fi
}

results=""
args=""
for c in $shared; do args="$args -p $c"; done
# shellcheck disable=SC2086
run "shared crates" $args
if [ -n "$shell" ]; then
    run "$platform shell ($shell)" -p "$shell"
else
    results="$results\n  skip  $platform shell (not started: no crate yet)"
fi

echo
echo "== Result for $platform"
printf '%b\n' "$results"
[ -z "$failed" ] && echo "All green." || { echo "Something fails: see above."; exit 1; }
