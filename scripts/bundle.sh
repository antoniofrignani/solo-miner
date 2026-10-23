#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked
app="dist/Solo Miner.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/solo-miner "$app/Contents/MacOS/solo-miner"
cp packaging/SoloMiner.icns "$app/Contents/Resources/SoloMiner.icns"
cp LICENSE "$app/Contents/Resources/LICENSE"
cp packaging/Info.plist "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf 'Bundle created: %s\n' "$app"
