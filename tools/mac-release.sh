#!/usr/bin/env bash
# Builds Kolada for release on the Mac: the app (tools/mac-bundle.sh), signed
# with the hardened runtime, in a DMG at target/Kolada-<version>.dmg, and
# notarized and stapled when credentials are given. Without them it signs ad
# hoc, so everything up to the Developer ID can be built and checked here.
#
#   tools/mac-release.sh
#
# Set, to ship:
#   KOLADA_SIGN_ID="Developer ID Application: <Name> (<TEAMID>)"
#       the certificate to sign with (security find-identity -v -p codesigning)
#   KOLADA_NOTARY_PROFILE=<profile>
#       a notarytool keychain profile, made once with:
#       xcrun notarytool store-credentials <profile> --apple-id <id> --team-id <TEAMID>
#           --keychain ~/.kolada-signing/kolada.keychain-db
#   KOLADA_UNIVERSAL=1
#       Apple silicon and Intel in one app (rustup target x86_64-apple-darwin)
#   KOLADA_WITH_AI=1
#       writing help in the app (the kolada-ai helper and assets/models)
#
# The Developer ID and its key live in their own keychain,
# ~/.kolada-signing/kolada.keychain-db, unlocked here with the password kept
# beside it, so a build over SSH signs without the login keychain or a prompt.
set -euo pipefail
cd "$(dirname "$0")/.."

id=${KOLADA_SIGN_ID:--}
keychain=~/.kolada-signing/kolada.keychain-db
notary_keychain=()
if [ "$id" != - ] && [ -f "$keychain" ]; then
    security unlock-keychain -p "$(cat ~/.kolada-signing/keychain-pass)" "$keychain"
    notary_keychain=(--keychain "$keychain")
fi
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=target/Kolada.app

bash tools/mac-bundle.sh
# Writing help ships only when asked for (KOLADA_WITH_AI=1): a model left in
# assets/models would otherwise go out in the DMG, on by default, and past
# the site's 25 MiB per file.
if [ -z "${KOLADA_WITH_AI:-}" ]; then
    rm -rf "$app/Contents/MacOS/kolada-ai" "$app/Contents/Resources/models"
fi
if [ -n "${KOLADA_UNIVERSAL:-}" ]; then
    cargo build --release -p kolada-mac --target x86_64-apple-darwin
    lipo -create target/release/kolada-mac target/x86_64-apple-darwin/release/kolada-mac -output "$app/Contents/MacOS/Kolada"
    # The writing helper: both halves when the Intel one builds, else Apple
    # silicon only (on an Intel Mac writing help then stays off, silently).
    if [ -f "$app/Contents/MacOS/kolada-ai" ] && cargo build --release -p kolada-ai --target x86_64-apple-darwin; then
        lipo -create target/release/kolada-ai target/x86_64-apple-darwin/release/kolada-ai -output "$app/Contents/MacOS/kolada-ai"
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
[ -f "$app/Contents/MacOS/kolada-ai" ] && sign "$app/Contents/MacOS/kolada-ai"
sign "$app"
codesign --verify --deep --strict --verbose=1 "$app"
echo "architectures: $(lipo -archs "$app/Contents/MacOS/Kolada")"

notarize() {
    xcrun notarytool submit "$1" --keychain-profile "$KOLADA_NOTARY_PROFILE" "${notary_keychain[@]}" --wait
}
if [ -n "${KOLADA_NOTARY_PROFILE:-}" ]; then
    [ "$id" != - ] || { echo "notarizing needs KOLADA_SIGN_ID (a Developer ID)"; exit 1; }
    # The app carries its own ticket too, not only the DMG: a copy dragged
    # to Applications then opens on a Mac that is offline the first time.
    zip=$(mktemp -d)/Kolada.zip
    ditto -c -k --keepParent "$app" "$zip"
    notarize "$zip"
    rm -rf "$(dirname "$zip")"
    xcrun stapler staple "$app"
fi

# The DMG: the app beside a link to Applications.
dmg="target/Kolada-$version.dmg"
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname Kolada -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
[ "$id" != - ] && codesign --force --timestamp --sign "$id" "$dmg"

if [ -n "${KOLADA_NOTARY_PROFILE:-}" ]; then
    notarize "$dmg"
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
else
    echo "not notarized (set KOLADA_SIGN_ID and KOLADA_NOTARY_PROFILE to ship)"
fi
echo "built $dmg ($(du -h "$dmg" | cut -f1))"
