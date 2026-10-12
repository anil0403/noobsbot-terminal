use alacritty_terminal::event::{Event, EventListener};
use winit::event_loop::EventLoopProxy;

/// Forwards terminal events (title, bell, PTY replies, wakeups) to the winit loop.
#[derive(Clone)]
pub struct Listener(pub EventLoopProxy<Event>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.0.send_event(event);
    }
}

pub type Term = alacritty_terminal::Term<Listener>;
