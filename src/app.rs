use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, TermMode};
use alacritty_terminal::tty;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::dpi::PhysicalSize;
use winit::window::{CursorIcon, Window, WindowId, WindowLevel};

use crate::blocks::{self, Blocks};
use crate::clipboard::Clipboard;
use crate::config::{self, Config};
use crate::context::{self, Context};
use crate::font::Font;
use crate::input::{self, Action, Vi, binding, key_bytes, mouse_report, paste_bytes, vi_binding};
use crate::links::{self, Hints, Link, Target};
use crate::palette::{Item, Palette};
use crate::pane::{self, Dir, Divider, Node, Pane, Rect, Tab};
use crate::pty::Pty;
use crate::render::{Renderer, crt};
use crate::search::Search;
use crate::tap::Mark;
use crate::shell;
use crate::term::{Listener, PaneId, Term, UserEvent};
use crate::theme::{self, Fade, Theme};

const BLINK: Duration = Duration::from_millis(530);
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Minimum gap between situation checks while output is streaming.
const CONTEXT_THROTTLE: Duration = Duration::from_millis(200);
/// Re-check at least this often when a rule depends on the time of day.
const CONTEXT_CLOCK: Duration = Duration::from_secs(60);
/// Widest a tab gets, in cells.
const TAB_CELLS: f32 = 30.0;
/// Step for keyboard split resizing, as a fraction of the split.
const RESIZE_STEP: f32 = 0.05;
/// A command running at least this long notifies when it ends while the window is unfocused.
const NOTIFY_AFTER: Duration = Duration::from_secs(10);
/// Red border after a failed command.
const FLASH: Duration = Duration::from_millis(400);
/// Quake window fade in/out.
const FADE: Duration = Duration::from_millis(150);

/// The drop-down window's state.
struct Quake {
    visible: bool,
    /// Fading in (true) or out since this time.
    fade: Option<(Instant, bool)>,
    last_toggle: Instant,
}

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    config: Config,
    _config_watcher: Option<notify::RecommendedWatcher>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    panes: HashMap<PaneId, Pane>,
    tabs: Vec<Tab>,
    active: usize,
    next_pane: PaneId,
    /// Dividers of the active tab, from the last layout.
    dividers: Vec<Divider>,
    /// Index into `dividers` of the one being dragged.
    drag: Option<usize>,
    cursor_icon: CursorIcon,
    window_title: String,
    /// Scrollback search in the focused pane.
    search: Option<Search>,
    hints: Hints,
    palette: Option<Palette>,
    /// Link under the pointer while Ctrl is held.
    hover: Option<(PaneId, Link)>,
    mods: ModifiersState,
    next_blink: Instant,
    /// Something on screen changed since the last frame.
    dirty: bool,
    last_frame: Instant,
    frame_time: Duration,
    /// Sub-line remainder of touchpad scrolling, in pixels.
    scroll_px: f64,
    mouse_px: (f32, f32),
    /// Pane under the pointer; kept while a button is held so drags stay in their pane.
    mouse_pane: Option<PaneId>,
    /// Pointer position in that pane's viewport cells (col, line).
    mouse_cell: (usize, usize),
    /// Button held down, for drag reporting.
    mouse_held: Option<u8>,
    /// Pointer x within its cell is past the middle → selection side.
    mouse_side: Side,
    /// Dragging out a selection with the left button.
    selecting: bool,
    last_click: Instant,
    clicks: u8,
    clipboard: Clipboard,
    /// Font size change from Ctrl+=/Ctrl+-, in px on top of the config size.
    zoom: f32,
    /// Running as the drop-down terminal (`--quake`).
    quake: Option<Quake>,
    /// CRT effect on (config, flipped from the palette).
    crt: bool,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, quake: bool) -> Self {
        let path = config::path();
        let config = config::load(&path).unwrap_or_else(|e| {
            log::error!("config error, using defaults: {e}");
            Config::default()
        });
        if quake {
            crate::quake::serve(proxy.clone());
            if !config.quake.hotkey.is_empty() {
                crate::quake::grab(&config.quake.hotkey, proxy.clone());
            }
        }
        let quake = quake.then(|| Quake {
            visible: false,
            fade: None,
            last_toggle: Instant::now() - Duration::from_secs(1),
        });
        Self {
            crt: config.crt.enabled,
            quake,
            _config_watcher: config::watch(&path, proxy.clone()),
            config,
            proxy,
            window: None,
            renderer: None,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_pane: 0,
            dividers: Vec::new(),
            drag: None,
            cursor_icon: CursorIcon::Default,
            window_title: String::new(),
            search: None,
            hints: Hints::new(),
            palette: None,
            hover: None,
            mods: ModifiersState::empty(),
            next_blink: Instant::now(),
            dirty: false,
            last_frame: Instant::now(),
            frame_time: Duration::from_micros(16_667),
            scroll_px: 0.0,
            mouse_px: (0.0, 0.0),
            mouse_pane: None,
            mouse_cell: (0, 0),
            mouse_held: None,
            mouse_side: Side::Left,
            selecting: false,
            last_click: Instant::now(),
            clicks: 0,
            clipboard: Clipboard::new(),
            zoom: 0.0,
        }
    }
}

/// Grid point at a viewport cell (accounts for scrollback offset).
fn grid_point(term: &Term, (col, line): (usize, usize)) -> Point {
    Point::new(Line(line as i32 - term.grid().display_offset() as i32), Column(col))
}

fn window_size(font: &Font, width: f32, height: f32) -> WindowSize {
    WindowSize {
        num_cols: (width / font.cell_w).max(1.0) as u16,
        num_lines: (height / font.cell_h).max(1.0) as u16,
        cell_width: font.cell_w as u16,
        cell_height: font.cell_h as u16,
    }
}

/// Palette for the theme name, falling back to `default` for unknown names.
fn theme_named(config: &Config, name: &str) -> Theme {
    let themes = config.themes();
    themes.get(name).cloned().unwrap_or_else(|| {
        log::warn!("unknown theme {name:?}, using default");
        themes["default"].clone()
    })
}

/// Tell a program that asked (DECSET 1004) whether it has focus.
fn focus_report(pane: &mut Pane, focused: bool) {
    if pane.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
        pane.pty.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
    }
}

// Panes and tabs.
impl App {
    fn focused(&self) -> PaneId {
        self.tabs[self.active].focused
    }

    fn focused_pane(&mut self) -> &mut Pane {
        let id = self.focused();
        self.panes.get_mut(&id).expect("focused pane exists")
    }

    /// Pane the mouse acts on.
    fn mouse_target(&self) -> PaneId {
        self.mouse_pane.filter(|id| self.panes.contains_key(id)).unwrap_or_else(|| self.focused())
    }

    fn pane_at(&self, x: f32, y: f32) -> Option<PaneId> {
        self.tabs[self.active].root.leaves().into_iter().find(|id| self.panes[id].rect.contains(x, y))
    }

    fn divider_at(&self, x: f32, y: f32) -> Option<usize> {
        const GRAB: f32 = 3.0;
        self.dividers.iter().position(|d| {
            let r = d.rect;
            Rect { x: r.x - GRAB, y: r.y - GRAB, w: r.w + 2.0 * GRAB, h: r.h + 2.0 * GRAB }.contains(x, y)
        })
    }

    /// Height of the tab bar; it only shows with more than one tab.
    fn bar_h(&self) -> f32 {
        match &self.renderer {
            Some(r) if self.tabs.len() > 1 => (r.font.cell_h * 1.4).round(),
            _ => 0.0,
        }
    }

    fn tab_w(&self) -> f32 {
        let Some(r) = &self.renderer else { return 1.0 };
        (r.size().0 / self.tabs.len() as f32).min(r.font.cell_w * TAB_CELLS).floor()
    }

    /// Start a shell (or `command`) in a new pane, not yet placed in any tab.
    fn spawn_pane(&mut self, cwd: Option<PathBuf>, command: Option<Vec<String>>) -> Option<PaneId> {
        let id = self.next_pane;
        self.next_pane += 1;
        let renderer = self.renderer.as_ref().expect("renderer exists");
        let (w, h) = renderer.size();
        let size = window_size(&renderer.font, w, h);
        let listener = Listener { proxy: self.proxy.clone(), pane: id };
        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(self.config.term(), &dims, listener.clone())));
        let (shell, env) = match command {
            Some(cmd) => (cmd.split_first().map(|(program, args)| tty::Shell::new(program.clone(), args.to_vec())), HashMap::new()),
            None => shell::login(&self.config.terminal.shell),
        };
        let what = format!("{shell:?}");
        let options = tty::Options { shell, working_directory: cwd, env, ..Default::default() };
        let pty = Pty::spawn(term.clone(), size, listener, &options).map_err(|e| log::error!("can't start {what}: {e}")).ok()?;
        let pane = Pane {
            term,
            pty,
            theme: Fade::new(theme_named(&self.config, &self.config.theme)),
            theme_name: String::new(),
            context: Context::default(),
            title: None,
            last_context_check: Instant::now(),
            context_due: None,
            rect: Rect::default(),
            blocks: Blocks::default(),
            cwd: None,
            failed_at: None,
        };
        self.panes.insert(id, pane);
        Some(id)
    }

    /// New panes start where the focused shell is.
    fn new_cwd(&self) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active)?;
        self.panes.get(&tab.focused)?.cwd()
    }

    fn new_tab(&mut self) {
        let cwd = self.new_cwd();
        let Some(id) = self.spawn_pane(cwd, None) else { return };
        let old = self.tabs.get(self.active).map(|t| t.focused);
        self.tabs.push(Tab { root: Node::Leaf(id), focused: id });
        self.active = self.tabs.len() - 1;
        self.mouse_pane = None;
        self.relayout();
        self.focus_changed(old);
        self.check_context(id, true);
    }

    fn split(&mut self, across: bool, command: Option<Vec<String>>) {
        let cwd = self.new_cwd();
        let Some(id) = self.spawn_pane(cwd, command) else { return };
        let target = self.focused();
        self.tabs[self.active].root.split(target, id, across);
        self.relayout();
        self.focus(id);
        self.check_context(id, true);
    }

    /// Close a pane (its shell exited, or Ctrl+Shift+W). The last pane closes the
    /// tab, the last tab closes the window.
    fn close_pane(&mut self, event_loop: &ActiveEventLoop, id: PaneId) {
        let Some(pane) = self.panes.remove(&id) else { return };
        // Dropping hangs up the shell and waits for it; a stubborn one mustn't freeze the UI.
        std::thread::spawn(move || drop(pane));
        let Some(ti) = self.tabs.iter().position(|t| t.root.contains(id)) else { return };
        let old = self.focused();
        let tab = &mut self.tabs[ti];
        match tab.root.remove(id) {
            Some(next) if tab.focused == id => tab.focused = next,
            Some(_) => {}
            None => {
                self.tabs.remove(ti);
                if self.tabs.is_empty() {
                    return event_loop.exit();
                }
                if ti < self.active || self.active == self.tabs.len() {
                    self.active -= 1;
                }
            }
        }
        self.relayout();
        self.focus_changed(Some(old));
    }

    fn focus(&mut self, id: PaneId) {
        let old = self.focused();
        self.tabs[self.active].focused = id;
        self.focus_changed(Some(old));
    }

    fn switch_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active {
            return;
        }
        let old = self.focused();
        self.active = i;
        self.mouse_pane = None;
        self.relayout(); // for the new tab's dividers
        self.focus_changed(Some(old));
    }

    /// Keyboard focus moved away from `old` (which may be gone by now).
    fn focus_changed(&mut self, old: Option<PaneId>) {
        let new = self.focused();
        if self.search.as_ref().is_some_and(|s| s.pane != new) {
            self.search = None;
        }
        if old != Some(new) && self.renderer.as_ref().is_some_and(|r| r.focused) {
            if let Some(pane) = old.and_then(|id| self.panes.get_mut(&id)) {
                focus_report(pane, false);
            }
            focus_report(self.focused_pane(), true);
        }
        if let Some(r) = self.renderer.as_mut() {
            r.cursor_on = true;
        }
        self.next_blink = Instant::now() + BLINK;
        self.update_title();
        self.dirty = true;
    }

    fn update_title(&mut self) {
        let Some(pane) = self.tabs.get(self.active).and_then(|t| self.panes.get(&t.focused)) else { return };
        let title = match pane.title() {
            "" => "noobsbot terminal",
            t => t,
        };
        if title != self.window_title {
            if let Some(w) = &self.window {
                w.set_title(title);
            }
            self.window_title = title.to_owned();
        }
    }

    /// Lay out every tab and fit each pane's grid and PTY to its rect.
    fn relayout(&mut self) {
        let bar = self.bar_h();
        let Some(renderer) = &self.renderer else { return };
        let (w, h) = renderer.size();
        let area = Rect { x: 0.0, y: bar, w, h: h - bar };
        for (i, tab) in self.tabs.iter().enumerate() {
            let (mut rects, mut dividers) = (Vec::new(), Vec::new());
            tab.root.layout(area, &mut rects, &mut dividers);
            if i == self.active {
                self.dividers = dividers;
            }
            for (id, rect) in rects {
                let Some(pane) = self.panes.get_mut(&id) else { continue };
                pane.rect = rect;
                let size = window_size(&renderer.font, rect.w, rect.h);
                let (cols, lines) = (size.num_cols as usize, size.num_lines as usize);
                let mut term = pane.term.lock();
                if (term.columns(), term.screen_lines()) != (cols, lines) {
                    term.resize(TermSize::new(cols, lines));
                    drop(term);
                    pane.pty.resize(size);
                }
            }
        }
        self.dirty = true;
    }

    fn active_rects(&self) -> Vec<(PaneId, Rect)> {
        self.tabs[self.active].root.leaves().into_iter().map(|id| (id, self.panes[&id].rect)).collect()
    }
}

// Links and the mouse pointer.
impl App {
    /// Find the link under the pointer. Links are only live while Ctrl is held,
    /// so plain clicks still select.
    fn update_hover(&mut self) {
        let id = self.mouse_pane.filter(|id| self.panes.contains_key(id) && self.tabs[self.active].root.contains(*id));
        let hover = match id {
            Some(id) if self.mods.control_key() && !self.selecting && self.drag.is_none() => {
                let pane = &self.panes[&id];
                let cwd = pane.cwd();
                let term = pane.term.lock();
                let point = grid_point(&term, self.mouse_cell);
                self.hints.at(&term, point, cwd.as_deref()).map(|link| (id, link))
            }
            _ => None,
        };
        let key = |h: &Option<(PaneId, Link)>| h.as_ref().map(|(id, l)| (*id, l.range.clone()));
        if key(&hover) != key(&self.hover) {
            self.dirty = true;
        }
        self.hover = hover;
        self.update_icon();
    }

    fn update_icon(&mut self) {
        let (x, y) = self.mouse_px;
        let icon = match self.divider_at(x, y).map(|i| self.dividers[i].across) {
            Some(true) => CursorIcon::ColResize,
            Some(false) => CursorIcon::RowResize,
            None if self.hover.is_some() => CursorIcon::Pointer,
            None => CursorIcon::Default,
        };
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            if let Some(w) = &self.window {
                w.set_cursor(icon);
            }
        }
    }

    /// URLs go to the browser; files open in `$EDITOR` in a split next to the output.
    fn open_link(&mut self, target: Target) {
        match target {
            Target::Url(url) => links::xdg_open(&url),
            Target::File { path, line } => match links::editor_command(&path, line) {
                Some(cmd) => self.split(true, Some(cmd)),
                None => links::xdg_open(&path.to_string_lossy()),
            },
        }
        self.hover = None;
        self.update_icon();
        self.dirty = true;
    }
}

impl App {
    fn paste(&mut self, primary: bool) {
        let Some(text) = self.clipboard.get(primary) else { return };
        let pane = self.focused_pane();
        let mut term = pane.term.lock();
        let bracketed = term.mode().contains(TermMode::BRACKETED_PASTE);
        term.scroll_display(Scroll::Bottom);
        drop(term);
        pane.pty.write(&paste_bytes(&text, bracketed));
        self.dirty = true;
    }

    fn scroll(&mut self, scroll: Scroll) {
        self.focused_pane().term.lock().scroll_display(scroll);
        self.dirty = true;
    }

    /// Run a bound action. False means it didn't apply and the key goes to the shell.
    fn run(&mut self, event_loop: &ActiveEventLoop, action: Action) -> bool {
        match action {
            Action::ScrollPageUp => self.scroll(Scroll::PageUp),
            Action::ScrollPageDown => self.scroll(Scroll::PageDown),
            Action::ScrollTop => self.scroll(Scroll::Top),
            Action::ScrollBottom => self.scroll(Scroll::Bottom),
            Action::Copy => {
                let text = self.focused_pane().term.lock().selection_to_string();
                if let Some(text) = text {
                    self.clipboard.set(false, text);
                }
            }
            Action::Paste => self.paste(false),
            Action::ZoomIn => self.set_zoom(self.zoom + 1.0),
            Action::ZoomOut => self.set_zoom(self.zoom - 1.0),
            Action::ZoomReset => self.set_zoom(0.0),
            Action::NewTab => self.new_tab(),
            Action::ClosePane => self.close_pane(event_loop, self.focused()),
            Action::NextTab => self.switch_tab((self.active + 1) % self.tabs.len()),
            Action::PrevTab => self.switch_tab((self.active + self.tabs.len() - 1) % self.tabs.len()),
            // With a single tab, Alt+digit stays readline's digit argument.
            Action::GoToTab(i) if i < self.tabs.len() && self.tabs.len() > 1 => self.switch_tab(i),
            Action::GoToTab(_) => return false,
            Action::SplitRight => self.split(true, None),
            Action::SplitDown => self.split(false, None),
            Action::Focus(dir) => match pane::neighbor(&self.active_rects(), self.focused(), dir) {
                Some(id) => self.focus(id),
                None => return false,
            },
            Action::Search => self.open_search(),
            Action::ViMode => {
                if self.focused_pane().term.lock().mode().contains(TermMode::VI) {
                    self.vi(Vi::Exit);
                } else {
                    self.focused_pane().term.lock().toggle_vi_mode();
                    self.dirty = true;
                }
            }
            Action::ToggleCrt => {
                self.crt = !self.crt;
                self.dirty = true;
            }
            Action::CommandPalette => {
                self.palette = Some(Palette::new(self.config.themes().into_keys().collect()));
                self.dirty = true;
            }
            Action::PrevPrompt => return self.jump(true),
            Action::NextPrompt => return self.jump(false),
            Action::CopyOutput => {
                let text = blocks::last_output(&self.focused_pane().term.lock());
                if let Some(text) = text.filter(|t| !t.is_empty()) {
                    self.clipboard.set(false, text);
                }
            }
            Action::Resize(dir) => {
                let delta = if matches!(dir, Dir::Left | Dir::Up) { -RESIZE_STEP } else { RESIZE_STEP };
                let id = self.focused();
                if !self.tabs[self.active].root.resize(id, dir.across(), delta) {
                    return false;
                }
                self.relayout();
            }
        }
        true
    }
}

// Shell integration.
impl App {
    fn shell_mark(&mut self, id: PaneId, prompt: u64, mark: Mark) {
        let window_focused = self.renderer.as_ref().is_some_and(|r| r.focused);
        let Some(pane) = self.panes.get_mut(&id) else { return };
        self.dirty = true;
        if let Mark::Cwd(path) = mark {
            pane.cwd = Some(path);
            return;
        }
        let Some((code, took)) = pane.blocks.mark(prompt, &mark) else { return };
        if code.is_some_and(|c| c != 0) {
            pane.failed_at = Some(Instant::now());
        }
        if took >= NOTIFY_AFTER && !window_focused {
            let cmd = blocks::command_text(&pane.term.lock(), prompt).unwrap_or_else(|| "Command".into());
            let status = match code {
                Some(0) => "done".to_owned(),
                Some(c) => format!("failed ({c})"),
                None => "finished".to_owned(),
            };
            notify(format!("{cmd} {status}"), format!("after {}", human(took)));
        }
    }

    /// Scroll so the previous (`up`) or next prompt is at the top.
    fn jump(&mut self, up: bool) -> bool {
        let mut term = self.focused_pane().term.lock();
        let offset = term.grid().display_offset() as i32;
        let top = Line(-offset);
        let prompts = blocks::prompts(&term);
        let target = if up { prompts.iter().rev().find(|(l, _)| *l < top) } else { prompts.iter().find(|(l, _)| *l > top) };
        let Some(&(line, _)) = target else { return false };
        let wanted = (-line.0).clamp(0, term.history_size() as i32);
        term.scroll_display(Scroll::Delta(wanted - offset));
        drop(term);
        self.dirty = true;
        true
    }
}

/// Desktop notification, without waiting for it.
fn notify(summary: String, body: String) {
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("notify-send").args(["-a", "noobsbot", &summary, &body]).status() {
            log::warn!("notify-send: {e}");
        }
    });
}

/// 75 s → "1m 15s".
fn human(d: Duration) -> String {
    match d.as_secs() {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m {}s", s / 60, s % 60),
        s => format!("{}h {}m", s / 3600, s / 60 % 60),
    }
}

// Command palette.
impl App {
    /// A key while the palette is open; it takes them all.
    fn palette_key(&mut self, event_loop: &ActiveEventLoop, key: &Key, text: Option<&str>) -> bool {
        let Some(p) = self.palette.as_mut() else { return false };
        self.dirty = true;
        match key {
            Key::Named(NamedKey::Escape) => self.palette = None,
            Key::Named(NamedKey::Enter) => {
                let item = p.chosen().cloned();
                self.palette = None;
                match item {
                    Some(Item::Action(action)) => {
                        self.run(event_loop, action);
                    }
                    Some(Item::Theme(name)) => self.set_theme(name),
                    None => {}
                }
            }
            Key::Named(NamedKey::ArrowUp) => p.selected = p.selected.checked_sub(1).unwrap_or(p.len().saturating_sub(1)),
            Key::Named(NamedKey::ArrowDown) => p.selected = if p.selected + 1 < p.len() { p.selected + 1 } else { 0 },
            Key::Named(NamedKey::Backspace) => {
                p.query.pop();
                p.filter();
            }
            _ => {
                if let Some(t) = text.filter(|t| !self.mods.control_key() && !self.mods.alt_key() && !t.chars().any(char::is_control)) {
                    p.query.push_str(t);
                    p.filter();
                }
            }
        }
        true
    }

    /// Theme picked by hand for the focused pane; holds until the situation changes.
    fn set_theme(&mut self, name: String) {
        let theme = theme_named(&self.config, &name);
        let pane = self.focused_pane();
        pane.theme.set(theme);
        pane.theme_name = name;
    }
}

// Search and vi mode.
impl App {
    fn open_search(&mut self) {
        let id = self.focused();
        match &mut self.search {
            Some(s) if s.pane == id => s.editing = true,
            _ => self.search = Some(Search::new(id, &self.panes[&id].term.lock())),
        }
        self.dirty = true;
    }

    /// A key while the search bar is open. False: not ours (Ctrl combos), carry on.
    fn search_key(&mut self, key: &Key, text: Option<&str>) -> bool {
        let Some(search) = self.search.as_mut().filter(|s| s.editing) else { return false };
        let Some(pane) = self.panes.get_mut(&search.pane) else { return false };
        let mut term = pane.term.lock();
        let vi = term.mode().contains(TermMode::VI);
        match key {
            Key::Named(NamedKey::Escape) if vi => search.editing = false,
            Key::Named(NamedKey::Escape) => {
                drop(term);
                self.search = None;
            }
            Key::Named(NamedKey::Enter) if vi => search.editing = false,
            Key::Named(NamedKey::Enter) => {
                let direction = if self.mods.shift_key() { Direction::Right } else { Direction::Left };
                search.step(&mut term, direction);
            }
            Key::Named(NamedKey::Backspace) => {
                search.query.pop();
                search.update(&mut term);
            }
            _ => match text {
                Some(t) if !self.mods.control_key() && !self.mods.alt_key() && !t.chars().any(char::is_control) => {
                    search.query.push_str(t);
                    search.update(&mut term);
                }
                _ if self.mods.control_key() || self.mods.alt_key() => return false,
                _ => {}
            },
        }
        self.dirty = true;
        true
    }

    /// A key in vi mode (focused pane).
    fn vi(&mut self, cmd: Vi) {
        let id = self.focused();
        let pane = self.panes.get_mut(&id).expect("focused pane exists");
        let mut term = pane.term.lock();
        let point = term.vi_mode_cursor.point;
        let mut exit = false;
        match cmd {
            Vi::Motion(motion) => term.vi_motion(motion),
            Vi::Top | Vi::Bottom => {
                let line = if cmd == Vi::Top { term.topmost_line() } else { term.bottommost_line() };
                term.vi_goto_point(Point::new(line, Column(0)));
            }
            Vi::HalfUp | Vi::HalfDown => {
                let half = term.screen_lines() as i32 / 2;
                let line = if cmd == Vi::HalfUp { point.line - half } else { point.line + half };
                let line = line.clamp(term.topmost_line(), term.bottommost_line());
                term.vi_goto_point(Point::new(line, point.column));
            }
            Vi::Select(ty) => match &term.selection {
                Some(sel) if sel.ty == ty => term.selection = None,
                _ => {
                    let mut sel = Selection::new(ty, point, Side::Left);
                    sel.include_all(); // non-empty, so motions extend it
                    term.selection = Some(sel);
                }
            },
            Vi::Copy => {
                if let Some(text) = term.selection_to_string() {
                    self.clipboard.set(false, text);
                }
                exit = true;
            }
            Vi::Escape if term.selection.is_some() => term.selection = None,
            Vi::Escape | Vi::Exit => exit = true,
            Vi::Search => {
                drop(term);
                return self.open_search();
            }
            Vi::SearchPrev | Vi::SearchNext => {
                if let Some(search) = self.search.as_mut().filter(|s| s.pane == id) {
                    search.step(&mut term, if cmd == Vi::SearchPrev { Direction::Left } else { Direction::Right });
                }
            }
        }
        if exit {
            term.selection = None;
            term.toggle_vi_mode();
            term.scroll_display(Scroll::Bottom);
            drop(term);
            self.search = None;
        } else {
            let point = term.vi_mode_cursor.point;
            term.scroll_to_point(point);
        }
        self.dirty = true;
    }
}

// Quake mode.
impl App {
    /// Hotkey or second launch: hide if shown and focused, else show and focus.
    fn toggle_quake(&mut self) {
        let (Some(q), Some(window)) = (self.quake.as_mut(), &self.window) else { return };
        let focused = crate::quake::has_focus(window).unwrap_or_else(|| self.renderer.as_ref().is_some_and(|r| r.focused));
        let now = Instant::now();
        // Held keys auto-repeat; one press, one toggle.
        if now - q.last_toggle < Duration::from_millis(250) {
            return;
        }
        q.last_toggle = now;
        if q.visible && !focused {
            return crate::quake::activate(window);
        }
        q.visible = !q.visible;
        q.fade = Some((now, q.visible));
        if q.visible {
            crate::quake::set_opacity(window, 0.0);
            window.set_visible(true);
        }
    }

    /// Step the quake window's fade. Returns when to step again.
    // ponytail: a slide would be nicer, but the window manager clamps managed windows on screen.
    fn fade_quake(&mut self, now: Instant) -> Option<Instant> {
        let (Some(q), Some(window)) = (self.quake.as_mut(), &self.window) else { return None };
        let (start, showing) = q.fade?;
        let t = ((now - start).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
        crate::quake::set_opacity(window, if showing { t } else { 1.0 - t });
        if t < 1.0 {
            return Some(now + self.frame_time);
        }
        q.fade = None;
        if showing {
            crate::quake::activate(window); // now that it's mapped
        } else {
            window.set_visible(false);
        }
        None
    }
}

impl App {
    /// Font size in physical pixels: config size + zoom, scaled for HiDPI.
    fn font_px(&self, window: &Window) -> f32 {
        (self.config.font.size + self.zoom).max(6.0) * window.scale_factor() as f32
    }

    /// Re-apply font family/size to the renderer (after zoom or config change).
    fn apply_font(&mut self) {
        let Some(px) = self.window.as_ref().map(|w| self.font_px(w)) else { return };
        let Some(renderer) = self.renderer.as_mut() else { return };
        renderer.set_font(&self.config.font.family, px);
        self.relayout();
    }

    fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
        self.apply_font();
    }

    /// Re-detect a pane's situation and switch its theme if a different rule now matches.
    fn check_context(&mut self, id: PaneId, force: bool) {
        let Some(pane) = self.panes.get_mut(&id) else { return };
        let ctx = context::detect(pane.pty.fd(), pane.pty.shell_pid());
        pane.last_context_check = Instant::now();
        pane.context_due = self
            .config
            .rules
            .iter()
            .any(|r| r.when.uses_hours())
            .then(|| pane.last_context_check + CONTEXT_CLOCK);
        if ctx == pane.context && !force {
            return;
        }
        let rule = crate::rules::pick(&self.config.rules, &ctx);
        let name = rule.map_or(self.config.theme.clone(), |r| r.theme.clone());
        if name != pane.theme_name || force {
            log::info!("pane {id}: theme → {name} ({})", if rule.is_some() { format!("{ctx:?}") } else { "no rule matched".into() });
            pane.theme.set(theme_named(&self.config, &name));
            pane.theme_name = name;
        }
        pane.context = ctx; // the foreground program is also the tab title
        self.dirty = true;
        self.update_title();
    }

    /// Output arrived: the foreground program may have changed. Check now or soon.
    fn context_poke(&mut self, id: PaneId) {
        let Some(pane) = self.panes.get_mut(&id) else { return };
        let due = pane.last_context_check + CONTEXT_THROTTLE;
        if Instant::now() >= due {
            self.check_context(id, false);
        } else if pane.context_due.is_none_or(|d| d > due) {
            pane.context_due = Some(due);
        }
    }

    fn reload_config(&mut self) {
        let config = match config::load(&config::path()) {
            Ok(c) => c,
            Err(e) => return log::error!("config error, keeping previous config: {e}"),
        };
        log::info!("config reloaded");
        let font_changed = config.font != self.config.font;
        if config.crt.enabled != self.config.crt.enabled {
            self.crt = config.crt.enabled;
        }
        self.config = config;
        let ids: Vec<PaneId> = self.panes.keys().copied().collect();
        for id in ids {
            self.panes[&id].term.lock().set_options(self.config.term());
            self.check_context(id, true);
        }
        if font_changed {
            self.apply_font();
        }
        self.dirty = true;
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes().with_title("noobsbot terminal");
        let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
        if let (Some(_), Some(m)) = (&self.quake, monitor) {
            // Borderless, full width, on top; hidden until it fades in.
            let size = m.size();
            let height = size.height as f32 * self.config.quake.height.clamp(10, 100) as f32 / 100.0;
            attrs = attrs
                .with_decorations(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_inner_size(PhysicalSize::new(size.width, height as u32))
                .with_position(m.position())
                .with_visible(false);
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        if let Some(mhz) = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()) {
            self.frame_time = Duration::from_secs_f64(1000.0 / mhz as f64);
        }
        let font = Font::new(&self.config.font.family, self.font_px(&window));
        self.renderer = Some(Renderer::new(window.clone(), font));
        self.window = Some(window);
        self.new_tab();
        if self.tabs.is_empty() {
            event_loop.exit(); // no shell to run
        }
        self.toggle_quake();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let (id, event) = match event {
            UserEvent::ConfigChanged => return self.reload_config(),
            UserEvent::ToggleQuake => return self.toggle_quake(),
            UserEvent::Shell(id, prompt, mark) => return self.shell_mark(id, prompt, mark),
            UserEvent::Term(id, event) => (id, event),
        };
        if matches!(event, Event::Exit | Event::ChildExit(_)) {
            return self.close_pane(event_loop, id);
        }
        let Some(pane) = self.panes.get_mut(&id) else { return };
        match event {
            Event::PtyWrite(text) => pane.pty.write(text.as_bytes()),
            Event::Title(title) => {
                pane.title = Some(title);
                self.update_title();
                self.dirty = true;
            }
            Event::ResetTitle => {
                pane.title = None;
                self.update_title();
                self.dirty = true;
            }
            Event::Wakeup => {
                if std::env::var_os("NOOBSBOT_DUMP").is_some() {
                    dump_grid(&pane.term.lock());
                }
                // Drawn from about_to_wait at most once per refresh, so a flood of output
                // doesn't make the renderer hog the terminal lock.
                self.dirty |= self.tabs.get(self.active).is_some_and(|t| t.root.contains(id));
                self.context_poke(id);
            }
            Event::ClipboardStore(ty, text) => self.clipboard.set(ty == ClipboardType::Selection, text),
            Event::ClipboardLoad(ty, format) => {
                // OSC 52 read; only reaches us if `terminal.clipboard` allows pasting.
                let text = self.clipboard.get(ty == ClipboardType::Selection).unwrap_or_default();
                pane.pty.write(format(&text).as_bytes());
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.tabs.is_empty() {
            return;
        }
        let now = Instant::now();
        // First, so a theme switch it causes is drawn below.
        let due: Vec<PaneId> = self.panes.iter().filter(|(_, p)| p.context_due.is_some_and(|d| now >= d)).map(|(id, _)| *id).collect();
        for id in due {
            self.check_context(id, false);
        }
        let blinking = self.focused_pane().term.lock().cursor_style().blinking;
        let mut wake = self.fade_quake(now);
        let (Some(renderer), Some(window)) = (self.renderer.as_mut(), &self.window) else { return };

        if renderer.focused && blinking {
            if now >= self.next_blink {
                renderer.cursor_on = !renderer.cursor_on;
                self.next_blink = now + BLINK;
                self.dirty = true;
            }
            wake = Some(self.next_blink);
        } else if !renderer.cursor_on {
            renderer.cursor_on = true;
            self.dirty = true;
        }
        let flicker = self.crt && self.config.crt.flicker > 0.0;
        if flicker || self.panes.values().any(|p| p.theme.fading() || p.failed_at.is_some()) {
            self.dirty = true;
        }

        if self.dirty {
            let due = self.last_frame + self.frame_time;
            if now >= due {
                self.dirty = false;
                window.request_redraw();
            } else {
                wake = Some(wake.map_or(due, |w: Instant| w.min(due)));
            }
        }
        for due in self.panes.values().filter_map(|p| p.context_due) {
            wake = Some(wake.map_or(due, |w: Instant| w.min(due)));
        }
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.tabs.is_empty() || self.renderer.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(px) => {
                self.renderer.as_mut().unwrap().resize(px.width, px.height);
                self.relayout();
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                self.update_hover();
            }
            WindowEvent::Focused(focused) => {
                self.renderer.as_mut().unwrap().focused = focused;
                focus_report(self.focused_pane(), focused);
                self.dirty = true;
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.palette_key(event_loop, &event.logical_key, event.text.as_deref()) {
                    return;
                }
                if self.search_key(&event.logical_key, event.text.as_deref()) {
                    return;
                }
                if let Some(action) = binding(&event.logical_key, self.mods)
                    && self.run(event_loop, action)
                {
                    return;
                }
                if self.focused_pane().term.lock().mode().contains(TermMode::VI) {
                    // Unbound keys do nothing in vi mode rather than reach the shell.
                    if let Some(cmd) = vi_binding(&event.logical_key, self.mods) {
                        self.vi(cmd);
                    }
                    return;
                }
                let mods = self.mods;
                let pane = self.panes.get_mut(&self.tabs[self.active].focused).unwrap();
                let mut term = pane.term.lock();
                let app_cursor = term.mode().contains(TermMode::APP_CURSOR);
                let Some(bytes) = key_bytes(&event.logical_key, event.text.as_deref(), mods, app_cursor) else { return };
                if term.grid().display_offset() != 0 {
                    term.scroll_display(Scroll::Bottom);
                    self.dirty = true;
                }
                drop(term);
                pane.pty.write(&bytes);
                // Keep the cursor solid while typing.
                self.renderer.as_mut().unwrap().cursor_on = true;
                self.next_blink = Instant::now() + BLINK;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let cell_h = self.renderer.as_ref().unwrap().font.cell_h as f64;
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px += p.y;
                        let lines = (self.scroll_px / cell_h).trunc();
                        self.scroll_px -= lines * cell_h;
                        lines as i32
                    }
                };
                if lines == 0 {
                    return;
                }
                let (mods, (col, line)) = (self.mods, self.mouse_cell);
                let pane = self.panes.get_mut(&self.mouse_target()).unwrap();
                let mut term = pane.term.lock();
                let mode = *term.mode();
                if mode.intersects(TermMode::MOUSE_MODE) {
                    let button = if lines > 0 { input::MOUSE_WHEEL_UP } else { input::MOUSE_WHEEL_DOWN };
                    drop(term);
                    if let Some(bytes) = mouse_report(button, true, col, line, mods, mode.contains(TermMode::SGR_MOUSE)) {
                        for _ in 0..lines.abs() {
                            pane.pty.write(&bytes);
                        }
                    }
                } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    // Full-screen apps without mouse support (less, man) get arrow keys instead.
                    let arrow: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                        (true, true) => b"\x1bOA",
                        (true, false) => b"\x1b[A",
                        (false, true) => b"\x1bOB",
                        (false, false) => b"\x1b[B",
                    };
                    drop(term);
                    pane.pty.write(&arrow.repeat(lines.unsigned_abs() as usize));
                } else {
                    term.scroll_display(Scroll::Delta(lines));
                    self.dirty = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let mut pos = (position.x as f32, position.y as f32);
                if self.crt {
                    pos = crt::warp(self.config.crt.params().curvature, self.renderer.as_ref().unwrap().size(), pos);
                }
                let (x, y) = pos;
                self.mouse_px = (x, y);
                if let Some(i) = self.drag {
                    let d = &self.dividers[i];
                    let ratio = if d.across { (x - d.area.x) / d.area.w } else { (y - d.area.y) / d.area.h };
                    let path = d.path.clone();
                    self.tabs[self.active].root.set_ratio(&path, ratio);
                    return self.relayout();
                }
                self.update_icon();

                let id = if self.selecting || self.mouse_held.is_some() {
                    self.mouse_target()
                } else {
                    match self.pane_at(x, y) {
                        Some(id) => id,
                        None => return,
                    }
                };
                let same_pane = self.mouse_pane == Some(id);
                self.mouse_pane = Some(id);
                let font = &self.renderer.as_ref().unwrap().font;
                let (cw, ch) = (font.cell_w, font.cell_h);
                let pane = &self.panes[&id];
                let r = pane.rect;
                let (cols, lines) = {
                    let term = pane.term.lock();
                    (term.columns(), term.screen_lines())
                };
                let x = (x - r.x).max(0.0);
                let col = ((x / cw) as usize).min(cols - 1);
                let line = (((y - r.y).max(0.0) / ch) as usize).min(lines - 1);
                let side = if x - col as f32 * cw > cw / 2.0 { Side::Right } else { Side::Left };
                if same_pane && (col, line, side) == (self.mouse_cell.0, self.mouse_cell.1, self.mouse_side) {
                    return;
                }
                self.mouse_side = side;
                self.mouse_cell = (col, line);
                self.update_hover();
                let pane = self.panes.get_mut(&id).unwrap();
                let mut term = pane.term.lock();
                if self.selecting {
                    // ponytail: no auto-scroll when dragging past the pane edge yet.
                    let point = grid_point(&term, self.mouse_cell);
                    if let Some(sel) = term.selection.as_mut() {
                        sel.update(point, side);
                        self.dirty = true;
                    }
                }
                let mode = *term.mode();
                drop(term);
                let report = match self.mouse_held {
                    Some(b) if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) => Some(b),
                    None if mode.contains(TermMode::MOUSE_MOTION) => Some(input::MOUSE_RELEASE),
                    _ => None,
                };
                if let Some(button) = report {
                    let sgr = mode.contains(TermMode::SGR_MOUSE);
                    if let Some(bytes) = mouse_report(button + input::MOUSE_MOTION, true, col, line, self.mods, sgr) {
                        pane.pty.write(&bytes);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                let (x, y) = self.mouse_px;
                if !pressed && button == MouseButton::Left && self.drag.take().is_some() {
                    return;
                }
                if pressed && y < self.bar_h() {
                    if button == MouseButton::Left {
                        self.switch_tab((x / self.tab_w()) as usize);
                    }
                    return;
                }
                if pressed && button == MouseButton::Left
                    && let Some(i) = self.divider_at(x, y)
                {
                    self.drag = Some(i);
                    return;
                }
                let id = self.mouse_target();
                if pressed && id != self.focused() {
                    self.focus(id);
                }
                if pressed && button == MouseButton::Left
                    && let Some((_, link)) = self.hover.take()
                {
                    return self.open_link(link.target);
                }
                let pane = self.panes.get_mut(&id).unwrap();
                let mode = *pane.term.lock().mode();
                // Shift bypasses mouse reporting so you can still select text in vim/tmux.
                if !mode.intersects(TermMode::MOUSE_MODE) || self.mods.shift_key() {
                    match (button, pressed) {
                        (MouseButton::Left, true) => {
                            let now = Instant::now();
                            self.clicks = if now - self.last_click < MULTI_CLICK { self.clicks % 3 + 1 } else { 1 };
                            self.last_click = now;
                            let ty = match self.clicks {
                                1 => SelectionType::Simple,
                                2 => SelectionType::Semantic,
                                _ => SelectionType::Lines,
                            };
                            let mut term = pane.term.lock();
                            let point = grid_point(&term, self.mouse_cell);
                            term.selection = Some(Selection::new(ty, point, self.mouse_side));
                            self.selecting = true;
                            self.dirty = true;
                        }
                        (MouseButton::Left, false) if self.selecting => {
                            self.selecting = false;
                            let mut term = pane.term.lock();
                            match term.selection_to_string().filter(|s| !s.is_empty()) {
                                Some(text) => self.clipboard.set(true, text),
                                None => term.selection = None,
                            }
                            self.dirty = true;
                        }
                        (MouseButton::Middle, true) => self.paste(true),
                        _ => {}
                    }
                    return;
                }
                let code = match button {
                    MouseButton::Left => input::MOUSE_LEFT,
                    MouseButton::Middle => input::MOUSE_MIDDLE,
                    MouseButton::Right => input::MOUSE_RIGHT,
                    _ => return,
                };
                if !pressed && self.mouse_held.is_none() {
                    return; // its press wasn't reported (it opened a link)
                }
                self.mouse_held = pressed.then_some(code);
                let (col, line) = self.mouse_cell;
                if let Some(bytes) = mouse_report(code, pressed, col, line, self.mods, mode.contains(TermMode::SGR_MOUSE)) {
                    pane.pty.write(&bytes);
                }
            }
            WindowEvent::RedrawRequested => {
                self.last_frame = Instant::now();
                self.update_hover(); // the text under the pointer may have changed
                let (bar_h, tab_w) = (self.bar_h(), self.tab_w());
                let renderer = self.renderer.as_mut().unwrap();
                let tab = &self.tabs[self.active];
                let mut focused_theme = None;
                for id in tab.root.leaves() {
                    let pane = self.panes.get_mut(&id).unwrap();
                    let theme = pane.theme.frame();
                    let term = pane.term.lock();
                    let search = self.search.as_mut().filter(|s| s.pane == id);
                    let (matches, current) = match search {
                        Some(s) => (s.visible(&term), s.current.clone()),
                        None => (Vec::new(), None),
                    };
                    let link = self.hover.as_ref().filter(|(p, _)| *p == id).map(|(_, l)| &l.range);
                    renderer.pane(&term, &theme, pane.rect, id == tab.focused, &matches, current.as_ref(), link);
                    let vi = term.mode().contains(TermMode::VI);
                    let gutter = gutter(&term, &pane.blocks, &theme);
                    drop(term);
                    let flash = pane.failed_at.map(|t| 1.0 - t.elapsed().as_secs_f32() / FLASH.as_secs_f32()).filter(|&a| a > 0.0);
                    if flash.is_none() {
                        pane.failed_at = None;
                    }
                    let ui = PaneUi { vi, search: self.search.as_ref().filter(|s| s.pane == id && s.editing), gutter, flash };
                    draw_pane_ui(renderer, pane.rect, &theme, ui);
                    if id == tab.focused {
                        focused_theme = Some(theme);
                    }
                }
                let theme = focused_theme.expect("focused pane is in its tab");
                for d in &self.dividers {
                    renderer.fill(d.rect, theme::to_f32(theme.selection));
                }
                if self.tabs.len() > 1 {
                    draw_tab_bar(renderer, &self.tabs, &mut self.panes, self.active, bar_h, tab_w);
                }
                if let Some(p) = &self.palette {
                    draw_palette(renderer, p, &theme);
                }
                renderer.crt_params = self.crt.then(|| self.config.crt.params());
                renderer.finish(theme.bg);
            }
            _ => {}
        }
    }
}

/// Colored marks for the prompts on screen: (viewport line, color) for
/// commands that ran or are running at that prompt.
fn gutter(term: &Term, blocks: &Blocks, theme: &Theme) -> Vec<(usize, [f32; 4])> {
    let offset = term.grid().display_offset() as i32;
    (0..term.screen_lines())
        .filter_map(|i| {
            let cmd = blocks.get(blocks::prompt_at(term, Line(i as i32 - offset))?)?;
            let color = match cmd.done {
                None => theme.ansi[3],
                Some((Some(0), _)) => theme.ansi[2],
                Some(_) => theme.ansi[1],
            };
            Some((i, theme::to_f32(color)))
        })
        .collect()
}

struct PaneUi<'a> {
    vi: bool,
    search: Option<&'a Search>,
    gutter: Vec<(usize, [f32; 4])>,
    /// Failed-command border, fading from 1 to 0.
    flash: Option<f32>,
}

/// Prompt marks (left edge), failure flash (border), vi mode badge (top right)
/// and the search bar (bottom row) over a pane.
fn draw_pane_ui(r: &mut Renderer, rect: Rect, theme: &Theme, ui: PaneUi) {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let bar = (cw / 4.0).round().max(2.0);
    for (line, color) in ui.gutter {
        r.fill(Rect { x: rect.x, y: rect.y + line as f32 * ch, w: bar, h: ch }, color);
    }
    if let Some(a) = ui.flash {
        let [red, g, b, _] = theme::to_f32(theme.ansi[1]);
        let c = [red, g, b, a];
        let w = bar;
        r.fill(Rect { x: rect.x, y: rect.y, w: rect.w, h: w }, c);
        r.fill(Rect { x: rect.x, y: rect.y + rect.h - w, w: rect.w, h: w }, c);
        r.fill(Rect { x: rect.x, y: rect.y, w, h: rect.h }, c);
        r.fill(Rect { x: rect.x + rect.w - w, y: rect.y, w, h: rect.h }, c);
    }
    let (vi, search) = (ui.vi, ui.search);
    if vi {
        let w = cw * 4.0;
        r.fill(Rect { x: rect.x + rect.w - w, y: rect.y, w, h: ch }, theme::to_f32(theme.cursor));
        r.text(rect.x + rect.w - w + cw / 2.0, rect.y, " VI", theme::to_f32(theme.bg));
    }
    if let Some(s) = search {
        let y = rect.y + rect.h - ch;
        r.fill(Rect { x: rect.x, y, w: rect.w, h: ch }, theme::to_f32(theme.selection));
        let status = if !s.query.is_empty() && s.current.is_none() { "   no match" } else { "" };
        let fit = (rect.w / cw) as usize;
        let line: String = format!("Find: {}_{status}", s.query).chars().take(fit).collect();
        r.text(rect.x, y, &line, theme::to_f32(theme.fg));
    }
}

/// The palette: a query line and the best matches, centered near the top.
fn draw_palette(r: &mut Renderer, p: &Palette, theme: &Theme) {
    const ROWS: usize = 12;
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let w = r.size().0;
    let cols = ((w / cw) as usize).saturating_sub(4).min(64);
    let (bw, pad) = (cols as f32 * cw, (cw / 2.0).round());
    let (x, y) = (((w - bw) / 2.0).round(), ch);
    let n = p.len().min(ROWS);
    let (fg, bg) = (theme::to_f32(theme.fg), theme::to_f32(theme.bg));
    r.fill(Rect { x: x - pad, y: y - pad, w: bw + 2.0 * pad, h: (n + 1) as f32 * ch + 2.0 * pad }, theme::to_f32(theme.selection));
    let query = format!("> {}_", p.query);
    let skip = query.chars().count().saturating_sub(cols); // keep the end of a long query in view
    r.text(x, y, &query.chars().skip(skip).collect::<String>(), fg);
    let first = p.selected.saturating_sub(ROWS - 1);
    for (i, row) in p.shown().skip(first).take(ROWS).enumerate() {
        let ry = y + (i + 1) as f32 * ch;
        let (name_fg, keys_fg) = if first + i == p.selected {
            r.fill(Rect { x: x - pad, y: ry, w: bw + 2.0 * pad, h: ch }, theme::to_f32(theme.cursor));
            (bg, bg)
        } else {
            (fg, theme::to_f32(theme::dim(theme.fg)))
        };
        let keys = row.keys.chars().count().min(cols);
        r.text(x, ry, &row.name.chars().take(cols.saturating_sub(keys + 2)).collect::<String>(), name_fg);
        r.text(x + (cols - keys) as f32 * cw, ry, &row.keys.chars().take(keys).collect::<String>(), keys_fg);
    }
}

/// One tab per slot, each tinted with its focused pane's situation theme, so a
/// red production tab stays visible while you work in another one.
fn draw_tab_bar(r: &mut Renderer, tabs: &[Tab], panes: &mut HashMap<PaneId, Pane>, active: usize, bar_h: f32, tab_w: f32) {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let backdrop = theme::dim(theme::dim(panes.get_mut(&tabs[active].focused).unwrap().theme.frame().bg));
    r.fill(Rect { x: 0.0, y: 0.0, w: r.size().0, h: bar_h }, theme::to_f32(backdrop));
    for (i, tab) in tabs.iter().enumerate() {
        let pane = panes.get_mut(&tab.focused).unwrap();
        let theme = pane.theme.frame();
        let x = i as f32 * tab_w;
        let (bg, fg) = if i == active { (theme.bg, theme.fg) } else { (theme::dim(theme.bg), theme::dim(theme.fg)) };
        r.fill(Rect { x, y: 0.0, w: tab_w - 1.0, h: bar_h }, theme::to_f32(bg));
        r.fill(Rect { x, y: 0.0, w: tab_w - 1.0, h: 2.0 }, theme::to_f32(theme.cursor));
        let fit = ((tab_w - cw) / cw).max(0.0) as usize;
        let label: String = format!("{} {}", i + 1, pane.title()).chars().take(fit).collect();
        r.text(x + (cw / 2.0).round(), ((bar_h - ch) / 2.0).round() + 1.0, &label, theme::to_f32(fg));
    }
}

/// Debug aid: print the visible grid to stdout. Enabled with NOOBSBOT_DUMP=1.
fn dump_grid(term: &Term) {
    let grid = term.grid();
    println!("----- {}x{} -----", grid.columns(), grid.screen_lines());
    for line in 0..grid.screen_lines() {
        let row: String = grid[Line(line as i32)][..].iter().map(|cell| cell.c).collect();
        println!("{}", row.trim_end());
    }
}
