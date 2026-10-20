#!/usr/bin/env bash
# Regenerate the UI icon PNGs in assets/icons from the committed lucide SVGs
# (same 64x64 RGBA format as the brand sprites).
set -euo pipefail

cd "$(dirname "$0")/.."
out=assets/icons

for svg in "$out"/*.svg; do
  rsvg-convert -w 64 -h 64 -b none "$svg" -o "${svg%.svg}.png"
done

echo "done: $(ls "$out"/*.png | wc -l) icons"
