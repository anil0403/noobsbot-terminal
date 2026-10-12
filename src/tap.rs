//! Watches the PTY byte stream for escape codes alacritty_terminal ignores:
//! OSC 133 (prompt and command marks from shell integration) and OSC 7 (cwd).

use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Mark {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandEnd(Option<i32>),
    Cwd(PathBuf),
}

/// Longest OSC we bother to collect; longer ones aren't ours.
const MAX: usize = 4096;

#[derive(Default)]
pub struct Tap {
    state: State,
    osc: Vec<u8>,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Esc,
    Osc,
    /// ESC inside an OSC ends it; a following `\` belongs to it too.
    OscEnd,
}

impl Tap {
    /// Scan a chunk. Each mark comes with the offset just past its sequence, so
    /// the caller can feed the parser up to there before acting on it.
    pub fn scan(&mut self, bytes: &[u8], out: &mut Vec<(usize, Mark)>) {
        for (i, &b) in bytes.iter().enumerate() {
            if self.state == State::OscEnd {
                self.state = State::Esc;
                if b == b'\\' {
                    self.finish(i + 1, out);
                    self.state = State::Ground;
                    continue;
                }
                self.finish(i, out);
            }
            self.state = match (self.state, b) {
                (State::Osc, 0x07) => {
                    self.finish(i + 1, out);
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEnd,
                (State::Osc, _) => {
                    if self.osc.len() <= MAX {
                        self.osc.push(b);
                    }
                    State::Osc
                }
                (State::Esc, b']') => {
                    self.osc.clear();
                    State::Osc
                }
                (_, 0x1b) => State::Esc,
                _ => State::Ground,
            };
        }
    }

    fn finish(&mut self, end: usize, out: &mut Vec<(usize, Mark)>) {
        if let Some(mark) = parse(&self.osc) {
            out.push((end, mark));
        }
        self.osc.clear();
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

fn percent_decode(s: &str) -> String {
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
        assert_eq!(
            scan(&[s]),
            vec![(9, Mark::PromptStart), (20, Mark::PromptEnd), (45, Mark::CommandStart), (58, Mark::CommandEnd(Some(2)))]
        );
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
}
