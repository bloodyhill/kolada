#!/usr/bin/env bash
# Builds Kolada for release on Linux, into dist/linux/:
#
#   kolada_<version>_<arch>.deb        Debian, Ubuntu and their kin
#   kolada-<version>-linux-<arch>.tar.gz   any other: unpack, run ./install.sh
#   SHA256SUMS
#
# Both hold the app (`kolada`, stripped), its icons at every size the
# desktop's icon theme asks (made from assets/logo.svg by tools/icongen),
# and a .desktop file that puts it in the desktop's app list and under
# "Open with" for Markdown notes. Run on Linux (or WSL) from anywhere:
#
#   tools/linux-release.sh
#
# Writing help is not in it yet (the Linux app does not offer it).
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

# What both packages hold, laid out as under /usr.
usr=$stage/usr
mkdir -p "$usr/lib/kolada" "$usr/bin" "$usr/share/applications" "$usr/share/icons" "$usr/share/doc/kolada" "$usr/share/metainfo"
install -m 755 "$exe" "$usr/lib/kolada/kolada"
strip --strip-unneeded "$usr/lib/kolada/kolada"
# The command: the app's own binary, where the writing helper will sit beside it.
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
glibc=$(readelf -V "$usr/lib/kolada/kolada" | sed -n 's/.*Name: GLIBC_\([0-9.]*\)  Flags: none.*/\1/p' | sort -V | tail -1)
glibc=${glibc:-2.31}

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
Depends: libc6 (>= $glibc), libxkbcommon0, libxkbcommon-x11-0 | libwayland-client0, libx11-6 | libwayland-client0, libx11-xcb1 | libwayland-client0, libxcursor1 | libwayland-cursor0, libxi6 | libwayland-client0, fonts-noto-core | fonts-liberation2 | fonts-dejavu-core
Recommends: xdg-utils, xdg-desktop-portal, hunspell-en-us | hunspell-dictionary, fonts-noto-color-emoji
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
cp "$here/kolada" "$lib/kolada"
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

rm -rf "$stage"
(cd "$out" && sha256sum "$(basename "$debfile")" "$(basename "$tarfile")" > SHA256SUMS)
echo "== dist/linux"
ls -l "$out" | tail -n +2
cat "$out/SHA256SUMS"
