#!/usr/bin/env bash
# Package the desktop build as a single-file AppImage.
#
#   scripts/appimage.sh
#
# `target/release/pandora` already links almost everything statically; the one
# library it asks for at runtime is ALSA, so it goes inside the bundle. The app
# data directory is untouched: a box stays wherever the user put it.
#
# appimagetool is fetched into target/appimage/tools on first use. Set
# APPIMAGETOOL to use one already on the machine.
set -euo pipefail

cd "$(dirname "$0")/.."
# Permissions inside the bundle must not depend on the caller's umask.
umask 022

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
test -n "$version"

# CI builds x86_64 (see .github/workflows/release.yml). aarch64 is mapped here
# because appimagetool publishes that runtime too, but it is not exercised by
# the workflow yet.
case "$(uname -m)" in
  x86_64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *)
    echo "no AppImage runtime for $(uname -m)" >&2
    exit 1
    ;;
esac

outdir=target/appimage
appdir="$outdir/Pandora.AppDir"
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" \
  "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons"

cargo build --release --locked
install -m 0755 target/release/pandora "$appdir/usr/bin/pandora"

# The binary is dynamically linked against libasound and nothing else besides
# glibc. Bundling it means the AppImage also opens on a host without the ALSA
# development/runtime package; its plugins are still loaded from the host.
alsa=$(ldd target/release/pandora | awk '/libasound\.so/ { print $3; exit }')
if [ -n "$alsa" ] && [ -f "$alsa" ]; then
  cp -L "$alsa" "$appdir/usr/lib/libasound.so.2"
fi

# The same desktop entry and icon the manual install uses, with a copy at the
# root because that is where appimagetool and the desktop integration look.
install -m 0644 assets/app/linux/pandora.desktop "$appdir/pandora.desktop"
install -m 0644 assets/app/linux/pandora.desktop \
  "$appdir/usr/share/applications/pandora.desktop"
# The desktop reads the size it needs from hicolor; a single 256 image is
# what makes a taskbar icon look soft. The root icon stays 512 for the AppImage
# integration that only looks at `.DirIcon`.
cp -a assets/app/linux/hicolor "$appdir/usr/share/icons/"
install -m 0644 assets/app/linux/hicolor/512x512/apps/pandora.png \
  "$appdir/pandora.png"
ln -s pandora.png "$appdir/.DirIcon"

cat > "$appdir/AppRun" <<'SH'
#!/bin/sh
# The mounted AppImage's root, not the caller's working directory.
here=$(dirname "$(readlink -f "$0")")
export LD_LIBRARY_PATH="$here/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/usr/bin/pandora" "$@"
SH
chmod 0755 "$appdir/AppRun"

tool=${APPIMAGETOOL:-"$outdir/tools/appimagetool-$arch.AppImage"}
if [ ! -x "$tool" ]; then
  mkdir -p "$(dirname "$tool")"
  url="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$arch.AppImage"
  echo "fetching appimagetool: $url"
  curl -fL --retry 3 -o "$tool" "$url"
  chmod +x "$tool"
fi

image="$outdir/Pandora-$version-$arch.AppImage"
rm -f "$image"

# A caller that pins the epoch (CI uses the commit date) gets a bundle whose
# file timestamps do not change from one build of the same tree to the next.
if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
  find "$appdir" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
fi

# The tool normally mounts itself with FUSE; extracting it is the fallback that
# also works in a container where /dev/fuse is missing.
env ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 \
  "$tool" --no-appstream "$appdir" "$image"

ls -lh "$image"
sha256sum "$image"
