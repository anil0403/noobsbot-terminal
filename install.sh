#!/usr/bin/env bash
# noobsbot terminal installer:
#   curl -fsSL https://github.com/noobsbot-team/noobsbot-terminal/raw/main/install.sh | bash
# Installs the dependencies (nano and the JetBrainsMono Nerd Font included),
# installs the latest release's prebuilt noobsbot (or builds it from source when
# that can't run here) and makes nano the default editor.
# Running it again updates noobsbot. NOOBSBOT_FROM_SOURCE=1 always builds;
# NOOBSBOT_BETA=1 installs the newest release, betas included.
set -euo pipefail

# All in a function, so nothing runs until the whole script has downloaded.
main() {

REPO=https://github.com/noobsbot-team/noobsbot-terminal
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

# The latest release's prebuilt tarball, when it runs here: x86_64 with glibc 2.35
# or newer (what release.yml builds against). Empty → build from source: no
# release yet, its build still running, another CPU, musl, or NOOBSBOT_FROM_SOURCE=1.
tarball="" ver=""
# NOOBSBOT_BETA=1 (noobsbot -u --beta): the newest release, betas included.
releases=releases/latest
[[ -z ${NOOBSBOT_BETA:-} ]] || releases="releases?per_page=1"
glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | cut -d' ' -f2 || true)
if [[ -z ${NOOBSBOT_FROM_SOURCE:-} && $(uname -m) == x86_64 && -n $glibc &&
      $(printf '2.35\n%s\n' "$glibc" | sort -V | head -1) == 2.35 ]]; then
    tarball=$(curl -fsSL "https://api.github.com/repos/${REPO#https://github.com/}/$releases" 2>/dev/null |
        sed -n 's/.*"browser_download_url": *"\([^"]*x86_64-linux\.tar\.gz\)".*/\1/p' | head -1 || true)
    ver=$(basename "$tarball" | sed -n 's/^noobsbot-\(.*\)-x86_64-linux\.tar\.gz$/\1/p')
    [[ -n $ver ]] || tarball=""
fi
sha256() { curl -fsSL "$tarball.sha256" | cut -d' ' -f1; }

if has pacman; then
    # Arch: a real package (pacman -R noobsbot-bin or noobsbot-git removes it):
    # noobsbot-bin wraps the tarball (extra/aur-bin), noobsbot-git builds (extra/aur).
    if [[ -n $tarball ]]; then pkg=noobsbot-bin dir=aur-bin old=noobsbot-git
    else pkg=noobsbot-git dir=aur old=noobsbot-bin; fi
    # pacman has no '-' in a version: a beta 0.6.1-beta.2 is 0.6.1beta2, which it
    # sorts before 0.6.1 too.
    pkgver=${ver/-beta./beta}
    if [[ -n $tarball && $(pacman -Q $pkg 2>/dev/null) == "$pkg $pkgver-1" ]]; then
        say "noobsbot $ver is already installed"
    else
        say "Making the $pkg package"
        rm -rf "$SRC" && mkdir -p "$SRC"
        curl -fsSL "$REPO/raw/main/extra/$dir/PKGBUILD" -o "$SRC/PKGBUILD"
        if [[ -n $tarball ]]; then
            sed -i "s/^pkgver=.*/pkgver=$pkgver/; s/^sha256sums=.*/sha256sums=('$(sha256)')/" "$SRC/PKGBUILD"
        fi
        (cd "$SRC" && makepkg -sf --noconfirm --cleanbuild)
        # The two conflict, and under --noconfirm pacman refuses to swap them itself.
        if pacman -Q $old >/dev/null 2>&1; then $SUDO pacman -R --noconfirm $old; fi
        $SUDO pacman -U --noconfirm "$SRC"/$pkg-*.pkg.tar.*
    fi
else
    if [[ -n $tarball ]]; then
        say "Downloading $(basename "$tarball")"
        tmp=$(mktemp -d)
        trap 'rm -rf "$tmp"' EXIT
        curl -fsSL "$tarball" -o "$tmp/noobsbot.tar.gz"
        echo "$(sha256)  $tmp/noobsbot.tar.gz" | sha256sum -c --quiet || die "the download doesn't match its checksum."
        tar -xzf "$tmp/noobsbot.tar.gz" -C "$tmp"
        bin=$tmp/noobsbot-$ver-x86_64-linux/noobsbot files=$tmp/noobsbot-$ver-x86_64-linux
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
        bin=$SRC/target/release/noobsbot files=$SRC/extra
    fi

    say "Installing"
    $SUDO install -Dm755 "$bin" /usr/local/bin/noobsbot
    $SUDO install -Dm644 "$files/noobsbot.desktop" /usr/local/share/applications/noobsbot.desktop
    $SUDO install -Dm644 "$files/noobsbot.svg" /usr/local/share/icons/hicolor/scalable/apps/noobsbot.svg
    has tic && $SUDO tic -x "$files/noobsbot.terminfo"

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
