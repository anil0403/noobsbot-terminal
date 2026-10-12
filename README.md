# noobsbot terminal

A GPU-rendered terminal emulator for Linux, written in Rust. Easy to use, fast,
and it changes its color palette to match the situation: red when you SSH into
production, amber as root, a different look on `main` or after dark.

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

## Situation themes

Built-in themes: `default`, `danger`, `root`, `careful`, `night`. Rules in the
config pick one based on what's happening in the terminal; the first rule whose
conditions all match wins, and the switch fades in over 150 ms.

```toml
[[rules]]
when = { ssh_host = ["prod-*", "*.prod.example.com"] }
theme = "danger"

[[rules]]
when = { user = "root" }
theme = "root"

[[rules]]
when = { git_branch = ["main", "master"], cwd = "~/work/*" }
theme = "careful"

[[rules]]
when = { hours = "22-6" }
theme = "night"
```

Conditions: `ssh_host`, `user`, `git_branch`, `cwd`, `process` (globs with `*`
and `?`, or a list of them) and `hours`. Detection reads `/proc` and
`.git/HEAD` directly, so nothing is spawned. Out of the box, root and any SSH
host containing "prod" are covered.

Make your own theme by naming it and listing only the colors that differ:

```toml
[themes.ocean]
background = "#0b1a2b"
cursor = "#5fd7ff"
```

## License

[MIT](LICENSE)
