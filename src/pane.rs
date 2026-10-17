//! Tabs hold a tree of splits; the leaves are panes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::vte::ansi::{ClearMode, Handler};

use crate::blocks::Blocks;
use crate::context::Context;
use crate::images::Images;
use crate::pty::Pty;
use crate::term::{PaneId, Term};
use crate::theme::Fade;

/// Gap between split panes, in pixels.
const DIVIDER: f32 = 2.0;

pub struct Pane {
    pub term: Arc<FairMutex<Term>>,
    /// Shared with the reader thread, which adds them.
    pub images: Arc<Mutex<Images>>,
    pub pty: Pty,
    pub theme: Fade,
    pub theme_name: String,
    /// Situation the theme was last chosen for.
    pub context: Context,
    pub last_context_check: Instant,
    /// A situation check is owed at this time (throttled output, or the clock).
    pub context_due: Option<Instant>,
    pub rect: Rect,
    pub blocks: Blocks,
    /// Reported by the shell (OSC 7).
    pub cwd: Option<PathBuf>,
    /// When the last command failed, for the red border flash.
    pub failed_at: Option<Instant>,
    /// When the program last rang the bell (BEL), for the border flash.
    pub bell_at: Option<Instant>,
    /// The shell is waiting at its prompt (shell integration), not running a command.
    pub at_prompt: bool,
    /// The command running and where it started, for the suggestions once it succeeds.
    pub running: Option<(String, PathBuf)>,
    /// The welcome screen is still up, until the first key at the prompt.
    pub welcome: bool,
    /// When the welcome screen started typing itself out, until it's all shown.
    pub typing: Option<Instant>,
    /// Lines the welcome screen takes, for drawing it again in place.
    pub welcome_rows: usize,
}

impl Pane {
    /// The shell's directory: what it reported if that exists here (not over
    /// ssh), else what /proc says.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.cwd.clone().filter(|p| p.is_dir()).or_else(|| self.pty.shell_cwd())
    }

    /// Draws the welcome screen again where it is, so its clock and meters stay current.
    /// True if it did (only once it's done typing, and while it's all still there).
    pub fn refresh_welcome(&mut self, gpu: &str, shell: &str) -> bool {
        if !self.welcome || self.typing.is_some() {
            return false;
        }
        let mut term = self.term.lock();
        let screen = crate::welcome::screen(term.columns(), gpu, shell);
        crate::welcome::redraw(&mut term, &screen, self.welcome_rows)
    }

    /// Clears the welcome screen (and its scrollback) on the first key at the
    /// prompt; Ctrl+L makes the shell redraw its prompt at the top. True if it did.
    pub fn clear_welcome(&mut self) -> bool {
        if !(self.welcome && self.at_prompt) {
            return false;
        }
        (self.welcome, self.typing) = (false, None);
        let mut term = self.term.lock();
        term.clear_screen(ClearMode::All);
        term.clear_screen(ClearMode::Saved);
        drop(term);
        self.pty.write(b"\x0c");
        true
    }
}

pub struct Tab {
    pub root: Node,
    pub focused: PaneId,
    /// The focused pane fills the tab; the others are hidden, not resized.
    pub zoomed: bool,
    /// Typing and pastes go to every pane in the tab.
    pub broadcast: bool,
}

impl Tab {
    /// Panes on screen, in tree order.
    pub fn visible(&self) -> Vec<PaneId> {
        if self.zoomed { vec![self.focused] } else { self.root.leaves() }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    /// Moves along the x axis.
    pub fn across(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }
}

pub enum Node {
    Leaf(PaneId),
    /// `across`: `a | b` side by side; otherwise `a` above `b`.
    Split {
        across: bool,
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

/// The gap between the halves of a split, for drawing and dragging.
pub struct Divider {
    pub rect: Rect,
    /// The whole split's area; the ratio is measured across it.
    pub area: Rect,
    pub across: bool,
    /// Which child to take at each level to reach the split (false = a).
    pub path: Vec<bool>,
}

impl Node {
    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Node::Leaf(leaf) => *leaf == id,
            Node::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    pub fn leaves(&self) -> Vec<PaneId> {
        match self {
            Node::Leaf(id) => vec![*id],
            Node::Split { a, b, .. } => [a.leaves(), b.leaves()].concat(),
        }
    }

    /// Put `new` next to `target` (right of it or below it).
    pub fn split(&mut self, target: PaneId, new: PaneId, side: Dir) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                let (old, new) = (Box::new(Node::Leaf(target)), Box::new(Node::Leaf(new)));
                let across = side.across();
                let (a, b) = if matches!(side, Dir::Left | Dir::Up) { (new, old) } else { (old, new) };
                *self = Node::Split { across, ratio: 0.5, a, b };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, side) || b.split(target, new, side),
        }
    }

    /// Remove a leaf; its sibling takes the parent split's place. Returns the
    /// pane to focus instead, or None if `target` is the whole tree (or absent).
    pub fn remove(&mut self, target: PaneId) -> Option<PaneId> {
        let Node::Split { a, b, .. } = self else { return None };
        let sibling = if matches!(**a, Node::Leaf(id) if id == target) {
            std::mem::replace(&mut **b, Node::Leaf(0))
        } else if matches!(**b, Node::Leaf(id) if id == target) {
            std::mem::replace(&mut **a, Node::Leaf(0))
        } else {
            return a.remove(target).or_else(|| b.remove(target));
        };
        *self = sibling;
        Some(self.leaves()[0])
    }

    /// Move the nearest divider around `target` that runs the right way.
    pub fn resize(&mut self, target: PaneId, across: bool, delta: f32) -> bool {
        let Node::Split { across: split_across, ratio, a, b } = self else { return false };
        if !a.contains(target) && !b.contains(target) {
            return false;
        }
        if a.resize(target, across, delta) || b.resize(target, across, delta) {
            return true;
        }
        if *split_across != across {
            return false;
        }
        *ratio = (*ratio + delta).clamp(0.1, 0.9);
        true
    }

    pub fn set_ratio(&mut self, path: &[bool], value: f32) {
        if let Node::Split { ratio, a, b, .. } = self {
            match path.split_first() {
                None => *ratio = value.clamp(0.1, 0.9),
                Some((false, rest)) => a.set_ratio(rest, value),
                Some((true, rest)) => b.set_ratio(rest, value),
            }
        }
    }

    /// Pixel rects for every pane, plus the dividers between them.
    pub fn layout(&self, area: Rect, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        self.layout_at(area, &mut Vec::new(), panes, dividers);
    }

    fn layout_at(&self, area: Rect, path: &mut Vec<bool>, panes: &mut Vec<(PaneId, Rect)>, dividers: &mut Vec<Divider>) {
        match self {
            Node::Leaf(id) => panes.push((*id, area)),
            Node::Split { across, ratio, a, b } => {
                let r = area;
                let (ra, div, rb) = if *across {
                    let wa = ((r.w - DIVIDER) * ratio).round();
                    (Rect { w: wa, ..r }, Rect { x: r.x + wa, w: DIVIDER, ..r }, Rect { x: r.x + wa + DIVIDER, w: r.w - wa - DIVIDER, ..r })
                } else {
                    let ha = ((r.h - DIVIDER) * ratio).round();
                    (Rect { h: ha, ..r }, Rect { y: r.y + ha, h: DIVIDER, ..r }, Rect { y: r.y + ha + DIVIDER, h: r.h - ha - DIVIDER, ..r })
                };
                dividers.push(Divider { rect: div, area, across: *across, path: path.clone() });
                path.push(false);
                a.layout_at(ra, path, panes, dividers);
                path.pop();
                path.push(true);
                b.layout_at(rb, path, panes, dividers);
                path.pop();
            }
        }
    }
}

/// The closest pane in direction `dir` that overlaps `from` on the other axis.
pub fn neighbor(panes: &[(PaneId, Rect)], from: PaneId, dir: Dir) -> Option<PaneId> {
    let r = panes.iter().find(|p| p.0 == from)?.1;
    let key = |o: &Rect| -> Option<(f32, f32)> {
        let overlap_x = o.x < r.x + r.w && o.x + o.w > r.x;
        let overlap_y = o.y < r.y + r.h && o.y + o.h > r.y;
        // (gap along the direction, offset of the centers across it)
        let center = |o: &Rect, across: bool| if across { o.y + o.h / 2.0 } else { o.x + o.w / 2.0 };
        let off = (center(o, dir.across()) - center(&r, dir.across())).abs();
        match dir {
            Dir::Left if overlap_y && o.x + o.w <= r.x => Some((r.x - o.x - o.w, off)),
            Dir::Right if overlap_y && o.x >= r.x + r.w => Some((o.x - r.x - r.w, off)),
            Dir::Up if overlap_x && o.y + o.h <= r.y => Some((r.y - o.y - o.h, off)),
            Dir::Down if overlap_x && o.y >= r.y + r.h => Some((o.y - r.y - r.h, off)),
            _ => None,
        }
    };
    panes.iter().filter_map(|(id, o)| Some((*id, key(o)?))).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0.0, y: 0.0, w: 102.0, h: 52.0 };

    fn layout(node: &Node) -> (Vec<(PaneId, Rect)>, Vec<Divider>) {
        let (mut panes, mut dividers) = (Vec::new(), Vec::new());
        node.layout(AREA, &mut panes, &mut dividers);
        (panes, dividers)
    }

    /// 1 | 2
    ///   | -
    ///   | 3
    fn three() -> Node {
        let mut root = Node::Leaf(1);
        assert!(root.split(1, 2, Dir::Right));
        assert!(root.split(2, 3, Dir::Down));
        assert!(!root.split(9, 4, Dir::Down));
        root
    }

    #[test]
    fn splits_before() {
        // Left and up put the new pane first.
        let mut root = Node::Leaf(1);
        root.split(1, 2, Dir::Left);
        root.split(1, 3, Dir::Up);
        assert_eq!(root.leaves(), [2, 3, 1]);
        let (panes, _) = layout(&root);
        assert!(panes[0].1.x < panes[2].1.x && panes[1].1.y < panes[2].1.y);
    }

    #[test]
    fn splits_and_lays_out() {
        let root = three();
        assert_eq!(root.leaves(), [1, 2, 3]);
        let (panes, dividers) = layout(&root);
        assert_eq!(panes[0], (1, Rect { x: 0.0, y: 0.0, w: 50.0, h: 52.0 }));
        assert_eq!(panes[1], (2, Rect { x: 52.0, y: 0.0, w: 50.0, h: 25.0 }));
        assert_eq!(panes[2], (3, Rect { x: 52.0, y: 27.0, w: 50.0, h: 25.0 }));
        assert_eq!(dividers[0].rect, Rect { x: 50.0, y: 0.0, w: 2.0, h: 52.0 });
        assert_eq!(dividers[1].path, [true]);
    }

    #[test]
    fn moves_focus() {
        let (panes, _) = layout(&three());
        assert_eq!(neighbor(&panes, 1, Dir::Right), Some(2)); // 2 and 3 tie; first in tree order wins
        assert_eq!(neighbor(&panes, 3, Dir::Left), Some(1));
        assert_eq!(neighbor(&panes, 3, Dir::Up), Some(2));
        assert_eq!(neighbor(&panes, 2, Dir::Down), Some(3));
        assert_eq!(neighbor(&panes, 1, Dir::Left), None);
        assert_eq!(neighbor(&panes, 1, Dir::Up), None);
    }

    #[test]
    fn resizes() {
        let mut root = three();
        assert!(root.resize(3, true, 0.1)); // the outer, side-by-side split
        assert!(root.resize(3, false, -0.1)); // the inner one
        assert!(!Node::Leaf(1).resize(1, true, 0.1));
        let (panes, _) = layout(&root);
        assert_eq!(panes[0].1.w, 60.0);
        assert_eq!(panes[1].1.h, 20.0);
        root.set_ratio(&[], 0.0);
        assert_eq!(layout(&root).0[0].1.w, 10.0); // clamped to 10%
    }

    #[test]
    fn removes() {
        let mut root = three();
        assert_eq!(root.remove(1), Some(2)); // 2 / 3 takes the whole area
        assert_eq!(root.leaves(), [2, 3]);
        assert_eq!(root.remove(3), Some(2));
        assert_eq!(root.remove(2), None);
        assert_eq!(root.leaves(), [2]);
    }
}
