//! Command blocks. Prompts are found on the grid by the tag the reader thread
//! puts on their cells; what the shell reported about the command typed at
//! each prompt (exit code, duration) is kept here by prompt number.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::{Cell, Flags};

use crate::pty::PROMPT_TAG;
use crate::tap::Mark;

/// Commands remembered per pane; older ones have scrolled away long ago.
const KEEP: usize = 10_000;

pub struct Cmd {
    pub started: Instant,
    /// Exit code and run time, once it finished.
    pub done: Option<(Option<i32>, Duration)>,
}

#[derive(Default)]
pub struct Blocks {
    cmds: BTreeMap<u64, Cmd>,
}

impl Blocks {
    /// Record a mark for the command typed at `prompt`. Returns exit code and
    /// run time when it finishes. A finish without a start (Enter on an empty
    /// line) is ignored.
    pub fn mark(&mut self, prompt: u64, mark: &Mark) -> Option<(Option<i32>, Duration)> {
        match mark {
            Mark::CommandStart => {
                self.cmds.insert(prompt, Cmd { started: Instant::now(), done: None });
                if self.cmds.len() > KEEP {
                    self.cmds.pop_first();
                }
                None
            }
            Mark::CommandEnd(code) => {
                let cmd = self.cmds.get_mut(&prompt).filter(|c| c.done.is_none())?;
                let done = (*code, cmd.started.elapsed());
                cmd.done = Some(done);
                Some(done)
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn get(&self, prompt: u64) -> Option<&Cmd> {
        self.cmds.get(&prompt)
    }
}

fn tag(cell: &Cell) -> Option<u64> {
    cell.hyperlink()?.id().strip_prefix(PROMPT_TAG)?.parse().ok()
}

/// Number of the prompt drawn on `line`, if any.
pub fn prompt_at<T>(term: &Term<T>, line: Line) -> Option<u64> {
    term.grid()[line][..].iter().find_map(tag)
}

/// Every prompt in the scrollback and on screen, oldest first: its first line and number.
pub fn prompts<T>(term: &Term<T>) -> Vec<(Line, u64)> {
    let mut out: Vec<(Line, u64)> = Vec::new();
    let mut last = None;
    for i in term.topmost_line().0..=term.bottommost_line().0 {
        let line = Line(i);
        let prompt = prompt_at(term, line);
        if let Some(p) = prompt
            && prompt != last
        {
            out.push((line, p));
        }
        last = prompt;
    }
    out
}

/// The text of the command typed at `prompt` (searched for near the bottom).
pub fn command_text<T>(term: &Term<T>, prompt: u64) -> Option<String> {
    let line = (term.topmost_line().0..=term.bottommost_line().0).rev().map(Line).find(|&l| prompt_at(term, l) == Some(prompt))?;
    let row = &term.grid()[line];
    let after = (0..term.columns()).rev().find(|&c| tag(&row[Column(c)]) == Some(prompt))? + 1;
    let start = Point::new(line, Column(after.min(term.columns() - 1)));
    let end = term.line_search_right(start);
    let text = term.bounds_to_string(start, end);
    let text = text.trim_end(); // a leading space stays: shells keep such commands out of history
    (!text.trim().is_empty() && after < term.columns()).then(|| text.to_owned())
}

/// What's typed at the prompt the cursor is on, when the cursor is at its end
/// (nothing after it on the line). Wrapped input is followed up to 8 lines up.
pub fn input<T>(term: &Term<T>) -> Option<String> {
    let cursor = term.grid().cursor.point;
    let row = &term.grid()[cursor.line];
    if (cursor.column.0..term.columns()).any(|c| !matches!(row[Column(c)].c, ' ' | '\0')) {
        return None;
    }
    let mut line = cursor.line;
    let prompt = loop {
        if let Some(p) = prompt_at(term, line) {
            break p;
        }
        let up: Line = line - 1;
        if cursor.line.0 - up.0 > 8 || up < term.topmost_line() || !term.grid()[up][term.last_column()].flags.contains(Flags::WRAPLINE) {
            return None;
        }
        line = up;
    };
    let start = (0..term.columns()).rev().find(|&c| tag(&term.grid()[line][Column(c)]) == Some(prompt))? + 1;
    let mut text = String::new();
    for l in line.0..=cursor.line.0 {
        let row = &term.grid()[Line(l)];
        let from = if l == line.0 { start } else { 0 };
        let to = if l == cursor.line.0 { cursor.column.0 } else { term.columns() };
        for c in from..to {
            let cell = &row[Column(c)];
            if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                text.push(if cell.c == '\0' { ' ' } else { cell.c });
            }
        }
    }
    Some(text)
}

/// Output of the last finished command: from below its prompt and command line
/// down to the line before the current prompt.
pub fn last_output<T>(term: &Term<T>) -> Option<String> {
    let prompts = prompts(term);
    let [.., (_, prev), (current, _)] = prompts[..] else { return None };
    // The previous prompt's last line, then past its (possibly wrapped) command.
    let prompt_end = (0..current.0).rev().map(Line).find(|&l| prompt_at(term, l) == Some(prev))?;
    let start = term.line_search_right(Point::new(prompt_end, Column(0))).line + 1;
    if start >= current {
        return Some(String::new());
    }
    let text = term.bounds_to_string(Point::new(start, Column(0)), Point::new(current - 1, term.last_column()));
    Some(text.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::Processor;

    /// Feed shell output with `{A}`/`{B}` standing for tagged prompt starts and ends.
    fn term(text: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &TermSize::new(20, 10), VoidListener);
        let mut parser: Processor = Processor::new();
        let mut n = 0;
        for (i, part) in text.split('{').enumerate() {
            let rest = match (i, part.split_once('}')) {
                (0, _) | (_, None) => part,
                (_, Some(("A", rest))) => {
                    n += 1;
                    parser.advance(&mut term, format!("\x1b]8;id={PROMPT_TAG}{n};{PROMPT_TAG}\x1b\\").as_bytes());
                    rest
                }
                (_, Some((_, rest))) => {
                    parser.advance(&mut term, b"\x1b]8;;\x1b\\");
                    rest
                }
            };
            parser.advance(&mut term, rest.as_bytes());
        }
        term
    }

    #[test]
    fn finds_prompts_and_output() {
        let t = term("{A}$ {B}ls\r\none\r\ntwo\r\n{A}top\r\n$ {B}true\r\n{A}$ {B}");
        assert_eq!(prompts(&t), vec![(Line(0), 1), (Line(3), 2), (Line(5), 3)]);
        assert_eq!(command_text(&t, 1).as_deref(), Some("ls"));
        assert_eq!(command_text(&t, 2).as_deref(), Some("true"));
        assert_eq!(command_text(&t, 3), None);
        assert_eq!(command_text(&term("{A}$ {B} secret"), 1).as_deref(), Some(" secret"));
        assert_eq!(last_output(&t).as_deref(), Some(""));
        let t = term("{A}$ {B}ls\r\none\r\ntwo\r\n{A}$ {B}");
        assert_eq!(last_output(&t).as_deref(), Some("one\ntwo"));
        assert_eq!(last_output(&term("{A}$ {B}")), None);
        assert_eq!(input(&term("{A}$ {B}git st")).as_deref(), Some("git st"));
        assert_eq!(input(&term("{A}$ {B}git ")).as_deref(), Some("git "));
        assert_eq!(input(&term("{A}$ {B}")).as_deref(), Some(""));
        assert_eq!(input(&term("{A}$ {B}echo 0123456789abcdef")).as_deref(), Some("echo 0123456789abcdef")); // wrapped
        assert_eq!(input(&term("{A}$ {B}ab\x1b[D")), None); // cursor before the end
        assert_eq!(input(&term("{A}$ {B}ls\r\nout\r\n")), None); // no prompt on the line
    }

    #[test]
    fn records_commands() {
        let mut b = Blocks::default();
        assert_eq!(b.mark(1, &Mark::CommandEnd(Some(0))), None); // Enter on an empty line
        b.mark(2, &Mark::CommandStart);
        assert!(b.get(2).unwrap().done.is_none());
        assert_eq!(b.mark(2, &Mark::CommandEnd(Some(1))).map(|d| d.0), Some(Some(1)));
        assert_eq!(b.mark(2, &Mark::CommandEnd(Some(0))), None); // only once
    }
}
