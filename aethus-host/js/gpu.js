// WebGPU for the web build, carried out by the embedder's own wgpu.
//
// wgpu's WebGPU backend inside the wasm drives `navigator.gpu` exactly as it would in a browser.
// Every object here is a thin wrapper around a native wgpu object that the embedder keeps in a
// table under a numeric id; descriptors cross as JSON in WebGPU's own vocabulary (camelCase
// members, string enums), with each wrapper written as `{"$": id}`. A wrapper that the page
// drops is released with `gpu_drop` once QuickJS collects it.
//
// Command encoders do not call out per command: a frame issues thousands of draws and bind
// group changes, so passes are recorded as numbers into one array and replayed natively when the
// encoder finishes (see the opcodes below). Map and work-done promises settle from `gpu_poll`
// at the start of a later task, as a browser's would.
//
// The canvas context's texture is the embedder's swapchain image; it is presented after the
// animation frame that asked for it, as a browser presents a WebGPU canvas.
'use strict';

(() => {
  const G = globalThis;
  const A = G.__aethus;
  const { rust, define, queueTask, reportError } = A;

  // ── Recorded commands: an opcode, then its operands. `desc` operands index the encoder's
  //    descriptor list, which crosses as JSON; ids are wrapper ids; -1 is "the rest". ─────────

  const OP = {
    BEGIN_RENDER_PASS: 1, // desc
    BEGIN_COMPUTE_PASS: 2, // desc
    END_PASS: 3,
    SET_PIPELINE: 4, // id
    SET_BIND_GROUP: 5, // index, id (0: none), count, offsets...
    SET_VERTEX_BUFFER: 6, // slot, id (0: none), offset, size
    SET_INDEX_BUFFER: 7, // id, format (0 uint16, 1 uint32), offset, size
    DRAW: 8, // vertexCount, instanceCount, firstVertex, firstInstance
    DRAW_INDEXED: 9, // indexCount, instanceCount, firstIndex, baseVertex, firstInstance
    DRAW_INDIRECT: 10, // id, offset
    DRAW_INDEXED_INDIRECT: 11, // id, offset
    SET_VIEWPORT: 12, // x, y, width, height, minDepth, maxDepth
    SET_SCISSOR_RECT: 13, // x, y, width, height
    SET_BLEND_CONSTANT: 14, // r, g, b, a
    SET_STENCIL_REFERENCE: 15, // reference
    DISPATCH: 16, // x, y, z
    DISPATCH_INDIRECT: 17, // id, offset
    COPY_BUFFER_TO_BUFFER: 18, // source, sourceOffset, destination, destinationOffset, size
    COPY_TEXTURE_TO_BUFFER: 19, // desc { source, destination, size }
    COPY_TEXTURE_TO_TEXTURE: 20, // desc { source, destination, size }
    CLEAR_BUFFER: 21, // id, offset, size
    RESOLVE_QUERY_SET: 22, // querySet, firstQuery, queryCount, destination, destinationOffset
    COPY_BUFFER_TO_TEXTURE: 23, // desc { source, destination, size }
    BEGIN_OCCLUSION_QUERY: 24, // queryIndex
    END_OCCLUSION_QUERY: 25,
    WRITE_TIMESTAMP: 26, // querySet, queryIndex
    PUSH_DEBUG_GROUP: 27, // desc (the label)
    POP_DEBUG_GROUP: 28,
    INSERT_DEBUG_MARKER: 29, // desc (the label)
  };

  // ── Lifetimes ──────────────────────────────────────────────────────────────────────────────

  // Ids of collected wrappers, released in one call per tick.
  let dropped = [];
  const collected = new FinalizationRegistry((id) => dropped.push(id));
  function flushDrops() {
    if (!dropped.length) return;
    const ids = new Float64Array(dropped);
    dropped = [];
    rust('gpu_drop', ids);
  }

  function track(wrapper, id) {
    define(wrapper, '_id', id);
    if (id) collected.register(wrapper, id);
  }

  // Map and work-done promises, by the token the embedder handed out.
  const waiting = new Map();
  function settleLater(token) {
    return new Promise((resolve, reject) => waiting.set(token, { resolve, reject }));
  }

  function poll() {
    flushDrops();
    if (!waiting.size) return;
    const text = rust('gpu_poll');
    if (!text) return;
    for (const [token, error] of JSON.parse(text)) {
      const w = waiting.get(token);
      if (!w) continue;
      waiting.delete(token);
      if (error) w.reject(new DOMException(String(error), 'OperationError'));
      else w.resolve(undefined);
    }
  }
  A.tickHooks.push(poll);

  // ── Helpers ────────────────────────────────────────────────────────────────────────────────

  const json = (value) => JSON.stringify(value === undefined ? {} : value);

  // A GPUExtent3D as a dictionary.
  function extent(size) {
    if (size === undefined || size === null) return { width: 1, height: 1, depthOrArrayLayers: 1 };
    if (typeof size[Symbol.iterator] === 'function') {
      const s = Array.from(size);
      return { width: s[0], height: s[1] ?? 1, depthOrArrayLayers: s[2] ?? 1 };
    }
    return { width: size.width, height: size.height ?? 1, depthOrArrayLayers: size.depthOrArrayLayers ?? 1 };
  }
  // A GPUOrigin3D as a dictionary.
  function origin(o) {
    if (o === undefined || o === null) return undefined;
    if (typeof o[Symbol.iterator] === 'function') {
      const s = Array.from(o);
      return { x: s[0] ?? 0, y: s[1] ?? 0, z: s[2] ?? 0 };
    }
    return { x: o.x ?? 0, y: o.y ?? 0, z: o.z ?? 0 };
  }
  function texelCopyTexture(t) {
    return { texture: t.texture, mipLevel: t.mipLevel ?? 0, origin: origin(t.origin), aspect: t.aspect ?? 'all' };
  }
  function color(c) {
    if (typeof c[Symbol.iterator] === 'function') {
      const s = Array.from(c);
      return [s[0], s[1], s[2], s[3]];
    }
    return [c.r, c.g, c.b, c.a];
  }

  // Bytes of a BufferSource, with WebGPU's offset and size in elements of a typed array.
  function sourceBytes(data, offset, size) {
    let bytes;
    let unit = 1;
    if (data instanceof ArrayBuffer) bytes = new Uint8Array(data);
    else if (ArrayBuffer.isView(data)) {
      unit = data.BYTES_PER_ELEMENT || 1;
      bytes = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    } else throw new TypeError('The data is not an ArrayBuffer or an ArrayBufferView.');
    const start = (offset || 0) * unit;
    const length = size === undefined ? bytes.byteLength - start : size * unit;
    if (start < 0 || length < 0 || start + length > bytes.byteLength) {
      throw new DOMException('The data offset and size are outside the data.', 'OperationError');
    }
    return start === 0 && length === bytes.byteLength ? bytes : bytes.subarray(start, start + length);
  }

  // ── Errors ─────────────────────────────────────────────────────────────────────────────────

  class GPUError {
    constructor(message) {
      this.message = String(message ?? '');
    }
  }
  class GPUValidationError extends GPUError {}
  class GPUOutOfMemoryError extends GPUError {}
  class GPUInternalError extends GPUError {}
  function makeError(e) {
    if (!e) return null;
    switch (e.type) {
      case 'out-of-memory':
        return new GPUOutOfMemoryError(e.message);
      case 'internal':
        return new GPUInternalError(e.message);
      default:
        return new GPUValidationError(e.message);
    }
  }
  class GPUUncapturedErrorEvent extends Event {
    constructor(type, init) {
      super(type, init);
      this.error = init && init.error;
    }
  }

  // ── Capabilities ───────────────────────────────────────────────────────────────────────────

  class GPUSupportedFeatures {
    constructor(names) {
      define(this, '_set', new Set(names || []));
    }
    get size() {
      return this._set.size;
    }
    has(name) {
      return this._set.has(String(name));
    }
    keys() {
      return this._set.values();
    }
    values() {
      return this._set.values();
    }
    entries() {
      return this._set.entries();
    }
    forEach(fn, thisArg) {
      for (const v of this._set) fn.call(thisArg, v, v, this);
    }
    [Symbol.iterator]() {
      return this._set.values();
    }
  }
  class WGSLLanguageFeatures extends GPUSupportedFeatures {}

  class GPUSupportedLimits {
    constructor(values) {
      Object.assign(this, values || {});
    }
  }

  class GPUAdapterInfo {
    constructor(info) {
      info = info || {};
      this.vendor = String(info.vendor || '');
      this.architecture = String(info.architecture || '');
      this.device = String(info.device || '');
      this.description = String(info.description || '');
      this.subgroupMinSize = info.subgroupMinSize || 4;
      this.subgroupMaxSize = info.subgroupMaxSize || 128;
      this.isFallbackAdapter = !!info.isFallbackAdapter;
    }
  }

  // ── Objects ────────────────────────────────────────────────────────────────────────────────

  class GPUObjectBase {
    constructor(id, label) {
      track(this, id);
      this.label = label ? String(label) : '';
    }
    toJSON() {
      return { $: this._id };
    }
  }

  class GPUSampler extends GPUObjectBase {}
  class GPUTextureView extends GPUObjectBase {}
  class GPUBindGroupLayout extends GPUObjectBase {}
  class GPUBindGroup extends GPUObjectBase {}
  class GPUPipelineLayout extends GPUObjectBase {}
  class GPUCommandBuffer extends GPUObjectBase {}

  class GPUShaderModule extends GPUObjectBase {
    getCompilationInfo() {
      return Promise.resolve({ messages: [] });
    }
  }

  class GPUPipelineBase extends GPUObjectBase {
    getBindGroupLayout(index) {
      const id = rust('gpu_create', this._id, 'pipeline_bind_group_layout', json({ index: index >>> 0 }));
      return new GPUBindGroupLayout(id, '');
    }
  }
  class GPURenderPipeline extends GPUPipelineBase {}
  class GPUComputePipeline extends GPUPipelineBase {}

  class GPUQuerySet extends GPUObjectBase {
    constructor(id, desc) {
      super(id, desc.label);
      this.type = desc.type;
      this.count = desc.count >>> 0;
    }
    destroy() {
      rust('gpu_destroy', this._id);
    }
  }

  class GPUTexture extends GPUObjectBase {
    constructor(id, desc) {
      super(id, desc.label);
      const e = extent(desc.size);
      this.width = e.width;
      this.height = e.height;
      this.depthOrArrayLayers = e.depthOrArrayLayers;
      this.mipLevelCount = desc.mipLevelCount ?? 1;
      this.sampleCount = desc.sampleCount ?? 1;
      this.dimension = desc.dimension ?? '2d';
      this.format = desc.format;
      this.usage = desc.usage >>> 0;
      this.textureBindingViewDimension = desc.textureBindingViewDimension;
    }
    createView(desc) {
      const id = rust('gpu_create', this._id, 'view', json(desc));
      return new GPUTextureView(id, desc && desc.label);
    }
    destroy() {
      rust('gpu_destroy', this._id);
    }
  }

  class GPUBuffer extends GPUObjectBase {
    constructor(id, desc) {
      super(id, desc.label);
      this.size = Number(desc.size);
      this.usage = desc.usage >>> 0;
      // 'create' while mapped at creation, else the GPUMapMode the buffer was mapped with.
      define(this, '_mode', desc.mappedAtCreation ? 'create' : 0);
      define(this, '_state', desc.mappedAtCreation ? 'mapped' : 'unmapped');
      define(this, '_range', [0, this.size]);
      define(this, '_ranges', []);
      define(this, '_pending', null);
    }
    get mapState() {
      return this._state;
    }
    mapAsync(mode, offset = 0, size) {
      if (this._state !== 'unmapped') {
        return Promise.reject(new DOMException('The buffer is already mapped or has a pending map request.', 'OperationError'));
      }
      offset = Number(offset);
      size = size === undefined ? this.size - offset : Number(size);
      let token;
      try {
        token = rust('gpu_map_async', this._id, mode >>> 0, offset, size);
      } catch (e) {
        return Promise.reject(new DOMException(e.message, 'OperationError'));
      }
      this._state = 'pending';
      const request = settleLater(token);
      this._pending = { token, reject: waiting.get(token).reject };
      return request.then(
        () => {
          this._pending = null;
          this._state = 'mapped';
          this._mode = mode >>> 0;
          this._range = [offset, offset + size];
        },
        (e) => {
          this._pending = null;
          if (this._state === 'pending') this._state = 'unmapped';
          throw e;
        }
      );
    }
    getMappedRange(offset = 0, size) {
      if (this._state !== 'mapped') {
        throw new DOMException("Failed to execute 'getMappedRange' on 'GPUBuffer': the buffer is not mapped.", 'OperationError');
      }
      offset = Number(offset);
      size = size === undefined ? this._range[1] - offset : Number(size);
      let buffer;
      if (this._mode === 'create') {
        // A buffer mapped at creation starts out zeroed.
        buffer = new ArrayBuffer(size);
      } else {
        const bytes = rust('gpu_mapped_read', this._id, offset, size);
        buffer = bytes.byteLength === bytes.buffer.byteLength ? bytes.buffer : bytes.slice().buffer;
      }
      this._ranges.push([offset, buffer]);
      return buffer;
    }
    unmap() {
      if (this._state === 'pending') {
        const p = this._pending;
        waiting.delete(p.token);
        this._state = 'unmapped';
        rust('gpu_unmap', this._id);
        p.reject(new DOMException('The buffer was unmapped before the mapping resolved.', 'AbortError'));
        return;
      }
      if (this._state !== 'mapped') return;
      const writes = this._mode === 'create' || (this._mode & 2) !== 0;
      for (const [offset, buffer] of this._ranges) {
        if (writes && !buffer.detached) rust('gpu_mapped_write', this._id, offset, new Uint8Array(buffer));
        // The ranges are detached on unmap, as in a browser.
        try {
          buffer.transfer(0);
        } catch (e) {}
      }
      this._ranges = [];
      this._state = 'unmapped';
      this._mode = 0;
      rust('gpu_unmap', this._id);
    }
    destroy() {
      if (this._state !== 'unmapped') this.unmap();
      rust('gpu_destroy', this._id);
    }
  }

  // ── Command encoding ───────────────────────────────────────────────────────────────────────

  class GPUCommandEncoder {
    constructor(device, desc) {
      define(this, '_device', device);
      this.label = desc && desc.label ? String(desc.label) : '';
      define(this, '_cmds', []);
      // Descriptors (they cross as JSON) and the wrappers that numeric commands name, both kept
      // alive until the encoder finishes.
      define(this, '_descs', []);
      define(this, '_refs', []);
    }
    _desc(d) {
      this._descs.push(d);
      return this._descs.length - 1;
    }
    _ref(o) {
      if (!o) return 0;
      this._refs.push(o);
      return o._id;
    }
    beginRenderPass(desc) {
      this._cmds.push(OP.BEGIN_RENDER_PASS, this._desc(desc));
      return new GPURenderPassEncoder(this, desc && desc.label);
    }
    beginComputePass(desc) {
      this._cmds.push(OP.BEGIN_COMPUTE_PASS, this._desc(desc || {}));
      return new GPUComputePassEncoder(this, desc && desc.label);
    }
    // (source, sourceOffset, destination, destinationOffset, size?) or (source, destination, size?)
    copyBufferToBuffer(source, a, b, c, d) {
      let sourceOffset, destination, destinationOffset, size;
      if (a !== null && typeof a === 'object') {
        sourceOffset = 0;
        destination = a;
        destinationOffset = 0;
        size = b;
      } else {
        sourceOffset = a;
        destination = b;
        destinationOffset = c;
        size = d;
      }
      this._cmds.push(
        OP.COPY_BUFFER_TO_BUFFER,
        this._ref(source),
        Number(sourceOffset),
        this._ref(destination),
        Number(destinationOffset),
        size === undefined ? -1 : Number(size)
      );
    }
    copyBufferToTexture(source, destination, size) {
      const desc = {
        source: { buffer: source.buffer, offset: source.offset ?? 0, bytesPerRow: source.bytesPerRow, rowsPerImage: source.rowsPerImage },
        destination: texelCopyTexture(destination),
        size: extent(size),
      };
      this._cmds.push(OP.COPY_BUFFER_TO_TEXTURE, this._desc(desc));
    }
    copyTextureToBuffer(source, destination, size) {
      const desc = {
        source: texelCopyTexture(source),
        destination: { buffer: destination.buffer, offset: destination.offset ?? 0, bytesPerRow: destination.bytesPerRow, rowsPerImage: destination.rowsPerImage },
        size: extent(size),
      };
      this._cmds.push(OP.COPY_TEXTURE_TO_BUFFER, this._desc(desc));
    }
    copyTextureToTexture(source, destination, size) {
      const desc = { source: texelCopyTexture(source), destination: texelCopyTexture(destination), size: extent(size) };
      this._cmds.push(OP.COPY_TEXTURE_TO_TEXTURE, this._desc(desc));
    }
    clearBuffer(buffer, offset = 0, size) {
      this._cmds.push(OP.CLEAR_BUFFER, this._ref(buffer), Number(offset), size === undefined ? -1 : Number(size));
    }
    resolveQuerySet(querySet, firstQuery, queryCount, destination, destinationOffset) {
      this._cmds.push(OP.RESOLVE_QUERY_SET, this._ref(querySet), firstQuery >>> 0, queryCount >>> 0, this._ref(destination), Number(destinationOffset));
    }
    writeTimestamp(querySet, queryIndex) {
      this._cmds.push(OP.WRITE_TIMESTAMP, this._ref(querySet), queryIndex >>> 0);
    }
    pushDebugGroup(label) {
      this._cmds.push(OP.PUSH_DEBUG_GROUP, this._desc(String(label)));
    }
    popDebugGroup() {
      this._cmds.push(OP.POP_DEBUG_GROUP);
    }
    insertDebugMarker(label) {
      this._cmds.push(OP.INSERT_DEBUG_MARKER, this._desc(String(label)));
    }
    finish(desc) {
      if (!this._cmds) throw new DOMException('The command encoder has already finished.', 'OperationError');
      const label = desc && desc.label ? String(desc.label) : this.label;
      const id = rust('gpu_encode', this._device._id, json({ label, descs: this._descs }), new Float64Array(this._cmds));
      this._cmds = null;
      this._descs = null;
      this._refs = null;
      return new GPUCommandBuffer(id, label);
    }
  }

  // What render and compute passes share.
  class GPUPassEncoder {
    constructor(encoder, label) {
      define(this, '_e', encoder);
      this.label = label ? String(label) : '';
    }
    setPipeline(pipeline) {
      const e = this._e;
      e._cmds.push(OP.SET_PIPELINE, e._ref(pipeline));
    }
    // (index, bindGroup, dynamicOffsets?) or (index, bindGroup, Uint32Array, start, length)
    setBindGroup(index, bindGroup, offsets, start, length) {
      const e = this._e;
      const cmds = e._cmds;
      const id = e._ref(bindGroup);
      if (offsets === undefined || offsets === null) {
        cmds.push(OP.SET_BIND_GROUP, index >>> 0, id, 0);
        return;
      }
      const from = start === undefined ? 0 : start >>> 0;
      const count = length === undefined ? offsets.length - from : length >>> 0;
      cmds.push(OP.SET_BIND_GROUP, index >>> 0, id, count);
      for (let i = 0; i < count; i++) cmds.push(offsets[from + i]);
    }
    pushDebugGroup(label) {
      this._e.pushDebugGroup(label);
    }
    popDebugGroup() {
      this._e.popDebugGroup();
    }
    insertDebugMarker(label) {
      this._e.insertDebugMarker(label);
    }
    end() {
      this._e._cmds.push(OP.END_PASS);
    }
  }

  class GPURenderPassEncoder extends GPUPassEncoder {
    setVertexBuffer(slot, buffer, offset = 0, size) {
      const e = this._e;
      e._cmds.push(OP.SET_VERTEX_BUFFER, slot >>> 0, e._ref(buffer), Number(offset), size === undefined ? -1 : Number(size));
    }
    setIndexBuffer(buffer, format, offset = 0, size) {
      const e = this._e;
      e._cmds.push(OP.SET_INDEX_BUFFER, e._ref(buffer), format === 'uint32' ? 1 : 0, Number(offset), size === undefined ? -1 : Number(size));
    }
    draw(vertexCount, instanceCount = 1, firstVertex = 0, firstInstance = 0) {
      this._e._cmds.push(OP.DRAW, vertexCount >>> 0, instanceCount >>> 0, firstVertex >>> 0, firstInstance >>> 0);
    }
    drawIndexed(indexCount, instanceCount = 1, firstIndex = 0, baseVertex = 0, firstInstance = 0) {
      this._e._cmds.push(OP.DRAW_INDEXED, indexCount >>> 0, instanceCount >>> 0, firstIndex >>> 0, baseVertex | 0, firstInstance >>> 0);
    }
    drawIndirect(buffer, offset) {
      const e = this._e;
      e._cmds.push(OP.DRAW_INDIRECT, e._ref(buffer), Number(offset));
    }
    drawIndexedIndirect(buffer, offset) {
      const e = this._e;
      e._cmds.push(OP.DRAW_INDEXED_INDIRECT, e._ref(buffer), Number(offset));
    }
    setViewport(x, y, width, height, minDepth, maxDepth) {
      this._e._cmds.push(OP.SET_VIEWPORT, x, y, width, height, minDepth, maxDepth);
    }
    setScissorRect(x, y, width, height) {
      this._e._cmds.push(OP.SET_SCISSOR_RECT, x >>> 0, y >>> 0, width >>> 0, height >>> 0);
    }
    setBlendConstant(c) {
      this._e._cmds.push(OP.SET_BLEND_CONSTANT, ...color(c));
    }
    setStencilReference(reference) {
      this._e._cmds.push(OP.SET_STENCIL_REFERENCE, reference >>> 0);
    }
    beginOcclusionQuery(queryIndex) {
      this._e._cmds.push(OP.BEGIN_OCCLUSION_QUERY, queryIndex >>> 0);
    }
    endOcclusionQuery() {
      this._e._cmds.push(OP.END_OCCLUSION_QUERY);
    }
    executeBundles() {
      throw new DOMException('Render bundles are not supported here.', 'NotSupportedError');
    }
  }

  class GPUComputePassEncoder extends GPUPassEncoder {
    dispatchWorkgroups(x, y = 1, z = 1) {
      this._e._cmds.push(OP.DISPATCH, x >>> 0, y >>> 0, z >>> 0);
    }
    dispatchWorkgroupsIndirect(buffer, offset) {
      const e = this._e;
      e._cmds.push(OP.DISPATCH_INDIRECT, e._ref(buffer), Number(offset));
    }
  }

  // ── Queue ──────────────────────────────────────────────────────────────────────────────────

  class GPUQueue extends GPUObjectBase {
    submit(commandBuffers) {
      const list = Array.from(commandBuffers || []);
      const ids = new Float64Array(list.length);
      for (let i = 0; i < list.length; i++) ids[i] = list[i]._id;
      rust('gpu_submit', this._id, ids);
    }
    writeBuffer(buffer, bufferOffset, data, dataOffset = 0, size) {
      rust('gpu_write_buffer', this._id, buffer._id, Number(bufferOffset), sourceBytes(data, dataOffset, size));
    }
    writeTexture(destination, data, dataLayout, size) {
      const layout = { offset: dataLayout.offset ?? 0, bytesPerRow: dataLayout.bytesPerRow, rowsPerImage: dataLayout.rowsPerImage };
      const desc = { destination: texelCopyTexture(destination), dataLayout: layout, size: extent(size) };
      rust('gpu_write_texture', this._id, json(desc), sourceBytes(data, 0, undefined));
    }
    onSubmittedWorkDone() {
      return settleLater(rust('gpu_on_submitted_work_done', this._id));
    }
    copyExternalImageToTexture() {
      throw new DOMException('copyExternalImageToTexture is not supported here.', 'NotSupportedError');
    }
  }

  // ── Device and adapter ─────────────────────────────────────────────────────────────────────

  class GPUDevice extends EventTarget {
    constructor(r, desc) {
      super();
      track(this, r.id);
      this.label = desc && desc.label ? String(desc.label) : '';
      this.features = new GPUSupportedFeatures(r.features);
      this.limits = new GPUSupportedLimits(r.limits);
      this.adapterInfo = new GPUAdapterInfo(r.info);
      this.queue = new GPUQueue(r.queue, (desc && desc.defaultQueue && desc.defaultQueue.label) || '');
      // Never lost: there is no other process to lose it to.
      this.lost = new Promise(() => {});
      this.onuncapturederror = null;
    }
    toJSON() {
      return { $: this._id };
    }
    _create(kind, desc) {
      return rust('gpu_create', this._id, kind, json(desc));
    }
    destroy() {
      rust('gpu_destroy', this._id);
    }
    createBuffer(desc) {
      return new GPUBuffer(this._create('buffer', desc), desc);
    }
    createTexture(desc) {
      const d = { ...desc, size: extent(desc.size) };
      return new GPUTexture(this._create('texture', d), d);
    }
    createSampler(desc) {
      return new GPUSampler(this._create('sampler', desc || {}), desc && desc.label);
    }
    createBindGroupLayout(desc) {
      return new GPUBindGroupLayout(this._create('bind_group_layout', desc), desc.label);
    }
    createPipelineLayout(desc) {
      return new GPUPipelineLayout(this._create('pipeline_layout', desc), desc.label);
    }
    createBindGroup(desc) {
      return new GPUBindGroup(this._create('bind_group', desc), desc.label);
    }
    createShaderModule(desc) {
      return new GPUShaderModule(this._create('shader_module', desc), desc.label);
    }
    createComputePipeline(desc) {
      return new GPUComputePipeline(this._create('compute_pipeline', desc), desc.label);
    }
    createRenderPipeline(desc) {
      return new GPURenderPipeline(this._create('render_pipeline', desc), desc.label);
    }
    createComputePipelineAsync(desc) {
      try {
        return Promise.resolve(this.createComputePipeline(desc));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    createRenderPipelineAsync(desc) {
      try {
        return Promise.resolve(this.createRenderPipeline(desc));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    createQuerySet(desc) {
      return new GPUQuerySet(this._create('query_set', desc), desc);
    }
    createCommandEncoder(desc) {
      return new GPUCommandEncoder(this, desc);
    }
    createRenderBundleEncoder() {
      throw new DOMException('Render bundles are not supported here.', 'NotSupportedError');
    }
    importExternalTexture() {
      throw new DOMException('External textures are not supported here.', 'NotSupportedError');
    }
    pushErrorScope(filter) {
      rust('gpu_push_error_scope', this._id, String(filter));
    }
    popErrorScope() {
      let text;
      try {
        text = rust('gpu_pop_error_scope', this._id);
      } catch (e) {
        return Promise.reject(new DOMException(e.message, 'OperationError'));
      }
      return Promise.resolve(text ? makeError(JSON.parse(text)) : null);
    }
  }

  class GPUAdapter {
    constructor(r) {
      track(this, r.id);
      this.features = new GPUSupportedFeatures(r.features);
      this.limits = new GPUSupportedLimits(r.limits);
      this.info = new GPUAdapterInfo(r.info);
      this.isFallbackAdapter = !!(r.info && r.info.isFallbackAdapter);
    }
    requestDevice(desc) {
      return new Promise((resolve, reject) =>
        queueTask(() => {
          let r;
          try {
            r = JSON.parse(rust('gpu_request_device', this._id, json(desc)));
          } catch (e) {
            reject(e instanceof TypeError ? e : new DOMException(e.message, 'OperationError'));
            return;
          }
          resolve(new GPUDevice(r, desc));
        })
      );
    }
    requestAdapterInfo() {
      return Promise.resolve(this.info);
    }
  }

  let preferred = null;
  class GPU {
    requestAdapter(options) {
      return new Promise((resolve) =>
        queueTask(() => {
          let r = null;
          try {
            const text = rust('gpu_request_adapter', json(options));
            r = text ? JSON.parse(text) : null;
          } catch (e) {
            reportError(e, 'requesting a GPU adapter');
          }
          resolve(r ? new GPUAdapter(r) : null);
        })
      );
    }
    getPreferredCanvasFormat() {
      if (preferred === null) {
        try {
          preferred = rust('gpu_preferred_format') || 'bgra8unorm';
        } catch (e) {
          preferred = 'bgra8unorm';
        }
      }
      return preferred;
    }
    get wgslLanguageFeatures() {
      return new WGSLLanguageFeatures([]);
    }
  }

  // ── The canvas ─────────────────────────────────────────────────────────────────────────────

  // Contexts that handed out a texture since the last frame, to present after the next one.
  const toPresent = new Set();

  class GPUCanvasContext {
    constructor(canvas) {
      this.canvas = canvas;
      define(this, '_config', null);
      define(this, '_texture', null);
    }
    configure(config) {
      const c = {
        format: config.format,
        usage: config.usage === undefined ? 0x10 : config.usage >>> 0,
        viewFormats: Array.from(config.viewFormats || []),
        alphaMode: config.alphaMode || 'opaque',
        colorSpace: config.colorSpace || 'srgb',
      };
      rust('gpu_configure', config.device._id, json(c));
      this._config = { ...c, device: config.device, toneMapping: config.toneMapping || { mode: 'standard' } };
      this._texture = null;
    }
    unconfigure() {
      rust('gpu_unconfigure');
      this._config = null;
      this._texture = null;
      toPresent.delete(this);
    }
    getConfiguration() {
      return this._config ? { ...this._config, viewFormats: this._config.viewFormats.slice() } : null;
    }
    getCurrentTexture() {
      if (!this._config) {
        throw new DOMException("Failed to execute 'getCurrentTexture' on 'GPUCanvasContext': context is not configured.", 'InvalidStateError');
      }
      if (!this._texture) {
        const width = Math.max(1, this.canvas.width);
        const height = Math.max(1, this.canvas.height);
        const id = rust('gpu_current_texture', width, height);
        this._texture = new GPUTexture(id, {
          size: { width, height, depthOrArrayLayers: 1 },
          format: this._config.format,
          usage: this._config.usage,
          label: 'canvas',
        });
        toPresent.add(this);
      }
      return this._texture;
    }
  }

  A.createCanvasContext = (canvas) => new GPUCanvasContext(canvas);
  A.frameHooks.push(() => {
    for (const context of toPresent) {
      if (context._texture) rust('gpu_present');
      context._texture = null;
    }
    toPresent.clear();
    flushDrops();
  });

  // ── Globals ────────────────────────────────────────────────────────────────────────────────

  const constants = {
    GPUBufferUsage: { MAP_READ: 0x1, MAP_WRITE: 0x2, COPY_SRC: 0x4, COPY_DST: 0x8, INDEX: 0x10, VERTEX: 0x20, UNIFORM: 0x40, STORAGE: 0x80, INDIRECT: 0x100, QUERY_RESOLVE: 0x200 },
    GPUTextureUsage: { COPY_SRC: 0x1, COPY_DST: 0x2, TEXTURE_BINDING: 0x4, STORAGE_BINDING: 0x8, RENDER_ATTACHMENT: 0x10 },
    GPUMapMode: { READ: 0x1, WRITE: 0x2 },
    GPUShaderStage: { VERTEX: 0x1, FRAGMENT: 0x2, COMPUTE: 0x4 },
    GPUColorWrite: { RED: 0x1, GREEN: 0x2, BLUE: 0x4, ALPHA: 0x8, ALL: 0xf },
  };
  for (const [name, value] of Object.entries(constants)) define(G, name, Object.freeze(value));

  const classes = {
    GPU,
    GPUAdapter,
    GPUAdapterInfo,
    GPUDevice,
    GPUQueue,
    GPUBuffer,
    GPUTexture,
    GPUTextureView,
    GPUSampler,
    GPUBindGroupLayout,
    GPUBindGroup,
    GPUPipelineLayout,
    GPUShaderModule,
    GPURenderPipeline,
    GPUComputePipeline,
    GPUQuerySet,
    GPUCommandEncoder,
    GPUCommandBuffer,
    GPURenderPassEncoder,
    GPUComputePassEncoder,
    GPUCanvasContext,
    GPUSupportedFeatures,
    GPUSupportedLimits,
    WGSLLanguageFeatures,
    GPUError,
    GPUValidationError,
    GPUOutOfMemoryError,
    GPUInternalError,
    GPUUncapturedErrorEvent,
  };
  for (const [name, value] of Object.entries(classes)) define(G, name, value);

  G.navigator.gpu = new GPU();
})();
