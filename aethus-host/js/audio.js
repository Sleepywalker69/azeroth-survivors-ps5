// Web Audio for the web build, mixed by the embedder.
//
// The client plays sound the way a page does with Web Audio: it fills AudioBuffers, starts
// AudioBufferSourceNodes on the context's clock (`start(when)`) and keeps its queue full from
// their `ended` events; compressed sound is decoded with an OfflineAudioContext's
// decodeAudioData and read back with getChannelData. Only that much of the API is here.
//
// The embedder owns the clock and the mixer. A buffer's samples cross once, when a source first
// plays them after a change, and then live natively under a handle; every started source is a
// voice that the embedder mixes at the context's rate and resamples to the device's. Voices that
// finish, and decodes (which run off the JS thread, as in a browser), are reported by
// `audio_poll` at the start of a later task.
//
// The embedder's operations:
//   audio_open(sampleRate or 0)  ->  JSON { handle, sampleRate, maxChannelCount, baseLatency,
//       outputLatency }. Without a usable device it still opens a context, on a silent sink
//       whose clock runs in real time; the destination starts with min(2, maxChannelCount)
//       channels.
//   audio_time(handle) -> seconds          audio_suspend(handle, suspended)    audio_close(handle)
//   audio_set_channels(handle, count): the destination's channel count, the layout voices are
//       mixed into (Web Audio's speaker rules) before it is mapped onto the device's channels.
//   audio_buffer_create(planar f32, channels, frames, sampleRate) -> id     audio_buffer_free(id)
//       (a voice keeps its samples alive natively after its buffer's handle is freed)
//   audio_play(handle, bufferId or 0, when, offset, duration or -1, gain) -> token
//   audio_stop(handle, token, when)
//   audio_decode(bytes, sampleRate) -> token: decodes a copy of the bytes off the JS thread,
//       resampled to the rate.
//   audio_decode_take(token) -> [u32 channels][u32 frames][f32 sampleRate][u32 0] then planar
//       f32 samples; throws with the decoder's message when the data could not be decoded.
//   audio_poll() -> JSON { ended: [token, ...], decoded: [token, ...] }, or '' for nothing.
'use strict';

(() => {
  const G = globalThis;
  const A = G.__aethus;
  const { rust, define, queueTask, runTask, reportError, dispatch, toBytes } = A;

  const MIN_RATE = 3000;
  const MAX_RATE = 768000;
  const MAX_CHANNELS = 32;

  const notSupported = (message) => new DOMException(message, 'NotSupportedError');
  const invalidState = (message) => new DOMException(message, 'InvalidStateError');

  // ── Buffers ────────────────────────────────────────────────────────────────────────────────

  // Handles of collected buffers, freed at the next tick.
  let freed = [];
  const collected = new FinalizationRegistry((native) => {
    if (native.id) freed.push(native.id);
  });

  function checkShape(channels, length, sampleRate) {
    if (!(channels >= 1 && channels <= MAX_CHANNELS)) {
      throw notSupported(`The number of channels provided (${channels}) is outside the range [1, ${MAX_CHANNELS}].`);
    }
    if (!(length >= 1)) throw notSupported(`The number of frames provided (${length}) is less than or equal to the minimum bound (0).`);
    if (!(sampleRate >= MIN_RATE && sampleRate <= MAX_RATE)) {
      throw notSupported(`The sample rate provided (${sampleRate}) is outside the range [${MIN_RATE}, ${MAX_RATE}].`);
    }
  }

  function initBuffer(buffer, data, length, sampleRate) {
    define(buffer, '_data', data); // one Float32Array per channel
    define(buffer, '_length', length);
    define(buffer, '_rate', sampleRate);
    define(buffer, '_native', { id: 0 });
    define(buffer, '_dirty', true);
    collected.register(buffer, buffer._native);
    return buffer;
  }

  function channelIndex(buffer, channel) {
    const c = Number(channel);
    if (!(c >= 0 && c < buffer._data.length)) {
      throw new DOMException(`The channel index provided (${channel}) is outside the range [0, ${buffer._data.length - 1}].`, 'IndexSizeError');
    }
    return c >>> 0;
  }

  class AudioBuffer {
    constructor(options) {
      if (!options || typeof options !== 'object') throw new TypeError("Failed to construct 'AudioBuffer': 1 argument required, but only 0 present.");
      const channels = options.numberOfChannels === undefined ? 1 : Number(options.numberOfChannels) >>> 0;
      const length = Number(options.length) >>> 0;
      const sampleRate = Number(options.sampleRate);
      checkShape(channels, length, sampleRate);
      const data = [];
      for (let c = 0; c < channels; c++) data.push(new Float32Array(length));
      initBuffer(this, data, length, sampleRate);
    }
    get numberOfChannels() {
      return this._data.length;
    }
    get length() {
      return this._length;
    }
    get sampleRate() {
      return this._rate;
    }
    get duration() {
      return this._length / this._rate;
    }
    getChannelData(channel) {
      const data = this._data[channelIndex(this, channel)];
      this._dirty = true; // the caller may write into it
      return data;
    }
    copyFromChannel(destination, channel, bufferOffset = 0) {
      const data = this._data[channelIndex(this, channel)];
      const start = Math.min(Number(bufferOffset) >>> 0, this._length);
      destination.set(data.subarray(start, start + Math.min(destination.length, this._length - start)));
    }
    copyToChannel(source, channel, bufferOffset = 0) {
      const data = this._data[channelIndex(this, channel)];
      const start = Number(bufferOffset) >>> 0;
      if (start >= this._length) return;
      // The glue passes a view of the wasm's memory: `set` copies it.
      const count = Math.min(source.length, this._length - start);
      data.set(count === source.length ? source : source.subarray(0, count), start);
      this._dirty = true;
    }
    // The handle of the samples as they are now, uploaded again after a change.
    _handle() {
      const native = this._native;
      if (native.id && !this._dirty) return native.id;
      const data = this._data;
      let planar = data[0];
      if (data.length > 1) {
        planar = new Float32Array(data.length * this._length);
        for (let c = 0; c < data.length; c++) planar.set(data[c], c * this._length);
      }
      const id = rust('audio_buffer_create', planar, data.length, this._length, this._rate);
      if (native.id) rust('audio_buffer_free', native.id);
      native.id = id;
      this._dirty = false;
      return id;
    }
  }

  function unpackDecoded(bytes) {
    if (!bytes || bytes.byteLength < 16) throw new DOMException('Unable to decode audio data', 'EncodingError');
    const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const channels = view.getUint32(0, true);
    const frames = view.getUint32(4, true);
    const sampleRate = view.getFloat32(8, true);
    if (!channels || !frames || 16 + channels * frames * 4 > bytes.byteLength) {
      throw new DOMException('Unable to decode audio data', 'EncodingError');
    }
    const data = [];
    for (let c = 0; c < channels; c++) data.push(new Float32Array(bytes.buffer, bytes.byteOffset + 16 + c * frames * 4, frames));
    return initBuffer(Object.create(AudioBuffer.prototype), data, frames, sampleRate);
  }

  // ── Nodes ──────────────────────────────────────────────────────────────────────────────────

  // Sources that are playing, by token.
  const voices = new Map();

  class AudioNode extends EventTarget {
    constructor(context, inputs, outputs) {
      super();
      define(this, '_context', context);
      define(this, '_outputs', new Set());
      define(this, '_channelCount', 2);
      this.numberOfInputs = inputs;
      this.numberOfOutputs = outputs;
      this.channelCountMode = 'max';
      this.channelInterpretation = 'speakers';
    }
    get context() {
      return this._context;
    }
    get channelCount() {
      return this._channelCount;
    }
    set channelCount(value) {
      this._channelCount = Number(value) >>> 0;
    }
    connect(destination) {
      if (!(destination instanceof AudioNode)) {
        throw new TypeError("Failed to execute 'connect' on 'AudioNode': parameter 1 is not of type 'AudioNode'.");
      }
      if (destination._context !== this._context) {
        throw new DOMException("Failed to execute 'connect' on 'AudioNode': cannot connect to an AudioNode belonging to a different audio context.", 'InvalidAccessError');
      }
      this._outputs.add(destination);
      return destination;
    }
    // Connections are read when a source starts; changing them later does not reach a voice
    // that is already playing.
    disconnect(destination) {
      if (destination === undefined) this._outputs.clear();
      else if (!this._outputs.delete(destination)) {
        throw new DOMException("Failed to execute 'disconnect' on 'AudioNode': the given destination is not connected.", 'InvalidAccessError');
      }
    }
  }

  class AudioDestinationNode extends AudioNode {
    constructor(context, maxChannelCount) {
      super(context, 1, 0);
      define(this, '_max', maxChannelCount);
      this._channelCount = Math.min(2, maxChannelCount);
      this.channelCountMode = 'explicit';
    }
    get maxChannelCount() {
      return this._max;
    }
    get channelCount() {
      return this._channelCount;
    }
    set channelCount(value) {
      const n = Number(value);
      if (!(n >= 1 && n <= this._max)) {
        throw new DOMException(`The channel count provided (${value}) is outside the range [1, ${this._max}].`, 'IndexSizeError');
      }
      this._channelCount = n >>> 0;
      const handle = this._context._handle;
      if (handle) rust('audio_set_channels', handle, this._channelCount);
    }
  }

  class AudioScheduledSourceNode extends AudioNode {
    constructor(context) {
      super(context, 0, 1);
      define(this, '_started', false);
      define(this, '_token', 0);
      this.onended = null;
    }
    stop(when = 0) {
      if (!this._started) {
        throw invalidState("Failed to execute 'stop' on 'AudioScheduledSourceNode': cannot call stop without calling start first.");
      }
      when = Number(when);
      if (!(when >= 0)) throw new RangeError(`Failed to execute 'stop' on 'AudioScheduledSourceNode': The stop time provided (${when}) is less than the minimum bound (0).`);
      if (voices.get(this._token) === this) rust('audio_stop', this._context._handle, this._token, when);
    }
  }

  class AudioBufferSourceNode extends AudioScheduledSourceNode {
    constructor(context, options) {
      super(context);
      define(this, '_buffer', null);
      define(this, '_bufferSet', false);
      if (options && options.buffer) this.buffer = options.buffer;
    }
    get buffer() {
      return this._buffer;
    }
    set buffer(value) {
      if (value !== null && !(value instanceof AudioBuffer)) {
        throw new TypeError("Failed to set the 'buffer' property on 'AudioBufferSourceNode': The provided value is not of type 'AudioBuffer'.");
      }
      if (value !== null && this._bufferSet) {
        throw invalidState("Failed to set the 'buffer' property on 'AudioBufferSourceNode': Cannot set buffer to non-null after it has been already been set to a non-null buffer");
      }
      if (value !== null) this._bufferSet = true;
      this._buffer = value;
    }
    start(when = 0, offset = 0, duration) {
      if (this._started) throw invalidState("Failed to execute 'start' on 'AudioScheduledSourceNode': cannot call start more than once.");
      when = Number(when);
      offset = Number(offset);
      if (!(when >= 0)) throw new RangeError(`Failed to execute 'start' on 'AudioBufferSourceNode': The start time provided (${when}) is less than the minimum bound (0).`);
      if (!(offset >= 0)) throw new RangeError(`Failed to execute 'start' on 'AudioBufferSourceNode': The offset provided (${offset}) is less than the minimum bound (0).`);
      if (duration !== undefined && !(Number(duration) >= 0)) {
        throw new RangeError(`Failed to execute 'start' on 'AudioBufferSourceNode': The duration provided (${duration}) is less than the minimum bound (0).`);
      }
      this._started = true;
      const context = this._context;
      // An offline or closed context plays nothing.
      if (!context._handle) return;
      // A source with no buffer plays silence; one that is not connected is timed but not heard.
      const id = this._buffer ? this._buffer._handle() : 0;
      const gain = this._outputs.has(context._destination) ? 1 : 0;
      this._token = rust('audio_play', context._handle, id, when, offset, duration === undefined ? -1 : Number(duration), gain);
      voices.set(this._token, this);
    }
  }

  // ── Contexts ───────────────────────────────────────────────────────────────────────────────

  // decodeAudioData calls waiting for the embedder, by token.
  const decodes = new Map();

  function settleDecode(job, buffer, error) {
    const callback = error ? job.failure : job.success;
    if (typeof callback === 'function') {
      try {
        callback(error || buffer);
      } catch (e) {
        reportError(e, 'in a decodeAudioData callback');
      }
    }
    if (error) job.reject(error);
    else job.resolve(buffer);
  }

  function encodingError(e) {
    if (e instanceof DOMException) return e;
    return new DOMException(`Unable to decode audio data: ${e && e.message ? e.message : e}`, 'EncodingError');
  }

  class BaseAudioContext extends EventTarget {
    constructor(sampleRate, maxChannelCount) {
      super();
      define(this, '_rate', sampleRate);
      define(this, '_state', 'suspended');
      define(this, '_handle', 0);
      define(this, '_destination', new AudioDestinationNode(this, maxChannelCount));
      this.onstatechange = null;
    }
    get sampleRate() {
      return this._rate;
    }
    get destination() {
      return this._destination;
    }
    get state() {
      return this._state;
    }
    get currentTime() {
      return 0;
    }
    createBuffer(numberOfChannels, length, sampleRate) {
      return new AudioBuffer({ numberOfChannels, length, sampleRate });
    }
    createBufferSource() {
      return new AudioBufferSourceNode(this);
    }
    decodeAudioData(audioData, successCallback, errorCallback) {
      return new Promise((resolve, reject) => {
        const job = { resolve, reject, success: successCallback, failure: errorCallback };
        let token;
        try {
          if (!(audioData instanceof ArrayBuffer) && !ArrayBuffer.isView(audioData)) {
            throw new TypeError("Failed to execute 'decodeAudioData' on 'BaseAudioContext': parameter 1 is not of type 'ArrayBuffer'.");
          }
          token = rust('audio_decode', toBytes(audioData), this._rate);
        } catch (e) {
          const error = e instanceof TypeError ? e : encodingError(e);
          queueTask(() => settleDecode(job, null, error));
          return;
        }
        decodes.set(token, job);
      });
    }
    _setState(state) {
      if (this._state === state) return;
      this._state = state;
      queueTask(() => dispatch(this, new Event('statechange')));
    }
  }

  class AudioContext extends BaseAudioContext {
    constructor(options) {
      let rate = 0;
      if (options && options.sampleRate !== undefined) {
        rate = Number(options.sampleRate);
        if (!(rate >= MIN_RATE && rate <= MAX_RATE)) {
          throw notSupported(`Failed to construct 'AudioContext': The hardware sample rate provided (${options.sampleRate}) is outside the range [${MIN_RATE}, ${MAX_RATE}].`);
        }
      }
      let info;
      try {
        info = JSON.parse(rust('audio_open', rate));
      } catch (e) {
        throw notSupported(`Failed to construct 'AudioContext': ${e && e.message ? e.message : e}`);
      }
      super(info.sampleRate, info.maxChannelCount);
      this._handle = info.handle;
      define(this, '_baseLatency', info.baseLatency || 0);
      define(this, '_outputLatency', info.outputLatency || 0);
      define(this, '_closedAt', 0);
      // No autoplay policy here: the context runs from the start.
      this._setState('running');
    }
    get baseLatency() {
      return this._baseLatency;
    }
    get outputLatency() {
      return this._outputLatency;
    }
    get currentTime() {
      return this._handle ? rust('audio_time', this._handle) : this._closedAt;
    }
    resume() {
      return this._suspend(false);
    }
    suspend() {
      return this._suspend(true);
    }
    _suspend(suspended) {
      return new Promise((resolve, reject) => {
        queueTask(() => {
          if (this._state === 'closed') {
            reject(invalidState(`Cannot ${suspended ? 'suspend' : 'resume'} a closed AudioContext.`));
            return;
          }
          rust('audio_suspend', this._handle, suspended);
          this._setState(suspended ? 'suspended' : 'running');
          resolve();
        });
      });
    }
    // Closing ends the context's voices without `ended` events.
    close() {
      return new Promise((resolve, reject) => {
        if (this._state === 'closed') {
          reject(invalidState('Cannot close a closed AudioContext.'));
          return;
        }
        this._closedAt = rust('audio_time', this._handle);
        rust('audio_close', this._handle);
        this._handle = 0;
        for (const [token, node] of voices) if (node._context === this) voices.delete(token);
        this._setState('closed');
        queueTask(() => resolve());
      });
    }
  }

  // The client makes one to decode with; nothing is rendered offline.
  class OfflineAudioContext extends BaseAudioContext {
    constructor(numberOfChannels, length, sampleRate) {
      if (numberOfChannels && typeof numberOfChannels === 'object') {
        ({ numberOfChannels = 1, length, sampleRate } = numberOfChannels);
      }
      numberOfChannels = Number(numberOfChannels) >>> 0;
      length = Number(length) >>> 0;
      sampleRate = Number(sampleRate);
      checkShape(numberOfChannels, length, sampleRate);
      super(sampleRate, numberOfChannels);
      define(this, '_length', length);
      this._destination.channelCount = numberOfChannels;
      this.oncomplete = null;
    }
    get length() {
      return this._length;
    }
  }

  // ── Completions ────────────────────────────────────────────────────────────────────────────

  function poll() {
    if (freed.length) {
      const ids = freed;
      freed = [];
      for (const id of ids) rust('audio_buffer_free', id);
    }
    if (!voices.size && !decodes.size) return;
    const text = rust('audio_poll');
    if (!text) return;
    const done = JSON.parse(text);
    for (const token of done.ended || []) {
      const node = voices.get(token);
      if (!node) continue;
      voices.delete(token);
      runTask(() => dispatch(node, new Event('ended')), 'in an "ended" event');
    }
    for (const token of done.decoded || []) {
      const job = decodes.get(token);
      if (!job) continue;
      decodes.delete(token);
      let buffer = null;
      let error = null;
      try {
        buffer = unpackDecoded(rust('audio_decode_take', token));
      } catch (e) {
        error = encodingError(e);
      }
      runTask(() => settleDecode(job, buffer, error), 'in decodeAudioData');
    }
  }
  A.tickHooks.push(poll);

  const globals = {
    AudioBuffer,
    AudioNode,
    AudioDestinationNode,
    AudioScheduledSourceNode,
    AudioBufferSourceNode,
    BaseAudioContext,
    AudioContext,
    OfflineAudioContext,
  };
  for (const [name, value] of Object.entries(globals)) define(G, name, value);
})();
