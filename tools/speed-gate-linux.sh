#!/usr/bin/env bash
# The speed gate for Linux: the promises of tools/speed-gate.sh, checked on
# Linux, failing (exit 1) when one is broken. Run it on a Linux desktop (or
# under Xvfb: with no display it starts itself again under xvfb-run) before
# pushing work near startup, drawing or typing (CLAUDE.md). Test instances
# have their own data (a HOME per vault under target/gate-linux), never the
# user's.
#
#   tools/speed-gate-linux.sh                  the budgets, and launch on this tree
#   tools/speed-gate-linux.sh --against REF    also launch against REF's build
#                                              (e.g. origin/main), alternating
#   tools/speed-gate-linux.sh --appimage dist/linux/Kolada-<v>-x86_64.AppImage
#                                              also launch from the AppImage
#
# It holds:
#   - a frame within 0.1 ms and a keystroke within 0.3 ms at 4K (the time_a_*
#     tests in crates/kolada-app/src/model/speed.rs, the same notes and
#     budgets as Windows' and the Mac's);
#   - launch to the first frame on the screen within LAUNCH_BUDGET ms
#     (median) with a 2,000-note vault, timed from before the program is
#     started (the loader included) to the frame presented;
#   - a 100,000-note vault within +SCALE_BUDGET ms of the 2,000 (the first
#     frame is O(1) in vault size);
#   - the session file each vault leaves under 16 KB;
#   - with --against: within +AB_BUDGET ms of REF, launched alternately so
#     machine load hits both alike.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
[ "$(uname)" = Linux ] || { echo "the Linux gate runs on Linux"; exit 2; }
if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    command -v xvfb-run >/dev/null || { echo "no display: run it on a desktop, or install xvfb"; exit 2; }
    exec xvfb-run -a -s "-screen 0 1920x1080x24" "$0" "$@"
fi

LAUNCH_BUDGET=${LAUNCH_BUDGET:-50}
SCALE_BUDGET=${SCALE_BUDGET:-5}
AB_BUDGET=${AB_BUDGET:-3}
RUNS=${RUNS:-15}
against=""
appimage=""
while [ $# -gt 0 ]; do
    case "$1" in
        --against) against=${2:?--against needs a commit}; shift 2 ;;
        --appimage) appimage=$(realpath "${2:?--appimage needs a file}"); shift 2 ;;
        *) echo "unknown: $1"; exit 2 ;;
    esac
done
target=${CARGO_TARGET_DIR:-$root/target}
gate="$root/target/gate-linux"
mkdir -p "$gate"

failed=0
fail() { echo "FAIL: $*"; failed=1; }
median() { sort -n | awk '{v[NR]=$1} END {if (NR) print (NR % 2 ? v[(NR+1)/2] : (v[NR/2] + v[NR/2+1]) / 2); else print "nan"}'; }

echo "== Frame and keystroke budgets (release build); load average: $(cut -d' ' -f1-3 /proc/loadavg)"
out=$(mktemp)
if cargo test -q --no-fail-fast -p kolada-app --release -- --ignored --test-threads=1 --nocapture time_a_ >"$out" 2>&1; then
    grep -E "median" "$out" | sed 's/^\.//'
else
    grep -E "median|over its|^error" "$out" | sed 's/^\.//' || true
    grep -q "over its" "$out" && fail "a frame or keystroke is over budget (above)" || { tail -15 "$out"; fail "the timing tests did not run (above)"; }
fi
rm -f "$out"

echo "== Launch (display: ${WAYLAND_DISPLAY:+wayland $WAYLAND_DISPLAY }${DISPLAY:+x11 $DISPLAY})"
cargo build -q -p kolada-linux -p vaultgen --release
exe="$target/release/kolada"
for v in 2k 100k; do
    if [ ! -d "vaults/gen-$v" ]; then
        n=$([ "$v" = 2k ] && echo 2000 || echo 100000)
        echo "making vaults/gen-$v ($n notes)..."
        "$target/release/vaultgen" "$n" "vaults/gen-$v" >/dev/null
    fi
done

export KOLADA_TEST_INSTANCE=1 KOLADA_TRACE=1 KOLADA_EXIT_AFTER_FIRST_FRAME=1
# X's key to the display is found from HOME unless named: name it before
# each launch gets a HOME of its own.
export XAUTHORITY=${XAUTHORITY:-$HOME/.Xauthority}
# Each vault gets data of its own (a home), as a user with that vault has.
home() { echo "$gate/home-$1"; }
launch() { # exe vault: one launch to its first frame; prints ms from before the start
    local exe=$1 v=$2 t0 at
    mkdir -p "$(home "$v")"
    t0=$(date +%s%N)
    at=$(HOME=$(home "$v") XDG_DATA_HOME=$(home "$v")/data timeout 30 "$exe" --vault "$root/vaults/gen-$v" 2>&1 >/dev/null | sed -n 's/^first frame presented: .* (at \([0-9]*\) ns).*/\1/p' | tail -1)
    [ -n "$at" ] && awk -v a="$at" -v b="$t0" 'BEGIN { printf "%.1f\n", (a - b) / 1e6 }'
}

for v in 2k 100k; do
    # A session as a user's would be: the app run once with the vault, then
    # quit (which saves it).
    rm -rf "$(home "$v")"
    launch "$exe" "$v" >/dev/null
    size=$(wc -c < "$(home "$v")/data/Kolada.test/session.bin" 2>/dev/null || echo 0)
    echo "session with $v notes: $size bytes"
    [ "$size" -gt 0 ] && [ "$size" -lt 16384 ] || fail "the session with $v notes is $size bytes (over 16 KB, or none)"
done

# Warm: the first runs of a new exe are cold.
for _ in 1 2 3; do launch "$exe" 2k >/dev/null; launch "$exe" 100k >/dev/null; done
small=(); big=()
for _ in $(seq "$RUNS"); do
    small+=("$(launch "$exe" 2k)")
    big+=("$(launch "$exe" 100k)")
done
m2k=$(printf '%s\n' "${small[@]}" | median)
m100k=$(printf '%s\n' "${big[@]}" | median)
echo "first frame on the screen, median of $RUNS: 2,000 notes $m2k ms, 100,000 notes $m100k ms (budget $LAUNCH_BUDGET ms); load average: $(cut -d' ' -f1-3 /proc/loadavg)"
awk -v m="$m2k" -v b="$LAUNCH_BUDGET" 'BEGIN { exit !(m <= b) }' || fail "2,000 notes launch in $m2k ms (budget $LAUNCH_BUDGET ms)"
awk -v a="$m2k" -v b="$m100k" -v s="$SCALE_BUDGET" 'BEGIN { exit !(b - a <= s) }' || fail "100,000 notes launch $(awk -v a="$m2k" -v b="$m100k" 'BEGIN { printf "%.1f", b - a }') ms slower than 2,000 (budget +$SCALE_BUDGET ms)"

if [ -n "$appimage" ]; then
    echo "== AppImage $(basename "$appimage")"
    # Mounted by its runtime at each launch, as a person's is.
    for _ in 1 2 3; do launch "$appimage" 2k >/dev/null; done
    runs=()
    for _ in $(seq "$RUNS"); do runs+=("$(launch "$appimage" 2k)"); done
    mai=$(printf '%s\n' "${runs[@]}" | median)
    echo "first frame on the screen from the AppImage, median of $RUNS: 2,000 notes $mai ms (budget $LAUNCH_BUDGET ms)"
    awk -v m="$mai" -v b="$LAUNCH_BUDGET" 'BEGIN { exit !(m <= b) }' || fail "the AppImage launches in $mai ms (budget $LAUNCH_BUDGET ms)"
fi

if [ -n "$against" ]; then
    echo "== Against $against"
    sha=$(git rev-parse --short "$against^{commit}")
    ref="$gate/ref-$sha"
    if [ ! -x "$ref/release/kolada" ]; then
        tree="$gate/tree-$sha"
        rm -rf "$tree"
        git worktree add -q --detach "$tree" "$sha"
        (cd "$tree" && cargo build -q -p kolada-linux --release --target-dir "$ref")
        git worktree remove --force "$tree"
    fi
    for _ in 1 2 3; do launch "$ref/release/kolada" 2k >/dev/null; launch "$exe" 2k >/dev/null; done
    old=(); new=()
    for _ in $(seq "$RUNS"); do
        old+=("$(launch "$ref/release/kolada" 2k)")
        new+=("$(launch "$exe" 2k)")
    done
    mo=$(printf '%s\n' "${old[@]}" | median)
    mn=$(printf '%s\n' "${new[@]}" | median)
    echo "first frame, median of $RUNS alternating: $sha $mo ms, this tree $mn ms"
    awk -v o="$mo" -v n="$mn" -v s="$AB_BUDGET" 'BEGIN { exit !(n - o <= s) }' || fail "this tree launches $(awk -v o="$mo" -v n="$mn" 'BEGIN { printf "%.1f", n - o }') ms slower than $sha (budget +$AB_BUDGET ms)"
fi

if [ "$failed" = 0 ]; then echo "PASS: every speed promise holds"; else exit 1; fi
