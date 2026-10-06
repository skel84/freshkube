#!/usr/bin/env bash
# Regenerate the macOS app icon, packaging/icon/freshkube.icns, from
# packaging/icon/freshkube.svg.
#
# Usage: scripts/app-icon.sh
#
# Needs rsvg-convert (`brew install librsvg`) and iconutil. The SVG draws its
# own rounded tile, so it is placed on Apple's icon grid as it is: an 824
# tile centred on a 1024 canvas, 100 clear on each side. Every size is drawn
# from the vector rather than scaled down from the largest.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SOURCE="$ROOT/packaging/icon/freshkube.svg"
OUTPUT="$ROOT/packaging/icon/freshkube.icns"

command -v rsvg-convert >/dev/null || {
    echo "app-icon: rsvg-convert is missing; brew install librsvg" >&2
    exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK:?}"' EXIT

# The source's own 512 viewBox, nested inside the 1024 grid.
{
    echo '<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">'
    echo '<svg x="100" y="100" width="824" height="824" viewBox="0 0 512 512">'
    sed -e 's|^<svg[^>]*>||' -e 's|</svg>[[:space:]]*$||' "$SOURCE"
    echo '</svg></svg>'
} >"$WORK/grid.svg"

ICONSET="$WORK/freshkube.iconset"
mkdir "$ICONSET"
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" -o "$ICONSET/icon_${size}x${size}.png" "$WORK/grid.svg"
    double=$((size * 2))
    rsvg-convert -w "$double" -h "$double" -o "$ICONSET/icon_${size}x${size}@2x.png" "$WORK/grid.svg"
done
iconutil --convert icns --output "$OUTPUT" "$ICONSET"
echo "app-icon: wrote ${OUTPUT#"$ROOT"/}"
