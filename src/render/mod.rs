mod atlas;

use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::CursorShape;
use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::font::Font;
use crate::term::Term;
use crate::theme::{self, Theme};
use atlas::Atlas;

const FADE: Duration = Duration::from_millis(150);

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
    /// Target palette; `fade_from` blends into it over FADE after a switch.
    theme: Theme,
    fade_from: Theme,
    fade_start: Instant,
    /// The fully faded-in theme hasn't been drawn yet.
    fade_pending: bool,
    pub focused: bool,
    /// Blink phase; the app toggles it on a timer.
    pub cursor_on: bool,
}

impl Renderer {
    pub fn new(window: Arc<Window>, font: Font, theme: Theme) -> Self {
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

        Self { surface, device, queue, config, pipeline, globals, bind_group, instances, atlas, font, fade_from: theme.clone(), theme, fade_start: Instant::now(), fade_pending: false, focused: true, cursor_on: true }
    }

    /// Switch palette with a short cross-fade.
    pub fn set_theme(&mut self, theme: Theme) {
        if theme != self.theme {
            self.fade_from = self.shown_theme();
            self.fade_start = Instant::now();
            self.theme = theme;
            self.fade_pending = true;
        }
    }

    pub fn fading(&self) -> bool {
        self.fade_pending
    }

    fn shown_theme(&self) -> Theme {
        let t = self.fade_start.elapsed().as_secs_f32() / FADE.as_secs_f32();
        if t >= 1.0 { self.theme.clone() } else { self.fade_from.mix(&self.theme, t) }
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

    pub fn render(&mut self, term: &Term) {
        let theme = self.shown_theme();
        self.fade_pending = self.fade_start.elapsed() < FADE;
        let instances = self.build(term, &theme);
        let [r, g, b, _] = theme::to_f32(theme.bg);
        let bg = wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: 1.0 };

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };

        let bytes: &[u8] = bytemuck::cast_slice(&instances);
        if bytes.len() as u64 > self.instances.size() {
            self.instances = instance_buffer(&self.device, instances.len().next_power_of_two());
        }
        self.queue.write_buffer(&self.instances, 0, bytes);
        let globals = [self.config.width as f32, self.config.height as f32, self.atlas.size(), self.atlas.size()];
        self.queue.write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));

        let view = frame.texture.create_view(&Default::default());
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
            pass.draw(0..4, 0..instances.len() as u32);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }

    /// Turn the visible grid into quads: backgrounds and lines first, glyphs on top.
    fn build(&mut self, term: &Term, theme: &Theme) -> Vec<Instance> {
        let (cw, ch, baseline) = (self.font.cell_w, self.font.cell_h, self.font.baseline);
        let line_h = (ch / 14.0).round().max(1.0);
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let cursor = content.cursor;
        let cursor_shape = match cursor.shape {
            _ if !self.cursor_on => CursorShape::Hidden,
            CursorShape::Hidden => CursorShape::Hidden,
            _ if !self.focused => CursorShape::HollowBlock,
            shape => shape,
        };
        let cursor_color = theme::to_f32(theme.cursor);
        let mut quads = Vec::new();
        let mut glyphs = Vec::new();
        let solid = |x: f32, y: f32, w: f32, h: f32, color: [f32; 4]| Instance {
            pos: [x, y],
            size: [w, h],
            uv: [0.0; 4],
            color,
            kind: SOLID,
        };
        for cell in content.display_iter {
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let x = cell.point.column.0 as f32 * cw;
            let y = (cell.point.line.0 + offset) as f32 * ch;
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
            if flags.intersects(Flags::ALL_UNDERLINES) {
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
        quads
    }
}

fn instance_buffer(device: &wgpu::Device, count: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (count * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
