use alacritty_terminal::event::{Event, EventListener};
use winit::event_loop::EventLoopProxy;

pub type PaneId = u64;

/// Everything that wakes the winit loop from another thread.
pub enum UserEvent {
    Term(PaneId, Event),
    ConfigChanged,
}

/// Forwards terminal events (title, bell, PTY replies, wakeups) to the winit loop.
#[derive(Clone)]
pub struct Listener {
    pub proxy: EventLoopProxy<UserEvent>,
    pub pane: PaneId,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.proxy.send_event(UserEvent::Term(self.pane, event));
    }
}

pub type Term = alacritty_terminal::Term<Listener>;
