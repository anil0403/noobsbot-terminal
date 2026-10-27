use alacritty_terminal::event::{Event, EventListener};
use winit::event_loop::EventLoopProxy;

use crate::tap::Mark;

pub type PaneId = u64;

/// Everything that wakes the winit loop from another thread.
pub enum UserEvent {
    Term(PaneId, Event),
    /// Shell integration mark (not prompt start/end), with the number of the
    /// prompt it belongs to.
    Shell(PaneId, u64, Mark),
    ConfigChanged,
    /// Show or hide the quake window (hotkey or a second launch).
    ToggleQuake,
    /// The Dock menu's "New Tab".
    NewTab,
    /// An action picked in the menu bar.
    Menu(crate::input::Action),
    /// The Dock menu's "Check for Updates": a new tab runs `noobsbot --update`.
    Update,
    /// Quit from the app menu (Cmd+Q): asks first while programs still run.
    Quit,
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
