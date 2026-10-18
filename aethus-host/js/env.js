// The browser the web build expects, for QuickJS.
//
// The web build's glue (wenilla.js) and the Rust crates behind it (winit, wgpu, cpal, gilrs,
// web-sys, the game's own web modules) talk to a browser page: a window with one canvas, DOM
// events, timers and animation frames, synchronous XHR for the game data, localStorage, WebGPU
// and Web Audio. This script builds that page as a plain global script, before the glue loads.
// It is deliberately small: only what the glue's 869 imports reach is here, and every platform
// question (input, data, storage, the GPU, sound) is answered by the embedder through
// `__native.call`.
//
// The event loop is the embedder's: it calls `__host.tick(now)` whenever a timer or a task may
// be due and `__host.frame(now)` once per display refresh, the way a browser runs tasks and then
// "updates the rendering". Every task ends with a microtask checkpoint, as in a browser.
//
// gpu.js (WebGPU) and audio.js (Web Audio) run after this script and use the internals it leaves
// on `__aethus`.
'use strict';

(() => {
  const G = globalThis;
  const N = G.__native;

  // ── The embedder's operations ──────────────────────────────────────────────────────────────
  //
  // Operation 0 resolves a name to its number; numbers are cached.

  const opIds = new Map();
  function opId(name) {
    let id = opIds.get(name);
    if (id === undefined) {
      id = N.call(0, name);
      if (typeof id !== 'number' || !(id > 0)) throw new Error(`the host has no operation "${name}"`);
      opIds.set(name, id);
    }
    return id;
  }
  function rust(name, ...args) {
    return N.call(opId(name), ...args);
  }

  // ── Small helpers ──────────────────────────────────────────────────────────────────────────

  function define(target, name, value) {
    Object.defineProperty(target, name, { value, writable: true, configurable: true, enumerable: false });
  }
  function getter(target, name, get, set) {
    Object.defineProperty(target, name, { get, set, configurable: true, enumerable: true });
  }
  function toBytes(data) {
    if (data == null) return new Uint8Array(0);
    if (data instanceof Uint8Array) return data;
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (data instanceof Blob) return data[BLOB_BYTES];
    return N.textEncode(String(data));
  }
  function exactBuffer(bytes) {
    if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) return bytes.buffer;
    return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
  }

  // ── Console ────────────────────────────────────────────────────────────────────────────────

  function inspect(value, depth = 0, seen = new Set()) {
    switch (typeof value) {
      case 'string':
        return depth ? JSON.stringify(value) : value;
      case 'number':
      case 'boolean':
      case 'undefined':
      case 'bigint':
        return String(value);
      case 'symbol':
        return value.toString();
      case 'function':
        return `[Function${value.name ? ': ' + value.name : ''}]`;
    }
    if (value === null) return 'null';
    if (value instanceof Error) {
      const head = `${value.name || 'Error'}: ${value.message}`;
      return value.stack ? `${head}\n${value.stack}` : head;
    }
    if (seen.has(value)) return '[Circular]';
    if (depth > 2) return Array.isArray(value) ? '[Array]' : '[Object]';
    seen.add(value);
    try {
      if (Array.isArray(value)) return `[${value.map((v) => inspect(v, depth + 1, seen)).join(', ')}]`;
      if (ArrayBuffer.isView(value)) return `${value.constructor.name}(${value.length})`;
      if (value instanceof ArrayBuffer) return `ArrayBuffer(${value.byteLength})`;
      const name = value.constructor && value.constructor !== Object ? value.constructor.name + ' ' : '';
      const keys = Object.keys(value);
      const body = keys.slice(0, 24).map((k) => `${k}: ${inspect(value[k], depth + 1, seen)}`);
      if (keys.length > 24) body.push('…');
      return `${name}{${body.join(', ')}}`;
    } catch (e) {
      return '[Object]';
    } finally {
      seen.delete(value);
    }
  }

  // printf-style substitution, as consoles do it; `%c` styles (tracing-wasm colours its level
  // and target) are dropped together with their arguments.
  function format(args) {
    if (args.length === 0) return '';
    let i = 0;
    let out;
    if (typeof args[0] === 'string') {
      i = 1;
      out = args[0].includes('%')
        ? args[0].replace(/%([csdifoOj%])/g, (m, c) => {
            if (c === '%') return '%';
            if (i >= args.length) return m;
            const a = args[i++];
            switch (c) {
              case 'c':
                return '';
              case 's':
                return typeof a === 'string' ? a : inspect(a, 1);
              case 'd':
              case 'i':
                return String(parseInt(a, 10));
              case 'f':
                return String(Number(a));
              default:
                return inspect(a, 1);
            }
          })
        : args[0];
    } else {
      out = '';
    }
    for (; i < args.length; i++) out += (out ? ' ' : '') + inspect(args[i]);
    return out;
  }

  const counts = new Map();
  const consoleObject = {
    log: (...a) => N.log(1, format(a)),
    info: (...a) => N.log(1, format(a)),
    debug: (...a) => N.log(0, format(a)),
    trace: (...a) => N.log(0, format(a)),
    warn: (...a) => N.log(2, format(a)),
    error: (...a) => N.log(3, format(a)),
    assert: (ok, ...a) => {
      if (!ok) N.log(3, 'Assertion failed' + (a.length ? ': ' + format(a) : ''));
    },
    group: (...a) => a.length && N.log(1, format(a)),
    groupCollapsed: (...a) => a.length && N.log(1, format(a)),
    groupEnd: () => {},
    time: () => {},
    timeEnd: () => {},
    timeLog: () => {},
    table: (v) => N.log(1, inspect(v)),
    dir: (v) => N.log(1, inspect(v)),
    count: (label = 'default') => {
      const n = (counts.get(label) || 0) + 1;
      counts.set(label, n);
      N.log(1, `${label}: ${n}`);
    },
    countReset: (label = 'default') => counts.delete(label),
  };
  define(G, 'console', consoleObject);

  function reportError(error, where) {
    N.log(3, `Uncaught ${where ? '(' + where + ') ' : ''}${inspect(error)}`);
  }

  // ── Tasks, timers and the microtask checkpoint ─────────────────────────────────────────────

  let tasks = [];
  function queueTask(fn) {
    tasks.push(fn);
  }
  function runTask(fn, where) {
    try {
      fn();
    } catch (e) {
      reportError(e, where);
    }
    N.drainJobs();
  }

  let timerSeq = 0;
  const timers = new Map(); // id -> { at, fn, args, every }
  function addTimer(fn, ms, args, repeat) {
    const id = ++timerSeq;
    let delay = Number(ms);
    if (!(delay > 0)) delay = 0;
    timers.set(id, { at: N.now() + delay, fn, args, every: repeat ? Math.max(delay, 1) : -1 });
    return id;
  }
  function clearTimer(id) {
    timers.delete(Number(id));
  }
  define(G, 'setTimeout', (fn, ms, ...args) => addTimer(fn, ms, args, false));
  define(G, 'setInterval', (fn, ms, ...args) => addTimer(fn, ms, args, true));
  define(G, 'clearTimeout', clearTimer);
  define(G, 'clearInterval', clearTimer);

  function runTimers(now) {
    const due = [];
    for (const [id, t] of timers) if (t.at <= now) due.push([id, t]);
    if (!due.length) return;
    due.sort((a, b) => a[1].at - b[1].at || a[0] - b[0]);
    for (const [id, t] of due) {
      if (timers.get(id) !== t) continue; // cleared by an earlier callback
      if (t.every >= 0) t.at = Math.max(t.at + t.every, now);
      else timers.delete(id);
      if (typeof t.fn === 'function') runTask(() => t.fn(...t.args), 'in a timer');
    }
  }

  function nextTimerDelay(now) {
    let at = Infinity;
    for (const t of timers.values()) if (t.at < at) at = t.at;
    return at === Infinity ? -1 : Math.max(0, at - now);
  }

  // ── Events ─────────────────────────────────────────────────────────────────────────────────

  const LISTENERS = Symbol('listeners');
  const STOP = Symbol('stop');
  const PATH = Symbol('path');

  class EventTarget {
    addEventListener(type, listener, options) {
      if (listener == null) return;
      const capture = typeof options === 'boolean' ? options : !!(options && options.capture);
      const once = !!(options && typeof options === 'object' && options.once);
      let map = this[LISTENERS];
      if (!map) {
        map = new Map();
        define(this, LISTENERS, map);
      }
      type = String(type);
      let list = map.get(type);
      if (!list) map.set(type, (list = []));
      if (list.some((l) => l.listener === listener && l.capture === capture)) return;
      list.push({ listener, capture, once, removed: false });
      const signal = options && typeof options === 'object' ? options.signal : undefined;
      if (signal) signal.addEventListener('abort', () => this.removeEventListener(type, listener, capture));
    }
    removeEventListener(type, listener, options) {
      const map = this[LISTENERS];
      const list = map && map.get(String(type));
      if (!list) return;
      const capture = typeof options === 'boolean' ? options : !!(options && options.capture);
      const i = list.findIndex((l) => l.listener === listener && l.capture === capture);
      if (i >= 0) {
        list[i].removed = true;
        list.splice(i, 1);
      }
    }
    dispatchEvent(event) {
      return dispatch(this, event);
    }
  }

  function parentOf(target) {
    if (target instanceof Node) {
      if (target.parentNode) return target.parentNode;
      if (target === document) return G;
    }
    return null;
  }

  function invoke(target, event, phase) {
    event.currentTarget = target;
    const map = target[LISTENERS];
    const list = map && map.get(event.type);
    if (list && list.length) {
      for (const l of list.slice()) {
        if (l.removed) continue;
        if (phase === 1 && !l.capture) continue;
        if (phase === 3 && l.capture) continue;
        if (l.once) target.removeEventListener(event.type, l.listener, l.capture);
        try {
          if (typeof l.listener === 'function') l.listener.call(target, event);
          else if (l.listener && typeof l.listener.handleEvent === 'function') l.listener.handleEvent(event);
        } catch (e) {
          reportError(e, `in a "${event.type}" listener`);
        }
        if (event[STOP] & 2) return;
      }
    }
    if (phase !== 1) {
      const handler = target['on' + event.type];
      if (typeof handler === 'function') {
        try {
          const r = handler.call(target, event);
          if (r === false && event.cancelable) event.defaultPrevented = true;
        } catch (e) {
          reportError(e, `in the "on${event.type}" handler`);
        }
      }
    }
  }

  function dispatch(target, event) {
    const path = [];
    for (let t = parentOf(target); t; t = parentOf(t)) path.push(t);
    event.target = target;
    event[PATH] = [target, ...path];
    event[STOP] = 0;
    event.eventPhase = 1;
    for (let i = path.length - 1; i >= 0 && !event[STOP]; i--) invoke(path[i], event, 1);
    if (!event[STOP]) {
      event.eventPhase = 2;
      // At the target: capturing listeners first, then the rest.
      invoke(target, event, 1);
      if (!event[STOP]) invoke(target, event, 3);
    }
    if (event.bubbles) {
      event.eventPhase = 3;
      for (let i = 0; i < path.length && !event[STOP]; i++) invoke(path[i], event, 3);
    }
    event.eventPhase = 0;
    event.currentTarget = null;
    return !event.defaultPrevented;
  }

  class Event {
    constructor(type, init) {
      init = init || {};
      this.type = String(type);
      this.bubbles = !!init.bubbles;
      this.cancelable = !!init.cancelable;
      this.composed = !!init.composed;
      this.defaultPrevented = false;
      this.isTrusted = true;
      this.target = null;
      this.currentTarget = null;
      this.eventPhase = 0;
      this.timeStamp = N.now();
      this[STOP] = 0;
    }
    get srcElement() {
      return this.target;
    }
    get returnValue() {
      return !this.defaultPrevented;
    }
    get cancelBubble() {
      return !!this[STOP];
    }
    set cancelBubble(v) {
      if (v) this[STOP] |= 1;
    }
    preventDefault() {
      if (this.cancelable) this.defaultPrevented = true;
    }
    stopPropagation() {
      this[STOP] |= 1;
    }
    stopImmediatePropagation() {
      this[STOP] |= 3;
    }
    composedPath() {
      return this[PATH] ? this[PATH].slice() : [];
    }
  }
  Event.NONE = 0;
  Event.CAPTURING_PHASE = 1;
  Event.AT_TARGET = 2;
  Event.BUBBLING_PHASE = 3;

  function assignInit(event, init, defaults) {
    for (const key in defaults) event[key] = init && init[key] !== undefined ? init[key] : defaults[key];
  }

  class CustomEvent extends Event {
    constructor(type, init) {
      super(type, init);
      this.detail = init && init.detail !== undefined ? init.detail : null;
    }
  }
  class UIEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { view: null, detail: 0 });
    }
  }
  class FocusEvent extends UIEvent {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { relatedTarget: null });
    }
  }
  const MODIFIERS = { altKey: false, ctrlKey: false, shiftKey: false, metaKey: false };
  class KeyboardEvent extends UIEvent {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { key: '', code: '', location: 0, repeat: false, isComposing: false, ...MODIFIERS });
      this.charCode = 0;
      this.keyCode = 0;
      this.which = 0;
    }
    getModifierState(name) {
      switch (name) {
        case 'Alt':
          return this.altKey;
        case 'Control':
          return this.ctrlKey;
        case 'Shift':
          return this.shiftKey;
        case 'Meta':
          return this.metaKey;
        default:
          return false;
      }
    }
  }
  KeyboardEvent.DOM_KEY_LOCATION_STANDARD = 0;
  KeyboardEvent.DOM_KEY_LOCATION_LEFT = 1;
  KeyboardEvent.DOM_KEY_LOCATION_RIGHT = 2;
  KeyboardEvent.DOM_KEY_LOCATION_NUMPAD = 3;
  class MouseEvent extends UIEvent {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, {
        screenX: 0,
        screenY: 0,
        clientX: 0,
        clientY: 0,
        offsetX: 0,
        offsetY: 0,
        movementX: 0,
        movementY: 0,
        button: 0,
        buttons: 0,
        relatedTarget: null,
        ...MODIFIERS,
      });
    }
    get x() {
      return this.clientX;
    }
    get y() {
      return this.clientY;
    }
    get pageX() {
      return this.clientX;
    }
    get pageY() {
      return this.clientY;
    }
    getModifierState(name) {
      return KeyboardEvent.prototype.getModifierState.call(this, name);
    }
  }
  class PointerEvent extends MouseEvent {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, {
        pointerId: 1,
        pointerType: 'mouse',
        isPrimary: true,
        pressure: 0,
        tangentialPressure: 0,
        width: 1,
        height: 1,
        tiltX: 0,
        tiltY: 0,
        twist: 0,
      });
    }
    getCoalescedEvents() {
      return [this];
    }
    getPredictedEvents() {
      return [];
    }
  }
  class WheelEvent extends MouseEvent {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { deltaX: 0, deltaY: 0, deltaZ: 0, deltaMode: 0 });
    }
  }
  WheelEvent.DOM_DELTA_PIXEL = 0;
  WheelEvent.DOM_DELTA_LINE = 1;
  WheelEvent.DOM_DELTA_PAGE = 2;
  class MessageEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { data: null, origin: '', lastEventId: '', source: null, ports: [] });
    }
  }
  class PageTransitionEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { persisted: false });
    }
  }
  class CloseEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { code: 0, reason: '', wasClean: false });
    }
  }
  class ErrorEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { message: '', filename: '', lineno: 0, colno: 0, error: null });
    }
  }
  class ProgressEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { lengthComputable: false, loaded: 0, total: 0 });
    }
  }
  class MediaQueryListEvent extends Event {
    constructor(type, init) {
      super(type, init);
      assignInit(this, init, { media: '', matches: false });
    }
  }

  // ── Styles ─────────────────────────────────────────────────────────────────────────────────

  const kebab = (name) => (name.startsWith('--') ? name : name.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()));

  class CSSStyleDeclaration {
    constructor(onChange) {
      define(this, '_props', new Map());
      define(this, '_onChange', onChange || null);
    }
    get length() {
      return this._props.size;
    }
    item(i) {
      return [...this._props.keys()][i] || '';
    }
    getPropertyValue(name) {
      const v = this._props.get(String(name).trim().toLowerCase());
      return v === undefined ? '' : v.value;
    }
    getPropertyPriority(name) {
      const v = this._props.get(String(name).trim().toLowerCase());
      return v && v.important ? 'important' : '';
    }
    setProperty(name, value, priority) {
      name = String(name).trim().toLowerCase();
      if (value === null || value === undefined || String(value) === '') {
        this.removeProperty(name);
        return;
      }
      this._props.set(name, { value: String(value).trim(), important: priority === 'important' });
      if (this._onChange) this._onChange(name);
    }
    removeProperty(name) {
      name = String(name).trim().toLowerCase();
      const old = this.getPropertyValue(name);
      if (this._props.delete(name) && this._onChange) this._onChange(name);
      return old;
    }
    get cssText() {
      return [...this._props].map(([k, v]) => `${k}: ${v.value}${v.important ? ' !important' : ''};`).join(' ');
    }
    set cssText(text) {
      const changed = [...this._props.keys()];
      this._props.clear();
      for (const decl of String(text).split(';')) {
        const i = decl.indexOf(':');
        if (i < 0) continue;
        let value = decl.slice(i + 1).trim();
        const important = /!\s*important$/i.test(value);
        if (important) value = value.replace(/!\s*important$/i, '').trim();
        const name = decl.slice(0, i).trim().toLowerCase();
        if (name && value) {
          this._props.set(name, { value, important });
          changed.push(name);
        }
      }
      if (this._onChange) for (const name of new Set(changed)) this._onChange(name);
    }
  }

  // `style.fooBar = x` and `style['foo-bar']` reach the declaration through a proxy, as on a page.
  function makeStyle(onChange) {
    const decl = new CSSStyleDeclaration(onChange);
    return new Proxy(decl, {
      get(target, key, receiver) {
        if (typeof key !== 'string' || key in target) {
          const v = Reflect.get(target, key, target);
          return typeof v === 'function' ? v.bind(target) : v;
        }
        return target.getPropertyValue(kebab(key));
      },
      set(target, key, value) {
        if (typeof key !== 'string' || key in target) return Reflect.set(target, key, value, target);
        target.setProperty(kebab(key), value);
        return true;
      },
    });
  }

  // ── The document ───────────────────────────────────────────────────────────────────────────

  const ATTRS = Symbol('attributes');

  class Node extends EventTarget {
    constructor(doc, nodeName, nodeType) {
      super();
      define(this, '_doc', doc);
      this.nodeName = nodeName;
      this.nodeType = nodeType;
      this.parentNode = null;
      this.childNodes = [];
    }
    get ownerDocument() {
      return this._doc;
    }
    get parentElement() {
      return this.parentNode instanceof Element ? this.parentNode : null;
    }
    get firstChild() {
      return this.childNodes[0] || null;
    }
    get lastChild() {
      return this.childNodes[this.childNodes.length - 1] || null;
    }
    get nextSibling() {
      const p = this.parentNode;
      return p ? p.childNodes[p.childNodes.indexOf(this) + 1] || null : null;
    }
    get previousSibling() {
      const p = this.parentNode;
      return p ? p.childNodes[p.childNodes.indexOf(this) - 1] || null : null;
    }
    get isConnected() {
      for (let n = this; n; n = n.parentNode) if (n === document) return true;
      return false;
    }
    hasChildNodes() {
      return this.childNodes.length > 0;
    }
    appendChild(child) {
      return this.insertBefore(child, null);
    }
    insertBefore(child, ref) {
      if (child instanceof DocumentFragment) {
        for (const c of child.childNodes.slice()) this.insertBefore(c, ref);
        return child;
      }
      if (child.parentNode) child.parentNode.removeChild(child);
      const i = ref ? this.childNodes.indexOf(ref) : -1;
      if (i >= 0) this.childNodes.splice(i, 0, child);
      else this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    append(...nodes) {
      for (const n of nodes) this.appendChild(typeof n === 'string' ? new Text(this._doc || this, n) : n);
    }
    prepend(...nodes) {
      const first = this.firstChild;
      for (const n of nodes) this.insertBefore(typeof n === 'string' ? new Text(this._doc || this, n) : n, first);
    }
    removeChild(child) {
      const i = this.childNodes.indexOf(child);
      if (i >= 0) this.childNodes.splice(i, 1);
      child.parentNode = null;
      if (document && child.contains(document.activeElement)) document._focus(document.body);
      return child;
    }
    replaceChild(next, old) {
      this.insertBefore(next, old);
      return this.removeChild(old);
    }
    remove() {
      if (this.parentNode) this.parentNode.removeChild(this);
    }
    contains(other) {
      for (let n = other; n; n = n.parentNode) if (n === this) return true;
      return false;
    }
    getRootNode() {
      let n = this;
      while (n.parentNode) n = n.parentNode;
      return n;
    }
    cloneNode() {
      return this;
    }
    get textContent() {
      return this.childNodes.map((c) => c.textContent).join('');
    }
    set textContent(text) {
      for (const c of this.childNodes) c.parentNode = null;
      this.childNodes = [];
      if (text !== null && text !== undefined && String(text) !== '') this.appendChild(new Text(this._doc || this, String(text)));
    }
  }
  Node.ELEMENT_NODE = 1;
  Node.TEXT_NODE = 3;
  Node.DOCUMENT_NODE = 9;
  Node.DOCUMENT_FRAGMENT_NODE = 11;

  class Text extends Node {
    constructor(doc, data) {
      super(doc, '#text', 3);
      this.data = String(data);
    }
    get textContent() {
      return this.data;
    }
    set textContent(v) {
      this.data = String(v);
    }
    get nodeValue() {
      return this.data;
    }
  }

  class DocumentFragment extends Node {
    constructor(doc) {
      super(doc, '#document-fragment', 11);
    }
  }

  class DOMRectReadOnly {
    constructor(x = 0, y = 0, width = 0, height = 0) {
      this.x = x;
      this.y = y;
      this.width = width;
      this.height = height;
    }
    get left() {
      return Math.min(this.x, this.x + this.width);
    }
    get top() {
      return Math.min(this.y, this.y + this.height);
    }
    get right() {
      return Math.max(this.x, this.x + this.width);
    }
    get bottom() {
      return Math.max(this.y, this.y + this.height);
    }
    toJSON() {
      const { x, y, width, height, top, left, right, bottom } = this;
      return { x, y, width, height, top, left, right, bottom };
    }
  }
  class DOMRect extends DOMRectReadOnly {}

  class ClassList {
    constructor(el) {
      define(this, '_el', el);
    }
    _get() {
      return (this._el.getAttribute('class') || '').split(/\s+/).filter(Boolean);
    }
    _set(list) {
      this._el.setAttribute('class', list.join(' '));
    }
    get length() {
      return this._get().length;
    }
    contains(c) {
      return this._get().includes(c);
    }
    add(...cs) {
      const list = this._get();
      for (const c of cs) if (!list.includes(c)) list.push(c);
      this._set(list);
    }
    remove(...cs) {
      this._set(this._get().filter((c) => !cs.includes(c)));
    }
    toggle(c, force) {
      const has = this.contains(c);
      const want = force === undefined ? !has : !!force;
      if (want && !has) this.add(c);
      if (!want && has) this.remove(c);
      return want;
    }
  }

  // A small selector engine: compound selectors (`tag#id.class[attr=value]`, `*`) joined by
  // descendant (` `) and child (`>`) combinators, in comma lists.
  function parseCompound(text) {
    const parts = { tag: null, id: null, classes: [], attrs: [] };
    const re = /(\*)|([a-zA-Z][\w-]*)|#([\w-]+)|\.([\w-]+)|\[\s*([\w-]+)\s*(?:([~|^$*]?=)\s*(?:"([^"]*)"|'([^']*)'|([^\]\s]*)))?\s*\]/gy;
    let m;
    let at = 0;
    while (at < text.length) {
      re.lastIndex = at;
      if (!(m = re.exec(text))) return null;
      at = re.lastIndex;
      if (m[2]) parts.tag = m[2].toUpperCase();
      else if (m[3]) parts.id = m[3];
      else if (m[4]) parts.classes.push(m[4]);
      else if (m[5]) parts.attrs.push({ name: m[5].toLowerCase(), op: m[6], value: m[7] ?? m[8] ?? m[9] });
    }
    return parts;
  }
  function parseSelector(selector) {
    return String(selector)
      .split(',')
      .map((complex) => {
        const steps = [];
        const tokens = complex.trim().replace(/\s*>\s*/g, ' > ').split(/\s+/).filter(Boolean);
        let combinator = ' ';
        for (const t of tokens) {
          if (t === '>') {
            combinator = '>';
            continue;
          }
          const compound = parseCompound(t);
          if (!compound) throw new DOMException(`'${selector}' is not a valid selector`, 'SyntaxError');
          steps.push({ combinator, compound });
          combinator = ' ';
        }
        return steps;
      });
  }
  function matchesCompound(el, c) {
    if (!(el instanceof Element)) return false;
    if (c.tag && el.tagName !== c.tag) return false;
    if (c.id && el.id !== c.id) return false;
    for (const cls of c.classes) if (!el.classList.contains(cls)) return false;
    for (const a of c.attrs) {
      const v = el.getAttribute(a.name);
      if (v === null) return false;
      if (a.op === '=' && v !== a.value) return false;
      if (a.op === '^=' && !v.startsWith(a.value)) return false;
      if (a.op === '$=' && !v.endsWith(a.value)) return false;
      if (a.op === '*=' && !v.includes(a.value)) return false;
      if (a.op === '~=' && !v.split(/\s+/).includes(a.value)) return false;
    }
    return true;
  }
  function matchesSteps(el, steps, i) {
    if (!matchesCompound(el, steps[i].compound)) return false;
    if (i === 0) return true;
    if (steps[i].combinator === '>') return !!el.parentNode && matchesSteps(el.parentNode, steps, i - 1);
    for (let p = el.parentNode; p; p = p.parentNode) if (matchesSteps(p, steps, i - 1)) return true;
    return false;
  }
  function matchesSelector(el, parsed) {
    return parsed.some((steps) => steps.length && matchesSteps(el, steps, steps.length - 1));
  }
  function* descendants(node) {
    for (const c of node.childNodes) {
      if (c instanceof Element) yield c;
      yield* descendants(c);
    }
  }
  function querySelectorAll(root, selector) {
    const parsed = parseSelector(selector);
    const out = [];
    for (const el of descendants(root)) if (matchesSelector(el, parsed)) out.push(el);
    return out;
  }

  class NodeList extends Array {
    item(i) {
      return this[i] || null;
    }
  }
  const nodeList = (items) => NodeList.from(items);

  class Element extends Node {
    constructor(doc, tag) {
      super(doc, String(tag).toUpperCase(), 1);
      define(this, ATTRS, new Map());
      this.tagName = String(tag).toUpperCase();
      this.localName = String(tag).toLowerCase();
      this.style = makeStyle((name) => this._styleChanged(name));
      this.classList = new ClassList(this);
      this.dataset = {};
      this.hidden = false;
      this.tabIndex = -1;
      define(this, '_html', '');
    }
    _styleChanged(name) {}
    get id() {
      return this.getAttribute('id') || '';
    }
    set id(v) {
      this.setAttribute('id', v);
    }
    get className() {
      return this.getAttribute('class') || '';
    }
    set className(v) {
      this.setAttribute('class', v);
    }
    get children() {
      return this.childNodes.filter((c) => c instanceof Element);
    }
    get firstElementChild() {
      return this.children[0] || null;
    }
    get innerHTML() {
      return this._html;
    }
    set innerHTML(html) {
      // Markup is not parsed: the pages' overlays are not part of this host.
      this.textContent = '';
      this._html = String(html);
    }
    get outerHTML() {
      return `<${this.localName}>`;
    }
    get innerText() {
      return this.textContent;
    }
    set innerText(v) {
      this.textContent = v;
    }
    setAttribute(name, value) {
      name = String(name).toLowerCase();
      value = String(value);
      this[ATTRS].set(name, value);
      if (name === 'style') this.style.cssText = value;
      if (name === 'tabindex') this.tabIndex = parseInt(value, 10) || 0;
    }
    getAttribute(name) {
      const v = this[ATTRS].get(String(name).toLowerCase());
      return v === undefined ? null : v;
    }
    hasAttribute(name) {
      return this[ATTRS].has(String(name).toLowerCase());
    }
    removeAttribute(name) {
      this[ATTRS].delete(String(name).toLowerCase());
    }
    toggleAttribute(name, force) {
      const has = this.hasAttribute(name);
      const want = force === undefined ? !has : !!force;
      if (want && !has) this.setAttribute(name, '');
      if (!want && has) this.removeAttribute(name);
      return want;
    }
    getAttributeNames() {
      return [...this[ATTRS].keys()];
    }
    querySelector(selector) {
      return querySelectorAll(this, selector)[0] || null;
    }
    querySelectorAll(selector) {
      return nodeList(querySelectorAll(this, selector));
    }
    getElementsByTagName(tag) {
      return nodeList(querySelectorAll(this, tag));
    }
    matches(selector) {
      return matchesSelector(this, parseSelector(selector));
    }
    closest(selector) {
      const parsed = parseSelector(selector);
      for (let n = this; n instanceof Element; n = n.parentNode) if (matchesSelector(n, parsed)) return n;
      return null;
    }
    // Layout: the one canvas fills the viewport and everything else sits on top of it.
    getBoundingClientRect() {
      return new DOMRect(0, 0, viewport.width, viewport.height);
    }
    getClientRects() {
      return [this.getBoundingClientRect()];
    }
    get clientWidth() {
      return viewport.width;
    }
    get clientHeight() {
      return viewport.height;
    }
    get offsetWidth() {
      return viewport.width;
    }
    get offsetHeight() {
      return viewport.height;
    }
    get offsetLeft() {
      return 0;
    }
    get offsetTop() {
      return 0;
    }
    get scrollWidth() {
      return viewport.width;
    }
    get scrollHeight() {
      return viewport.height;
    }
    scrollIntoView() {}
    focus() {
      if (this._doc) this._doc._focus(this);
    }
    blur() {
      if (this._doc && this._doc.activeElement === this) this._doc._focus(this._doc.body);
    }
    click() {
      dispatch(this, new MouseEvent('click', { bubbles: true, cancelable: true, button: 0 }));
    }
    setPointerCapture() {}
    releasePointerCapture() {}
    hasPointerCapture() {
      return false;
    }
    requestPointerLock() {
      const doc = this._doc;
      rust('pointer_lock', true);
      if (doc.pointerLockElement !== this) {
        doc.pointerLockElement = this;
        queueTask(() => dispatch(doc, new Event('pointerlockchange', { bubbles: true })));
      }
      return Promise.resolve();
    }
    requestFullscreen() {
      const doc = this._doc;
      if (doc.fullscreenElement !== this) {
        doc.fullscreenElement = this;
        rust('fullscreen', true);
        queueTask(() => dispatch(this, new Event('fullscreenchange', { bubbles: true })));
      }
      return Promise.resolve();
    }
    webkitRequestFullscreen() {
      this.requestFullscreen();
    }
    animate(keyframes, options) {
      return new Animation(this, keyframes, options);
    }
    getAnimations() {
      return [];
    }
    attachShadow() {
      return new DocumentFragment(this._doc);
    }
  }

  class HTMLElement extends Element {}
  class HTMLDivElement extends HTMLElement {}
  class HTMLButtonElement extends HTMLElement {}
  class HTMLInputElement extends HTMLElement {
    constructor(doc, tag) {
      super(doc, tag);
      this.value = '';
      this.type = 'text';
      this.files = [];
    }
  }
  class HTMLAnchorElement extends HTMLElement {
    click() {
      // A download link (the saves page's export) or a navigation: neither is a page here.
      const href = this.getAttribute('href') || this.href || '';
      N.log(1, `link clicked: ${href}${this.download ? ' (download ' + this.download + ')' : ''}`);
    }
  }

  // The canvas the client draws into. Its pixel size (width/height) is whatever winit sets from
  // the ResizeObserver; its CSS size is always the viewport's.
  const CONTEXT = Symbol('context');
  const BITMAP = Symbol('bitmap');
  class HTMLCanvasElement extends HTMLElement {
    constructor(doc, tag) {
      super(doc, tag);
      define(this, '_width', 300);
      define(this, '_height', 150);
      define(this, CONTEXT, null);
      define(this, BITMAP, null);
    }
    get width() {
      return this._width;
    }
    set width(v) {
      this._width = Number(v) >>> 0;
    }
    get height() {
      return this._height;
    }
    set height(v) {
      this._height = Number(v) >>> 0;
    }
    getContext(kind, options) {
      kind = String(kind);
      const existing = this[CONTEXT];
      if (existing) return existing.kind === kind ? existing.context : null;
      let context = null;
      if (kind === 'webgpu' && typeof G.__aethus.createCanvasContext === 'function') {
        context = G.__aethus.createCanvasContext(this);
      } else if (kind === 'bitmaprenderer') {
        context = new ImageBitmapRenderingContext(this);
      }
      if (context) this[CONTEXT] = { kind, context };
      return context;
    }
    toBlob(callback, type) {
      const bitmap = this[BITMAP];
      queueTask(() => {
        const blob = new Blob([], { type: type || 'image/png' });
        blob[BITMAP] = bitmap ? { width: bitmap.width, height: bitmap.height, pixels: bitmap[PIXELS] } : null;
        callback(blob);
      });
    }
    toDataURL() {
      return 'data:,';
    }
    captureStream() {
      throw new DOMException('captureStream is not supported', 'NotSupportedError');
    }
    _styleChanged(name) {
      if (name === 'cursor' && this === canvas) cursorChanged(this.style.getPropertyValue('cursor'));
    }
  }

  // An <img>: only what winit's custom cursors use (a blob: URL and decode()).
  class HTMLImageElement extends HTMLElement {
    constructor(doc, tag) {
      super(doc, tag || 'img');
      define(this, '_src', '');
      this.complete = false;
      this.naturalWidth = 0;
      this.naturalHeight = 0;
      this.crossOrigin = null;
      this.decoding = 'auto';
    }
    get src() {
      return this._src;
    }
    set src(url) {
      this._src = String(url);
      const blob = objectUrls.get(this._src);
      const image = blob && blob[BITMAP];
      this.complete = true;
      this.naturalWidth = image ? image.width : 0;
      this.naturalHeight = image ? image.height : 0;
      queueTask(() => dispatch(this, new Event(blob ? 'load' : 'error')));
    }
    get width() {
      return this.naturalWidth;
    }
    get height() {
      return this.naturalHeight;
    }
    decode() {
      return objectUrls.has(this._src)
        ? Promise.resolve()
        : Promise.reject(new DOMException('The source image cannot be decoded.', 'EncodingError'));
    }
  }

  class Animation extends EventTarget {
    constructor(el, keyframes, options) {
      super();
      this.effect = { target: el, keyframes, options };
      this.playState = 'running';
      this.onfinish = null;
      this.oncancel = null;
      this.id = '';
      if (el === canvas) canvasAnimation(this, keyframes, options);
    }
    play() {
      this.playState = 'running';
      if (this.effect.target === canvas) canvasAnimation(this, this.effect.keyframes, this.effect.options);
    }
    pause() {
      this.playState = 'paused';
    }
    finish() {
      this.playState = 'finished';
    }
    cancel() {
      this.playState = 'idle';
      if (animatedCursor === this) {
        animatedCursor = null;
        cursorChanged(canvas.style.getPropertyValue('cursor'));
      }
    }
    get finished() {
      return Promise.resolve(this);
    }
  }

  class Document extends Node {
    constructor() {
      super(null, '#document', 9);
      this.documentElement = null;
      this.head = null;
      this.body = null;
      this.activeElement = null;
      this.fullscreenElement = null;
      this.pointerLockElement = null;
      this.visibilityState = 'visible';
      this.readyState = 'complete';
      this.title = '';
      this.cookie = '';
      this.fullscreenEnabled = true;
      this.characterSet = 'UTF-8';
      this.compatMode = 'CSS1Compat';
    }
    get hidden() {
      return this.visibilityState !== 'visible';
    }
    get defaultView() {
      return G;
    }
    get webkitFullscreenElement() {
      return this.fullscreenElement;
    }
    get location() {
      return G.location;
    }
    get URL() {
      return G.location.href;
    }
    hasFocus() {
      return windowFocused;
    }
    createElement(tag) {
      tag = String(tag).toLowerCase();
      switch (tag) {
        case 'canvas':
          return new HTMLCanvasElement(this, tag);
        case 'img':
          return new HTMLImageElement(this, tag);
        case 'div':
          return new HTMLDivElement(this, tag);
        case 'button':
          return new HTMLButtonElement(this, tag);
        case 'input':
          return new HTMLInputElement(this, tag);
        case 'a':
          return new HTMLAnchorElement(this, tag);
        default:
          return new HTMLElement(this, tag);
      }
    }
    createElementNS(ns, tag) {
      return this.createElement(String(tag).replace(/^.*:/, ''));
    }
    createTextNode(text) {
      return new Text(this, text);
    }
    createDocumentFragment() {
      return new DocumentFragment(this);
    }
    createEvent() {
      return new Event('');
    }
    getElementById(id) {
      id = String(id);
      for (const el of descendants(this)) if (el.id === id) return el;
      return null;
    }
    getElementsByTagName(tag) {
      return nodeList(querySelectorAll(this, tag));
    }
    querySelector(selector) {
      return querySelectorAll(this, selector)[0] || null;
    }
    querySelectorAll(selector) {
      return nodeList(querySelectorAll(this, selector));
    }
    exitFullscreen() {
      const el = this.fullscreenElement;
      if (el) {
        this.fullscreenElement = null;
        rust('fullscreen', false);
        queueTask(() => dispatch(el.isConnected ? el : this, new Event('fullscreenchange', { bubbles: true })));
      }
      return Promise.resolve();
    }
    webkitExitFullscreen() {
      this.exitFullscreen();
    }
    exitPointerLock() {
      if (this.pointerLockElement) {
        this.pointerLockElement = null;
        rust('pointer_lock', false);
        queueTask(() => dispatch(this, new Event('pointerlockchange', { bubbles: true })));
      }
    }
    getSelection() {
      return { removeAllRanges() {}, addRange() {}, toString: () => '' };
    }
    // Focus moves the way a click or element.focus() moves it: blur/focusout on the old element,
    // focus/focusin on the new one.
    _focus(el) {
      const old = this.activeElement;
      if (old === el) return;
      this.activeElement = el;
      if (!windowFocused) return;
      if (old && old !== this.body) {
        dispatch(old, new FocusEvent('blur', { relatedTarget: el }));
        dispatch(old, new FocusEvent('focusout', { bubbles: true, relatedTarget: el }));
      }
      if (el && el !== this.body) {
        dispatch(el, new FocusEvent('focus', { relatedTarget: old }));
        dispatch(el, new FocusEvent('focusin', { bubbles: true, relatedTarget: old }));
      }
    }
  }

  // ── Images and blobs (winit's custom cursors) ──────────────────────────────────────────────

  const PIXELS = Symbol('pixels');
  const BLOB_BYTES = Symbol('bytes');

  class ImageData {
    constructor(data, width, height) {
      if (typeof data === 'number') {
        height = width;
        width = data;
        data = new Uint8ClampedArray(width * height * 4);
      } else {
        // The glue hands a view of the wasm's memory: keep a copy.
        data = new Uint8ClampedArray(data);
        if (height === undefined) height = width ? data.length / 4 / width : 0;
      }
      this.data = data;
      this.width = width >>> 0;
      this.height = height >>> 0;
      this.colorSpace = 'srgb';
    }
  }

  class ImageBitmap {
    constructor(width, height, pixels) {
      this.width = width;
      this.height = height;
      define(this, PIXELS, pixels);
    }
    close() {
      this[PIXELS] = null;
      this.width = this.height = 0;
    }
  }

  function createImageBitmap(source, options) {
    return new Promise((resolve, reject) => {
      queueTask(() => {
        if (source instanceof ImageData) {
          resolve(new ImageBitmap(source.width, source.height, new Uint8Array(source.data)));
        } else if (source instanceof ImageBitmap) {
          resolve(new ImageBitmap(source.width, source.height, source[PIXELS]));
        } else if (source instanceof Blob && source[BITMAP]) {
          const b = source[BITMAP];
          resolve(new ImageBitmap(b.width, b.height, b.pixels));
        } else {
          reject(new DOMException('The source image could not be decoded.', 'InvalidStateError'));
        }
      });
    });
  }

  class ImageBitmapRenderingContext {
    constructor(canvasElement) {
      this.canvas = canvasElement;
    }
    transferFromImageBitmap(bitmap) {
      this.canvas[BITMAP] = bitmap;
      if (bitmap) {
        this.canvas.width = bitmap.width;
        this.canvas.height = bitmap.height;
      }
    }
  }

  class Blob {
    constructor(parts, options) {
      const chunks = (parts || []).map(toBytes);
      const size = chunks.reduce((n, c) => n + c.byteLength, 0);
      const bytes = new Uint8Array(size);
      let at = 0;
      for (const c of chunks) {
        bytes.set(c, at);
        at += c.byteLength;
      }
      define(this, BLOB_BYTES, bytes);
      define(this, BITMAP, null);
      this.type = options && options.type ? String(options.type).toLowerCase() : '';
    }
    get size() {
      return this[BLOB_BYTES].byteLength;
    }
    arrayBuffer() {
      return Promise.resolve(this[BLOB_BYTES].slice().buffer);
    }
    bytes() {
      return Promise.resolve(this[BLOB_BYTES].slice());
    }
    text() {
      return Promise.resolve(N.textDecode(this[BLOB_BYTES]));
    }
    slice(start, end, type) {
      return new Blob([this[BLOB_BYTES].slice(start, end)], { type });
    }
  }
  class File extends Blob {
    constructor(parts, name, options) {
      super(parts, options);
      this.name = String(name);
      this.lastModified = Date.now();
    }
  }

  // ── URLs ───────────────────────────────────────────────────────────────────────────────────

  const ORIGIN = 'https://aethus.invalid';
  const objectUrls = new Map();
  let objectUrlSeq = 0;

  class URLSearchParams {
    constructor(init) {
      define(this, '_list', []);
      if (typeof init === 'string') {
        for (const pair of init.replace(/^\?/, '').split('&')) {
          if (!pair) continue;
          const i = pair.indexOf('=');
          const dec = (s) => decodeURIComponent(s.replace(/\+/g, ' '));
          this._list.push(i < 0 ? [dec(pair), ''] : [dec(pair.slice(0, i)), dec(pair.slice(i + 1))]);
        }
      } else if (init && typeof init === 'object') {
        for (const [k, v] of Array.isArray(init) ? init : Object.entries(init)) this._list.push([String(k), String(v)]);
      }
    }
    get(name) {
      const e = this._list.find(([k]) => k === name);
      return e ? e[1] : null;
    }
    getAll(name) {
      return this._list.filter(([k]) => k === name).map(([, v]) => v);
    }
    has(name) {
      return this._list.some(([k]) => k === name);
    }
    set(name, value) {
      this.delete(name);
      this._list.push([String(name), String(value)]);
    }
    append(name, value) {
      this._list.push([String(name), String(value)]);
    }
    delete(name) {
      for (let i = this._list.length - 1; i >= 0; i--) if (this._list[i][0] === name) this._list.splice(i, 1);
    }
    forEach(fn) {
      for (const [k, v] of this._list) fn(v, k, this);
    }
    entries() {
      return this._list.map((e) => e.slice())[Symbol.iterator]();
    }
    keys() {
      return this._list.map(([k]) => k)[Symbol.iterator]();
    }
    values() {
      return this._list.map(([, v]) => v)[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
    toString() {
      return this._list.map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`).join('&');
    }
  }

  // Enough of the URL standard for this page: absolute URLs, and relative ones against a base.
  function parseUrl(input, base) {
    input = String(input).trim();
    const abs = /^([a-zA-Z][a-zA-Z0-9+.-]*):(.*)$/.exec(input);
    if (abs) {
      const protocol = abs[1].toLowerCase() + ':';
      let rest = abs[2];
      if (protocol === 'blob:' || protocol === 'data:' || protocol === 'about:') {
        return { protocol, host: '', hostname: '', port: '', pathname: rest, search: '', hash: '', opaque: true };
      }
      const m = /^\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/.exec(rest);
      if (!m) throw new TypeError(`Failed to construct 'URL': Invalid URL '${input}'`);
      const host = m[1].toLowerCase();
      const colon = host.lastIndexOf(':');
      return {
        protocol,
        host,
        hostname: colon >= 0 ? host.slice(0, colon) : host,
        port: colon >= 0 ? host.slice(colon + 1) : '',
        pathname: normalizePath(m[2] || '/'),
        search: m[3] && m[3] !== '?' ? m[3] : '',
        hash: m[4] && m[4] !== '#' ? m[4] : '',
      };
    }
    if (base === undefined) throw new TypeError(`Failed to construct 'URL': Invalid URL '${input}'`);
    const b = typeof base === 'string' ? parseUrl(base) : parseUrl(base.href);
    const m = /^([^?#]*)(\?[^#]*)?(#.*)?$/.exec(input);
    let pathname;
    let search = m[2] && m[2] !== '?' ? m[2] : '';
    const hash = m[3] && m[3] !== '#' ? m[3] : '';
    if (input.startsWith('//')) return parseUrl(b.protocol + input);
    if (m[1] === '') {
      pathname = b.pathname;
      if (!m[2]) search = b.search;
    } else if (m[1].startsWith('/')) {
      pathname = normalizePath(m[1]);
    } else {
      pathname = normalizePath(b.pathname.replace(/[^/]*$/, '') + m[1]);
    }
    return { protocol: b.protocol, host: b.host, hostname: b.hostname, port: b.port, pathname, search, hash };
  }
  function normalizePath(path) {
    const out = [];
    const parts = path.split('/');
    for (let i = 0; i < parts.length; i++) {
      const p = parts[i];
      if (p === '..') {
        if (out.length > 1) out.pop();
        if (i === parts.length - 1) out.push('');
      } else if (p === '.') {
        if (i === parts.length - 1) out.push('');
      } else {
        out.push(p);
      }
    }
    const joined = out.join('/');
    return joined.startsWith('/') ? joined : '/' + joined;
  }

  class URL {
    constructor(input, base) {
      const u = parseUrl(input instanceof URL ? input.href : input, base);
      define(this, '_u', u);
      define(this, '_params', null);
    }
    get protocol() {
      return this._u.protocol;
    }
    get host() {
      return this._u.host;
    }
    get hostname() {
      return this._u.hostname;
    }
    get port() {
      return this._u.port;
    }
    get pathname() {
      return this._u.pathname;
    }
    set pathname(v) {
      this._u.pathname = normalizePath(String(v));
    }
    get search() {
      return this._u.search;
    }
    set search(v) {
      v = String(v);
      this._u.search = v && v !== '?' ? (v.startsWith('?') ? v : '?' + v) : '';
      this._params = null;
    }
    get hash() {
      return this._u.hash;
    }
    set hash(v) {
      v = String(v);
      this._u.hash = v && v !== '#' ? (v.startsWith('#') ? v : '#' + v) : '';
    }
    get origin() {
      return this._u.opaque ? 'null' : `${this._u.protocol}//${this._u.host}`;
    }
    get searchParams() {
      if (!this._params) this._params = new URLSearchParams(this._u.search);
      return this._params;
    }
    get username() {
      return '';
    }
    get password() {
      return '';
    }
    get href() {
      const u = this._u;
      if (u.opaque) return u.protocol + u.pathname;
      return `${u.protocol}//${u.host}${u.pathname}${u.search}${u.hash}`;
    }
    toString() {
      return this.href;
    }
    toJSON() {
      return this.href;
    }
    static createObjectURL(blob) {
      const url = `blob:${ORIGIN}/${(++objectUrlSeq).toString(16).padStart(8, '0')}`;
      objectUrls.set(url, blob);
      return url;
    }
    static revokeObjectURL(url) {
      objectUrls.delete(String(url));
    }
    static canParse(input, base) {
      try {
        parseUrl(input, base);
        return true;
      } catch (e) {
        return false;
      }
    }
  }

  // ── Text ───────────────────────────────────────────────────────────────────────────────────

  class TextEncoder {
    get encoding() {
      return 'utf-8';
    }
    encode(text = '') {
      return N.textEncode(String(text));
    }
    encodeInto(text, view) {
      const r = N.textEncodeInto(String(text), view);
      return { read: r[0], written: r[1] };
    }
  }

  class TextDecoder {
    constructor(label = 'utf-8', options) {
      const l = String(label).trim().toLowerCase();
      if (l === 'utf-8' || l === 'utf8' || l === 'unicode-1-1-utf-8') this._kind = 'utf-8';
      else if (l === 'latin1' || l === 'iso-8859-1' || l === 'windows-1252' || l === 'ascii' || l === 'us-ascii') this._kind = 'windows-1252';
      else if (l === 'x-user-defined') this._kind = 'x-user-defined';
      else throw new RangeError(`The encoding label provided ('${label}') is invalid.`);
      this.fatal = !!(options && options.fatal);
      this.ignoreBOM = !!(options && options.ignoreBOM);
    }
    get encoding() {
      return this._kind;
    }
    decode(input) {
      if (input === undefined || input === null) return '';
      let bytes = toBytes(input);
      switch (this._kind) {
        case 'utf-8':
          if (!this.ignoreBOM && bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
            bytes = bytes.subarray(3);
          }
          return N.textDecode(bytes);
        case 'x-user-defined':
          return N.xUserDefined(bytes);
        default:
          return N.latin1Decode(bytes);
      }
    }
  }

  // ── Storage ────────────────────────────────────────────────────────────────────────────────

  class Storage {
    constructor(entries, persist) {
      define(this, '_map', new Map(entries));
      define(this, '_persist', persist);
    }
    get length() {
      return this._map.size;
    }
    key(i) {
      const k = [...this._map.keys()][i];
      return k === undefined ? null : k;
    }
    getItem(key) {
      const v = this._map.get(String(key));
      return v === undefined ? null : v;
    }
    setItem(key, value) {
      key = String(key);
      value = String(value);
      if (this._map.get(key) === value) return;
      this._map.set(key, value);
      if (this._persist) this._persist(key, value);
    }
    removeItem(key) {
      key = String(key);
      if (this._map.delete(key) && this._persist) this._persist(key, undefined);
    }
    clear() {
      for (const key of [...this._map.keys()]) this.removeItem(key);
    }
  }

  // ── Network: the game data, and nothing else ───────────────────────────────────────────────
  //
  // The client reads its data from `<data base>/<path>` (benilla-formats web.rs); every such URL
  // is answered from the embedder's patch chain. There is no other network: other same-origin
  // URLs are 404s, and other origins fail the way an offline page's requests fail.

  const DATA_PREFIX = ORIGIN + '/data/';

  function dataName(href) {
    if (!href.startsWith(DATA_PREFIX)) return null;
    const path = href.slice(DATA_PREFIX.length).replace(/[?#].*$/, '');
    if (!path || path === '__index') return null;
    try {
      return path.split('/').filter(Boolean).map(decodeURIComponent).join('\\');
    } catch (e) {
      return null;
    }
  }

  // GET or HEAD a URL: { status, bytes } or null for a network error.
  function load(method, href) {
    if (href.startsWith('blob:')) {
      const blob = objectUrls.get(href);
      return blob ? { status: 200, bytes: blob[BLOB_BYTES], type: blob.type } : null;
    }
    if (href.startsWith('data:')) {
      const m = /^data:([^,]*?)(;base64)?,(.*)$/s.exec(href);
      if (!m) return null;
      const bytes = m[2]
        ? Uint8Array.from(atob(m[3]), (c) => c.charCodeAt(0))
        : N.textEncode(decodeURIComponent(m[3]));
      return { status: 200, bytes, type: m[1] || 'text/plain' };
    }
    if (!href.startsWith(ORIGIN + '/')) return null;
    const name = dataName(href);
    if (name === null) return { status: 404, bytes: null };
    if (method === 'HEAD') return { status: rust('data_exists', name) ? 200 : 404, bytes: null };
    const bytes = rust('data_read', name);
    return bytes ? { status: 200, bytes, type: 'application/octet-stream' } : { status: 404, bytes: null };
  }

  function resolveHref(url) {
    return new URL(url, G.location.href).href;
  }

  class Headers {
    constructor(init) {
      define(this, '_map', new Map());
      if (init instanceof Headers) init.forEach((v, k) => this.append(k, v));
      else if (init) for (const [k, v] of Array.isArray(init) ? init : Object.entries(init)) this.append(k, v);
    }
    get(name) {
      const v = this._map.get(String(name).toLowerCase());
      return v === undefined ? null : v;
    }
    has(name) {
      return this._map.has(String(name).toLowerCase());
    }
    set(name, value) {
      this._map.set(String(name).toLowerCase(), String(value));
    }
    append(name, value) {
      const k = String(name).toLowerCase();
      this._map.set(k, this._map.has(k) ? `${this._map.get(k)}, ${value}` : String(value));
    }
    delete(name) {
      this._map.delete(String(name).toLowerCase());
    }
    forEach(fn) {
      for (const [k, v] of this._map) fn(v, k, this);
    }
    entries() {
      return this._map.entries();
    }
    keys() {
      return this._map.keys();
    }
    values() {
      return this._map.values();
    }
    [Symbol.iterator]() {
      return this._map.entries();
    }
  }

  class Request {
    constructor(input, init) {
      this.url = input instanceof Request ? input.url : resolveHref(input);
      this.method = ((init && init.method) || 'GET').toUpperCase();
      this.headers = new Headers(init && init.headers);
      this.signal = (init && init.signal) || null;
    }
  }

  class Response {
    constructor(body, init) {
      define(this, '_bytes', body == null ? null : toBytes(body));
      this.status = init && init.status !== undefined ? init.status : 200;
      this.statusText = (init && init.statusText) || (this.status === 200 ? 'OK' : this.status === 404 ? 'Not Found' : '');
      this.headers = new Headers(init && init.headers);
      this.url = (init && init.url) || '';
      this.type = 'basic';
      this.redirected = false;
      this.bodyUsed = false;
      this.body = null;
    }
    get ok() {
      return this.status >= 200 && this.status < 300;
    }
    _take() {
      if (this.bodyUsed) return Promise.reject(new TypeError('Body has already been consumed.'));
      this.bodyUsed = true;
      return Promise.resolve(this._bytes || new Uint8Array(0));
    }
    arrayBuffer() {
      return this._take().then(exactBuffer);
    }
    bytes() {
      return this._take().then((b) => new Uint8Array(exactBuffer(b)));
    }
    text() {
      return this._take().then((b) => N.textDecode(b));
    }
    json() {
      return this.text().then(JSON.parse);
    }
    blob() {
      return this._take().then((b) => new Blob([b], { type: this.headers.get('content-type') || '' }));
    }
    clone() {
      const r = new Response(this._bytes, { status: this.status, statusText: this.statusText, headers: this.headers, url: this.url });
      return r;
    }
    static error() {
      return new Response(null, { status: 0 });
    }
    static json(data, init) {
      return new Response(JSON.stringify(data), { ...init, headers: { 'content-type': 'application/json' } });
    }
  }

  class AbortSignal extends EventTarget {
    constructor() {
      super();
      this.aborted = false;
      this.reason = undefined;
      this.onabort = null;
    }
    throwIfAborted() {
      if (this.aborted) throw this.reason;
    }
    static abort(reason) {
      const c = new AbortController();
      c.abort(reason);
      return c.signal;
    }
    static timeout(ms) {
      const c = new AbortController();
      setTimeout(() => c.abort(new DOMException('signal timed out', 'TimeoutError')), ms);
      return c.signal;
    }
  }
  class AbortController {
    constructor() {
      this.signal = new AbortSignal();
    }
    abort(reason) {
      const s = this.signal;
      if (s.aborted) return;
      s.aborted = true;
      s.reason = reason !== undefined ? reason : new DOMException('signal is aborted without reason', 'AbortError');
      dispatch(s, new Event('abort'));
    }
  }

  function fetch(input, init) {
    let href;
    let method = 'GET';
    let signal = null;
    try {
      if (input instanceof Request) {
        href = input.url;
        method = input.method;
        signal = input.signal;
      } else {
        href = resolveHref(input);
      }
      if (init && init.method) method = String(init.method).toUpperCase();
      if (init && init.signal) signal = init.signal;
    } catch (e) {
      return Promise.reject(e);
    }
    return new Promise((resolve, reject) => {
      if (signal && signal.aborted) return reject(signal.reason);
      // A response arrives in a later task, as from a network.
      queueTask(() => {
        if (signal && signal.aborted) return reject(signal.reason);
        let r;
        try {
          r = load(method, href);
        } catch (e) {
          return reject(e);
        }
        if (!r) return reject(new TypeError('Failed to fetch'));
        const headers = {};
        if (r.type) headers['content-type'] = r.type;
        if (r.bytes) headers['content-length'] = String(r.bytes.byteLength);
        resolve(new Response(method === 'HEAD' ? null : r.bytes, { status: r.status, headers, url: href }));
      });
    });
  }

  class XMLHttpRequest extends EventTarget {
    constructor() {
      super();
      this.readyState = 0;
      this.status = 0;
      this.statusText = '';
      this.responseType = '';
      this.responseURL = '';
      this.timeout = 0;
      this.withCredentials = false;
      this.upload = new EventTarget();
      define(this, '_bytes', null);
      define(this, '_mime', null);
      define(this, '_method', 'GET');
      define(this, '_url', '');
      define(this, '_async', true);
      define(this, '_type', null);
    }
    open(method, url, async) {
      this._method = String(method).toUpperCase();
      this._url = resolveHref(url);
      this._async = async === undefined ? true : !!async;
      this._bytes = null;
      this.status = 0;
      this.readyState = 1;
    }
    setRequestHeader() {}
    overrideMimeType(mime) {
      this._mime = String(mime).toLowerCase();
    }
    getResponseHeader(name) {
      if (this.readyState < 2) return null;
      name = String(name).toLowerCase();
      if (name === 'content-type') return this._type || null;
      if (name === 'content-length') return this._bytes ? String(this._bytes.byteLength) : null;
      return null;
    }
    getAllResponseHeaders() {
      return this._type ? `content-type: ${this._type}\r\n` : '';
    }
    send() {
      const complete = () => {
        let r = null;
        try {
          r = load(this._method, this._url);
        } catch (e) {
          reportError(e, 'in an XMLHttpRequest');
        }
        this.readyState = 4;
        this.responseURL = this._url;
        if (!r) {
          this.status = 0;
          if (this._async) {
            dispatch(this, new Event('readystatechange'));
            dispatch(this, new ProgressEvent('error'));
            dispatch(this, new ProgressEvent('loadend'));
            return;
          }
          throw new DOMException(`Failed to execute 'send' on 'XMLHttpRequest': Failed to load '${this._url}'.`, 'NetworkError');
        }
        this.status = r.status;
        this.statusText = r.status === 200 ? 'OK' : 'Not Found';
        this._bytes = r.bytes;
        this._type = r.type || null;
        if (this._async) {
          dispatch(this, new Event('readystatechange'));
          dispatch(this, new ProgressEvent('load'));
          dispatch(this, new ProgressEvent('loadend'));
        }
      };
      if (this._async) queueTask(complete);
      else complete();
    }
    abort() {
      this.readyState = 0;
    }
    // The client reads bytes through responseText under `charset=x-user-defined` and keeps the
    // low 8 bits of each character, so a latin-1 string carries the same bytes more cheaply.
    get responseText() {
      if (!this._bytes) return '';
      return this._mime && this._mime.includes('x-user-defined') ? N.latin1Decode(this._bytes) : N.textDecode(this._bytes);
    }
    get response() {
      if (this.readyState !== 4) return this.responseType === '' || this.responseType === 'text' ? '' : null;
      switch (this.responseType) {
        case 'arraybuffer':
          return this._bytes ? exactBuffer(this._bytes) : new ArrayBuffer(0);
        case 'blob':
          return new Blob(this._bytes ? [this._bytes] : []);
        case 'json':
          try {
            return JSON.parse(N.textDecode(this._bytes || new Uint8Array(0)));
          } catch (e) {
            return null;
          }
        default:
          return this.responseText;
      }
    }
  }
  XMLHttpRequest.UNSENT = 0;
  XMLHttpRequest.OPENED = 1;
  XMLHttpRequest.HEADERS_RECEIVED = 2;
  XMLHttpRequest.LOADING = 3;
  XMLHttpRequest.DONE = 4;

  // There are no realm servers to reach: a socket fails to connect.
  class WebSocket extends EventTarget {
    constructor(url) {
      super();
      this.url = String(url);
      this.readyState = 0;
      this.binaryType = 'blob';
      this.bufferedAmount = 0;
      this.protocol = '';
      this.extensions = '';
      N.log(2, `WebSocket ${this.url}: no network in this host`);
      queueTask(() => {
        if (this.readyState === 3) return;
        this.readyState = 3;
        dispatch(this, new Event('error'));
        dispatch(this, new CloseEvent('close', { code: 1006, reason: '', wasClean: false }));
      });
    }
    send() {
      if (this.readyState === 0) throw new DOMException("Failed to execute 'send' on 'WebSocket': Still in CONNECTING state.", 'InvalidStateError');
    }
    close() {
      this.readyState = 3;
    }
  }
  WebSocket.CONNECTING = 0;
  WebSocket.OPEN = 1;
  WebSocket.CLOSING = 2;
  WebSocket.CLOSED = 3;

  class Worker extends EventTarget {
    constructor(url) {
      super();
      throw new DOMException(`Workers are not supported here (${url})`, 'SecurityError');
    }
  }

  // ── Message channels and window.postMessage ────────────────────────────────────────────────

  class MessagePort extends EventTarget {
    constructor() {
      super();
      define(this, '_other', null);
      define(this, '_started', false);
      define(this, '_closed', false);
      define(this, '_queue', []);
      define(this, '_onmessage', null);
    }
    get onmessage() {
      return this._onmessage;
    }
    set onmessage(fn) {
      this._onmessage = fn;
      this.start();
    }
    start() {
      if (this._started) return;
      this._started = true;
      for (const data of this._queue.splice(0)) queueTask(() => this._deliver(data));
    }
    close() {
      this._closed = true;
    }
    postMessage(data) {
      const other = this._other;
      if (!other || this._closed || other._closed) return;
      if (other._started) queueTask(() => other._deliver(data));
      else other._queue.push(data);
    }
    _deliver(data) {
      if (!this._closed) dispatch(this, new MessageEvent('message', { data }));
    }
  }
  class MessageChannel {
    constructor() {
      this.port1 = new MessagePort();
      this.port2 = new MessagePort();
      this.port1._other = this.port2;
      this.port2._other = this.port1;
    }
  }

  // ── Observers ──────────────────────────────────────────────────────────────────────────────

  class ResizeObserverSize {
    constructor(inlineSize, blockSize) {
      this.inlineSize = inlineSize;
      this.blockSize = blockSize;
    }
  }
  class ResizeObserverEntry {
    constructor(target, cssWidth, cssHeight, deviceWidth, deviceHeight) {
      this.target = target;
      this.contentRect = new DOMRectReadOnly(0, 0, cssWidth, cssHeight);
      const css = [new ResizeObserverSize(cssWidth, cssHeight)];
      this.contentBoxSize = css;
      this.borderBoxSize = css;
      define(this, '_device', [new ResizeObserverSize(deviceWidth, deviceHeight)]);
    }
    // A getter on the prototype, as in Chromium: winit asks the prototype whether the
    // device-pixel box is supported.
    get devicePixelContentBoxSize() {
      return this._device;
    }
  }

  const resizeObservers = new Set();
  class ResizeObserver {
    constructor(callback) {
      define(this, '_callback', callback);
      define(this, '_targets', new Map()); // element -> last [w, h, dw, dh] reported, or null
    }
    observe(target) {
      if (!this._targets.has(target)) this._targets.set(target, null);
      resizeObservers.add(this);
    }
    unobserve(target) {
      this._targets.delete(target);
      if (!this._targets.size) resizeObservers.delete(this);
    }
    disconnect() {
      this._targets.clear();
      resizeObservers.delete(this);
    }
  }

  const intersectionObservers = new Set();
  class IntersectionObserver {
    constructor(callback, options) {
      define(this, '_callback', callback);
      define(this, '_targets', new Map()); // element -> last isIntersecting reported, or null
      this.root = (options && options.root) || null;
      this.rootMargin = (options && options.rootMargin) || '0px';
      this.thresholds = [0];
    }
    observe(target) {
      if (!this._targets.has(target)) this._targets.set(target, null);
      intersectionObservers.add(this);
    }
    unobserve(target) {
      this._targets.delete(target);
      if (!this._targets.size) intersectionObservers.delete(this);
    }
    disconnect() {
      this._targets.clear();
      intersectionObservers.delete(this);
    }
    takeRecords() {
      return [];
    }
  }

  function notifyObservers(now) {
    for (const observer of [...resizeObservers]) {
      const entries = [];
      for (const [target, last] of observer._targets) {
        const connected = target.isConnected;
        const w = connected ? viewport.width : 0;
        const h = connected ? viewport.height : 0;
        const dw = Math.round(w * viewport.dpr);
        const dh = Math.round(h * viewport.dpr);
        if (last && last[0] === w && last[1] === h && last[2] === dw && last[3] === dh) continue;
        observer._targets.set(target, [w, h, dw, dh]);
        entries.push(new ResizeObserverEntry(target, w, h, dw, dh));
      }
      if (entries.length) runTask(() => observer._callback(entries, observer), 'in a ResizeObserver');
    }
    for (const observer of [...intersectionObservers]) {
      const entries = [];
      for (const [target, last] of observer._targets) {
        const visible = target.isConnected;
        if (last === visible) continue;
        observer._targets.set(target, visible);
        const rect = target.getBoundingClientRect();
        entries.push({
          target,
          time: now,
          isIntersecting: visible,
          intersectionRatio: visible ? 1 : 0,
          boundingClientRect: rect,
          intersectionRect: visible ? rect : new DOMRectReadOnly(),
          rootBounds: new DOMRectReadOnly(0, 0, viewport.width, viewport.height),
        });
      }
      if (entries.length) runTask(() => observer._callback(entries, observer), 'in an IntersectionObserver');
    }
  }

  // ── Media queries ──────────────────────────────────────────────────────────────────────────

  function evaluateMedia(query) {
    const q = String(query).trim().toLowerCase();
    let m;
    if ((m = /\(\s*(min-|max-)?resolution\s*:\s*([\d.]+)\s*dppx\s*\)/.exec(q))) {
      const v = Number(m[2]);
      if (m[1] === 'min-') return viewport.dpr >= v;
      if (m[1] === 'max-') return viewport.dpr <= v;
      return Math.abs(viewport.dpr - v) < 1e-9;
    }
    if ((m = /\(\s*(min-|max-)?-webkit-device-pixel-ratio\s*:\s*([\d.]+)\s*\)/.exec(q))) {
      const v = Number(m[2]);
      if (m[1] === 'min-') return viewport.dpr >= v;
      if (m[1] === 'max-') return viewport.dpr <= v;
      return Math.abs(viewport.dpr - v) < 1e-9;
    }
    if (/prefers-color-scheme\s*:\s*dark/.test(q)) return true;
    if (/prefers-color-scheme\s*:\s*light/.test(q)) return false;
    if (/prefers-reduced-motion\s*:\s*reduce/.test(q)) return false;
    if (/(any-)?pointer\s*:\s*fine/.test(q)) return true;
    if (/(any-)?pointer\s*:\s*(coarse|none)/.test(q)) return false;
    if (/(any-)?hover\s*:\s*hover/.test(q)) return true;
    if (/(any-)?hover\s*:\s*none/.test(q)) return false;
    if (/display-mode\s*:\s*fullscreen/.test(q)) return true;
    if (/orientation\s*:\s*landscape/.test(q)) return viewport.width >= viewport.height;
    if (/orientation\s*:\s*portrait/.test(q)) return viewport.width < viewport.height;
    return false;
  }

  const mediaLists = new Set();
  class MediaQueryList extends EventTarget {
    constructor(media) {
      super();
      this.media = String(media);
      this.onchange = null;
      define(this, '_last', evaluateMedia(this.media));
    }
    get matches() {
      return evaluateMedia(this.media);
    }
    addListener(fn) {
      this.addEventListener('change', fn);
      mediaLists.add(this);
    }
    removeListener(fn) {
      this.removeEventListener('change', fn);
    }
    addEventListener(type, fn, options) {
      super.addEventListener(type, fn, options);
      mediaLists.add(this);
    }
  }
  function matchMedia(query) {
    return new MediaQueryList(query);
  }
  function mediaChanged() {
    for (const mql of [...mediaLists]) {
      const now = evaluateMedia(mql.media);
      if (now === mql._last) continue;
      mql._last = now;
      const list = mql[LISTENERS] && mql[LISTENERS].get('change');
      if (!(list && list.length) && typeof mql.onchange !== 'function') {
        mediaLists.delete(mql);
        continue;
      }
      runTask(() => dispatch(mql, new MediaQueryListEvent('change', { media: mql.media, matches: now })), 'in a media query listener');
    }
  }

  function getComputedStyle(el) {
    const own = el && el.style ? el.style : makeStyle();
    const value = (prop) => {
      prop = String(prop).trim().toLowerCase();
      switch (prop) {
        case 'display':
          return own.getPropertyValue('display') === 'none' ? 'none' : 'block';
        case 'writing-mode':
          return 'horizontal-tb';
        case 'direction':
          return 'ltr';
        case 'visibility':
          return 'visible';
        case 'box-sizing':
          return own.getPropertyValue('box-sizing') || 'content-box';
        case 'width':
          return `${viewport.width}px`;
        case 'height':
          return `${viewport.height}px`;
        default:
          if (/^(padding|margin|border)(-(top|right|bottom|left))?(-width)?$/.test(prop)) return '0px';
          return own.getPropertyValue(prop);
      }
    };
    const computed = { getPropertyValue: value, getPropertyPriority: () => '', length: 0, item: () => '' };
    return new Proxy(computed, {
      get(target, key) {
        if (typeof key !== 'string' || key in target) return target[key];
        return value(kebab(key));
      },
    });
  }

  // ── Window, location, navigator ────────────────────────────────────────────────────────────

  const config = JSON.parse(rust('config') || '{}');
  const viewport = {
    width: (config.viewport && config.viewport[0]) || 1920,
    height: (config.viewport && config.viewport[1]) || 1080,
    dpr: config.dpr || 1,
  };
  let windowFocused = true;

  class Window extends EventTarget {}
  try {
    Object.setPrototypeOf(G, Window.prototype);
  } catch (e) {
    // A global whose prototype cannot change still needs the methods.
    for (const name of ['addEventListener', 'removeEventListener', 'dispatchEvent']) define(G, name, EventTarget.prototype[name]);
  }

  const location = {
    href: ORIGIN + '/',
    origin: ORIGIN,
    protocol: 'https:',
    host: 'aethus.invalid',
    hostname: 'aethus.invalid',
    port: '',
    pathname: '/',
    search: '',
    hash: '',
    ancestorOrigins: [],
    assign(url) {
      N.log(2, `navigation to ${url} ignored`);
    },
    replace(url) {
      N.log(2, `navigation to ${url} ignored`);
    },
    reload() {
      rust('reload');
    },
    toString() {
      return this.href;
    },
  };
  // Assigning location.href navigates; a page that wants to leave is told so in the log.
  let href = location.href;
  Object.defineProperty(location, 'href', {
    get: () => href,
    set: (url) => {
      const next = resolveHref(url);
      if (next.split('#')[0] === href.split('#')[0]) {
        href = next;
        location.hash = new URL(next).hash;
      } else {
        N.log(2, `navigation to ${next} ignored`);
      }
    },
    enumerable: true,
  });

  const gamepadSlots = [null, null, null, null];
  let gamepadsRead = false;
  function getGamepads() {
    if (!gamepadsRead) {
      gamepadsRead = true;
      const text = rust('gamepads');
      const list = text ? JSON.parse(text) : [];
      for (let i = 0; i < gamepadSlots.length; i++) {
        const g = list[i];
        if (!g) {
          gamepadSlots[i] = null;
          continue;
        }
        gamepadSlots[i] = {
          id: g.id || 'Aethus pad',
          index: i,
          connected: true,
          mapping: 'standard',
          timestamp: N.now(),
          axes: g.axes || [0, 0, 0, 0],
          buttons: (g.buttons || []).map((b) => ({ pressed: !!b[0], touched: !!b[0], value: Number(b[1]) || 0 })),
          vibrationActuator: null,
        };
      }
    }
    return gamepadSlots.slice();
  }

  const navigator = {
    userAgent: config.userAgent || 'Mozilla/5.0 (PlayStation 5) AethusHost/1.0',
    appVersion: '5.0',
    appName: 'Netscape',
    product: 'Gecko',
    vendor: '',
    platform: config.platform || '',
    language: config.language || 'en-US',
    languages: [config.language || 'en-US'],
    onLine: true,
    cookieEnabled: false,
    hardwareConcurrency: 1,
    maxTouchPoints: 0,
    deviceMemory: 8,
    webdriver: false,
    pdfViewerEnabled: false,
    getGamepads,
    sendBeacon: () => false,
    vibrate: () => false,
  };

  const screenObject = {
    get width() {
      return viewport.width;
    },
    get height() {
      return viewport.height;
    },
    get availWidth() {
      return viewport.width;
    },
    get availHeight() {
      return viewport.height;
    },
    colorDepth: 24,
    pixelDepth: 24,
    orientation: Object.assign(new EventTarget(), {
      get type() {
        return viewport.width >= viewport.height ? 'landscape-primary' : 'portrait-primary';
      },
      angle: 0,
      lock: () => Promise.reject(new DOMException('screen.orientation.lock() is not available on this device.', 'NotSupportedError')),
      unlock() {},
    }),
  };

  // The persisted localStorage of the page, kept by the embedder.
  let storedEntries = [];
  try {
    const text = rust('storage_load');
    if (text) storedEntries = Object.entries(JSON.parse(text)).map(([k, v]) => [k, String(v)]);
  } catch (e) {
    reportError(e, 'reading localStorage');
  }
  const localStorage = new Storage(storedEntries, (key, value) => rust('storage_set', key, value));
  const sessionStorage = new Storage([], null);

  // A new object rather than QuickJS's own `performance`, whose `now` cannot be replaced: the
  // page's clock is the embedder's, the one timers and animation frames run on.
  const performanceObject = {};
  define(performanceObject, 'now', () => N.now());
  define(performanceObject, 'mark', () => undefined);
  define(performanceObject, 'measure', () => undefined);
  define(performanceObject, 'clearMarks', () => {});
  define(performanceObject, 'clearMeasures', () => {});
  define(performanceObject, 'getEntriesByName', () => []);
  define(performanceObject, 'getEntriesByType', () => []);
  define(performanceObject, 'timeOrigin', Date.now() - N.now());

  const cryptoObject = {
    getRandomValues(view) {
      if (!ArrayBuffer.isView(view) || view instanceof Float32Array || view instanceof Float64Array) {
        throw new DOMException("Failed to execute 'getRandomValues' on 'Crypto': The provided ArrayBufferView is not an integer array type.", 'TypeMismatchError');
      }
      if (view.byteLength > 65536) {
        throw new DOMException("Failed to execute 'getRandomValues' on 'Crypto': The ArrayBufferView's byte length exceeds the number of bytes of entropy available via this API (65536).", 'QuotaExceededError');
      }
      N.randomFill(view);
      return view;
    },
    randomUUID() {
      const b = N.randomFill(new Uint8Array(16));
      b[6] = (b[6] & 0x0f) | 0x40;
      b[8] = (b[8] & 0x3f) | 0x80;
      const h = [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
      return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
    },
  };

  function structuredClone(value) {
    if (value === null || typeof value !== 'object') return value;
    if (ArrayBuffer.isView(value)) return value.slice();
    if (value instanceof ArrayBuffer) return value.slice(0);
    if (value instanceof Date) return new Date(value.getTime());
    if (value instanceof Map) return new Map([...value].map(([k, v]) => [structuredClone(k), structuredClone(v)]));
    if (value instanceof Set) return new Set([...value].map(structuredClone));
    if (Array.isArray(value)) return value.map(structuredClone);
    const out = {};
    for (const k of Object.keys(value)) out[k] = structuredClone(value[k]);
    return out;
  }

  // ── The page: <html><head></head><body><canvas id="benilla" tabindex="0"></canvas></body> ─────

  const document = new Document();
  const html = document.createElement('html');
  const head = document.createElement('head');
  const body = document.createElement('body');
  document.appendChild(html);
  html.appendChild(head);
  html.appendChild(body);
  document.documentElement = html;
  document.head = head;
  document.body = body;
  document.activeElement = body;
  const canvas = document.createElement('canvas');
  canvas.setAttribute('id', 'benilla');
  canvas.setAttribute('tabindex', '0');
  body.appendChild(canvas);

  // ── Cursors ────────────────────────────────────────────────────────────────────────────────
  //
  // winit sets the canvas's CSS `cursor`: a keyword, or `url("blob:…") x y, auto` for a custom
  // cursor whose image it drew into a bitmaprenderer canvas and turned into a blob. The embedder
  // gets the keyword, or the RGBA pixels and hotspot.

  let lastCursor = null;
  let animatedCursor = null;
  function cursorChanged(css) {
    if (animatedCursor) return;
    applyCursor(css);
  }
  function applyCursor(css) {
    css = String(css || 'auto').trim();
    if (css === lastCursor) return;
    lastCursor = css;
    const m = /url\(\s*["']?([^"')]+)["']?\s*\)\s*(-?[\d.]+)?\s*(-?[\d.]+)?/.exec(css);
    if (m) {
      const blob = objectUrls.get(m[1]);
      const image = blob && blob[BITMAP];
      if (image && image.pixels) {
        rust('set_cursor', 'image', image.pixels, image.width, image.height, Number(m[2]) || 0, Number(m[3]) || 0);
        return;
      }
      const fallback = /,\s*([a-z-]+)\s*$/.exec(css);
      rust('set_cursor', fallback ? fallback[1] : 'default');
      return;
    }
    rust('set_cursor', css.split(',').pop().trim() || 'auto');
  }
  // An animated cursor is a CSS animation of the `cursor` property; it is shown by its first
  // frame.
  function canvasAnimation(animation, keyframes, options) {
    const frames = Array.isArray(keyframes) ? keyframes : [];
    const first = frames.find((f) => f && f.cursor);
    if (!first) return;
    animatedCursor = null;
    applyCursor(first.cursor);
    animatedCursor = animation;
  }

  // ── Platform input ─────────────────────────────────────────────────────────────────────────
  //
  // The embedder's events, as `poll_events` JSON:
  //   { t: 'key', down, key, code, location, repeat, m }
  //   { t: 'move', x, y, dx, dy, m } / { t: 'down' | 'up', b, x, y, m } / { t: 'wheel', x, y, dx, dy, mode, m }
  //   { t: 'over', x, y } / { t: 'out' }
  //   { t: 'resize', width, height, dpr } / { t: 'focus', focused } / { t: 'visible', visible }
  // `m` is the modifier bits (1 shift, 2 ctrl, 4 alt, 8 meta); x/y are CSS pixels in the canvas.

  const BUTTON_BITS = [1, 4, 2, 8, 16];
  let buttons = 0;
  let pointer = { x: 0, y: 0, inside: false };

  function modifierInit(m) {
    return { shiftKey: !!(m & 1), ctrlKey: !!(m & 2), altKey: !!(m & 4), metaKey: !!(m & 8) };
  }

  function pointerInit(e, button, extra) {
    const locked = !!document.pointerLockElement;
    const x = locked ? pointer.x : e.x !== undefined ? e.x : pointer.x;
    const y = locked ? pointer.y : e.y !== undefined ? e.y : pointer.y;
    return {
      bubbles: true,
      cancelable: true,
      composed: true,
      view: G,
      pointerId: 1,
      pointerType: 'mouse',
      isPrimary: true,
      button,
      buttons,
      pressure: buttons ? 0.5 : 0,
      clientX: x,
      clientY: y,
      offsetX: x,
      offsetY: y,
      screenX: x,
      screenY: y,
      movementX: e.dx || 0,
      movementY: e.dy || 0,
      ...modifierInit(e.m || 0),
      ...extra,
    };
  }

  function pointerTarget() {
    return document.pointerLockElement || canvas;
  }

  function platformEvent(e) {
    switch (e.t) {
      case 'pointer_mode':
        document.exitPointerLock();
        pointer.x = e.x;
        pointer.y = e.y;
        dispatch(canvas, new PointerEvent('pointermove', pointerInit(e, -1, { movementX: 0, movementY: 0 })));
        break;
      case 'key': {
        const target = document.activeElement || body;
        const init = {
          bubbles: true,
          cancelable: true,
          composed: true,
          view: G,
          key: e.key || 'Unidentified',
          code: e.code || '',
          location: e.location || 0,
          repeat: !!e.repeat,
          ...modifierInit(e.m || 0),
        };
        dispatch(target, new KeyboardEvent(e.down ? 'keydown' : 'keyup', init));
        break;
      }
      case 'over':
        pointer = { x: e.x || 0, y: e.y || 0, inside: true };
        dispatch(canvas, new PointerEvent('pointerover', pointerInit(e, -1)));
        break;
      case 'out':
        if (!pointer.inside) break;
        pointer.inside = false;
        dispatch(canvas, new PointerEvent('pointerout', pointerInit(e, -1)));
        break;
      case 'move':
        if (!pointer.inside && !document.pointerLockElement) {
          pointer.inside = true;
          dispatch(canvas, new PointerEvent('pointerover', pointerInit(e, -1, { movementX: 0, movementY: 0 })));
        }
        if (!document.pointerLockElement) {
          pointer.x = e.x;
          pointer.y = e.y;
        }
        dispatch(pointerTarget(), new PointerEvent('pointermove', pointerInit(e, -1)));
        break;
      case 'down': {
        const button = e.b || 0;
        if (!document.pointerLockElement) {
          pointer.x = e.x;
          pointer.y = e.y;
        }
        const first = buttons === 0;
        buttons |= BUTTON_BITS[button] || 0;
        if (document.activeElement !== canvas) canvas.focus();
        // A second button pressed while another is held is a pointermove, as in browsers.
        dispatch(pointerTarget(), new PointerEvent(first ? 'pointerdown' : 'pointermove', pointerInit(e, button)));
        break;
      }
      case 'up': {
        const button = e.b || 0;
        if (!document.pointerLockElement) {
          pointer.x = e.x;
          pointer.y = e.y;
        }
        buttons &= ~(BUTTON_BITS[button] || 0);
        dispatch(pointerTarget(), new PointerEvent(buttons === 0 ? 'pointerup' : 'pointermove', pointerInit(e, button)));
        if (button === 2) dispatch(pointerTarget(), new PointerEvent('contextmenu', pointerInit(e, button)));
        break;
      }
      case 'wheel':
        dispatch(pointerTarget(), new WheelEvent('wheel', { ...pointerInit(e, 0), deltaX: e.dx || 0, deltaY: e.dy || 0, deltaMode: e.mode || 0, movementX: 0, movementY: 0 }));
        break;
      case 'resize': {
        const dprChanged = e.dpr && e.dpr !== viewport.dpr;
        if (e.width) viewport.width = e.width;
        if (e.height) viewport.height = e.height;
        if (e.dpr) viewport.dpr = e.dpr;
        dispatch(G, new UIEvent('resize'));
        if (dprChanged) mediaChanged();
        else mediaChanged();
        break;
      }
      case 'focus':
        if (!!e.focused === windowFocused) break;
        windowFocused = !!e.focused;
        if (windowFocused) {
          dispatch(G, new FocusEvent('focus'));
          const el = document.activeElement;
          if (el && el !== body) {
            dispatch(el, new FocusEvent('focus'));
            dispatch(el, new FocusEvent('focusin', { bubbles: true }));
          }
        } else {
          const el = document.activeElement;
          if (el && el !== body) {
            dispatch(el, new FocusEvent('blur'));
            dispatch(el, new FocusEvent('focusout', { bubbles: true }));
          }
          dispatch(G, new FocusEvent('blur'));
        }
        break;
      case 'visible': {
        const state = e.visible ? 'visible' : 'hidden';
        if (document.visibilityState === state) break;
        document.visibilityState = state;
        dispatch(document, new Event('visibilitychange', { bubbles: true }));
        break;
      }
      default:
        N.log(2, `unknown platform event ${JSON.stringify(e)}`);
    }
  }

  function pumpPlatformEvents() {
    const text = rust('poll_events');
    if (!text) return;
    let events;
    try {
      events = JSON.parse(text);
    } catch (e) {
      reportError(e, 'reading platform events');
      return;
    }
    for (const e of events) runTask(() => platformEvent(e), `in a ${e.t} event`);
  }

  // ── Animation frames and the host's entry points ───────────────────────────────────────────

  let frameSeq = 0;
  let frameCallbacks = new Map();
  let runningFrame = null;
  function requestAnimationFrame(callback) {
    const id = ++frameSeq;
    frameCallbacks.set(id, callback);
    return id;
  }
  function cancelAnimationFrame(id) {
    frameCallbacks.delete(id);
    if (runningFrame) runningFrame.delete(id);
  }

  // Hooks gpu.js and audio.js add: run at the start of every tick (completions to deliver) and
  // after every frame (the canvas to present).
  const tickHooks = [];
  const frameHooks = [];

  const host = {
    // Runs what is due; returns ms until the next timer, 0 when tasks are waiting, -1 for none.
    tick(now) {
      gamepadsRead = false;
      pumpPlatformEvents();
      for (const hook of tickHooks) runTask(hook, 'in a host hook');
      runTimers(now);
      const batch = tasks;
      tasks = [];
      for (const fn of batch) runTask(fn, 'in a task');
      if (tasks.length) return 0;
      return nextTimerDelay(N.now());
    },
    // One display refresh: animation frame callbacks, then observers, as a browser updates the
    // rendering.
    frame(now) {
      const batch = frameCallbacks;
      frameCallbacks = new Map();
      runningFrame = batch;
      for (const [id, callback] of batch) {
        if (!batch.has(id)) continue; // cancelled by an earlier callback
        runTask(() => callback(now), 'in an animation frame');
      }
      runningFrame = null;
      notifyObservers(now);
      for (const hook of frameHooks) runTask(hook, 'in a host hook');
    },
    wantsFrame() {
      return frameCallbacks.size > 0;
    },
  };

  // ── Globals ────────────────────────────────────────────────────────────────────────────────

  const globals = {
    window: G,
    self: G,
    top: G,
    parent: G,
    frames: G,
    document,
    location,
    navigator,
    screen: screenObject,
    localStorage,
    sessionStorage,
    performance: performanceObject,
    crypto: cryptoObject,
    origin: ORIGIN,
    isSecureContext: true,
    crossOriginIsolated: false,
    closed: false,
    name: '',
    length: 0,
    requestAnimationFrame,
    cancelAnimationFrame,
    matchMedia,
    getComputedStyle,
    structuredClone,
    createImageBitmap,
    fetch,
    reportError: (e) => reportError(e),
    alert: (message) => N.log(2, `alert: ${message}`),
    confirm: (message) => {
      N.log(2, `confirm: ${message}`);
      return false;
    },
    prompt: (message) => {
      N.log(2, `prompt: ${message}`);
      return null;
    },
    open: (url) => {
      N.log(2, `window.open(${url}) ignored`);
      return null;
    },
    close: () => rust('quit'),
    focus: () => {},
    blur: () => {},
    print: () => {},
    stop: () => {},
    scroll: () => {},
    scrollTo: () => {},
    scrollBy: () => {},
    getSelection: () => document.getSelection(),
    postMessage(data) {
      queueTask(() => dispatch(G, new MessageEvent('message', { data, origin: ORIGIN, source: G })));
    },
    EventTarget,
    Event,
    CustomEvent,
    UIEvent,
    FocusEvent,
    KeyboardEvent,
    MouseEvent,
    PointerEvent,
    WheelEvent,
    MessageEvent,
    PageTransitionEvent,
    CloseEvent,
    ErrorEvent,
    ProgressEvent,
    MediaQueryListEvent,
    MediaQueryList,
    Node,
    Text,
    DocumentFragment,
    Element,
    HTMLElement,
    HTMLDivElement,
    HTMLButtonElement,
    HTMLInputElement,
    HTMLAnchorElement,
    HTMLCanvasElement,
    HTMLImageElement,
    Image: function Image(width, height) {
      const img = new HTMLImageElement(document, 'img');
      if (width !== undefined) img.naturalWidth = width;
      if (height !== undefined) img.naturalHeight = height;
      return img;
    },
    Document,
    Window,
    NodeList,
    DOMRect,
    DOMRectReadOnly,
    CSSStyleDeclaration,
    Animation,
    ImageData,
    ImageBitmap,
    ImageBitmapRenderingContext,
    Blob,
    File,
    URL,
    URLSearchParams,
    TextEncoder,
    TextDecoder,
    Storage,
    Headers,
    Request,
    Response,
    AbortController,
    AbortSignal,
    XMLHttpRequest,
    WebSocket,
    Worker,
    MessageChannel,
    MessagePort,
    ResizeObserver,
    ResizeObserverEntry,
    ResizeObserverSize,
    IntersectionObserver,
    WebGL2RenderingContext: class WebGL2RenderingContext {},
    WebGLRenderingContext: class WebGLRenderingContext {},
  };
  globals.Image.prototype = HTMLImageElement.prototype;
  for (const [name, value] of Object.entries(globals)) define(G, name, value);
  for (const name of ['innerWidth', 'outerWidth']) getter(G, name, () => viewport.width);
  for (const name of ['innerHeight', 'outerHeight']) getter(G, name, () => viewport.height);
  getter(G, 'devicePixelRatio', () => viewport.dpr);
  for (const name of ['screenX', 'screenY', 'screenLeft', 'screenTop', 'scrollX', 'scrollY', 'pageXOffset', 'pageYOffset']) getter(G, name, () => 0);

  if (typeof G.DOMException !== 'function') {
    class DOMException extends Error {
      constructor(message = '', name = 'Error') {
        super(message);
        this.name = name;
      }
    }
    define(G, 'DOMException', DOMException);
  }

  // A browser page has none of these; their presence would steer crates to Node.js or worker
  // paths.
  for (const name of ['global', 'process', 'module', 'require', 'WorkerGlobalScope', 'msCrypto', 'scheduler', 'requestIdleCallback', 'cancelIdleCallback']) {
    if (name in G) delete G[name];
  }

  define(G, '__host', host);
  define(G, '__aethus', {
    config,
    viewport,
    rust,
    opId,
    define,
    getter,
    queueTask,
    runTask,
    reportError,
    dispatch,
    tickHooks,
    frameHooks,
    canvas,
    toBytes,
    exactBuffer,
    inspect,
    createCanvasContext: null,
  });
})();
