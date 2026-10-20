#!/usr/bin/env bash
# Package the desktop build as a universal macOS app and a DMG/zip.
#
#   scripts/package-macos.sh
#
# macOS only: `lipo`, `codesign`, `hdiutil` and `ditto` are the platform's own.
# The bundle is ad-hoc signed, not notarized — Gatekeeper still asks the user to
# confirm the first launch.
set -euo pipefail

cd "$(dirname "$0")/.."

version=${VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}
test -n "$version"
# CFBundleVersion is the numeric half; a pre-release suffix belongs only to
# CFBundleShortVersionString.
build_version=$(printf '%s' "$version" | sed 's/[^0-9.].*$//')
test -n "$build_version"

dist=${DIST_DIR:-dist}
app="$dist/Pandora.app"
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-11.0}

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$dist"

cargo build --release --locked --target aarch64-apple-darwin
cargo build --release --locked --target x86_64-apple-darwin

lipo -create \
  target/aarch64-apple-darwin/release/pandora \
  target/x86_64-apple-darwin/release/pandora \
  -output "$app/Contents/MacOS/pandora"
chmod 0755 "$app/Contents/MacOS/pandora"

install -m 0644 assets/app/macos/pandora.icns \
  "$app/Contents/Resources/pandora.icns"
printf 'APPL????' > "$app/Contents/PkgInfo"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Pandora</string>
  <key>CFBundleExecutable</key>
  <string>pandora</string>
  <key>CFBundleIconFile</key>
  <string>pandora.icns</string>
  <key>CFBundleIdentifier</key>
  <string>dev.pandora</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Pandora</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>$version</string>
  <key>CFBundleSignature</key>
  <string>????</string>
  <key>CFBundleVersion</key>
  <string>$build_version</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>
  <string>$MACOSX_DEPLOYMENT_TARGET</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>MIT License</string>
</dict>
</plist>
PLIST

xattr -cr "$app"
codesign --force --deep --sign - "$app"

dmg="$dist/Pandora-$version-macos-universal.dmg"
zip="$dist/Pandora-$version-macos-universal.zip"
rm -f "$dmg" "$zip"
hdiutil create -volname "Pandora" -srcfolder "$app" -ov -format UDZO "$dmg"
ditto -c -k --keepParent "$app" "$zip"

ls -lh "$dmg" "$zip"
shasum -a 256 "$dmg" "$zip"
