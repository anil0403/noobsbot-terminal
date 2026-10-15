//! Session restore: the tabs, splits and each shell's folder, saved when the
//! window closes and reopened by the next one. A line per tree node:
//!
//! ```text
//! active 0
//! tab 1             (the focused pane, by its place in the tab)
//! split | 0.5       (| side by side, - one above the other)
//! pane /home/me
//! pane              (folder unknown)
//! ```

use std::path::PathBuf;

use crate::pane::{Node, Tab};
use crate::term::PaneId;

pub struct SavedTab {
    /// Leaves number the panes in `cwds`, in tree order.
    pub root: Node,
    pub focused: usize,
    pub cwds: Vec<Option<PathBuf>>,
}

/// `$XDG_STATE_HOME/noobsbot/session`.
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("noobsbot").join("session"))
}

pub fn encode(tabs: &[Tab], active: usize, cwd: impl Fn(PaneId) -> Option<PathBuf>) -> String {
    let mut out = format!("active {active}\n");
    for tab in tabs {
        let focused = tab.root.leaves().iter().position(|&id| id == tab.focused).unwrap_or(0);
        out += &format!("tab {focused}\n");
        write_node(&tab.root, &mut out, &cwd);
    }
    out
}

fn write_node(node: &Node, out: &mut String, cwd: &impl Fn(PaneId) -> Option<PathBuf>) {
    match node {
        Node::Leaf(id) => {
            let path = cwd(*id).map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.contains('\n'));
            *out += &format!("pane {}\n", path.unwrap_or_default());
        }
        Node::Split { across, ratio, a, b } => {
            *out += &format!("split {} {ratio}\n", if *across { '|' } else { '-' });
            write_node(a, out, cwd);
            write_node(b, out, cwd);
        }
    }
}

/// The saved tabs and the active one; None if it doesn't read as a session.
pub fn decode(text: &str) -> Option<(Vec<SavedTab>, usize)> {
    let mut lines = text.lines();
    let active = lines.next()?.strip_prefix("active ")?.parse().ok()?;
    let mut tabs = Vec::new();
    while let Some(line) = lines.next() {
        let focused = line.strip_prefix("tab ")?.parse().ok()?;
        let mut cwds = Vec::new();
        let root = read_node(&mut lines, &mut cwds)?;
        if focused >= cwds.len() {
            return None;
        }
        tabs.push(SavedTab { root, focused, cwds });
    }
    Some((tabs, active))
}

fn read_node<'a>(lines: &mut impl Iterator<Item = &'a str>, cwds: &mut Vec<Option<PathBuf>>) -> Option<Node> {
    let line = lines.next()?;
    if let Some(path) = line.strip_prefix("pane ").or((line == "pane").then_some("")) {
        cwds.push((!path.is_empty()).then(|| PathBuf::from(path)));
        return Some(Node::Leaf(cwds.len() as PaneId - 1));
    }
    let (across, ratio) = line.strip_prefix("split ")?.split_once(' ')?;
    let across = match across {
        "|" => true,
        "-" => false,
        _ => return None,
    };
    let ratio = ratio.parse::<f32>().ok()?.clamp(0.1, 0.9);
    let a = Box::new(read_node(lines, cwds)?);
    let b = Box::new(read_node(lines, cwds)?);
    Some(Node::Split { across, ratio, a, b })
}

/// Swap the saved numbering for the panes started in their place.
pub fn renumber(node: &mut Node, ids: &[PaneId]) {
    match node {
        Node::Leaf(n) => *n = ids[*n as usize],
        Node::Split { a, b, .. } => {
            renumber(a, ids);
            renumber(b, ids);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pane::Dir;

    #[test]
    fn round_trip() {
        // Tab 1: 7 | (8 / 9), 9 focused and in a folder with a space. Tab 2: just 4.
        let mut root = Node::Leaf(7);
        root.split(7, 8, Dir::Right);
        root.split(8, 9, Dir::Down);
        root.set_ratio(&[], 0.3);
        let tabs = [Tab { root, focused: 9, zoomed: false, broadcast: true }, Tab { root: Node::Leaf(4), focused: 4, zoomed: false, broadcast: false }];
        let cwd = |id| match id {
            7 => Some(PathBuf::from("/tmp")),
            9 => Some(PathBuf::from("/my notes")),
            _ => None,
        };
        let text = encode(&tabs, 1, cwd);
        assert_eq!(text, "active 1\ntab 2\nsplit | 0.3\npane /tmp\nsplit - 0.5\npane \npane /my notes\ntab 0\npane \n");

        let (mut saved, active) = decode(&text).unwrap();
        assert_eq!((saved.len(), active), (2, 1));
        assert_eq!(saved[0].cwds, [Some("/tmp".into()), None, Some("/my notes".into())]);
        assert_eq!(saved[0].focused, 2);
        renumber(&mut saved[0].root, &[20, 21, 22]);
        assert_eq!(saved[0].root.leaves(), [20, 21, 22]);
        assert!(matches!(saved[0].root, Node::Split { across: true, ratio, .. } if ratio == 0.3));

        for bad in ["", "active x", "active 0\ntab 1\npane /a\n", "active 0\ntab 0\nsplit | 0.5\npane /a\n", "active 0\nsplit / 0.5"] {
            assert!(decode(bad).is_none(), "{bad:?}");
        }
    }
}
