use std::collections::HashMap;

use crate::font::Font;

const SIZE: u32 = 2048;

/// Where a glyph lives in the atlas and how to place it in its cell.
#[derive(Clone, Copy)]
pub struct Entry {
    /// x, y, w, h in atlas pixels.
    pub rect: [f32; 4],
    pub left: f32,
    pub top: f32,
    pub color: bool,
}

type Key = (char, bool, bool);

/// RGBA glyph texture, packed in rows ("shelves").
/// Masks are stored as white with alpha so one texture serves text and color emoji.
pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    packer: Shelf,
    glyphs: HashMap<Key, Option<Entry>>,
}

impl Atlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph atlas"),
            size: wgpu::Extent3d { width: SIZE, height: SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view, packer: Shelf::new(SIZE), glyphs: HashMap::new() }
    }

    pub fn size(&self) -> f32 {
        SIZE as f32
    }

    /// Look up a glyph, rasterizing and uploading it on first use.
    /// `None` means the character has nothing to draw (e.g. a space).
    pub fn get(&mut self, queue: &wgpu::Queue, font: &mut Font, c: char, bold: bool, italic: bool) -> Option<Entry> {
        if let Some(entry) = self.glyphs.get(&(c, bold, italic)) {
            return *entry;
        }
        let entry = self.insert(queue, font, c, bold, italic);
        self.glyphs.insert((c, bold, italic), entry);
        entry
    }

    fn insert(&mut self, queue: &wgpu::Queue, font: &mut Font, c: char, bold: bool, italic: bool) -> Option<Entry> {
        let glyph = font.rasterize(c, bold, italic)?;
        if glyph.width == 0 || glyph.height == 0 {
            return None;
        }
        let (x, y) = match self.packer.alloc(glyph.width, glyph.height) {
            Some(pos) => pos,
            None => {
                // ponytail: full atlas → forget everything and start over. LRU eviction if this ever shows up in practice.
                log::warn!("glyph atlas full, resetting");
                self.packer = Shelf::new(SIZE);
                self.glyphs.clear();
                self.packer.alloc(glyph.width, glyph.height)?
            }
        };
        let rgba: Vec<u8> = if glyph.color {
            glyph.data
        } else {
            glyph.data.iter().flat_map(|&a| [255, 255, 255, a]).collect()
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(glyph.width * 4), rows_per_image: None },
            wgpu::Extent3d { width: glyph.width, height: glyph.height, depth_or_array_layers: 1 },
        );
        Some(Entry {
            rect: [x as f32, y as f32, glyph.width as f32, glyph.height as f32],
            left: glyph.left as f32,
            top: glyph.top as f32,
            color: glyph.color,
        })
    }
}

/// Shelf packer: fill a row left to right, start a new row when it runs out.
struct Shelf {
    size: u32,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    fn new(size: u32) -> Self {
        Self { size, x: 0, y: 0, row_h: 0 }
    }

    fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size {
            return None;
        }
        if self.x + w > self.size {
            self.x = 0;
            self.y += self.row_h + 1;
            self.row_h = 0;
        }
        if self.y + h > self.size {
            return None;
        }
        let pos = (self.x, self.y);
        self.x += w + 1; // 1px gap so linear sampling never bleeds between glyphs
        self.row_h = self.row_h.max(h);
        Some(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::Shelf;

    #[test]
    fn packs_rows_and_reports_full() {
        let mut s = Shelf::new(10);
        assert_eq!(s.alloc(4, 3), Some((0, 0)));
        assert_eq!(s.alloc(4, 5), Some((5, 0)));
        assert_eq!(s.alloc(4, 2), Some((0, 6))); // wraps below the tallest glyph
        assert_eq!(s.alloc(4, 5), None); // 6 + 5 > 10
        assert_eq!(s.alloc(11, 1), None);
    }
}
