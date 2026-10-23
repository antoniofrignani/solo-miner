#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cargo run --locked --release --example app_icon -- "$work/icon.png"
mkdir "$work/SoloMiner.iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/icon.png" --out "$work/SoloMiner.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$work/icon.png" --out "$work/SoloMiner.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/SoloMiner.iconset" -o packaging/SoloMiner.icns
