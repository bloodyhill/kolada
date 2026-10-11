#!/usr/bin/env bash
# The iPad's launch, measured in the simulator: Notsy's first frame against
# an empty UIKit app's (apps/ios/Baseline), launched alternately, median of
# RUNS. Notsy's own cost is the difference. Needs tools/ios-build.sh first.
#
#   tools/ios-launch.sh [device name]
set -euo pipefail
cd "$(dirname "$0")/.."
RUNS=${RUNS:-9}
name=${1:-"iPad Pro 13-inch (M5)"}
udid=$(xcrun simctl list devices available -j | python3 -c "import json,sys; d=json.load(sys.stdin)['devices']; print(next(x['udid'] for r in d.values() for x in r if x['name']==sys.argv[1]))" "$name")
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null

base=target/ios/Baseline.app
rm -rf "$base" && mkdir -p "$base"
sdk=$(xcrun --sdk iphonesimulator --show-sdk-path)
xcrun --sdk iphonesimulator swiftc -O -target arm64-apple-ios17.0-simulator -sdk "$sdk" -parse-as-library apps/ios/Baseline/Baseline.swift -o "$base/Baseline"
sed -e 's/com.notsy.ios/com.notsy.baseline/' -e 's/<string>Notsy<\/string>/<string>Baseline<\/string>/g' apps/ios/Notsy/Info.plist > "$base/Info.plist"
plutil -insert CFBundleSupportedPlatforms -json '["iPhoneSimulator"]' "$base/Info.plist"
codesign --force --sign - "$base" >/dev/null
xcrun simctl install "$udid" "$base"
xcrun simctl install "$udid" target/ios/Notsy-sim.app

once() { # bundle id: ms to its first frame
    xcrun simctl terminate "$udid" "$1" 2>/dev/null || true
    SIMCTL_CHILD_NOTSY_TRACE=1 perl -e 'alarm 10; exec @ARGV' xcrun simctl launch --console-pty "$udid" "$1" 2>&1 | sed -n 's/^first frame presented: \([0-9.]*\) ms.*/\1/p' | head -1
}
median() { sort -n | awk '{v[NR]=$1} END {print (NR % 2 ? v[(NR+1)/2] : (v[NR/2] + v[NR/2+1]) / 2)}'; }
for _ in 1 2; do once com.notsy.ios >/dev/null; once com.notsy.baseline >/dev/null; done
n=(); b=()
for _ in $(seq "$RUNS"); do n+=("$(once com.notsy.ios)"); b+=("$(once com.notsy.baseline)"); done
mn=$(printf '%s\n' "${n[@]}" | median); mb=$(printf '%s\n' "${b[@]}" | median)
echo "first frame, median of $RUNS on $name: Notsy $mn ms, an empty UIKit app $mb ms; Notsy's own $(awk -v a="$mn" -v b="$mb" 'BEGIN { printf "%.1f", a - b }') ms"
