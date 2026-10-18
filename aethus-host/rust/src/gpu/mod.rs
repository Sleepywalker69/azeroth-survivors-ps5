//! WebGPU for the page (`../js/gpu.js`), carried out with wgpu.
//!
//! Every object the page holds is a wgpu object in one table, under the id its JS wrapper
//! carries. Descriptors arrive as JSON in WebGPU's own vocabulary and are read in `desc.rs`; a
//! command encoder arrives as one recording when it finishes and is replayed in `encode.rs`.
//!
//! Errors are WebGPU's: a bad descriptor makes an invalid object and an error for the device's
//! error scopes (or the log), never a failure of the call, so the client carries on as it would
//! in a browser. Only the calls gpu.js turns into exceptions or rejections return Err.

// Only the console draws its own cursor; the desktop has the window system's.
#[cfg_attr(not(target_env = "ps5"), allow(dead_code))]
pub mod cursor;
mod desc;
mod encode;

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use serde_json::{json, Value};

use crate::log::{say, warn};

pub use desc::format_name;

pub enum Obj {
    Adapter(wgpu::Adapter),
    Device(wgpu::Device),
    Queue(wgpu::Queue),
    Buffer(wgpu::Buffer),
    Texture(wgpu::Texture),
    View(wgpu::TextureView),
    Sampler(wgpu::Sampler),
    BindGroupLayout(wgpu::BindGroupLayout),
    PipelineLayout(wgpu::PipelineLayout),
    BindGroup(wgpu::BindGroup),
    ShaderModule(wgpu::ShaderModule),
    RenderPipeline(wgpu::RenderPipeline),
    ComputePipeline(wgpu::ComputePipeline),
    QuerySet(wgpu::QuerySet),
    CommandBuffer(wgpu::CommandBuffer),
}

impl Obj {
    fn kind(&self) -> &'static str {
        match self {
            Obj::Adapter(_) => "adapter",
            Obj::Device(_) => "device",
            Obj::Queue(_) => "queue",
            Obj::Buffer(_) => "buffer",
            Obj::Texture(_) => "texture",
            Obj::View(_) => "texture view",
            Obj::Sampler(_) => "sampler",
            Obj::BindGroupLayout(_) => "bind group layout",
            Obj::PipelineLayout(_) => "pipeline layout",
            Obj::BindGroup(_) => "bind group",
            Obj::ShaderModule(_) => "shader module",
            Obj::RenderPipeline(_) => "render pipeline",
            Obj::ComputePipeline(_) => "compute pipeline",
            Obj::QuerySet(_) => "query set",
            Obj::CommandBuffer(_) => "command buffer",
        }
    }
}

/// What the page's canvas context was configured with.
#[derive(Clone, PartialEq)]
struct Canvas {
    device: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    view_formats: Vec<wgpu::TextureFormat>,
    alpha: wgpu::CompositeAlphaMode,
}

/// Map and work-done completions: a token and, for a failure, its message.
type Completions = Arc<Mutex<Vec<(u32, Option<String>)>>>;

pub struct Gpu {
    instance: wgpu::Instance,
    surface: Option<wgpu::Surface<'static>>,
    objects: HashMap<u32, Obj>,
    next_id: u32,
    devices: Vec<u32>,
    canvas: Option<Canvas>,
    /// The surface's configuration and size, as last configured.
    configured: Option<(Canvas, u32, u32)>,
    /// The swapchain image the page is drawing this frame, until it is presented.
    frame: Option<wgpu::SurfaceTexture>,
    present_mode: wgpu::PresentMode,
    done: Completions,
    next_token: u32,
    /// Map and work-done requests not yet handed to the page.
    outstanding: usize,
    /// Each device's queue.
    queue_of: HashMap<u32, u32>,
    /// A cursor drawn over each frame, where the display has none of its own.
    pub cursor: Option<cursor::Overlay>,
    /// Messages of skipped commands, each logged once (then every thousandth time).
    skipped: RefCell<HashMap<String, u32>>,
}

/// Runs a future that wgpu's native backends have already completed.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        std::thread::yield_now();
    }
}

macro_rules! getters {
    ($($name:ident: $variant:ident => $ty:ty),* $(,)?) => {
        impl Gpu {
            $(
                pub(crate) fn $name(&self, id: u32) -> Result<&$ty, String> {
                    match self.objects.get(&id) {
                        Some(Obj::$variant(o)) => Ok(o),
                        Some(o) => Err(format!("object {id} is a {}, not a {}", o.kind(), stringify!($name))),
                        None => Err(format!("there is no {} {id}", stringify!($name))),
                    }
                }
            )*
        }
    };
}

getters! {
    adapter: Adapter => wgpu::Adapter,
    device: Device => wgpu::Device,
    queue: Queue => wgpu::Queue,
    buffer: Buffer => wgpu::Buffer,
    texture: Texture => wgpu::Texture,
    view: View => wgpu::TextureView,
    sampler: Sampler => wgpu::Sampler,
    bind_group_layout: BindGroupLayout => wgpu::BindGroupLayout,
    pipeline_layout: PipelineLayout => wgpu::PipelineLayout,
    bind_group: BindGroup => wgpu::BindGroup,
    shader_module: ShaderModule => wgpu::ShaderModule,
    render_pipeline: RenderPipeline => wgpu::RenderPipeline,
    compute_pipeline: ComputePipeline => wgpu::ComputePipeline,
    query_set: QuerySet => wgpu::QuerySet,
}

impl Gpu {
    /// `surface` makes the surface the canvas presents to, from the instance.
    pub fn new(
        backends: wgpu::Backends,
        present_mode: wgpu::PresentMode,
        surface: impl FnOnce(&wgpu::Instance) -> Option<wgpu::Surface<'static>>,
    ) -> Gpu {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::from_env_or_default()
        });
        let surface = surface(&instance);
        if surface.is_none() {
            warn!("gpu: there is no surface; frames are drawn but not shown");
        }
        Gpu {
            instance,
            surface,
            objects: HashMap::new(),
            next_id: 1,
            devices: Vec::new(),
            canvas: None,
            configured: None,
            frame: None,
            present_mode,
            done: Arc::new(Mutex::new(Vec::new())),
            next_token: 1,
            outstanding: 0,
            queue_of: HashMap::new(),
            cursor: None,
            skipped: RefCell::new(HashMap::new()),
        }
    }

    fn insert(&mut self, obj: Obj) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, obj);
        id
    }

    /// A token for a map or work-done request, which `poll` hands back once.
    fn token(&mut self) -> u32 {
        let token = self.next_token;
        self.next_token += 1;
        self.outstanding += 1;
        token
    }

    /// Notes a command or member that was left out, as the log's warning.
    fn skip(&self, message: String) {
        let mut skipped = self.skipped.borrow_mut();
        let count = skipped.entry(message.clone()).or_insert(0);
        *count += 1;
        if *count == 1 || *count % 1000 == 0 {
            warn!("gpu: {message} (left out, {count} time(s))");
        }
    }

    // ── Adapter and device ─────────────────────────────────────────────────────────────────

    /// The adapter as JSON `{ id, features, limits, info }`, or None when there is none.
    pub fn request_adapter(&mut self, options: &Value) -> Option<String> {
        let power_preference = match options.get("powerPreference").and_then(Value::as_str) {
            Some("low-power") => wgpu::PowerPreference::LowPower,
            _ => wgpu::PowerPreference::HighPerformance,
        };
        let request = self.instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference,
            force_fallback_adapter: false,
            compatible_surface: self.surface.as_ref(),
        });
        let adapter = match block_on(request) {
            Ok(adapter) => adapter,
            Err(e) => {
                warn!("gpu: no adapter: {e}");
                return None;
            }
        };
        let info = adapter.get_info();
        say!("gpu: adapter {} ({:?}; {} {})", info.name, info.backend, info.driver, info.driver_info);
        let result = json!({
            "features": desc::feature_names(adapter.features()),
            "limits": desc::limits_json(&adapter.limits()),
            "info": desc::info_json(&info),
        });
        let id = self.insert(Obj::Adapter(adapter));
        let mut result = result;
        result["id"] = json!(id);
        Some(result.to_string())
    }

    /// A device and its queue, as JSON `{ id, queue, features, limits, info }`.
    pub fn request_device(&mut self, adapter_id: u32, d: &Value) -> Result<String, String> {
        let adapter = self.adapter(adapter_id)?;
        let mut features = wgpu::Features::empty();
        for name in d.get("requiredFeatures").and_then(Value::as_array).into_iter().flatten() {
            let name = name.as_str().unwrap_or_default();
            features |= desc::feature(name).ok_or_else(|| format!("the feature {name:?} is not supported here"))?;
        }
        if !adapter.features().contains(features) {
            return Err(format!("the adapter lacks {:?}", features - adapter.features()));
        }
        // The adapter's best limits: whatever the page asked for is within them, and the page
        // reads the device's limits back from what is returned here.
        let request = adapter.request_device(&wgpu::DeviceDescriptor {
            label: desc::label(d),
            required_features: features,
            required_limits: adapter.limits(),
            memory_hints: wgpu::MemoryHints::Performance,
            ..Default::default()
        });
        let (device, queue) = block_on(request).map_err(|e| e.to_string())?;
        let errors = AtomicU32::new(0);
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let n = errors.fetch_add(1, Ordering::Relaxed) + 1;
            if n <= 50 || n % 1000 == 0 {
                crate::log::line(2, &format!("gpu: uncaptured error #{n}: {e}"));
            }
        }));
        let info = self.adapter(adapter_id)?.get_info();
        let result = json!({
            "features": desc::feature_names(device.features()),
            "limits": desc::limits_json(&device.limits()),
            "info": desc::info_json(&info),
        });
        let id = self.insert(Obj::Device(device));
        let queue = self.insert(Obj::Queue(queue));
        self.queue_of.insert(id, queue);
        self.devices.push(id);
        say!("gpu: device {id} with {features:?}");
        let mut result = result;
        result["id"] = json!(id);
        result["queue"] = json!(queue);
        Ok(result.to_string())
    }

    /// The canvas format the page should use: one the surface takes.
    pub fn preferred_format(&self) -> String {
        "bgra8unorm".into()
    }

    // ── Objects ────────────────────────────────────────────────────────────────────────────

    /// Creates a `kind` of object from `parent` (a device, or the texture of a view, or the
    /// pipeline of a bind group layout) and its descriptor; the new object's id.
    pub fn create(&mut self, parent: u32, kind: &str, d: &Value) -> Result<u32, String> {
        let obj = match kind {
            "buffer" => Obj::Buffer(self.device(parent)?.create_buffer(&wgpu::BufferDescriptor {
                label: desc::label(d),
                size: desc::u64_or(d, "size", 0),
                usage: wgpu::BufferUsages::from_bits_truncate(desc::u32_or(d, "usage", 0)),
                mapped_at_creation: desc::bool_or(d, "mappedAtCreation", false),
            })),
            "texture" => {
                let view_formats = desc::formats(d.get("viewFormats"))?;
                Obj::Texture(self.device(parent)?.create_texture(&wgpu::TextureDescriptor {
                    label: desc::label(d),
                    size: desc::extent(d.get("size")),
                    mip_level_count: desc::u32_or(d, "mipLevelCount", 1),
                    sample_count: desc::u32_or(d, "sampleCount", 1),
                    dimension: desc::texture_dimension(desc::str(d, "dimension")),
                    format: desc::texture_format(desc::str(d, "format").unwrap_or_default())?,
                    usage: wgpu::TextureUsages::from_bits_truncate(desc::u32_or(d, "usage", 0)),
                    view_formats: &view_formats,
                }))
            }
            "view" => Obj::View(self.texture(parent)?.create_view(&desc::view_desc(d)?)),
            "sampler" => Obj::Sampler(self.device(parent)?.create_sampler(&desc::sampler_desc(d))),
            "bind_group_layout" => {
                let entries = desc::layout_entries(d)?;
                Obj::BindGroupLayout(self.device(parent)?.create_bind_group_layout(
                    &wgpu::BindGroupLayoutDescriptor { label: desc::label(d), entries: &entries },
                ))
            }
            "pipeline_layout" => Obj::PipelineLayout(self.create_pipeline_layout(parent, d)?),
            "bind_group" => Obj::BindGroup(self.create_bind_group(parent, d)?),
            "shader_module" => {
                let code = desc::str(d, "code").unwrap_or_default();
                Obj::ShaderModule(self.device(parent)?.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: desc::label(d),
                    source: wgpu::ShaderSource::Wgsl(code.into()),
                }))
            }
            "render_pipeline" => Obj::RenderPipeline(self.create_render_pipeline(parent, d)?),
            "compute_pipeline" => Obj::ComputePipeline(self.create_compute_pipeline(parent, d)?),
            "query_set" => {
                let ty = match desc::str(d, "type") {
                    Some("timestamp") => wgpu::QueryType::Timestamp,
                    _ => wgpu::QueryType::Occlusion,
                };
                Obj::QuerySet(self.device(parent)?.create_query_set(&wgpu::QuerySetDescriptor {
                    label: desc::label(d),
                    ty,
                    count: desc::u32_or(d, "count", 1),
                }))
            }
            "pipeline_bind_group_layout" => {
                let index = desc::u32_or(d, "index", 0);
                match self.objects.get(&parent) {
                    Some(Obj::RenderPipeline(p)) => Obj::BindGroupLayout(p.get_bind_group_layout(index)),
                    Some(Obj::ComputePipeline(p)) => Obj::BindGroupLayout(p.get_bind_group_layout(index)),
                    _ => return Err(format!("there is no pipeline {parent}")),
                }
            }
            _ => return Err(format!("objects of the kind {kind:?} are not made here")),
        };
        Ok(self.insert(obj))
    }

    fn create_pipeline_layout(&self, device: u32, d: &Value) -> Result<wgpu::PipelineLayout, String> {
        let device = self.device(device)?;
        // A null entry is an empty layout; wgpu has no gaps, so one is made for it.
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[] });
        let mut layouts = Vec::new();
        for v in d.get("bindGroupLayouts").and_then(Value::as_array).into_iter().flatten() {
            match desc::id(v) {
                Some(id) => layouts.push(self.bind_group_layout(id)?),
                None => layouts.push(&empty),
            }
        }
        Ok(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: desc::label(d),
            bind_group_layouts: &layouts,
            push_constant_ranges: &[],
        }))
    }

    fn create_bind_group(&self, device: u32, d: &Value) -> Result<wgpu::BindGroup, String> {
        let device = self.device(device)?;
        let layout = self.bind_group_layout(desc::id(&d["layout"]).unwrap_or(0))?;
        let list = d.get("entries").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        // A texture bound directly stands for its default view, made first so the entries can
        // borrow it.
        let mut default_views = Vec::new();
        for e in list {
            if let Some(Obj::Texture(t)) = desc::id(&e["resource"]).and_then(|id| self.objects.get(&id)) {
                default_views.push(Some(t.create_view(&wgpu::TextureViewDescriptor::default())));
            } else {
                default_views.push(None);
            }
        }
        let mut entries = Vec::new();
        for (e, default_view) in list.iter().zip(&default_views) {
            let binding = desc::u32_or(e, "binding", 0);
            let r = &e["resource"];
            let resource = if let Some(view) = default_view {
                wgpu::BindingResource::TextureView(view)
            } else if let Some(id) = desc::id(r) {
                match self.objects.get(&id) {
                    Some(Obj::Sampler(s)) => wgpu::BindingResource::Sampler(s),
                    Some(Obj::View(v)) => wgpu::BindingResource::TextureView(v),
                    Some(Obj::Buffer(b)) => wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: b, offset: 0, size: None }),
                    Some(o) => return Err(format!("a {} cannot be bound", o.kind())),
                    None => return Err(format!("binding {binding} names no object ({id})")),
                }
            } else if let Some(id) = desc::id(&r["buffer"]) {
                wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: self.buffer(id)?,
                    offset: desc::u64_or(r, "offset", 0),
                    size: r.get("size").and_then(Value::as_f64).and_then(|s| wgpu::BufferSize::new(s as u64)),
                })
            } else {
                return Err(format!("binding {binding} has no resource"));
            };
            entries.push(wgpu::BindGroupEntry { binding, resource });
        }
        Ok(device.create_bind_group(&wgpu::BindGroupDescriptor { label: desc::label(d), layout, entries: &entries }))
    }

    fn pipeline_layout_of(&self, d: &Value) -> Result<Option<&wgpu::PipelineLayout>, String> {
        match d.get("layout") {
            Some(Value::String(s)) if s == "auto" => Ok(None),
            Some(v) => match desc::id(v) {
                Some(id) => Ok(Some(self.pipeline_layout(id)?)),
                None => Ok(None),
            },
            None => Ok(None),
        }
    }

    fn create_render_pipeline(&self, device: u32, d: &Value) -> Result<wgpu::RenderPipeline, String> {
        let device = self.device(device)?;
        let layout = self.pipeline_layout_of(d)?;

        let vertex = &d["vertex"];
        let vertex_module = self.shader_module(desc::id(&vertex["module"]).unwrap_or(0))?;
        let vertex_constants = desc::constants(vertex);
        let vertex_constants: Vec<(&str, f64)> = vertex_constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let buffers = desc::vertex_buffers(vertex)?;
        let buffer_layouts: Vec<wgpu::VertexBufferLayout> = buffers
            .iter()
            .map(|(stride, step_mode, attributes)| wgpu::VertexBufferLayout {
                array_stride: *stride,
                step_mode: *step_mode,
                attributes,
            })
            .collect();

        let fragment = d.get("fragment").filter(|f| f.is_object());
        let fragment_module = match fragment {
            Some(f) => Some(self.shader_module(desc::id(&f["module"]).unwrap_or(0))?),
            None => None,
        };
        let fragment_constants = fragment.map(desc::constants).unwrap_or_default();
        let fragment_constants: Vec<(&str, f64)> = fragment_constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let targets = match fragment {
            Some(f) => desc::color_targets(f)?,
            None => Vec::new(),
        };

        Ok(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: desc::label(d),
            layout,
            vertex: wgpu::VertexState {
                module: vertex_module,
                entry_point: desc::str(vertex, "entryPoint"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &vertex_constants,
                    zero_initialize_workgroup_memory: true,
                },
                buffers: &buffer_layouts,
            },
            primitive: desc::primitive(d.get("primitive")),
            depth_stencil: desc::depth_stencil(d.get("depthStencil"))?,
            multisample: desc::multisample(d.get("multisample")),
            fragment: match (fragment, fragment_module) {
                (Some(f), Some(module)) => Some(wgpu::FragmentState {
                    module,
                    entry_point: desc::str(f, "entryPoint"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &fragment_constants,
                        zero_initialize_workgroup_memory: true,
                    },
                    targets: &targets,
                }),
                _ => None,
            },
            multiview: None,
            cache: None,
        }))
    }

    fn create_compute_pipeline(&self, device: u32, d: &Value) -> Result<wgpu::ComputePipeline, String> {
        let device = self.device(device)?;
        let layout = self.pipeline_layout_of(d)?;
        let compute = &d["compute"];
        let module = self.shader_module(desc::id(&compute["module"]).unwrap_or(0))?;
        let constants = desc::constants(compute);
        let constants: Vec<(&str, f64)> = constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        Ok(device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: desc::label(d),
            layout,
            module,
            entry_point: desc::str(compute, "entryPoint"),
            compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, zero_initialize_workgroup_memory: true },
            cache: None,
        }))
    }

    /// `destroy()`: the buffer's, texture's or device's memory goes now; the wrapper stays.
    pub fn destroy(&mut self, id: u32) {
        match self.objects.get(&id) {
            Some(Obj::Buffer(b)) => b.destroy(),
            Some(Obj::Texture(t)) => t.destroy(),
            Some(Obj::Device(d)) => d.destroy(),
            _ => {}
        }
    }

    /// Wrappers the page let go of.
    pub fn drop_objects(&mut self, ids: impl IntoIterator<Item = u32>) {
        for id in ids {
            if self.objects.remove(&id).is_some_and(|o| matches!(o, Obj::Device(_))) {
                self.devices.retain(|d| *d != id);
            }
        }
    }

    // ── Queue ──────────────────────────────────────────────────────────────────────────────

    pub fn submit(&mut self, queue: u32, ids: impl IntoIterator<Item = u32>) -> Result<(), String> {
        let mut buffers = Vec::new();
        for id in ids {
            match self.objects.remove(&id) {
                Some(Obj::CommandBuffer(b)) => buffers.push(b),
                Some(other) => {
                    self.objects.insert(id, other);
                    self.skip(format!("object {id} was submitted but is not a command buffer"));
                }
                None => self.skip(format!("command buffer {id} was submitted twice or never finished")),
            }
        }
        self.queue(queue)?.submit(buffers);
        Ok(())
    }

    pub fn write_buffer(&self, queue: u32, buffer: u32, offset: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        self.queue(queue)?.write_buffer(self.buffer(buffer)?, offset, data);
        Ok(())
    }

    pub fn write_texture(&self, queue: u32, d: &Value, data: &[u8]) -> Result<(), String> {
        let destination = self.texel_copy_texture(&d["destination"])?;
        let layout = desc::texel_layout(&d["dataLayout"]);
        self.queue(queue)?.write_texture(destination, data, layout, desc::extent(d.get("size")));
        Ok(())
    }

    pub(crate) fn texel_copy_texture(&self, v: &Value) -> Result<wgpu::TexelCopyTextureInfo<'_>, String> {
        Ok(wgpu::TexelCopyTextureInfo {
            texture: self.texture(desc::id(&v["texture"]).unwrap_or(0))?,
            mip_level: desc::u32_or(v, "mipLevel", 0),
            origin: desc::origin(v.get("origin")),
            aspect: desc::aspect(desc::str(v, "aspect")),
        })
    }

    pub(crate) fn texel_copy_buffer(&self, v: &Value) -> Result<wgpu::TexelCopyBufferInfo<'_>, String> {
        Ok(wgpu::TexelCopyBufferInfo {
            buffer: self.buffer(desc::id(&v["buffer"]).unwrap_or(0))?,
            layout: desc::texel_layout(v),
        })
    }

    pub fn on_submitted_work_done(&mut self, queue: u32) -> Result<u32, String> {
        self.queue(queue)?;
        let token = self.token();
        let done = self.done.clone();
        self.queue(queue)?.on_submitted_work_done(move || done.lock().unwrap().push((token, None)));
        Ok(token)
    }

    // ── Mapping ────────────────────────────────────────────────────────────────────────────

    /// Checks a range of a buffer that wgpu would otherwise panic over.
    fn range(buffer: &wgpu::Buffer, offset: u64, size: u64) -> Result<std::ops::Range<u64>, String> {
        let end = offset.checked_add(size).filter(|&end| end <= buffer.size() && size > 0);
        end.map(|end| offset..end)
            .ok_or_else(|| format!("the range {offset}+{size} is outside the buffer ({} bytes)", buffer.size()))
    }

    pub fn map_async(&mut self, id: u32, mode: u32, offset: u64, size: u64) -> Result<u32, String> {
        let mode = match mode {
            1 => wgpu::MapMode::Read,
            2 => wgpu::MapMode::Write,
            _ => return Err(format!("{mode} is not a map mode")),
        };
        let buffer = self.buffer(id)?;
        let range = Self::range(buffer, offset, size)?;
        let token = self.token();
        let done = self.done.clone();
        self.buffer(id)?.slice(range).map_async(mode, move |result| {
            done.lock().unwrap().push((token, result.err().map(|e| e.to_string())));
        });
        Ok(token)
    }

    pub fn mapped_read(&self, id: u32, offset: u64, size: u64) -> Result<Vec<u8>, String> {
        if size == 0 {
            return Ok(Vec::new());
        }
        let buffer = self.buffer(id)?;
        let range = Self::range(buffer, offset, size)?;
        let view = buffer.slice(range).get_mapped_range();
        Ok(view.to_vec())
    }

    pub fn mapped_write(&self, id: u32, offset: u64, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }
        let buffer = self.buffer(id)?;
        let range = Self::range(buffer, offset, data.len() as u64)?;
        buffer.slice(range).get_mapped_range_mut().copy_from_slice(data);
        Ok(())
    }

    pub fn unmap(&self, id: u32) -> Result<(), String> {
        self.buffer(id)?.unmap();
        Ok(())
    }

    /// Lets the devices finish work and run callbacks, then hands the finished map and work-done
    /// requests to the page as JSON `[[token, error or null], ...]`, or '' for none.
    pub fn poll(&mut self) -> String {
        self.maintain();
        let done = std::mem::take(&mut *self.done.lock().unwrap());
        if done.is_empty() {
            return String::new();
        }
        self.outstanding -= done.len().min(self.outstanding);
        Value::Array(done.into_iter().map(|(token, error)| json!([token, error])).collect()).to_string()
    }

    /// Checks the devices once without waiting, so finished work is retired.
    pub fn maintain(&self) {
        for id in &self.devices {
            if let Some(Obj::Device(device)) = self.objects.get(id) {
                let _ = device.poll(wgpu::PollType::Poll);
            }
        }
    }

    /// Whether map or work-done requests are waiting on the GPU or on the page, so the devices
    /// should be polled soon.
    pub fn busy(&self) -> bool {
        self.outstanding > 0
    }

    // ── Error scopes ───────────────────────────────────────────────────────────────────────

    pub fn push_error_scope(&self, device: u32, filter: &str) -> Result<(), String> {
        let filter = match filter {
            "out-of-memory" => wgpu::ErrorFilter::OutOfMemory,
            "internal" => wgpu::ErrorFilter::Internal,
            _ => wgpu::ErrorFilter::Validation,
        };
        self.device(device)?.push_error_scope(filter);
        Ok(())
    }

    /// The scope's error as JSON `{ type, message }`, or '' for none.
    pub fn pop_error_scope(&self, device: u32) -> Result<String, String> {
        let error = block_on(self.device(device)?.pop_error_scope());
        Ok(match error {
            None => String::new(),
            Some(e) => {
                let kind = match &e {
                    wgpu::Error::OutOfMemory { .. } => "out-of-memory",
                    wgpu::Error::Internal { .. } => "internal",
                    _ => "validation",
                };
                json!({ "type": kind, "message": e.to_string() }).to_string()
            }
        })
    }

    // ── The canvas ─────────────────────────────────────────────────────────────────────────

    pub fn configure(&mut self, device: u32, c: &Value) -> Result<(), String> {
        self.device(device)?;
        let format = desc::texture_format(desc::str(c, "format").unwrap_or_default())?;
        let alpha = match desc::str(c, "alphaMode") {
            Some("premultiplied") => wgpu::CompositeAlphaMode::PreMultiplied,
            _ => wgpu::CompositeAlphaMode::Opaque,
        };
        self.canvas = Some(Canvas {
            device,
            format,
            usage: wgpu::TextureUsages::from_bits_truncate(desc::u32_or(c, "usage", 0x10)),
            view_formats: desc::formats(c.get("viewFormats"))?,
            alpha,
        });
        Ok(())
    }

    pub fn unconfigure(&mut self) {
        self.canvas = None;
        self.frame = None;
    }

    /// Configures the surface for the canvas at this size, unless it already is.
    fn configure_surface(&mut self, width: u32, height: u32) -> Result<(), String> {
        let canvas = self.canvas.clone().ok_or("the canvas context is not configured")?;
        if self.configured.as_ref().is_some_and(|(c, w, h)| *c == canvas && *w == width && *h == height) {
            return Ok(());
        }
        let surface = self.surface.as_ref().ok_or("there is no surface")?;
        let device = self.device(canvas.device)?;
        let adapter = self.objects.values().find_map(|o| match o {
            Obj::Adapter(a) => Some(a),
            _ => None,
        });
        let caps = adapter.map(|a| surface.get_capabilities(a));
        let mut usage = canvas.usage | wgpu::TextureUsages::RENDER_ATTACHMENT;
        let mut alpha = canvas.alpha;
        if let Some(caps) = &caps {
            if !caps.formats.contains(&canvas.format) {
                return Err(format!("the surface does not take {} (it takes {:?})", format_name(canvas.format), caps.formats));
            }
            if !caps.usages.contains(usage) {
                warn!("gpu: the surface does not take the usage {usage:?}; using {:?}", usage & caps.usages);
                usage &= caps.usages;
            }
            if !caps.alpha_modes.contains(&alpha) {
                alpha = caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto);
            }
        }
        surface.configure(
            device,
            &wgpu::SurfaceConfiguration {
                usage,
                format: canvas.format,
                width,
                height,
                present_mode: self.present_mode,
                desired_maximum_frame_latency: 2,
                alpha_mode: alpha,
                view_formats: canvas.view_formats.clone(),
            },
        );
        say!("gpu: the surface is {width}x{height} {} ({:?})", format_name(canvas.format), self.present_mode);
        self.configured = Some((canvas, width, height));
        Ok(())
    }

    /// The texture the page draws this frame into: the surface's next image, or when there is
    /// no surface image to be had, a texture of its own that is never shown.
    pub fn current_texture(&mut self, width: u32, height: u32) -> Result<u32, String> {
        self.frame = None;
        let frame = match self.configure_surface(width, height) {
            Ok(()) => self.acquire(),
            Err(e) => Err(e),
        };
        let texture = match frame {
            Ok(frame) => {
                let texture = frame.texture.clone();
                self.frame = Some(frame);
                texture
            }
            Err(e) => {
                self.skip(format!("no surface image ({e}); this frame is not shown"));
                let canvas = self.canvas.clone().ok_or("the canvas context is not configured")?;
                self.device(canvas.device)?.create_texture(&wgpu::TextureDescriptor {
                    label: Some("canvas (not shown)"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: canvas.format,
                    usage: canvas.usage | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &canvas.view_formats,
                })
            }
        };
        Ok(self.insert(Obj::Texture(texture)))
    }

    fn acquire(&mut self) -> Result<wgpu::SurfaceTexture, String> {
        let surface = self.surface.as_ref().ok_or("there is no surface")?;
        match surface.get_current_texture() {
            Ok(frame) => {
                if frame.suboptimal {
                    // Configured again at the next frame.
                    self.configured = None;
                }
                Ok(frame)
            }
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                let (_, w, h) = self.configured.take().ok_or("the surface is not configured")?;
                self.configure_surface(w, h)?;
                let surface = self.surface.as_ref().ok_or("there is no surface")?;
                surface.get_current_texture().map_err(|e| e.to_string())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Shows the frame the page drew.
    pub fn present(&mut self) {
        if let Some(frame) = self.frame.take() {
            if let (Some(overlay), Some(canvas)) = (self.cursor.as_mut(), self.canvas.as_ref()) {
                let queue = self.queue_of.get(&canvas.device).and_then(|q| self.objects.get(q));
                if let (Some(Obj::Device(device)), Some(Obj::Queue(queue))) = (self.objects.get(&canvas.device), queue) {
                    overlay.draw(device, queue, &frame.texture);
                }
            }
            frame.present();
        }
    }

    /// Gives up the surface's image without showing it (the page drew nothing into it).
    pub fn discard_frame(&mut self) {
        self.frame = None;
    }
}
