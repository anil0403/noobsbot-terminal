use std::path::{Path, PathBuf};

use alacritty_terminal::term::{self, Osc52};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle, Rgb};
use notify::{RecursiveMode, Watcher};
use serde::Deserialize;
use winit::event_loop::EventLoopProxy;

use crate::term::UserEvent;
use crate::theme::Theme;

const EXAMPLE: &str = include_str!("../config.example.toml");

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub terminal: Terminal,
    pub cursor: Cursor,
    pub colors: Colors,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub size: f32,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Terminal {
    pub scrollback: usize,
    pub shell: String,
    pub clipboard: Clipboard,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum Clipboard {
    Off,
    Copy,
    Paste,
    CopyPaste,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Cursor {
    pub shape: Shape,
    pub blink: bool,
}

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    Block,
    Beam,
    Underline,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Colors {
    pub foreground: Hex,
    pub background: Hex,
    pub cursor: Hex,
    pub selection: Hex,
    pub normal: [Hex; 8],
    pub bright: [Hex; 8],
}

/// "#rrggbb" in the file.
#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(try_from = "String")]
pub struct Hex(pub Rgb);

impl TryFrom<String> for Hex {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6).ok_or(format!("expected #rrggbb, got {s:?}"))?;
        let v = u32::from_str_radix(hex, 16).map_err(|_| format!("bad hex color {s:?}"))?;
        Ok(Hex(Rgb { r: (v >> 16) as u8, g: (v >> 8) as u8, b: v as u8 }))
    }
}

impl Default for Font {
    fn default() -> Self {
        Self { family: "JetBrainsMono Nerd Font Mono".into(), size: 15.0 }
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self { scrollback: 10_000, shell: String::new(), clipboard: Clipboard::Copy }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self { shape: Shape::Block, blink: false }
    }
}

impl Default for Colors {
    fn default() -> Self {
        let t = Theme::default();
        Self {
            foreground: Hex(t.fg),
            background: Hex(t.bg),
            cursor: Hex(t.cursor),
            selection: Hex(t.selection),
            normal: std::array::from_fn(|i| Hex(t.ansi[i])),
            bright: std::array::from_fn(|i| Hex(t.ansi[i + 8])),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self { font: Font::default(), terminal: Terminal::default(), cursor: Cursor::default(), colors: Colors::default() }
    }
}

impl Config {
    pub fn theme(&self) -> Theme {
        let c = &self.colors;
        Theme {
            fg: c.foreground.0,
            bg: c.background.0,
            cursor: c.cursor.0,
            selection: c.selection.0,
            ansi: std::array::from_fn(|i| if i < 8 { c.normal[i].0 } else { c.bright[i - 8].0 }),
        }
    }

    pub fn term(&self) -> term::Config {
        term::Config {
            scrolling_history: self.terminal.scrollback,
            default_cursor_style: CursorStyle {
                shape: match self.cursor.shape {
                    Shape::Block => CursorShape::Block,
                    Shape::Beam => CursorShape::Beam,
                    Shape::Underline => CursorShape::Underline,
                },
                blinking: self.cursor.blink,
            },
            osc52: match self.terminal.clipboard {
                Clipboard::Off => Osc52::Disabled,
                Clipboard::Copy => Osc52::OnlyCopy,
                Clipboard::Paste => Osc52::OnlyPaste,
                Clipboard::CopyPaste => Osc52::CopyPaste,
            },
            ..Default::default()
        }
    }
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
    base.join("noobsbot").join("config.toml")
}

/// Read the config. A missing file gets the documented example written in its place.
pub fn load(path: &Path) -> Result<Config, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(path, EXAMPLE) {
                Ok(()) => log::info!("wrote default config to {}", path.display()),
                Err(e) => log::warn!("could not write {}: {e}", path.display()),
            }
            Ok(Config::default())
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Watch the config's directory (editors often replace the file rather than write it).
pub fn watch(path: &Path, proxy: EventLoopProxy<UserEvent>) -> Option<notify::RecommendedWatcher> {
    let name = path.file_name()?.to_owned();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            if (event.kind.is_modify() || event.kind.is_create()) && event.paths.iter().any(|p| p.file_name() == Some(&name)) {
                let _ = proxy.send_event(UserEvent::ConfigChanged);
            }
        }
    })
    .map_err(|e| log::warn!("config watch failed: {e}"))
    .ok()?;
    watcher.watch(path.parent()?, RecursiveMode::NonRecursive).map_err(|e| log::warn!("config watch failed: {e}")).ok()?;
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_matches_defaults() {
        assert_eq!(toml::from_str::<Config>(EXAMPLE).unwrap(), Config::default());
    }

    #[test]
    fn partial_and_bad_configs() {
        let c: Config = toml::from_str("[font]\nsize = 20.0").unwrap();
        assert_eq!(c.font.size, 20.0);
        assert_eq!(c.font.family, Font::default().family);
        assert!(toml::from_str::<Config>("[font]\nsise = 20.0").is_err());
        assert!(toml::from_str::<Config>("[colors]\nforeground = \"red\"").is_err());
    }
}
