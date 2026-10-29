#!/usr/bin/env bash
# Builds Kolada for release on Linux, into dist/linux/:
#
#   kolada_<version>_<arch>.deb        Debian, Ubuntu and their kin
#   kolada-<version>-linux-<arch>.tar.gz   any other: unpack, run ./install.sh
#   Kolada-<version>-<arch>.AppImage   one file for any: chmod +x, run it
#   linux.json                         the feed the .tar.gz's install updates from
#   SHA256SUMS
#
# Both hold the app (`kolada`, stripped), its icons at every size the
# desktop's icon theme asks (made from assets/logo.svg by tools/icongen),
# and a .desktop file that puts it in the desktop's app list and under
# "Open with" for Markdown notes. Run on Linux (or WSL) from anywhere:
#
#   tools/linux-release.sh
#
# Each holds the writing helper (`kolada-ai`, beside `kolada`; it needs
# cmake, a C++ compiler and the llama.cpp submodule) and no model: one is
# chosen and downloaded at first run. KOLADA_NO_AI=1 leaves the helper out
# (writing help then stays off).
# KOLADA_MAINTAINER="Name <address>" sets the .deb's Maintainer field.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$PWD
[ "$(uname)" = Linux ] || { echo "the Linux release builds on Linux"; exit 2; }
for tool in cargo dpkg-deb strip; do
    command -v "$tool" >/dev/null || { echo "needs $tool (apt install dpkg-dev binutils)"; exit 2; }
done

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
case "$(uname -m)" in
    x86_64) arch=x86_64 debarch=amd64 ;;
    aarch64) arch=aarch64 debarch=arm64 ;;
    *) arch=$(uname -m) debarch=$(uname -m) ;;
esac
target=${CARGO_TARGET_DIR:-$root/target}
out=$root/dist/linux
stage=$out/stage
rm -rf "$stage"
mkdir -p "$out" "$stage"

echo "== Building Kolada $version for $arch"
cargo build -q --release -p kolada-linux -p icongen
exe=$target/release/kolada
if [ -z "${KOLADA_NO_AI:-}" ]; then
    command -v cmake >/dev/null || { echo "the writing helper needs cmake (apt install cmake g++), or KOLADA_NO_AI=1"; exit 2; }
    [ -f third_party/llama.cpp/CMakeLists.txt ] || { echo "the writing helper needs the llama.cpp submodule (git submodule update --init), or KOLADA_NO_AI=1"; exit 2; }
    cargo build -q --release -p kolada-ai
fi

# What the packages hold, laid out as under /usr.
usr=$stage/usr
mkdir -p "$usr/lib/kolada" "$usr/bin" "$usr/share/applications" "$usr/share/icons" "$usr/share/doc/kolada" "$usr/share/metainfo"
install -m 755 "$exe" "$usr/lib/kolada/kolada"
strip --strip-unneeded "$usr/lib/kolada/kolada"
if [ -z "${KOLADA_NO_AI:-}" ]; then
    install -m 755 "$target/release/kolada-ai" "$usr/lib/kolada/kolada-ai"
    strip --strip-unneeded "$usr/lib/kolada/kolada-ai"
fi
# The command: the app's own binary, the writing helper beside it.
ln -s ../lib/kolada/kolada "$usr/bin/kolada"
"$target/release/icongen" --linux "$usr/share/icons" >/dev/null
cat > "$usr/share/applications/kolada.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Kolada
GenericName=Notes
Comment=Notes in plain Markdown, fast
Exec=kolada %f
Icon=kolada
Terminal=false
Categories=Office;TextEditor;Utility;
MimeType=text/markdown;text/x-markdown;
Keywords=notes;markdown;writing;vault;
StartupNotify=true
StartupWMClass=kolada
Actions=new-note;search;beam;

[Desktop Action new-note]
Name=New Note
Exec=kolada --new

[Desktop Action search]
Name=Search Notes
Exec=kolada --search

[Desktop Action beam]
Name=Beam
Exec=kolada --beam
EOF
cat > "$usr/share/metainfo/kolada.metainfo.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>kolada</id>
  <name>Kolada</name>
  <summary>Notes in plain Markdown, fast</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>LicenseRef-proprietary</project_license>
  <description>
    <p>Kolada opens and types faster than any other notes app. Your notes are a plain folder of Markdown files.</p>
  </description>
  <launchable type="desktop-id">kolada.desktop</launchable>
  <url type="homepage">https://kolada.app</url>
  <provides><binary>kolada</binary></provides>
  <releases><release version="$version"/></releases>
</component>
EOF
cat > "$usr/share/doc/kolada/copyright" <<EOF
Kolada $version
Copyright Kolada. All rights reserved.

Third-party software in Kolada, and its licenses:
EOF
cat THIRD-PARTY.md >> "$usr/share/doc/kolada/copyright"
maintainer=${KOLADA_MAINTAINER:-Kolada (https://kolada.app)}
printf 'kolada (%s) stable; urgency=medium\n\n  * Kolada %s for Linux.\n\n -- %s  %s\n' "$version" "$version" "$maintainer" "$(date -R)" | gzip -9n > "$usr/share/doc/kolada/changelog.gz"
mkdir -p "$usr/share/man/man1"
gzip -9n > "$usr/share/man/man1/kolada.1.gz" <<EOF
.TH KOLADA 1 "" "Kolada $version" "User Commands"
.SH NAME
kolada \- notes in plain Markdown, fast
.SH SYNOPSIS
.B kolada
[\fIFOLDER\fR | \fINOTE\fR.md]
.SH DESCRIPTION
Kolada is a notes app: a window on a folder of Markdown notes (a vault).
With a folder, that folder opens as the vault; with a note, its vault opens
on it. With neither, the vault last open opens, else ~/Documents/Kolada.
Launched while it runs, Kolada comes to the front and opens what it was given.
.SH ENVIRONMENT
.TP
.B KOLADA_VAULT
The vault to open when none is given.
.SH FILES
.TP
.I \$XDG_DATA_HOME/Kolada
What Kolada keeps between launches (where it was, settings, the notes' past,
words taught to the spell checker): ~/.local/share/Kolada when XDG_DATA_HOME
is not set. The notes themselves are only ever in the vault.
EOF

# The oldest C library it runs on: the newest symbol version it needs (weak
# ones, which it looks for and does without, do not count).
glibc=$(readelf -V "$usr"/lib/kolada/kolada* | sed -n 's/.*Name: GLIBC_\([0-9.]*\)  Flags: none.*/\1/p' | sort -V | tail -1)
glibc=${glibc:-2.31}
# The writing helper is C++ (llama.cpp): the C++ library beside it.
aidep=""
[ -f "$usr/lib/kolada/kolada-ai" ] && aidep=" libstdc++6,"

echo "== .deb"
deb=$stage/deb
mkdir -p "$deb/DEBIAN"
cp -a "$usr" "$deb/usr"
size=$(du -sk "$deb/usr" | cut -f1)
# What it opens at run time (winit and softbuffer load the window system's
# libraries themselves): X11 or Wayland, and the keyboard's layouts. The
# fonts it reads by name. The rest makes it better where it is there.
cat > "$deb/DEBIAN/control" <<EOF
Package: kolada
Version: $version
Architecture: $debarch
Maintainer: ${KOLADA_MAINTAINER:-Kolada (https://kolada.app)}
Installed-Size: $size
Depends: libc6 (>= $glibc),${aidep} libxkbcommon0, libxkbcommon-x11-0 | libwayland-client0, libx11-6 | libwayland-client0, libx11-xcb1 | libwayland-client0, libxcursor1 | libwayland-cursor0, libxi6 | libwayland-client0, fonts-noto-core | fonts-liberation2 | fonts-dejavu-core
Recommends: curl, xdg-utils, xdg-desktop-portal, hunspell-en-us | hunspell-dictionary, fonts-noto-color-emoji
Suggests: zenity | kdialog, chromium | libreoffice-writer
Section: editors
Priority: optional
Homepage: https://kolada.app
Description: Notes in plain Markdown, fast
 Kolada opens and types faster than any other notes app. Your notes are a
 plain folder of Markdown files, which other apps (Obsidian among them)
 open as they are.
EOF
debfile=$out/kolada_${version}_${debarch}.deb
rm -f "$debfile"
dpkg-deb --build --root-owner-group -Zxz "$deb" "$debfile" >/dev/null

echo "== .tar.gz"
tarname=kolada-$version-linux-$arch
tardir=$stage/$tarname
mkdir -p "$tardir"
cp -a "$usr/lib/kolada/kolada" "$tardir/kolada"
[ -f "$usr/lib/kolada/kolada-ai" ] && cp -a "$usr/lib/kolada/kolada-ai" "$tardir/kolada-ai"
cp -a "$usr/share/icons" "$tardir/icons"
cp "$usr/share/applications/kolada.desktop" "$tardir/kolada.desktop"
cp "$usr/share/doc/kolada/copyright" "$tardir/COPYRIGHT"
cat > "$tardir/install.sh" <<'EOF'
#!/bin/sh
# Installs Kolada for you alone (no root): the app in ~/.local/lib/kolada,
# the command in ~/.local/bin, its icons and its place in the desktop's app
# list and "Open with". ./install.sh --remove takes it all away again.
set -e
here=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
lib=$HOME/.local/lib/kolada
bin=$HOME/.local/bin
if [ "${1:-}" = --remove ]; then
    rm -rf "$lib" "$bin/kolada" "$data/applications/kolada.desktop"
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
    find "$data/icons/hicolor" -name 'kolada.*' -delete 2>/dev/null || true
    echo "Kolada removed (your notes are where they were)."
    exit 0
fi
mkdir -p "$lib" "$bin" "$data/applications" "$data/icons"
# Beside, then renamed over: a Kolada running keeps its own file.
for f in kolada kolada-ai; do
    [ -f "$here/$f" ] || continue
    cp "$here/$f" "$lib/.$f.part" && mv -f "$lib/.$f.part" "$lib/$f"
done
ln -sf "$lib/kolada" "$bin/kolada"
cp -R "$here/icons/hicolor" "$data/icons/"
sed "s|^Exec=kolada|Exec=\"$lib/kolada\"|" "$here/kolada.desktop" > "$data/applications/kolada.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
echo "Kolada installed: in your apps, or run kolada (from $bin)."
EOF
chmod 755 "$tardir/install.sh"
tarfile=$out/$tarname.tar.gz
tar -C "$stage" --owner=0 --group=0 -czf "$tarfile" "$tarname"

echo "== AppImage"
# One file that runs on any distribution: the AppImage runtime (it mounts
# the image and starts AppRun inside) followed by a squashfs of the app.
# The runtime is AppImage's own (github.com/AppImage/type2-runtime), kept
# at KOLADA_APPIMAGE_RUNTIME or ~/.cache/kolada/appimage-runtime-<arch>;
# without it there is no AppImage, and the rest is built as before.
runtime=${KOLADA_APPIMAGE_RUNTIME:-$HOME/.cache/kolada/appimage-runtime-$arch}
appimage=""
if [ -f "$runtime" ] && command -v mksquashfs >/dev/null; then
    appdir=$stage/Kolada.AppDir
    mkdir -p "$appdir/usr"
    cp -a "$usr/lib" "$usr/share" "$appdir/usr/"
    # AppRun runs the app from a copy on the disk, made at its first launch
    # (and the copies of other builds removed then): run from the mounted
    # image, every page of the program comes through FUSE, which took the
    # first frame from 32 to 63 ms; from the copy it is 38. Where the copy
    # cannot be made, it runs from the image.
    key=$version-$(sha256sum "$usr/lib/kolada/kolada" | cut -c1-12)
    cat > "$appdir/AppRun" <<EOF
#!/bin/sh
d=\${XDG_CACHE_HOME:-\$HOME/.cache}/kolada/appimage
c=\$d/$key
copy() {
    rm -rf "\$d" && mkdir -p "\$c" || return 1
    # The program last: once it is there, the copy is whole.
    for f in kolada-ai kolada; do
        [ -f "\$APPDIR/usr/lib/kolada/\$f" ] || continue
        cp "\$APPDIR/usr/lib/kolada/\$f" "\$c/.\$f.\$\$" && mv -f "\$c/.\$f.\$\$" "\$c/\$f" || return 1
    done
}
[ -x "\$c/kolada" ] || copy || exec "\$APPDIR/usr/lib/kolada/kolada" "\$@"
exec "\$c/kolada" "\$@"
EOF
    chmod 755 "$appdir/AppRun"
    cp "$usr/share/applications/kolada.desktop" "$appdir/kolada.desktop"
    cp "$usr/share/icons/hicolor/256x256/apps/kolada.png" "$appdir/kolada.png"
    ln -s kolada.png "$appdir/.DirIcon"
    appimage=$out/Kolada-$version-$arch.AppImage
    rm -f "$appimage"
    mksquashfs "$appdir" "$stage/image.squashfs" -root-owned -noappend -comp zstd -quiet >/dev/null
    cat "$runtime" "$stage/image.squashfs" > "$appimage"
    chmod 755 "$appimage"
    echo "runtime $(sha256sum "$runtime" | cut -d' ' -f1)"
else
    echo "no AppImage: needs the runtime at $runtime and mksquashfs (apt install squashfs-tools)"
fi

echo "== linux.json"
# The updater's feed, for the .tar.gz's install (a package manager updates
# the .deb): the website serves it and the tarball side by side at
# /download/. KOLADA_DOWNLOAD_BASE names another place (a test's server).
base=${KOLADA_DOWNLOAD_BASE:-https://kolada.app/download}
feed=linux.json
[ "$arch" = x86_64 ] || feed=linux-$arch.json
printf '{\n  "version": "%s",\n  "url": "%s/%s",\n  "sha256": "%s",\n  "size": %s\n}\n' \
    "$version" "$base" "$(basename "$tarfile")" "$(sha256sum "$tarfile" | cut -d' ' -f1)" "$(stat -c %s "$tarfile")" > "$out/$feed"

rm -rf "$stage"
(cd "$out" && sha256sum "$(basename "$debfile")" "$(basename "$tarfile")" ${appimage:+"$(basename "$appimage")"} > SHA256SUMS)
echo "== dist/linux"
ls -l "$out" | tail -n +2
cat "$out/SHA256SUMS"
