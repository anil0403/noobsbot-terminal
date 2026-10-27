//! Watches the PTY byte stream for escape codes alacritty_terminal ignores:
//! OSC 133 (prompt and command marks from shell integration), OSC 7 (cwd),
//! notifications (OSC 9, 777, 99) and progress (OSC 9;4),
//! images: kitty graphics (APC `G`) and sixel (DCS `q`); and XTVERSION (CSI `> q`).

use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Mark {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandEnd(Option<i32>),
    Cwd(PathBuf),
    /// Kitty graphics command: everything after the `G`.
    Graphics(Vec<u8>),
    /// Sixel image: parameters, `q`, data.
    Sixel(Vec<u8>),
    /// XTVERSION (`CSI > q`): the app asks which terminal this is.
    XtVersion,
    /// A desktop notification a program asks for (OSC 9, 777, 99).
    Notify {
        title: String,
        body: String,
    },
    /// Progress a program reports (OSC 9;4); `None` takes it away.
    Progress(Option<Progress>),
}

/// OSC 9;4 states, with the percent where there is one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Normal(u8),
    Error(Option<u8>),
    /// Working, no percent.
    Busy,
    Paused(Option<u8>),
}

/// A kitty notification (OSC 99) arriving in pieces, until the one marked done.
#[derive(Default)]
struct Note {
    id: String,
    title: String,
    body: String,
}

/// Longest notification text we keep.
const MAX_NOTE: usize = 500;

/// Longest OSC we bother to collect; longer ones aren't ours.
const MAX: usize = 4096;
/// Longest image sequence we collect.
const MAX_IMAGE: usize = 64 << 20;
/// CSI parameters kept; ours are at most `>0`.
const MAX_CSI: usize = 8;

#[derive(Default)]
pub struct Tap {
    state: State,
    kind: Kind,
    osc: Vec<u8>,
    /// A CSI's parameter and intermediate bytes so far.
    csi: Vec<u8>,
    note: Option<Note>,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Str,
    /// ESC inside a string ends it; a following `\` belongs to it too.
    StrEnd,
    Csi,
}

/// Which kind of string sequence is being collected.
#[derive(Default, Clone, Copy, PartialEq)]
enum Kind {
    #[default]
    Osc,
    Apc,
    Dcs,
}

impl Tap {
    /// Scan a chunk. Each mark comes with the offset just past its sequence, so
    /// the caller can feed the parser up to there before acting on it.
    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<(usize, Mark)>) {
        let mut i = 0;
        while i < bytes.len() {
            // Runs that can't change the state are skipped (text) or taken (a string's body) whole.
            match self.state {
                State::Ground => match bytes[i..].iter().position(|&b| b == 0x1b) {
                    Some(n) => i += n,
                    None => return,
                },
                State::Str => {
                    let osc = self.kind == Kind::Osc;
                    let n = bytes[i..].iter().position(|&b| b == 0x1b || osc && b == 0x07).unwrap_or(bytes.len() - i);
                    let room = (if osc { MAX } else { MAX_IMAGE } + 1).saturating_sub(self.osc.len());
                    self.osc.extend_from_slice(&bytes[i..i + n.min(room)]);
                    i += n;
                    if i == bytes.len() {
                        return;
                    }
                }
                _ => {}
            }
            let b = bytes[i];
            i += 1;
            if self.state == State::StrEnd {
                self.state = State::Esc;
                if b == b'\\' {
                    self.finish(i, out);
                    self.state = State::Ground;
                    continue;
                }
                self.finish(i - 1, out);
            }
            self.state = match (self.state, b) {
                (State::Str, 0x07) => {
                    self.finish(i, out);
                    State::Ground
                }
                (State::Str, _) => State::StrEnd, // ESC: the body was taken above
                (State::Esc, b']' | b'_' | b'P') => {
                    self.osc.clear();
                    self.kind = match b {
                        b']' => Kind::Osc,
                        b'_' => Kind::Apc,
                        _ => Kind::Dcs,
                    };
                    State::Str
                }
                (State::Esc, b'[') => {
                    self.csi.clear();
                    State::Csi
                }
                (State::Csi, 0x20..=0x3f) => {
                    if self.csi.len() < MAX_CSI {
                        self.csi.push(b);
                    }
                    State::Csi
                }
                (State::Csi, b'q') => {
                    if matches!(&self.csi[..], b">" | b">0") {
                        out.push((i, Mark::XtVersion));
                    }
                    State::Ground
                }
                // C0 controls inside a CSI run without ending it.
                (State::Csi, 0x00..=0x1a | 0x1c..=0x1f) => State::Csi,
                (_, 0x1b) => State::Esc,
                _ => State::Ground,
            };
        }
    }

    /// OSC 99 `metadata;payload`: `i` id, `d=0` more to come, `p` title or body,
    /// `e=1` base64. Pieces with the same id add up until one is done.
    fn kitty_note(&mut self, osc: &[u8]) -> Option<Mark> {
        use base64::Engine;
        let osc = std::str::from_utf8(osc).ok()?;
        let (meta, payload) = osc.split_once(';')?;
        let (mut id, mut done, mut part, mut b64) = ("", true, "title", false);
        for kv in meta.split(':') {
            match kv.split_once('=') {
                Some(("i", v)) => id = v,
                Some(("d", v)) => done = v != "0",
                Some(("p", v)) => part = v,
                Some(("e", v)) => b64 = v == "1",
                _ => {}
            }
        }
        let text = match b64 {
            true => String::from_utf8_lossy(&base64::engine::general_purpose::STANDARD.decode(payload).ok()?).into_owned(),
            false => payload.to_owned(),
        };
        if self.note.as_ref().is_none_or(|n| n.id != id) {
            self.note = Some(Note { id: id.to_owned(), ..Default::default() });
        }
        let note = self.note.as_mut()?;
        match part {
            "title" if note.title.len() < MAX_NOTE => note.title.push_str(&text),
            "body" if note.body.len() < MAX_NOTE => note.body.push_str(&text),
            _ => {}
        }
        if !done {
            return None;
        }
        let note = self.note.take()?;
        let (title, body) = (clean(&note.title), clean(&note.body));
        (!title.is_empty() || !body.is_empty()).then_some(Mark::Notify { title, body })
    }

    fn finish(&mut self, end: usize, out: &mut Vec<(usize, Mark)>) {
        let s = std::mem::take(&mut self.osc);
        let mark = match self.kind {
            _ if s.len() > MAX_IMAGE => None,
            Kind::Osc if s.len() > MAX => None, // cut short, so not what it says
            Kind::Osc if s.starts_with(b"99;") => self.kitty_note(&s[3..]),
            Kind::Osc => parse(&s),
            Kind::Apc => s.strip_prefix(b"G").map(|g| Mark::Graphics(g.to_vec())),
            // `ESC P 0;1;0 q data`: digits and `;`, then `q`.
            Kind::Dcs => {
                let q = s.iter().position(|&b| !(b.is_ascii_digit() || b == b';'));
                (q.map(|q| s[q]) == Some(b'q')).then_some(Mark::Sixel(s))
            }
        };
        if let Some(mark) = mark {
            out.push((end, mark));
        }
    }
}

fn parse(osc: &[u8]) -> Option<Mark> {
    let osc = std::str::from_utf8(osc).ok()?;
    // urxvt / VTE: `777;notify;title;body`.
    if let Some(rest) = osc.strip_prefix("777;notify;") {
        let (title, body) = rest.split_once(';').unwrap_or((rest, ""));
        return Some(Mark::Notify { title: clean(title), body: clean(body) });
    }
    if let Some(rest) = osc.strip_prefix("9;") {
        // ConEmu: `9;4;state;percent` is progress; its other `9;<n>;` commands aren't ours.
        if let Some(progress) = rest.strip_prefix("4;") {
            return Some(Mark::Progress(parse_progress(progress)));
        }
        let (n, _) = rest.split_once(';').unwrap_or((rest, ""));
        if rest.contains(';') && matches!(n, "1" | "2" | "3" | "5" | "6" | "7" | "8" | "9" | "10" | "11" | "12") {
            return None;
        }
        // iTerm2: `9;text`, the whole text is the message.
        let body = clean(rest);
        return (!body.is_empty()).then_some(Mark::Notify { title: String::new(), body });
    }
    if let Some(rest) = osc.strip_prefix("133;") {
        let mut parts = rest.split(';');
        return match parts.next()? {
            "A" => Some(Mark::PromptStart),
            "B" => Some(Mark::PromptEnd),
            "C" => Some(Mark::CommandStart),
            "D" => Some(Mark::CommandEnd(parts.next().and_then(|c| c.parse().ok()))),
            _ => None,
        };
    }
    let url = osc.strip_prefix("7;")?;
    // file://host/path → /path; the host is ours unless you're ssh'd somewhere.
    let path = match url.strip_prefix("file://") {
        Some(rest) => &rest[rest.find('/')?..],
        None if url.starts_with('/') => url,
        None => return None,
    };
    Some(Mark::Cwd(PathBuf::from(percent_decode(path))))
}

/// `state;percent`: 0 off, 1 normal, 2 error, 3 busy, 4 paused.
fn parse_progress(s: &str) -> Option<Progress> {
    let mut parts = s.split(';');
    let state = parts.next()?;
    let percent = parts.next().and_then(|p| p.parse::<u8>().ok()).map(|p| p.min(100));
    match state {
        "1" => Some(Progress::Normal(percent.unwrap_or(0))),
        "2" => Some(Progress::Error(percent)),
        "3" => Some(Progress::Busy),
        "4" => Some(Progress::Paused(percent)),
        _ => None,
    }
}

/// Notification text as shown: no control characters, not too long.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_NOTE).collect::<String>().trim().to_owned()
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<(usize, Mark)> {
        let mut tap = Tap::default();
        let mut out = Vec::new();
        for chunk in chunks {
            tap.scan(chunk, &mut out);
        }
        out
    }

    #[test]
    fn any_split_gives_the_same_marks() {
        let s: &[u8] = b"text\x1b[0m\x1b]133;A\x07$ \x1b]133;B\x1b\\ls\x1b]7;file://h/tmp\x07\x1b_Ga=T;QUJD\x1b\\\x1bP0;1q#0~\x1b\\\x1b]133;D;0\x1b\\";
        let whole: Vec<Mark> = scan(&[s]).into_iter().map(|(_, m)| m).collect();
        assert_eq!(whole.len(), 6);
        let bytes: Vec<&[u8]> = s.chunks(1).collect();
        assert_eq!(scan(&bytes).into_iter().map(|(_, m)| m).collect::<Vec<_>>(), whole);
        for at in 1..s.len() {
            let (a, b) = s.split_at(at);
            assert_eq!(scan(&[a, b]).into_iter().map(|(_, m)| m).collect::<Vec<_>>(), whole, "split at {at}");
        }
    }

    #[test]
    fn marks_and_offsets() {
        let s = b"x\x1b]133;A\x07$ \x1b]133;B\x1b\\ls\x1b]133;C;cmdline_url=ls\x07out\x1b]133;D;2\x07";
        assert_eq!(scan(&[s]), vec![(9, Mark::PromptStart), (20, Mark::PromptEnd), (45, Mark::CommandStart), (58, Mark::CommandEnd(Some(2)))]);
        assert_eq!(scan(&[b"\x1b]133;D\x07"]), vec![(8, Mark::CommandEnd(None))]);
    }

    #[test]
    fn split_across_reads() {
        // ESC of the terminator at the end of one read, `\` at the start of the next.
        assert_eq!(scan(&[b"\x1b]13", b"3;B\x1b", b"\\"]), vec![(1, Mark::PromptEnd)]);
        // ESC followed by something else still ends the OSC, before that byte.
        assert_eq!(scan(&[b"\x1b]133;B\x1b", b"[0m"]), vec![(0, Mark::PromptEnd)]);
    }

    #[test]
    fn cwd() {
        let cwd = |s: &str| scan(&[s.as_bytes()]).pop().map(|(_, m)| m);
        assert_eq!(cwd("\x1b]7;file://archpc/home/a b\x07"), Some(Mark::Cwd("/home/a b".into())));
        assert_eq!(cwd("\x1b]7;file://h/home/a%20b%2Fc\x1b\\"), Some(Mark::Cwd("/home/a b/c".into())));
        assert_eq!(cwd("\x1b]0;title\x07"), None);
        assert_eq!(cwd("\x1b]7;bogus\x07"), None);
    }

    #[test]
    fn images() {
        assert_eq!(scan(&[b"a\x1b_Ga=T,f=100;AAAA\x1b\\b"]), vec![(20, Mark::Graphics(b"a=T,f=100;AAAA".to_vec()))]);
        assert_eq!(scan(&[b"\x1bP0;1;0q#0~-\x1b\\"]), vec![(14, Mark::Sixel(b"0;1;0q#0~-".to_vec()))]);
        // BEL doesn't end these, and other DCS/APC aren't ours.
        assert_eq!(scan(&[b"\x1b_Gq=1\x07;x\x1b\\"]), vec![(11, Mark::Graphics(b"q=1\x07;x".to_vec()))]);
        assert_eq!(scan(&[b"\x1bP=1s\x1b\\\x1b_X\x1b\\"]), vec![]);
    }

    #[test]
    fn xtversion() {
        assert_eq!(scan(&[b"a\x1b[>qb"]), vec![(5, Mark::XtVersion)]);
        assert_eq!(scan(&[b"\x1b[>0q"]), vec![(5, Mark::XtVersion)]);
        assert_eq!(scan(&[b"\x1b[", b">", b"q"]), vec![(1, Mark::XtVersion)]);
        // Claude Code's probe: XTVERSION, DECRQM 2026, then DA1.
        assert_eq!(scan(&[b"\x1b[>0q\x1b[?2026$p\x1b[c"]), vec![(5, Mark::XtVersion)]);
        // Other `q`s aren't it: DECSCUSR (cursor shape), DECLL, XTVERSION with a stray parameter.
        assert_eq!(scan(&[b"\x1b[2 q\x1b[1q\x1b[>1q\x1b[>00000000q\x1b[0m"]), vec![]);
    }

    #[test]
    fn notifications_and_progress() {
        let notify = |title: &str, body: &str| Mark::Notify { title: title.into(), body: body.into() };
        let marks = |chunks: &[&[u8]]| scan(chunks).into_iter().map(|(_, m)| m).collect::<Vec<Mark>>();
        assert_eq!(marks(&[b"\x1b]9;Build done\x07"]), vec![notify("", "Build done")]);
        assert_eq!(marks(&[b"\x1b]777;notify;Claude;Needs you\x1b\\"]), vec![notify("Claude", "Needs you")]);
        // ConEmu's own 9;n commands are not messages; 9;4 is progress.
        assert_eq!(marks(&[b"\x1b]9;9;/tmp\x07"]), vec![]);
        assert_eq!(marks(&[b"\x1b]9;4;1;45\x07"]), vec![Mark::Progress(Some(Progress::Normal(45)))]);
        assert_eq!(marks(&[b"\x1b]9;4;3\x07"]), vec![Mark::Progress(Some(Progress::Busy))]);
        assert_eq!(marks(&[b"\x1b]9;4;0\x07"]), vec![Mark::Progress(None)]);
        // kitty: title and body in pieces (one base64), done with the last.
        let pieces: &[&[u8]] = &[b"\x1b]99;i=1:d=0;Tests\x1b\\", b"\x1b]99;i=1:d=0:p=body:e=1;MyBwYXNzZWQ=\x1b\\", b"\x1b]99;i=1:p=body;, 1 failed\x1b\\"];
        assert_eq!(marks(pieces), vec![notify("Tests", "3 passed, 1 failed")]);
    }
}
