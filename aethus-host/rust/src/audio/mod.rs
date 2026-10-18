//! Web Audio for the page (`../js/audio.js`): contexts and their clocks, buffers, the voices
//! started from them, and decodes. The mixing is in `mixer.rs`, the device in `output.rs`.

mod decode;
mod mixer;
mod output;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::json;

use crate::log::say;
use mixer::{Clip, Mixer, QUANTUM};

pub struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    /// Opened with the first context.
    output: Option<output::Output>,
    clips: HashMap<u32, Arc<Clip>>,
    /// Context handles, buffer ids and voice and decode tokens, from one count.
    next_id: u32,
    decoder: decode::Decoder,
}

impl Audio {
    pub fn new() -> Audio {
        Audio {
            mixer: Arc::new(Mutex::new(Mixer::new(48_000.0))),
            output: None,
            clips: HashMap::new(),
            next_id: 1,
            decoder: decode::Decoder::new(),
        }
    }

    fn mixer(&self) -> MutexGuard<'_, Mixer> {
        self.mixer.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A new context at `rate` (0: the device's), as JSON `{ handle, sampleRate,
    /// maxChannelCount, baseLatency, outputLatency }`.
    pub fn open(&mut self, rate: f64) -> String {
        let output = self.output.get_or_insert_with(|| output::open(&self.mixer));
        let rate = if rate > 0.0 { rate } else { f64::from(output.rate) };
        let (device_rate, latency) = (output.rate, output.latency);
        let handle = self.id();
        self.mixer().open(handle, rate);
        say!("audio: context {handle} at {rate} Hz (the output runs at {device_rate} Hz)");
        json!({
            "handle": handle,
            "sampleRate": rate,
            "maxChannelCount": 2,
            "baseLatency": QUANTUM as f64 / rate,
            "outputLatency": latency,
        })
        .to_string()
    }

    pub fn time(&self, handle: u32) -> f64 {
        self.mixer().time(handle)
    }

    pub fn suspend(&self, handle: u32, suspended: bool) {
        self.mixer().suspend(handle, suspended);
    }

    pub fn close(&self, handle: u32) {
        self.mixer().close(handle);
    }

    pub fn set_channels(&self, handle: u32, channels: u32) {
        self.mixer().set_channels(handle, channels.clamp(1, 32));
    }

    /// An AudioBuffer's samples (planar little-endian f32), kept under a new id.
    pub fn buffer_create(&mut self, planar: &[u8], channels: usize, frames: usize, rate: f64) -> Result<u32, String> {
        if channels == 0 || frames == 0 || !(rate > 0.0) {
            return Err(format!("a buffer of {channels} channels, {frames} frames at {rate} Hz cannot be made"));
        }
        let plane_bytes = frames * 4;
        if planar.len() < channels * plane_bytes {
            return Err(format!("{} bytes are too few for {channels} channels of {frames} frames", planar.len()));
        }
        let planes = planar
            .chunks_exact(plane_bytes)
            .take(channels)
            .map(|plane| plane.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
            .collect();
        let id = self.id();
        self.clips.insert(id, Arc::new(Clip { planes, frames, rate }));
        Ok(id)
    }

    /// Lets go of a buffer; voices playing it keep its samples.
    pub fn buffer_free(&mut self, id: u32) {
        self.clips.remove(&id);
    }

    /// Starts a voice; its token. `buffer` 0 plays silence, `duration` < 0 plays to the end, and
    /// `gain` 0 is a source that is timed but not heard.
    pub fn play(&mut self, handle: u32, buffer: u32, when: f64, offset: f64, duration: f64, gain: f64) -> u32 {
        let clip = self.clips.get(&buffer).cloned();
        let token = self.id();
        let duration = (duration >= 0.0).then_some(duration);
        self.mixer().play(handle, token, clip, when.max(0.0), offset.max(0.0), duration, gain as f32);
        token
    }

    pub fn stop(&self, handle: u32, token: u32, when: f64) {
        self.mixer().stop(handle, token, when.max(0.0));
    }

    /// Decodes a copy of `bytes` at `rate` off this thread; the token `poll` reports it under.
    pub fn decode(&mut self, bytes: &[u8], rate: f64) -> u32 {
        let token = self.id();
        self.decoder.start(token, bytes.to_vec(), rate);
        token
    }

    pub fn decode_take(&mut self, token: u32) -> Result<Vec<u8>, String> {
        self.decoder.take(token)
    }

    /// The voices that ended and the decodes that finished, as JSON `{ ended, decoded }`, or ''.
    pub fn poll(&mut self) -> String {
        let ended = self.mixer().take_ended();
        let decoded = self.decoder.poll();
        if ended.is_empty() && decoded.is_empty() {
            return String::new();
        }
        json!({ "ended": ended, "decoded": decoded }).to_string()
    }

    /// Whether something will need delivering soon: voices are scheduled or decodes running.
    pub fn busy(&self) -> bool {
        self.decoder.busy() || self.mixer().busy()
    }
}
