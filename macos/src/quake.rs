//! Quake mode plumbing: the global show/hide hotkey (a Carbon hot key, via
//! global-hotkey; no Accessibility permission needed) and the socket a second
//! `noobsbot --quake` uses to toggle the running one.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::event_loop::EventLoopProxy;

use crate::term::UserEvent;

fn socket_path() -> PathBuf {
    // $TMPDIR is per user on macOS.
    std::env::temp_dir().join(format!("noobsbot-quake-{}.sock", unsafe { libc::getuid() }))
}

/// Ask a running quake terminal to toggle. False: none is running.
pub fn toggle_running() -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => s.write_all(b"toggle").is_ok(),
        Err(_) => false,
    }
}

/// Listen for toggles from later launches.
pub fn serve(proxy: EventLoopProxy<UserEvent>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // stale: nobody answered on it
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => return log::warn!("quake socket {}: {e}", path.display()),
    };
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut msg = String::new();
            if stream.read_to_string(&mut msg).is_ok() && msg == "toggle" {
                let _ = proxy.send_event(UserEvent::ToggleQuake);
            }
        }
    });
}

/// Does `window` have keyboard focus? macOS tells winit reliably, so None:
/// the caller uses winit's focus events.
pub fn has_focus(_window: &winit::window::Window) -> Option<bool> {
    None
}

/// Window opacity 0..=1 (NSWindow alphaValue).
pub fn set_opacity(window: &winit::window::Window, opacity: f32) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };
    // SAFETY: ns_view is the live NSView of this window, used on the main thread.
    unsafe {
        let view: &AnyObject = h.ns_view.cast().as_ref();
        let ns_window: *mut AnyObject = msg_send![view, window];
        if let Some(ns_window) = ns_window.as_ref() {
            let _: () = msg_send![ns_window, setAlphaValue: opacity.clamp(0.0, 1.0) as f64];
        }
    }
}

/// Bring noobsbot to the front and focus `window` (winit activates the app too).
pub fn activate(window: &winit::window::Window) {
    window.focus_window();
}

/// "Ctrl+grave", "Cmd+Shift+t", "F12" → a hot key. Names as on Linux, plus Cmd/Option.
fn parse_hotkey(spec: &str) -> Option<HotKey> {
    let spec: Vec<String> = spec
        .split('+')
        .map(|p| match p.to_ascii_lowercase().as_str() {
            "grave" => "Backquote".to_owned(),
            "super" | "cmd" | "command" => "Super".to_owned(),
            "alt" | "option" => "Alt".to_owned(),
            _ => p.to_owned(),
        })
        .collect();
    spec.join("+").parse().ok()
}

/// Register the hotkey and send a toggle each time it's pressed. Call on the main thread.
pub fn grab(spec: &str, proxy: EventLoopProxy<UserEvent>) {
    let Some(hotkey) = parse_hotkey(spec) else {
        return log::warn!("quake hotkey {spec:?} not understood; try \"F12\" or \"Ctrl+grave\"");
    };
    let manager = match GlobalHotKeyManager::new() {
        Ok(m) => m,
        Err(e) => return log::warn!("quake hotkey: {e}"),
    };
    if let Err(e) = manager.register(hotkey) {
        return log::warn!("quake hotkey {spec}: {e} (already taken by another program?)");
    }
    std::mem::forget(manager); // registered for the life of the program
    let proxy = Mutex::new(proxy);
    GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
        if e.state == HotKeyState::Pressed {
            let _ = proxy.lock().unwrap().send_event(UserEvent::ToggleQuake);
        }
    }));
    log::info!("quake hotkey {spec} registered");
}

#[cfg(test)]
mod tests {
    use super::*;
    use global_hotkey::hotkey::{Code, Modifiers};

    #[test]
    fn hotkeys() {
        assert_eq!(parse_hotkey("F12"), Some(HotKey::new(None, Code::F12)));
        assert_eq!(parse_hotkey("Ctrl+grave"), Some(HotKey::new(Some(Modifiers::CONTROL), Code::Backquote)));
        assert_eq!(parse_hotkey("Cmd+Shift+t"), Some(HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyT)));
        assert_eq!(parse_hotkey("Hyper+F1"), None);
        assert_eq!(parse_hotkey(""), None);
    }
}
