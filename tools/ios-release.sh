#!/usr/bin/env bash
# Packs Kolada for iPad and iPhone for TestFlight and the App Store: the
# device app (tools/ios-build.sh --device) with the build information the
# store reads, signed with the team's distribution certificate and profile,
# in an .ipa at target/ios/Kolada-<version>.ipa, and uploaded when an App
# Store Connect API key is given. Without a team it packs the .ipa unsigned,
# so everything up to the certificate is built and checked here.
#
#   tools/ios-release.sh
#
# Set, to ship (docs/ios-release.md says how to get each):
#   KOLADA_IOS_SIGN_ID="Apple Distribution: <Name> (<TEAMID>)"
#   KOLADA_IOS_PROFILE=/path/to/Kolada_App_Store.mobileprovision
#   KOLADA_ASC_KEY_ID=<key id> KOLADA_ASC_ISSUER=<issuer id>
#       an App Store Connect API key, its .p8 in ~/.appstoreconnect/private_keys/
set -euo pipefail
cd "$(dirname "$0")/.."

tools/ios-build.sh --device
app=target/ios/Kolada-device.app
plist="$app/Info.plist"
version=$(plutil -extract CFBundleShortVersionString raw "$plist")

# What Xcode would have written: the store checks the SDK and toolchain.
sdk_version=$(xcrun --sdk iphoneos --show-sdk-version)
sdk_build=$(xcrun --sdk iphoneos --show-sdk-build-version)
xcode_build=$(xcodebuild -version | sed -n 's/^Build version //p')
xcode_version=$(xcodebuild -version | sed -n 's/^Xcode //p' | awk -F. '{ printf "%d%d%d", $1, $2, $3 }')
for kv in "DTPlatformName:iphoneos" "DTPlatformVersion:$sdk_version" "DTPlatformBuild:$sdk_build" "DTSDKName:iphoneos$sdk_version" "DTSDKBuild:$sdk_build" \
          "DTXcode:$xcode_version" "DTXcodeBuild:$xcode_build" "DTCompiler:com.apple.compilers.llvm.clang.1_0" "BuildMachineOSBuild:$(sw_vers -buildVersion)"; do
    plutil -replace "${kv%%:*}" -string "${kv#*:}" "$plist"
done

if [ -n "${KOLADA_IOS_SIGN_ID:-}" ] && [ -n "${KOLADA_IOS_PROFILE:-}" ]; then
    cp "$KOLADA_IOS_PROFILE" "$app/embedded.mobileprovision"
    # The entitlements are the profile's own.
    entitlements=$(mktemp)
    security cms -D -i "$KOLADA_IOS_PROFILE" | plutil -extract Entitlements xml1 -o "$entitlements" -
    codesign --force --sign "$KOLADA_IOS_SIGN_ID" --entitlements "$entitlements" --timestamp=none "$app"
    rm -f "$entitlements"
    codesign --verify --strict "$app"
    signed=1
else
    signed=""
    echo "not signed (set KOLADA_IOS_SIGN_ID and KOLADA_IOS_PROFILE to ship)"
fi

ipa="target/ios/Kolada-$version.ipa"
stage=$(mktemp -d)
mkdir "$stage/Payload"
cp -R "$app" "$stage/Payload/Kolada.app"
rm -f "$ipa"
(cd "$stage" && zip -qry - Payload) > "$ipa"
rm -rf "$stage"
echo "packed $ipa ($(du -h "$ipa" | cut -f1))"

if [ -n "$signed" ] && [ -n "${KOLADA_ASC_KEY_ID:-}" ] && [ -n "${KOLADA_ASC_ISSUER:-}" ]; then
    xcrun altool --validate-app -f "$ipa" -t ios --apiKey "$KOLADA_ASC_KEY_ID" --apiIssuer "$KOLADA_ASC_ISSUER"
    xcrun altool --upload-app -f "$ipa" -t ios --apiKey "$KOLADA_ASC_KEY_ID" --apiIssuer "$KOLADA_ASC_ISSUER"
    echo "uploaded: it appears in App Store Connect under TestFlight once processed"
fi
