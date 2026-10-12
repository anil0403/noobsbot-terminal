mod app;

use winit::event_loop::EventLoop;

fn main() {
    env_logger::init();
    let event_loop = EventLoop::new().expect("create event loop");
    event_loop
        .run_app(&mut app::App::default())
        .expect("run event loop");
}
