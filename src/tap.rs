//! Watches the PTY byte stream for escape codes alacritty_terminal ignores:
//! OSC 133 (prompt and command marks from shell integration), OSC 7 (cwd),
//! and images: kitty graphics (APC `G`) and sixel (DCS `q`).

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
}

/// Longest OSC we bother to collect; longer ones aren't ours.
const MAX: usize = 4096;
/// Longest image sequence we collect.
const MAX_IMAGE: usize = 64 << 20;

#[derive(Default)]
pub struct Tap {
    state: State,
    kind: Kind,
    osc: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Str,
    /// ESC inside a string ends it; a following `\` belongs to it too.
    StrEnd,
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
        for (i, &b) in bytes.iter().enumerate() {
            if self.state == State::StrEnd {
                self.state = State::Esc;
                if b == b'\\' {
                    self.finish(i + 1, out);
                    self.state = State::Ground;
                    continue;
                }
                self.finish(i, out);
            }
            self.state = match (self.state, b) {
                (State::Str, 0x07) if self.kind == Kind::Osc => {
                    self.finish(i + 1, out);
                    State::Ground
                }
                (State::Str, 0x1b) => State::StrEnd,
                (State::Str, _) => {
                    let max = if self.kind == Kind::Osc { MAX } else { MAX_IMAGE };
                    if self.osc.len() <= max {
                        self.osc.push(b);
                    }
                    State::Str
                }
                (State::Esc, b']' | b'_' | b'P') => {
                    self.osc.clear();
                    self.kind = match b {
                        b']' => Kind::Osc,
                        b'_' => Kind::Apc,
                        _ => Kind::Dcs,
                    };
                    State::Str
                }
                (_, 0x1b) => State::Esc,
                _ => State::Ground,
            };
        }
    }

    fn finish(&mut self, end: usize, out: &mut Vec<(usize, Mark)>) {
        let s = std::mem::take(&mut self.osc);
        let mark = match self.kind {
            _ if s.len() > MAX_IMAGE => None,
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
}
