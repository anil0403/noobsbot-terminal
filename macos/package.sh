#!/usr/bin/env bash
# Wraps the release build in Noobsbot.app (target/Noobsbot.app). Builds for
# both Apple Silicon and Intel make it a universal app.
set -euo pipefail
cd "$(dirname "$0")"
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
bins=()
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    [[ -f target/$t/release/noobsbot ]] && bins+=("target/$t/release/noobsbot")
done
(( ${#bins[@]} )) || bins=(target/release/noobsbot)
app=target/Noobsbot.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "${bins[@]}" -output "$app/Contents/MacOS/noobsbot"
sed "s/@VERSION@/$ver/" extra/Info.plist > "$app/Contents/Info.plist"
cp extra/noobsbot.icns extra/noobsbot.terminfo extra/config.example.toml "$app/Contents/Resources/"
# Ad-hoc signature: Apple Silicon runs nothing unsigned. Not notarized.
codesign --force --sign - "$app"
echo "$app ($(lipo -archs "$app/Contents/MacOS/noobsbot"), $ver)"
