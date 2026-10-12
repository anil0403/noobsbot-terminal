mod app;
mod clipboard;
mod config;
mod context;
mod font;
mod input;
mod links;
mod pane;
mod pty;
mod render;
mod rules;
mod search;
mod tap;
mod shell;
mod term;
mod theme;

use winit::event_loop::EventLoop;

fn main() {
    env_logger::init();
    alacritty_terminal::tty::setup_env();
    let event_loop = EventLoop::with_user_event().build().expect("create event loop");
    let mut app = app::App::new(event_loop.create_proxy());
    event_loop.run_app(&mut app).expect("run event loop");
}
