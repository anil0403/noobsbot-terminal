//! Clickable links: OSC 8 hyperlinks, URLs and `path:line[:col]` in the output.

use std::path::{Path, PathBuf};

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// URLs, or a file path with a line number. No uppercase, so the search is
/// case-insensitive.
const PATTERN: &str =
    concat!(r#"(https?://|file://|ftp://|mailto:)[^\x00-\x20\x7f-\x9f<>"{}^`\\]+"#, r"|[a-z0-9_.~/+-]*[a-z0-9_-]\.[a-z0-9]+:[0-9]+(:[0-9]+)?",);

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

    /// The link under `point`. Relative paths resolve against `cwd` (asked only
    /// for those) and only count if the file exists, so `example.com:8080` isn't one.
    pub fn at<T>(&mut self, term: &Term<T>, point: Point, cwd: impl FnOnce() -> Option<PathBuf>) -> Option<Link> {
        if let Some(link) = hyperlink(term, point) {
            return Some(link);
        }
        let (start, end) = (term.line_search_left(point), term.line_search_right(point));
        let m = RegexIter::new(start, end, Direction::Right, term, &mut self.0).find(|m| m.contains(&point))?;
        let text = term.bounds_to_string(*m.start(), *m.end());
        if text.contains("://") || text.to_lowercase().starts_with("mailto:") {
            let url = trim_url(&text);
            let end = m.end().sub(term, Boundary::None, text.chars().count() - url.chars().count());
            return (point <= end).then(|| checked(url)).flatten().map(|target| Link { range: *m.start()..=end, target });
        }
        let (path, line) = parse_file(&text)?;
        let path = resolve(path, cwd)?;
        Some(Link { range: m, target: Target::File { path, line } })
    }

    /// Every link on screen, top to bottom, for keyboard hints.
    pub fn visible<T>(&mut self, term: &Term<T>, cwd: Option<PathBuf>) -> Vec<Link> {
        let offset = term.grid().display_offset() as i32;
        let (top, bottom) = (Line(-offset), Line(term.screen_lines() as i32 - 1 - offset));
        let mut starts: Vec<Point> =
            RegexIter::new(Point::new(top, Column(0)), Point::new(bottom, term.last_column()), Direction::Right, term, &mut self.0).map(|m| *m.start()).collect();
        // OSC 8 hyperlinks: the first cell of each run.
        for line in top.0..=bottom.0 {
            for col in 0..term.columns() {
                let link = |c: usize| term.grid()[Point::new(Line(line), Column(c))].hyperlink();
                if link(col).is_some() && (col == 0 || link(col - 1) != link(col)) {
                    starts.push(Point::new(Line(line), Column(col)));
                }
            }
        }
        starts.sort();
        let mut links: Vec<Link> = Vec::new();
        for p in starts {
            if links.last().is_some_and(|l| p <= *l.range.end()) {
                continue; // a URL in a hyperlink's text
            }
            links.extend(self.at(term, p, || cwd.clone()));
        }
        links
    }
}

/// Labels for `n` hints: a letter each while they last, else two (all the
/// same length, so none is the start of another).
pub fn labels(n: usize) -> Vec<String> {
    let keys: Vec<char> = "asdfjklghqweruiopzxcvbnmty".chars().collect(); // home row first
    match n <= keys.len() {
        true => keys[..n].iter().map(char::to_string).collect(),
        false => keys.iter().flat_map(|a| keys.iter().map(move |b| format!("{a}{b}"))).take(n).collect(),
    }
}

/// The run of cells around `point` carrying the same OSC 8 link.
fn hyperlink<T>(term: &Term<T>, point: Point) -> Option<Link> {
    let link = term.grid()[point].hyperlink().filter(|l| !l.uri().starts_with("noobsbot-"))?;
    let same = |p: Point| term.grid()[p].hyperlink().as_ref() == Some(&link);
    let (first, last) = (term.line_search_left(point), term.line_search_right(point));
    let (mut start, mut end) = (point, point);
    while start > first && same(start.sub(term, Boundary::None, 1)) {
        start = start.sub(term, Boundary::None, 1);
    }
    while end < last && same(end.add(term, Boundary::None, 1)) {
        end = end.add(term, Boundary::None, 1);
    }
    Some(Link { range: start..=end, target: checked(link.uri())? })
}

/// What a URL may open. Web, ftp and mail links go to `open`; a local
/// `file://` file opens like a file link, a folder in Finder.
/// Anything else isn't a link: a program's hyperlink could otherwise have
/// `open` start any handler (apps, custom schemes).
fn checked(url: &str) -> Option<Target> {
    let (scheme, rest) = url.split_once(':')?;
    match scheme.to_ascii_lowercase().as_str() {
        "http" | "https" | "ftp" | "mailto" => Some(Target::Url(url.to_owned())),
        "file" => {
            let rest = rest.strip_prefix("//")?;
            let (host, path) = rest.split_at(rest.find('/')?);
            static HOST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            let here = HOST.get_or_init(crate::welcome::hostname);
            if !(host.is_empty() || host == "localhost" || host == here) {
                return None;
            }
            let path = PathBuf::from(crate::tap::percent_decode(path));
            match path.is_dir() {
                true => Some(Target::Url(url.to_owned())),
                false => path.is_file().then_some(Target::File { path, line: 1 }),
            }
        }
        _ => None,
    }
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

fn resolve(path: &str, cwd: impl FnOnce() -> Option<PathBuf>) -> Option<PathBuf> {
    let path = match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var_os("HOME")?).join(rest),
        None if path.starts_with('/') => PathBuf::from(path),
        None => cwd()?.join(path),
    };
    path.is_file().then_some(path)
}

/// Open a URL or file with its default app (macOS `open`), without waiting.
pub fn os_open(what: &str) {
    let what = what.to_owned();
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("/usr/bin/open").arg(&what).status() {
            log::warn!("open {what}: {e}");
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
        let link = Hints::new().at(&term, Point::new(Line(0), Column(col)), || cwd.map(Path::to_owned))?;
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
        assert!(at("ssh://evil.example/x", 3, None).is_none());
    }

    #[test]
    fn only_safe_targets() {
        assert_eq!(checked("HTTPS://x.org"), Some(Target::Url("HTTPS://x.org".into())));
        assert_eq!(checked("mailto:a@b.c"), Some(Target::Url("mailto:a@b.c".into())));
        assert_eq!(checked("file:///"), Some(Target::Url("file:///".into()))); // a folder
        let main = concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs");
        assert_eq!(checked(&format!("file://{main}")), Some(Target::File { path: main.into(), line: 1 }));
        assert_eq!(checked(&format!("file://localhost{}", main.replace(' ', "%20"))), Some(Target::File { path: main.into(), line: 1 }));
        for bad in ["file://elsewhere.example/etc", "file:///nope/nothing", "javascript:alert(1)", "smb://h/s", "x-scheme:foo", "/etc/passwd", "-help"] {
            assert_eq!(checked(bad), None, "{bad}");
        }
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
    fn visible_links() {
        let cwd = Path::new(env!("CARGO_MANIFEST_DIR"));
        let term = mock_term("a https://x.org b\r\nsrc/main.rs:3 and http://y.org/z.");
        let links = Hints::new().visible(&term, Some(cwd.into()));
        let targets: Vec<_> = links.iter().map(|l| &l.target).collect();
        assert_eq!(
            targets,
            [&Target::Url("https://x.org".into()), &Target::File { path: cwd.join("src/main.rs"), line: 3 }, &Target::Url("http://y.org/z".into())]
        );
        assert!(Hints::new().visible(&term, None).len() == 2); // no cwd: no relative files
        assert_eq!(labels(3), ["a", "s", "d"]);
        let many = labels(30);
        assert_eq!((many[0].as_str(), many[29].as_str()), ("aa", "sf"));
    }

    #[test]
    fn parse() {
        assert_eq!(parse_file("a/b.rs:12:5"), Some(("a/b.rs", 12)));
        assert_eq!(parse_file("a/b.rs:12"), Some(("a/b.rs", 12)));
        assert_eq!(trim_url("http://x/y)."), "http://x/y");
        assert_eq!(trim_url("http://x/(y)"), "http://x/(y)");
    }
}
