//! The kitty keyboard protocol (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>):
//! keys as `CSI code ; mods u`, for programs that ask for it with `CSI > flags u`.
//! alacritty_terminal keeps the flags in the terminal's mode; this turns a key
//! into bytes. Programs that don't ask get the legacy bytes (`input::key_bytes`).

use alacritty_terminal::term::TermMode;
use winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Press,
    Repeat,
    Release,
}

/// One key event, as the protocol needs it.
pub struct KeyIn<'a> {
    /// The key as the layout gives it with the modifiers held ("A" with Shift).
    pub key: &'a Key,
    /// The same key with no modifiers ("a").
    pub base: &'a Key,
    pub text: Option<&'a str>,
    pub location: KeyLocation,
    pub mods: ModifiersState,
    pub kind: Kind,
}

#[derive(Debug, PartialEq)]
pub enum Out {
    /// Send the legacy bytes, as without the protocol.
    Legacy,
    Send(Vec<u8>),
    /// Send nothing (a release the program didn't ask for).
    Nothing,
}

/// The bytes for `k` under the protocol `flags` (the terminal's mode).
pub fn encode(k: &KeyIn, flags: TermMode) -> Out {
    let all = flags.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let types = flags.contains(TermMode::REPORT_EVENT_TYPES);
    let release = k.kind == Kind::Release;
    if release && !types {
        return Out::Nothing;
    }
    // A modifier key's own bit: on while it's down, off in its release (the
    // modifiers we're told about change only after the key event).
    let own = match k.key {
        Key::Named(NamedKey::Shift) => ModifiersState::SHIFT,
        Key::Named(NamedKey::Control) => ModifiersState::CONTROL,
        Key::Named(NamedKey::Alt) => ModifiersState::ALT,
        Key::Named(NamedKey::Super) => ModifiersState::SUPER,
        _ => ModifiersState::empty(),
    };
    let held = if release { k.mods - own } else { k.mods | own };
    let m = mods(held);
    let event = match (types, k.kind) {
        (true, Kind::Repeat) => ":2",
        (true, Kind::Release) => ":3",
        _ => "",
    };
    // A press with no modifiers says nothing after the key.
    let plain = m == 1 && event.is_empty();
    let tail = if plain { String::new() } else { format!(";{m}{event}") };
    let csi_u = |code: u32| Out::Send(format!("\x1b[{code}{tail}u").into_bytes());

    let named = match k.key {
        Key::Named(NamedKey::Space) => None, // a text key
        Key::Named(named) => Some(*named),
        _ => None,
    };
    if let Some(named) = named {
        // Arrows, Home, End, F1, F2, F4: CSI 1 ; mods <letter>.
        if let Some(letter) = letter_key(named) {
            return match (plain, all) {
                (true, false) => Out::Legacy,
                (true, true) => Out::Send(format!("\x1b[{letter}").into_bytes()),
                _ => Out::Send(format!("\x1b[1{tail}{letter}").into_bytes()),
            };
        }
        // Insert, Delete, Page Up/Down, F3, F5-F12: CSI n ; mods ~.
        if let Some(n) = tilde_key(named) {
            return match (plain, all) {
                (true, false) => Out::Legacy,
                _ => Out::Send(format!("\x1b[{n}{tail}~").into_bytes()),
            };
        }
        return match named {
            NamedKey::Escape => csi_u(27),
            // These still type as before when plain, so a shell stays usable after a
            // program that set the protocol crashed. Their releases need "all keys".
            NamedKey::Enter | NamedKey::Tab | NamedKey::Backspace => {
                let code = match named {
                    NamedKey::Enter => 13,
                    NamedKey::Tab => 9,
                    _ => 127,
                };
                match (all, plain, release) {
                    (false, _, true) => Out::Nothing,
                    (false, true, _) => Out::Legacy,
                    _ => csi_u(code),
                }
            }
            NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super | NamedKey::CapsLock if all => csi_u(modifier_code(named, k.location)),
            NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super | NamedKey::CapsLock => Out::Nothing,
            _ if release => Out::Nothing,
            _ => Out::Legacy,
        };
    }

    // Text keys: the code is the key with no modifiers, lowercase.
    let Some(code) = char_of(k.base).or_else(|| char_of(k.key)).map(|c| c.to_lowercase().next().unwrap_or(c)) else {
        return if release { Out::Nothing } else { Out::Legacy };
    };
    // Alone or with Shift, a key types its text as before (a release is still reported).
    if !all && (k.mods - ModifiersState::SHIFT).is_empty() && !release {
        return Out::Legacy;
    }
    let mut key = (code as u32).to_string();
    if flags.contains(TermMode::REPORT_ALTERNATE_KEYS)
        && k.mods.shift_key()
        && let Some(shifted) = char_of(k.key).filter(|&c| c != code)
    {
        key.push_str(&format!(":{}", shifted as u32));
    }
    let text = k.text.filter(|t| all && flags.contains(TermMode::REPORT_ASSOCIATED_TEXT) && !release && !t.chars().any(char::is_control));
    Out::Send(
        match text {
            Some(t) => {
                let points: Vec<String> = t.chars().map(|c| (c as u32).to_string()).collect();
                format!("\x1b[{key};{m}{event};{}u", points.join(":"))
            }
            None => format!("\x1b[{key}{tail}u"),
        }
        .into_bytes(),
    )
}

/// 1 + shift | alt << 1 | ctrl << 2 | super << 3, as in xterm.
fn mods(m: ModifiersState) -> u8 {
    1 + m.shift_key() as u8 + ((m.alt_key() as u8) << 1) + ((m.control_key() as u8) << 2) + ((m.super_key() as u8) << 3)
}

fn char_of(key: &Key) -> Option<char> {
    match key {
        Key::Character(s) => s.chars().next(),
        Key::Named(NamedKey::Space) => Some(' '),
        _ => None,
    }
}

fn letter_key(k: NamedKey) -> Option<char> {
    Some(match k {
        NamedKey::ArrowUp => 'A',
        NamedKey::ArrowDown => 'B',
        NamedKey::ArrowRight => 'C',
        NamedKey::ArrowLeft => 'D',
        NamedKey::End => 'F',
        NamedKey::Home => 'H',
        NamedKey::F1 => 'P',
        NamedKey::F2 => 'Q',
        NamedKey::F4 => 'S',
        _ => return None,
    })
}

fn tilde_key(k: NamedKey) -> Option<u8> {
    Some(match k {
        NamedKey::Insert => 2,
        NamedKey::Delete => 3,
        NamedKey::PageUp => 5,
        NamedKey::PageDown => 6,
        NamedKey::F3 => 13, // not CSI R: that is a cursor position report
        NamedKey::F5 => 15,
        NamedKey::F6 => 17,
        NamedKey::F7 => 18,
        NamedKey::F8 => 19,
        NamedKey::F9 => 20,
        NamedKey::F10 => 21,
        NamedKey::F11 => 23,
        NamedKey::F12 => 24,
        _ => return None,
    })
}

/// The protocol's own codes for the modifier keys, left and right.
fn modifier_code(k: NamedKey, at: KeyLocation) -> u32 {
    let right = at == KeyLocation::Right;
    match k {
        NamedKey::CapsLock => 57358,
        NamedKey::Shift => 57441 + 6 * right as u32,
        NamedKey::Control => 57442 + 6 * right as u32,
        NamedKey::Alt => 57443 + 6 * right as u32,
        _ => 57444 + 6 * right as u32, // Super
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISAMBIGUATE: TermMode = TermMode::DISAMBIGUATE_ESC_CODES;

    fn enc(key: Key, base: Key, mods: ModifiersState, kind: Kind, flags: TermMode) -> Out {
        let text = match &key {
            Key::Character(s) => Some(s.to_string()),
            _ => None,
        };
        encode(&KeyIn { key: &key, base: &base, text: text.as_deref(), location: KeyLocation::Standard, mods, kind }, flags)
    }
    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }
    fn send(s: &str) -> Out {
        Out::Send(s.as_bytes().to_vec())
    }
    const NONE: ModifiersState = ModifiersState::empty();
    const SHIFT: ModifiersState = ModifiersState::SHIFT;
    const CTRL: ModifiersState = ModifiersState::CONTROL;

    #[test]
    fn disambiguate() {
        let enter = Key::Named(NamedKey::Enter);
        // Shift+Enter is no longer just Enter; plain Enter types as before.
        assert_eq!(enc(enter.clone(), enter.clone(), SHIFT, Kind::Press, DISAMBIGUATE), send("\x1b[13;2u"));
        assert_eq!(enc(enter.clone(), enter, NONE, Kind::Press, DISAMBIGUATE), Out::Legacy);
        // Ctrl+I is not Tab, Esc is its own key.
        assert_eq!(enc(ch("i"), ch("i"), CTRL, Kind::Press, DISAMBIGUATE), send("\x1b[105;5u"));
        let esc = Key::Named(NamedKey::Escape);
        assert_eq!(enc(esc.clone(), esc, NONE, Kind::Press, DISAMBIGUATE), send("\x1b[27u"));
        // Text alone or with Shift still types.
        assert_eq!(enc(ch("A"), ch("a"), SHIFT, Kind::Press, DISAMBIGUATE), Out::Legacy);
        // Ctrl+Shift+a: the code is the unshifted key.
        assert_eq!(enc(ch("A"), ch("a"), SHIFT | CTRL, Kind::Press, DISAMBIGUATE), send("\x1b[97;6u"));
        // Arrows keep their form; with modifiers CSI 1 ; mods.
        let up = Key::Named(NamedKey::ArrowUp);
        assert_eq!(enc(up.clone(), up.clone(), NONE, Kind::Press, DISAMBIGUATE), Out::Legacy);
        assert_eq!(enc(up.clone(), up, CTRL, Kind::Press, DISAMBIGUATE), send("\x1b[1;5A"));
        let f3 = Key::Named(NamedKey::F3);
        assert_eq!(enc(f3.clone(), f3, SHIFT, Kind::Press, DISAMBIGUATE), send("\x1b[13;2~"));
        // Releases only when asked for.
        assert_eq!(enc(ch("a"), ch("a"), CTRL, Kind::Release, DISAMBIGUATE), Out::Nothing);
    }

    #[test]
    fn event_types() {
        let flags = DISAMBIGUATE | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(enc(ch("a"), ch("a"), CTRL, Kind::Repeat, flags), send("\x1b[97;5:2u"));
        assert_eq!(enc(ch("a"), ch("a"), NONE, Kind::Release, flags), send("\x1b[97;1:3u"));
        let up = Key::Named(NamedKey::ArrowUp);
        assert_eq!(enc(up.clone(), up, NONE, Kind::Release, flags), send("\x1b[1;1:3A"));
        // Enter's release needs "all keys".
        let enter = Key::Named(NamedKey::Enter);
        assert_eq!(enc(enter.clone(), enter, NONE, Kind::Release, flags), Out::Nothing);
    }

    #[test]
    fn all_keys_alternates_and_text() {
        let all = DISAMBIGUATE | TermMode::REPORT_ALL_KEYS_AS_ESC;
        assert_eq!(enc(ch("a"), ch("a"), NONE, Kind::Press, all), send("\x1b[97u"));
        let enter = Key::Named(NamedKey::Enter);
        assert_eq!(enc(enter.clone(), enter, NONE, Kind::Press, all), send("\x1b[13u"));
        let up = Key::Named(NamedKey::ArrowUp);
        assert_eq!(enc(up.clone(), up, NONE, Kind::Press, all), send("\x1b[A"));
        let flags = all | TermMode::REPORT_ALTERNATE_KEYS | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(enc(ch("A"), ch("a"), SHIFT, Kind::Press, flags), send("\x1b[97:65;2;65u"));
        assert_eq!(enc(ch("!"), ch("1"), SHIFT, Kind::Press, flags), send("\x1b[49:33;2;33u"));
        // A modifier key's own bit is set while it's down, cleared in its release.
        let shift = Key::Named(NamedKey::Shift);
        assert_eq!(enc(shift.clone(), shift.clone(), NONE, Kind::Press, all), send("\x1b[57441;2u"));
        let types = all | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(enc(shift.clone(), shift, SHIFT, Kind::Release, types), send("\x1b[57441;1:3u"));
    }
}
