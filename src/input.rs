use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Encode a key press as the bytes a shell expects.
/// ponytail: basic xterm encoding only; modifier combos on special keys and the
/// kitty keyboard protocol arrive in M3.
pub fn key_bytes(key: &Key, text: Option<&str>, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
    let cursor = |c: char| {
        let prefix = if app_cursor { "\x1bO" } else { "\x1b[" };
        Some(format!("{prefix}{c}").into_bytes())
    };
    let bytes = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => b"\r".to_vec(),
            NamedKey::Backspace => b"\x7f".to_vec(),
            NamedKey::Tab if mods.shift_key() => b"\x1b[Z".to_vec(),
            NamedKey::Tab => b"\t".to_vec(),
            NamedKey::Escape => b"\x1b".to_vec(),
            NamedKey::ArrowUp => return cursor('A'),
            NamedKey::ArrowDown => return cursor('B'),
            NamedKey::ArrowRight => return cursor('C'),
            NamedKey::ArrowLeft => return cursor('D'),
            NamedKey::Home => return cursor('H'),
            NamedKey::End => return cursor('F'),
            NamedKey::Insert => b"\x1b[2~".to_vec(),
            NamedKey::Delete => b"\x1b[3~".to_vec(),
            NamedKey::PageUp => b"\x1b[5~".to_vec(),
            NamedKey::PageDown => b"\x1b[6~".to_vec(),
            NamedKey::Space if mods.control_key() => vec![0],
            _ => text?.as_bytes().to_vec(),
        },
        Key::Character(s) if mods.control_key() => {
            let c = s.chars().next()?.to_ascii_lowercase();
            match c {
                'a'..='z' => vec![c as u8 & 0x1f],
                '[' | '3' => vec![0x1b],
                '\\' | '4' => vec![0x1c],
                ']' | '5' => vec![0x1d],
                '/' | '7' => vec![0x1f],
                _ => return None,
            }
        }
        _ => text?.as_bytes().to_vec(),
    };
    if mods.alt_key() && !bytes.is_empty() {
        return Some([b"\x1b".as_slice(), &bytes].concat());
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    #[test]
    fn encodes_keys() {
        let none = ModifiersState::empty();
        assert_eq!(key_bytes(&ch("a"), Some("a"), none, false), Some(b"a".to_vec()));
        assert_eq!(key_bytes(&ch("c"), Some("c"), ModifiersState::CONTROL, false), Some(vec![3]));
        assert_eq!(key_bytes(&ch("x"), Some("x"), ModifiersState::ALT, false), Some(b"\x1bx".to_vec()));
        assert_eq!(key_bytes(&Key::Named(NamedKey::Enter), None, none, false), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&Key::Named(NamedKey::ArrowUp), None, none, false), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(&Key::Named(NamedKey::ArrowUp), None, none, true), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(&Key::Named(NamedKey::Shift), None, none, false), None);
    }
}
