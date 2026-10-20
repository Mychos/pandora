#!/usr/bin/env bash
# Regenerate every app-mark asset from assets/isotipo.svg:
#   assets/logo.png, assets/icon.png   — UI mark and the eframe window icon
#   assets/app/**                      — the OS packaging icons: Windows .ico,
#                                        macOS .icns, Linux hicolor, Android
#                                        mipmaps and the iOS appiconset.
# The source is unpainted monochrome: paths default to black, so we resolve the
# art to white for the UI to tint and bake the accent tile for everything the
# OS reads (a window manager cannot tint).
set -euo pipefail

cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# KEEP IN SYNC — theme::ACCENT in src/ui/theme.rs.
accent='#00a650'

# Paint every path white: the root fill covers unpainted paths, the explicit
# replacements cover sources that spell the colour out.
sed -z \
    -e 's|<svg|<svg fill="#ffffff"|' \
    -e 's|stroke="currentColor"|stroke="#ffffff"|g' \
    -e 's|stroke:currentColor|stroke:#ffffff|g' \
    -e 's|fill="currentColor"|fill="#ffffff"|g' \
    -e 's|fill:currentColor|fill:#ffffff|g' \
  assets/isotipo.svg > "$tmp/white.svg"

viewbox=$(grep -o 'viewBox="[^"]*"' assets/isotipo.svg | head -n1 | cut -d'"' -f2)

# A margin so the ink is not flush against the texture edge — full-bleed art
# reads as cropped once the UI shrinks it.
padded=$(echo "$viewbox" | awk '{m=0.06*($3>$4?$3:$4); printf "%.3f %.3f %.3f %.3f", $1-m, $2-m, $3+2*m, $4+2*m}')
sed "s|viewBox=\"$viewbox\"|viewBox=\"$padded\"|" "$tmp/white.svg" > "$tmp/mark.svg"

# The mark, sized near how the UI draws it: the wgpu backend has no mipmaps,
# so a large source turns to aliasing when shrunk into the window bar.
rsvg-convert -h 128 -b none "$tmp/mark.svg" -o assets/logo.png

# The app icon: the white mark inset on an accent tile, in a 48-unit canvas. The
# tile takes an iOS-style rounded corner (22% of the canvas, the ratio iOS and
# macOS round their icons with); the radius is a knob because iOS gets a
# full-bleed square instead and masks it itself. Every OS variant below is a
# raster of one of these two SVGs, so tile and mark cannot drift apart.
#
# `NUDGE` is optical centring, not slack in the maths: the mark's ink bbox is
# centred on its own, but its centre of *mass* sits 11% of the side above the
# tile's middle (a dense winged head over a thin box), so the mark reads as
# floating. Going all the way with that deviation overshoots — the bbox ends up
# visibly low — so this is roughly half of it, which is where the eye sits. The
# tighter bottom margin costs nothing because the art tapers to a point there.
NUDGE=${NUDGE:-1.4}
tile() { # $1 = destination svg, $2 = mark box size, $3 = corner radius, $4 = nudge
  local inset top
  inset=$(awk -v b="$2" 'BEGIN{printf "%.3f", (48-b)/2}')
  top=$(awk -v i="$inset" -v n="$4" 'BEGIN{printf "%.3f", i+n}')
  {
    printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">'
    printf '<rect width="48" height="48" rx="%s" fill="%s"/>' "$3" "$accent"
    printf '<svg x="%s" y="%s" width="%s" height="%s" viewBox="%s" fill="#ffffff">' \
      "$inset" "$top" "$2" "$2" "$viewbox"
    sed -z -e 's|<svg[^>]*>||' -e 's|</svg>||' "$tmp/white.svg"
    printf '</svg></svg>\n'
  } > "$1"
}

tile "$tmp/tile.svg" 40 11 "$NUDGE"
tile "$tmp/square.svg" 40 0 "$NUDGE"

# Android paints one shape of its own over whatever it is handed, and that shape
# is a circle. At the tile's size the wing tips reach the canvas corners, land
# outside that circle and get sliced flat, which is why Android gets the mark at
# its own size: at 34/48 the tips sit a fifth of the radius inside the mask, and
# they are the ink that reaches furthest out (the check at the end holds it).
android=34
# The adaptive layer is the same 48-unit canvas at 1.5x — Android shows the
# central 72 of its 108 — so the mark's box and its optical nudge scale with it.
# Without the nudge the mark reads as sitting high, the same way it would on any
# other tile.
read -r fbox fx fy <<<"$(awk -v b="$android" -v n="$NUDGE" \
  'BEGIN{ s = 1.5 * b; printf "%.1f %.1f %.1f", s, (108 - s) / 2, (108 - s) / 2 + 1.5 * n }')"
tile "$tmp/android.svg" "$android" 11 "$NUDGE"

# The window icon keeps the size src/ui/theme.rs and the UI expect.
rsvg-convert -w 256 -h 256 "$tmp/tile.svg" -o assets/icon.png

# Android masks the launcher icon itself, so its tile is a circle. Clipping the
# already-built tile reuses the paint instead of repeating it.
{ printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">'
  printf '<clipPath id="c"><circle cx="24" cy="24" r="24"/></clipPath><g clip-path="url(#c)">'
  sed -z -e 's|<svg[^>]*>||' -e 's|</svg>||' "$tmp/android.svg"
  printf '</g></svg>\n'; } > "$tmp/round.svg"

# The adaptive foreground is the mark on transparent canvas: Android masks it and
# the solid accent background below shows through.
{ printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 108 108">'
  printf '<svg x="%s" y="%s" width="%s" height="%s" viewBox="%s" fill="#ffffff">' \
    "$fx" "$fy" "$fbox" "$fbox" "$viewbox"
  sed -z -e 's|<svg[^>]*>||' -e 's|</svg>||' "$tmp/white.svg"
  printf '</svg></svg>\n'; } > "$tmp/foreground.svg"

# Every raster below is the tile SVG at some size; rendering from the vector at
# each size keeps the 16px frames crisp. Android needs two more: a circular tile
# and the safe-zone foreground.
sizes='16 20 24 29 32 40 48 58 60 64 72 76 80 87 96 120 128 144 152 167 180 192 256 512 1024'
for s in $sizes; do
  rsvg-convert -w "$s" -h "$s" "$tmp/tile.svg" -o "$tmp/tile-$s.png"
done
# iOS applies its own corner mask, so its rasters stay full bleed.
for s in 20 29 40 58 60 76 80 87 120 152 167 180 1024; do
  rsvg-convert -w "$s" -h "$s" "$tmp/square.svg" -o "$tmp/square-$s.png"
done
for s in 48 72 96 144 192; do
  rsvg-convert -w "$s" -h "$s" "$tmp/round.svg" -o "$tmp/round-$s.png"
  rsvg-convert -w "$s" -h "$s" "$tmp/android.svg" -o "$tmp/android-$s.png"
done

app=assets/app
rm -rf "$app"
mkdir -p "$app/windows" "$app/macos" "$app/linux" "$app/android" "$app/ios"

# Windows: one multi-size .ico; the shell picks the frame that fits.
magick \
  "$tmp/tile-16.png" "$tmp/tile-24.png" "$tmp/tile-32.png" "$tmp/tile-48.png" \
  "$tmp/tile-64.png" "$tmp/tile-128.png" "$tmp/tile-256.png" \
  "$app/windows/pandora.ico"

# macOS: a .icns is a header plus typed chunks. This ImageMagick build cannot
# write one, and the container is small enough to emit directly: every chunk
# carries a PNG, so each is a length-prefixed copy.
be32() { printf '%08x' "$1" | xxd -r -p; }
chunks=(
  icp4 "$tmp/tile-16.png"   icp5 "$tmp/tile-32.png"   icp6 "$tmp/tile-64.png"
  ic07 "$tmp/tile-128.png"  ic08 "$tmp/tile-256.png"  ic09 "$tmp/tile-512.png"
  ic10 "$tmp/tile-1024.png"
  ic11 "$tmp/tile-32.png"   ic12 "$tmp/tile-64.png"   ic13 "$tmp/tile-256.png"
  ic14 "$tmp/tile-512.png"
)
# The header carries the total length, so the chunks go to a body file first and
# the magic is prepended once that length is known.
{
  for ((i = 0; i < ${#chunks[@]}; i += 2)); do
    printf '%s' "${chunks[i]}"
    be32 "$((8 + $(stat -c%s "${chunks[i + 1]}")))"
    cat "${chunks[i + 1]}"
  done
} > "$tmp/pandora.body"
total=$((8 + $(stat -c%s "$tmp/pandora.body")))
{ printf 'icns'; be32 "$total"; cat "$tmp/pandora.body"; } > "$app/macos/pandora.icns"

# Linux: the freedesktop hicolor tree, dropped in as-is under /usr/share/icons,
# plus the desktop entry that names the icon. On Wayland the icon is not on the
# window at all: the shell reads it from the `.desktop` whose file name matches
# the window's app id (`with_app_id("pandora")` in src/main.rs).
for s in 16 24 32 48 64 128 256 512; do
  dir="$app/linux/hicolor/${s}x${s}/apps"
  mkdir -p "$dir"
  cp "$tmp/tile-$s.png" "$dir/pandora.png"
done
cat > "$app/linux/pandora.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Pandora
Comment=Offline secrets vault, one portable encrypted file
Exec=pandora
Icon=pandora
Terminal=false
Categories=Utility;Security;
StartupWMClass=pandora
DESKTOP

# Android: legacy mipmaps (still the fallback) plus the adaptive pair.
for density in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
  dir="$app/android/mipmap-${density%%:*}"
  px="${density##*:}"
  mkdir -p "$dir"
  cp "$tmp/android-$px.png" "$dir/ic_launcher.png"
  cp "$tmp/round-$px.png" "$dir/ic_launcher_round.png"
done
mkdir -p "$app/android/mipmap-anydpi-v26"
rsvg-convert -w 432 -h 432 "$tmp/foreground.svg" -o "$app/android/mipmap-xxxhdpi/ic_launcher_foreground.png"
# Solid accent, drawn as SVG rather than by ImageMagick: magick stamps a
# timestamp into the PNG, which makes the asset change on every run.
printf '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48"><rect width="48" height="48" fill="%s"/></svg>' \
  "$accent" | rsvg-convert -w 432 -h 432 -o "$app/android/mipmap-xxxhdpi/ic_launcher_background.png" -
# ponytail: one density for the adaptive pair, Android scales the rest.
cat > "$app/android/mipmap-anydpi-v26/ic_launcher.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background"/>
    <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
</adaptive-icon>
XML

# iOS: an asset catalog. Xcode names nothing, it only follows Contents.json, so
# the entries and the PNGs are generated from the same table. Every pixel size in
# it is one of the tiles rendered above.
ios="$app/ios/AppIcon.appiconset"
mkdir -p "$ios"
printf '{\n  "images" : [\n' > "$ios/Contents.json"
sep=''
while read -r idiom size scale; do
  [ -n "$idiom" ] || continue
  px=$(awk -v s="${size%%x*}" -v k="${scale%x}" 'BEGIN{printf "%d", s*k}')
  name="icon-${size}@${scale}.png"
  cp "$tmp/square-$px.png" "$ios/$name"
  printf '%s    {"filename":"%s","idiom":"%s","scale":"%s","size":"%s"}' \
    "$sep" "$name" "$idiom" "$scale" "$size" >> "$ios/Contents.json"
  sep=$',\n'
done <<'EOF'
iphone 20x20 2x
iphone 20x20 3x
iphone 29x29 2x
iphone 29x29 3x
iphone 40x40 2x
iphone 40x40 3x
iphone 60x60 2x
iphone 60x60 3x
ipad 20x20 1x
ipad 20x20 2x
ipad 29x29 1x
ipad 29x29 2x
ipad 40x40 1x
ipad 40x40 2x
ipad 76x76 1x
ipad 76x76 2x
ipad 83.5x83.5 2x
ios-marketing 1024x1024 1x
EOF
printf '\n  ],\n  "info" : {\n    "author" : "xcode",\n    "version" : 1\n  }\n}\n' \
  >> "$ios/Contents.json"

# A wrong frame size or a broken .icns header is not an error to the rasterizer:
# the OS just shows a blank or oversized icon. Check what it would see.
fail=0
check() { # $1 = expected WxH, $2 = file
  got=$(magick identify -format '%wx%h\n' "$2" 2>/dev/null | head -n1)
  if [ "$got" != "$1" ]; then
    echo "icon check failed: $2 is ${got:-missing}, want $1" >&2
    fail=1
  fi
}
check 137x128 assets/logo.png
check 256x256 assets/icon.png
check 512x512 "$app/linux/hicolor/512x512/apps/pandora.png"
check 48x48 "$app/android/mipmap-mdpi/ic_launcher.png"
check 432x432 "$app/android/mipmap-xxxhdpi/ic_launcher_background.png"
check 1024x1024 "$app/ios/AppIcon.appiconset/icon-1024x1024@1x.png"
# The rounded tile must actually be transparent at the corner, and the iOS asset
# must not be: iOS rounds a corner that is already rounded into a clipped, matte
# square, which is worse than shipping it square.
corner() { magick "$1" -format '%[pixel:p{0,0}]' info: 2>/dev/null; }
case "$(corner "$app/linux/hicolor/256x256/apps/pandora.png")" in
  *",0)") ;;
  *) echo "icon check failed: desktop tile corner is opaque" >&2; fail=1 ;;
esac
case "$(corner "$app/ios/AppIcon.appiconset/icon-1024x1024@1x.png")" in
  *",0)") echo "icon check failed: iOS icon must stay full bleed" >&2; fail=1 ;;
esac
# Optical centring: the mark's ink must sit a touch below the geometric middle
# (its mass is heavy at the top) without ever reaching the bottom edge. Measured
# as ink margins, so it fails if `NUDGE` is dropped or overshot. The same pass
# reports `reach`: the furthest ink from the centre, as a percentage of half the
# side, which is how the circular Android mask is judged below.
margins() { # $1 = png, prints "top bottom reach" as percentages
  local size
  size=$(magick identify -format '%w' "$1" 2>/dev/null)
  magick "$1" txt:- 2>/dev/null | awk -v s="$size" '
    NR > 1 {
      split($1, p, ",")
      split($2, c, "[,()]")
      if (c[5] + 0 > 200 && c[3] + 0 > 210) {
        if (first == "") first = p[2]
        last = p[2]
        d = sqrt((p[1] - s / 2) ^ 2 + (p[2] - s / 2) ^ 2)
        if (d > reach) reach = d
      }
    }
    END { printf "%.1f %.1f %.0f\n", first * 100 / s, (s - last - 1) * 100 / s, reach * 100 / (s / 2) }'
}
read -r top_pct bot_pct _ <<<"$(margins "$app/linux/hicolor/256x256/apps/pandora.png")"
awk -v t="$top_pct" -v b="$bot_pct" 'BEGIN{ exit !(t > b + 2 && b > 3) }' ||
  { echo "icon check failed: mark off centre (${top_pct}% top, ${bot_pct}% bottom)" >&2; fail=1; }
# The Android mask is a circle 100% of the half-side across — the mipmaps are
# drawn full bleed, so any ink past it is sliced — and 66% on the adaptive layer,
# whose mask leaves 36 of the 54. A wing tip is the ink that reaches furthest, so
# this fails the moment the mark grows back to the tile's size.
fits() { # $1 = png, $2 = the mask's radius, % of the half-side
  local reach
  read -r _ _ reach <<<"$(margins "$1")"
  [ "$reach" -le "$2" ] ||
    { echo "icon check failed: $1 ink reaches ${reach}% of the mask's ${2}%" >&2; fail=1; }
}
fits "$app/android/mipmap-xxhdpi/ic_launcher_round.png" 92
fits "$app/android/mipmap-xxxhdpi/ic_launcher_foreground.png" 61
ico_frames=$(magick identify -format '%wx%h\n' "$app/windows/pandora.ico" 2>/dev/null)
[ "$(printf '%s\n' "$ico_frames" | tail -n1)" = 256x256 ] &&
  [ "$(printf '%s\n' "$ico_frames" | wc -l)" -eq 7 ] ||
  { echo "icon check failed: .ico frames are $(printf '%s' "$ico_frames" | tr '\n' ' ')" >&2; fail=1; }
[ "$(head -c4 "$app/macos/pandora.icns")" = icns ] ||
  { echo "icon check failed: .icns magic" >&2; fail=1; }
[ "$(stat -c%s "$app/macos/pandora.icns")" -eq "$total" ] ||
  { echo "icon check failed: .icns header length != file size" >&2; fail=1; }
[ "$fail" -eq 0 ] || exit 1

find assets/logo.png assets/icon.png "$app" -type f | sort
