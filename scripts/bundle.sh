#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked
app="dist/Solo Miner.app"
mkdir -p "$app/Contents/MacOS"
cp target/release/solo-miner "$app/Contents/MacOS/solo-miner"
cp packaging/Info.plist "$app/Contents/Info.plist"
codesign --force --sign - "$app"
printf 'Bundle creato: %s\n' "$app"
