//! decodeAudioData: compressed sound decoded off the JS thread (symphonia: MP3, WAV and its PCM
//! and ADPCM codecs), then resampled to the context's rate as a browser does.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::mixer::hermite;
use crate::log::warn;

/// How many decodes run at once.
const WORKERS: usize = 2;

struct Job {
    token: u32,
    bytes: Vec<u8>,
    rate: f64,
}

type Results = Arc<Mutex<Vec<(u32, Result<Vec<u8>, String>)>>>;

pub struct Decoder {
    jobs: Option<Sender<Job>>,
    finished: Results,
    /// Finished decodes that the page was told of and has yet to take.
    ready: HashMap<u32, Result<Vec<u8>, String>>,
    in_flight: usize,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder { jobs: None, finished: Arc::new(Mutex::new(Vec::new())), ready: HashMap::new(), in_flight: 0 }
    }

    fn workers(&mut self) -> &Sender<Job> {
        self.jobs.get_or_insert_with(|| {
            let (send, receive) = channel::<Job>();
            let receive = Arc::new(Mutex::new(receive));
            for i in 0..WORKERS {
                let (receive, finished) = (receive.clone(), self.finished.clone());
                let spawned = std::thread::Builder::new()
                    .name(format!("audio decode {i}"))
                    .stack_size(1 << 20)
                    .spawn(move || work(&receive, &finished));
                if let Err(e) = spawned {
                    warn!("audio: a decode thread did not start: {e}");
                }
            }
            send
        })
    }

    /// Decodes a copy of `bytes` to `rate`; the token `poll` reports it under.
    pub fn start(&mut self, token: u32, bytes: Vec<u8>, rate: f64) {
        self.in_flight += 1;
        if let Err(e) = self.workers().send(Job { token, bytes, rate }) {
            // No worker to take it: decoded here instead.
            let Job { token, bytes, rate } = e.0;
            let result = decode(bytes, rate);
            self.finished.lock().unwrap_or_else(|e| e.into_inner()).push((token, result));
        }
    }

    /// The decodes that finished since the last poll.
    pub fn poll(&mut self) -> Vec<u32> {
        let finished = std::mem::take(&mut *self.finished.lock().unwrap_or_else(|e| e.into_inner()));
        self.in_flight -= finished.len().min(self.in_flight);
        finished
            .into_iter()
            .map(|(token, result)| {
                self.ready.insert(token, result);
                token
            })
            .collect()
    }

    pub fn take(&mut self, token: u32) -> Result<Vec<u8>, String> {
        self.ready.remove(&token).unwrap_or_else(|| Err(format!("there is no decoded sound {token}")))
    }

    pub fn busy(&self) -> bool {
        self.in_flight > 0
    }
}

fn work(jobs: &Mutex<Receiver<Job>>, finished: &Mutex<Vec<(u32, Result<Vec<u8>, String>)>>) {
    loop {
        let job = jobs.lock().unwrap_or_else(|e| e.into_inner()).recv();
        let Ok(Job { token, bytes, rate }) = job else { return };
        let result = std::panic::catch_unwind(move || decode(bytes, rate))
            .unwrap_or_else(|_| Err("the decoder failed".to_string()));
        finished.lock().unwrap_or_else(|e| e.into_inner()).push((token, result));
    }
}

/// The decoded sound, as `audio_decode_take` returns it: `[u32 channels][u32 frames][f32 rate]
/// [u32 0]` and then the planar f32 samples, all little-endian.
fn decode(bytes: Vec<u8>, rate: f64) -> Result<Vec<u8>, String> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(&Hint::new(), source, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| e.to_string())?;
    let track = format.default_track(TrackType::Audio).ok_or("there is no audio track")?;
    let track_id = track.id;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("the track is not audio")?.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| e.to_string())?;

    let mut planes: Vec<Vec<f32>> = Vec::new();
    let mut source_rate = params.sample_rate;
    let mut chunk: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A stream cut short is decoded as far as it goes.
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) if !planes.is_empty() => {
                warn!("audio: a decode stopped early: {e}");
                break;
            }
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buffer) => {
                source_rate = Some(buffer.spec().rate());
                buffer.copy_to_vecs_planar(&mut chunk);
                if planes.len() < chunk.len() {
                    let frames = planes.first().map_or(0, Vec::len);
                    planes.resize(chunk.len(), vec![0.0; frames]);
                }
                for (plane, samples) in planes.iter_mut().zip(&chunk) {
                    plane.extend_from_slice(samples);
                }
            }
            // A damaged packet is left out, as browsers' decoders do.
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let frames = planes.first().map_or(0, Vec::len);
    if frames == 0 {
        return Err("the data holds no sound".into());
    }
    let source_rate = f64::from(source_rate.ok_or("the sound has no sample rate")?);
    let target = if rate > 0.0 { rate } else { source_rate };
    let planes: Vec<Vec<f32>> = planes.iter().map(|p| resample(p, source_rate, target)).collect();
    let frames = planes[0].len();

    let mut out = Vec::with_capacity(16 + planes.len() * frames * 4);
    out.extend_from_slice(&(planes.len() as u32).to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    out.extend_from_slice(&(target as f32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for plane in &planes {
        for s in plane {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    Ok(out)
}

/// One channel from `from` Hz to `to` Hz (cubic Hermite).
fn resample(plane: &[f32], from: f64, to: f64) -> Vec<f32> {
    if from == to || plane.is_empty() {
        return plane.to_vec();
    }
    let len = plane.len() as isize;
    let frames = ((plane.len() as f64) * to / from).round().max(1.0) as usize;
    let step = from / to;
    let at = |j: isize| plane[j.clamp(0, len - 1) as usize];
    (0..frames)
        .map(|i| {
            let pos = i as f64 * step;
            let k = pos as isize;
            let t = (pos - k as f64) as f32;
            hermite(at(k - 1), at(k), at(k + 1), at(k + 2), t)
        })
        .collect()
}
