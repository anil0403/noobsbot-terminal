# noobsbot terminal — build plan

GPU-rendered Linux terminal emulator in Rust. Easy to use, with color palettes
that switch automatically based on the situation (SSH, root, git branch, time,
failed command).

Target machine first: Arch Linux, X11, Cinnamon. Wayland later through winit.

## Stack

| Concern | Crate | Why |
|---|---|---|
| Window, input events | `winit` | Standard, X11 + Wayland |
| GPU rendering | `wgpu` | Vulkan/GL backends, runs custom shaders (CRT) |
| Font shaping + rasterizing | `cosmic-text` / `swash` | Ligatures, fallback fonts, emoji |
| Terminal state + VT parsing | `alacritty_terminal` | Grid, scrollback, escape codes, vi mode, regex search, OSC 8 links. Battle-tested; we don't rewrite it |
| PTY | `alacritty_terminal::tty` | Spawns shell on a pseudo-terminal |
| Config | `serde` + `toml`, `notify` | TOML file, hot reload |
| Clipboard | `arboard` | X11/Wayland clipboard |
| Desktop notifications | `notify-send` (libnotify CLI) | Long-command-finished alerts, no D-Bus crate needed |
| Global hotkey (Quake mode) | `global-hotkey` | Works on X11 |
| Logging | `log` + `env_logger` | `RUST_LOG=debug` |

Single crate with modules. Split into a workspace only if compile times hurt.

## Architecture

```
 keyboard/mouse ──► winit event loop (main thread)
                        │
                        ├─ input.rs   key → bytes ──────────────► PTY ──► shell
                        │                                          │
                        │   reader thread ◄────────── bytes ───────┘
                        │        │
                        │        ├─ tap.rs: pull out OSC 133 / OSC 7 /
                        │        │  kitty-graphics APC / sixel DCS
                        │        └─ vte Processor ──► Term (grid + scrollback)
                        │                               (Arc<FairMutex>)
                        │   wakeup event ◄──────────────────┘
                        ▼
                    renderer: Term snapshot → glyph atlas + cell quads
                              → (optional CRT post-pass) → screen
```

Key decision: we run our **own PTY reader loop** instead of alacritty's
`EventLoop`, so every byte passes through `tap.rs` first. That is where
shell-integration marks, cwd reports and image protocols get picked up —
`alacritty_terminal` ignores those.

### Module layout

```
src/
  main.rs          entry, logging, config load
  app.rs           winit ApplicationHandler, window(s), event dispatch
  pty.rs           spawn shell, reader thread, writer, resize
  tap.rs           byte-stream scanner for OSC 133/7, APC G, DCS sixel
  term.rs          wrapper around alacritty_terminal::Term + EventListener
  input.rs         key/mouse → escape sequences, keybinding table
  config.rs        TOML schema, defaults, hot reload
  theme.rs         palettes + situation rules engine
  context.rs       detects ssh/root/cwd/git branch/time for theme rules
  pane.rs          tab list + split tree layout
  blocks.rs        command blocks from OSC 133 marks
  palette_ui.rs    command palette overlay
  links.rs         URL / path:line detection, open in editor/browser
  images.rs        kitty graphics + sixel decode, placement on grid
  render/
    mod.rs         frame orchestration, damage tracking
    atlas.rs       glyph atlas texture
    cells.wgsl     cell background + glyph shader
    crt.wgsl       CRT post-process shader
shell/
  noobsbot.bash    shell integration (OSC 133 + OSC 7)
  noobsbot.zsh
  noobsbot.fish
```

## Situation palettes (the headline feature)

Config example:

```toml
[themes.default]   # base palette, 16 colors + fg/bg/cursor
[themes.danger]    # red-tinted
[themes.night]
[themes.root]

# first matching rule wins; unmatched → "default"
[[rules]]
when = { ssh_host = "prod-*" }
theme = "danger"

[[rules]]
when = { user = "root" }
theme = "root"

[[rules]]
when = { git_branch = ["main", "master"] }
theme = "careful"

[[rules]]
when = { hours = "20-6" }
theme = "night"
```

How context is detected (no polling of external commands):

- **SSH / root / foreground program:** `tcgetpgrp(pty_fd)` gives the
  foreground process group → read `/proc/<pid>/comm`, `/proc/<pid>/cmdline`,
  `/proc/<pid>/status` (uid). Checked on each OSC 133 prompt mark and on a
  1 s timer.
- **cwd:** OSC 7 from shell integration; fallback `/proc/<pid>/cwd`.
- **git branch:** walk up from cwd to `.git/HEAD`, read the file. No `git`
  subprocess.
- **time:** checked once a minute.
- **exit code:** OSC 133;D;<code> → brief red border flash (not a theme
  switch).

Theme changes cross-fade over ~150 ms (interpolate palette in the shader).
Each pane has its own context, so a split can be red while its neighbor isn't.

## Milestones

Each milestone ends in something runnable. Commits stay small — one step per
commit.

### M0 — Toolchain + scaffold ✅
1. Install Rust: `sudo pacman -S rustup && rustup default stable`
2. `cargo init`, add deps, `cargo run` opens an empty winit window.
3. wgpu surface clears the window to the background color.

### M1 — Shell runs (headless) ✅
1. Spawn `$SHELL` on a PTY, reader thread feeds `Term`.
2. Debug dump of the grid to stdout to prove parsing works.
3. Write keyboard input to PTY; handle window resize → `TIOCSWINSZ`.

### M2 — Text on screen ✅
1. Font loading via cosmic-text, cell size from font metrics.
2. Glyph atlas (R8 texture, LRU eviction later).
3. Instanced quads: one draw for backgrounds, one for glyphs.
4. Cursor (block/beam/underline, blink).
5. Bold/italic/underline/strikethrough, 256-color + truecolor.
6. ~~Damage tracking~~ — skipped: a full frame builds in ~70 µs. Frame pacing
   to monitor refresh instead (this was the real bottleneck: 100 MB `cat`
   went 4.1 s → 1.2 s; kitty is 1.7 s on the same machine).
7. **Check:** `vttest` basics, `htop`, `vim`, `cat` of a 100 MB file without lockup.

### M3 — Daily-drivable ✅
1. Key encoding: modifiers, function keys, app cursor mode, bracketed paste.
2. Mouse reporting (SGR) for vim/tmux.
3. Scrollback with wheel + Shift+PgUp/PgDn.
4. Mouse selection (char/word/line), copy/paste via arboard, primary selection.
5. Config file at `~/.config/noobsbot/config.toml`, hot reload.
6. Font size zoom (Ctrl +/-/0).
7. `.desktop` file so it appears in the Cinnamon menu.

### M4 — Situation palettes ✅
1. Theme struct + built-in themes (default, danger, night, root, careful).
2. `context.rs` detection (foreground process, cwd, git branch, time).
3. Rules engine + unit tests.
4. Cross-fade transition.
5. ~~Exit-code border flash~~ → moved to M8: the exit code only exists once
   shell integration (OSC 133;D) does.

### M5 — Tabs and splits ✅
1. Tab bar (rendered by us, not native) with titles from OSC 0/2.
2. Split tree: horizontal/vertical, resize with mouse/keys, focus movement.
3. New tab/split opens in current pane's cwd.

### M6 — Search + vi-mode copy ✅
1. Ctrl+Shift+F search overlay using `alacritty_terminal`'s `RegexSearch`.
2. Highlight all matches, jump next/prev.
3. Vi mode (Ctrl+Shift+Space): hjkl, w/b, v/V selection, y to copy. Built on
   `alacritty_terminal::vi_mode`.
4. ~~Scrollback `0` = unlimited~~ — skipped: `terminal.scrollback` already
   sets the size; unlimited history is a memory foot-gun nobody asked for.

### M7 — Links ✅
1. OSC 8 hyperlinks (already parsed by alacritty_terminal) → underline on
   Ctrl+hover (only with Ctrl, so plain clicks still select).
2. Regex hints for URLs and `path:line[:col]`; paths only count if the file
   exists, so `host:8080` isn't a link.
3. Ctrl+click: URL → `xdg-open`; path → `$EDITOR +line path` in a split,
   resolved against the pane's cwd (`/proc` for now, OSC 7 once M8 lands).

### M8 — Shell integration + command blocks ✅
1. `shell/noobsbot.bash`, `shell/zsh/` emitting OSC 133 A/B/C/D and OSC 7;
   auto-injected via `--rcfile`/`ZDOTDIR` so users don't edit dotfiles. Fish
   ≥ 4 emits both itself, so it needs nothing. (zsh script untested: no zsh on
   the dev machine.)
2. `tap.rs` scans the PTY stream; on 133;A the reader injects a hidden OSC 8
   hyperlink (`id=noobsbot-prompt-<n>`) that stays open until 133;B, so prompt
   cells carry their number through scrolling and reflow. `blocks.rs` finds
   blocks from those tags and keeps exit code + duration per prompt number.
3. Gutter marks per block (green/red, yellow while running), jump prev/next
   command with Ctrl+Shift+Z/X (Ctrl+Shift+↑/↓ already resize splits).
4. Copy last command's output (Ctrl+Shift+G). ~~Collapse/expand a block~~ —
   skipped: hiding grid rows breaks the 1:1 screen↔grid mapping that mouse,
   selection and vi mode rely on; jump + copy cover the use.
5. `notify-send` when a command ≥ 10 s finishes and the window isn't focused.
6. Exit-code red border flash (moved here from M4).

### M9 — Command palette ✅
1. Every action registered once in `input::actions()` with a name + default
   keys; `binding()` reads the same table.
2. Ctrl+Shift+P overlay with fuzzy match, shows the keybinding. ~~`nucleo-matcher`~~
   — a 20-line subsequence scorer is plenty for ~40 entries.
3. Includes "Theme: …" (holds until the situation changes) and every action.
   "Toggle CRT" joins when M11 exists.

### M10 — Quake mode ✅
1. `--quake` flag: borderless, top of screen, `quake.height` % (default 40),
   always on top.
2. Global hotkey (`quake.hotkey`, default F12) via an X11 key grab with
   x11rb (already in the tree through winit). ~~Slide animation~~ → 150 ms
   fade via `_NET_WM_WINDOW_OPACITY`: the WM clamps managed windows on screen,
   so they can't slide in from above. Focus is requested as a pager
   (`_NET_ACTIVE_WINDOW` source 2), since plain requests get refused as
   focus stealing.
3. Single-instance for quake: a second `noobsbot --quake` sends "toggle" over
   `$XDG_RUNTIME_DIR/noobsbot-quake-<uid>.sock` and exits, which also makes a
   desktop shortcut work on Wayland. Plain `noobsbot` launches stay separate
   windows.

### M11 — CRT mode
1. Render terminal to offscreen texture.
2. `crt.wgsl` post-pass: scanlines, bloom/glow, barrel curvature, vignette,
   subtle flicker. All intensities configurable, toggle from the palette.

### M12 — Inline images
1. Kitty graphics protocol (APC `_G`): transmit (direct + file), place, delete.
   Test with `kitten icat`.
2. Sixel (DCS `q`) decode to RGBA. Test with `img2sixel`.
3. Images anchored to grid cells, scroll with text, evicted with scrollback.

### M13 — Polish + release
1. Custom terminfo `noobsbot` (fallback `xterm-256color`).
2. Performance pass: compare `cat` throughput and input latency with Alacritty.
3. Wayland check.
4. README with screenshots, AUR `PKGBUILD`.

## Testing

- Unit tests for pure logic: key encoding, `tap.rs` scanner, rules engine,
  link regex, block tracking.
- Manual checks per milestone: `vttest`, `vim`, `htop`, `tmux`, `kitten icat`,
  `img2sixel`, `ssh` into a host matching a rule.
- Throughput: `time cat big.txt`.

## Order rationale

M0–M3 get a terminal you can actually use; everything after is a feature on
top of a working base. M4 (palettes) comes right after because it's the
reason this project exists. Images are last because they touch the parser,
grid and renderer at once and are the least used day to day.
