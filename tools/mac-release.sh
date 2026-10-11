#!/usr/bin/env bash
# Builds Notsy for release on the Mac: the app (tools/mac-bundle.sh), signed
# with the hardened runtime, in a DMG at target/Notsy-<version>.dmg, and
# notarized and stapled when credentials are given. Without them it signs ad
# hoc, so everything up to the Developer ID can be built and checked here.
#
#   tools/mac-release.sh
#
# Set, to ship:
#   NOTSY_SIGN_ID="Developer ID Application: <Name> (<TEAMID>)"
#       the certificate to sign with (security find-identity -v -p codesigning)
#   NOTSY_NOTARY_PROFILE=<profile>
#       a notarytool keychain profile, made once with:
#       xcrun notarytool store-credentials <profile> --apple-id <id> --team-id <TEAMID>
#   NOTSY_UNIVERSAL=1
#       Apple silicon and Intel in one app (rustup target x86_64-apple-darwin)
#   NOTSY_WITH_AI=1
#       writing help in the app (the notsy-ai helper and assets/models)
set -euo pipefail
cd "$(dirname "$0")/.."

id=${NOTSY_SIGN_ID:--}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=target/Notsy.app

bash tools/mac-bundle.sh
# Writing help ships only when asked for (NOTSY_WITH_AI=1): a model left in
# assets/models would otherwise go out in the DMG, on by default, and past
# the site's 25 MiB per file.
if [ -z "${NOTSY_WITH_AI:-}" ]; then
    rm -rf "$app/Contents/MacOS/notsy-ai" "$app/Contents/Resources/models"
fi
if [ -n "${NOTSY_UNIVERSAL:-}" ]; then
    cargo build --release -p notsy-mac --target x86_64-apple-darwin
    lipo -create target/release/notsy-mac target/x86_64-apple-darwin/release/notsy-mac -output "$app/Contents/MacOS/Notsy"
    # The writing helper: both halves when the Intel one builds, else Apple
    # silicon only (on an Intel Mac writing help then stays off, silently).
    if [ -f "$app/Contents/MacOS/notsy-ai" ] && cargo build --release -p notsy-ai --target x86_64-apple-darwin; then
        lipo -create target/release/notsy-ai target/x86_64-apple-darwin/release/notsy-ai -output "$app/Contents/MacOS/notsy-ai"
    fi
fi

# Inside out: the helper, then the app around it. The hardened runtime
# needs no exceptions: no JIT, no unsigned code, no Apple Events.
sign() {
    if [ "$id" = - ]; then
        codesign --force --options runtime --sign - "$1"
    else
        codesign --force --options runtime --timestamp --sign "$id" "$1"
    fi
}
[ -f "$app/Contents/MacOS/notsy-ai" ] && sign "$app/Contents/MacOS/notsy-ai"
sign "$app"
codesign --verify --deep --strict --verbose=1 "$app"
echo "architectures: $(lipo -archs "$app/Contents/MacOS/Notsy")"

# The DMG: the app beside a link to Applications.
dmg="target/Notsy-$version.dmg"
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname Notsy -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
[ "$id" != - ] && codesign --force --timestamp --sign "$id" "$dmg"

if [ -n "${NOTSY_NOTARY_PROFILE:-}" ]; then
    [ "$id" != - ] || { echo "notarizing needs NOTSY_SIGN_ID (a Developer ID)"; exit 1; }
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTSY_NOTARY_PROFILE" --wait
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
else
    echo "not notarized (set NOTSY_SIGN_ID and NOTSY_NOTARY_PROFILE to ship)"
fi
echo "built $dmg ($(du -h "$dmg" | cut -f1))"
