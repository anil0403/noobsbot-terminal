//! The list under the prompt while `cd` (or `pushd`) is typed, like
//! PowerShell's list view: the folders that match what's typed after it, and
//! folders `cd` went to before. Up and Down pick one, Tab or Right fills it in,
//! Enter fills it in and runs it, Escape closes the list until the line changes.

use std::path::{Path, PathBuf};

use crate::suggest::{self, Suggest};

/// Rows shown at once; more scroll.
pub const ROWS: usize = 8;

pub struct List {
    /// What was typed at the prompt and where, when the list was made.
    typed: String,
    cwd: Option<PathBuf>,
    /// Characters of `cd`'s argument as typed: the Backspaces filling one takes.
    arg_len: usize,
    pub entries: Vec<String>,
    pub selected: Option<usize>,
    dismissed: bool,
}

pub enum ListKey {
    Up,
    Down,
    /// Tab or Right.
    Take,
    Enter,
    Escape,
}

/// What a key did to the list.
#[derive(Debug, PartialEq)]
pub enum Done {
    /// Selection moved or the list closed: draw again.
    Redraw,
    /// Bytes for the shell (filling an entry in, maybe running it).
    Write(Vec<u8>),
    /// Not the list's: the key goes to the shell as usual.
    Pass,
}

impl List {
    /// The list for `typed` at a prompt in `cwd`, if it's a `cd`.
    pub fn new(typed: &str, cwd: Option<&Path>, suggest: &mut Suggest) -> Option<Self> {
        let arg = suggest::cd_arg(typed)?;
        let entries = suggest.folders(&suggest::unescape(arg), cwd);
        Some(Self { typed: typed.to_owned(), cwd: cwd.map(Path::to_owned), arg_len: arg.chars().count(), entries, selected: None, dismissed: false })
    }

    /// Made for this line, so its selection still stands.
    pub fn is_for(&self, typed: &str, cwd: Option<&Path>) -> bool {
        self.typed == typed && self.cwd.as_deref() == cwd
    }

    pub fn shown(&self) -> bool {
        !self.dismissed && !self.entries.is_empty()
    }

    /// The characters typed after `cd `.
    pub fn arg_len(&self) -> usize {
        self.arg_len
    }

    pub fn key(&mut self, key: ListKey) -> Done {
        if !self.shown() {
            return Done::Pass;
        }
        let n = self.entries.len();
        match key {
            ListKey::Down => self.selected = Some(self.selected.map_or(0, |i| (i + 1) % n)),
            ListKey::Up => self.selected = Some(self.selected.map_or(n - 1, |i| (i + n - 1) % n)),
            ListKey::Escape => self.dismissed = true,
            // Nothing picked: the first one.
            ListKey::Take => return Done::Write(self.fill(self.selected.unwrap_or(0))),
            ListKey::Enter => match self.selected {
                Some(i) => {
                    let mut bytes = self.fill(i);
                    bytes.push(b'\r');
                    return Done::Write(bytes);
                }
                None => return Done::Pass,
            },
        }
        Done::Redraw
    }

    /// Backspaces over what's typed after `cd `, then the entry.
    fn fill(&self, i: usize) -> Vec<u8> {
        let mut bytes = vec![0x7f; self.arg_len];
        bytes.extend(suggest::escape(&self.entries[i]).into_bytes());
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn list(entries: &[&str], arg_len: usize) -> List {
        List { typed: String::new(), cwd: None, arg_len, entries: entries.iter().map(|e| e.to_string()).collect(), selected: None, dismissed: false }
    }

    #[test]
    fn keys() {
        let mut l = list(&["src/", "My Stuff/", "target/"], 2);
        // Enter with nothing picked runs what's typed; Tab takes the first.
        assert_eq!(l.key(ListKey::Enter), Done::Pass);
        assert_eq!(l.key(ListKey::Take), Done::Write(b"\x7f\x7fsrc/".to_vec()));
        // Down from nothing is the first, Up from nothing the last; both wrap.
        assert_eq!(l.key(ListKey::Down), Done::Redraw);
        assert_eq!(l.selected, Some(0));
        l.key(ListKey::Up);
        assert_eq!(l.selected, Some(2));
        l.key(ListKey::Down);
        assert_eq!(l.selected, Some(0));
        l.selected = None;
        l.key(ListKey::Up);
        assert_eq!(l.selected, Some(2));
        l.key(ListKey::Up);
        assert_eq!(l.key(ListKey::Enter), Done::Write(b"\x7f\x7fMy\\ Stuff/\r".to_vec()));
        // Escape closes it; then every key is the shell's.
        assert_eq!(l.key(ListKey::Escape), Done::Redraw);
        assert!(!l.shown());
        assert_eq!(l.key(ListKey::Down), Done::Pass);
        assert_eq!(l.key(ListKey::Take), Done::Pass);
        let mut empty = list(&[], 0);
        assert_eq!(empty.key(ListKey::Down), Done::Pass);
    }

    #[test]
    fn arguments_and_quoting() {
        assert_eq!(suggest::cd_arg("cd "), Some(""));
        assert_eq!(suggest::cd_arg("  cd  Doc"), Some("Doc"));
        assert_eq!(suggest::cd_arg("pushd ~/wo"), Some("~/wo"));
        assert_eq!(suggest::cd_arg("cd My\\ St"), Some("My\\ St"));
        assert_eq!(suggest::cd_arg("cd 'My St"), Some("'My St"));
        assert_eq!(suggest::cd_arg("cd a b"), None);
        assert_eq!(suggest::cd_arg("cd -P x"), None);
        assert_eq!(suggest::cd_arg("cd"), None);
        assert_eq!(suggest::cd_arg("cdx y"), None);
        assert_eq!(suggest::cd_arg("ls Doc"), None);
        assert_eq!(suggest::unescape("'My St"), "My St");
        assert_eq!(suggest::unescape("My\\ St\\\\"), "My St\\");
        assert_eq!(suggest::escape("~/My Stuff (old)/"), "~/My\\ Stuff\\ \\(old\\)/");
        assert_eq!(suggest::escape("src/"), "src/");
        assert_eq!(suggest::escape("a~b/"), "a\\~b/");
    }

    #[test]
    fn folders_then_history() {
        let base = std::env::temp_dir().join(format!("noobsbot-list-{}", std::process::id()));
        for d in ["Documents/projects", "Documents/Private", "Downloads", "my docs", ".config", "elsewhere/deep"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        fs::write(base.join("Doc.txt"), "").unwrap();
        let mut s = Suggest::default();
        // Ran in `base`: a relative cd; an absolute one; one that's gone; a repeat of a listed folder.
        s.ran("cd elsewhere/deep", &base);
        s.ran(&format!("cd {}", base.join("gone").display()), &base);
        s.ran("cd Downloads", &base);
        let here = Some(base.as_path());
        let mut get = |typed: &str| List::new(typed, here, &mut s).map(|l| l.entries);
        // Starts with first (case aside), then contains; files and dot folders left out.
        let deep = format!("{}/", base.join("elsewhere/deep").display());
        // (The history folder's path is under the temp dir, which could hold "do" too.)
        assert!(get("cd do").unwrap().starts_with(&["Documents/".into(), "Downloads/".into(), "my docs/".into()]));
        assert_eq!(get("cd ").unwrap(), vec!["Documents/", "Downloads/", "elsewhere/", "my docs/", deep.as_str()]);
        assert_eq!(get("cd .c").unwrap()[0], ".config/");
        // A subfolder: its own folders, keeping what was typed before the slash.
        assert_eq!(get("cd Documents/p").unwrap(), vec!["Documents/Private/", "Documents/projects/"]);
        assert_eq!(get("cd Documents/ro").unwrap(), vec!["Documents/projects/"]);
        assert_eq!(get("cd my\\ d").unwrap(), vec!["my docs/"]);
        // History matches anywhere in the path.
        assert_eq!(get("cd dee").unwrap(), vec![deep.as_str()]);
        assert_eq!(get("cd zzz").unwrap(), Vec::<String>::new());
        assert!(get("ls do").is_none());
        fs::remove_dir_all(&base).unwrap();
    }
}
