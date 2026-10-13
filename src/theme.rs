use std::collections::HashMap;
use std::time::{Duration, Instant};

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use serde::Deserialize;

/// A full color palette. Situation rules pick one by name.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

const fn rgb(hex: u32) -> Rgb {
    Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            // One Half Dark's colors on black, with the warm amber / teal /
            // blue of powerline prompts (christitus.com/pretty-powershell) as accents.
            fg: rgb(0xdcdfe4),
            bg: rgb(0x000000),
            cursor: rgb(0xdcdfe4), // the text color: an inverted block, like Claude Code
            selection: rgb(0x454545),
            ansi: [
                rgb(0x3d3d3d),
                rgb(0xe06c75),
                rgb(0x98c379),
                rgb(0xe5c07b),
                rgb(0x61afef),
                rgb(0xc678dd),
                rgb(0x56b6c2),
                rgb(0xdcdfe4),
                rgb(0x686868),
                rgb(0xff7b86),
                rgb(0xb5e890),
                rgb(0xf2ad36),
                rgb(0x4e92e8),
                rgb(0xd7a1f0),
                rgb(0x5bc8a6),
                rgb(0xffffff),
            ],
        }
    }
}

/// "#rrggbb" in the config file.
#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
#[serde(try_from = "String")]
pub struct Hex(pub Rgb);

impl TryFrom<String> for Hex {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        let hex = s.strip_prefix('#').filter(|h| h.len() == 6).ok_or(format!("expected #rrggbb, got {s:?}"))?;
        let v = u32::from_str_radix(hex, 16).map_err(|_| format!("bad hex color {s:?}"))?;
        Ok(Hex(rgb(v)))
    }
}

/// A theme as written in the config: only the colors it changes.
#[derive(Debug, Deserialize, PartialEq, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ThemePatch {
    pub foreground: Option<Hex>,
    pub background: Option<Hex>,
    pub cursor: Option<Hex>,
    pub selection: Option<Hex>,
    /// black, red, green, yellow, blue, magenta, cyan, white
    pub normal: Option<[Hex; 8]>,
    pub bright: Option<[Hex; 8]>,
}

impl ThemePatch {
    fn apply(&self, t: &mut Theme) {
        let set = |dst: &mut Rgb, src: Option<Hex>| {
            if let Some(Hex(c)) = src {
                *dst = c;
            }
        };
        set(&mut t.fg, self.foreground);
        set(&mut t.bg, self.background);
        set(&mut t.cursor, self.cursor);
        set(&mut t.selection, self.selection);
        for i in 0..8 {
            set(&mut t.ansi[i], self.normal.map(|n| n[i]));
            set(&mut t.ansi[i + 8], self.bright.map(|b| b[i]));
        }
    }
}

const fn patch(fg: u32, bg: u32, cursor: u32, selection: u32) -> ThemePatch {
    ThemePatch {
        foreground: Some(Hex(rgb(fg))),
        background: Some(Hex(rgb(bg))),
        cursor: Some(Hex(rgb(cursor))),
        selection: Some(Hex(rgb(selection))),
        normal: None,
        bright: None,
    }
}

/// Built-in themes, layered on top of `default`.
fn builtin(name: &str) -> Option<ThemePatch> {
    Some(match name {
        "danger" => patch(0xf4dede, 0x2b0d12, 0xff5c6c, 0x5a1f28),
        "root" => patch(0xf2e6d4, 0x261808, 0xffb347, 0x5a3a14),
        "careful" => patch(0xe4dcf2, 0x1c1628, 0xc792ea, 0x3d2f5c),
        "night" => patch(0xb8b2a6, 0x0b0b0d, 0xd4a373, 0x2e2a24),
        _ => return None,
    })
}

pub const BUILTIN_NAMES: [&str; 5] = ["default", "danger", "root", "careful", "night"];

/// Final palettes by name: `default` (+ user tweaks), then every other theme as
/// default + built-in colors + user tweaks.
pub fn resolve_all(user: &HashMap<String, ThemePatch>) -> HashMap<String, Theme> {
    let mut base = Theme::default();
    if let Some(p) = user.get("default") {
        p.apply(&mut base);
    }
    let names = BUILTIN_NAMES.iter().map(|s| s.to_string()).chain(user.keys().cloned());
    names
        .map(|name| {
            let mut t = base.clone();
            if name != "default" {
                builtin(&name).inspect(|p| p.apply(&mut t));
                user.get(&name).inspect(|p| p.apply(&mut t));
            }
            (name, t)
        })
        .collect()
}

impl Theme {
    /// Resolve a cell color. `overrides` holds colors the app set via OSC 4/10/11.
    pub fn resolve(&self, color: Color, overrides: &Colors) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(i) => overrides[i as usize].unwrap_or_else(|| self.indexed(i)),
            Color::Named(name) => overrides[name as usize].unwrap_or_else(|| self.named(name)),
        }
    }

    fn indexed(&self, i: u8) -> Rgb {
        match i {
            0..16 => self.ansi[i as usize],
            16..232 => {
                // 6x6x6 color cube
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                let i = i - 16;
                Rgb { r: level(i / 36), g: level(i / 6 % 6), b: level(i % 6) }
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Rgb { r: v, g: v, b: v }
            }
        }
    }

    fn named(&self, name: NamedColor) -> Rgb {
        use NamedColor::*;
        match name {
            Foreground | BrightForeground => self.fg,
            Background => self.bg,
            Cursor => self.cursor,
            DimForeground => dim(self.fg),
            DimBlack | DimRed | DimGreen | DimYellow | DimBlue | DimMagenta | DimCyan | DimWhite => dim(self.ansi[name as usize - DimBlack as usize]),
            _ => self.ansi[name as usize],
        }
    }
}

impl Theme {
    /// Blend from `self` (t = 0) to `other` (t = 1).
    pub fn mix(&self, other: &Theme, t: f32) -> Theme {
        let m = |a, b| blend(a, b, t);
        Theme {
            fg: m(self.fg, other.fg),
            bg: m(self.bg, other.bg),
            cursor: m(self.cursor, other.cursor),
            selection: m(self.selection, other.selection),
            ansi: std::array::from_fn(|i| m(self.ansi[i], other.ansi[i])),
        }
    }
}

const FADE: Duration = Duration::from_millis(150);

/// A palette that cross-fades to a new target when switched.
pub struct Fade {
    target: Theme,
    from: Theme,
    start: Instant,
    /// The fully faded-in theme hasn't been drawn yet.
    pending: bool,
}

impl Fade {
    pub fn new(theme: Theme) -> Self {
        Self { from: theme.clone(), target: theme, start: Instant::now(), pending: false }
    }

    pub fn set(&mut self, theme: Theme) {
        if theme != self.target {
            self.from = self.current();
            self.start = Instant::now();
            self.target = theme;
            self.pending = true;
        }
    }

    /// The theme being faded to.
    pub fn target(&self) -> &Theme {
        &self.target
    }

    /// Needs more frames to finish fading.
    pub fn fading(&self) -> bool {
        self.pending
    }

    /// The palette to draw this frame.
    pub fn frame(&mut self) -> Theme {
        self.pending = self.start.elapsed() < FADE;
        self.current()
    }

    fn current(&self) -> Theme {
        let t = self.start.elapsed().as_secs_f32() / FADE.as_secs_f32();
        if t >= 1.0 { self.target.clone() } else { self.from.mix(&self.target, t) }
    }
}

/// `a` moved toward `b` by `t` (0..=1).
pub fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Rgb { r: ch(a.r, b.r), g: ch(a.g, b.g), b: ch(a.b, b.b) }
}

pub fn dim(c: Rgb) -> Rgb {
    Rgb { r: (c.r as u16 * 2 / 3) as u8, g: (c.g as u16 * 2 / 3) as u8, b: (c.b as u16 * 2 / 3) as u8 }
}

pub fn to_f32(c: Rgb) -> [f32; 4] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_indexed_and_named() {
        let t = Theme::default();
        let none = Colors::default();
        assert_eq!(t.resolve(Color::Indexed(16), &none), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(t.resolve(Color::Indexed(231), &none), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(t.resolve(Color::Indexed(196), &none), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(t.resolve(Color::Indexed(232), &none), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(t.resolve(Color::Named(NamedColor::Red), &none), t.ansi[1]);
        assert_eq!(t.resolve(Color::Named(NamedColor::Background), &none), t.bg);
        assert_eq!(t.resolve(Color::Named(NamedColor::DimRed), &none), dim(t.ansi[1]));
    }

    #[test]
    fn mixes_themes() {
        let (a, mut b) = (Theme::default(), Theme::default());
        b.bg = rgb(0xffffff);
        assert_eq!(a.mix(&b, 0.0), a);
        assert_eq!(a.mix(&b, 1.0), b);
        assert_eq!(a.mix(&b, 0.5).bg, Rgb { r: 0x80, g: 0x80, b: 0x80 });
    }

    #[test]
    fn layers_themes() {
        let user: HashMap<String, ThemePatch> = toml::from_str(
            r##"
            default = { foreground = "#010203" }
            danger = { cursor = "#00ff00" }
            mine = { background = "#0a0b0c" }
            "##,
        )
        .unwrap();
        let all = resolve_all(&user);
        // user tweak to default flows into themes that don't set their own foreground
        assert_eq!(all["default"].fg, rgb(0x010203));
        assert_eq!(all["mine"].fg, rgb(0x010203));
        // built-in danger background kept, user cursor applied, ANSI inherited from default
        assert_eq!(all["danger"].bg, rgb(0x2b0d12));
        assert_eq!(all["danger"].cursor, rgb(0x00ff00));
        assert_eq!(all["danger"].ansi, Theme::default().ansi);
        assert_eq!(all["mine"].bg, rgb(0x0a0b0c));
        assert_eq!(all.len(), 6);
    }
}
