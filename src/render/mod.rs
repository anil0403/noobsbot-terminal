mod atlas;
pub mod crt;

use std::collections::HashMap;
use std::sync::Arc;

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb};
use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::font::Font;
use crate::images::{self, Images};
use crate::pane::Rect;
use crate::term::Term;
use crate::theme::{self, Theme};
use atlas::Atlas;
use crt::Crt;

const SOLID: u32 = 0;
const MASK: u32 = 1;
const COLOR: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    pos: [f32; 2],
    size: [f32; 2],
    uv: [f32; 4],
    color: [f32; 4],
    kind: u32,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    atlas: Atlas,
    pub font: Font,
    /// Quads for the frame being built.
    quads: Vec<Instance>,
    /// The window has keyboard focus.
    pub focused: bool,
    /// Blink phase; the app toggles it on a timer.
    pub cursor_on: bool,
    /// Inline images: GPU copies by picture uid, and this frame's draws
    /// (position among `quads`, picture, quad).
    textures: HashMap<u64, Texture>,
    image_draws: Vec<(usize, u64, Instance)>,
    image_sampler: wgpu::Sampler,
    crt: Crt,
    /// CRT effect strengths when CRT mode is on.
    pub crt_params: Option<crt::Params>,
    started: std::time::Instant,
}

impl Renderer {
    pub fn new(window: Arc<Window>, font: Font) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let size = window.inner_size();
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        log::info!("adapter: {:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default()))
            .expect("request device");
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface unsupported by adapter");
        // Terminal colors are sRGB bytes; write them as-is instead of gamma-converting.
        config.format = config.format.remove_srgb_suffix();
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("cells.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cells"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Uint32],
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let atlas = Atlas::new(&device);
        let sampler = device.create_sampler(&Default::default()); // nearest: glyphs sit on whole pixels
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let instances = instance_buffer(&device, 4096);

        let crt = Crt::new(&device, config.format);
        let image_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            textures: HashMap::new(),
            image_draws: Vec::new(),
            image_sampler,
            surface,
            device,
            queue,
            config,
            pipeline,
            globals,
            bind_group,
            instances,
            atlas,
            font,
            quads: Vec::new(),
            focused: true,
            cursor_on: true,
            crt,
            crt_params: None,
            started: std::time::Instant::now(),
        }
    }

    pub fn set_font(&mut self, family: &str, size: f32) {
        self.font.set(family, size);
        self.atlas.clear();
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    pub fn fill(&mut self, r: Rect, color: [f32; 4]) {
        self.quads.push(solid(r.x, r.y, r.w, r.h, color));
    }

    /// One line of UI text (tab titles); returns the x where it ends.
    pub fn text(&mut self, mut x: f32, y: f32, text: &str, color: [f32; 4]) -> f32 {
        for c in text.chars() {
            if c != ' ' {
                if let Some(e) = self.atlas.get(&self.queue, &mut self.font, c, false, false) {
                    self.quads.push(Instance {
                        pos: [x + e.left, y + self.font.baseline - e.top],
                        size: [e.rect[2], e.rect[3]],
                        uv: e.rect,
                        color,
                        kind: if e.color { COLOR } else { MASK },
                    });
                }
            }
            x += self.font.cell_w;
        }
        x
    }

    /// Draw everything queued since the last frame on a `bg`-cleared window.
    pub fn finish(&mut self, bg: Rgb) {
        let instances = std::mem::take(&mut self.quads);
        let [r, g, b, _] = theme::to_f32(bg);
        let bg = wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: 1.0 };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };

        // Image quads go after the cells', drawn in between them in order below.
        let draws = std::mem::take(&mut self.image_draws);
        let mut instances = instances;
        let cells = instances.len();
        instances.extend(draws.iter().map(|d| d.2));
        for (_, uid, _) in &draws {
            let t = &self.textures[uid];
            let globals = [self.config.width as f32, self.config.height as f32, t.size.0, t.size.1];
            self.queue.write_buffer(&t.globals, 0, bytemuck::cast_slice(&globals));
        }
        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        if bytes.len() as u64 > self.instances.size() {
            self.instances = instance_buffer(&self.device, instances.len().next_power_of_two());
        }
        self.queue.write_buffer(&self.instances, 0, bytes);
        let globals = [self.config.width as f32, self.config.height as f32, self.atlas.size(), self.atlas.size()];
        self.queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));

        let frame_view = frame.texture.create_view(&Default::default());
        // With CRT on, cells go to a texture first and the effect pass draws that to the window.
        let view = match self.crt_params {
            Some(_) => self.crt.target(&self.device, (self.config.width, self.config.height)).clone(),
            None => frame_view.clone(),
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(bg), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            let mut done = 0;
            for (i, (at, uid, _)) in draws.iter().enumerate() {
                pass.draw(0..4, done as u32..*at as u32);
                done = *at;
                pass.set_bind_group(0, &self.textures[uid].bind_group, &[]);
                let n = (cells + i) as u32;
                pass.draw(0..4, n..n + 1);
                pass.set_bind_group(0, &self.bind_group, &[]);
            }
            pass.draw(0..4, done as u32..cells as u32);
        }
        // Keep only the textures still on screen; scrolling back re-uploads.
        self.textures.retain(|_, t| std::mem::take(&mut t.used));
        if let Some(params) = self.crt_params {
            let screen = [self.config.width as f32, self.config.height as f32];
            let time = self.started.elapsed().as_secs_f32();
            self.crt.draw(&self.queue, &mut encoder, &frame_view, crt::Params { screen, time, ..params });
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        instances.clear();
        self.quads = instances;
        self.image_draws = draws;
        self.image_draws.clear();
    }

    /// Queue one pane's grid: backgrounds and lines first, glyphs on top.
    /// Only the `active` pane gets a live cursor; the others are dimmed.
    /// Search `matches` are highlighted, `current` most strongly; the hovered `link` is underlined.
    /// Images tagged on the grid are drawn over their cells.
    #[allow(clippy::too_many_arguments)]
    pub fn pane(&mut self, term: &Term, images: &Images, theme: &Theme, rect: Rect, active: bool, matches: &[Match], current: Option<&Match>, link: Option<&Match>) {
        let (cw, ch, baseline) = (self.font.cell_w, self.font.cell_h, self.font.baseline);
        let (ox, oy) = (rect.x, rect.y);
        self.fill(rect, theme::to_f32(theme.bg));
        let line_h = (ch / 14.0).round().max(1.0);
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cursor = content.cursor;
        let cursor_shape = match cursor.shape {
            CursorShape::Hidden => CursorShape::Hidden,
            _ if !active || !self.focused => CursorShape::HollowBlock,
            _ if !self.cursor_on => CursorShape::Hidden,
            shape => shape,
        };
        let cursor_color = theme::to_f32(theme.cursor);
        let mut quads = std::mem::take(&mut self.quads);
        let mut glyphs = Vec::new();
        // (screen line, placement, image row) for image cells on screen.
        let mut tags: Vec<(i32, u64, usize)> = Vec::new();
        for cell in content.display_iter {
            let flags = cell.flags;
            if let Some((n, row)) = images::tag(cell.cell) {
                let t = (cell.point.line.0 + offset, n, row);
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let x = ox + cell.point.column.0 as f32 * cw;
            let y = oy + (cell.point.line.0 + offset) as f32 * ch;
            let w = if flags.contains(Flags::WIDE_CHAR) { cw * 2.0 } else { cw };

            let mut fg = theme.resolve(cell.fg, content.colors);
            let mut bg = theme.resolve(cell.bg, content.colors);
            if flags.contains(Flags::DIM) {
                fg = theme::dim(fg);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if content.selection.is_some_and(|s| s.contains(cell.point)) {
                bg = theme.selection;
            }
            if let Some(m) = matches.iter().find(|m| m.contains(&cell.point)) {
                let strong = current == Some(m);
                (fg, bg) = (theme.bg, if strong { theme.cursor } else { theme.ansi[3] });
            }
            if cell.point == cursor.point {
                match cursor_shape {
                    CursorShape::Block => (fg, bg) = (theme.bg, theme.cursor),
                    CursorShape::Beam => glyphs.push(solid(x, y, line_h * 2.0, ch, cursor_color)),
                    CursorShape::Underline => glyphs.push(solid(x, y + ch - line_h * 2.0, w, line_h * 2.0, cursor_color)),
                    CursorShape::HollowBlock => {
                        glyphs.push(solid(x, y, w, line_h, cursor_color));
                        glyphs.push(solid(x, y + ch - line_h, w, line_h, cursor_color));
                        glyphs.push(solid(x, y, line_h, ch, cursor_color));
                        glyphs.push(solid(x + w - line_h, y, line_h, ch, cursor_color));
                    }
                    CursorShape::Hidden => {}
                }
            }
            if bg != theme.bg {
                quads.push(solid(x, y, w, ch, theme::to_f32(bg)));
            }
            if flags.contains(Flags::HIDDEN) {
                continue;
            }
            let fg = theme::to_f32(fg);
            if flags.intersects(Flags::ALL_UNDERLINES) || link.is_some_and(|l| l.contains(&cell.point)) {
                quads.push(solid(x, y + baseline + line_h, w, line_h, fg));
            }
            if flags.contains(Flags::STRIKEOUT) {
                quads.push(solid(x, y + baseline - (baseline / 3.0).round(), w, line_h, fg));
            }
            if cell.c == ' ' {
                continue;
            }
            let bold = flags.contains(Flags::BOLD);
            let italic = flags.contains(Flags::ITALIC);
            let Some(e) = self.atlas.get(&self.queue, &mut self.font, cell.c, bold, italic) else { continue };
            glyphs.push(Instance {
                pos: [x + e.left, y + baseline - e.top],
                size: [e.rect[2], e.rect[3]],
                uv: e.rect,
                color: fg,
                kind: if e.color { COLOR } else { MASK },
            });
        }
        // Cursor outlines were pushed into `glyphs` so they land on top of the text.
        // ponytail: every underline style draws as a single line; undercurl etc. when someone misses them.
        quads.append(&mut glyphs);
        self.quads = quads;
        for (line, n, row) in tags {
            let Some(placed) = images.placed.get(&n) else { continue };
            // This row's slice of the image, clipped to the pane's right edge.
            let (dw, dh) = (placed.cols * cw, placed.rows * ch);
            let top = row as f32 * ch;
            let slice_h = (dh - top).min(ch);
            let x = ox + placed.col as f32 * cw;
            let vis_w = dw.min(rect.x + rect.w - x);
            if slice_h <= 0.0 || vis_w <= 0.0 {
                continue;
            }
            let pic = &placed.pic;
            let (pw, ph) = (pic.w as f32, pic.h as f32);
            self.upload(pic);
            let quad = Instance {
                pos: [x, oy + line as f32 * ch],
                size: [vis_w, slice_h],
                uv: [0.0, top / dh * ph, vis_w / dw * pw, slice_h / dh * ph],
                color: [1.0; 4],
                kind: COLOR,
            };
            self.image_draws.push((self.quads.len(), pic.uid, quad));
        }
        let mut quads = std::mem::take(&mut self.quads);
        if !active {
            let [r, g, b, _] = theme::to_f32(theme.bg);
            quads.push(solid(rect.x, rect.y, rect.w, rect.h, [r, g, b, 0.4]));
        }
        self.quads = quads;
    }
}

struct Texture {
    /// The cells shader's globals, with this image's size in place of the atlas's.
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    size: (f32, f32),
    /// Drawn this frame.
    used: bool,
}

impl Renderer {
    /// Put a picture on the GPU unless it's there already.
    fn upload(&mut self, pic: &images::Pic) {
        if let Some(t) = self.textures.get_mut(&pic.uid) {
            t.used = true;
            return;
        }
        let size = wgpu::Extent3d { width: pic.w, height: pic.h, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &pic.rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(pic.w * 4), rows_per_image: None },
            size,
        );
        let view = texture.create_view(&Default::default());
        let globals = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("image globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.image_sampler) },
            ],
        });
        self.textures.insert(pic.uid, Texture { globals, bind_group, size: (pic.w as f32, pic.h as f32), used: true });
    }
}

fn solid(x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) -> Instance {
    Instance { pos: [x, y], size: [w, h], uv: [0.0; 4], color, kind: SOLID }
}

fn instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (count * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
