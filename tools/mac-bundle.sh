#!/usr/bin/env bash
# Builds Kolada.app from this tree (macOS), signed ad hoc so it opens on this
# Mac, at target/Kolada.app.
#
#   tools/mac-bundle.sh            build the app
#   tools/mac-bundle.sh --open     build it and open it
#
# Open it with a vault: open -n target/Kolada.app --args /path/to/vault
# (without one it opens ~/Documents/Kolada, or KOLADA_VAULT).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release -p kolada-mac
app=target/Kolada.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/kolada-mac "$app/Contents/MacOS/Kolada"
# The icon, from the user's drawing (tools/icongen writes it).
cp crates/kolada-mac/Kolada.icns "$app/Contents/Resources/Kolada.icns"
# Writing help: the model's process, when it has been built (cargo build
# --release -p kolada-ai, which needs the llama.cpp submodule), and the model,
# when it is in assets/models. Without them writing help stays off.
if [ -f target/release/kolada-ai ]; then
    cp target/release/kolada-ai "$app/Contents/MacOS/kolada-ai"
fi
for model in assets/models/*.gguf; do
    [ -f "$model" ] || continue
    mkdir -p "$app/Contents/Resources/models"
    cp "$model" "$app/Contents/Resources/models/"
done
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Kolada</string>
    <key>CFBundleDisplayName</key><string>Kolada</string>
    <key>CFBundleExecutable</key><string>Kolada</string>
    <key>CFBundleIdentifier</key><string>app.kolada.mac</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${version:-0.0.1}</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>CFBundleIconFile</key><string>Kolada</string>
    <key>NSHumanReadableCopyright</key><string>Kolada</string>
    <!-- What macOS says when Kolada first opens a vault in a protected folder. -->
    <key>NSDocumentsFolderUsageDescription</key><string>Kolada keeps your notes as Markdown files in the folder you choose.</string>
    <key>NSDesktopFolderUsageDescription</key><string>Kolada keeps your notes as Markdown files in the folder you choose.</string>
    <key>NSDownloadsFolderUsageDescription</key><string>Kolada opens and imports the files you give it.</string>
    <key>NSRemovableVolumesUsageDescription</key><string>Kolada keeps your notes as Markdown files in the folder you choose.</string>
    <key>NSNetworkVolumesUsageDescription</key><string>Kolada keeps your notes as Markdown files in the folder you choose.</string>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Markdown note</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key><array><string>md</string><string>markdown</string><string>txt</string></array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>Notes from another app</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key><array><string>nbk</string><string>enex</string><string>docx</string><string>pdf</string><string>html</string><string>htm</string><string>zip</string></array>
        </dict>
    </array>
</dict>
</plist>
PLIST
codesign --force --sign - "$app" 2>/dev/null
echo "built $app"
[ "${1:-}" = "--open" ] && open -n "$app"
exit 0
