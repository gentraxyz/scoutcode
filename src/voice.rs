//! Voice input: records the default microphone while the mic button is on, then sends the recording to ScoutCode's
//! /audio/transcriptions, which transcribes it with Deepgram. Nothing is recorded while the button is off.

use crate::agent::{Ev, Hooks};
use crate::config::Config;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What's sent: 16 kHz mono, plenty for speech and a quarter of the size of 44.1 kHz stereo.
const SAMPLE_RATE: usize = 16_000;
/// Longest recording kept: 5 minutes (about 9.6 MB as 16-bit WAV).
const MAX_SECONDS: usize = 300;

/// What the worker tells the window.
pub enum Update {
    /// Recording stopped and work started: what it's doing now.
    Busy(&'static str),
    /// The transcript, or why there isn't one. Empty when nothing was said.
    Done(Result<String, String>),
}

/// A recording in progress. Stopping it hands the audio to the worker, which transcribes it.
pub struct Recording {
    stop: Arc<AtomicBool>,
    /// The latest loudness, 0..=1000, for the button.
    level: Arc<AtomicU32>,
}

impl Recording {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn level(&self) -> f32 {
        self.level.load(Ordering::Relaxed) as f32 / 1000.0
    }
}

pub fn start(cfg: Config, h: Hooks) -> Recording {
    let stop = Arc::new(AtomicBool::new(false));
    let level = Arc::new(AtomicU32::new(0));
    let rec = Recording { stop: stop.clone(), level: level.clone() };
    std::thread::spawn(move || {
        let send = |u: Update| h.send(Ev::Voice(u));
        let audio = match record(&stop, &level) {
            Ok(a) => a,
            Err(e) => return send(Update::Done(Err(e))),
        };
        // Under a third of a second, or near silence: nothing to transcribe.
        let loudest = audio.iter().fold(0f32, |m, s| m.max(s.abs()));
        if audio.len() < SAMPLE_RATE / 3 || loudest < 0.01 {
            return send(Update::Done(Ok(String::new())));
        }
        send(Update::Busy("Transcribing…"));
        let result = crate::api::transcribe(&cfg, &wav(&audio)).map_err(|e| format!("Could not transcribe: {e}"));
        send(Update::Done(result));
    });
    rec
}

/// Records from the default input until `stop`, as 16 kHz mono.
fn record(stop: &AtomicBool, level: &Arc<AtomicU32>) -> Result<Vec<f32>, String> {
    let device = cpal::default_host().default_input_device().ok_or("No microphone found.")?;
    let supported = device.default_input_config().map_err(|e| format!("Could not open the microphone: {e}"))?;
    let (rate, channels) = (supported.sample_rate() as usize, supported.channels() as usize);
    let samples: Arc<Mutex<Vec<f32>>> = Arc::default();
    let cap = rate * MAX_SECONDS;

    fn on_data<T: cpal::Sample>(data: &[T], channels: usize, cap: usize, samples: &Mutex<Vec<f32>>, level: &AtomicU32)
    where
        f32: cpal::FromSample<T>,
    {
        let mono: Vec<f32> = data.chunks(channels).map(|f| f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32).collect();
        let rms = (mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32).sqrt();
        level.store((rms * 4000.0).min(1000.0) as u32, Ordering::Relaxed);
        let mut all = samples.lock().unwrap();
        let room = cap.saturating_sub(all.len());
        all.extend(mono.into_iter().take(room));
    }
    macro_rules! build {
        ($t:ty) => {{
            let (samples, level) = (samples.clone(), level.clone());
            device.build_input_stream::<$t, _, _>(
                supported.config(),
                move |d, _| on_data(d, channels, cap, &samples, &level),
                |_| {},
                None,
            )
        }};
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        f => return Err(format!("The microphone's sample format ({f}) isn't supported.")),
    }
    .map_err(|e| format!("Could not open the microphone: {e}"))?;
    stream.play().map_err(|e| format!("Could not start the microphone: {e}"))?;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);
    level.store(0, Ordering::Relaxed);
    let all = std::mem::take(&mut *samples.lock().unwrap());
    Ok(resample(&all, rate, SAMPLE_RATE))
}

/// Box-filtered resampling: each output sample averages the input samples it covers. Plenty for speech.
fn resample(input: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let n = (input.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let a = (i as f64 * ratio) as usize;
            let b = (((i + 1) as f64 * ratio) as usize).clamp(a + 1, input.len());
            input[a..b].iter().sum::<f32>() / (b - a) as f32
        })
        .collect()
}

/// The samples as a 16-bit PCM mono WAV file.
fn wav(pcm: &[f32]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE as u32 * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_header_matches_the_samples() {
        let w = super::wav(&[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 8);
        assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 8);
    }
}
