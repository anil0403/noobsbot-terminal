mod app;
mod blocks;
mod clipboard;
mod config;
mod context;
mod font;
mod images;
mod input;
mod links;
mod palette;
mod pane;
mod pty;
mod quake;
mod render;
mod rules;
mod search;
mod shell;
mod tap;
mod term;
mod theme;
mod welcome;

use winit::event_loop::EventLoop;

fn main() {
    env_logger::init();
    let quake = std::env::args().skip(1).any(|a| a == "--quake");
    if quake && quake::toggle_running() {
        return; // the running one shows/hides itself
    }
    alacritty_terminal::tty::setup_env();
    shell::set_term();
    let event_loop = EventLoop::with_user_event().build().expect("create event loop");
    let mut app = app::App::new(event_loop.create_proxy(), quake);
    event_loop.run_app(&mut app).expect("run event loop");
}
