//! WebGPU's descriptors, as gpu.js sends them (JSON: camelCase members, string enums, wrappers as
//! `{"$": id}`), read into wgpu's. A member that is absent takes WebGPU's default.

use serde_json::{json, Value};

pub fn id(v: &Value) -> Option<u32> {
    v.get("$").and_then(Value::as_f64).map(|n| n as u32)
}

pub fn label(v: &Value) -> Option<&str> {
    v.get("label").and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub fn str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

pub fn u32_or(v: &Value, key: &str, default: u32) -> u32 {
    num(v, key).map(|n| n as u32).unwrap_or(default)
}

pub fn u64_or(v: &Value, key: &str, default: u64) -> u64 {
    num(v, key).map(|n| n as u64).unwrap_or(default)
}

pub fn f32_or(v: &Value, key: &str, default: f32) -> f32 {
    num(v, key).map(|n| n as f32).unwrap_or(default)
}

pub fn bool_or(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

// ── Formats ──────────────────────────────────────────────────────────────────────────────────

pub fn texture_format(name: &str) -> Result<wgpu::TextureFormat, String> {
    serde_json::from_value(Value::String(name.to_owned())).map_err(|_| format!("{name:?} is not a texture format"))
}

pub fn format_name(format: wgpu::TextureFormat) -> String {
    match serde_json::to_value(format) {
        Ok(Value::String(s)) => s,
        _ => format!("{format:?}"),
    }
}

pub fn formats(v: Option<&Value>) -> Result<Vec<wgpu::TextureFormat>, String> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| texture_format(f.as_str().unwrap_or_default()))
        .collect()
}

pub fn vertex_format(name: &str) -> Result<wgpu::VertexFormat, String> {
    // wgpu-types' names are WebGPU's but for the two with a separator.
    match name {
        "unorm10-10-10-2" => Ok(wgpu::VertexFormat::Unorm10_10_10_2),
        "unorm8x4-bgra" => Ok(wgpu::VertexFormat::Unorm8x4Bgra),
        _ => serde_json::from_value(Value::String(name.to_owned())).map_err(|_| format!("{name:?} is not a vertex format")),
    }
}

// ── Small enums ──────────────────────────────────────────────────────────────────────────────

fn address_mode(s: Option<&str>) -> wgpu::AddressMode {
    match s {
        Some("repeat") => wgpu::AddressMode::Repeat,
        Some("mirror-repeat") => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::ClampToEdge,
    }
}

fn filter_mode(s: Option<&str>) -> wgpu::FilterMode {
    match s {
        Some("linear") => wgpu::FilterMode::Linear,
        _ => wgpu::FilterMode::Nearest,
    }
}

fn compare(s: Option<&str>) -> Option<wgpu::CompareFunction> {
    Some(match s? {
        "never" => wgpu::CompareFunction::Never,
        "less" => wgpu::CompareFunction::Less,
        "equal" => wgpu::CompareFunction::Equal,
        "less-equal" => wgpu::CompareFunction::LessEqual,
        "greater" => wgpu::CompareFunction::Greater,
        "not-equal" => wgpu::CompareFunction::NotEqual,
        "greater-equal" => wgpu::CompareFunction::GreaterEqual,
        "always" => wgpu::CompareFunction::Always,
        _ => return None,
    })
}

fn view_dimension(s: Option<&str>) -> Option<wgpu::TextureViewDimension> {
    Some(match s? {
        "1d" => wgpu::TextureViewDimension::D1,
        "2d" => wgpu::TextureViewDimension::D2,
        "2d-array" => wgpu::TextureViewDimension::D2Array,
        "cube" => wgpu::TextureViewDimension::Cube,
        "cube-array" => wgpu::TextureViewDimension::CubeArray,
        "3d" => wgpu::TextureViewDimension::D3,
        _ => return None,
    })
}

pub fn texture_dimension(s: Option<&str>) -> wgpu::TextureDimension {
    match s {
        Some("1d") => wgpu::TextureDimension::D1,
        Some("3d") => wgpu::TextureDimension::D3,
        _ => wgpu::TextureDimension::D2,
    }
}

pub fn aspect(s: Option<&str>) -> wgpu::TextureAspect {
    match s {
        Some("stencil-only") => wgpu::TextureAspect::StencilOnly,
        Some("depth-only") => wgpu::TextureAspect::DepthOnly,
        _ => wgpu::TextureAspect::All,
    }
}

fn blend_factor(s: Option<&str>, default: wgpu::BlendFactor) -> wgpu::BlendFactor {
    use wgpu::BlendFactor as F;
    match s {
        Some("zero") => F::Zero,
        Some("one") => F::One,
        Some("src") => F::Src,
        Some("one-minus-src") => F::OneMinusSrc,
        Some("src-alpha") => F::SrcAlpha,
        Some("one-minus-src-alpha") => F::OneMinusSrcAlpha,
        Some("dst") => F::Dst,
        Some("one-minus-dst") => F::OneMinusDst,
        Some("dst-alpha") => F::DstAlpha,
        Some("one-minus-dst-alpha") => F::OneMinusDstAlpha,
        Some("src-alpha-saturated") => F::SrcAlphaSaturated,
        Some("constant") => F::Constant,
        Some("one-minus-constant") => F::OneMinusConstant,
        Some("src1") => F::Src1,
        Some("one-minus-src1") => F::OneMinusSrc1,
        Some("src1-alpha") => F::Src1Alpha,
        Some("one-minus-src1-alpha") => F::OneMinusSrc1Alpha,
        _ => default,
    }
}

fn blend_operation(s: Option<&str>) -> wgpu::BlendOperation {
    match s {
        Some("subtract") => wgpu::BlendOperation::Subtract,
        Some("reverse-subtract") => wgpu::BlendOperation::ReverseSubtract,
        Some("min") => wgpu::BlendOperation::Min,
        Some("max") => wgpu::BlendOperation::Max,
        _ => wgpu::BlendOperation::Add,
    }
}

fn stencil_operation(s: Option<&str>) -> wgpu::StencilOperation {
    use wgpu::StencilOperation as S;
    match s {
        Some("zero") => S::Zero,
        Some("replace") => S::Replace,
        Some("invert") => S::Invert,
        Some("increment-clamp") => S::IncrementClamp,
        Some("decrement-clamp") => S::DecrementClamp,
        Some("increment-wrap") => S::IncrementWrap,
        Some("decrement-wrap") => S::DecrementWrap,
        _ => S::Keep,
    }
}

pub fn store_op(s: Option<&str>) -> wgpu::StoreOp {
    match s {
        Some("discard") => wgpu::StoreOp::Discard,
        _ => wgpu::StoreOp::Store,
    }
}

// ── Plain structures ─────────────────────────────────────────────────────────────────────────

/// A GPUExtent3D (gpu.js sends the dictionary form).
pub fn extent(v: Option<&Value>) -> wgpu::Extent3d {
    let Some(v) = v else {
        return wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
    };
    if let Some(a) = v.as_array() {
        let at = |i: usize| a.get(i).and_then(Value::as_f64).map(|n| n as u32).unwrap_or(1);
        return wgpu::Extent3d { width: at(0), height: at(1), depth_or_array_layers: at(2) };
    }
    wgpu::Extent3d {
        width: u32_or(v, "width", 1),
        height: u32_or(v, "height", 1),
        depth_or_array_layers: u32_or(v, "depthOrArrayLayers", 1),
    }
}

pub fn origin(v: Option<&Value>) -> wgpu::Origin3d {
    match v {
        Some(v) if v.is_object() => wgpu::Origin3d { x: u32_or(v, "x", 0), y: u32_or(v, "y", 0), z: u32_or(v, "z", 0) },
        _ => wgpu::Origin3d::ZERO,
    }
}

pub fn texel_layout(v: &Value) -> wgpu::TexelCopyBufferLayout {
    wgpu::TexelCopyBufferLayout {
        offset: u64_or(v, "offset", 0),
        bytes_per_row: num(v, "bytesPerRow").map(|n| n as u32),
        rows_per_image: num(v, "rowsPerImage").map(|n| n as u32),
    }
}

pub fn color(v: Option<&Value>) -> wgpu::Color {
    match v {
        Some(Value::Array(a)) => {
            let at = |i: usize| a.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            wgpu::Color { r: at(0), g: at(1), b: at(2), a: at(3) }
        }
        Some(v) if v.is_object() => {
            let at = |k: &str| num(v, k).unwrap_or(0.0);
            wgpu::Color { r: at("r"), g: at("g"), b: at("b"), a: at("a") }
        }
        _ => wgpu::Color::TRANSPARENT,
    }
}

pub fn view_desc(d: &Value) -> Result<wgpu::TextureViewDescriptor<'_>, String> {
    Ok(wgpu::TextureViewDescriptor {
        label: label(d),
        format: str(d, "format").map(texture_format).transpose()?,
        dimension: view_dimension(str(d, "dimension")),
        usage: num(d, "usage").map(|n| wgpu::TextureUsages::from_bits_truncate(n as u32)).filter(|u| !u.is_empty()),
        aspect: aspect(str(d, "aspect")),
        base_mip_level: u32_or(d, "baseMipLevel", 0),
        mip_level_count: num(d, "mipLevelCount").map(|n| n as u32),
        base_array_layer: u32_or(d, "baseArrayLayer", 0),
        array_layer_count: num(d, "arrayLayerCount").map(|n| n as u32),
    })
}

pub fn sampler_desc(d: &Value) -> wgpu::SamplerDescriptor<'_> {
    wgpu::SamplerDescriptor {
        label: label(d),
        address_mode_u: address_mode(str(d, "addressModeU")),
        address_mode_v: address_mode(str(d, "addressModeV")),
        address_mode_w: address_mode(str(d, "addressModeW")),
        mag_filter: filter_mode(str(d, "magFilter")),
        min_filter: filter_mode(str(d, "minFilter")),
        mipmap_filter: filter_mode(str(d, "mipmapFilter")),
        lod_min_clamp: f32_or(d, "lodMinClamp", 0.0),
        lod_max_clamp: f32_or(d, "lodMaxClamp", 32.0),
        compare: compare(str(d, "compare")),
        anisotropy_clamp: u32_or(d, "maxAnisotropy", 1).clamp(1, 16) as u16,
        border_color: None,
    }
}

pub fn layout_entries(d: &Value) -> Result<Vec<wgpu::BindGroupLayoutEntry>, String> {
    let mut entries = Vec::new();
    for e in array(d, "entries") {
        let ty = if let Some(b) = e.get("buffer").filter(|v| v.is_object()) {
            wgpu::BindingType::Buffer {
                ty: match str(b, "type") {
                    Some("storage") => wgpu::BufferBindingType::Storage { read_only: false },
                    Some("read-only-storage") => wgpu::BufferBindingType::Storage { read_only: true },
                    _ => wgpu::BufferBindingType::Uniform,
                },
                has_dynamic_offset: bool_or(b, "hasDynamicOffset", false),
                min_binding_size: num(b, "minBindingSize").and_then(|n| wgpu::BufferSize::new(n as u64)),
            }
        } else if let Some(s) = e.get("sampler").filter(|v| v.is_object()) {
            wgpu::BindingType::Sampler(match str(s, "type") {
                Some("non-filtering") => wgpu::SamplerBindingType::NonFiltering,
                Some("comparison") => wgpu::SamplerBindingType::Comparison,
                _ => wgpu::SamplerBindingType::Filtering,
            })
        } else if let Some(t) = e.get("texture").filter(|v| v.is_object()) {
            wgpu::BindingType::Texture {
                sample_type: match str(t, "sampleType") {
                    Some("unfilterable-float") => wgpu::TextureSampleType::Float { filterable: false },
                    Some("depth") => wgpu::TextureSampleType::Depth,
                    Some("sint") => wgpu::TextureSampleType::Sint,
                    Some("uint") => wgpu::TextureSampleType::Uint,
                    _ => wgpu::TextureSampleType::Float { filterable: true },
                },
                view_dimension: view_dimension(str(t, "viewDimension")).unwrap_or(wgpu::TextureViewDimension::D2),
                multisampled: bool_or(t, "multisampled", false),
            }
        } else if let Some(t) = e.get("storageTexture").filter(|v| v.is_object()) {
            wgpu::BindingType::StorageTexture {
                access: match str(t, "access") {
                    Some("read-only") => wgpu::StorageTextureAccess::ReadOnly,
                    Some("read-write") => wgpu::StorageTextureAccess::ReadWrite,
                    _ => wgpu::StorageTextureAccess::WriteOnly,
                },
                format: texture_format(str(t, "format").unwrap_or_default())?,
                view_dimension: view_dimension(str(t, "viewDimension")).unwrap_or(wgpu::TextureViewDimension::D2),
            }
        } else {
            return Err(format!("binding {} has no resource layout", u32_or(e, "binding", 0)));
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: u32_or(e, "binding", 0),
            visibility: wgpu::ShaderStages::from_bits_truncate(u32_or(e, "visibility", 0)),
            ty,
            count: None,
        });
    }
    Ok(entries)
}

/// A stage's pipeline-overridable constants.
pub fn constants(stage: &Value) -> Vec<(String, f64)> {
    match stage.get("constants") {
        Some(Value::Object(map)) => map
            .iter()
            .filter_map(|(k, v)| v.as_f64().or_else(|| v.as_bool().map(|b| f64::from(u8::from(b)))).map(|n| (k.clone(), n)))
            .collect(),
        _ => Vec::new(),
    }
}

/// The vertex buffers' layouts: stride, step mode and attributes. A null entry (a slot the
/// pipeline leaves empty) is an empty layout.
pub fn vertex_buffers(vertex: &Value) -> Result<Vec<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)>, String> {
    let mut buffers = Vec::new();
    for b in array(vertex, "buffers") {
        if !b.is_object() {
            buffers.push((0, wgpu::VertexStepMode::Vertex, Vec::new()));
            continue;
        }
        let step_mode = match str(b, "stepMode") {
            Some("instance") => wgpu::VertexStepMode::Instance,
            _ => wgpu::VertexStepMode::Vertex,
        };
        let mut attributes = Vec::new();
        for a in array(b, "attributes") {
            attributes.push(wgpu::VertexAttribute {
                format: vertex_format(str(a, "format").unwrap_or_default())?,
                offset: u64_or(a, "offset", 0),
                shader_location: u32_or(a, "shaderLocation", 0),
            });
        }
        buffers.push((u64_or(b, "arrayStride", 0), step_mode, attributes));
    }
    Ok(buffers)
}

pub fn primitive(v: Option<&Value>) -> wgpu::PrimitiveState {
    let null = Value::Null;
    let v = v.unwrap_or(&null);
    wgpu::PrimitiveState {
        topology: match str(v, "topology") {
            Some("point-list") => wgpu::PrimitiveTopology::PointList,
            Some("line-list") => wgpu::PrimitiveTopology::LineList,
            Some("line-strip") => wgpu::PrimitiveTopology::LineStrip,
            Some("triangle-strip") => wgpu::PrimitiveTopology::TriangleStrip,
            _ => wgpu::PrimitiveTopology::TriangleList,
        },
        strip_index_format: match str(v, "stripIndexFormat") {
            Some("uint16") => Some(wgpu::IndexFormat::Uint16),
            Some("uint32") => Some(wgpu::IndexFormat::Uint32),
            _ => None,
        },
        front_face: match str(v, "frontFace") {
            Some("cw") => wgpu::FrontFace::Cw,
            _ => wgpu::FrontFace::Ccw,
        },
        cull_mode: match str(v, "cullMode") {
            Some("front") => Some(wgpu::Face::Front),
            Some("back") => Some(wgpu::Face::Back),
            _ => None,
        },
        unclipped_depth: bool_or(v, "unclippedDepth", false),
        polygon_mode: wgpu::PolygonMode::Fill,
        conservative: false,
    }
}

fn stencil_face(v: Option<&Value>) -> wgpu::StencilFaceState {
    let null = Value::Null;
    let v = v.unwrap_or(&null);
    wgpu::StencilFaceState {
        compare: compare(str(v, "compare")).unwrap_or(wgpu::CompareFunction::Always),
        fail_op: stencil_operation(str(v, "failOp")),
        depth_fail_op: stencil_operation(str(v, "depthFailOp")),
        pass_op: stencil_operation(str(v, "passOp")),
    }
}

pub fn depth_stencil(v: Option<&Value>) -> Result<Option<wgpu::DepthStencilState>, String> {
    let Some(v) = v.filter(|v| v.is_object()) else { return Ok(None) };
    Ok(Some(wgpu::DepthStencilState {
        format: texture_format(str(v, "format").unwrap_or_default())?,
        depth_write_enabled: bool_or(v, "depthWriteEnabled", false),
        depth_compare: compare(str(v, "depthCompare")).unwrap_or(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState {
            front: stencil_face(v.get("stencilFront")),
            back: stencil_face(v.get("stencilBack")),
            read_mask: u32_or(v, "stencilReadMask", 0xFFFF_FFFF),
            write_mask: u32_or(v, "stencilWriteMask", 0xFFFF_FFFF),
        },
        bias: wgpu::DepthBiasState {
            constant: num(v, "depthBias").map(|n| n as i32).unwrap_or(0),
            slope_scale: f32_or(v, "depthBiasSlopeScale", 0.0),
            clamp: f32_or(v, "depthBiasClamp", 0.0),
        },
    }))
}

pub fn multisample(v: Option<&Value>) -> wgpu::MultisampleState {
    let null = Value::Null;
    let v = v.unwrap_or(&null);
    wgpu::MultisampleState {
        count: u32_or(v, "count", 1),
        mask: u64_or(v, "mask", 0xFFFF_FFFF),
        alpha_to_coverage_enabled: bool_or(v, "alphaToCoverageEnabled", false),
    }
}

fn blend_component(v: Option<&Value>) -> wgpu::BlendComponent {
    let null = Value::Null;
    let v = v.unwrap_or(&null);
    wgpu::BlendComponent {
        src_factor: blend_factor(str(v, "srcFactor"), wgpu::BlendFactor::One),
        dst_factor: blend_factor(str(v, "dstFactor"), wgpu::BlendFactor::Zero),
        operation: blend_operation(str(v, "operation")),
    }
}

pub fn color_targets(fragment: &Value) -> Result<Vec<Option<wgpu::ColorTargetState>>, String> {
    let mut targets = Vec::new();
    for t in array(fragment, "targets") {
        if !t.is_object() {
            targets.push(None);
            continue;
        }
        let blend = t.get("blend").filter(|b| b.is_object()).map(|b| wgpu::BlendState {
            color: blend_component(b.get("color")),
            alpha: blend_component(b.get("alpha")),
        });
        targets.push(Some(wgpu::ColorTargetState {
            format: texture_format(str(t, "format").unwrap_or_default())?,
            blend,
            write_mask: wgpu::ColorWrites::from_bits_truncate(u32_or(t, "writeMask", 0xF)),
        }));
    }
    Ok(targets)
}

// ── Capabilities ─────────────────────────────────────────────────────────────────────────────

/// WebGPU's optional features and wgpu's names for them.
const FEATURES: [(&str, wgpu::Features); 15] = [
    ("depth-clip-control", wgpu::Features::DEPTH_CLIP_CONTROL),
    ("depth32float-stencil8", wgpu::Features::DEPTH32FLOAT_STENCIL8),
    ("texture-compression-bc", wgpu::Features::TEXTURE_COMPRESSION_BC),
    ("texture-compression-bc-sliced-3d", wgpu::Features::TEXTURE_COMPRESSION_BC_SLICED_3D),
    ("texture-compression-etc2", wgpu::Features::TEXTURE_COMPRESSION_ETC2),
    ("texture-compression-astc", wgpu::Features::TEXTURE_COMPRESSION_ASTC),
    ("texture-compression-astc-sliced-3d", wgpu::Features::TEXTURE_COMPRESSION_ASTC_SLICED_3D),
    ("timestamp-query", wgpu::Features::TIMESTAMP_QUERY),
    ("indirect-first-instance", wgpu::Features::INDIRECT_FIRST_INSTANCE),
    ("shader-f16", wgpu::Features::SHADER_F16),
    ("rg11b10ufloat-renderable", wgpu::Features::RG11B10UFLOAT_RENDERABLE),
    ("bgra8unorm-storage", wgpu::Features::BGRA8UNORM_STORAGE),
    ("float32-filterable", wgpu::Features::FLOAT32_FILTERABLE),
    ("dual-source-blending", wgpu::Features::DUAL_SOURCE_BLENDING),
    ("clip-distances", wgpu::Features::CLIP_DISTANCES),
];

pub fn feature(name: &str) -> Option<wgpu::Features> {
    FEATURES.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
}

pub fn feature_names(features: wgpu::Features) -> Vec<&'static str> {
    FEATURES.iter().filter(|(_, f)| features.contains(*f)).map(|(n, _)| *n).collect()
}

/// The limits under WebGPU's names (GPUSupportedLimits).
pub fn limits_json(l: &wgpu::Limits) -> Value {
    json!({
        "maxTextureDimension1D": l.max_texture_dimension_1d,
        "maxTextureDimension2D": l.max_texture_dimension_2d,
        "maxTextureDimension3D": l.max_texture_dimension_3d,
        "maxTextureArrayLayers": l.max_texture_array_layers,
        "maxBindGroups": l.max_bind_groups,
        "maxBindGroupsPlusVertexBuffers": l.max_bind_groups + l.max_vertex_buffers,
        "maxBindingsPerBindGroup": l.max_bindings_per_bind_group,
        "maxDynamicUniformBuffersPerPipelineLayout": l.max_dynamic_uniform_buffers_per_pipeline_layout,
        "maxDynamicStorageBuffersPerPipelineLayout": l.max_dynamic_storage_buffers_per_pipeline_layout,
        "maxSampledTexturesPerShaderStage": l.max_sampled_textures_per_shader_stage,
        "maxSamplersPerShaderStage": l.max_samplers_per_shader_stage,
        "maxStorageBuffersPerShaderStage": l.max_storage_buffers_per_shader_stage,
        "maxStorageTexturesPerShaderStage": l.max_storage_textures_per_shader_stage,
        "maxUniformBuffersPerShaderStage": l.max_uniform_buffers_per_shader_stage,
        "maxUniformBufferBindingSize": l.max_uniform_buffer_binding_size,
        "maxStorageBufferBindingSize": l.max_storage_buffer_binding_size,
        "minUniformBufferOffsetAlignment": l.min_uniform_buffer_offset_alignment,
        "minStorageBufferOffsetAlignment": l.min_storage_buffer_offset_alignment,
        "maxVertexBuffers": l.max_vertex_buffers,
        "maxBufferSize": l.max_buffer_size,
        "maxVertexAttributes": l.max_vertex_attributes,
        "maxVertexBufferArrayStride": l.max_vertex_buffer_array_stride,
        "maxInterStageShaderComponents": l.max_inter_stage_shader_components,
        "maxInterStageShaderVariables": l.max_inter_stage_shader_components / 4,
        "maxColorAttachments": l.max_color_attachments,
        "maxColorAttachmentBytesPerSample": l.max_color_attachment_bytes_per_sample,
        "maxComputeWorkgroupStorageSize": l.max_compute_workgroup_storage_size,
        "maxComputeInvocationsPerWorkgroup": l.max_compute_invocations_per_workgroup,
        "maxComputeWorkgroupSizeX": l.max_compute_workgroup_size_x,
        "maxComputeWorkgroupSizeY": l.max_compute_workgroup_size_y,
        "maxComputeWorkgroupSizeZ": l.max_compute_workgroup_size_z,
        "maxComputeWorkgroupsPerDimension": l.max_compute_workgroups_per_dimension,
    })
}

/// GPUAdapterInfo.
pub fn info_json(info: &wgpu::AdapterInfo) -> Value {
    let vendor = match info.vendor {
        0x10DE => "nvidia",
        0x1002 | 0x1022 => "amd",
        0x8086 => "intel",
        0x13B5 => "arm",
        0x5143 => "qualcomm",
        0x106B => "apple",
        _ => "",
    };
    json!({
        "vendor": vendor,
        "architecture": "",
        "device": "",
        "description": info.name,
        "isFallbackAdapter": info.device_type == wgpu::DeviceType::Cpu,
    })
}
