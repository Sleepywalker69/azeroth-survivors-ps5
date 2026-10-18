//! Command encoders. gpu.js records a whole encoder as numbers (an opcode, then its operands; see
//! the opcode table at the top of gpu.js) plus the descriptors the commands named, and hands both
//! over when the page calls `finish()`; they are replayed here into a wgpu command buffer.
//!
//! A command whose object is gone or whose range wgpu would panic over is left out and noted in
//! the log, as the page would see a validation error for it; the rest of the encoder still runs.
//! Only a recording that does not parse fails the call.

use serde_json::Value;

use super::{desc, Gpu, Obj};

const BEGIN_RENDER_PASS: u32 = 1;
const BEGIN_COMPUTE_PASS: u32 = 2;
const END_PASS: u32 = 3;
const SET_PIPELINE: u32 = 4;
const SET_BIND_GROUP: u32 = 5;
const SET_VERTEX_BUFFER: u32 = 6;
const SET_INDEX_BUFFER: u32 = 7;
const DRAW: u32 = 8;
const DRAW_INDEXED: u32 = 9;
const DRAW_INDIRECT: u32 = 10;
const DRAW_INDEXED_INDIRECT: u32 = 11;
const SET_VIEWPORT: u32 = 12;
const SET_SCISSOR_RECT: u32 = 13;
const SET_BLEND_CONSTANT: u32 = 14;
const SET_STENCIL_REFERENCE: u32 = 15;
const DISPATCH: u32 = 16;
const DISPATCH_INDIRECT: u32 = 17;
const COPY_BUFFER_TO_BUFFER: u32 = 18;
const COPY_TEXTURE_TO_BUFFER: u32 = 19;
const COPY_TEXTURE_TO_TEXTURE: u32 = 20;
const CLEAR_BUFFER: u32 = 21;
const RESOLVE_QUERY_SET: u32 = 22;
const COPY_BUFFER_TO_TEXTURE: u32 = 23;
const BEGIN_OCCLUSION_QUERY: u32 = 24;
const END_OCCLUSION_QUERY: u32 = 25;
const WRITE_TIMESTAMP: u32 = 26;
const PUSH_DEBUG_GROUP: u32 = 27;
const POP_DEBUG_GROUP: u32 = 28;
const INSERT_DEBUG_MARKER: u32 = 29;

/// The recording, read front to back.
struct Reader<'a> {
    cmds: &'a [f64],
    pos: usize,
    descs: &'a [Value],
}

impl<'a> Reader<'a> {
    fn done(&self) -> bool {
        self.pos >= self.cmds.len()
    }
    fn f64(&mut self) -> Result<f64, String> {
        let v = *self.cmds.get(self.pos).ok_or("the recording ends in the middle of a command")?;
        self.pos += 1;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(self.f64()? as u32)
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(self.f64()? as i32)
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(self.f64()? as u64)
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(self.f64()? as f32)
    }
    /// A size operand: -1 is "the rest".
    fn size(&mut self) -> Result<Option<u64>, String> {
        let v = self.f64()?;
        Ok((v >= 0.0).then_some(v as u64))
    }
    fn desc(&mut self) -> Result<&'a Value, String> {
        let i = self.u32()? as usize;
        self.descs.get(i).ok_or_else(|| format!("the recording names descriptor {i}, which it does not have"))
    }
    fn label(&mut self) -> Result<&'a str, String> {
        Ok(self.desc()?.as_str().unwrap_or_default())
    }
}

impl Gpu {
    /// Replays a recording; the new command buffer's id.
    pub fn encode(&mut self, device: u32, info: &Value, cmds: &[f64]) -> Result<u32, String> {
        let buffer = self.record(device, info, cmds)?;
        Ok(self.insert(Obj::CommandBuffer(buffer)))
    }

    fn record(&self, device: u32, info: &Value, cmds: &[f64]) -> Result<wgpu::CommandBuffer, String> {
        let device = self.device(device)?;
        let descs = info.get("descs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: desc::label(info) });
        let mut r = Reader { cmds, pos: 0, descs };
        while !r.done() {
            match r.u32()? {
                BEGIN_RENDER_PASS => {
                    let d = r.desc()?;
                    self.render_pass(&mut encoder, d, &mut r)?;
                }
                BEGIN_COMPUTE_PASS => {
                    let d = r.desc()?;
                    self.compute_pass(&mut encoder, d, &mut r)?;
                }
                COPY_BUFFER_TO_BUFFER => {
                    let (source, source_offset) = (r.u32()?, r.u64()?);
                    let (destination, destination_offset, size) = (r.u32()?, r.u64()?, r.size()?);
                    match (self.buffer(source), self.buffer(destination)) {
                        (Ok(s), Ok(d)) => encoder.copy_buffer_to_buffer(s, source_offset, d, destination_offset, size),
                        (Err(e), _) | (_, Err(e)) => self.skip(e),
                    }
                }
                COPY_BUFFER_TO_TEXTURE => {
                    let d = r.desc()?;
                    match (self.texel_copy_buffer(&d["source"]), self.texel_copy_texture(&d["destination"])) {
                        (Ok(s), Ok(t)) => encoder.copy_buffer_to_texture(s, t, desc::extent(d.get("size"))),
                        (Err(e), _) | (_, Err(e)) => self.skip(e),
                    }
                }
                COPY_TEXTURE_TO_BUFFER => {
                    let d = r.desc()?;
                    match (self.texel_copy_texture(&d["source"]), self.texel_copy_buffer(&d["destination"])) {
                        (Ok(t), Ok(b)) => encoder.copy_texture_to_buffer(t, b, desc::extent(d.get("size"))),
                        (Err(e), _) | (_, Err(e)) => self.skip(e),
                    }
                }
                COPY_TEXTURE_TO_TEXTURE => {
                    let d = r.desc()?;
                    match (self.texel_copy_texture(&d["source"]), self.texel_copy_texture(&d["destination"])) {
                        (Ok(s), Ok(t)) => encoder.copy_texture_to_texture(s, t, desc::extent(d.get("size"))),
                        (Err(e), _) | (_, Err(e)) => self.skip(e),
                    }
                }
                CLEAR_BUFFER => {
                    let (id, offset, size) = (r.u32()?, r.u64()?, r.size()?);
                    match self.buffer(id) {
                        Ok(b) => encoder.clear_buffer(b, offset, size),
                        Err(e) => self.skip(e),
                    }
                }
                RESOLVE_QUERY_SET => {
                    let (set, first, count) = (r.u32()?, r.u32()?, r.u32()?);
                    let (destination, offset) = (r.u32()?, r.u64()?);
                    match (self.query_set(set), self.buffer(destination)) {
                        (Ok(q), Ok(b)) => encoder.resolve_query_set(q, first..first.saturating_add(count), b, offset),
                        (Err(e), _) | (_, Err(e)) => self.skip(e),
                    }
                }
                WRITE_TIMESTAMP => {
                    let (set, index) = (r.u32()?, r.u32()?);
                    // Outside passes only with wgpu's own feature; WebGPU dropped the command.
                    if device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS) {
                        match self.query_set(set) {
                            Ok(q) => encoder.write_timestamp(q, index),
                            Err(e) => self.skip(e),
                        }
                    }
                }
                PUSH_DEBUG_GROUP => encoder.push_debug_group(r.label()?),
                POP_DEBUG_GROUP => encoder.pop_debug_group(),
                INSERT_DEBUG_MARKER => encoder.insert_debug_marker(r.label()?),
                op => return Err(format!("command {op} is not one an encoder takes outside a pass")),
            }
        }
        Ok(encoder.finish())
    }

    /// A slice wgpu takes without panicking, or None for an empty or out-of-range one.
    fn slice(&self, id: u32, offset: u64, size: Option<u64>) -> Result<Option<wgpu::BufferSlice<'_>>, String> {
        let buffer = self.buffer(id)?;
        let end = match size {
            Some(size) => offset.checked_add(size),
            None => Some(buffer.size()),
        };
        Ok(match end {
            Some(end) if end > offset && end <= buffer.size() => Some(buffer.slice(offset..end)),
            _ => {
                if end != Some(offset) {
                    self.skip(format!("the range {offset}..{end:?} is outside buffer {id} ({} bytes)", buffer.size()));
                }
                None
            }
        })
    }

    fn render_pass(&self, encoder: &mut wgpu::CommandEncoder, d: &Value, r: &mut Reader) -> Result<(), String> {
        let mut colors = Vec::new();
        for a in d.get("colorAttachments").and_then(Value::as_array).into_iter().flatten() {
            if !a.is_object() {
                colors.push(None);
                continue;
            }
            let view = match self.view(desc::id(&a["view"]).unwrap_or(0)) {
                Ok(view) => view,
                Err(e) => {
                    self.skip(format!("a color attachment: {e}"));
                    colors.push(None);
                    continue;
                }
            };
            let load = match desc::str(a, "loadOp") {
                Some("clear") => wgpu::LoadOp::Clear(desc::color(a.get("clearValue"))),
                _ => wgpu::LoadOp::Load,
            };
            colors.push(Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: a.get("depthSlice").and_then(Value::as_f64).map(|n| n as u32),
                resolve_target: desc::id(&a["resolveTarget"]).and_then(|id| self.view(id).ok()),
                ops: wgpu::Operations { load, store: desc::store_op(desc::str(a, "storeOp")) },
            }));
        }
        let depth_stencil = d.get("depthStencilAttachment").filter(|a| a.is_object()).and_then(|a| {
            let view = match self.view(desc::id(&a["view"]).unwrap_or(0)) {
                Ok(view) => view,
                Err(e) => {
                    self.skip(format!("a depth attachment: {e}"));
                    return None;
                }
            };
            let depth_ops = (!desc::bool_or(a, "depthReadOnly", false) && a.get("depthLoadOp").is_some()).then(|| {
                wgpu::Operations {
                    load: match desc::str(a, "depthLoadOp") {
                        Some("clear") => wgpu::LoadOp::Clear(desc::f32_or(a, "depthClearValue", 0.0)),
                        _ => wgpu::LoadOp::Load,
                    },
                    store: desc::store_op(desc::str(a, "depthStoreOp")),
                }
            });
            let stencil_ops = (!desc::bool_or(a, "stencilReadOnly", false) && a.get("stencilLoadOp").is_some()).then(|| {
                wgpu::Operations {
                    load: match desc::str(a, "stencilLoadOp") {
                        Some("clear") => wgpu::LoadOp::Clear(desc::u32_or(a, "stencilClearValue", 0)),
                        _ => wgpu::LoadOp::Load,
                    },
                    store: desc::store_op(desc::str(a, "stencilStoreOp")),
                }
            });
            Some(wgpu::RenderPassDepthStencilAttachment { view, depth_ops, stencil_ops })
        });
        let timestamp_writes = d.get("timestampWrites").filter(|t| t.is_object()).and_then(|t| {
            Some(wgpu::RenderPassTimestampWrites {
                query_set: self.query_set(desc::id(&t["querySet"])?).ok()?,
                beginning_of_pass_write_index: t.get("beginningOfPassWriteIndex").and_then(Value::as_f64).map(|n| n as u32),
                end_of_pass_write_index: t.get("endOfPassWriteIndex").and_then(Value::as_f64).map(|n| n as u32),
            })
        });
        let occlusion_query_set = d.get("occlusionQuerySet").and_then(desc::id).and_then(|id| self.query_set(id).ok());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: desc::label(d),
            color_attachments: &colors,
            depth_stencil_attachment: depth_stencil,
            timestamp_writes,
            occlusion_query_set,
        });

        loop {
            if r.done() {
                return Err("the recording ends inside a render pass".into());
            }
            match r.u32()? {
                END_PASS => return Ok(()),
                SET_PIPELINE => match self.render_pipeline(r.u32()?) {
                    Ok(p) => pass.set_pipeline(p),
                    Err(e) => self.skip(e),
                },
                SET_BIND_GROUP => {
                    let (index, id, count) = (r.u32()?, r.u32()?, r.u32()? as usize);
                    let mut offsets = Vec::with_capacity(count);
                    for _ in 0..count {
                        offsets.push(r.u32()?);
                    }
                    if id == 0 {
                        pass.set_bind_group(index, None, &offsets);
                    } else {
                        match self.bind_group(id) {
                            Ok(g) => pass.set_bind_group(index, Some(g), &offsets),
                            Err(e) => self.skip(e),
                        }
                    }
                }
                SET_VERTEX_BUFFER => {
                    let (slot, id, offset, size) = (r.u32()?, r.u32()?, r.u64()?, r.size()?);
                    // wgpu cannot unbind a slot; a null buffer leaves it as it was.
                    if id != 0 {
                        match self.slice(id, offset, size) {
                            Ok(Some(slice)) => pass.set_vertex_buffer(slot, slice),
                            Ok(None) => {}
                            Err(e) => self.skip(e),
                        }
                    }
                }
                SET_INDEX_BUFFER => {
                    let id = r.u32()?;
                    let format = if r.u32()? == 1 { wgpu::IndexFormat::Uint32 } else { wgpu::IndexFormat::Uint16 };
                    let (offset, size) = (r.u64()?, r.size()?);
                    match self.slice(id, offset, size) {
                        Ok(Some(slice)) => pass.set_index_buffer(slice, format),
                        Ok(None) => {}
                        Err(e) => self.skip(e),
                    }
                }
                DRAW => {
                    let (vertices, instances, first_vertex, first_instance) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                    pass.draw(
                        first_vertex..first_vertex.saturating_add(vertices),
                        first_instance..first_instance.saturating_add(instances),
                    );
                }
                DRAW_INDEXED => {
                    let (indices, instances, first_index) = (r.u32()?, r.u32()?, r.u32()?);
                    let (base_vertex, first_instance) = (r.i32()?, r.u32()?);
                    pass.draw_indexed(
                        first_index..first_index.saturating_add(indices),
                        base_vertex,
                        first_instance..first_instance.saturating_add(instances),
                    );
                }
                DRAW_INDIRECT => {
                    let (id, offset) = (r.u32()?, r.u64()?);
                    match self.buffer(id) {
                        Ok(b) => pass.draw_indirect(b, offset),
                        Err(e) => self.skip(e),
                    }
                }
                DRAW_INDEXED_INDIRECT => {
                    let (id, offset) = (r.u32()?, r.u64()?);
                    match self.buffer(id) {
                        Ok(b) => pass.draw_indexed_indirect(b, offset),
                        Err(e) => self.skip(e),
                    }
                }
                SET_VIEWPORT => {
                    let (x, y, w, h) = (r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                    let (min_depth, max_depth) = (r.f32()?, r.f32()?);
                    pass.set_viewport(x, y, w, h, min_depth, max_depth);
                }
                SET_SCISSOR_RECT => {
                    let (x, y, w, h) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                    pass.set_scissor_rect(x, y, w, h);
                }
                SET_BLEND_CONSTANT => {
                    let (red, g, b, a) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?);
                    pass.set_blend_constant(wgpu::Color { r: red, g, b, a });
                }
                SET_STENCIL_REFERENCE => pass.set_stencil_reference(r.u32()?),
                BEGIN_OCCLUSION_QUERY => pass.begin_occlusion_query(r.u32()?),
                END_OCCLUSION_QUERY => pass.end_occlusion_query(),
                PUSH_DEBUG_GROUP => pass.push_debug_group(r.label()?),
                POP_DEBUG_GROUP => pass.pop_debug_group(),
                INSERT_DEBUG_MARKER => pass.insert_debug_marker(r.label()?),
                op => return Err(format!("command {op} is not one a render pass takes")),
            }
        }
    }

    fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder, d: &Value, r: &mut Reader) -> Result<(), String> {
        let timestamp_writes = d.get("timestampWrites").filter(|t| t.is_object()).and_then(|t| {
            Some(wgpu::ComputePassTimestampWrites {
                query_set: self.query_set(desc::id(&t["querySet"])?).ok()?,
                beginning_of_pass_write_index: t.get("beginningOfPassWriteIndex").and_then(Value::as_f64).map(|n| n as u32),
                end_of_pass_write_index: t.get("endOfPassWriteIndex").and_then(Value::as_f64).map(|n| n as u32),
            })
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: desc::label(d), timestamp_writes });
        loop {
            if r.done() {
                return Err("the recording ends inside a compute pass".into());
            }
            match r.u32()? {
                END_PASS => return Ok(()),
                SET_PIPELINE => match self.compute_pipeline(r.u32()?) {
                    Ok(p) => pass.set_pipeline(p),
                    Err(e) => self.skip(e),
                },
                SET_BIND_GROUP => {
                    let (index, id, count) = (r.u32()?, r.u32()?, r.u32()? as usize);
                    let mut offsets = Vec::with_capacity(count);
                    for _ in 0..count {
                        offsets.push(r.u32()?);
                    }
                    if id == 0 {
                        pass.set_bind_group(index, None, &offsets);
                    } else {
                        match self.bind_group(id) {
                            Ok(g) => pass.set_bind_group(index, Some(g), &offsets),
                            Err(e) => self.skip(e),
                        }
                    }
                }
                DISPATCH => {
                    let (x, y, z) = (r.u32()?, r.u32()?, r.u32()?);
                    pass.dispatch_workgroups(x, y, z);
                }
                DISPATCH_INDIRECT => {
                    let (id, offset) = (r.u32()?, r.u64()?);
                    match self.buffer(id) {
                        Ok(b) => pass.dispatch_workgroups_indirect(b, offset),
                        Err(e) => self.skip(e),
                    }
                }
                PUSH_DEBUG_GROUP => pass.push_debug_group(r.label()?),
                POP_DEBUG_GROUP => pass.pop_debug_group(),
                INSERT_DEBUG_MARKER => pass.insert_debug_marker(r.label()?),
                op => return Err(format!("command {op} is not one a compute pass takes")),
            }
        }
    }
}
