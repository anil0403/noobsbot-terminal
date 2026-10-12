use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Line;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::term::test::TermSize;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::font::Font;
use crate::input::{self, Action, binding, key_bytes, mouse_report};
use crate::pty::Pty;
use crate::render::Renderer;
use crate::term::{Listener, Term};

// ponytail: hardcoded until the config file lands in M3.
const FONT_FAMILY: &str = "JetBrainsMono Nerd Font Mono";
const FONT_SIZE: f32 = 15.0;
const BLINK: Duration = Duration::from_millis(530);

pub struct App {
    proxy: EventLoopProxy<Event>,
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
}

impl App {
    pub fn new(proxy: EventLoopProxy<Event>) -> Self {
        Self { proxy, window: None, renderer: None, term: None, pty: None, mods: ModifiersState::empty(), next_blink: Instant::now(),
            dirty: false,
            last_frame: Instant::now(),
            frame_time: Duration::from_micros(16_667),
            scroll_px: 0.0,
            mouse_cell: (0, 0),
            mouse_held: None,
        }
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

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("noobsbot terminal");
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        if let Some(mhz) = window.current_monitor().and_then(|m| m.refresh_rate_millihertz()) {
            self.frame_time = Duration::from_secs_f64(1000.0 / mhz as f64);
        }
        let font = Font::new(FONT_FAMILY, FONT_SIZE * window.scale_factor() as f32);
        let px = window.inner_size();
        let size = window_size(&font, px.width, px.height);

        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &dims, Listener(self.proxy.clone()))));
        self.pty = Some(Pty::spawn(term.clone(), size, self.proxy.clone()));
        self.term = Some(term);
        self.renderer = Some(Renderer::new(window.clone(), font));
        self.window = Some(window);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
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
            }
            Event::Exit | Event::ChildExit(_) => event_loop.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let (Some(term), Some(renderer), Some(window)) = (&self.term, self.renderer.as_mut(), &self.window) else { return };
        let now = Instant::now();
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
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(px) => {
                renderer.resize(px.width, px.height);
                let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) else { return };
                let size = window_size(&renderer.font, px.width, px.height);
                {
                    term.lock().resize(TermSize::new(size.num_cols as usize, size.num_lines as usize));
                    pty.resize(size);
                }
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
                let term = term.lock();
                let col = ((position.x.max(0.0) as f32 / renderer.font.cell_w) as usize).min(term.columns() - 1);
                let line = ((position.y.max(0.0) as f32 / renderer.font.cell_h) as usize).min(term.screen_lines() - 1);
                if (col, line) == self.mouse_cell {
                    return;
                }
                self.mouse_cell = (col, line);
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
