#!/usr/bin/env bash
# Runs a Driver.swift script (apps/ios/Kolada/Driver.swift lists the steps)
# in a simulator: the app built for the simulator is installed, given a
# generated vault and files, and launched with the script; the log and the
# pictures it saves land beside the script.
#
#   tools/ios-drive.sh <script> [options]
#     --device NAME   the simulator (default: iPad Pro 13-inch (M5))
#     --vault DIR     copied in as the app's own vault (Documents/Kolada)
#     --files DIR     copied to the app's tmp/drive-files (dragged in, opened)
#     --fresh         the app removed first (its data and session with it)
#     --env K=V       passed to the app (repeatable)
#     --wait S        the most seconds to wait for the script (default 120)
#     --no-build      use the app built last
set -euo pipefail
cd "$(dirname "$0")/.."

script="" device="iPad Pro 13-inch (M5)" vault="" files="" fresh="" wait=120 build=1
envs=()
while [ $# -gt 0 ]; do
    case "$1" in
        --device) device=$2; shift ;;
        --vault) vault=$2; shift ;;
        --files) files=$2; shift ;;
        --fresh) fresh=1 ;;
        --env) envs+=("SIMCTL_CHILD_$2"); shift ;;
        --wait) wait=$2; shift ;;
        --no-build) build="" ;;
        *) script=$1 ;;
    esac
    shift
done
[ -f "$script" ] || { echo "usage: tools/ios-drive.sh <script> [options]" >&2; exit 2; }
script=$(cd "$(dirname "$script")" && pwd)/$(basename "$script")
out=$(dirname "$script")

[ -n "$build" ] && tools/ios-build.sh >/dev/null
udid=$(xcrun simctl list devices available -j | python3 -c "import json,sys; d=json.load(sys.stdin)['devices']; print(next(x['udid'] for r in d.values() for x in r if x['name']==sys.argv[1]))" "$device")
xcrun simctl boot "$udid" 2>/dev/null || true
xcrun simctl bootstatus "$udid" -b >/dev/null
xcrun simctl terminate "$udid" app.kolada.ios 2>/dev/null || true
[ -n "$fresh" ] && xcrun simctl uninstall "$udid" app.kolada.ios 2>/dev/null || true
xcrun simctl install "$udid" target/ios/Kolada-sim.app
data=$(xcrun simctl get_app_container "$udid" app.kolada.ios data)
if [ -n "$vault" ]; then
    rm -rf "$data/Documents/Kolada"
    mkdir -p "$data/Documents"
    cp -Rp "$vault" "$data/Documents/Kolada"
fi
if [ -n "$files" ]; then
    rm -rf "$data/tmp/drive-files"
    mkdir -p "$data/tmp"
    cp -Rp "$files" "$data/tmp/drive-files"
fi
rm -f "$out/drive.log"
env SIMCTL_CHILD_KOLADA_DRIVE="$script" ${envs[@]+"${envs[@]}"} xcrun simctl launch --terminate-running-process "$udid" app.kolada.ios >/dev/null
for _ in $(seq 1 "$wait"); do
    grep -q '^done$' "$out/drive.log" 2>/dev/null && break
    sleep 1
done
cat "$out/drive.log" 2>/dev/null || echo "no log: the script did not start"
echo "data: $data"
