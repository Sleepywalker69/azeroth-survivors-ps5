//! A cursor drawn over each frame before it is shown, for a display with no pointer of its own
//! (the console): the image the page set as its cursor, or an arrow for a cursor keyword.

use crate::ops::Cursor;

const SHADER: &str = r"
struct Rect { r: vec4<f32> }
@group(0) @binding(0) var<uniform> rect: Rect;
@group(0) @binding(1) var image: texture_2d<f32>;
@group(0) @binding(2) var linear: sampler;
struct Out { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let c = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    var out: Out;
    out.uv = c;
    out.position = vec4<f32>(mix(rect.r.x, rect.r.z, c.x), mix(rect.r.y, rect.r.w, c.y), 0.0, 1.0);
    return out;
}
@fragment fn fs(v: Out) -> @location(0) vec4<f32> {
    return textureSample(image, linear, v.uv);
}
";

/// Straight RGBA pixels and the hotspot, in CSS pixels.
struct Image {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
    hot_x: u32,
    hot_y: u32,
}

/// An arrow, white with a black edge, its hotspot the tip: for every cursor keyword.
fn arrow() -> Image {
    let (width, height) = (13u32, 20u32);
    let inside = |x: i32, y: i32| x >= 0 && y >= 0 && y < 18 && f64::from(x) <= f64::from(y) * 0.62 && f64::from(y) - f64::from(x) * 0.45 < 16.5;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let pixel = if !inside(x, y) {
                [0, 0, 0, 0]
            } else if [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().any(|(dx, dy)| !inside(x + dx, y + dy)) {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    Image { rgba, width, height, hot_x: 0, hot_y: 0 }
}

/// What is ready on the GPU: the pipeline for one target format and the bind group for one
/// image.
struct Drawn {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    rect: wgpu::Buffer,
    sampler: wgpu::Sampler,
    /// The image the bind group holds, by its generation.
    bound: Option<(u64, wgpu::BindGroup)>,
}

pub struct Overlay {
    image: Image,
    /// The image is the page's, not the arrow.
    custom: bool,
    /// Bumped each time the image changes.
    generation: u64,
    /// The page set the cursor 'none', or holds the pointer.
    hidden: bool,
    locked: bool,
    /// The cursor, physical pixels, and physical pixels to a CSS pixel.
    at: (f32, f32),
    scale: f32,
    drawn: Option<Drawn>,
}

impl Overlay {
    pub fn new(scale: f32) -> Overlay {
        Overlay { image: arrow(), custom: false, generation: 0, hidden: false, locked: false, at: (0.0, 0.0), scale, drawn: None }
    }

    pub fn set(&mut self, cursor: Cursor) {
        match cursor {
            Cursor::Named(name) => {
                self.hidden = name == "none";
                if !self.hidden && self.custom {
                    self.image = arrow();
                    self.custom = false;
                    self.generation += 1;
                }
            }
            Cursor::Image { rgba, width, height, hot_x, hot_y } => {
                if rgba.len() != (width as usize) * (height as usize) * 4 || width == 0 || height == 0 {
                    return;
                }
                self.hidden = false;
                self.image = Image { rgba, width, height, hot_x, hot_y };
                self.custom = true;
                self.generation += 1;
            }
        }
    }

    pub fn set_locked(&mut self, locked: bool) {
        self.locked = locked;
    }

    /// Where the cursor is, CSS pixels.
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.at = (x as f32 * self.scale, y as f32 * self.scale);
    }

    /// Draws the cursor into `target`, the frame about to be shown.
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) {
        if self.hidden || self.locked {
            return;
        }
        let format = target.format();
        if self.drawn.as_ref().is_none_or(|d| d.format != format) {
            self.drawn = Some(Self::prepare(device, format));
        }
        let Some(drawn) = self.drawn.as_mut() else { return };
        if drawn.bound.as_ref().is_none_or(|(generation, _)| *generation != self.generation) {
            let image = &self.image;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("cursor"),
                size: wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // The image's bytes are what the screen shows: decoded on a target that encodes.
                format: if format.is_srgb() { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &image.rgba,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.width * 4), rows_per_image: None },
                wgpu::Extent3d { width: image.width, height: image.height, depth_or_array_layers: 1 },
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cursor"),
                layout: &drawn.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: drawn.rect.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&drawn.sampler) },
                ],
            });
            drawn.bound = Some((self.generation, group));
        }
        let Some((_, group)) = drawn.bound.as_ref() else { return };

        let image = &self.image;
        let (tw, th) = (target.width() as f32, target.height() as f32);
        let x0 = self.at.0 - image.hot_x as f32 * self.scale;
        let y0 = self.at.1 - image.hot_y as f32 * self.scale;
        let (x1, y1) = (x0 + image.width as f32 * self.scale, y0 + image.height as f32 * self.scale);
        let rect = [x0 / tw * 2.0 - 1.0, 1.0 - y0 / th * 2.0, x1 / tw * 2.0 - 1.0, 1.0 - y1 / th * 2.0];
        let bytes: Vec<u8> = rect.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&drawn.rect, 0, &bytes);

        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cursor") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cursor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&drawn.pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..4, 0..1);
        }
        queue.submit([encoder.finish()]);
    }

    fn prepare(device: &wgpu::Device, format: wgpu::TextureFormat) -> Drawn {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cursor"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cursor"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let rect = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cursor rect"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cursor"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Drawn { format, pipeline, rect, sampler, bound: None }
    }
}
