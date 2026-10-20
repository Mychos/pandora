#!/usr/bin/env bash
# Regenerate the brand icon PNGs in assets/icons/brands.
# Sources are simple-icons (CC0), rasterized to the same 64x64 RGBA format
# the lucide sprites use. Current icons come from the official CDN (white);
# the brands simple-icons purged in 2026 (microsoft, amazon, slack, linkedin,
# onedrive…) are pinned to tag 9.21.0 and recolored to white.
set -euo pipefail

cd "$(dirname "$0")/.."
out=assets/icons/brands
mkdir -p "$out"

# slug is the simple-icons name, stored in the vault as Item::icon
current=(
  github gitlab bitbucket google apple netflix spotify discord twitch
  youtube x instagram facebook telegram whatsapp dropbox googledrive
  protonmail paypal steam reddit cloudflare ebay shopify zoom stripe
)
# CC0 even though the project later dropped these brands
pin=9.21.0
purged=(microsoft amazon amazonaws slack linkedin microsoftonedrive)

for slug in "${current[@]}"; do
  curl -fsSL "https://cdn.simpleicons.org/$slug/FFFFFF" -o "$out/$slug.svg"
  rsvg-convert -w 64 -h 64 -b none "$out/$slug.svg" -o "$out/$slug.png"
  rm "$out/$slug.svg"
done

for slug in "${purged[@]}"; do
  curl -fsSL "https://raw.githubusercontent.com/simple-icons/simple-icons/$pin/icons/$slug.svg" \
    -o "$out/$slug.svg"
  sed -i 's|<svg |<svg fill="#FFFFFF" |' "$out/$slug.svg"
  rsvg-convert -w 64 -h 64 -b none "$out/$slug.svg" -o "$out/$slug.png"
  rm "$out/$slug.svg"
  echo "ok $slug"
done

echo "done: $(( ${#current[@]} + ${#purged[@]} )) icons"