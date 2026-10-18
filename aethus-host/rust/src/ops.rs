//! The operations the page calls (`rust(name, ...)` in `../js`), and the host state they act on.
//!
//! The state lives in a thread-local on the thread that runs the page. Operations run inside
//! `ffi::js_tick` and `ffi::js_frame`, so the platform's loop takes the state only between those
//! calls (`with`), never across one. What an operation asks of the window (a cursor, pointer
//! lock, full screen, quitting) is queued as a `Request` for the loop to carry out.

use std::cell::RefCell;

use serde_json::Value;

use crate::audio::Audio;
use crate::data::Data;
use crate::ffi::{self, Arg, Ret};
use crate::files::Files;
use crate::gpu::Gpu;
use crate::log::{fail, say, warn};
use crate::storage::Storage;

pub enum Cursor {
    /// A CSS cursor keyword ('default', 'pointer', 'none', ...).
    Named(String),
    /// Straight RGBA pixels and the hotspot.
    Image { rgba: Vec<u8>, width: u32, height: u32, hot_x: u32, hot_y: u32 },
}

pub enum Request {
    Cursor(Cursor),
    PointerLock(bool),
    Fullscreen(bool),
}

pub struct Host {
    /// What `config` returns: JSON `{ viewport, dpr, userAgent, platform, language, rogue, env,
    /// addonsDir }`.
    pub config: String,
    pub data: Data,
    pub storage: Storage,
    pub files: Files,
    pub gpu: Gpu,
    pub audio: Audio,
    /// Platform events for the page, each a JSON object, in order.
    pub events: Vec<String>,
    /// The gamepads as the platform last read them, JSON (see `gamepads`).
    pub gamepads: String,
    pub requests: Vec<Request>,
    /// Why the page asked to end, once it has.
    pub quit: Option<String>,
    /// The client's last boot stage.
    pub stage: String,
}

impl Host {
    /// Queues a platform event (a JSON object) for the page's next tick.
    pub fn event(&mut self, event: Value) {
        self.events.push(event.to_string());
    }
}

thread_local! {
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

pub fn install(host: Host) {
    HOST.with(|h| *h.borrow_mut() = Some(host));
}

/// Runs `f` on the host state; None when there is none, or when the state is already in use (an
/// operation calling back into the loop, which does not happen).
pub fn with<R>(f: impl FnOnce(&mut Host) -> R) -> Option<R> {
    HOST.with(|h| h.try_borrow_mut().ok().and_then(|mut h| h.as_mut().map(f)))
}

/// Takes the host state away, to be dropped.
pub fn take() -> Option<Host> {
    HOST.with(|h| h.try_borrow_mut().ok().and_then(|mut h| h.take()))
}

// ── Arguments ────────────────────────────────────────────────────────────────────────────────

fn num(args: &[Arg], i: usize) -> f64 {
    args.get(i).and_then(Arg::num).filter(|n| !n.is_nan()).unwrap_or(0.0)
}

fn uint(args: &[Arg], i: usize) -> u32 {
    num(args, i).max(0.0) as u32
}

fn text<'a>(args: &'a [Arg], i: usize) -> &'a str {
    args.get(i).and_then(Arg::str).unwrap_or_default()
}

fn bytes<'a>(args: &'a [Arg], i: usize) -> &'a [u8] {
    args.get(i).and_then(Arg::bytes).unwrap_or_default()
}

fn flag(args: &[Arg], i: usize) -> bool {
    args.get(i).is_some_and(Arg::truthy)
}

/// A JSON argument; anything but a JSON string reads as null.
fn json(args: &[Arg], i: usize) -> Result<Value, String> {
    match args.get(i).and_then(Arg::str) {
        Some(s) if !s.is_empty() => serde_json::from_str(s).map_err(|e| format!("argument {i} is not JSON: {e}")),
        _ => Ok(Value::Null),
    }
}

/// The ids in a Float64Array's bytes.
fn ids(bytes: &[u8]) -> impl Iterator<Item = u32> + '_ {
    bytes.chunks_exact(8).map(|b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as u32)
}

fn f64s(bytes: &[u8]) -> Vec<f64> {
    bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        .collect()
}

fn unit(r: Result<(), String>) -> Ret {
    match r {
        Ok(()) => Ret::Undefined,
        Err(e) => Ret::Error(e),
    }
}

fn id(r: Result<u32, String>) -> Ret {
    match r {
        Ok(id) => Ret::Number(f64::from(id)),
        Err(e) => Ret::Error(e),
    }
}

fn string(r: Result<String, String>) -> Ret {
    match r {
        Ok(s) => Ret::Str(s),
        Err(e) => Ret::Error(e),
    }
}

// ── The operations ───────────────────────────────────────────────────────────────────────────

type Op = fn(&mut Host, &[Arg]) -> Ret;

static OPS: &[(&str, Op)] = &[
    // The page (env.js, boot.mjs).
    ("config", |h, _| Ret::Str(h.config.clone())),
    ("poll_events", |h, _| {
        if h.events.is_empty() {
            return Ret::Str(String::new());
        }
        let list = format!("[{}]", h.events.join(","));
        h.events.clear();
        Ret::Str(list)
    }),
    ("gamepads", |h, _| Ret::Str(h.gamepads.clone())),
    ("set_cursor", |h, a| {
        let cursor = if text(a, 0) == "image" {
            let (width, height) = (uint(a, 2), uint(a, 3));
            let rgba = bytes(a, 1);
            if width == 0 || height == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
                return Ret::Error(format!("a {width}x{height} cursor needs more than {} bytes", rgba.len()));
            }
            Cursor::Image { rgba: rgba.to_vec(), width, height, hot_x: uint(a, 4), hot_y: uint(a, 5) }
        } else {
            Cursor::Named(text(a, 0).to_owned())
        };
        h.requests.push(Request::Cursor(cursor));
        Ret::Undefined
    }),
    ("pointer_lock", |h, a| {
        h.requests.push(Request::PointerLock(flag(a, 0)));
        Ret::Undefined
    }),
    ("fullscreen", |h, a| {
        h.requests.push(Request::Fullscreen(flag(a, 0)));
        Ret::Undefined
    }),
    ("quit", |h, _| {
        h.quit.get_or_insert_with(|| "the page closed its window".into());
        Ret::Undefined
    }),
    ("reload", |h, _| {
        warn!("host: the page asked to reload, which is not done here; quitting instead");
        h.quit.get_or_insert_with(|| "the page asked to reload".into());
        Ret::Undefined
    }),
    ("progress", |h, a| {
        say!("host: the client is at stage {:?}", text(a, 0));
        h.stage = text(a, 0).to_owned();
        Ret::Undefined
    }),
    ("fatal", |h, a| {
        fail!("host: the client could not start: {}", text(a, 0));
        h.quit.get_or_insert_with(|| format!("the client could not start: {}", text(a, 0)));
        Ret::Undefined
    }),
    // Game data.
    ("data_read", |h, a| h.data.read(text(a, 0)).map_or(Ret::Null, Ret::Bytes)),
    ("data_exists", |h, a| Ret::Bool(h.data.exists(text(a, 0)))),
    // localStorage.
    ("storage_load", |h, _| Ret::Str(h.storage.load())),
    ("storage_set", |h, a| {
        let value = a.get(1).and_then(Arg::str);
        h.storage.set(text(a, 0), value, ffi::now_ms());
        Ret::Undefined
    }),
    // The AddOns folder and saves.
    ("fs_read", |h, a| h.files.read(text(a, 0)).map_or(Ret::Null, Ret::Bytes)),
    ("fs_list", |h, a| Ret::Str(h.files.list(text(a, 0)))),
    ("fs_kind", |h, a| Ret::Str(h.files.kind(text(a, 0)).to_owned())),
    ("addon_remove", |h, a| unit(h.files.remove_addon(text(a, 0)))),
    ("save_export", |h, a| string(h.files.export_save(text(a, 0)))),
    ("save_pick", |h, _| h.files.pick_save().map_or(Ret::Null, Ret::Str)),
    // WebGPU (gpu.js).
    ("gpu_request_adapter", |h, a| match json(a, 0) {
        Ok(options) => Ret::Str(h.gpu.request_adapter(&options).unwrap_or_default()),
        Err(e) => Ret::Error(e),
    }),
    ("gpu_request_device", |h, a| string(json(a, 1).and_then(|d| h.gpu.request_device(uint(a, 0), &d)))),
    ("gpu_preferred_format", |h, _| Ret::Str(h.gpu.preferred_format())),
    ("gpu_create", |h, a| id(json(a, 2).and_then(|d| h.gpu.create(uint(a, 0), text(a, 1), &d)))),
    ("gpu_destroy", |h, a| {
        h.gpu.destroy(uint(a, 0));
        Ret::Undefined
    }),
    ("gpu_drop", |h, a| {
        h.gpu.drop_objects(ids(bytes(a, 0)));
        Ret::Undefined
    }),
    ("gpu_encode", |h, a| id(json(a, 1).and_then(|info| h.gpu.encode(uint(a, 0), &info, &f64s(bytes(a, 2)))))),
    ("gpu_submit", |h, a| unit(h.gpu.submit(uint(a, 0), ids(bytes(a, 1))))),
    ("gpu_write_buffer", |h, a| unit(h.gpu.write_buffer(uint(a, 0), uint(a, 1), num(a, 2) as u64, bytes(a, 3)))),
    ("gpu_write_texture", |h, a| unit(json(a, 1).and_then(|d| h.gpu.write_texture(uint(a, 0), &d, bytes(a, 2))))),
    ("gpu_map_async", |h, a| id(h.gpu.map_async(uint(a, 0), uint(a, 1), num(a, 2) as u64, num(a, 3) as u64))),
    ("gpu_mapped_read", |h, a| match h.gpu.mapped_read(uint(a, 0), num(a, 1) as u64, num(a, 2) as u64) {
        Ok(data) => Ret::Bytes(data),
        Err(e) => Ret::Error(e),
    }),
    ("gpu_mapped_write", |h, a| unit(h.gpu.mapped_write(uint(a, 0), num(a, 1) as u64, bytes(a, 2)))),
    ("gpu_unmap", |h, a| unit(h.gpu.unmap(uint(a, 0)))),
    ("gpu_on_submitted_work_done", |h, a| id(h.gpu.on_submitted_work_done(uint(a, 0)))),
    ("gpu_poll", |h, _| Ret::Str(h.gpu.poll())),
    ("gpu_push_error_scope", |h, a| unit(h.gpu.push_error_scope(uint(a, 0), text(a, 1)))),
    ("gpu_pop_error_scope", |h, a| string(h.gpu.pop_error_scope(uint(a, 0)))),
    ("gpu_configure", |h, a| unit(json(a, 1).and_then(|c| h.gpu.configure(uint(a, 0), &c)))),
    ("gpu_unconfigure", |h, _| {
        h.gpu.unconfigure();
        Ret::Undefined
    }),
    ("gpu_current_texture", |h, a| id(h.gpu.current_texture(uint(a, 0).max(1), uint(a, 1).max(1)))),
    ("gpu_present", |h, _| {
        h.gpu.present();
        Ret::Undefined
    }),
    // Web Audio (audio.js).
    ("audio_open", |h, a| Ret::Str(h.audio.open(num(a, 0)))),
    ("audio_time", |h, a| Ret::Number(h.audio.time(uint(a, 0)))),
    ("audio_suspend", |h, a| {
        h.audio.suspend(uint(a, 0), flag(a, 1));
        Ret::Undefined
    }),
    ("audio_close", |h, a| {
        h.audio.close(uint(a, 0));
        Ret::Undefined
    }),
    ("audio_set_channels", |h, a| {
        h.audio.set_channels(uint(a, 0), uint(a, 1));
        Ret::Undefined
    }),
    ("audio_buffer_create", |h, a| {
        id(h.audio.buffer_create(bytes(a, 0), uint(a, 1) as usize, uint(a, 2) as usize, num(a, 3)))
    }),
    ("audio_buffer_free", |h, a| {
        h.audio.buffer_free(uint(a, 0));
        Ret::Undefined
    }),
    ("audio_play", |h, a| {
        let duration = a.get(4).and_then(Arg::num).unwrap_or(-1.0);
        let token = h.audio.play(uint(a, 0), uint(a, 1), num(a, 2), num(a, 3), duration, num(a, 5));
        Ret::Number(f64::from(token))
    }),
    ("audio_stop", |h, a| {
        h.audio.stop(uint(a, 0), uint(a, 1), num(a, 2));
        Ret::Undefined
    }),
    ("audio_decode", |h, a| Ret::Number(f64::from(h.audio.decode(bytes(a, 0), num(a, 1))))),
    ("audio_decode_take", |h, a| match h.audio.decode_take(uint(a, 0)) {
        Ok(data) => Ret::Bytes(data),
        Err(e) => Ret::Error(e),
    }),
    ("audio_poll", |h, _| Ret::Str(h.audio.poll())),
];

/// Operation `op` (0 resolves a name to its number, or 0 for none).
pub fn call(op: u32, args: &[Arg]) -> Ret {
    if op == 0 {
        let name = text(args, 0);
        let index = OPS.iter().position(|(n, _)| *n == name);
        if index.is_none() {
            warn!("host: the page asked for the operation {name:?}, which there is not");
        }
        return Ret::Number(index.map_or(0.0, |i| (i + 1) as f64));
    }
    let Some((_, f)) = OPS.get(op as usize - 1) else {
        return Ret::Error(format!("there is no operation {op}"));
    };
    HOST.with(|h| match h.try_borrow_mut() {
        Ok(mut h) => match h.as_mut() {
            Some(host) => f(host, args),
            None => Ret::Error("the host is not running".into()),
        },
        Err(_) => Ret::Error(format!("{} was called while the host was busy", name(op))),
    })
}

pub fn name(op: u32) -> &'static str {
    match op {
        0 => "resolving an operation",
        _ => OPS.get(op as usize - 1).map_or("an unknown operation", |(n, _)| n),
    }
}
