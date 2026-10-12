use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::vi_mode::ViMotion;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use crate::pane::Dir;

/// Terminal-level actions. M9's command palette lists these.
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
    Focus(Dir),
    Resize(Dir),
    Search,
    ViMode,
    /// Scroll to the previous / next command's prompt.
    PrevPrompt,
    NextPrompt,
    /// Copy the last command's output.
    CopyOutput,
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

/// Default keybindings. Checked before a key is sent to the shell.
pub fn binding(key: &Key, mods: ModifiersState) -> Option<Action> {
    let shift = ModifiersState::SHIFT;
    let ctrl = ModifiersState::CONTROL;
    let alt = ModifiersState::ALT;
    let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
    let letter = |c: &str, l: &str| c.eq_ignore_ascii_case(l);
    Some(match (key, mods) {
        (Key::Named(NamedKey::PageUp), m) if m == shift => Action::ScrollPageUp,
        (Key::Named(NamedKey::PageDown), m) if m == shift => Action::ScrollPageDown,
        (Key::Named(NamedKey::Home), m) if m == ctrl_shift => Action::ScrollTop,
        (Key::Named(NamedKey::End), m) if m == ctrl_shift => Action::ScrollBottom,
        (Key::Character(c), m) if m == ctrl_shift && c.eq_ignore_ascii_case("c") => Action::Copy,
        (Key::Character(c), m) if m == ctrl_shift && c.eq_ignore_ascii_case("v") => Action::Paste,
        (Key::Named(NamedKey::Insert), m) if m == shift => Action::Paste,
        (Key::Character(c), m) if (m == ctrl && c == "=") || (m == ctrl_shift && c == "+") => Action::ZoomIn,
        (Key::Character(c), m) if m == ctrl && c == "-" => Action::ZoomOut,
        (Key::Character(c), m) if m == ctrl && c == "0" => Action::ZoomReset,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "t") => Action::NewTab,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "w") => Action::ClosePane,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "e") => Action::SplitRight,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "o") => Action::SplitDown,
        (Key::Named(NamedKey::Tab), m) if m == ctrl => Action::NextTab,
        (Key::Named(NamedKey::Tab), m) if m == ctrl_shift => Action::PrevTab,
        (Key::Character(c), m) if m == alt && c.len() == 1 && ("1"..="9").contains(&c.as_str()) => {
            Action::GoToTab(c.parse::<usize>().unwrap() - 1)
        }
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "f") => Action::Search,
        (Key::Named(NamedKey::Space), m) if m == ctrl_shift => Action::ViMode,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "z") => Action::PrevPrompt,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "x") => Action::NextPrompt,
        (Key::Character(c), m) if m == ctrl_shift && letter(c, "g") => Action::CopyOutput,
        (Key::Named(k), m) if m == alt && arrow(k).is_some() => Action::Focus(arrow(k).unwrap()),
        (Key::Named(k), m) if m == ctrl_shift && arrow(k).is_some() => Action::Resize(arrow(k).unwrap()),
        _ => return None,
    })
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
        Some(match m {
            1 if ss3_default || app_cursor => format!("\x1bO{c}"),
            1 => format!("\x1b[{c}"),
            m => format!("\x1b[1;{m}{c}"),
        }.into_bytes())
    };
    // Tilde keys: CSI n ~ or CSI n;m ~.
    let tilde = |n: u8| {
        Some(match m {
            1 => format!("\x1b[{n}~"),
            m => format!("\x1b[{n};{m}~"),
        }.into_bytes())
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
