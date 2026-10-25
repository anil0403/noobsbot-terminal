//! Folders the shells were in, newest first, for the Dock menu. One file for
//! every noobsbot window: `$XDG_STATE_HOME/noobsbot/recent-folders`.

use std::path::{Path, PathBuf};

/// How many the Dock menu shows.
const KEEP: usize = 8;

fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("noobsbot").join("recent-folders"))
}

/// A shell went to `dir`. Home isn't kept: a new window opens there anyway.
pub fn add(dir: &Path) {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(file) = path().filter(|_| home.as_deref() != Some(dir)) else { return };
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let _ = std::fs::write(&file, merge(&old, dir));
}

/// The folders that still exist, newest first.
pub fn list() -> Vec<PathBuf> {
    let text = path().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    text.lines().map(PathBuf::from).filter(|p| p.is_dir()).collect()
}

/// `dir` on top of the list in `old`, without its older copy, at most KEEP lines.
fn merge(old: &str, dir: &Path) -> String {
    let dir = dir.to_string_lossy();
    let rest = old.lines().filter(|l| !l.is_empty() && *l != dir);
    std::iter::once(dir.as_ref()).chain(rest).take(KEEP).map(|l| format!("{l}\n")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_copies() {
        assert_eq!(merge("", Path::new("/a")), "/a\n");
        assert_eq!(merge("/a\n/b\n", Path::new("/b")), "/b\n/a\n");
        let full: String = (0..KEEP).map(|i| format!("/{i}\n")).collect();
        let out = merge(&full, Path::new("/new"));
        assert_eq!(out.lines().count(), KEEP);
        assert!(out.starts_with("/new\n/0\n"));
    }
}
