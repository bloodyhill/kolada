#!/usr/bin/env bash
# Builds Notsy for iPad and iPhone without an Xcode project: the Rust engine
# (crates/notsy-ios) as a static library, the Swift shell (apps/ios/Notsy)
# compiled and linked against it, and the app bundle around them.
#
#   tools/ios-build.sh                 the simulator app, at target/ios/Notsy-sim.app
#   tools/ios-build.sh --run [device]  ... installed and opened in a simulator
#                                      (default: an iPad Pro 13-inch; booted if needed)
#   tools/ios-build.sh --device        the device app, unsigned (signing needs a team)
set -euo pipefail
cd "$(dirname "$0")/.."

mode=sim run="" device_name="iPad Pro 13-inch (M5)"
case "${1:-}" in
    --run) run=1; device_name=${2:-$device_name} ;;
    --device) mode=device ;;
esac
if [ "$mode" = sim ]; then
    rust=aarch64-apple-ios-sim swift=arm64-apple-ios17.0-simulator sdk=iphonesimulator
else
    rust=aarch64-apple-ios swift=arm64-apple-ios17.0 sdk=iphoneos
fi

cargo build --release -p notsy-ios --target "$rust" --target-dir target/ios
lib="target/ios/$rust/release/libnotsy_ios.a"
app="target/ios/Notsy-$mode.app"
rm -rf "$app"
mkdir -p "$app"
xcrun --sdk "$sdk" swiftc -O -target "$swift" -sdk "$(xcrun --sdk "$sdk" --show-sdk-path)" \
    -module-name Notsy -parse-as-library -import-objc-header apps/ios/Notsy/notsy.h \
    apps/ios/Notsy/*.swift "$lib" -o "$app/Notsy"
cp apps/ios/Notsy/Info.plist "$app/Info.plist"
# The workspace's version, as the Mac and Windows builds carry it.
plutil -replace CFBundleShortVersionString -string "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)" "$app/Info.plist"
platform=$([ "$mode" = sim ] && echo iPhoneSimulator || echo iPhoneOS)
plutil -insert CFBundleSupportedPlatforms -json "[\"$platform\"]" "$app/Info.plist"
# The icon and the launch screen's paper or graphite (Assets.xcassets).
partial=$(mktemp)
xcrun actool apps/ios/Notsy/Assets.xcassets --compile "$app" --platform "$sdk" --minimum-deployment-target 17.0 \
    --target-device ipad --target-device iphone --app-icon AppIcon --output-partial-info-plist "$partial" >/dev/null
/usr/libexec/PlistBuddy -c "Merge $partial" "$app/Info.plist" >/dev/null
rm -f "$partial"
cp apps/ios/Notsy/PrivacyInfo.xcprivacy "$app/"
[ "$mode" = sim ] && codesign --force --sign - "$app" >/dev/null
echo "built $app ($(du -sh "$app" | cut -f1))"

if [ -n "$run" ]; then
    udid=$(xcrun simctl list devices available -j | python3 -c "import json,sys; d=json.load(sys.stdin)['devices']; print(next(x['udid'] for r in d.values() for x in r if x['name']==sys.argv[1]))" "$device_name")
    xcrun simctl boot "$udid" 2>/dev/null || true
    xcrun simctl bootstatus "$udid" -b >/dev/null
    xcrun simctl install "$udid" "$app"
    xcrun simctl launch --terminate-running-process "$udid" com.notsy.ios
    echo "running on $device_name ($udid)"
fi
