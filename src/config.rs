use std::collections::HashMap;
use std::path::{Path, PathBuf};

use alacritty_terminal::term::{self, Osc52};
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle};
use notify::{RecursiveMode, Watcher};
use serde::Deserialize;
use winit::event_loop::EventLoopProxy;

use crate::render::crt;
use crate::rules::{Patterns, Rule, When};
use crate::term::UserEvent;
use crate::theme::{self, Theme, ThemePatch};

const EXAMPLE: &str = include_str!("../extra/config.example.toml");

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub font: Font,
    pub terminal: Terminal,
    pub cursor: Cursor,
    /// Theme used when no situation rule matches.
    pub theme: String,
    /// User themes and tweaks to built-in ones, layered on `default`.
    pub themes: HashMap<String, ThemePatch>,
    /// Situation rules, first match wins.
    pub rules: Vec<Rule>,
    pub quake: Quake,
    pub crt: Crt,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Crt {
    pub enabled: bool,
    pub scanlines: f32,
    pub glow: f32,
    pub curvature: f32,
    pub vignette: f32,
    pub flicker: f32,
}

impl Default for Crt {
    fn default() -> Self {
        Self { enabled: false, scanlines: 0.3, glow: 0.35, curvature: 0.06, vignette: 0.35, flicker: 0.0 }
    }
}

impl Crt {
    pub fn params(&self) -> crt::Params {
        let c = |v: f32| v.clamp(0.0, 1.0);
        crt::Params {
            scanlines: c(self.scanlines),
            glow: c(self.glow),
            curvature: c(self.curvature),
            vignette: c(self.vignette),
            flicker: c(self.flicker),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Quake {
    /// Global key that shows/hides the quake window; empty for none.
    pub hotkey: String,
    /// Percent of the screen height.
    pub height: u8,
}

impl Default for Quake {
    fn default() -> Self {
        Self { hotkey: "F12".into(), height: 40 }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Font {
    pub family: String,
    pub size: f32,
    /// 100 (thin) to 900 (black); 400 is regular, 600 (semibold) the default. Bold text is 700, or this if heavier.
    pub weight: u16,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Terminal {
    pub scrollback: usize,
    pub shell: String,
    pub clipboard: Clipboard,
    /// Replace the shell's prompt with noobsbot's powerline one.
    pub pretty_prompt: bool,
    /// Greet each new window with the cat and the system summary.
    pub welcome: bool,
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

impl Default for Font {
    fn default() -> Self {
        Self { family: "JetBrainsMono Nerd Font Mono".into(), size: 18.0, weight: 600 }
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self { scrollback: 10_000, shell: String::new(), clipboard: Clipboard::Copy, pretty_prompt: false, welcome: true }
    }
}

impl Default for Cursor {
    fn default() -> Self {
        Self { shape: Shape::Beam, blink: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: Font::default(),
            terminal: Terminal::default(),
            cursor: Cursor::default(),
            theme: "default".into(),
            themes: HashMap::new(),
            rules: vec![
                Rule { when: When { user: Some(Patterns::One("root".into())), ..Default::default() }, theme: "root".into() },
                Rule { when: When { ssh_host: Some(Patterns::One("*prod*".into())), ..Default::default() }, theme: "danger".into() },
            ],
            quake: Quake::default(),
            crt: Crt::default(),
        }
    }
}

impl Config {
    /// All resolved themes by name.
    pub fn themes(&self) -> HashMap<String, Theme> {
        theme::resolve_all(&self.themes)
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
    let base =
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"));
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
        if let Ok(event) = res
            && (event.kind.is_modify() || event.kind.is_create())
            && event.paths.iter().any(|p| p.file_name() == Some(&name))
        {
            let _ = proxy.send_event(UserEvent::ConfigChanged);
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
        assert!(toml::from_str::<Config>("[themes.default]\nforeground = \"red\"").is_err());
        assert!(toml::from_str::<Config>("[themes.default]\nforegrund = \"#ffffff\"").is_err());
    }
}
