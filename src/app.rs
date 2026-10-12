use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::test::TermSize;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::clipboard::Clipboard;
use crate::config::{self, Config};
use crate::context::{self, Context};
use crate::font::Font;
use crate::input::{self, Action, binding, key_bytes, mouse_report, paste_bytes};
use crate::pty::Pty;
use crate::render::Renderer;
use crate::term::{Listener, Term, UserEvent};
use crate::theme::Theme;

const BLINK: Duration = Duration::from_millis(530);
const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Minimum gap between situation checks while output is streaming.
const CONTEXT_THROTTLE: Duration = Duration::from_millis(200);
/// Re-check at least this often when a rule depends on the time of day.
const CONTEXT_CLOCK: Duration = Duration::from_secs(60);

pub struct App {
    proxy: EventLoopProxy<UserEvent>,
    config: Config,
    _config_watcher: Option<notify::RecommendedWatcher>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    term: Option<Arc<FairMutex<Term>>>,
    pty: Option<Pty>,
    mods: ModifiersState,
    next_blink: Instant,
    /// Terminal content changed since the last frame.
    dirty: bool,
    last_frame: Instant,
    frame_time: Duration,
    /// Sub-line remainder of touchpad scrolling, in pixels.
    scroll_px: f64,
    /// Pointer position in viewport cells (col, line).
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
    /// Situation the theme was last chosen for, and the theme's name.
    context: Context,
    theme_name: String,
    last_context_check: Instant,
    /// A situation check is owed at this time (throttled output, or the clock).
    context_due: Option<Instant>,
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let path = config::path();
        let config = config::load(&path).unwrap_or_else(|e| {
            log::error!("config error, using defaults: {e}");
            Config::default()
        });
        Self {
            _config_watcher: config::watch(&path, proxy.clone()),
            config,
            proxy,
            window: None,
            renderer: None,
            term: None,
            pty: None,
            mods: ModifiersState::empty(),
            next_blink: Instant::now(),
            dirty: false,
            last_frame: Instant::now(),
            frame_time: Duration::from_micros(16_667),
            scroll_px: 0.0,
            mouse_cell: (0, 0),
            mouse_held: None,
            mouse_side: Side::Left,
            selecting: false,
            last_click: Instant::now(),
            clicks: 0,
            clipboard: Clipboard::new(),
            zoom: 0.0,
            context: Context::default(),
            theme_name: String::new(),
            last_context_check: Instant::now(),
            context_due: None,
        }
    }
}

impl App {
    /// Grid point under the pointer (accounts for scrollback offset).
    fn mouse_point(&self, term: &Term) -> Point {
        let (col, line) = self.mouse_cell;
        Point::new(Line(line as i32 - term.grid().display_offset() as i32), Column(col))
    }

    fn paste(&mut self, primary: bool) {
        let (Some(text), Some(term), Some(pty)) = (self.clipboard.get(primary), &self.term, self.pty.as_mut()) else { return };
        let mut term = term.lock();
        let bracketed = term.mode().contains(TermMode::BRACKETED_PASTE);
        term.scroll_display(Scroll::Bottom);
        drop(term);
        pty.write(&paste_bytes(&text, bracketed));
        self.dirty = true;
    }
}

fn window_size(font: &Font, width: u32, height: u32) -> WindowSize {
    WindowSize {
        num_cols: (width as f32 / font.cell_w).max(1.0) as u16,
        num_lines: (height as f32 / font.cell_h).max(1.0) as u16,
        cell_width: font.cell_w as u16,
        cell_height: font.cell_h as u16,
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
        self.resize_grid();
    }

    fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
        self.apply_font();
    }

    /// Fit the grid and PTY to the window after the window or the cell size changed.
    fn resize_grid(&mut self) {
        let (Some(window), Some(renderer), Some(term), Some(pty)) = (&self.window, &self.renderer, &self.term, self.pty.as_mut()) else { return };
        let px = window.inner_size();
        let size = window_size(&renderer.font, px.width, px.height);
        term.lock().resize(TermSize::new(size.num_cols as usize, size.num_lines as usize));
        pty.resize(size);
        self.dirty = true;
    }

    /// Palette for the theme name, falling back to `default` for unknown names.
    fn theme(&self, name: &str) -> Theme {
        let themes = self.config.themes();
        themes.get(name).cloned().unwrap_or_else(|| {
            log::warn!("unknown theme {name:?}, using default");
            themes["default"].clone()
        })
    }

    /// Re-detect the situation and switch theme if a different rule now matches.
    fn check_context(&mut self, force: bool) {
        let Some(pty) = &self.pty else { return };
        let ctx = context::detect(pty.fd(), pty.shell_pid());
        self.last_context_check = Instant::now();
        self.context_due = self
            .config
            .rules
            .iter()
            .any(|r| r.when.uses_hours())
            .then(|| self.last_context_check + CONTEXT_CLOCK);
        if ctx == self.context && !force {
            return;
        }
        let rule = crate::rules::pick(&self.config.rules, &ctx);
        let name = rule.map_or(self.config.theme.clone(), |r| r.theme.clone());
        if name != self.theme_name || force {
            log::info!("theme → {name} ({})", if rule.is_some() { format!("{ctx:?}") } else { "no rule matched".into() });
            let theme = self.theme(&name);
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.theme = theme;
            }
            self.theme_name = name;
            self.dirty = true;
        }
        self.context = ctx;
    }

    /// Output arrived: the foreground program may have changed. Check now or soon.
    fn context_poke(&mut self) {
        let due = self.last_context_check + CONTEXT_THROTTLE;
        if Instant::now() >= due {
            self.check_context(false);
        } else if self.context_due.is_none_or(|d| d > due) {
            self.context_due = Some(due);
        }
    }

    fn reload_config(&mut self) {
        let config = match config::load(&config::path()) {
            Ok(c) => c,
            Err(e) => return log::error!("config error, keeping previous config: {e}"),
        };
        log::info!("config reloaded");
        let font_changed = config.font != self.config.font;
        self.config = config;
        let Some(term) = &self.term else { return };
        term.lock().set_options(self.config.term());
        self.check_context(true);
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
        let attrs = Window::default_attributes().with_title("noobsbot terminal");
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        if let Some(mhz) = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()) {
            self.frame_time = Duration::from_secs_f64(1000.0 / mhz as f64);
        }
        let font = Font::new(&self.config.font.family, self.font_px(&window));
        let px = window.inner_size();
        let size = window_size(&font, px.width, px.height);

        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(self.config.term(), &dims, Listener(self.proxy.clone()))));
        self.pty = Some(Pty::spawn(term.clone(), size, self.proxy.clone(), &self.config.terminal.shell));
        self.term = Some(term);
        self.renderer = Some(Renderer::new(window.clone(), font, self.theme(&self.config.theme)));
        self.window = Some(window);
        self.check_context(true);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        let event = match event {
            UserEvent::ConfigChanged => return self.reload_config(),
            UserEvent::Term(event) => event,
        };
        match event {
            Event::PtyWrite(text) => {
                if let Some(pty) = self.pty.as_mut() {
                    pty.write(text.as_bytes());
                }
            }
            Event::Title(title) => {
                if let Some(w) = &self.window {
                    w.set_title(&title);
                }
            }
            Event::Wakeup => {
                if std::env::var_os("NOOBSBOT_DUMP").is_some() {
                    if let Some(term) = &self.term {
                        dump_grid(&term.lock());
                    }
                }
                // Drawn from about_to_wait at most once per refresh, so a flood of output
                // doesn't make the renderer hog the terminal lock.
                self.dirty = true;
                self.context_poke();
            }
            Event::ClipboardStore(ty, text) => {
                self.clipboard.set(ty == alacritty_terminal::term::ClipboardType::Selection, text);
            }
            Event::ClipboardLoad(ty, format) => {
                // OSC 52 read; only reaches us if `terminal.clipboard` allows pasting.
                let text = self.clipboard.get(ty == alacritty_terminal::term::ClipboardType::Selection).unwrap_or_default();
                if let Some(pty) = self.pty.as_mut() {
                    pty.write(format(&text).as_bytes());
                }
            }
            Event::Exit | Event::ChildExit(_) => event_loop.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // First, so a theme switch it causes is drawn below.
        if self.context_due.is_some_and(|due| now >= due) {
            self.check_context(false);
        }
        let (Some(term), Some(renderer), Some(window)) = (&self.term, self.renderer.as_mut(), &self.window) else { return };
        let mut wake = None;

        if renderer.focused && term.lock().cursor_style().blinking {
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

        if self.dirty {
            let due = self.last_frame + self.frame_time;
            if now >= due {
                self.dirty = false;
                window.request_redraw();
            } else {
                wake = Some(wake.map_or(due, |w: Instant| w.min(due)));
            }
        }
        if let Some(due) = self.context_due {
            wake = Some(wake.map_or(due, |w: Instant| w.min(due)));
        }
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(px) => {
                renderer.resize(px.width, px.height);
                self.resize_grid();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::Focused(focused) => {
                renderer.focused = focused;
                if let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) {
                    if term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                        pty.write(if focused { b"\x1b[I" } else { b"\x1b[O" });
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) else { return };
                if let Some(action) = binding(&event.logical_key, self.mods) {
                    let scroll = match action {
                        Action::ScrollPageUp => Scroll::PageUp,
                        Action::ScrollPageDown => Scroll::PageDown,
                        Action::ScrollTop => Scroll::Top,
                        Action::ScrollBottom => Scroll::Bottom,
                        Action::Copy => {
                            if let Some(text) = term.lock().selection_to_string() {
                                self.clipboard.set(false, text);
                            }
                            return;
                        }
                        Action::Paste => return self.paste(false),
                        Action::ZoomIn => return self.set_zoom(self.zoom + 1.0),
                        Action::ZoomOut => return self.set_zoom(self.zoom - 1.0),
                        Action::ZoomReset => return self.set_zoom(0.0),
                    };
                    term.lock().scroll_display(scroll);
                    self.dirty = true;
                    return;
                }
                let mut term = term.lock();
                let app_cursor = term.mode().contains(TermMode::APP_CURSOR);
                if let Some(bytes) = key_bytes(&event.logical_key, event.text.as_deref(), self.mods, app_cursor) {
                    if term.grid().display_offset() != 0 {
                        term.scroll_display(Scroll::Bottom);
                        self.dirty = true;
                    }
                    drop(term);
                    pty.write(&bytes);
                    // Keep the cursor solid while typing.
                    renderer.cursor_on = true;
                    self.next_blink = Instant::now() + BLINK;
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(term) = &self.term else { return };
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => {
                        self.scroll_px += p.y;
                        let cell_h = renderer.font.cell_h as f64;
                        let lines = (self.scroll_px / cell_h).trunc();
                        self.scroll_px -= lines * cell_h;
                        lines as i32
                    }
                };
                if lines == 0 {
                    return;
                }
                let mut term = term.lock();
                let mode = *term.mode();
                if mode.intersects(TermMode::MOUSE_MODE) {
                    let button = if lines > 0 { input::MOUSE_WHEEL_UP } else { input::MOUSE_WHEEL_DOWN };
                    let (col, line) = self.mouse_cell;
                    drop(term);
                    if let (Some(bytes), Some(pty)) = (mouse_report(button, true, col, line, self.mods, mode.contains(TermMode::SGR_MOUSE)), self.pty.as_mut()) {
                        for _ in 0..lines.abs() {
                            pty.write(&bytes);
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
                    if let Some(pty) = self.pty.as_mut() {
                        pty.write(&arrow.repeat(lines.unsigned_abs() as usize));
                    }
                } else {
                    term.scroll_display(Scroll::Delta(lines));
                    self.dirty = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(term) = &self.term else { return };
                let mut term = term.lock();
                let (cw, ch) = (renderer.font.cell_w, renderer.font.cell_h);
                let x = position.x.max(0.0) as f32;
                let col = ((x / cw) as usize).min(term.columns() - 1);
                let line = ((position.y.max(0.0) as f32 / ch) as usize).min(term.screen_lines() - 1);
                let side = if x - col as f32 * cw > cw / 2.0 { Side::Right } else { Side::Left };
                if (col, line, side) == (self.mouse_cell.0, self.mouse_cell.1, self.mouse_side) {
                    return;
                }
                self.mouse_side = side;
                self.mouse_cell = (col, line);
                if self.selecting {
                    // ponytail: no auto-scroll when dragging past the window edge yet.
                    let point = self.mouse_point(&term);
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
                if let (Some(button), Some(pty)) = (report, self.pty.as_mut()) {
                    let sgr = mode.contains(TermMode::SGR_MOUSE);
                    if let Some(bytes) = mouse_report(button + input::MOUSE_MOTION, true, col, line, self.mods, sgr) {
                        pty.write(&bytes);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) else { return };
                let mode = *term.lock().mode();
                // Shift bypasses mouse reporting so you can still select text in vim/tmux.
                if !mode.intersects(TermMode::MOUSE_MODE) || self.mods.shift_key() {
                    let pressed = state == ElementState::Pressed;
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
                            let mut term = term.lock();
                            let point = self.mouse_point(&term);
                            term.selection = Some(Selection::new(ty, point, self.mouse_side));
                            self.selecting = true;
                            self.dirty = true;
                        }
                        (MouseButton::Left, false) if self.selecting => {
                            self.selecting = false;
                            let mut term = term.lock();
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
                let pressed = state == ElementState::Pressed;
                self.mouse_held = pressed.then_some(code);
                let (col, line) = self.mouse_cell;
                if let Some(bytes) = mouse_report(code, pressed, col, line, self.mods, mode.contains(TermMode::SGR_MOUSE)) {
                    pty.write(&bytes);
                }
            }
            WindowEvent::RedrawRequested => {
                self.last_frame = Instant::now();
                if let Some(term) = &self.term {
                    renderer.render(&term.lock());
                }
            }
            _ => {}
        }
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
