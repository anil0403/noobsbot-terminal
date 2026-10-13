//! Quake mode plumbing: the global show/hide hotkey (X11 key grab) and the
//! socket a second `noobsbot --quake` uses to toggle the running one.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::OnceLock;

use winit::event_loop::EventLoopProxy;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, GrabMode, ModMask, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::term::UserEvent;

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("noobsbot-quake-{}.sock", unsafe { libc::getuid() }))
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
    // Only you can toggle it (it's in /tmp when there's no XDG_RUNTIME_DIR).
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

fn x11_id(window: &winit::window::Window) -> Option<u32> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

/// One connection for focus queries and opacity changes, made on first use.
fn conn() -> Option<&'static (RustConnection, usize)> {
    static CONN: OnceLock<Option<(RustConnection, usize)>> = OnceLock::new();
    CONN.get_or_init(|| x11rb::connect(None).map_err(|e| log::warn!("X11: {e}")).ok()).as_ref()
}

/// Does `window` have keyboard focus, per the X server? Unlike winit's focus
/// events this stays true while our hotkey grab is active. None off X11.
pub fn has_focus(window: &winit::window::Window) -> Option<bool> {
    let id = x11_id(window)?;
    let (conn, _) = conn()?;
    Some(conn.get_input_focus().ok()?.reply().ok()?.focus == id)
}

/// Window opacity 0..=1, applied by the compositor.
pub fn set_opacity(window: &winit::window::Window, opacity: f32) {
    let (Some(id), Some((conn, _))) = (x11_id(window), conn()) else { return };
    let sent = (|| -> Result<(), Box<dyn std::error::Error>> {
        let atom = conn.intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?.reply()?.atom;
        let value = (opacity.clamp(0.0, 1.0) as f64 * u32::MAX as f64) as u32;
        conn.change_property32(PropMode::REPLACE, id, atom, AtomEnum::CARDINAL, &[value])?;
        conn.flush()?;
        Ok(())
    })();
    if let Err(e) = sent {
        log::warn!("window opacity: {e}");
    }
}

/// Ask the window manager to raise and focus `window`. Sent as if from a pager
/// (source 2): plain focus requests from apps are often refused as focus stealing.
pub fn activate(window: &winit::window::Window) {
    let (Some(id), Some((conn, screen))) = (x11_id(window), conn()) else { return window.focus_window() }; // Wayland
    let sent = (|| -> Result<(), Box<dyn std::error::Error>> {
        let root = conn.setup().roots[*screen].root;
        let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
        let event = ClientMessageEvent::new(32, id, atom, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event)?;
        conn.flush()?;
        Ok(())
    })();
    if let Err(e) = sent {
        log::warn!("activate window: {e}");
        window.focus_window();
    }
}

/// "Ctrl+Alt+F12", "Super+grave", "F12" → (modifiers, keysym).
fn parse_hotkey(spec: &str) -> Option<(ModMask, u32)> {
    let mut parts: Vec<&str> = spec.split('+').collect();
    let key = parts.pop()?;
    let mut mods = ModMask::from(0u16);
    for m in parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "ctrl" => ModMask::CONTROL,
            "shift" => ModMask::SHIFT,
            "alt" => ModMask::M1,
            "super" => ModMask::M4,
            _ => return None,
        };
    }
    let keysym = match key.to_ascii_lowercase().as_str() {
        "grave" => 0x60,
        "space" => 0x20,
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => k.as_bytes()[0] as u32,
        k => match k.strip_prefix('f')?.parse::<u32>().ok()? {
            n @ 1..=24 => 0xffbe + n - 1, // XK_F1..
            _ => return None,
        },
    };
    Some((mods, keysym))
}

/// Grab the hotkey on the X server and send a toggle each time it's pressed.
pub fn grab(spec: &str, proxy: EventLoopProxy<UserEvent>) {
    let Some((mods, keysym)) = parse_hotkey(spec) else {
        return log::warn!("quake hotkey {spec:?} not understood; try \"F12\" or \"Ctrl+grave\"");
    };
    let spec = spec.to_owned();
    std::thread::spawn(move || {
        if let Err(e) = grab_loop(mods, keysym, &proxy) {
            log::warn!("quake hotkey {spec}: {e} (bind `noobsbot --quake` in your desktop's shortcuts instead)");
        }
    });
}

fn grab_loop(mods: ModMask, keysym: u32, proxy: &EventLoopProxy<UserEvent>) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = map.keysyms_per_keycode as usize;
    let index = map.keysyms.chunks(per).position(|syms| syms.contains(&keysym)).ok_or("key not on this keyboard")?;
    let keycode = min + index as u8;
    // Also grab with Caps Lock / Num Lock on, or the key does nothing while they are.
    for extra in [0u16, ModMask::LOCK.into(), ModMask::M2.into(), u16::from(ModMask::LOCK) | u16::from(ModMask::M2)] {
        conn.grab_key(true, root, mods | ModMask::from(extra), keycode, GrabMode::ASYNC, GrabMode::ASYNC)?
            .check()
            .map_err(|_| "already taken by another program")?;
    }
    log::info!("quake hotkey grabbed (keycode {keycode})");
    loop {
        if let Event::KeyPress(_) = conn.wait_for_event()? {
            let _ = proxy.send_event(UserEvent::ToggleQuake);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        assert_eq!(parse_hotkey("F12"), Some((ModMask::from(0u16), 0xffc9)));
        assert_eq!(parse_hotkey("Ctrl+grave"), Some((ModMask::CONTROL, 0x60)));
        assert_eq!(parse_hotkey("Super+Shift+t"), Some((ModMask::M4 | ModMask::SHIFT, b't' as u32)));
        assert_eq!(parse_hotkey("Hyper+F1"), None);
        assert_eq!(parse_hotkey("F25"), None);
        assert_eq!(parse_hotkey(""), None);
    }
}
