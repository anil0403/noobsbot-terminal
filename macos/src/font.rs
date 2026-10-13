use cosmic_text::{Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, Style, SwashCache, SwashContent, Weight};

/// A rasterized glyph, positioned relative to the cell's baseline origin.
pub struct Glyph {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// RGBA for color emoji, otherwise a 1-byte alpha mask per pixel.
    pub color: bool,
    pub data: Vec<u8>,
}

pub struct Font {
    system: FontSystem,
    cache: SwashCache,
    family: String,
    size: f32,
    weight: u16,
    pub cell_w: f32,
    pub cell_h: f32,
    /// Distance from cell top to baseline.
    pub baseline: f32,
}

impl Font {
    pub fn new(family: &str, size: f32, weight: u16) -> Self {
        let mut font =
            Self { system: FontSystem::new(), cache: SwashCache::new(), family: String::new(), size, weight, cell_w: 0.0, cell_h: 0.0, baseline: 0.0 };
        font.set(family, size, weight);
        font
    }

    /// Switch family/size/weight, keeping the already-scanned system font database.
    pub fn set(&mut self, family: &str, size: f32, weight: u16) {
        // Not installed: Menlo, macOS's monospace, rather than a proportional fallback.
        let installed = self.system.db().faces().any(|f| f.families.iter().any(|(name, _)| name.eq_ignore_ascii_case(family)));
        if !installed {
            log::warn!("font {family:?} not installed; using Menlo");
        }
        self.family = if installed { family } else { "Menlo" }.to_owned();
        self.size = size;
        self.weight = weight.clamp(100, 900);
        self.measure();
    }

    fn measure(&mut self) {
        let (key, advance) = self.shape('M', false, false).expect("font has no 'M' glyph");
        let face = self.system.get_font(key.font_id, Weight::NORMAL).expect("load font face");
        let m = face.as_swash().metrics(&[]).scale(self.size);
        self.cell_w = advance.round();
        self.cell_h = (m.ascent + m.descent + m.leading).ceil();
        self.baseline = (m.ascent + m.leading / 2.0).round();
        log::info!("font {:?} {}px → cell {}x{}", self.family, self.size, self.cell_w, self.cell_h);
    }

    /// Shape one character, letting cosmic-text pick a fallback font if needed.
    fn shape(&mut self, c: char, bold: bool, italic: bool) -> Option<(CacheKey, f32)> {
        let weight = if bold { self.weight.max(700) } else { self.weight };
        let attrs = Attrs::new().family(Family::Name(&self.family)).weight(Weight(weight)).style(if italic { Style::Italic } else { Style::Normal });
        let mut buffer = Buffer::new_empty(Metrics::new(self.size, self.size));
        let mut tmp = [0u8; 4];
        buffer.set_text(c.encode_utf8(&mut tmp), &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        let run = buffer.layout_runs().next()?;
        let glyph = run.glyphs.first()?;
        Some((glyph.physical((0.0, 0.0), 1.0).cache_key, glyph.w))
    }

    pub fn rasterize(&mut self, c: char, bold: bool, italic: bool) -> Option<Glyph> {
        if let Some(glyph) = self.powerline(c) {
            return Some(glyph);
        }
        let (key, _) = self.shape(c, bold, italic)?;
        let image = self.cache.get_image_uncached(&mut self.system, key)?;
        Some(Glyph {
            left: image.placement.left,
            top: image.placement.top,
            width: image.placement.width,
            height: image.placement.height,
            color: matches!(image.content, SwashContent::Color),
            data: image.data,
        })
    }
}

/// Drawn as a cross in the bar icons' weight.
pub const CLOSE: char = '\u{10fffd}';

impl Font {
    /// Powerline separators (U+E0B0..=E0B7) drawn to fill the cell exactly:
    /// font versions rarely line up with the cells around them, leaving seams
    /// in prompts built from colored segments.
    fn powerline(&self, c: char) -> Option<Glyph> {
        let (w, h) = (self.cell_w as usize, self.cell_h as usize);
        let stroke = (self.cell_w / 9.0).max(1.0);
        // The bar icons' line weight.
        let icon = (self.cell_h / 11.0).round().max(1.0);
        // Distance from point p to segment a-b, in pixels.
        let seg = |p: (f32, f32), a: (f32, f32), b: (f32, f32)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
        };
        let (fw, fh) = (w as f32, h as f32);
        // Does pixel-space point (x, y) fall inside the shape?
        let inside: Box<dyn Fn(f32, f32) -> bool> = match c {
            '\u{e0b0}' => Box::new(move |x, y| x / fw <= 1.0 - (2.0 * y / fh - 1.0).abs()),
            '\u{e0b2}' => Box::new(move |x, y| x / fw >= (2.0 * y / fh - 1.0).abs()),
            '\u{e0b1}' => Box::new(move |x, y| seg((x, y), (0.0, 0.0), (fw, fh / 2.0)).min(seg((x, y), (fw, fh / 2.0), (0.0, fh))) <= stroke / 2.0),
            '\u{e0b3}' => Box::new(move |x, y| seg((x, y), (fw, 0.0), (0.0, fh / 2.0)).min(seg((x, y), (0.0, fh / 2.0), (fw, fh))) <= stroke / 2.0),
            // Half ellipses: the flat side on the cell edge, the round side bulging out.
            '\u{e0b4}' | '\u{e0b5}' | '\u{e0b6}' | '\u{e0b7}' => {
                let right = matches!(c, '\u{e0b4}' | '\u{e0b5}');
                let outline = matches!(c, '\u{e0b5}' | '\u{e0b7}');
                Box::new(move |x, y| {
                    let nx = if right { x / fw } else { 1.0 - x / fw };
                    let ny = 2.0 * y / fh - 1.0;
                    let r = (nx * nx + ny * ny).sqrt();
                    if outline { (r - 1.0).abs() * fw.min(fh / 2.0) <= stroke / 2.0 } else { r <= 1.0 }
                })
            }
            // The tab close button (a private codepoint, so terminal text never gets it).
            CLOSE => {
                let (c, h) = (fw / 2.0, (fh * 0.17).round());
                let (top, bot) = (fh / 2.0 - h, fh / 2.0 + h);
                Box::new(move |x, y| seg((x, y), (c - h, top), (c + h, bot)).min(seg((x, y), (c + h, top), (c - h, bot))) <= icon / 2.0)
            }
            _ => return None,
        };
        // 4x4 samples per pixel for smooth edges.
        let mut data = vec![0u8; w * h];
        for py in 0..h {
            for px in 0..w {
                let hits = (0..16).filter(|i| inside(px as f32 + (i % 4) as f32 / 4.0 + 0.125, py as f32 + (i / 4) as f32 / 4.0 + 0.125)).count();
                data[py * w + px] = (hits * 255 / 16) as u8;
            }
        }
        Some(Glyph { left: 0, top: self.baseline as i32, width: w as u32, height: h as u32, color: false, data })
    }
}
