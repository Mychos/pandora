#!/usr/bin/env bash
# Package the desktop build as a universal macOS app and a DMG/zip.
#
#   scripts/package-macos.sh
#
# macOS only: `lipo`, `codesign`, `hdiutil`, `ditto` and `xcrun` are the platform's own.
#
# Signing is optional, and its absence is not silent. With
# `PANDORA_APPLE_CERT_P12` (base64 of a Developer ID Application .p12),
# `PANDORA_APPLE_CERT_PASSWORD`, `PANDORA_APPLE_ID`, `PANDORA_APPLE_TEAM_ID` and
# `PANDORA_APPLE_APP_PASSWORD` the bundle is signed, notarized and stapled — the only
# way a download opens on the first double-click on a Mac that never saw the app.
# Without them it is ad-hoc signed: fine on the machine that built it, and a
# right-click → Open everywhere else, because Gatekeeper quarantines what a browser
# downloaded. `release.yml` passes the five secrets when they exist.
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

# A universal build is two builds lipo'd together, and a missing slice only shows up
# on the Mac it cannot start on.
for arch in arm64 x86_64; do
  case " $(lipo -archs "$app/Contents/MacOS/pandora") " in
    *" $arch "*) ;;
    *)
      echo "error: the binary has no $arch slice" >&2
      exit 1
      ;;
  esac
done

# A bundle that asks for something in a Homebrew prefix opens on the machine that built
# it and dies on the user's. libusb is the one that bit (the YubiKey, now vendored), so
# the check is on the whole binary rather than on one dylib.
if otool -L "$app/Contents/MacOS/pandora" | grep -Eq '/(opt/homebrew|usr/local)/'; then
  echo "error: the binary loads a library from a local prefix" >&2
  otool -L "$app/Contents/MacOS/pandora" >&2
  exit 1
fi

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

# The Developer ID lives in a keychain of its own, made from the secret and deleted
# again, so nothing is imported into the runner's login keychain.
keychain=""
work=""
cleanup() {
  [ -z "$work" ] || rm -rf "$work"
  [ -z "$keychain" ] || security delete-keychain "$keychain" >/dev/null 2>&1 || true
}
trap cleanup EXIT

signing=""
if [ -n "${PANDORA_APPLE_CERT_P12:-}" ]; then
  work=$(mktemp -d)
  keychain="$work/pandora-signing.keychain-db"
  password=$(uuidgen)
  security create-keychain -p "$password" "$keychain"
  security set-keychain-settings -lut 3600 "$keychain"
  security unlock-keychain -p "$password" "$keychain"
  printf '%s' "$PANDORA_APPLE_CERT_P12" \
    | openssl base64 -d -A -out "$work/cert.p12"
  security import "$work/cert.p12" -k "$keychain" \
    -P "${PANDORA_APPLE_CERT_PASSWORD:?}" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple:,codesign: \
    -k "$password" "$keychain" >/dev/null
  signing=$(security find-identity -v -p codesigning "$keychain" \
    | sed -n 's/.*"\(Developer ID Application: .*\)"/\1/p' | head -1)
  if [ -z "$signing" ]; then
    echo "error: PANDORA_APPLE_CERT_P12 holds no Developer ID Application identity" >&2
    exit 1
  fi
fi

if [ -n "$signing" ]; then
  codesign --force --timestamp --options runtime \
    --keychain "$keychain" --sign "$signing" "$app"
else
  codesign --force --sign - "$app"
fi

notarizable=no
if [ -n "$signing" ]; then
  if [ -n "${PANDORA_APPLE_ID:-}" ]; then
    notarizable=yes
  else
    echo "warning: signed with the Developer ID but not notarized: a download will still be refused" >&2
  fi
fi

notarize() {
  xcrun notarytool submit "$1" \
    --apple-id "${PANDORA_APPLE_ID:?}" \
    --team-id "${PANDORA_APPLE_TEAM_ID:?}" \
    --password "${PANDORA_APPLE_APP_PASSWORD:?}" \
    --wait
}

# The app is notarized before the image is built, so both carry a stapled ticket: the
# one in the .app covers a copy dragged out of the image, the one in the .dmg covers the
# image itself.
if [ "$notarizable" = yes ]; then
  [ -n "$work" ] || work=$(mktemp -d)
  ditto -c -k --keepParent "$app" "$work/app.zip"
  notarize "$work/app.zip"
  xcrun stapler staple "$app"
fi

dmg="$dist/Pandora-$version-macos-universal.dmg"
zip="$dist/Pandora-$version-macos-universal.zip"
rm -f "$dmg" "$zip"

# A volume with the app and a way to install it: dragging onto a folder in Finder is
# how a Mac user installs an app, and an image with only the app in it reads as one
# that has to be copied by hand.
stage="$dist/.dmg"
rm -rf "$stage"
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Pandora" -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"

if [ -n "$signing" ]; then
  codesign --force --timestamp --keychain "$keychain" --sign "$signing" "$dmg"
  if [ "$notarizable" = yes ]; then
    notarize "$dmg"
    xcrun stapler staple "$dmg"
  fi
fi

ditto -c -k --keepParent "$app" "$zip"

# What the user's launch actually asks: a valid signature, both slices, and — when the
# secrets were there — the two assessments Gatekeeper runs.
codesign --verify --strict --verbose=2 "$app"
hdiutil verify "$dmg" >/dev/null
if [ "$notarizable" = yes ]; then
  spctl --assess --type execute -vv "$app"
  spctl --assess --type open --context context:primary-signature -vv "$dmg"
fi

ls -lh "$dmg" "$zip"
shasum -a 256 "$dmg" "$zip"
