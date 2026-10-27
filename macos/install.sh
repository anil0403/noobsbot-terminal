#!/usr/bin/env bash
# noobsbot terminal installer for macOS:
#   curl -fsSL https://github.com/noobsbot-team/noobsbot-terminal/raw/main/macos/install.sh | bash
# Installs Noobsbot.app (the latest release, or built from source when there is
# none), the `noobsbot` command and the JetBrainsMono Nerd Font.
# Running it again updates noobsbot. NOOBSBOT_FROM_SOURCE=1 always builds;
# NOOBSBOT_BETA=1 installs the newest release, betas included.
set -euo pipefail

# All in a function, so nothing runs until the whole script has downloaded.
main() {

REPO=noobsbot-team/noobsbot-terminal
RUST_MIN=90 # rustc 1.90 or newer
SRC=$HOME/Library/Caches/noobsbot/src
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

say() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

[[ $(uname -s) == Darwin ]] || die "this installer is for macOS; on Linux use install.sh at the repo's top."
[[ $EUID -ne 0 ]] || die "run this as your user, not root."

app=""
if [[ -z ${NOOBSBOT_FROM_SOURCE:-} ]]; then
    # NOOBSBOT_BETA=1 (noobsbot -u --beta): the newest release, betas included.
    releases=releases/latest
    [[ -z ${NOOBSBOT_BETA:-} ]] || releases="releases?per_page=1"
    url=$(curl -fsSL "https://api.github.com/repos/$REPO/$releases" 2>/dev/null |
        sed -n 's/.*"browser_download_url": *"\([^"]*macos-universal\.zip\)".*/\1/p' | head -1 || true)
    if [[ -n $url ]]; then
        say "Downloading $(basename "$url")"
        curl -fsSL "$url" -o "$TMP/noobsbot.zip"
        # Checked before it's unpacked: its quarantine flag is removed below.
        sum=$(curl -fsSL "$url.sha256" | cut -d' ' -f1)
        [[ -n $sum ]] || die "can't get the download's checksum."
        echo "$sum  $TMP/noobsbot.zip" | shasum -a 256 -c --status || die "the download doesn't match its checksum."
        ditto -x -k "$TMP/noobsbot.zip" "$TMP"
        app=$TMP/Noobsbot.app
        [[ -d $app ]] || die "the download has no Noobsbot.app"
    fi
fi

if [[ -z $app ]]; then
    say "No release to download; building from source"
    xcode-select -p >/dev/null 2>&1 || {
        xcode-select --install 2>/dev/null || true
        die "install the Xcode Command Line Tools (a window just opened), then run this again."
    }
    [[ -f $HOME/.cargo/env ]] && . "$HOME/.cargo/env"
    minor=$(rustc --version 2>/dev/null | sed -n 's/^rustc 1\.\([0-9]*\).*/\1/p' || true)
    if (( ${minor:-0} < RUST_MIN )); then
        say "Installing Rust (rustup)"
        if has rustup; then rustup update stable && rustup default stable
        else curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal; fi
        . "$HOME/.cargo/env"
    fi
    say "Building noobsbot (a few minutes)"
    if [[ -d $SRC/.git ]]; then git -C "$SRC" pull --ff-only; else rm -rf "$SRC"; git clone --depth 1 "https://github.com/$REPO" "$SRC"; fi
    (cd "$SRC/macos" && cargo build --release --locked && ./package.sh)
    app=$SRC/macos/target/Noobsbot.app
fi

# /Applications when we may write there (admin users), else ~/Applications.
dest=/Applications
[[ -w $dest ]] || { dest=$HOME/Applications; mkdir -p "$dest"; }
say "Installing $dest/Noobsbot.app"
rm -rf "$dest/Noobsbot.app"
ditto "$app" "$dest/Noobsbot.app"
# Unsigned by Apple: without this Gatekeeper says it's "damaged".
xattr -dr com.apple.quarantine "$dest/Noobsbot.app" 2>/dev/null || true

# The `noobsbot` command (noobsbot --quake for the drop-down terminal).
bin=""
for d in /opt/homebrew/bin /usr/local/bin; do [[ -d $d && -w $d ]] && { bin=$d; break; }; done
if [[ -z $bin ]]; then bin=$HOME/.local/bin; mkdir -p "$bin"; fi
ln -sf "$dest/Noobsbot.app/Contents/MacOS/noobsbot" "$bin/noobsbot"
case ":$PATH:" in *":$bin:"*) ;; *) say "Add $bin to your PATH to run \`noobsbot\` from a shell." ;; esac

if ! ls "$HOME/Library/Fonts" /Library/Fonts 2>/dev/null | grep -q JetBrainsMonoNerdFontMono; then
    say "Installing the JetBrainsMono Nerd Font"
    curl -fsSL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/JetBrainsMono.tar.xz | tar -xJ -C "$TMP"
    mkdir -p "$HOME/Library/Fonts"
    cp "$TMP"/JetBrainsMonoNerdFontMono-*.ttf "$HOME/Library/Fonts/"
fi

say "Done. Open Noobsbot Terminal from Launchpad or Spotlight (Cmd+Space, \"noobsbot\")."
say "Settings: ~/.config/noobsbot/config.toml (written on first run). Cmd+Shift+P shows every action."
}

main "$@"
