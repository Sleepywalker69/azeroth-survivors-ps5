//! The boundary with the JS side (`../c/host.h`): the values that cross it, the JS side's entry
//! points, and the three functions it calls back.

use std::borrow::Cow;
use std::ffi::{c_char, c_int, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

const UNDEFINED: u32 = 0;
const NULL: u32 = 1;
const BOOL: u32 = 2;
const NUMBER: u32 = 3;
const STRING: u32 = 4;
const BYTES: u32 = 5;
const ERROR: u32 = 7;

/// `AethusValue`.
#[repr(C)]
pub struct RawValue {
    kind: u32,
    reserved: u32,
    number: f64,
    ptr: *mut u8,
    len: usize,
}

extern "C" {
    fn aethus_js_init(js_root: *const c_char, web_root: *const c_char) -> c_int;
    fn aethus_js_tick(now_ms: f64) -> f64;
    fn aethus_js_frame(now_ms: f64);
    fn aethus_js_collect();
    #[cfg(target_env = "ps5")]
    fn aethus_js_profile_report();
    #[cfg(target_env = "ps5")]
    fn aethus_ps5_memory_report();
    fn aethus_now_ms() -> f64;
}

/// Creates the JS context and starts the page: env.js, gpu.js, audio.js, then the boot module,
/// which loads the glue from `web_root`.
pub fn js_init(js_root: &Path, web_root: &Path) -> Result<(), String> {
    let path = |p: &Path| CString::new(p.to_string_lossy().replace('\\', "/")).map_err(|e| e.to_string());
    let (js, web) = (path(js_root)?, path(web_root)?);
    // SAFETY: two NUL-terminated paths, read during the call.
    match unsafe { aethus_js_init(js.as_ptr(), web.as_ptr()) } {
        0 => Ok(()),
        status => Err(format!("the JS host did not start ({status})")),
    }
}

/// Runs the tasks that are due; the milliseconds until the next timer, 0 when tasks are waiting,
/// or a negative number when nothing is pending.
pub fn js_tick(now_ms: f64) -> f64 {
    // SAFETY: called on the thread that initialised the JS side, outside any operation.
    unsafe { aethus_js_tick(now_ms) }
}

/// One display refresh: the animation frame callbacks, then the canvas is presented.
pub fn js_frame(now_ms: f64) {
    // SAFETY: as js_tick.
    unsafe { aethus_js_frame(now_ms) }
}

/// Releases the JS values the wasm let go of.
pub fn js_collect() {
    // SAFETY: as js_tick.
    unsafe { aethus_js_collect() }
}

#[cfg(target_env = "ps5")]
pub fn js_profile_report() {
    // SAFETY: called on the JS thread between tasks, with no bridge calls on the stack.
    unsafe { aethus_js_profile_report(); aethus_ps5_memory_report(); }
}

/// Milliseconds on the page's clock (`performance.now()`).
pub fn now_ms() -> f64 {
    // SAFETY: a clock read.
    unsafe { aethus_now_ms() }
}

/// An argument of an operation.
pub enum Arg<'a> {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Cow<'a, str>),
    Bytes(&'a [u8]),
    /// Some other object, which has no payload here.
    Object,
}

impl Arg<'_> {
    pub fn num(&self) -> Option<f64> {
        match self {
            Arg::Number(n) => Some(*n),
            Arg::Bool(b) => Some(f64::from(u8::from(*b))),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Arg::Bytes(b) => Some(b),
            _ => None,
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            Arg::Undefined | Arg::Null => false,
            Arg::Bool(b) => *b,
            Arg::Number(n) => *n != 0.0 && !n.is_nan(),
            Arg::Str(s) => !s.is_empty(),
            Arg::Bytes(_) | Arg::Object => true,
        }
    }
}

/// An operation's result.
pub enum Ret {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Bytes(Vec<u8>),
    /// Thrown in JS as an Error with this message.
    Error(String),
}

impl From<Result<Ret, String>> for Ret {
    fn from(r: Result<Ret, String>) -> Ret {
        r.unwrap_or_else(Ret::Error)
    }
}

/// Bytes handed to the JS side, which gives them back through `aethus_rust_free`.
fn leak(bytes: Vec<u8>) -> (*mut u8, usize) {
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    (Box::into_raw(boxed).cast::<u8>(), len)
}

/// Called by the JS side for every `rust(name, ...)`.
///
/// # Safety
/// `args` points to `argc` values whose strings and bytes stay valid for the call; `ret` is
/// writable.
#[no_mangle]
pub unsafe extern "C" fn aethus_rust_call(op: u32, args: *const RawValue, argc: u32, ret: *mut RawValue) {
    let raw = if argc == 0 || args.is_null() {
        &[][..]
    } else {
        // SAFETY: the caller's array of `argc` values.
        unsafe { std::slice::from_raw_parts(args, argc as usize) }
    };
    let args: Vec<Arg<'_>> = raw
        .iter()
        .map(|v| match v.kind {
            NULL => Arg::Null,
            BOOL => Arg::Bool(v.number != 0.0),
            NUMBER => Arg::Number(v.number),
            STRING | BYTES => {
                let bytes = if v.ptr.is_null() || v.len == 0 {
                    &[][..]
                } else {
                    // SAFETY: the caller's string or buffer, valid for the call.
                    unsafe { std::slice::from_raw_parts(v.ptr, v.len) }
                };
                if v.kind == STRING {
                    Arg::Str(String::from_utf8_lossy(bytes))
                } else {
                    Arg::Bytes(bytes)
                }
            }
            UNDEFINED => Arg::Undefined,
            _ => Arg::Object,
        })
        .collect();
    #[cfg(target_env = "ps5")]
    let operation_start = crate::ps5::perf::start();
    let result = catch_unwind(AssertUnwindSafe(|| crate::ops::call(op, &args))).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".to_string());
        Ret::Error(format!("{} failed: {message}", crate::ops::name(op)))
    });
    #[cfg(target_env = "ps5")]
    crate::ps5::perf::finish(crate::ops::name(op), operation_start);
    let (kind, number, (ptr, len)) = match result {
        Ret::Undefined => (UNDEFINED, 0.0, (std::ptr::null_mut(), 0)),
        Ret::Null => (NULL, 0.0, (std::ptr::null_mut(), 0)),
        Ret::Bool(b) => (BOOL, f64::from(u8::from(b)), (std::ptr::null_mut(), 0)),
        Ret::Number(n) => (NUMBER, n, (std::ptr::null_mut(), 0)),
        Ret::Str(s) => (STRING, 0.0, leak(s.into_bytes())),
        Ret::Bytes(b) => (BYTES, 0.0, leak(b)),
        Ret::Error(e) => (ERROR, 0.0, leak(e.into_bytes())),
    };
    // SAFETY: the caller's result slot.
    unsafe {
        *ret = RawValue { kind, reserved: 0, number, ptr, len };
    }
}

/// Gives back a string or bytes result.
///
/// # Safety
/// `ptr` and `len` are a result's, given back once.
#[no_mangle]
pub unsafe extern "C" fn aethus_rust_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        // SAFETY: the boxed slice `leak` made.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
    }
}

/// One line of the page's console, or of the wasm's standard output.
///
/// # Safety
/// `text` points to `len` bytes.
#[no_mangle]
pub unsafe extern "C" fn aethus_log(level: c_int, text: *const c_char, len: usize) {
    let bytes = if text.is_null() || len == 0 {
        &[][..]
    } else {
        // SAFETY: the caller's text.
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) }
    };
    crate::log::line(level, &String::from_utf8_lossy(bytes));
}
