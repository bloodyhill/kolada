#!/usr/bin/env bash
# The speed gate: Notsy's speed promises, checked, failing (exit 1) when one
# is broken. Run it before pushing work near startup, drawing or typing
# (CLAUDE.md). It uses test instances with their own data, never the
# user's, and windows flash up briefly as they launch.
#
#   tools/speed-gate.sh                  the budgets, and launch on this tree
#   tools/speed-gate.sh --against REF    also launch against REF's build
#                                        (e.g. origin/main), alternating
#
# It holds:
#   - a frame within 0.1 ms and a keystroke within 0.3 ms at 4K (the
#     time_a_* tests in crates/notsy-win/src/cpu/tests.rs);
#   - launch to a visible window within LAUNCH_BUDGET ms (median) with a
#     2,000-note vault;
#   - a 100,000-note vault within +SCALE_BUDGET ms of the 2,000 (the first
#     frame is O(1) in vault size);
#   - the session file each vault leaves under 16 KB (a 120 KB one cost
#     8 ms a launch to antivirus scans, unnoticed);
#   - with --against: within +AB_BUDGET ms of REF, launched alternately so
#     machine load hits both alike.
#
# Launch numbers move with machine load; the alternating comparisons are
# the reliable ones. Windows-only for now: other platforms adapt the exe
# path and data directory.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD

LAUNCH_BUDGET=${LAUNCH_BUDGET:-50}
SCALE_BUDGET=${SCALE_BUDGET:-5}
AB_BUDGET=${AB_BUDGET:-3}
RUNS=${RUNS:-15}
against=""
[ "${1:-}" = "--against" ] && against=${2:?--against needs a commit}

failed=0
fail() { echo "FAIL: $*"; failed=1; }
median() { sort -n | awk '{v[NR]=$1} END {if (NR) print (NR % 2 ? v[(NR+1)/2] : (v[NR/2] + v[NR/2+1]) / 2); else print "nan"}'; }

echo "== Frame and keystroke budgets (release build)"
out=$(mktemp)
if cargo test -q -p notsy-win --release --target-dir target/test -- --ignored --test-threads=1 --nocapture time_a_ >"$out" 2>&1; then
    grep -E "median" "$out" | sed 's/^\.//'
else
    # Over budget, or it did not build: say which.
    grep -E "median|over its|^error" "$out" | sed 's/^\.//' || true
    grep -q "over its" "$out" && fail "a frame or keystroke is over budget (above)" || { tail -15 "$out"; fail "the timing tests did not run (above)"; }
fi
rm -f "$out"

echo "== Launch"
cargo build -q -p notsy-win --release --target-dir target/test
exe="$root/target/test/release/notsy.exe"
for v in 2k 100k; do
    if [ ! -d "vaults/gen-$v" ]; then
        n=$([ "$v" = 2k ] && echo 2000 || echo 100000)
        echo "making vaults/gen-$v ($n notes)..."
        cargo run -q --release -p vaultgen -- "$n" "vaults/gen-$v" >/dev/null
    fi
done

export NOTSY_TEST_INSTANCE=1
# Each vault gets data of its own, as a user with that vault has.
data() { echo "$root/target/gate/data-$1"; }
launch() { # exe vault: one launch to its first frame; prints ms to visible
    # As a user launches: the vault and note from the session, no --vault
    # (which reads the vault's top level for a note to open).
    local exe=$1 v=$2 d log
    d=$(data "$v")
    log="$d/Notsy/test/startup-trace.log"
    rm -f "$log"
    LOCALAPPDATA="$d" "$exe" --trace --exit-after-first-frame --no-activate
    sed -n 's/.*visible=\([0-9.]*\).*/\1/p' "$log" | tail -1
}

for v in 2k 100k; do
    d=$(data "$v")
    mkdir -p "$d"
    # A session as a user's would be: the app run once with the vault, its
    # tree read, then quit (which saves it).
    LOCALAPPDATA="$d" "$exe" --no-activate --vault "$root/vaults/gen-$v" &
    sleep 6
    LOCALAPPDATA="$d" "$exe" --quit || true
    wait 2>/dev/null || true
    size=$(wc -c < "$d/Notsy/test/session.bin" 2>/dev/null || echo 0)
    echo "session with $v notes: $size bytes"
    [ "$size" -gt 0 ] && [ "$size" -lt 16384 ] || fail "the session with $v notes is $size bytes (over 16 KB, or none)"
done

# Warm: the first runs of a new exe are scanned and cold.
for _ in 1 2 3; do launch "$exe" 2k >/dev/null; launch "$exe" 100k >/dev/null; done
small=(); big=()
for _ in $(seq "$RUNS"); do
    small+=("$(launch "$exe" 2k)")
    big+=("$(launch "$exe" 100k)")
done
m2k=$(printf '%s\n' "${small[@]}" | median)
m100k=$(printf '%s\n' "${big[@]}" | median)
echo "visible, median of $RUNS: 2,000 notes $m2k ms, 100,000 notes $m100k ms"
awk -v m="$m2k" -v b="$LAUNCH_BUDGET" 'BEGIN { exit !(m <= b) }' || fail "launch $m2k ms is over its $LAUNCH_BUDGET ms budget"
awk -v a="$m2k" -v b="$m100k" -v s="$SCALE_BUDGET" 'BEGIN { exit !(b - a <= s) }' || fail "100,000 notes launch $(awk -v a="$m2k" -v b="$m100k" 'BEGIN { printf "%.1f", b - a }') ms slower than 2,000 (budget +$SCALE_BUDGET ms)"

if [ -n "$against" ]; then
    echo "== Against $against"
    sha=$(git rev-parse --short "$against^{commit}")
    ref="$root/target/gate/ref-$sha"
    if [ ! -x "$ref/release/notsy.exe" ]; then
        tree="$root/target/gate/tree-$sha"
        rm -rf "$tree"
        git worktree add -q --detach "$tree" "$sha"
        (cd "$tree" && cargo build -q -p notsy-win --release --target-dir "$ref")
        git worktree remove --force "$tree"
    fi
    for _ in 1 2 3; do launch "$ref/release/notsy.exe" 2k >/dev/null; launch "$exe" 2k >/dev/null; done
    old=(); new=()
    for _ in $(seq "$RUNS"); do
        old+=("$(launch "$ref/release/notsy.exe" 2k)")
        new+=("$(launch "$exe" 2k)")
    done
    mo=$(printf '%s\n' "${old[@]}" | median)
    mn=$(printf '%s\n' "${new[@]}" | median)
    echo "visible, median of $RUNS alternating: $sha $mo ms, this tree $mn ms"
    awk -v o="$mo" -v n="$mn" -v s="$AB_BUDGET" 'BEGIN { exit !(n - o <= s) }' || fail "this tree launches $(awk -v o="$mo" -v n="$mn" 'BEGIN { printf "%.1f", n - o }') ms slower than $sha (budget +$AB_BUDGET ms)"
fi

if [ "$failed" = 0 ]; then echo "PASS: every speed promise holds"; else exit 1; fi
