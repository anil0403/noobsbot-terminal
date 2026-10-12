use winit::keyboard::{Key, ModifiersState, NamedKey};

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

#[cfg(test)]
mod tests {
    use super::*;

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
