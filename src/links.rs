//! Clickable links: OSC 8 hyperlinks, URLs and `path:line[:col]` in the output.

use std::path::{Path, PathBuf};

use alacritty_terminal::Term;
use alacritty_terminal::index::{Boundary, Direction, Point};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

use crate::pty::PROMPT_TAG;

/// URLs, or a file path with a line number. No uppercase, so the search is
/// case-insensitive.
const PATTERN: &str = concat!(
    r#"(https?://|file://|ftp://|ssh://|git://|mailto:)[^\x00-\x20\x7f-\x9f<>"{}^`\\]+"#,
    r"|[a-z0-9_.~/+-]*[a-z0-9_-]\.[a-z0-9]+:[0-9]+(:[0-9]+)?",
);

#[derive(Debug, PartialEq)]
pub enum Target {
    Url(String),
    File { path: PathBuf, line: usize },
}

pub struct Link {
    pub range: Match,
    pub target: Target,
}

pub struct Hints(RegexSearch);

impl Hints {
    pub fn new() -> Self {
        Self(RegexSearch::new(PATTERN).expect("link pattern compiles"))
    }

    /// The link under `point`. Relative paths resolve against `cwd` and only
    /// count if the file exists, so `example.com:8080` isn't one.
    pub fn at<T>(&mut self, term: &Term<T>, point: Point, cwd: Option<&Path>) -> Option<Link> {
        if let Some(link) = hyperlink(term, point) {
            return Some(link);
        }
        let (start, end) = (term.line_search_left(point), term.line_search_right(point));
        let m = RegexIter::new(start, end, Direction::Right, term, &mut self.0).find(|m| m.contains(&point))?;
        let text = term.bounds_to_string(*m.start(), *m.end());
        if text.contains("://") || text.to_lowercase().starts_with("mailto:") {
            let url = trim_url(&text);
            let end = m.end().sub(term, Boundary::None, text.chars().count() - url.chars().count());
            return (point <= end).then(|| Link { range: *m.start()..=end, target: Target::Url(url.to_owned()) });
        }
        let (path, line) = parse_file(&text)?;
        let path = resolve(path, cwd)?;
        Some(Link { range: m, target: Target::File { path, line } })
    }
}

/// The run of cells around `point` carrying the same OSC 8 link.
fn hyperlink<T>(term: &Term<T>, point: Point) -> Option<Link> {
    let link = term.grid()[point].hyperlink().filter(|l| l.uri() != PROMPT_TAG)?;
    let same = |p: Point| term.grid()[p].hyperlink().as_ref() == Some(&link);
    let (first, last) = (term.line_search_left(point), term.line_search_right(point));
    let (mut start, mut end) = (point, point);
    while start > first && same(start.sub(term, Boundary::None, 1)) {
        start = start.sub(term, Boundary::None, 1);
    }
    while end < last && same(end.add(term, Boundary::None, 1)) {
        end = end.add(term, Boundary::None, 1);
    }
    Some(Link { range: start..=end, target: Target::Url(link.uri().to_owned()) })
}

/// Drop sentence punctuation after a URL, and a `)` that closes text around it.
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        let trimmed = match trimmed.strip_suffix(')') {
            Some(t) if trimmed.matches('(').count() < trimmed.matches(')').count() => t,
            _ => trimmed,
        };
        if trimmed == url {
            return url;
        }
        url = trimmed;
    }
}

/// `src/main.rs:12:5` → (`src/main.rs`, 12). The column is dropped: `+line` can't take it.
fn parse_file(text: &str) -> Option<(&str, usize)> {
    let (path, line) = text.split_once(':')?;
    Some((path, line.split(':').next()?.parse().ok()?))
}

fn resolve(path: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None if path.starts_with('/') => PathBuf::from(path),
        None => cwd?.join(path),
    };
    path.is_file().then_some(path)
}

/// Open a URL or file with the desktop's default handler, without waiting.
pub fn xdg_open(what: &str) {
    let what = what.to_owned();
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("xdg-open").arg(&what).status() {
            log::warn!("xdg-open {what}: {e}");
        }
    });
}

/// `$VISUAL`/`$EDITOR` with `+line path`, if one is set.
pub fn editor_command(path: &Path, line: usize) -> Option<Vec<String>> {
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).ok()?;
    // ponytail: `+line` is the vi/emacs/nano/micro convention; GUI editors (code -g) need their own syntax.
    let mut cmd: Vec<String> = editor.split_whitespace().map(str::to_owned).collect();
    if cmd.is_empty() {
        return None;
    }
    cmd.push(format!("+{line}"));
    cmd.push(path.to_string_lossy().into_owned());
    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line};
    use alacritty_terminal::term::test::mock_term;

    fn at(text: &str, col: usize, cwd: Option<&Path>) -> Option<(usize, usize, Target)> {
        let term = mock_term(text);
        let link = Hints::new().at(&term, Point::new(Line(0), Column(col)), cwd)?;
        Some((link.range.start().column.0, link.range.end().column.0, link.target))
    }

    #[test]
    fn urls() {
        let url = |s: &str| Target::Url(s.into());
        assert_eq!(at("see https://example.com/a_(b). ok", 8, None), Some((4, 28, url("https://example.com/a_(b)"))));
        assert_eq!(at("(https://example.com/x)", 3, None), Some((1, 21, url("https://example.com/x"))));
        assert_eq!(at("go HTTP://x.org/?q=1, now", 5, None), Some((3, 19, url("HTTP://x.org/?q=1"))));
        assert!(at("see https://example.com/a. ok", 1, None).is_none());
        assert!(at("example.com:8080", 3, None).is_none());
    }

    #[test]
    fn files() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let file = |line| Target::File { path: cwd.join("src/main.rs"), line };
        assert_eq!(at("--> src/main.rs:12:5 here", 6, Some(cwd)), Some((4, 19, file(12))));
        assert_eq!(at("src/main.rs:3", 0, Some(cwd)), Some((0, 12, file(3))));
        assert!(at("src/nope.rs:3", 0, Some(cwd)).is_none());
        assert!(at("src/main.rs:3", 0, None).is_none());
    }

    #[test]
    fn parse() {
        assert_eq!(parse_file("a/b.rs:12:5"), Some(("a/b.rs", 12)));
        assert_eq!(parse_file("a/b.rs:12"), Some(("a/b.rs", 12)));
        assert_eq!(trim_url("http://x/y)."), "http://x/y");
        assert_eq!(trim_url("http://x/(y)"), "http://x/(y)");
    }
}
