use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};

/// A color palette. M4 picks one of these per pane based on the situation.
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
            fg: rgb(0xd9dee8),
            bg: rgb(0x121317),
            cursor: rgb(0xf5c2e7),
            selection: rgb(0x3b4261),
            ansi: [
                rgb(0x1d1f27), rgb(0xf7768e), rgb(0x9ece6a), rgb(0xe0af68),
                rgb(0x7aa2f7), rgb(0xbb9af7), rgb(0x7dcfff), rgb(0xc0caf5),
                rgb(0x565f89), rgb(0xff899d), rgb(0xb9f27c), rgb(0xffc777),
                rgb(0x8db0ff), rgb(0xc7a9ff), rgb(0xa4daff), rgb(0xe9ecf5),
            ],
        }
    }
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
            DimBlack | DimRed | DimGreen | DimYellow | DimBlue | DimMagenta | DimCyan | DimWhite => {
                dim(self.ansi[name as usize - DimBlack as usize])
            }
            _ => self.ansi[name as usize],
        }
    }
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
}
