#!/usr/bin/env bash
# The speed gate for the Mac: the promises of tools/speed-gate.sh, checked on
# the Mac, failing (exit 1) when one is broken. Run it on a Mac before pushing
# work near startup, drawing or typing (CLAUDE.md). Test instances have their
# own data (a HOME per vault under target/gate-mac), never the user's.
#
#   tools/speed-gate-mac.sh                  the budgets, and launch on this tree
#   tools/speed-gate-mac.sh --against REF    also launch against REF's build
#                                            (e.g. origin/main), alternating
#
# It holds:
#   - a frame within 0.1 ms and a keystroke within 0.3 ms at 4K (the time_a_*
#     tests in crates/kolada-app/src/model/speed.rs, on stand-in fonts, and in
#     crates/kolada-mac/src/mac/speed.rs, on the Mac's own; the same notes and
#     budgets as Windows');
#   - Kolada's own launch overhead: the time to its first frame minus an empty
#     AppKit window's time to its first draw (tools/mac-launch-baseline.swift,
#     launched alternately with it), within OVERHEAD_BUDGET ms (median) with
#     a 2,000-note vault. AppKit alone is about 170 ms, so the 100 ms promise
#     is not reachable on the Mac by launching faster than AppKit; what is
#     held is that Kolada adds little to it;
#   - a 100,000-note vault within +SCALE_BUDGET ms of the 2,000 (the first
#     frame is O(1) in vault size);
#   - the session file each vault leaves under 16 KB;
#   - with --against: within +AB_BUDGET ms of REF, launched alternately so
#     machine load hits both alike.
#
# Launch numbers move with machine load (other builds on this Mac make it
# noisy: look at the load average printed): the alternating comparisons are
# the reliable ones, and a run that fails on launch is worth repeating.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
[ "$(uname)" = Darwin ] || { echo "the Mac gate runs on a Mac"; exit 2; }

OVERHEAD_BUDGET=${OVERHEAD_BUDGET:-30}
SCALE_BUDGET=${SCALE_BUDGET:-5}
AB_BUDGET=${AB_BUDGET:-3}
RUNS=${RUNS:-15}
against=""
[ "${1:-}" = "--against" ] && against=${2:?--against needs a commit}
target=${CARGO_TARGET_DIR:-$root/target}
gate="$root/target/gate-mac"
mkdir -p "$gate"

failed=0
fail() { echo "FAIL: $*"; failed=1; }
median() { sort -n | awk '{v[NR]=$1} END {if (NR) print (NR % 2 ? v[(NR+1)/2] : (v[NR/2] + v[NR/2+1]) / 2); else print "nan"}'; }

echo "== Frame and keystroke budgets (release build); load average: $(uptime | sed 's/.*averages*: //')"
out=$(mktemp)
if cargo test -q --no-fail-fast -p kolada-app -p kolada-mac --release -- --ignored --test-threads=1 --nocapture time_a_ >"$out" 2>&1; then
    grep -E "median" "$out" | sed 's/^\.//'
else
    # Over budget, or it did not build: say which.
    grep -E "median|over its|^error" "$out" | sed 's/^\.//' || true
    grep -q "over its" "$out" && fail "a frame or keystroke is over budget (above)" || { tail -15 "$out"; fail "the timing tests did not run (above)"; }
fi
rm -f "$out"

echo "== Launch"
cargo build -q -p kolada-mac -p vaultgen --release
exe="$target/release/kolada-mac"
base="$gate/Base"
if [ ! -x "$base" ] || [ tools/mac-launch-baseline.swift -nt "$base" ]; then
    swiftc -O tools/mac-launch-baseline.swift -o "$base" 2>/dev/null
fi
for v in 2k 100k; do
    if [ ! -d "vaults/gen-$v" ]; then
        n=$([ "$v" = 2k ] && echo 2000 || echo 100000)
        echo "making vaults/gen-$v ($n notes)..."
        "$target/release/vaultgen" "$n" "vaults/gen-$v" >/dev/null
    fi
done

export KOLADA_TEST_INSTANCE=1 KOLADA_TRACE=1 KOLADA_EXIT_AFTER_FIRST_FRAME=1
# Each vault gets data of its own (a home), as a user with that vault has.
home() { echo "$gate/home-$1"; }
launch() { # exe vault: one launch to its first frame; prints ms since the process began
    local exe=$1 v=$2
    mkdir -p "$(home "$v")"
    HOME=$(home "$v") perl -e 'alarm 30; exec @ARGV' "$exe" "$root/vaults/gen-$v" 2>&1 >/dev/null | sed -n 's/^first frame presented: \([0-9.]*\) ms.*/\1/p' | tail -1
}
launch_base() { # an empty AppKit window's time to its first draw
    perl -e 'alarm 30; exec @ARGV' "$base" 2>&1 >/dev/null | sed -n 's/^first draw: \([0-9.]*\) ms.*/\1/p' | tail -1
}

for v in 2k 100k; do
    # A session as a user's would be: the app run once with the vault, then
    # quit (which saves it).
    rm -rf "$(home "$v")"
    launch "$exe" "$v" >/dev/null
    size=$(wc -c < "$(home "$v")/Library/Application Support/Kolada.test/session.bin" 2>/dev/null || echo 0)
    echo "session with $v notes: $size bytes"
    [ "$size" -gt 0 ] && [ "$size" -lt 16384 ] || fail "the session with $v notes is $size bytes (over 16 KB, or none)"
done

# Warm: the first runs of a new exe are scanned and cold.
for _ in 1 2 3; do launch "$exe" 2k >/dev/null; launch "$exe" 100k >/dev/null; launch_base >/dev/null; done
small=(); big=(); empty=()
for _ in $(seq "$RUNS"); do
    small+=("$(launch "$exe" 2k)")
    big+=("$(launch "$exe" 100k)")
    empty+=("$(launch_base)")
done
m2k=$(printf '%s\n' "${small[@]}" | median)
m100k=$(printf '%s\n' "${big[@]}" | median)
mbase=$(printf '%s\n' "${empty[@]}" | median)
over=$(awk -v a="$m2k" -v b="$mbase" 'BEGIN { printf "%.1f", a - b }')
echo "first frame, median of $RUNS: 2,000 notes $m2k ms, 100,000 notes $m100k ms; an empty AppKit window $mbase ms"
echo "Kolada's own overhead over AppKit: $over ms (budget $OVERHEAD_BUDGET ms); load average: $(uptime | sed 's/.*averages*: //')"
awk -v m="$over" -v b="$OVERHEAD_BUDGET" 'BEGIN { exit !(m <= b) }' || fail "Kolada adds $over ms to AppKit's own launch (budget $OVERHEAD_BUDGET ms)"
awk -v a="$m2k" -v b="$m100k" -v s="$SCALE_BUDGET" 'BEGIN { exit !(b - a <= s) }' || fail "100,000 notes launch $(awk -v a="$m2k" -v b="$m100k" 'BEGIN { printf "%.1f", b - a }') ms slower than 2,000 (budget +$SCALE_BUDGET ms)"

if [ -n "$against" ]; then
    echo "== Against $against"
    sha=$(git rev-parse --short "$against^{commit}")
    ref="$gate/ref-$sha"
    if [ ! -x "$ref/release/kolada-mac" ]; then
        tree="$gate/tree-$sha"
        rm -rf "$tree"
        git worktree add -q --detach "$tree" "$sha"
        (cd "$tree" && cargo build -q -p kolada-mac --release --target-dir "$ref")
        git worktree remove --force "$tree"
    fi
    # The old build has no exit-after-first-frame: only a tree that has it can be compared.
    for _ in 1 2 3; do launch "$ref/release/kolada-mac" 2k >/dev/null; launch "$exe" 2k >/dev/null; done
    old=(); new=()
    for _ in $(seq "$RUNS"); do
        old+=("$(launch "$ref/release/kolada-mac" 2k)")
        new+=("$(launch "$exe" 2k)")
    done
    mo=$(printf '%s\n' "${old[@]}" | median)
    mn=$(printf '%s\n' "${new[@]}" | median)
    echo "first frame, median of $RUNS alternating: $sha $mo ms, this tree $mn ms"
    [ -n "$mo" ] || fail "$sha prints no first frame (it predates KOLADA_EXIT_AFTER_FIRST_FRAME: compare against a newer commit)"
    awk -v o="$mo" -v n="$mn" -v s="$AB_BUDGET" 'BEGIN { exit !(n - o <= s) }' || fail "this tree launches $(awk -v o="$mo" -v n="$mn" 'BEGIN { printf "%.1f", n - o }') ms slower than $sha (budget +$AB_BUDGET ms)"
fi

if [ "$failed" = 0 ]; then echo "PASS: every speed promise holds"; else exit 1; fi
