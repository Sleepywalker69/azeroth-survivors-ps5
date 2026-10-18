//! The host on the PlayStation 5: the display the title owns, the pad (and a USB or Bluetooth
//! mouse and keyboard) as the page's input, the console's sound, and the loop that winit runs on
//! a desktop. `ps5/main.c` calls `aethus_ps5_main` on a thread with a desktop-sized stack.
//!
//! A title sees its own folder as /app0:
//!
//! | Path | Holds |
//! | --- | --- |
//! | /app0/js | the host's scripts (env.js, gpu.js, audio.js, boot.mjs) |
//! | /app0/web | the web build (wenilla.js and its snippets) |
//! | $WOW_DATA (/app0/Data) | the MPQs; several folders may be listed, `:` between them |
//! | $BENILLA_HOME (/app0/userdata) | settings, saves, AddOns, the log |

mod input;
pub(crate) mod perf;

use std::ffi::c_int;
use std::path::PathBuf;
use std::time::Duration;

use raw_window_handle::{DrmDisplayHandle, DrmWindowHandle, RawDisplayHandle, RawWindowHandle};
use serde_json::{json, Map};

use crate::audio::Audio;
use crate::data::Data;
use crate::ffi;
use crate::files::Files;
use crate::gpu::cursor::Overlay;
use crate::gpu::Gpu;
use crate::log::{fail, say, warn};
use crate::ops::{self, Host, Request};
use crate::storage::Storage;

/// The page is laid out 1080 CSS pixels tall at any display size: a 4K screen is a device pixel
/// ratio of 2.
const LOGICAL_HEIGHT: f64 = 1080.0;
/// The display's refresh, ms.
const INTERVAL: f64 = 1000.0 / 60.0;

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(|| PathBuf::from(default), PathBuf::from)
}

/// `$PS5_DISPLAY_MODE` (WIDTHxHEIGHT), the size the patched wgpu-hal gives the display surface.
fn display_size() -> (u32, u32) {
    std::env::var("PS5_DISPLAY_MODE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .filter(|&(w, h): &(u32, u32)| w > 0 && h > 0)
        .unwrap_or((3840, 2160))
}

#[no_mangle]
pub extern "C" fn aethus_ps5_main() -> c_int {
    match run() {
        Ok(code) => code,
        Err(e) => {
            fail!("host: {e}");
            1
        }
    }
}

fn run() -> Result<c_int, String> {
    let home = env_path("BENILLA_HOME", "/app0/userdata");
    if let Err(e) = std::fs::create_dir_all(&home) {
        warn!("host: {}: {e}", home.display());
    }
    crate::log::to_file(&home.join("aethus-host.log"));
    let (mut updater, close) = crate::update::runtime_ps5::Service::open(
        &env_path("AETHUS_INSTALL_ROOT", "/app0"))?;
    if close { return Ok(0); }
    let js = env_path("AETHUS_JS", "/app0/js");
    let web = env_path("AETHUS_WEB", "/app0/web");
    let data: Vec<PathBuf> = std::env::var("WOW_DATA")
        .unwrap_or_else(|_| "/app0/Data".into())
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();

    let (width, height) = display_size();
    let dpr = f64::from(height) / LOGICAL_HEIGHT;
    let viewport = (f64::from(width) / dpr, LOGICAL_HEIGHT);
    say!("host: the display is {width}x{height}, the page {}x{} at {dpr}", viewport.0, viewport.1);

    let mut gpu = Gpu::new(wgpu::Backends::VULKAN, wgpu::PresentMode::Fifo, |instance| {
        // The console's display: the patched wgpu-hal answers a DRM handle with the VideoOut
        // plane, which lives as long as the title.
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: RawDisplayHandle::Drm(DrmDisplayHandle::new(-1)),
            raw_window_handle: RawWindowHandle::Drm(DrmWindowHandle::new(0)),
        };
        // SAFETY: the handles name the display itself, which outlives the surface.
        match unsafe { instance.create_surface_unsafe(target) } {
            Ok(surface) => Some(surface),
            Err(e) => {
                warn!("gpu: the display has no surface: {e}");
                None
            }
        }
    });
    gpu.cursor = Some(Overlay::new(dpr as f32));

    let config = json!({
        "viewport": [viewport.0, viewport.1],
        "dpr": dpr,
        "userAgent": "Mozilla/5.0 (PlayStation 5) AethusHost/1.0",
        "platform": "PlayStation 5",
        "language": "en-US",
        "rogue": Map::new(),
        "env": Map::new(),
        "addonsDir": home.join("AddOns").display().to_string(),
    });
    ops::install(Host {
        config: config.to_string(),
        data: Data::mount(&data),
        storage: Storage::open(home.join("localStorage.json")),
        files: Files::new(&home),
        gpu,
        audio: Audio::new(),
        events: Vec::new(),
        gamepads: String::new(),
        requests: Vec::new(),
        quit: None,
        stage: String::new(),
    });
    let mut input = input::Input::open(viewport);
    let (x, y) = input.cursor();
    ops::with(|h| {
        h.event(json!({ "t": "focus", "focused": true }));
        h.event(json!({ "t": "over", "x": x, "y": y, "m": 0 }));
        h.gpu.cursor.as_mut().map(|c| c.move_to(x, y));
    });
    say!("host: the page is {} with the scripts in {}", web.display(), js.display());
    ffi::js_init(&js, &web)?;

    let mut locked = false;
    let mut next_frame = ffi::now_ms();
    let mut events = Vec::new();
    loop {
        let now = ffi::now_ms();
        let phase = perf::start();
        input.poll(now, locked, &mut events);
        perf::finish("phase.input", phase);
        let (x, y) = input.cursor();
        ops::with(|h| {
            for event in events.drain(..) {
                h.event(event);
            }
            if let Some(cursor) = h.gpu.cursor.as_mut() {
                cursor.move_to(x, y);
                cursor.set_locked(locked || input.camera_mode());
            }
        });

        let phase = perf::start();
        let mut wait = ffi::js_tick(now);
        perf::finish("phase.tick", phase);
        let drew_frame = now >= next_frame;
        if drew_frame {
            let phase = perf::start();
            ffi::js_frame(now);
            perf::finish("phase.frame", phase);
            perf::frame();
            // FIFO already paces presentation. If work exceeded the frame budget, poll
            // input and start the next frame immediately instead of adding another delay.
            next_frame = (next_frame + INTERVAL).max(ffi::now_ms());
            let phase = perf::start();
            wait = ffi::js_tick(ffi::now_ms());
            perf::finish("phase.tick", phase);
        }
        let phase = perf::start();
        ffi::js_collect();
        perf::finish("phase.collect", phase);

        let now = ffi::now_ms();
        let phase = perf::start();
        let Some((requests, quit, busy)) = ops::with(|h| {
            h.gpu.maintain();
            h.storage.flush(now, false);
            (std::mem::take(&mut h.requests), h.quit.clone(), h.audio.busy() || h.gpu.busy())
        }) else {
            return Err("the host's state is gone".into());
        };
        perf::finish("phase.maintain", phase);
        perf::report();
        let game_ready = ops::with(|h| h.stage == "ready").unwrap_or(false);
        if updater.poll(game_ready, drew_frame, input.take_update_action()) {
            // No more JS callbacks after this point. Preserve the browser storage before
            // replacing any game files; the old executable stays mapped until title exit.
            ops::with(|h| h.storage.flush(ffi::now_ms(), true));
            updater.install_and_close()?;
            return Ok(0);
        }
        for request in requests {
            match request {
                Request::Cursor(cursor) => {
                    ops::with(|h| h.gpu.cursor.as_mut().map(|c| c.set(cursor)));
                }
                Request::PointerLock(on) => {
                    locked = on;
                    ops::with(|h| h.gpu.cursor.as_mut().map(|c| c.set_locked(on || input.camera_mode())));
                }
                // The title is always the whole screen.
                Request::Fullscreen(_) => {}
            }
        }
        if let Some(reason) = quit {
            say!("host: quitting: {reason}");
            ops::with(|h| h.storage.flush(ffi::now_ms(), true));
            return Ok(if reason.starts_with("the client could not start") { 1 } else { 0 });
        }

        // Until the next frame or timer, polling the pad at least every 8 ms; at once when
        // the audio or the GPU has completions to deliver.
        let mut next = next_frame.min(now + 8.0);
        if wait >= 0.0 {
            next = next.min(now + wait);
        }
        if busy {
            next = next.min(now + 2.0);
        }
        let delay = next - ffi::now_ms();
        if delay > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(delay / 1000.0));
        }
    }
}
