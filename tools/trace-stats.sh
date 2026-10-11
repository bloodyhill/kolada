#!/usr/bin/env bash
# Launches notsy N times with --trace --exit-after-first-frame and prints the
# median of every startup marker (ms since process creation).
#
# Usage: tools/trace-stats.sh [runs=20] [extra notsy args...]
# Internal markers only; the real gate number comes from notsy-bench.
set -euo pipefail
runs=${1:-20}
shift || true
exe="$(dirname "$0")/../target/release/notsy.exe"
log="$LOCALAPPDATA/Notsy/startup-trace.log"
rm -f "$log"
for _ in $(seq "$runs"); do
  "$exe" --trace --exit-after-first-frame "$@"
done
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
    # insertion sort
    for (i = 2; i <= m; i++) { x = v[i] + 0; j = i - 1; while (j > 0 && v[j] + 0 > x) { v[j + 1] = v[j]; j-- } v[j + 1] = x }
    printf "%-10s median %6.2f  (min %6.2f  max %6.2f, n=%d)\n", name, v[int((m + 1) / 2)], v[1], v[m], m
  }
}' "$log"
