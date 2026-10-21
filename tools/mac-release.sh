#!/usr/bin/env bash
# Builds Kolada for release on the Mac: the app (tools/mac-bundle.sh) with its
# writing helper (kolada-ai) and no model (one is chosen and downloaded at
# first run), signed with the hardened runtime, in a DMG at
# target/Kolada-<version>.dmg, notarized and stapled when credentials are
# given; and for the app's updater, the same app zipped at
# target/Kolada-<version>-mac.zip with its feed, target/mac.json (for
# kolada.app/download/). Without credentials it signs ad hoc, so everything
# up to the Developer ID can be built and checked here.
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
#   KOLADA_NO_AI=1
#       no writing helper in the app (writing help then stays off)
#   KOLADA_APP_ONLY=1
#       stop once target/Kolada.app is built and signed: no update zip, no DMG
#       (to check a build, its architectures and its size)
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

# The writing helper ships with the app, as on Windows (it needs cmake and
# the llama.cpp submodule); a model never does: a model left in
# assets/models would go out in the DMG and past the site's 25 MiB a file.
if [ -z "${KOLADA_NO_AI:-}" ]; then
    PATH="/opt/homebrew/bin:$PATH" cargo build --release -p kolada-ai
fi
bash tools/mac-bundle.sh
rm -rf "$app/Contents/Resources/models"
if [ -n "${KOLADA_NO_AI:-}" ]; then
    rm -f "$app/Contents/MacOS/kolada-ai"
fi
if [ -n "${KOLADA_UNIVERSAL:-}" ]; then
    cargo build --release -p kolada-mac --target x86_64-apple-darwin
    lipo -create target/release/kolada-mac target/x86_64-apple-darwin/release/kolada-mac -output "$app/Contents/MacOS/Kolada"
    # The writing helper: both halves when the Intel one builds, else Apple
    # silicon only (on an Intel Mac writing help then stays off, silently).
    if [ -f "$app/Contents/MacOS/kolada-ai" ] && PATH="/opt/homebrew/bin:$PATH" cargo build --release -p kolada-ai --target x86_64-apple-darwin; then
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
if [ -n "${KOLADA_APP_ONLY:-}" ]; then
    echo "built $app ($(du -sh "$app" | cut -f1)); no update zip or DMG (KOLADA_APP_ONLY)"
    exit 0
fi

# The site takes files of 25 MiB at most.
within_site() {
    local bytes
    bytes=$(stat -f %z "$1")
    [ "$bytes" -le $((25 * 1024 * 1024)) ] || { echo "$1 is $bytes bytes: over the site's 25 MiB a file"; exit 1; }
}

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

# The update: the app zipped as the updater unpacks it (ditto), and its
# feed, which kolada.app serves beside the zip at /download/.
zip="target/Kolada-$version-mac.zip"
rm -f "$zip"
ditto -c -k --keepParent "$app" "$zip"
sha=$(shasum -a 256 "$zip" | cut -d' ' -f1)
size=$(stat -f %z "$zip")
within_site "$zip"
printf '{ "version": "%s", "url": "https://kolada.app/download/%s", "sha256": "%s", "size": %s }\n' "$version" "$(basename "$zip")" "$sha" "$size" > target/mac.json
echo "update $zip ($size bytes) and target/mac.json"

# The DMG: the app beside a link to Applications.
dmg="target/Kolada-$version.dmg"
stage=$(mktemp -d)
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname Kolada -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
[ "$id" != - ] && codesign --force --timestamp --sign "$id" "$dmg"
within_site "$dmg"

if [ -n "${KOLADA_NOTARY_PROFILE:-}" ]; then
    notarize "$dmg"
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
else
    echo "not notarized (set KOLADA_SIGN_ID and KOLADA_NOTARY_PROFILE to ship)"
fi
echo "built $dmg ($(du -h "$dmg" | cut -f1))"
