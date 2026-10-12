use std::sync::Arc;

use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::term::test::TermSize;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use crate::input::key_bytes;
use crate::pty::Pty;
use crate::render::Renderer;
use crate::term::{Listener, Term};

const BG: wgpu::Color = wgpu::Color { r: 0.07, g: 0.07, b: 0.09, a: 1.0 };
// Placeholder until M2 measures the real font.
const CELL_W: u32 = 9;
const CELL_H: u32 = 18;

pub struct App {
    proxy: EventLoopProxy<Event>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    term: Option<Arc<FairMutex<Term>>>,
    pty: Option<Pty>,
    mods: ModifiersState,
}

impl App {
    pub fn new(proxy: EventLoopProxy<Event>) -> Self {
        Self { proxy, window: None, renderer: None, term: None, pty: None, mods: ModifiersState::empty() }
    }
}

fn window_size(width: u32, height: u32) -> WindowSize {
    WindowSize {
        num_cols: (width / CELL_W).max(1) as u16,
        num_lines: (height / CELL_H).max(1) as u16,
        cell_width: CELL_W as u16,
        cell_height: CELL_H as u16,
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("noobsbot terminal");
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        let px = window.inner_size();
        let size = window_size(px.width, px.height);

        let dims = TermSize::new(size.num_cols as usize, size.num_lines as usize);
        let term = Arc::new(FairMutex::new(Term::new(Config::default(), &dims, Listener(self.proxy.clone()))));
        self.pty = Some(Pty::spawn(term.clone(), size, self.proxy.clone()));
        self.term = Some(term);
        self.renderer = Some(Renderer::new(window.clone()));
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
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            Event::Exit | Event::ChildExit(_) => event_loop.exit(),
            _ => {}
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(renderer) = self.renderer.as_mut() else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(px) => {
                renderer.resize(px.width, px.height);
                let size = window_size(px.width, px.height);
                if let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) {
                    term.lock().resize(TermSize::new(size.num_cols as usize, size.num_lines as usize));
                    pty.resize(size);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let (Some(term), Some(pty)) = (&self.term, self.pty.as_mut()) else { return };
                let app_cursor = term.lock().mode().contains(TermMode::APP_CURSOR);
                if let Some(bytes) = key_bytes(&event.logical_key, event.text.as_deref(), self.mods, app_cursor) {
                    pty.write(&bytes);
                }
            }
            WindowEvent::RedrawRequested => renderer.render(BG),
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
