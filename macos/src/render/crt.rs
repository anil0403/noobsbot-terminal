//! CRT mode: the frame is drawn into a texture, then onto the window through `crt.wgsl`.

use bytemuck::{Pod, Zeroable};

/// Effect strengths, 0 = off. Mirrors `Params` in crt.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Params {
    pub screen: [f32; 2],
    pub time: f32,
    pub scanlines: f32,
    pub glow: f32,
    pub curvature: f32,
    pub vignette: f32,
    pub flicker: f32,
}

/// Where a window pixel shows its content from, under the barrel distortion.
/// Mouse positions go through this so clicks land on what you see.
pub fn warp(curvature: f32, (w, h): (f32, f32), (x, y): (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (x / w * 2.0 - 1.0, y / h * 2.0 - 1.0);
    let s = (1.0 + curvature * (cx * cx + cy * cy)) / (1.0 + curvature);
    ((cx * s + 1.0) / 2.0 * w, (cy * s + 1.0) / 2.0 * h)
}

pub struct Crt {
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// Offscreen frame and its bind group, for the current window size.
    target: Option<(wgpu::TextureView, wgpu::BindGroup, (u32, u32))>,
    format: wgpu::TextureFormat,
}

impl Crt {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("crt.wgsl"));
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("crt"),
            layout: None,
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("crt params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Linear: curvature samples between pixels.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, params, sampler, target: None, format }
    }

    /// The texture to draw this frame into, (re)made to match the window.
    pub fn target(&mut self, device: &wgpu::Device, size: (u32, u32)) -> &wgpu::TextureView {
        if self.target.as_ref().is_none_or(|t| t.2 != size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("crt frame"),
                size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.target = Some((view, bind_group, size));
        }
        &self.target.as_ref().unwrap().0
    }

    /// Draw the offscreen frame onto `view` with the effect.
    pub fn draw(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, params: Params) {
        let Some((_, bind_group, _)) = &self.target else { return };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warp_keeps_center_and_edges() {
        let size = (800.0, 600.0);
        let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
        assert!(near(warp(0.1, size, (400.0, 300.0)), (400.0, 300.0)));
        assert!(near(warp(0.1, size, (800.0, 300.0)), (800.0, 300.0)));
        assert!(near(warp(0.1, size, (400.0, 0.0)), (400.0, 0.0)));
        // Corners show what's beyond the edge (black).
        assert!(warp(0.1, size, (800.0, 600.0)).0 > 800.0);
        assert!(near(warp(0.0, size, (123.0, 45.0)), (123.0, 45.0)));
        // In between, points are pulled toward the center.
        let (x, _) = warp(0.1, size, (600.0, 300.0));
        assert!(x < 600.0 && x > 400.0);
    }
}
