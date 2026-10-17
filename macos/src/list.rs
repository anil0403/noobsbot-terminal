//! What the argument being typed can name, laid out under the prompt like
//! `ls`, narrowing as it grows: the folders `cd` (or `pushd`) can go to, or
//! everything in the folder for `ls`. Tab fills in the only match (or what all
//! of them start with) and is never the shell's while the grid shows; Escape
//! hides the grid until the line changes. Every other key is the shell's.

use std::path::{Path, PathBuf};

use crate::suggest::{self, Suggest};

/// Rows of names shown at most; a line under them says how many more.
pub const ROWS: usize = 6;
/// Spaces between columns.
pub const GAP: usize = 2;

pub struct List {
    /// What was typed at the prompt and where, when the list was made.
    typed: String,
    cwd: Option<PathBuf>,
    /// Characters of `cd`'s argument as typed: the Backspaces filling one takes.
    arg_len: usize,
    /// The argument up to its last slash (unescaped), and what's after it.
    dir: String,
    needle: String,
    /// Just the command, no space after it yet.
    bare: bool,
    /// The matching names, each with whether it's a folder.
    pub entries: Vec<(String, bool)>,
    dismissed: bool,
}

pub enum ListKey {
    Tab,
    Escape,
}

/// What a key did to the list.
#[derive(Debug, PartialEq)]
pub enum Done {
    /// The grid closed: draw again.
    Redraw,
    /// Bytes for the shell, filling a folder in.
    Write(Vec<u8>),
    /// Taken, with nothing to do (Tab with nothing to fill in: the grid
    /// already shows the choices, and the shell would list them again).
    Swallow,
    /// Not the list's: the key goes to the shell as usual.
    Pass,
}

impl List {
    /// The list for `typed` at a prompt in `cwd`, if it's a `cd` or an `ls`.
    pub fn new(typed: &str, cwd: Option<&Path>, suggest: &mut Suggest) -> Option<Self> {
        let (arg, files) = suggest::arg(typed)?;
        let plain = suggest::unescape(arg);
        let entries = suggest.entries(&plain, cwd, files);
        let (dir, needle) = plain.rsplit_once('/').map_or((String::new(), plain.clone()), |(d, n)| (format!("{d}/"), n.to_owned()));
        Some(Self {
            typed: typed.to_owned(),
            cwd: cwd.map(Path::to_owned),
            arg_len: arg.chars().count(),
            bare: !typed.trim_start().contains(' '),
            dir,
            needle,
            entries,
            dismissed: false,
        })
    }

    /// Made for this line, so it's still the one to show.
    pub fn is_for(&self, typed: &str, cwd: Option<&Path>) -> bool {
        self.typed == typed && self.cwd.as_deref() == cwd
    }

    pub fn shown(&self) -> bool {
        !self.dismissed && !self.entries.is_empty()
    }

    pub fn key(&mut self, key: ListKey) -> Done {
        if !self.shown() {
            return Done::Pass;
        }
        match key {
            ListKey::Escape => {
                self.dismissed = true;
                Done::Redraw
            }
            // Just `cd`: the space after it (the shell would list commands).
            ListKey::Tab if self.bare => Done::Write(b" ".to_vec()),
            // The only match: a folder with its slash (the grid then shows
            // what's in it), a file with a space.
            ListKey::Tab if self.entries.len() == 1 => {
                let (name, dir) = &self.entries[0];
                Done::Write(self.fill(name, if *dir { "/" } else { " " }))
            }
            // What they all start with, if that's more than what's typed.
            ListKey::Tab => match common_prefix(&self.entries.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()) {
                p if p.chars().count() > self.needle.chars().count() => Done::Write(self.fill(p, "")),
                _ => Done::Swallow,
            },
        }
    }

    /// Backspaces over the argument as typed, then `name` in its folder and `end`.
    fn fill(&self, name: &str, end: &str) -> Vec<u8> {
        let mut bytes = vec![0x7f; self.arg_len];
        bytes.extend(suggest::escape(&format!("{}{name}", self.dir)).into_bytes());
        bytes.extend(end.as_bytes());
        bytes
    }
}

/// The longest start all `names` share.
pub fn common_prefix<'a>(names: &[&'a str]) -> &'a str {
    let Some(&first) = names.first() else { return "" };
    let mut end = first.len();
    for n in &names[1..] {
        end = first.char_indices().zip(n.chars()).take_while(|((_, a), b)| a == b).last().map_or(0, |((i, a), _)| i + a.len_utf8()).min(end);
    }
    &first[..end]
}

/// Columns for `names` across `cols` cells, and each column's width: the
/// longest name plus the gap (the last column needs none).
pub fn grid(entries: &[(String, bool)], cols: usize) -> (usize, usize) {
    let width = entries.iter().map(|(n, _)| n.chars().count()).max().unwrap_or(0) + GAP;
    (((cols + GAP) / width).max(1), width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Folders, or files when the name has a dot.
    fn list(names: &[&str], typed_after: &str) -> List {
        let (dir, needle) = typed_after.rsplit_once('/').map_or((String::new(), typed_after.to_owned()), |(d, n)| (format!("{d}/"), n.to_owned()));
        let entries = names.iter().map(|e| (e.to_string(), !e.contains('.'))).collect();
        List { typed: String::new(), cwd: None, arg_len: typed_after.chars().count(), bare: false, dir, needle, entries, dismissed: false }
    }

    #[test]
    fn keys() {
        // One match: filled in with its slash.
        assert_eq!(list(&["My Stuff"], "my").key(ListKey::Tab), Done::Write(b"\x7f\x7fMy\\ Stuff/".to_vec()));
        assert_eq!(list(&["src"], "a/s").key(ListKey::Tab), Done::Write(b"\x7f\x7f\x7fa/src/".to_vec()));
        // A file (from ls): with a space.
        assert_eq!(list(&["notes.txt"], "no").key(ListKey::Tab), Done::Write(b"\x7f\x7fnotes.txt ".to_vec()));
        // Several: what they share, when that's longer than what's typed; else nothing.
        assert_eq!(list(&["Documents", "Downloads"], "d").key(ListKey::Tab), Done::Write(b"\x7fDo".to_vec()));
        assert_eq!(list(&["Documents", "Downloads"], "Do").key(ListKey::Tab), Done::Swallow);
        assert_eq!(list(&["Documents", "my docs"], "doc").key(ListKey::Tab), Done::Swallow);
        // Escape hides it; then every key is the shell's.
        let mut l = list(&["src", "target"], "");
        assert_eq!(l.key(ListKey::Escape), Done::Redraw);
        assert!(!l.shown());
        assert_eq!(l.key(ListKey::Tab), Done::Pass);
        assert_eq!(list(&[], "x").key(ListKey::Tab), Done::Pass);
        // Just the command: Tab is the space after it, whatever's listed.
        let mut bare = List { bare: true, ..list(&["src"], "") };
        assert_eq!(bare.key(ListKey::Tab), Done::Write(b" ".to_vec()));
    }

    #[test]
    fn layout() {
        let names: Vec<(String, bool)> = ["NoobsCrypto", "Projects", "Screenshots"].map(|n| (n.to_owned(), true)).into();
        assert_eq!(common_prefix(&["NoobsCrypto", "Projects", "Screenshots"]), "");
        assert_eq!(common_prefix(&["Documents", "Downloads"]), "Do");
        assert_eq!(common_prefix(&["caf\u{e9}s", "caf\u{e9}"]), "caf\u{e9}");
        assert_eq!(common_prefix(&["one"]), "one");
        // Columns 13 wide (11 + the gap); the last one needs no gap.
        assert_eq!(grid(&names, 80), (6, 13));
        assert_eq!(grid(&names, 37), (3, 13));
        assert_eq!(grid(&names, 36), (2, 13));
        assert_eq!(grid(&names, 5), (1, 13));
    }

    #[test]
    fn arguments_and_quoting() {
        use suggest::arg;
        assert_eq!(arg("cd "), Some(("", false)));
        assert_eq!(arg("  cd  Doc"), Some(("Doc", false)));
        assert_eq!(arg("pushd ~/wo"), Some(("~/wo", false)));
        assert_eq!(arg("cd My\\ St"), Some(("My\\ St", false)));
        assert_eq!(arg("cd 'My St"), Some(("'My St", false)));
        assert_eq!(arg("cd a b"), None);
        // The command alone, before its space, lists too; a longer name doesn't.
        assert_eq!(arg("cd"), Some(("", false)));
        assert_eq!(arg("ls"), Some(("", true)));
        assert_eq!(arg("  pushd"), Some(("", false)));
        assert_eq!(arg("cdx y"), None);
        assert_eq!(arg("cdx"), None);
        assert_eq!(arg("lsof"), None);
        assert_eq!(arg("cat Doc"), None);
        // ls lists files too; options before the argument are passed over.
        assert_eq!(arg("ls "), Some(("", true)));
        assert_eq!(arg("ls Docu"), Some(("Docu", true)));
        assert_eq!(arg("ls -la "), Some(("", true)));
        assert_eq!(arg("ls -l -a Documents/"), Some(("Documents/", true)));
        assert_eq!(arg("ls -l"), None); // still typing the option
        assert_eq!(arg("cd -P x"), Some(("x", false)));
        assert_eq!(suggest::unescape("'My St"), "My St");
        assert_eq!(suggest::unescape("My\\ St\\\\"), "My St\\");
        assert_eq!(suggest::escape("~/My Stuff (old)/"), "~/My\\ Stuff\\ \\(old\\)/");
        assert_eq!(suggest::escape("src/"), "src/");
        assert_eq!(suggest::escape("a~b/"), "a\\~b/");
    }

    #[test]
    fn entries() {
        let base = std::env::temp_dir().join(format!("noobsbot-list-{}", std::process::id()));
        for d in ["Documents/projects", "Documents/Private", "Downloads", "my docs", ".config", "elsewhere"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        fs::write(base.join("Doc.txt"), "").unwrap();
        fs::write(base.join("Documents/plan.md"), "").unwrap();
        fs::write(base.join(".env"), "").unwrap();
        let mut s = Suggest::default();
        // History never shows: only what's there.
        s.ran("cd /usr", &base);
        let here = Some(base.as_path());
        let mut get = |typed: &str| List::new(typed, here, &mut s).map(|l| l.entries.into_iter().map(|(n, _)| n).collect::<Vec<_>>());
        // cd: every folder; files and dot folders left out.
        assert_eq!(get("cd ").unwrap(), vec!["Documents", "Downloads", "elsewhere", "my docs"]);
        assert_eq!(get("cd").unwrap(), get("cd ").unwrap());
        assert!(get("cdx").is_none());
        let bare = |typed: &str| List::new(typed, here, &mut Suggest::default()).unwrap().bare;
        assert!(bare("cd") && bare(" ls") && !bare("cd ") && !bare("ls Doc"));
        // Starts with first (case aside), then contains.
        assert_eq!(get("cd do").unwrap(), vec!["Documents", "Downloads", "my docs"]);
        assert_eq!(get("cd .c").unwrap(), vec![".config"]);
        // A subfolder: its own folders.
        assert_eq!(get("cd Documents/").unwrap(), vec!["Private", "projects"]);
        assert_eq!(get("cd Documents/ro").unwrap(), vec!["projects"]);
        assert_eq!(get("cd my\\ d").unwrap(), vec!["my docs"]);
        assert_eq!(get("cd us").unwrap(), Vec::<String>::new());
        // ls: files too, in the same order; dot files only for a dot.
        assert_eq!(get("ls ").unwrap(), vec!["Doc.txt", "Documents", "Downloads", "elsewhere", "my docs"]);
        assert_eq!(get("ls -la Documents/p").unwrap(), vec!["Private", "plan.md", "projects"]);
        assert_eq!(get("ls .").unwrap(), vec![".config", ".env", "Doc.txt"]);
        assert!(get("cat do").is_none());
        let folders = List::new("ls Documents/", here, &mut Suggest::default()).unwrap().entries;
        assert_eq!(folders, vec![("Private".into(), true), ("plan.md".into(), false), ("projects".into(), true)]);
        fs::remove_dir_all(&base).unwrap();
    }
}
