//! Opt-in phase and host-operation timings, emitted at most once every five seconds.
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Default)]
struct Sample { count: u64, total: f64, max: f64 }
#[derive(Default)]
struct Window { since: f64, frames: u64, samples: HashMap<&'static str, Sample> }
thread_local! { static WINDOW: RefCell<Window> = RefCell::new(Window::default()); }

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("AETHUS_PROFILE").is_ok_and(|v| v == "1"))
}

// Keep a cheap cadence counter available when detailed bridge/operation profiling is off.
fn show_fps() -> bool {
    static SHOW: OnceLock<bool> = OnceLock::new();
    *SHOW.get_or_init(|| enabled() || std::env::var("AETHUS_FPS").is_ok_and(|v| v == "1"))
}

pub fn start() -> f64 { if enabled() { crate::ffi::now_ms() } else { 0.0 } }

pub fn finish(name: &'static str, start: f64) {
    if !enabled() { return; }
    let elapsed = crate::ffi::now_ms() - start;
    WINDOW.with(|window| {
        let mut window = window.borrow_mut();
        let sample = window.samples.entry(name).or_default();
        sample.count += 1;
        sample.total += elapsed;
        sample.max = sample.max.max(elapsed);
    });
}

pub fn frame() {
    if show_fps() { WINDOW.with(|window| window.borrow_mut().frames += 1); }
}

pub fn report() {
    if !show_fps() { return; }
    let now = crate::ffi::now_ms();
    WINDOW.with(|window| {
        let mut window = window.borrow_mut();
        let elapsed = now - window.since;
        if elapsed < 5000.0 { return; }
        crate::log::say!("perf: {:.2} frames/s over {:.1}s ({})", window.frames as f64 * 1000.0 / elapsed, elapsed / 1000.0,
            if enabled() { "phase and nested operation totals" } else { "frame cadence only; detailed profiling off" });
        let mut samples: Vec<_> = window.samples.drain().collect();
        samples.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
        for (name, sample) in samples.into_iter().take(14) {
            crate::log::say!("perf: {name}: {} calls, {:.1}ms total, {:.1}ms max", sample.count, sample.total, sample.max);
        }
        if enabled() { crate::ffi::js_profile_report(); }
        window.frames = 0;
        window.since = now;
    });
}
