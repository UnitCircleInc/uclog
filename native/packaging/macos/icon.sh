#!/usr/bin/env bash
# Make ucLog.icns from icon.svg (needs rsvg-convert: brew install librsvg). The .icns is committed,
# so the release build does not need this; run it again after changing icon.svg.
set -euo pipefail
cd "$(dirname "$0")"
set_dir=$(mktemp -d)/ucLog.iconset
mkdir "$set_dir"
for s in 16 32 128 256 512; do
  rsvg-convert -w "$s" -h "$s" icon.svg -o "$set_dir/icon_${s}x${s}.png"
  rsvg-convert -w $((s * 2)) -h $((s * 2)) icon.svg -o "$set_dir/icon_${s}x${s}@2x.png"
done
iconutil -c icns "$set_dir" -o ucLog.icns
