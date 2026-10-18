//! The device the mixer plays through: the default output device (cpal) on a desktop, the
//! console's output on the PS5, and when there is neither, a sink that renders the sound in real
//! time and discards it, so the page's clocks still run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::mixer::{render, Mixer};
use crate::log::{say, warn};

pub struct Output {
    pub rate: u32,
    pub channels: u16,
    /// Seconds from the mixer to the speakers, roughly.
    pub latency: f64,
    _device: Device,
}

enum Device {
    #[cfg(windows)]
    Cpal(cpal::Stream),
    #[cfg(target_env = "ps5")]
    Console(console::Console),
    Silent(SilentSink),
}

/// Opens the output and starts pulling sound from `mixer`.
pub fn open(mixer: &Arc<Mutex<Mixer>>) -> Output {
    #[cfg(windows)]
    match open_cpal(mixer) {
        Ok(output) => return output,
        Err(e) => warn!("audio: no output device ({e}); sound is not played"),
    }
    #[cfg(target_env = "ps5")]
    match console::open(mixer) {
        Some(output) => return output,
        None => warn!("audio: the console gave no output; sound is not played"),
    }
    let rate = 48_000;
    mixer.lock().unwrap_or_else(|e| e.into_inner()).set_device_rate(f64::from(rate));
    Output { rate, channels: 2, latency: 0.0, _device: Device::Silent(SilentSink::start(mixer.clone(), rate)) }
}

#[cfg(windows)]
fn open_cpal(mixer: &Arc<Mutex<Mixer>>) -> Result<Output, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::SampleFormat;

    let device = cpal::default_host().default_output_device().ok_or("there is none")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let config = supported.config();
    mixer.lock().unwrap_or_else(|e| e.into_inner()).set_device_rate(f64::from(config.sample_rate));
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, mixer, |s| s),
        SampleFormat::I16 => build::<i16>(&device, &config, mixer, |s| (s * 32767.0) as i16),
        SampleFormat::I32 => build::<i32>(&device, &config, mixer, |s| (f64::from(s) * 2_147_483_647.0) as i32),
        SampleFormat::U16 => build::<u16>(&device, &config, mixer, |s| ((s * 32767.0) as i32 + 32768) as u16),
        format => return Err(format!("its sample format {format:?} is not one played here")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "the default device".into());
    say!("audio: playing on {name} at {} Hz, {} channels", config.sample_rate, config.channels);
    Ok(Output { rate: config.sample_rate, channels: config.channels, latency: 0.02, _device: Device::Cpal(stream) })
}

#[cfg(windows)]
fn build<T: cpal::SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: &Arc<Mutex<Mixer>>,
    convert: fn(f32) -> T,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;

    let mixer = mixer.clone();
    let channels = usize::from(config.channels);
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream::<T, _, _>(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                scratch.resize(data.len(), 0.0);
                render(&mixer, &mut scratch, channels);
                for (d, s) in data.iter_mut().zip(&scratch) {
                    *d = convert(*s);
                }
            },
            |e| crate::log::line(2, &format!("audio: {e}")),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Renders in real time and drops the sound.
struct SilentSink {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SilentSink {
    fn start(mixer: Arc<Mutex<Mixer>>, rate: u32) -> SilentSink {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("audio (silent)".into())
            .spawn(move || {
                let start = Instant::now();
                let mut done: u64 = 0;
                let mut scratch = vec![0.0f32; 2 * 1024];
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                    let due = (start.elapsed().as_secs_f64() * f64::from(rate)) as u64;
                    while done < due {
                        let n = (due - done).min(1024) as usize;
                        render(&mixer, &mut scratch[..n * 2], 2);
                        done += n as u64;
                    }
                }
            })
            .map_err(|e| warn!("audio: the silent sink did not start: {e}"))
            .ok();
        SilentSink { stop, thread }
    }
}

impl Drop for SilentSink {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The console's output (`../../../azeroth-survivors-re/ps5/src/platform.h`): 48 kHz stereo
/// 16-bit, filled 256 frames at a time from a thread of its own.
#[cfg(target_env = "ps5")]
mod console {
    use std::ffi::{c_int, c_void};
    use std::sync::{Arc, Mutex};

    use super::{render, Device, Mixer, Output};

    extern "C" {
        fn audio_start(fill: extern "C" fn(*mut i16, c_int, *mut c_void), user: *mut c_void) -> bool;
        fn audio_stop();
    }

    const RATE: u32 = 48_000;
    const GRAIN: usize = 256;

    /// Holds the mixer the fill thread reads; the thread is stopped before it goes.
    pub struct Console {
        mixer: *const Mutex<Mixer>,
    }

    impl Drop for Console {
        fn drop(&mut self) {
            // SAFETY: stops the thread that uses the pointer, then gives the pointer back.
            unsafe {
                audio_stop();
                drop(Arc::from_raw(self.mixer));
            }
        }
    }

    extern "C" fn fill(frames: *mut i16, count: c_int, user: *mut c_void) {
        let count = usize::try_from(count).unwrap_or(0);
        if frames.is_null() || count == 0 {
            return;
        }
        // SAFETY: `count` interleaved stereo frames to fill, and the mixer `open` handed over.
        let out = unsafe { std::slice::from_raw_parts_mut(frames, count * 2) };
        let mixer = unsafe { &*(user as *const Mutex<Mixer>) };
        let mut grain = [0.0f32; GRAIN * 2];
        for chunk in out.chunks_mut(GRAIN * 2) {
            let scratch = &mut grain[..chunk.len()];
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(mixer, scratch, 2))).is_err() {
                scratch.fill(0.0);
            }
            for (d, s) in chunk.iter_mut().zip(scratch.iter()) {
                *d = (s * 32767.0) as i16;
            }
        }
    }

    pub fn open(mixer: &Arc<Mutex<Mixer>>) -> Option<Output> {
        mixer.lock().unwrap_or_else(|e| e.into_inner()).set_device_rate(f64::from(RATE));
        let user = Arc::into_raw(mixer.clone());
        // SAFETY: the pointer stays valid until `Console` stops the thread.
        if !unsafe { audio_start(fill, user as *mut c_void) } {
            // SAFETY: the thread did not start; the pointer is given back.
            drop(unsafe { Arc::from_raw(user) });
            return None;
        }
        crate::log::say!("audio: playing on the console at {RATE} Hz");
        Some(Output { rate: RATE, channels: 2, latency: (GRAIN * 2) as f64 / f64::from(RATE), _device: Device::Console(Console { mixer: user }) })
    }
}
