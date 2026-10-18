//! The mixer: every context's voices, rendered at the context's own rate in 128-frame quanta (Web
//! Audio's render quantum) and resampled to the device's rate. The device's thread pulls sound
//! from it (`render`); the JS thread schedules voices and reads the clocks.
//!
//! A context's clock is the frames it has rendered, so it runs at the device's pace and stops
//! while the context is suspended, as an AudioContext's `currentTime` does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const QUANTUM: usize = 128;

/// An AudioBuffer's samples: planar, `frames` to a channel.
pub struct Clip {
    pub planes: Vec<Vec<f32>>,
    pub frames: usize,
    pub rate: f64,
}

impl Clip {
    /// The frame at a fractional position, mixed down or up to stereo by Web Audio's speaker
    /// rules.
    fn stereo_at(&self, pos: f64) -> [f32; 2] {
        let i = pos as usize;
        let t = (pos - i as f64) as f32;
        let at = |c: usize| {
            let plane = &self.planes[c];
            let a = plane.get(i).copied().unwrap_or(0.0);
            if t == 0.0 {
                return a;
            }
            let b = plane.get(i + 1).copied().unwrap_or(a);
            a + (b - a) * t
        };
        match self.planes.len() {
            0 => [0.0, 0.0],
            1 => {
                let m = at(0);
                [m, m]
            }
            // Quad: L R SL SR.
            4 => [0.5 * (at(0) + at(2)), 0.5 * (at(1) + at(3))],
            // 5.1: L R C LFE SL SR.
            6 => {
                let c = at(2);
                [at(0) + 0.707_106_8 * (c + at(4)), at(1) + 0.707_106_8 * (c + at(5))]
            }
            _ => [at(0), at(1)],
        }
    }
}

struct Voice {
    token: u32,
    /// None for a source with no buffer, which plays silence.
    clip: Option<Arc<Clip>>,
    /// The context frame it starts at, and the one it stops at once `stop` was called.
    start: f64,
    stop: Option<f64>,
    /// Where it is, and where it ends: clip frames, or for a source with no buffer, context
    /// frames since it started.
    pos: f64,
    end: f64,
    /// How far `pos` moves for each context frame.
    step: f64,
    gain: f32,
    started: bool,
}

impl Voice {
    /// Mixes the voice into a quantum that starts at context frame `t0`; false once it has
    /// finished.
    fn render(&mut self, out: &mut [[f32; 2]; QUANTUM], t0: u64) -> bool {
        for (i, frame) in out.iter_mut().enumerate() {
            let t = (t0 + i as u64) as f64;
            if self.stop.is_some_and(|stop| t >= stop) {
                return false;
            }
            if !self.started {
                // A start time already past starts at once, from the offset.
                if t < self.start {
                    continue;
                }
                self.started = true;
            }
            if self.pos >= self.end {
                return false;
            }
            if let Some(clip) = &self.clip {
                if self.gain != 0.0 {
                    let [l, r] = clip.stereo_at(self.pos);
                    frame[0] += l * self.gain;
                    frame[1] += r * self.gain;
                }
            }
            self.pos += self.step;
        }
        !(self.started && self.pos >= self.end)
    }
}

pub struct Context {
    rate: f64,
    /// Frames rendered: the context's clock.
    frames: u64,
    suspended: bool,
    /// The destination's channel count.
    channels: u32,
    voices: Vec<Voice>,
    /// The quantum being read out, and how much of it has been.
    quantum: [[f32; 2]; QUANTUM],
    read: usize,
    /// The resampler's last four context frames, and where it is between the middle two.
    history: [[f32; 2]; 4],
    frac: f64,
}

impl Context {
    fn render_quantum(&mut self, ended: &mut Vec<u32>) {
        let mut out = [[0.0f32; 2]; QUANTUM];
        let t0 = self.frames;
        self.voices.retain_mut(|v| {
            let alive = v.render(&mut out, t0);
            if !alive {
                ended.push(v.token);
            }
            alive
        });
        if self.channels == 1 {
            for f in &mut out {
                let m = 0.5 * (f[0] + f[1]);
                *f = [m, m];
            }
        }
        self.frames += QUANTUM as u64;
        self.quantum = out;
        self.read = 0;
    }

    fn next_frame(&mut self, ended: &mut Vec<u32>) -> [f32; 2] {
        if self.read >= QUANTUM {
            self.render_quantum(ended);
        }
        let f = self.quantum[self.read];
        self.read += 1;
        f
    }

    /// The next device frame: the context's frames, resampled (cubic Hermite) by `step` context
    /// frames per device frame.
    fn resampled(&mut self, step: f64, ended: &mut Vec<u32>) -> [f32; 2] {
        while self.frac >= 1.0 {
            self.history.rotate_left(1);
            self.history[3] = self.next_frame(ended);
            self.frac -= 1.0;
        }
        let h = &self.history;
        let t = self.frac as f32;
        let mut out = [0.0; 2];
        for (c, o) in out.iter_mut().enumerate() {
            *o = hermite(h[0][c], h[1][c], h[2][c], h[3][c], t);
        }
        self.frac += step;
        out
    }
}

/// Cubic Hermite interpolation between `y1` and `y2`.
pub fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

pub struct Mixer {
    device_rate: f64,
    contexts: HashMap<u32, Context>,
    /// Tokens of the voices that finished, for `audio_poll`.
    ended: Vec<u32>,
}

impl Mixer {
    pub fn new(device_rate: f64) -> Mixer {
        Mixer { device_rate, contexts: HashMap::new(), ended: Vec::new() }
    }

    pub fn set_device_rate(&mut self, rate: f64) {
        self.device_rate = rate;
    }

    pub fn device_rate(&self) -> f64 {
        self.device_rate
    }

    /// Mixes the next device frames into `out` (interleaved, `channels` to a frame): the
    /// running contexts' stereo, on the first two channels.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        out.fill(0.0);
        let channels = channels.max(1);
        let Mixer { device_rate, contexts, ended } = self;
        for context in contexts.values_mut().filter(|c| !c.suspended) {
            let step = context.rate / *device_rate;
            for frame in out.chunks_exact_mut(channels) {
                let [l, r] = context.resampled(step, ended);
                if channels == 1 {
                    frame[0] += 0.5 * (l + r);
                } else {
                    frame[0] += l;
                    frame[1] += r;
                }
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    pub fn open(&mut self, handle: u32, rate: f64) {
        self.contexts.insert(
            handle,
            Context {
                rate,
                frames: 0,
                suspended: false,
                channels: 2,
                voices: Vec::new(),
                quantum: [[0.0; 2]; QUANTUM],
                read: QUANTUM,
                history: [[0.0; 2]; 4],
                frac: 0.0,
            },
        );
    }

    /// Ends the context; its voices go without being reported as ended.
    pub fn close(&mut self, handle: u32) {
        self.contexts.remove(&handle);
    }

    /// The context's clock in seconds.
    pub fn time(&self, handle: u32) -> f64 {
        self.contexts.get(&handle).map_or(0.0, |c| c.frames as f64 / c.rate)
    }

    pub fn suspend(&mut self, handle: u32, suspended: bool) {
        if let Some(c) = self.contexts.get_mut(&handle) {
            c.suspended = suspended;
        }
    }

    pub fn set_channels(&mut self, handle: u32, channels: u32) {
        if let Some(c) = self.contexts.get_mut(&handle) {
            c.channels = channels;
        }
    }

    /// Starts a voice: `clip` (or silence) from `offset` seconds into it, at context time
    /// `when`, for `duration` seconds or to its end.
    pub fn play(&mut self, handle: u32, token: u32, clip: Option<Arc<Clip>>, when: f64, offset: f64, duration: Option<f64>, gain: f32) -> bool {
        let Some(context) = self.contexts.get_mut(&handle) else { return false };
        let (pos, end, step) = match &clip {
            Some(clip) => {
                let pos = offset * clip.rate;
                let end = duration.map_or(clip.frames as f64, |d| (pos + d * clip.rate).min(clip.frames as f64));
                (pos, end, clip.rate / context.rate)
            }
            None => (0.0, duration.map_or(f64::INFINITY, |d| d * context.rate), 1.0),
        };
        context.voices.push(Voice { token, clip, start: when * context.rate, stop: None, pos, end, step, gain, started: false });
        true
    }

    /// Stops a voice at context time `when` (at once when that has passed).
    pub fn stop(&mut self, handle: u32, token: u32, when: f64) {
        let Some(context) = self.contexts.get_mut(&handle) else { return };
        let at = when * context.rate;
        if let Some(v) = context.voices.iter_mut().find(|v| v.token == token) {
            v.stop = Some(v.stop.map_or(at, |s| s.min(at)));
        }
    }

    pub fn take_ended(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.ended)
    }

    /// Whether any voice is waiting to start, playing, or waiting to be reported.
    pub fn busy(&self) -> bool {
        !self.ended.is_empty() || self.contexts.values().any(|c| !c.voices.is_empty())
    }
}

/// Renders from a mixer shared with the JS thread.
pub fn render(mixer: &Mutex<Mixer>, out: &mut [f32], channels: usize) {
    mixer.lock().unwrap_or_else(|e| e.into_inner()).render(out, channels);
}
