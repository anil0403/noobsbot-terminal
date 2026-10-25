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
# Signed with the "Noobsbot Terminal" certificate when it is in the keychain
# (CI loads it into $SIGN_KEYCHAIN). macOS then knows the app by its id and that
# certificate, so permissions like Screen Recording stay after an update. With
# an ad-hoc signature it knows only this build's hash, and every update loses
# them. Not notarized either way.
id="Noobsbot Terminal"
if security find-identity -p codesigning ${SIGN_KEYCHAIN:+"$SIGN_KEYCHAIN"} 2>/dev/null | grep -q "\"$id\""; then
    codesign --force --sign "$id" ${SIGN_KEYCHAIN:+--keychain "$SIGN_KEYCHAIN"} "$app"
    signed=$id
elif [[ -n ${REQUIRE_SIGNING:-} ]]; then
    echo "no \"$id\" certificate to sign with" >&2
    exit 1
else
    codesign --force --sign - "$app"   # Apple Silicon runs nothing unsigned
    signed=ad-hoc
fi
echo "$app ($(lipo -archs "$app/Contents/MacOS/noobsbot"), $ver, $signed)"
