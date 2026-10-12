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
    pub cell_w: f32,
    pub cell_h: f32,
    /// Distance from cell top to baseline.
    pub baseline: f32,
}

impl Font {
    pub fn new(family: &str, size: f32) -> Self {
        let mut font = Self {
            system: FontSystem::new(),
            cache: SwashCache::new(),
            family: family.to_owned(),
            size,
            cell_w: 0.0,
            cell_h: 0.0,
            baseline: 0.0,
        };
        font.measure();
        font
    }

    /// Switch family/size, keeping the already-scanned system font database.
    pub fn set(&mut self, family: &str, size: f32) {
        self.family = family.to_owned();
        self.size = size;
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
        let attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .weight(if bold { Weight::BOLD } else { Weight::NORMAL })
            .style(if italic { Style::Italic } else { Style::Normal });
        let mut buffer = Buffer::new_empty(Metrics::new(self.size, self.size));
        let mut tmp = [0u8; 4];
        buffer.set_text(c.encode_utf8(&mut tmp), &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.system, false);
        let run = buffer.layout_runs().next()?;
        let glyph = run.glyphs.first()?;
        Some((glyph.physical((0.0, 0.0), 1.0).cache_key, glyph.w))
    }

    pub fn rasterize(&mut self, c: char, bold: bool, italic: bool) -> Option<Glyph> {
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
