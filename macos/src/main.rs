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
mod restart;
mod rules;
mod search;
mod shell;
mod suggest;
mod tap;
mod term;
mod theme;
mod welcome;

use winit::event_loop::EventLoop;

fn main() {
    env_logger::init();
    match std::env::args().nth(1).as_deref() {
        Some("--update") => std::process::exit(update()),
        Some("--restart") => std::process::exit(restart::cli()),
        _ => {}
    }
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

/// `noobsbot --update`: rerun the installer, which fetches the latest version.
fn update() -> i32 {
    const INSTALL: &str = "curl -fsSL https://github.com/anil0403/noobsbot-terminal/raw/main/macos/install.sh | bash";
    match std::process::Command::new("bash").args(["-o", "pipefail", "-c", INSTALL]).status() {
        Ok(s) if s.success() => {
            println!("Updated. Run `noobsbot --restart` to use the new version.");
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("noobsbot --update needs bash and curl: {e}");
            1
        }
    }
}
