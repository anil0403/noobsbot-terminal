use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::vi_mode::ViMotion;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::pane::Dir;

/// Terminal-level actions, each listed in [`actions`] with its name and keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    NewTab,
    ClosePane,
    NextTab,
    PrevTab,
    /// Zero-based.
    GoToTab(usize),
    SplitRight,
    SplitDown,
    SplitLeft,
    SplitUp,
    Focus(Dir),
    Resize(Dir),
    /// The focused pane fills the tab, or the splits come back.
    ToggleZoom,
    Search,
    ViMode,
    CommandPalette,
    ToggleCrt,
    /// Scroll to the previous / next command's prompt.
    PrevPrompt,
    NextPrompt,
    /// Copy the last command's output.
    CopyOutput,
    /// A fresh noobsbot in place of this one.
    Restart,
}

/// Keys while vi mode is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vi {
    Motion(ViMotion),
    /// Start a selection of this kind, or stop it if one is already going.
    Select(SelectionType),
    /// Copy the selection and leave vi mode.
    Copy,
    /// Drop the selection, or leave vi mode if there is none.
    Escape,
    Exit,
    Search,
    /// Next older / newer search match.
    SearchPrev,
    SearchNext,
    Top,
    Bottom,
    HalfUp,
    HalfDown,
}

pub fn vi_binding(key: &Key, mods: ModifiersState) -> Option<Vi> {
    use ViMotion as M;
    let ctrl = mods.control_key();
    Some(match key {
        Key::Named(NamedKey::Escape) => Vi::Escape,
        Key::Named(k) => Vi::Motion(match arrow(k)? {
            Dir::Left => M::Left,
            Dir::Right => M::Right,
            Dir::Up => M::Up,
            Dir::Down => M::Down,
        }),
        Key::Character(c) if ctrl => match c.to_lowercase().as_str() {
            "u" => Vi::HalfUp,
            "d" => Vi::HalfDown,
            "v" => Vi::Select(SelectionType::Block),
            "c" => Vi::Escape,
            _ => return None,
        },
        Key::Character(c) => match c.as_str() {
            "h" => Vi::Motion(M::Left),
            "j" => Vi::Motion(M::Down),
            "k" => Vi::Motion(M::Up),
            "l" => Vi::Motion(M::Right),
            "w" => Vi::Motion(M::SemanticRight),
            "b" => Vi::Motion(M::SemanticLeft),
            "e" => Vi::Motion(M::SemanticRightEnd),
            "W" => Vi::Motion(M::WordRight),
            "B" => Vi::Motion(M::WordLeft),
            "E" => Vi::Motion(M::WordRightEnd),
            "0" => Vi::Motion(M::First),
            "$" => Vi::Motion(M::Last),
            "^" => Vi::Motion(M::FirstOccupied),
            "H" => Vi::Motion(M::High),
            "M" => Vi::Motion(M::Middle),
            "L" => Vi::Motion(M::Low),
            "%" => Vi::Motion(M::Bracket),
            "{" => Vi::Motion(M::ParagraphUp),
            "}" => Vi::Motion(M::ParagraphDown),
            "g" => Vi::Top,
            "G" => Vi::Bottom,
            "v" => Vi::Select(SelectionType::Simple),
            "V" => Vi::Select(SelectionType::Lines),
            "y" => Vi::Copy,
            "i" | "q" => Vi::Exit,
            "/" | "?" => Vi::Search,
            "n" => Vi::SearchPrev,
            "N" => Vi::SearchNext,
            _ => return None,
        },
        _ => return None,
    })
}

fn arrow(key: &NamedKey) -> Option<Dir> {
    Some(match key {
        NamedKey::ArrowLeft => Dir::Left,
        NamedKey::ArrowRight => Dir::Right,
        NamedKey::ArrowUp => Dir::Up,
        NamedKey::ArrowDown => Dir::Down,
        _ => return None,
    })
}

pub struct Entry {
    pub action: Action,
    /// What the command palette shows.
    pub name: String,
    /// The name in the config's `[keys]`: `name` in snake_case ("split_right").
    pub id: String,
    /// Default keys, like "Ctrl+Shift+T"; alternatives separated by " or ".
    pub keys: String,
}

/// Keys from the config's `[keys]`, by action id, in place of the defaults.
static USER_KEYS: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Check the config's `[keys]` (action ids and key specs).
pub fn check_keys(keys: &HashMap<String, String>) -> Result<(), String> {
    for (id, spec) in keys {
        if !actions().iter().any(|e| e.id == *id) {
            return Err(format!("[keys]: no action called `{id}`"));
        }
        if let Some(bad) = spec.split(" or ").find(|s| !s.is_empty() && parse(s).is_none()) {
            return Err(format!("[keys]: can't read `{bad}` for `{id}` (like \"Ctrl+Shift+T\")"));
        }
    }
    Ok(())
}

/// Use the config's `[keys]` from now on.
pub fn set_keys(keys: &HashMap<String, String>) {
    *USER_KEYS.write().unwrap() = Some(keys.clone());
}

/// An action's keys: the config's if it sets them, else the default.
pub fn keys(e: &Entry) -> String {
    USER_KEYS.read().unwrap().as_ref().and_then(|k| k.get(&e.id).cloned()).unwrap_or_else(|| e.keys.clone())
}

/// Every action, registered once: name for the palette and default keys.
pub fn actions() -> &'static [Entry] {
    static ACTIONS: OnceLock<Vec<Entry>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        use Action as A;
        let mut list: Vec<(Action, String, String)> = [
            (A::CommandPalette, "Command palette", "Ctrl+Shift+P"),
            (A::Copy, "Copy", "Ctrl+Shift+C"),
            (A::Paste, "Paste", "Ctrl+Shift+V or Shift+Insert"),
            (A::CopyOutput, "Copy last command's output", "Ctrl+Shift+G"),
            (A::Search, "Search", "Ctrl+Shift+F"),
            (A::ViMode, "Vi mode", "Ctrl+Shift+Space"),
            (A::PrevPrompt, "Previous prompt", "Ctrl+Shift+Z"),
            (A::NextPrompt, "Next prompt", "Ctrl+Shift+X"),
            (A::NewTab, "New tab", "Ctrl+Shift+T"),
            (A::SplitRight, "Split right", "Ctrl+Shift+E"),
            (A::SplitDown, "Split down", "Ctrl+Shift+O"),
            (A::SplitLeft, "Split left", ""),
            (A::SplitUp, "Split up", ""),
            (A::ClosePane, "Close pane", "Ctrl+Shift+W"),
            (A::ToggleZoom, "Zoom pane", "Ctrl+Shift+Enter"),
            (A::NextTab, "Next tab", "Ctrl+Tab"),
            (A::PrevTab, "Previous tab", "Ctrl+Shift+Tab"),
            (A::ScrollPageUp, "Scroll page up", "Shift+PageUp"),
            (A::ScrollPageDown, "Scroll page down", "Shift+PageDown"),
            (A::ScrollTop, "Scroll to top", "Ctrl+Shift+Home"),
            (A::ScrollBottom, "Scroll to bottom", "Ctrl+Shift+End"),
            (A::ZoomIn, "Bigger font", "Ctrl+= or Ctrl+Shift++"),
            (A::ZoomOut, "Smaller font", "Ctrl+-"),
            (A::ZoomReset, "Reset font size", "Ctrl+0"),
            (A::ToggleCrt, "Toggle CRT effect", ""),
            (A::Restart, "Restart noobsbot", ""),
        ]
        .into_iter()
        .map(|(a, n, k)| (a, n.to_owned(), k.to_owned()))
        .collect();
        for (dir, name) in [(Dir::Left, "Left"), (Dir::Right, "Right"), (Dir::Up, "Up"), (Dir::Down, "Down")] {
            list.push((A::Focus(dir), format!("Focus pane {}", name.to_lowercase()), format!("Alt+{name}")));
            list.push((A::Resize(dir), format!("Resize split {}", name.to_lowercase()), format!("Ctrl+Shift+{name}")));
        }
        for i in 0..9 {
            list.push((A::GoToTab(i), format!("Go to tab {}", i + 1), format!("Alt+{}", i + 1)));
        }
        list.into_iter()
            .map(|(action, name, keys)| {
                let id = name.to_lowercase().replace(' ', "_").chars().filter(|c| c.is_alphanumeric() || *c == '_').collect();
                Entry { action, name, id, keys }
            })
            .collect()
    })
}

/// The action bound to a key, checked before the key goes to the shell.
pub fn binding(key: &Key, mods: ModifiersState) -> Option<Action> {
    binding_in(USER_KEYS.read().unwrap().as_ref().unwrap_or(&HashMap::new()), key, mods)
}

/// With `user` keys in place of the defaults. Keys the user set win over a
/// default on the same keys.
fn binding_in(user: &HashMap<String, String>, key: &Key, mods: ModifiersState) -> Option<Action> {
    let key = match key {
        Key::Character(c) => Key::Character(c.to_lowercase().into()),
        k => k.clone(),
    };
    let hit = |keys: &str| keys.split(" or ").any(|spec| parse(spec).is_some_and(|(want, k)| mods == want && k == key));
    let mine = actions().iter().find(|e| user.get(&e.id).is_some_and(|k| hit(k)));
    mine.or_else(|| actions().iter().find(|e| !user.contains_key(&e.id) && hit(&e.keys))).map(|e| e.action)
}

/// A key spec like "Ctrl+Shift+T" as modifiers and key; None if it isn't one
/// (or is empty: palette only).
fn parse(spec: &str) -> Option<(ModifiersState, Key)> {
    let (prefix, name) = match spec.strip_suffix("++") {
        Some(prefix) => (prefix, "+"),
        None => spec.rsplit_once('+').unwrap_or(("", spec)),
    };
    let mut want = ModifiersState::empty();
    for m in prefix.split('+').filter(|m| !m.is_empty()) {
        want |= match m {
            "Ctrl" => ModifiersState::CONTROL,
            "Shift" => ModifiersState::SHIFT,
            "Alt" => ModifiersState::ALT,
            "Super" => ModifiersState::SUPER,
            _ => return None,
        };
    }
    let named = match name {
        "PageUp" => NamedKey::PageUp,
        "PageDown" => NamedKey::PageDown,
        "Home" => NamedKey::Home,
        "End" => NamedKey::End,
        "Insert" => NamedKey::Insert,
        "Tab" => NamedKey::Tab,
        "Space" => NamedKey::Space,
        "Enter" => NamedKey::Enter,
        "Left" => NamedKey::ArrowLeft,
        "Right" => NamedKey::ArrowRight,
        "Up" => NamedKey::ArrowUp,
        "Down" => NamedKey::ArrowDown,
        "Escape" => NamedKey::Escape,
        "Backspace" => NamedKey::Backspace,
        "Delete" => NamedKey::Delete,
        "F1" => NamedKey::F1,
        "F2" => NamedKey::F2,
        "F3" => NamedKey::F3,
        "F4" => NamedKey::F4,
        "F5" => NamedKey::F5,
        "F6" => NamedKey::F6,
        "F7" => NamedKey::F7,
        "F8" => NamedKey::F8,
        "F9" => NamedKey::F9,
        "F10" => NamedKey::F10,
        "F11" => NamedKey::F11,
        "F12" => NamedKey::F12,
        // One character; letters match either case ("T" is what Shift+t sends).
        _ if name.chars().count() == 1 => return Some((want, Key::Character(name.to_lowercase().into()))),
        _ => return None,
    };
    Some((want, Key::Named(named)))
}

/// xterm modifier parameter: 1 + shift|alt<<1|ctrl<<2|super<<3. 1 means "no modifiers".
fn mod_param(mods: ModifiersState) -> u8 {
    1 + mods.shift_key() as u8 + ((mods.alt_key() as u8) << 1) + ((mods.control_key() as u8) << 2) + ((mods.super_key() as u8) << 3)
}

/// Encode a key press as the bytes a shell expects (xterm conventions).
/// ponytail: no kitty keyboard protocol or keypad application mode yet.
pub fn key_bytes(key: &Key, text: Option<&str>, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    let m = mod_param(mods);
    // Cursor-style keys: CSI <c>, SS3 <c> in app-cursor mode, CSI 1;m <c> with modifiers.
    let letter = |c: char, ss3_default: bool| {
        Some(
            match m {
                1 if ss3_default || app_cursor => format!("\x1bO{c}"),
                1 => format!("\x1b[{c}"),
                m => format!("\x1b[1;{m}{c}"),
            }
            .into_bytes(),
        )
    };
    // Tilde keys: CSI n ~ or CSI n;m ~.
    let tilde = |n: u8| {
        Some(
            match m {
                1 => format!("\x1b[{n}~"),
                m => format!("\x1b[{n};{m}~"),
            }
            .into_bytes(),
        )
    };
    let alt = |bytes: &[u8]| {
        let prefix: &[u8] = if mods.alt_key() { b"\x1b" } else { b"" };
        Some([prefix, bytes].concat())
    };

    match key {
        Key::Named(named) => match named {
            NamedKey::ArrowUp => letter('A', false),
            NamedKey::ArrowDown => letter('B', false),
            NamedKey::ArrowRight => letter('C', false),
            NamedKey::ArrowLeft => letter('D', false),
            NamedKey::Home => letter('H', false),
            NamedKey::End => letter('F', false),
            NamedKey::F1 => letter('P', true),
            NamedKey::F2 => letter('Q', true),
            NamedKey::F3 => letter('R', true),
            NamedKey::F4 => letter('S', true),
            NamedKey::F5 => tilde(15),
            NamedKey::F6 => tilde(17),
            NamedKey::F7 => tilde(18),
            NamedKey::F8 => tilde(19),
            NamedKey::F9 => tilde(20),
            NamedKey::F10 => tilde(21),
            NamedKey::F11 => tilde(23),
            NamedKey::F12 => tilde(24),
            NamedKey::Insert => tilde(2),
            NamedKey::Delete => tilde(3),
            NamedKey::PageUp => tilde(5),
            NamedKey::PageDown => tilde(6),
            NamedKey::Enter => alt(b"\r"),
            NamedKey::Backspace if mods.control_key() => alt(b"\x08"),
            NamedKey::Backspace => alt(b"\x7f"),
            NamedKey::Tab if mods.shift_key() => Some(b"\x1b[Z".to_vec()),
            NamedKey::Tab => alt(b"\t"),
            NamedKey::Escape => alt(b"\x1b"),
            NamedKey::Space if mods.control_key() => alt(b"\0"),
            _ => alt(text?.as_bytes()),
        },
        Key::Character(s) if mods.control_key() => {
            let c = s.chars().next()?.to_ascii_lowercase();
            let byte = match c {
                'a'..='z' => c as u8 & 0x1f,
                '@' | '2' | ' ' => 0x00,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '/' | '7' => 0x1f,
                '8' | '?' => 0x7f,
                _ => return None,
            };
            alt(&[byte])
        }
        _ => alt(text?.as_bytes()),
    }
}

/// A path as a shell word: as is when it's plain, else in single quotes
/// ("my notes.txt" -> 'my notes.txt', it's -> 'it'\''s').
pub fn shell_quote(path: &str) -> String {
    if !path.is_empty() && path.chars().all(|c| c.is_alphanumeric() || "/._-+,:@%=~".contains(c)) && !path.starts_with('~') {
        return path.to_owned();
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Bytes for pasted text. With bracketed paste the app sees it as a paste, not typing,
/// so a pasted newline can't run a command by itself.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Strip ESC so the text can't close the bracket early and inject keystrokes.
        let clean = text.replace('\x1b', "");
        return [b"\x1b[200~", clean.as_bytes(), b"\x1b[201~"].concat();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

pub const MOUSE_LEFT: u8 = 0;
pub const MOUSE_MIDDLE: u8 = 1;
pub const MOUSE_RIGHT: u8 = 2;
pub const MOUSE_RELEASE: u8 = 3; // legacy encoding has no per-button release
pub const MOUSE_WHEEL_UP: u8 = 64;
pub const MOUSE_WHEEL_DOWN: u8 = 65;
pub const MOUSE_MOTION: u8 = 32;

/// Encode a mouse event for apps that enabled mouse reporting.
/// `col`/`line` are 0-based viewport cells. `sgr` selects the modern SGR 1006 format.
pub fn mouse_report(button: u8, pressed: bool, col: usize, line: usize, mods: ModifiersState, sgr: bool) -> Option<Vec<u8>> {
    let code = button + 4 * mods.shift_key() as u8 + 8 * mods.alt_key() as u8 + 16 * mods.control_key() as u8;
    if sgr {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, line + 1).into_bytes());
    }
    // X10/normal encoding: values offset by 32 in a single byte, so coordinates max out at 222.
    if col > 222 || line > 222 {
        return None;
    }
    let code = if pressed { code } else { MOUSE_RELEASE + (code & !3) };
    Some(vec![0x1b, b'[', b'M', 32 + code, 33 + col as u8, 33 + line as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_tab_keys() {
        let alt = ModifiersState::ALT;
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(binding(&ch("3"), alt), Some(Action::GoToTab(2)));
        assert_eq!(binding(&ch("0"), alt), None);
        assert_eq!(binding(&ch("E"), ctrl_shift), Some(Action::SplitRight));
        assert_eq!(binding(&Key::Named(NamedKey::Tab), ctrl_shift), Some(Action::PrevTab));
        assert_eq!(binding(&Key::Named(NamedKey::ArrowUp), alt), Some(Action::Focus(Dir::Up)));
        assert_eq!(binding(&Key::Named(NamedKey::ArrowUp), ModifiersState::CONTROL), None);
        assert_eq!(binding(&ch("+"), ctrl_shift), Some(Action::ZoomIn));
        assert_eq!(binding(&ch("="), ModifiersState::CONTROL), Some(Action::ZoomIn));
        assert_eq!(binding(&ch("-"), ModifiersState::CONTROL), Some(Action::ZoomOut));
        assert_eq!(binding(&Key::Named(NamedKey::Insert), ModifiersState::SHIFT), Some(Action::Paste));
        assert_eq!(binding(&Key::Named(NamedKey::Space), ctrl_shift), Some(Action::ViMode));
        assert_eq!(binding(&ch("t"), ModifiersState::CONTROL), None);
        assert_eq!(binding(&Key::Named(NamedKey::Enter), ctrl_shift), Some(Action::ToggleZoom));
        assert_eq!(binding(&Key::Named(NamedKey::Enter), NONE), None);
    }

    #[test]
    fn user_keys() {
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        let user: HashMap<String, String> = [("split_right", "Ctrl+Shift+T or Super+F5"), ("close_pane", "")].map(|(a, k)| (a.into(), k.into())).into();
        assert!(check_keys(&user).is_ok());
        // Theirs beat the default on the same keys; the replaced default is gone.
        assert_eq!(binding_in(&user, &ch("T"), ctrl_shift), Some(Action::SplitRight));
        assert_eq!(binding_in(&user, &Key::Named(NamedKey::F5), ModifiersState::SUPER), Some(Action::SplitRight));
        assert_eq!(binding_in(&user, &ch("E"), ctrl_shift), None);
        assert_eq!(binding_in(&user, &ch("W"), ctrl_shift), None);
        assert_eq!(binding_in(&user, &ch("P"), ctrl_shift), Some(Action::CommandPalette));
        assert!(actions().iter().any(|e| e.id == "copy_last_commands_output"));
        let bad = |a: &str, k: &str| check_keys(&[(a.to_owned(), k.to_owned())].into()).is_err();
        assert!(bad("split_sideways", "Ctrl+E"));
        assert!(bad("split_right", "Ctrl+Hyper+E"));
        assert!(bad("split_right", "Ctrl+Esc"));
    }

    #[test]
    fn binds_vi_keys() {
        assert_eq!(vi_binding(&ch("G"), ModifiersState::SHIFT), Some(Vi::Bottom));
        assert_eq!(vi_binding(&ch("w"), NONE), Some(Vi::Motion(ViMotion::SemanticRight)));
        assert_eq!(vi_binding(&ch("v"), ModifiersState::CONTROL), Some(Vi::Select(SelectionType::Block)));
        assert_eq!(vi_binding(&Key::Named(NamedKey::ArrowDown), NONE), Some(Vi::Motion(ViMotion::Down)));
        assert_eq!(vi_binding(&ch("z"), NONE), None);
    }

    #[test]
    fn encodes_paste() {
        assert_eq!(shell_quote("/home/me/src/main.rs"), "/home/me/src/main.rs");
        assert_eq!(shell_quote("/tmp/my notes.txt"), "'/tmp/my notes.txt'");
        assert_eq!(shell_quote("/tmp/it's $HOME;rm"), "'/tmp/it'\\''s $HOME;rm'");
        assert_eq!(shell_quote("~literal"), "'~literal'");
        assert_eq!(shell_quote("/tmp/café.png"), "/tmp/café.png");
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(paste_bytes("ls\n", true), b"\x1b[200~ls\n\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~rm -rf ~\n", true), b"\x1b[200~x[201~rm -rf ~\n\x1b[201~");
    }

    #[test]
    fn encodes_mouse() {
        assert_eq!(mouse_report(MOUSE_LEFT, true, 0, 0, NONE, true).unwrap(), b"\x1b[<0;1;1M");
        assert_eq!(mouse_report(MOUSE_LEFT, false, 4, 9, NONE, true).unwrap(), b"\x1b[<0;5;10m");
        assert_eq!(mouse_report(MOUSE_WHEEL_UP, true, 1, 1, ModifiersState::CONTROL, true).unwrap(), b"\x1b[<80;2;2M");
        assert_eq!(mouse_report(MOUSE_RIGHT, true, 0, 0, NONE, false).unwrap(), b"\x1b[M\"!!");
        assert_eq!(mouse_report(MOUSE_RIGHT, false, 0, 0, NONE, false).unwrap(), b"\x1b[M#!!");
        assert_eq!(mouse_report(MOUSE_LEFT, true, 300, 0, NONE, false), None);
    }

    const NONE: ModifiersState = ModifiersState::empty();

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    fn named(k: NamedKey, mods: ModifiersState) -> Vec<u8> {
        key_bytes(&Key::Named(k), None, mods, false).unwrap()
    }

    #[test]
    fn encodes_text_and_control() {
        assert_eq!(key_bytes(&ch("a"), Some("a"), NONE, false), Some(b"a".to_vec()));
        assert_eq!(key_bytes(&ch("c"), Some("c"), ModifiersState::CONTROL, false), Some(vec![3]));
        assert_eq!(key_bytes(&ch("x"), Some("x"), ModifiersState::ALT, false), Some(b"\x1bx".to_vec()));
        assert_eq!(key_bytes(&ch("é"), Some("é"), NONE, false), Some("é".as_bytes().to_vec()));
        assert_eq!(key_bytes(&Key::Named(NamedKey::Shift), None, NONE, false), None);
    }

    #[test]
    fn encodes_special_keys() {
        assert_eq!(named(NamedKey::Enter, NONE), b"\r");
        assert_eq!(named(NamedKey::ArrowUp, NONE), b"\x1b[A");
        assert_eq!(key_bytes(&Key::Named(NamedKey::ArrowUp), None, NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(named(NamedKey::ArrowLeft, ModifiersState::CONTROL), b"\x1b[1;5D");
        assert_eq!(named(NamedKey::ArrowRight, ModifiersState::SHIFT | ModifiersState::ALT), b"\x1b[1;4C");
        assert_eq!(named(NamedKey::F1, NONE), b"\x1bOP");
        assert_eq!(named(NamedKey::F1, ModifiersState::SHIFT), b"\x1b[1;2P");
        assert_eq!(named(NamedKey::F5, NONE), b"\x1b[15~");
        assert_eq!(named(NamedKey::Delete, ModifiersState::CONTROL), b"\x1b[3;5~");
        assert_eq!(named(NamedKey::Backspace, ModifiersState::ALT), b"\x1b\x7f");
        assert_eq!(named(NamedKey::Backspace, ModifiersState::CONTROL), b"\x08");
        assert_eq!(named(NamedKey::Tab, ModifiersState::SHIFT), b"\x1b[Z");
    }
}
