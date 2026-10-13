#!/usr/bin/env bash
# noobsbot terminal installer:
#   curl -fsSL https://raw.githubusercontent.com/anil0403/noobsbot-terminal/main/install.sh | bash
# Installs the dependencies (nano and the JetBrainsMono Nerd Font included),
# builds noobsbot from source and makes nano the default editor.
# Running it again updates noobsbot.
set -euo pipefail

# All in a function, so nothing runs until the whole script has downloaded.
main() {

REPO=https://github.com/anil0403/noobsbot-terminal
FONT="JetBrainsMono Nerd Font Mono"
RUST_MIN=90 # rustc 1.90 or newer
SRC=${XDG_CACHE_HOME:-$HOME/.cache}/noobsbot/src

say() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
has() { command -v "$1" >/dev/null 2>&1; }

[[ $(uname -s) == Linux ]] || die "noobsbot runs on Linux only."
[[ $EUID -ne 0 ]] || die "run this as your user, not root (it asks for sudo when needed)."
SUDO=sudo; has sudo || SUDO=doas; has $SUDO || die "needs sudo (or doas) to install packages."

# Dependencies: build tools, the X11/Vulkan libraries noobsbot loads at runtime,
# tic (terminfo), nano and the font.
say "Installing dependencies"
if has pacman; then
    $SUDO pacman -S --needed --noconfirm base-devel git # the package pulls in nano and the font
elif has apt-get; then
    $SUDO apt-get update
    $SUDO apt-get install -y git curl build-essential nano ncurses-bin xz-utils fontconfig \
        libx11-6 libxcursor1 libxi6 libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers
elif has dnf; then
    $SUDO dnf install -y git curl gcc nano ncurses xz fontconfig \
        libX11 libXcursor libXi libxkbcommon-x11 vulkan-loader mesa-vulkan-drivers
elif has zypper; then
    $SUDO zypper --non-interactive install git curl gcc nano ncurses-utils xz fontconfig \
        libX11-6 libXcursor1 libXi6 libxkbcommon-x11-0 libvulkan1 libvulkan_intel libvulkan_radeon
else
    say "Unknown package manager: install git, a C compiler, nano, tic (ncurses), and the"
    say "X11 (libX11, libXcursor, libXi, libxkbcommon-x11) and Vulkan libraries yourself."
fi

if has pacman; then
    # Arch: a real package (pacman -R noobsbot-git removes it), built from extra/aur.
    say "Building the noobsbot-git package"
    rm -rf "$SRC" && mkdir -p "$SRC"
    curl -fsSL "$REPO/raw/main/extra/aur/PKGBUILD" -o "$SRC/PKGBUILD"
    (cd "$SRC" && makepkg -si --noconfirm --needed --cleanbuild)
else
    # Rust: the distro's if new enough, else rustup's (in ~/.cargo, no root).
    [[ -f $HOME/.cargo/env ]] && . "$HOME/.cargo/env"
    minor=$(rustc --version 2>/dev/null | sed -n 's/^rustc 1\.\([0-9]*\).*/\1/p' || true)
    if (( ${minor:-0} < RUST_MIN )); then
        say "Installing Rust (rustup)"
        if has rustup; then rustup update stable && rustup default stable
        else curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal; fi
        . "$HOME/.cargo/env"
    fi

    say "Building noobsbot (a few minutes)"
    if [[ -d $SRC/.git ]]; then git -C "$SRC" pull --ff-only; else rm -rf "$SRC"; git clone --depth 1 "$REPO" "$SRC"; fi
    (cd "$SRC" && cargo build --release --locked)

    say "Installing"
    $SUDO install -Dm755 "$SRC/target/release/noobsbot" /usr/local/bin/noobsbot
    $SUDO install -Dm644 "$SRC/extra/noobsbot.desktop" /usr/local/share/applications/noobsbot.desktop
    $SUDO install -Dm644 "$SRC/extra/noobsbot.svg" /usr/local/share/icons/hicolor/scalable/apps/noobsbot.svg
    has tic && $SUDO tic -x "$SRC/extra/noobsbot.terminfo"

    if [[ $(fc-list 2>/dev/null) != *"$FONT"* ]]; then
        say "Installing the $FONT font"
        fonts=${XDG_DATA_HOME:-$HOME/.local/share}/fonts/JetBrainsMonoNerd
        mkdir -p "$fonts"
        curl -fsSL https://github.com/ryanoasis/nerd-fonts/releases/latest/download/JetBrainsMono.tar.xz | tar -xJ -C "$fonts"
        fc-cache -f "$fonts" >/dev/null
    fi
fi

# nano as the default editor (git, gh, crontab, ...), unless one is already set.
say "Setting nano as the default editor"
line='export EDITOR=nano VISUAL=nano'
for rc in ~/.bashrc ~/.zshrc; do
    if [[ -f $rc ]] && ! grep -qE '^\s*(export\s+)?(EDITOR|VISUAL)=' "$rc"; then
        printf '\n# Default editor (added by the noobsbot installer)\n%s\n' "$line" >> "$rc"
    fi
done
if [[ ! -f ~/.bashrc && ! -f ~/.zshrc ]]; then
    printf '\n# Default editor (added by the noobsbot installer)\n%s\n' "$line" >> ~/.profile
fi
if has fish && ! fish -c 'set -q EDITOR'; then
    fish -c 'set -Ux EDITOR nano; set -Ux VISUAL nano'
fi
if has gh && [[ -z $(gh config get editor 2>/dev/null) ]]; then
    gh config set editor nano
fi

say "Done. Open \"noobsbot terminal\" from your app menu, or run: noobsbot"
}

main "$@"
