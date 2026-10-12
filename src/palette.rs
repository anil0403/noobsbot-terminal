//! Command palette (Ctrl+Shift+P): every action and theme, fuzzy-filtered.

use crate::input::{self, Action};

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Action(Action),
    Theme(String),
}

pub struct Row {
    pub item: Item,
    pub name: String,
    pub keys: String,
}

pub struct Palette {
    pub query: String,
    /// Index into `shown`.
    pub selected: usize,
    rows: Vec<Row>,
    /// Rows matching the query, best first.
    shown: Vec<usize>,
}

impl Palette {
    pub fn new(mut themes: Vec<String>) -> Self {
        themes.sort();
        let actions = input::actions().iter().filter(|e| e.action != Action::CommandPalette).map(|e| Row { item: Item::Action(e.action), name: e.name.clone(), keys: e.keys.clone() });
        let themes = themes.into_iter().map(|t| Row { name: format!("Theme: {t}"), item: Item::Theme(t), keys: String::new() });
        let mut palette = Self { query: String::new(), selected: 0, rows: actions.chain(themes).collect(), shown: Vec::new() };
        palette.filter();
        palette
    }

    pub fn filter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self.rows.iter().enumerate().filter_map(|(i, r)| Some((score(&self.query, &r.name)?, i))).collect();
        scored.sort_by_key(|&(s, _)| -s); // stable: ties keep table order
        self.shown = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = 0;
    }

    pub fn shown(&self) -> impl Iterator<Item = &Row> {
        self.shown.iter().map(|&i| &self.rows[i])
    }

    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn chosen(&self) -> Option<&Item> {
        self.shown.get(self.selected).map(|&i| &self.rows[i].item)
    }
}

/// Fuzzy match: `query`'s letters must appear in `text` in order. Runs of
/// consecutive letters and word starts score higher, gaps lower.
fn score(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut at = 0;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().filter(|c| !c.is_whitespace()) {
        let i = at + text[at..].iter().position(|&c| c == q)?;
        score += match last {
            Some(l) if l + 1 == i => 3,
            _ if i == 0 || !text[i - 1].is_alphanumeric() => 2,
            _ => -((i - at) as i32).min(3),
        };
        last = Some(i);
        at = i + 1;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert!(score("spr", "Split right").is_some());
        assert!(score("xyz", "Split right").is_none());
        assert!(score("", "anything").is_some());
        // Word starts beat letters buried in a word.
        assert!(score("sr", "Split right") > score("sr", "Scroll page up"));
        assert!(score("new", "New tab") > score("new", "Next window"));
    }

    #[test]
    fn filters_and_picks() {
        let mut p = Palette::new(vec!["night".into(), "danger".into()]);
        p.query = "split r".into();
        p.filter();
        assert_eq!(p.chosen(), Some(&Item::Action(Action::SplitRight)));
        p.query = "theme dan".into();
        p.filter();
        assert_eq!(p.chosen(), Some(&Item::Theme("danger".into())));
        p.query = "zzzz".into();
        p.filter();
        assert_eq!((p.len(), p.chosen()), (0, None));
    }
}
