//! The PlayStation 5 half of [`super`]: the same surface as `cpal.rs` over the title's own audio
//! output (`ps5/src/platform.c`'s `audio_start`, libSceAudioOut underneath): one 48 kHz stereo
//! port of 16-bit samples, filled 256 frames at a time by a thread the platform layer owns and
//! paced by the port's blocking output call.
//!
//! The console has one output and it never changes under the title, so [`Listeners`] watches
//! nothing and no notice is ever raised; there is no realtime policy to ask for either, since the
//! platform's thread already runs at the port's priority.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};

/// kira mixes stereo, and the port takes stereo.
pub(super) const CHANNELS: u32 = 2;

/// The port's rate and the frames each fill carries (`platform.c`'s `AUDIO_GRAIN`).
const SAMPLE_RATE: u32 = 48_000;
const GRAIN_FRAMES: u32 = 256;

/// Nanoseconds on a monotonic clock, as `cpal.rs` keeps it.
pub(super) fn now_ns() -> u64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

unsafe extern "C" {
    fn audio_start(
        fill: unsafe extern "C" fn(frames: *mut i16, count: i32, user: *mut c_void),
        user: *mut c_void,
    ) -> bool;
    fn audio_stop();
}

// ---------------------------------------------------------------------------------------------
// Device

/// The console's one output.
#[derive(Clone, Debug)]
pub(super) struct Device {
    pub name: String,
    pub sample_rate: u32,
    pub buffer_range: (u32, u32),
    pub latency_frames: u32,
    pub safety_frames: u32,
}

pub(super) fn default_output() -> Result<Device> {
    now_ns();
    Ok(Device {
        name: "PS5 audio out (main port)".into(),
        sample_rate: SAMPLE_RATE,
        buffer_range: (GRAIN_FRAMES, GRAIN_FRAMES),
        // The port holds the grain being played while the next is filled.
        latency_frames: GRAIN_FRAMES,
        safety_frames: 0,
    })
}

// ---------------------------------------------------------------------------------------------
// Stream

/// What the IO callback sees each cycle, as on every other target.
pub(super) struct Cycle<'a> {
    pub buffer: &'a mut [f32],
    pub frames: usize,
    pub output_time_ns: u64,
}

/// The one output stream; the platform layer has a single port, so only one is ever open.
pub(super) struct Stream {
    // Boxed so the pointer `audio_start` was given stays put; freed after `audio_stop`.
    callback: *mut c_void,
    drop_callback: unsafe fn(*mut c_void),
}

// SAFETY: the pointer is the boxed callback, touched only by the platform's audio thread until
// `Drop` has stopped that thread.
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

/// The one stream open, so a second open fails instead of stealing the port.
static OPEN: AtomicBool = AtomicBool::new(false);

struct Callback<F> {
    on_cycle: F,
    scratch: Vec<f32>,
}

unsafe extern "C" fn fill<F: FnMut(Cycle<'_>) + Send + 'static>(
    frames: *mut i16,
    count: i32,
    user: *mut c_void,
) {
    // SAFETY: `user` is the `Callback<F>` `Stream::open` boxed, alive until `audio_stop` returns;
    // `frames` holds `count` stereo frames.
    let ctx = unsafe { &mut *(user as *mut Callback<F>) };
    let count = count.max(0) as usize;
    let out = unsafe { std::slice::from_raw_parts_mut(frames, count * CHANNELS as usize) };
    let per_cycle = ctx.scratch.len() / CHANNELS as usize;
    let due = now_ns() + u64::from(GRAIN_FRAMES) * 1_000_000_000 / u64::from(SAMPLE_RATE);
    let mut done = 0;
    while done < count {
        let take = (count - done).min(per_cycle);
        let stereo = &mut ctx.scratch[..take * CHANNELS as usize];
        (ctx.on_cycle)(Cycle {
            buffer: stereo,
            frames: take,
            output_time_ns: due + (done as u64 * 1_000_000_000) / u64::from(SAMPLE_RATE),
        });
        for (sample, slot) in stereo
            .iter()
            .zip(out[done * CHANNELS as usize..].iter_mut())
        {
            *slot = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        }
        done += take;
    }
}

unsafe fn drop_boxed<F>(ptr: *mut c_void) {
    // SAFETY: `ptr` came from `Box::into_raw` of a `Callback<F>` in `Stream::open`.
    drop(unsafe { Box::from_raw(ptr as *mut Callback<F>) });
}

impl Stream {
    pub(super) fn buffer_frames(&self) -> u32 {
        GRAIN_FRAMES
    }

    pub(super) fn open<F>(
        _device: &Device,
        _buffer_frames: u32,
        _notices: &Arc<Notices>,
        on_cycle: F,
    ) -> Result<Self>
    where
        F: FnMut(Cycle<'_>) + Send + 'static,
    {
        if OPEN.swap(true, Ordering::AcqRel) {
            bail!("the PS5 audio port is already open");
        }
        let callback = Box::into_raw(Box::new(Callback {
            on_cycle,
            scratch: vec![0.0; GRAIN_FRAMES as usize * 4 * CHANNELS as usize],
        })) as *mut c_void;
        // SAFETY: `fill::<F>` reads `callback` as the `Callback<F>` just boxed.
        if !unsafe { audio_start(fill::<F>, callback) } {
            // SAFETY: the platform never saw the pointer.
            unsafe { drop_boxed::<F>(callback) };
            OPEN.store(false, Ordering::Release);
            bail!("audio_start: the console gave the title no audio output");
        }
        Ok(Self {
            callback,
            drop_callback: drop_boxed::<F>,
        })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: `audio_stop` joins the fill thread, after which nothing reads the callback.
        unsafe {
            audio_stop();
            (self.drop_callback)(self.callback);
        }
        OPEN.store(false, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------------------------
// Listeners

/// Shaped as on every other target; nothing on the console raises one.
#[derive(Default)]
pub(super) struct Notices {
    pub default_changed: AtomicBool,
    pub device_died: AtomicBool,
    pub rate_changed: AtomicBool,
    pub overloads: AtomicU64,
    pub last_overload_ns: AtomicU64,
}

pub(super) struct Listeners;

impl Listeners {
    pub(super) fn arm(_device: &Device, _notices: Arc<Notices>) -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------------------------
// Realtime scheduling

pub(super) struct Workgroup;

impl Workgroup {
    pub(super) fn of_device(_device: &Device) -> Option<Self> {
        None
    }
}

pub(super) struct Joined;

impl Joined {
    pub(super) fn join(_group: Workgroup) -> Result<Self> {
        bail!("no audio workgroups on this platform")
    }
}

pub(super) struct Realtime;

pub(super) fn set_realtime(_period_ns: u64) -> Result<Realtime> {
    bail!("no realtime thread policy on the PS5 title")
}
