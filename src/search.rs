//! Scrollback search (Ctrl+Shift+F) on top of alacritty_terminal's regex search.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

use crate::term::{PaneId, Term};

pub struct Search {
    pub pane: PaneId,
    pub query: String,
    regex: Option<RegexSearch>,
    pub current: Option<Match>,
    /// Where the first match is looked for: the bottom of the screen when opened.
    origin: Point,
    /// The query bar is open and takes keys. In vi mode it closes but the
    /// matches stay for n/N.
    pub editing: bool,
}

impl Search {
    pub fn new(pane: PaneId, term: &Term) -> Self {
        let offset = term.grid().display_offset() as i32;
        let origin = Point::new(Line(term.screen_lines() as i32 - 1 - offset), term.last_column());
        Self { pane, query: String::new(), regex: None, current: None, origin, editing: true }
    }

    /// The query changed: look again, starting at the current match so typing
    /// more of the same word keeps you where you are.
    pub fn update(&mut self, term: &mut Term) {
        self.regex = build(&self.query);
        let origin = self.current.as_ref().map_or(self.origin, |m| *m.start());
        self.current = None;
        self.find(term, origin, Direction::Left);
    }

    /// Jump to the next match: `Left` is older (up), `Right` is newer (down).
    pub fn step(&mut self, term: &mut Term, direction: Direction) {
        let Some(current) = self.current.clone() else { return self.update(term) };
        let origin = match direction {
            Direction::Left => current.start().sub(term, Boundary::None, 1),
            Direction::Right => current.start().add(term, Boundary::None, 1),
        };
        self.find(term, origin, direction);
    }

    fn find(&mut self, term: &mut Term, origin: Point, direction: Direction) {
        let Some(regex) = self.regex.as_mut() else { return };
        self.current = term.search_next(regex, origin, direction, Side::Left, None);
        if let Some(m) = &self.current {
            if term.mode().contains(TermMode::VI) {
                term.vi_goto_point(*m.start());
            } else {
                term.scroll_to_point(*m.start());
            }
        }
    }

    /// Matches on screen, for highlighting.
    pub fn visible(&mut self, term: &Term) -> Vec<Match> {
        let Some(regex) = self.regex.as_mut() else { return Vec::new() };
        let offset = term.grid().display_offset() as i32;
        // Widen to whole lines so matches wrapping in from off-screen are found too.
        let start = term.line_search_left(Point::new(Line(-offset), Column(0)));
        let end = term.line_search_right(Point::new(Line(term.screen_lines() as i32 - 1 - offset), term.last_column()));
        // ponytail: a pattern matching every cell on a huge wrapped line is slow; cap if it shows up.
        RegexIter::new(start, end, Direction::Right, term, regex).collect()
    }
}

/// The query as a regex, or as plain text if it isn't a valid regex (so typing
/// `foo(` still finds `foo(`). Empty query: nothing.
fn build(query: &str) -> Option<RegexSearch> {
    if query.is_empty() {
        return None;
    }
    RegexSearch::new(query).or_else(|_| RegexSearch::new(&literal(query))).ok()
}

fn literal(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if r"\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_literal() {
        assert!(RegexSearch::new("foo(").is_err());
        assert_eq!(literal("a.b(c"), r"a\.b\(c");
        assert!(build("foo(").is_some());
        assert!(build("[a-z]+").is_some());
        assert!(build("").is_none());
    }
}
