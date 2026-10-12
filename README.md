# noobsbot terminal

A GPU-rendered terminal emulator for Linux, written in Rust. Easy to use, fast,
and (soon) it changes its color palette to match the situation: red when you
SSH into production, a different look as root or on `main`, a night theme
after dark.

> Work in progress. See [PLAN.md](PLAN.md) for the roadmap and what's done.

## Install

Needs Rust (`sudo pacman -S rustup && rustup default stable` on Arch).

```sh
cargo build --release
install -Dm755 target/release/noobsbot ~/.local/bin/noobsbot
install -Dm644 extra/noobsbot.desktop ~/.local/share/applications/noobsbot.desktop
```

`~/.local/bin` must be on your `PATH`. "noobsbot terminal" then shows up in
your desktop's app menu.

## Keys

| Keys | Action |
|---|---|
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste |
| Shift+Insert, middle click | Paste (middle click pastes the mouse selection) |
| Mouse drag, double / triple click | Select text, word, line (copied to the mouse selection) |
| Shift + mouse | Select even when an app (vim, tmux) captures the mouse |
| Shift+PgUp / Shift+PgDn, wheel | Scroll through history |
| Ctrl+Shift+Home / End | Jump to top / bottom of history |
| Ctrl+= / Ctrl+- / Ctrl+0 | Bigger / smaller / reset font |

## Config

`~/.config/noobsbot/config.toml` is created with every option documented on
first launch ([example](config.example.toml)). Changes apply as soon as you
save.

## License

[MIT](LICENSE)
