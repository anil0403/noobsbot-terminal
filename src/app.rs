use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{ClipboardType, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId, WindowLevel};

use crate::blocks::{self, Blocks};
use crate::clipboard::Clipboard;
use crate::config::{self, Config};
use crate::context::{self, Context};
use crate::font::Font;
use crate::images::Images;
use crate::input::{self, Action, Vi, binding, key_bytes, mouse_report, paste_bytes, vi_binding};
use crate::links::{self, Hints, Link, Target};
use crate::palette::{Item, Palette};
use crate::pane::{self, Dir, Divider, Node, Pane, Rect, Tab};
use crate::pty::Pty;
use crate::render::{Renderer, crt};
use crate::search::Search;
use crate::shell;
use crate::tap::Mark;
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
/// Buttons at the right end of the tab bar: label, action.
const BAR_BUTTONS: [Action; 3] = [Action::NewTab, Action::SplitRight, Action::SplitDown];
/// Space around the tab bar's tabs and buttons, in pixels.
const PAD: f32 = 2.0;
/// Margin around the window's contents, in the background color.
const BORDER: f32 = 4.0;
/// One button: a 1-cell label with two cells of padding each side.
const BUTTON_CELLS: f32 = 5.0;
/// The right-click menu, top to bottom.
const MENU: [Action; 8] =
    [Action::Copy, Action::Paste, Action::SplitRight, Action::SplitDown, Action::SplitLeft, Action::SplitUp, Action::NewTab, Action::ClosePane];
/// The menu's groups end after these rows; a rule separates them.
const MENU_BREAKS: [usize; 2] = [1, 5];
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
    /// Shell for new panes: the configured one until picked in the palette.
    shell: String,
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
    /// Scrollback search in the focused pane.
    search: Option<Search>,
    hints: Hints,
    palette: Option<Palette>,
    /// The right-click menu: top-left corner, and the row under the pointer.
    menu: Option<((f32, f32), Option<usize>)>,
    /// Link under the pointer while Ctrl is held.
    hover: Option<(PaneId, Link)>,
    /// Tab, close button or bar button under the pointer.
    bar_hover: Option<BarHit>,
    /// Split divider under the pointer (or being dragged).
    divider_hover: Option<usize>,
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
        let quake = quake.then(|| Quake { visible: false, fade: None, last_toggle: Instant::now() - Duration::from_secs(1) });
        Self {
            crt: config.crt.enabled,
            shell: config.terminal.shell.clone(),
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
            search: None,
            hints: Hints::new(),
            palette: None,
            menu: None,
            hover: None,
            bar_hover: None,
            divider_hover: None,
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

/// Grid point at a viewport cell (accounts for scrollback offset). Clamped:
/// the cell may be from before the pane shrank (split, resize).
fn grid_point(term: &Term, (col, line): (usize, usize)) -> Point {
    let (col, line) = (col.min(term.columns() - 1), line.min(term.screen_lines() - 1));
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
        let r = self.renderer.as_ref()?;
        // Grabbable half a cell either side: the line itself is a pixel or two.
        let grab = (r.font.cell_w / 2.0).max(r.px(4.0));
        self.dividers.iter().position(|d| {
            let r = d.rect;
            Rect { x: r.x - grab, y: r.y - grab, w: r.w + 2.0 * grab, h: r.h + 2.0 * grab }.contains(x, y)
        })
    }

    /// Height of the tab bar (always shown, for its buttons).
    fn bar_h(&self) -> f32 {
        match &self.renderer {
            // Under the window border's top edge; a 1px line at the bottom.
            Some(r) => r.font.cell_h + r.px(BORDER) + 2.0 * r.px(PAD) + r.px(1.0),
            _ => 0.0,
        }
    }

    fn tab_w(&self) -> f32 {
        let Some(r) = &self.renderer else { return 1.0 };
        ((r.size().0 - bar_buttons_w(r) - r.px(BORDER) - r.px(PAD)) / self.tabs.len() as f32).min(r.font.cell_w * TAB_CELLS).floor()
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
        let command_is_shell = command.is_none();
        let (shell, env) = match command {
            Some(cmd) => (cmd.split_first().map(|(program, args)| tty::Shell::new(program.clone(), args.to_vec())), HashMap::new()),
            None => shell::login(&self.shell, self.config.terminal.pretty_prompt),
        };
        let what = format!("{shell:?}");
        // The first shell of a window opens under the welcome screen.
        let welcome = id == 0 && command_is_shell && self.config.terminal.welcome;
        if welcome {
            let screen = crate::welcome::screen(dims.columns(), &renderer.gpu, &self.shell_name());
            alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut *term.lock(), screen.as_bytes());
        }
        let options = tty::Options { shell, working_directory: cwd, env, ..Default::default() };
        let cell = (renderer.font.cell_w, renderer.font.cell_h);
        let images = Arc::new(Mutex::new(Images::default()));
        images.lock().unwrap().cell = cell;
        let pty = Pty::spawn(term.clone(), images.clone(), size, listener, &options).map_err(|e| log::error!("can't start {what}: {e}")).ok()?;
        let pane = Pane {
            term,
            images,
            pty,
            theme: Fade::new(theme_named(&self.config, &self.config.theme)),
            theme_name: String::new(),
            context: Context::default(),
            last_context_check: Instant::now(),
            context_due: None,
            rect: Rect::default(),
            blocks: Blocks::default(),
            cwd: None,
            failed_at: None,
            at_prompt: false,
            welcome,
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

    /// Open a pane on `side` of the focused one.
    fn split(&mut self, side: Dir, command: Option<Vec<String>>) {
        let cwd = self.new_cwd();
        let Some(id) = self.spawn_pane(cwd, command) else { return };
        let target = self.focused();
        self.tabs[self.active].root.split(target, id, side);
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

    /// Close every pane in tab `i`.
    fn close_tab(&mut self, event_loop: &ActiveEventLoop, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        for id in tab.root.leaves() {
            self.close_pane(event_loop, id);
        }
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
        self.dirty = true;
    }

    /// The shell's name, for the welcome screen.
    fn shell_name(&self) -> String {
        let path = if self.shell.is_empty() { std::env::var("SHELL").unwrap_or_default() } else { self.shell.clone() };
        path.rsplit('/').next().unwrap_or("").to_owned()
    }

    /// Lay out every tab and fit each pane's grid and PTY to its rect.
    fn relayout(&mut self) {
        let bar = self.bar_h();
        let shell = self.shell_name();
        let Some(renderer) = &self.renderer else { return };
        let (w, h) = renderer.size();
        let border = renderer.px(BORDER);
        let area = Rect { x: border, y: bar, w: w - 2.0 * border, h: h - bar - border };
        for (i, tab) in self.tabs.iter().enumerate() {
            let (mut rects, mut dividers) = (Vec::new(), Vec::new());
            tab.root.layout(area, &mut rects, &mut dividers);
            if i == self.active {
                self.dividers = dividers;
            }
            for (id, rect) in rects {
                let Some(pane) = self.panes.get_mut(&id) else { continue };
                pane.rect = rect;
                pane.images.lock().unwrap().cell = (renderer.font.cell_w, renderer.font.cell_h);
                let size = window_size(&renderer.font, rect.w, rect.h);
                let (cols, lines) = (size.num_cols as usize, size.num_lines as usize);
                let mut term = pane.term.lock();
                if (term.columns(), term.screen_lines()) != (cols, lines) {
                    term.resize(TermSize::new(cols, lines));
                    // A welcome screen still up is drawn again for the new width (reflowed,
                    // it falls apart); the resize then has the shell redraw its prompt below.
                    if pane.welcome {
                        let screen = format!("\x1b[H\x1b[2J\x1b[3J{}", crate::welcome::screen(cols, &renderer.gpu, &shell));
                        alacritty_terminal::vte::ansi::Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new().advance(&mut *term, screen.as_bytes());
                    }
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
        let divider = self.drag.or_else(|| self.divider_at(x, y));
        let bar = match &self.renderer {
            Some(r) if y < self.bar_h() && self.menu.is_none() => bar_hit(r, self.tabs.len(), self.tab_w(), x),
            _ => None,
        };
        if (divider, bar) != (self.divider_hover, self.bar_hover) {
            (self.divider_hover, self.bar_hover) = (divider, bar);
            self.dirty = true;
        }
        let icon = match divider.map(|i| self.dividers[i].across) {
            Some(true) => CursorIcon::ColResize,
            Some(false) => CursorIcon::RowResize,
            None if self.hover.is_some() || bar.is_some() => CursorIcon::Pointer,
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
                Some(cmd) => self.split(Dir::Right, Some(cmd)),
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
        self.panes.get_mut(&self.tabs[self.active].focused).unwrap().clear_welcome();
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
            Action::SplitRight => self.split(Dir::Right, None),
            Action::SplitDown => self.split(Dir::Down, None),
            Action::SplitLeft => self.split(Dir::Left, None),
            Action::SplitUp => self.split(Dir::Up, None),
            Action::Focus(dir) => match pane::neighbor(&self.active_rects(), self.focused(), dir) {
                Some(id) => self.focus(id),
                // Shells print unbound Alt+Arrows as ;3A junk; programs still get them.
                None if self.focused_pane().at_prompt => {}
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
                self.palette = Some(Palette::new(self.config.themes().into_keys().collect(), shell::installed()));
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
        match mark {
            Mark::CommandEnd(_) => pane.at_prompt = true, // sent before every prompt
            Mark::CommandStart => pane.at_prompt = false,
            _ => {}
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

// Right-click menu and tab bar buttons.
impl App {
    /// A key while the menu is open: arrows pick, Enter runs, Esc closes; the rest are swallowed.
    /// Copy needs a selection; the rest always work.
    fn menu_enabled(&self, i: usize) -> bool {
        MENU[i] != Action::Copy || self.panes[&self.focused()].term.lock().selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    fn menu_key(&mut self, event_loop: &ActiveEventLoop, key: &Key) -> bool {
        let Some((at, hovered)) = self.menu else { return false };
        self.dirty = true;
        let n = MENU.len();
        let step = |from: usize, by: usize| (1..=n).map(|k| (from + k * by) % n).find(|&i| self.menu_enabled(i));
        match key {
            Key::Named(NamedKey::ArrowDown) => self.menu = Some((at, step(hovered.unwrap_or(n - 1), 1))),
            Key::Named(NamedKey::ArrowUp) => self.menu = Some((at, step(hovered.unwrap_or(0), n - 1))),
            Key::Named(NamedKey::Enter) => {
                self.menu = None;
                if let Some(i) = hovered.filter(|&i| self.menu_enabled(i)) {
                    self.run(event_loop, MENU[i]);
                }
            }
            Key::Named(NamedKey::Escape) => self.menu = None,
            _ => {}
        }
        true
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
                    Some(Item::Shell(path)) => {
                        self.shell = path;
                        self.new_tab();
                    }
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
        renderer.set_font(&self.config.font.family, px, self.config.font.weight);
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
        pane.context_due = self.config.rules.iter().any(|r| r.when.uses_hours()).then(|| pane.last_context_check + CONTEXT_CLOCK);
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
        pane.context = ctx;
        self.dirty = true;
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
        let icon =
            crate::images::png(include_bytes!("../extra/noobsbot-128.png")).ok().and_then(|p| winit::window::Icon::from_rgba(p.rgba.clone(), p.w, p.h).ok());
        let mut attrs = Window::default_attributes().with_title("Noobsbot Terminal").with_window_icon(icon);
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
        } else {
            // Room for the welcome screen.
            attrs = attrs.with_inner_size(winit::dpi::LogicalSize::new(1400.0, 860.0));
        }
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        if let Some(mhz) = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()) {
            self.frame_time = Duration::from_secs_f64(1000.0 / mhz as f64);
        }
        let font = Font::new(&self.config.font.family, self.font_px(&window), self.config.font.weight);
        self.renderer = Some(Renderer::new(window.clone(), font));
        self.window = Some(window);
        self.new_tab();
        if self.tabs.is_empty() {
            event_loop.exit(); // no shell to run
        }
        if self.quake.is_none() {
            // Focus it: left unfocused on top, clicks on the window behind wouldn't raise that.
            crate::quake::activate(self.window.as_ref().unwrap());
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
            // alacritty_terminal answers DA1 as a VT102; say VT220 with sixel (4) and color (22),
            // so image tools find our graphics (kitten icat also ignores answers as short as `?6c`).
            Event::PtyWrite(text) if text == "\x1b[?6c" => pane.pty.write(b"\x1b[?62;4;22c"),
            Event::PtyWrite(text) => pane.pty.write(text.as_bytes()),
            Event::Wakeup => {
                if std::env::var_os("NOOBSBOT_DUMP").is_some() {
                    dump_grid(&pane.term.lock());
                }
                // Drawn from about_to_wait at most once per refresh, so a flood of output
                // doesn't make the renderer hog the terminal lock.
                self.dirty |= self.tabs.get(self.active).is_some_and(|t| t.root.contains(id));
                self.context_poke(id);
            }
            // CSI 14 t; image tools ask to size pictures.
            Event::TextAreaSizeRequest(reply) => {
                let Some(r) = &self.renderer else { return };
                pane.pty.write(reply(window_size(&r.font, pane.rect.w, pane.rect.h)).as_bytes());
            }
            // OSC 4/10/11/12 queries: 0..=255 palette, then foreground, background, cursor.
            Event::ColorRequest(index, reply) => {
                let color = match index {
                    0..=255 => Color::Indexed(index as u8),
                    256 => Color::Named(NamedColor::Foreground),
                    257 => Color::Named(NamedColor::Background),
                    _ => Color::Named(NamedColor::Cursor),
                };
                let rgb = pane.theme.target().resolve(color, pane.term.lock().colors());
                pane.pty.write(reply(rgb).as_bytes());
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
                if self.menu_key(event_loop, &event.logical_key) {
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
                // Ctrl+V pastes at the shell prompt, where the shell would only take the
                // next key literally (and show a paste's ^[[200~). Programs still get it.
                if self.mods == ModifiersState::CONTROL
                    && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("v"))
                    && self.focused_pane().at_prompt
                {
                    self.paste(false);
                    return;
                }
                let mods = self.mods;
                let pane = self.panes.get_mut(&self.tabs[self.active].focused).unwrap();
                let app_cursor = pane.term.lock().mode().contains(TermMode::APP_CURSOR);
                let Some(bytes) = key_bytes(&event.logical_key, event.text.as_deref(), mods, app_cursor) else { return };
                if pane.clear_welcome() {
                    self.dirty = true;
                    // Enter only clears the welcome screen; any other key also types.
                    if event.logical_key == Key::Named(NamedKey::Enter) {
                        return;
                    }
                }
                let mut term = pane.term.lock();
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
            WindowEvent::CursorLeft { .. } => {
                if self.bar_hover.take().is_some() | self.divider_hover.take().is_some() {
                    self.dirty = true;
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.renderer.as_mut().unwrap().scale = scale_factor as f32;
                self.apply_font();
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
                if let Some((at, hovered)) = self.menu {
                    let row = menu_row(self.renderer.as_ref().unwrap(), at, (x, y));
                    if row != hovered {
                        self.menu = Some((at, row));
                        self.dirty = true;
                    }
                    return;
                }
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
                // An open menu takes the next click: an item runs, anywhere else closes it.
                if let Some((at, _)) = self.menu.filter(|_| pressed) {
                    self.menu = None;
                    self.dirty = true;
                    let r = self.renderer.as_ref().unwrap();
                    if button == MouseButton::Left
                        && let Some(i) = menu_row(r, at, (x, y))
                        && self.menu_enabled(i)
                    {
                        self.run(event_loop, MENU[i]);
                    }
                    return;
                }
                if pressed && y < self.bar_h() {
                    let hit = bar_hit(self.renderer.as_ref().unwrap(), self.tabs.len(), self.tab_w(), x);
                    match (button, hit) {
                        (MouseButton::Left, Some(BarHit::Button(action))) => _ = self.run(event_loop, action),
                        (MouseButton::Left, Some(BarHit::Tab(i))) => self.switch_tab(i),
                        (MouseButton::Left, Some(BarHit::Close(i))) | (MouseButton::Middle, Some(BarHit::Tab(i) | BarHit::Close(i))) => {
                            self.close_tab(event_loop, i)
                        }
                        _ => {}
                    }
                    self.update_icon();
                    return;
                }
                if pressed
                    && button == MouseButton::Left
                    && let Some(i) = self.divider_at(x, y)
                {
                    self.drag = Some(i);
                    return;
                }
                let id = self.mouse_target();
                if pressed && id != self.focused() {
                    self.focus(id);
                }
                if pressed
                    && button == MouseButton::Left
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
                                // Copy on select: middle-click paste and Ctrl+V both get it.
                                Some(text) => {
                                    self.clipboard.set(true, text.clone());
                                    self.clipboard.set(false, text);
                                }
                                None => term.selection = None,
                            }
                            self.dirty = true;
                        }
                        (MouseButton::Middle, true) => self.paste(true),
                        (MouseButton::Right, true) => {
                            let r = self.renderer.as_ref().unwrap();
                            self.menu = Some((menu_origin(r, (x, y)), None));
                            self.dirty = true;
                        }
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
                    renderer.pane(&term, &pane.images.lock().unwrap(), &theme, pane.rect, id == tab.focused, &matches, current.as_ref(), link);
                    let vi = term.mode().contains(TermMode::VI);
                    let gutter = gutter(&term, &pane.blocks, &theme);
                    let cursor_row = usize::try_from(term.grid().cursor.point.line.0 + term.grid().display_offset() as i32).ok();
                    drop(term);
                    let flash = pane.failed_at.map(|t| 1.0 - t.elapsed().as_secs_f32() / FLASH.as_secs_f32()).filter(|&a| a > 0.0);
                    if flash.is_none() {
                        pane.failed_at = None;
                    }
                    let ui = PaneUi { vi, search: self.search.as_ref().filter(|s| s.pane == id && s.editing), gutter, flash, cursor_row };
                    draw_pane_ui(renderer, pane.rect, &theme, ui);
                    if id == tab.focused {
                        focused_theme = Some(theme);
                    }
                }
                let theme = focused_theme.expect("focused pane is in its tab");
                for (i, d) in self.dividers.iter().enumerate() {
                    // The one you can grab lights up, at twice the width.
                    let (color, grow) = match self.divider_hover == Some(i) {
                        true => (theme.cursor, renderer.px(1.0)),
                        false => (theme.selection, 0.0),
                    };
                    let r = d.rect;
                    let r = if d.across { Rect { x: r.x - grow / 2.0, w: r.w + grow, ..r } } else { Rect { y: r.y - grow / 2.0, h: r.h + grow, ..r } };
                    renderer.fill(r, theme::to_f32(color));
                }
                draw_tab_bar(renderer, &self.tabs, &mut self.panes, self.active, bar_h, tab_w, self.bar_hover);
                if let Some(p) = &self.palette {
                    draw_palette(renderer, p, &theme);
                }
                if let Some((at, hovered)) = self.menu {
                    let can_copy = self.panes[&tab.focused].term.lock().selection.as_ref().is_some_and(|s| !s.is_empty());
                    draw_menu(renderer, at, hovered, can_copy, &theme);
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
    /// The cursor's row on screen, if shown: the search box keeps off it.
    cursor_row: Option<usize>,
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
    let (accent, ink) = (theme::to_f32(theme.cursor), theme::to_f32(theme.bg));
    let muted = theme::to_f32(theme::blend(theme.fg, theme.bg, 0.45));
    // The search box floats at the top right, clear of the prompt at the bottom;
    // at the bottom right when the prompt is at the top.
    let mut right = rect.x + rect.w;
    if let Some(s) = search {
        let status = match (s.query.is_empty(), s.total) {
            (true, _) => String::new(),
            (false, 0) => "No matches".to_owned(),
            (false, n) if n >= crate::search::COUNT_MAX => format!("{} of {}+", s.index, n),
            (false, n) => format!("{} of {}", s.index, n),
        };
        let status_color = if s.total == 0 { theme::to_f32(theme.ansi[1]) } else { muted };
        let cols = ((rect.w / cw) as usize).saturating_sub(1).min(48);
        let (w, pad) = (cols as f32 * cw, r.px(1.0) + (cw / 4.0).round());
        let top = ui.cursor_row != Some(0);
        let y = if top { rect.y + pad } else { rect.y + rect.h - ch - pad };
        let x = (rect.x + rect.w - w - pad).round();
        panel(r, Rect { x: x - pad, y: y - pad, w: w + 2.0 * pad, h: ch + 2.0 * pad }, theme);
        let qx = segment(r, x, y, " Find ", accent, ink);
        let status_cells = status.chars().count() + 1;
        // Keep the end of a long query in view, left of the count.
        let room = ((x + w - qx) / cw) as usize;
        let room = room.saturating_sub(status_cells);
        let query = format!(" {}_", s.query);
        let skip = query.chars().count().saturating_sub(room);
        r.text(qx, y, &query.chars().skip(skip).collect::<String>(), theme::to_f32(theme.fg));
        if room > 0 {
            r.text(x + w - status_cells as f32 * cw, y, &status, status_color);
        }
        if top {
            right = x - pad - cw;
        }
    }
    if vi {
        pill(r, right - cw * 5.0, rect.y, " VI", accent, ink);
    }
}

/// Panel color for overlays: between the background and the selection.
fn surface(theme: &Theme) -> Rgb {
    theme::blend(theme.bg, theme.selection, 0.6)
}

/// `text` on a rounded `bg` pill (powerline half-circle caps). Returns where it ends.
fn pill(r: &mut Renderer, x: f32, y: f32, text: &str, bg: [f32; 4], fg: [f32; 4]) -> f32 {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let n = text.chars().count() as f32;
    r.text(x, y, "\u{e0b6}", bg);
    r.fill(Rect { x: x + cw, y, w: n * cw, h: ch }, bg);
    r.text(x + cw, y, text, fg);
    r.text(x + (n + 1.0) * cw, y, "\u{e0b4}", bg);
    x + (n + 2.0) * cw
}

/// A powerline segment: rounded on the left, an arrow on the right.
fn segment(r: &mut Renderer, x: f32, y: f32, text: &str, bg: [f32; 4], fg: [f32; 4]) -> f32 {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let n = text.chars().count() as f32;
    r.text(x, y, "\u{e0b6}", bg);
    r.fill(Rect { x: x + cw, y, w: n * cw, h: ch }, bg);
    r.text(x + cw, y, text, fg);
    r.text(x + (n + 1.0) * cw, y, "\u{e0b0}", bg);
    x + (n + 2.0) * cw
}

/// The palette: a query line and the best matches, centered near the top.
fn draw_palette(r: &mut Renderer, p: &Palette, theme: &Theme) {
    const ROWS: usize = 12;
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let w = r.size().0;
    let cols = ((w / cw) as usize).saturating_sub(4).min(64);
    let (bw, pad) = (cols as f32 * cw, (cw / 2.0).round());
    let (x, y) = (((w - bw) / 2.0).round(), ch * 2.0); // below the tab bar
    let n = p.len().clamp(1, ROWS); // a row for "No matching commands"
    let (fg, ink, accent) = (theme::to_f32(theme.fg), theme::to_f32(theme.bg), theme::to_f32(theme.cursor));
    let muted = theme::to_f32(theme::blend(theme.fg, theme.bg, 0.45));
    // A rule between the query and the list.
    panel(r, Rect { x: x - pad, y: y - pad, w: bw + 2.0 * pad, h: (n + 2) as f32 * ch + 2.0 * pad }, theme);
    r.fill(Rect { x, y: (y + ch * 1.5).round(), w: bw, h: r.px(1.0) }, theme::to_f32(theme.selection));
    let qx = segment(r, x, y, " \u{276f} ", accent, ink);
    // How many match, right-aligned on the query line once you've typed something.
    let count = if p.query.is_empty() || p.len() == 0 { String::new() } else { format!("{} ", p.len()) };
    r.text(x + (cols - count.chars().count()) as f32 * cw, y, &count, muted);
    let room = cols.saturating_sub(5 + count.chars().count());
    let query = format!(" {}_", p.query);
    let skip = query.chars().count().saturating_sub(room); // keep the end of a long query in view
    r.text(qx, y, &query.chars().skip(skip).collect::<String>(), fg);
    if p.len() == 0 {
        r.text(x + cw * 2.0, y + 2.0 * ch, "No matching commands", muted);
    }
    let first = p.selected.saturating_sub(ROWS - 1);
    for (i, row) in p.shown().skip(first).take(ROWS).enumerate() {
        let ry = y + (i + 2) as f32 * ch;
        // Room for the name between the pill's caps, before the keys.
        let keys = row.keys.chars().count().min(cols.saturating_sub(4));
        let name: String = row.name.chars().take(cols.saturating_sub(keys + 4)).collect();
        let keys_x = x + (cols - keys - 1) as f32 * cw;
        if first + i == p.selected {
            pill(r, x, ry, &" ".repeat(cols - 2), accent, ink);
            r.text(x + cw * 2.0, ry, &name, ink);
            r.text(keys_x, ry, &row.keys.chars().take(keys).collect::<String>(), ink);
        } else {
            r.text(x + cw * 2.0, ry, &name, fg);
            r.text(keys_x, ry, &row.keys.chars().take(keys).collect::<String>(), muted);
        }
    }
}

/// Width of the tab bar's buttons: each with the gap before the next, then the window border.
fn bar_buttons_w(r: &Renderer) -> f32 {
    BAR_BUTTONS.len() as f32 * (BUTTON_CELLS * r.font.cell_w + r.px(PAD)) + r.px(BORDER)
}

/// Something in the tab bar you can point at.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BarHit {
    Tab(usize),
    Close(usize),
    Button(Action),
}

/// Label cells in a pill `tab_w` wide: its caps take a cell each, plus a cell of gap after.
fn tab_cells(cw: f32, tab_w: f32) -> usize {
    ((tab_w / cw) as usize).saturating_sub(4)
}

/// Tabs with this many label cells get a close button (in the last two).
const CLOSE_MIN: usize = 8;

/// What's at `x` in the tab bar. The drawing uses the same geometry, so a click
/// always lands on what's drawn under it.
fn bar_hit(r: &Renderer, tabs: usize, tab_w: f32, x: f32) -> Option<BarHit> {
    let buttons = r.size().0 - bar_buttons_w(r);
    hit_at(r.font.cell_w, r.px(BORDER) + r.px(PAD), r.px(PAD), buttons, tabs, tab_w, x)
}

/// `bar_hit` in plain numbers: tabs start at `first`, buttons at `buttons`.
fn hit_at(cw: f32, first: f32, pad: f32, buttons: f32, tabs: usize, tab_w: f32, x: f32) -> Option<BarHit> {
    if x >= buttons {
        let i = ((x - buttons) / (BUTTON_CELLS * cw + pad)) as usize;
        return BAR_BUTTONS.get(i).map(|a| BarHit::Button(*a));
    }
    let into = x - first;
    let i = (into / tab_w).floor();
    if tabs < 2 || i < 0.0 || i as usize >= tabs {
        return None;
    }
    let (i, into, fit) = (i as usize, into - i * tab_w, tab_cells(cw, tab_w));
    if into >= (fit + 2) as f32 * cw {
        return None; // the gap between pills
    }
    // The cross and the cap after it.
    Some(if fit >= CLOSE_MIN && into >= fit as f32 * cw { BarHit::Close(i) } else { BarHit::Tab(i) })
}

/// A bar button's icon, drawn with pixel-aligned rects so it stays sharp:
/// a plus, or a window split side by side or stacked, centered on (cx, cy).
fn button_icon(r: &mut Renderer, action: Action, cx: f32, cy: f32, color: [f32; 4]) {
    let ch = r.font.cell_h;
    let t = (ch / 11.0).round().max(1.0);
    let mut rect = |x: f32, y: f32, w: f32, h: f32| r.fill(Rect { x: x.round(), y: y.round(), w, h }, color);
    if action == Action::NewTab {
        let s = (ch * 0.5).round();
        rect(cx - s / 2.0, cy - t / 2.0, s, t);
        rect(cx - t / 2.0, cy - s / 2.0, t, s);
        return;
    }
    let (w, h) = ((ch * 0.7).round(), (ch * 0.55).round());
    let (x, y) = ((cx - w / 2.0).round(), (cy - h / 2.0).round());
    rect(x, y, w, t);
    rect(x, y + h - t, w, t);
    rect(x, y, t, h);
    rect(x + w - t, y, t, h);
    match action {
        Action::SplitRight => rect(x + (w - t) / 2.0, y, t, h),
        _ => rect(x, y + (h - t) / 2.0, w, t),
    }
}

/// Menu rows: (name, keys) from the action table.
fn menu_rows() -> impl Iterator<Item = (&'static str, &'static str)> {
    MENU.iter().filter_map(|a| input::actions().iter().find(|e| e.action == *a)).map(|e| (e.name.as_str(), e.keys.split(" or ").next().unwrap_or("")))
}

/// The menu's width in cells (room for the longest name, a gap, the longest
/// keys) and its height in pixels: half a row above and below, rows, rules.
fn menu_size(r: &Renderer) -> (f32, f32) {
    let name = menu_rows().map(|(n, _)| n.chars().count()).max().unwrap_or(0);
    let keys = menu_rows().map(|(_, k)| k.chars().count()).max().unwrap_or(0);
    let ch = r.font.cell_h;
    ((name + keys + 7) as f32, (MENU.len() + 1) as f32 * ch + MENU_BREAKS.len() as f32 * ch / 2.0)
}

/// Top of row `i`, from the menu's top: rules between groups take half a row.
fn menu_row_y(ch: f32, i: usize) -> f32 {
    ch / 2.0 + i as f32 * ch + MENU_BREAKS.iter().filter(|&&b| b < i).count() as f32 * ch / 2.0
}

/// Where the menu goes for a click at `at`: down and right of it, flipped up or
/// left where it would leave the window.
fn menu_origin(r: &Renderer, at: (f32, f32)) -> (f32, f32) {
    let (cols, mh) = menu_size(r);
    let (w, h) = r.size();
    let mw = cols * r.font.cell_w;
    let x = if at.0 + mw > w { at.0 - mw } else { at.0 };
    let y = if at.1 + mh > h { at.1 - mh } else { at.1 };
    (x.clamp(0.0, (w - mw).max(0.0)).round(), y.clamp(0.0, (h - mh).max(0.0)).round())
}

/// The menu row under `p`, if any.
fn menu_row(r: &Renderer, origin: (f32, f32), p: (f32, f32)) -> Option<usize> {
    let (cols, _) = menu_size(r);
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let (x, y) = (p.0 - origin.0, p.1 - origin.1);
    if x < 0.0 || x >= cols * cw {
        return None;
    }
    (0..MENU.len()).find(|&i| (menu_row_y(ch, i)..menu_row_y(ch, i) + ch).contains(&y))
}

/// A popup's panel: the surface color with a hairline edge, so it stands off
/// whatever is behind it.
fn panel(r: &mut Renderer, rect: Rect, theme: &Theme) {
    let t = r.px(1.0);
    r.fill(rect, theme::to_f32(theme.selection));
    r.fill(Rect { x: rect.x + t, y: rect.y + t, w: rect.w - 2.0 * t, h: rect.h - 2.0 * t }, theme::to_f32(surface(theme)));
}

/// The right-click menu: action names with their keys in groups, the hovered
/// row as a pill. Copy is dimmed with nothing selected.
fn draw_menu(r: &mut Renderer, origin: (f32, f32), hovered: Option<usize>, can_copy: bool, theme: &Theme) {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let (cols, h) = menu_size(r);
    let (fg, ink, accent) = (theme::to_f32(theme.fg), theme::to_f32(theme.bg), theme::to_f32(theme.cursor));
    let muted = theme::to_f32(theme::blend(theme.fg, theme.bg, 0.45));
    let faint = theme::to_f32(theme::blend(theme.fg, surface(theme), 0.7));
    panel(r, Rect { x: origin.0, y: origin.1, w: cols * cw, h }, theme);
    for &b in &MENU_BREAKS {
        let y = (origin.1 + menu_row_y(ch, b + 1) - ch / 4.0).round();
        r.fill(Rect { x: origin.0 + cw, y, w: (cols - 2.0) * cw, h: r.px(1.0) }, theme::to_f32(theme.selection));
    }
    for (i, (name, keys)) in menu_rows().enumerate() {
        let y = origin.1 + menu_row_y(ch, i);
        let keys_x = origin.0 + (cols - 2.0 - keys.chars().count() as f32) * cw;
        let enabled = MENU[i] != Action::Copy || can_copy;
        if hovered == Some(i) && enabled {
            pill(r, origin.0 + cw / 2.0, y, &" ".repeat(cols as usize - 3), accent, ink);
            r.text(origin.0 + cw * 2.0, y, name, ink);
            r.text(keys_x, y, keys, ink);
        } else {
            r.text(origin.0 + cw * 2.0, y, name, if enabled { fg } else { faint });
            r.text(keys_x, y, keys, if enabled { muted } else { faint });
        }
    }
}

/// One tab per slot, each tinted with its focused pane's situation theme, so a
/// red production tab stays visible while you work in another one.
fn draw_tab_bar(r: &mut Renderer, tabs: &[Tab], panes: &mut HashMap<PaneId, Pane>, active: usize, bar_h: f32, tab_w: f32, hover: Option<BarHit>) {
    let (cw, ch) = (r.font.cell_w, r.font.cell_h);
    let backdrop = theme::dim(panes.get_mut(&tabs[active].focused).unwrap().theme.frame().bg);
    r.fill(Rect { x: 0.0, y: 0.0, w: r.size().0, h: bar_h }, theme::to_f32(backdrop));
    let y = r.px(BORDER) + r.px(PAD);
    let fit = tab_cells(cw, tab_w);
    let closable = fit >= CLOSE_MIN;
    // A lone tab isn't drawn: the bar is just for the buttons then.
    for (i, tab) in tabs.iter().enumerate().filter(|_| tabs.len() > 1) {
        let pane = panes.get_mut(&tab.focused).unwrap();
        let theme = pane.theme.frame();
        let x = r.px(BORDER) + r.px(PAD) + i as f32 * tab_w;
        let hovered = matches!(hover, Some(BarHit::Tab(h) | BarHit::Close(h)) if h == i);
        // The close button keeps the last two cells, so labels don't shift when it shows.
        let room = if closable { fit - 2 } else { fit };
        let mut label = format!(" Terminal {} ", i + 1);
        // Tight: just the number, padded if there's room.
        for short in [format!(" {} ", i + 1), (i + 1).to_string()] {
            if label.chars().count() > room {
                label = short;
            }
        }
        if label.chars().count() > room {
            label = label.chars().take(room.saturating_sub(1)).chain((room > 0).then_some('…')).collect();
        }
        // Fill the slot, so each tab reads as its own block.
        let label = format!("{label:<fit$}");
        let (bg, fg) = match (i == active, hovered) {
            (true, _) => (theme.cursor, theme.bg),
            (false, true) => (theme::blend(theme.selection, theme.fg, 0.12), theme.fg),
            (false, false) => (theme::blend(backdrop, theme.selection, 0.7), theme::blend(theme.fg, theme.bg, 0.3)),
        };
        pill(r, x, y, &label, theme::to_f32(bg), theme::to_f32(fg));
        if closable && (i == active || hovered) {
            let cx = x + fit as f32 * cw;
            if hover == Some(BarHit::Close(i)) {
                r.fill(Rect { x: cx, y, w: cw, h: ch }, theme::to_f32(theme::blend(bg, fg, 0.25)));
            }
            r.text(cx, y, &crate::font::CLOSE.to_string(), theme::to_f32(fg));
        }
    }
    let theme = panes.get_mut(&tabs[active].focused).unwrap().theme.frame();
    r.fill(Rect { x: 0.0, y: bar_h - r.px(1.0), w: r.size().0, h: r.px(1.0) }, theme::to_f32(theme::blend(backdrop, theme.selection, 0.7)));
    let mut x = r.size().0 - bar_buttons_w(r);
    let w = BUTTON_CELLS * cw;
    for action in BAR_BUTTONS {
        let (bg, fg) = match hover == Some(BarHit::Button(action)) {
            true => (theme::blend(theme.selection, theme.fg, 0.12), theme.fg),
            false => (theme::blend(backdrop, theme.selection, 0.7), theme::blend(theme.fg, theme.bg, 0.15)),
        };
        r.fill(Rect { x, y, w, h: ch }, theme::to_f32(bg));
        button_icon(r, action, x + w / 2.0, y + ch / 2.0, theme::to_f32(fg));
        x += w + r.px(PAD);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_hits() {
        // 10px cells, tabs from x=6 and 200px wide (16 label cells), buttons from x=1000.
        let hit = |x| hit_at(10.0, 6.0, 2.0, 1000.0, 3, 200.0, x);
        assert_eq!(hit(10.0), Some(BarHit::Tab(0)));
        assert_eq!(hit(6.0 + 200.0 + 50.0), Some(BarHit::Tab(1)));
        // The cross sits in the last label cell; the cap after it counts too.
        assert_eq!(hit(6.0 + 160.0), Some(BarHit::Close(0)));
        assert_eq!(hit(6.0 + 175.0), Some(BarHit::Close(0)));
        assert_eq!(hit(6.0 + 185.0), None); // gap between pills
        assert_eq!(hit(6.0 + 3.0 * 200.0 + 10.0), None); // past the last tab
        assert_eq!(hit(1001.0), Some(BarHit::Button(Action::NewTab)));
        assert_eq!(hit(1000.0 + 52.0 + 5.0), Some(BarHit::Button(Action::SplitRight)));
        // A lone tab isn't drawn, so it isn't hit either.
        assert_eq!(hit_at(10.0, 6.0, 2.0, 1000.0, 1, 200.0, 10.0), None);
    }

    #[test]
    fn menu_rows_and_rules() {
        // Half a row of padding, then rows, with half a row per rule passed.
        assert_eq!((menu_row_y(20.0, 0), menu_row_y(20.0, 1), menu_row_y(20.0, 2), menu_row_y(20.0, 6)), (10.0, 30.0, 60.0, 150.0));
    }
}
