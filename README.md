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
| Ctrl+Shift+T | New tab (opens in the current shell's directory) |
| Ctrl+Shift+E / Ctrl+Shift+O | Split right / split down |
| Ctrl+Shift+W | Close pane (the last pane closes its tab) |
| Ctrl+Tab / Ctrl+Shift+Tab, Alt+1…9, click | Next / previous / numbered tab |
| Alt+Arrows, click | Move focus between split panes |
| Ctrl+Shift+Arrows, drag the divider | Resize splits |
| Ctrl+Shift+F | Search history (regex, or plain text if it isn't one). Enter / Shift+Enter: older / newer match, Esc: close |
| Ctrl+Shift+Space | Vi mode: `hjkl` `wbe` `0$^` `g` `G` `Ctrl+U/D`, `v` / `V` / `Ctrl+V` select, `y` copy, `/` search, `n` / `N` next, Esc / `q` leave |

## Config

`~/.config/noobsbot/config.toml` is created with every option documented on
first launch ([example](config.example.toml)). Changes apply as soon as you
save.

## Situation themes

Built-in themes: `default`, `danger`, `root`, `careful`, `night`. Rules in the
config pick one based on what's happening in the terminal; the first rule whose
conditions all match wins, and the switch fades in over 150 ms. Every pane
picks its own theme, and each tab carries its pane's accent color, so an SSH
session to production stays red in the tab bar while you work elsewhere.

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
