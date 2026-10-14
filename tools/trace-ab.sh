#!/usr/bin/env bash
# Interleaved A/B comparison of two kolada builds' startup markers. Runs
# alternate A, B, A, B... so drift in machine state (power, thermals,
# background work) hits both equally.
#
# Usage: tools/trace-ab.sh <a.exe> <b.exe> [pairs=20]
set -euo pipefail
a=$1
b=$2
pairs=${3:-20}
log="$LOCALAPPDATA/Kolada/startup-trace.log"
out=$(mktemp -d)
for _ in $(seq "$pairs"); do
  for side in a b; do
    rm -f "$log"
    exe=$a
    [ "$side" = b ] && exe=$b
    "$exe" --trace --exit-after-first-frame
    cat "$log" >> "$out/$side.log"
  done
done
summarize() {
  awk '
  {
    for (i = 1; i <= NF; i++) {
      split($i, kv, "=")
      if (kv[2] ~ /^[0-9.]+$/) {
        if (!(kv[1] in seen)) { order[++n] = kv[1]; seen[kv[1]] = 1 }
        vals[kv[1]] = vals[kv[1]] " " kv[2]
      }
    }
  }
  END {
    for (k = 1; k <= n; k++) {
      name = order[k]
      m = split(substr(vals[name], 2), v, " ")
      for (i = 2; i <= m; i++) { x = v[i] + 0; j = i - 1; while (j > 0 && v[j] + 0 > x) { v[j + 1] = v[j]; j-- } v[j + 1] = x }
      printf "%s %.2f\n", name, v[int((m + 1) / 2)]
    }
  }' "$1"
}
join <(summarize "$out/a.log" | nl -w3 -s' ' | sort -k2,2) <(summarize "$out/b.log" | sort -k1,1) -1 2 -2 1 \
  | sort -k2,2n | awk '{ printf "%-10s A %6.2f   B %6.2f   B-A %+6.2f\n", $1, $3, $4, $4 - $3 }'
rm -rf "$out"
