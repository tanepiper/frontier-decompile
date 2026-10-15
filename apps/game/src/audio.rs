//! The game's one audio output: everything the game plays is mixed here into a stereo stream at
//! the host's rate, then handed to the platform backend (WebAudio in the browser).
//!
//! Natively the backend is the system's default output device through cpal (CoreAudio, WASAPI
//! or ALSA); devices with more than two channels get the stereo pair on their first two.
//!
//! Sources today: the OPL2 FM chip (music and effects from the original AdLib driver,
//! docs/AUDIO.md), at its native 49,716 Hz. It is band-limited and resampled to the output rate.
//! The resampling ratio is nudged by up to ±0.5 % to hold the backend's queue near
//! [`TARGET_LATENCY`], so virtual time and the sound card's clock never drift apart audibly.

use frontier_opl::{Resampler, SAMPLE_RATE};

/// Audio queued ahead of the output, in seconds.
pub const TARGET_LATENCY: f64 = 0.08;

pub struct Mixer {
    fm: Resampler,
    mono: Vec<f32>,
    /// Master gain, 0–1.
    pub volume: f32,
    pub muted: bool,
}

impl Mixer {
    pub fn new(out_rate: f64) -> Self {
        Mixer { fm: Resampler::new(SAMPLE_RATE, out_rate), mono: Vec::new(), volume: 0.8, muted: false }
    }

    /// Mix the chip samples produced since the last call into interleaved stereo frames appended
    /// to `out`. `queued` is how much audio (seconds) the backend still has buffered.
    pub fn process(&mut self, fm: &mut Vec<i16>, queued: f64, out: &mut Vec<f32>) {
        let error = queued - TARGET_LATENCY;
        self.fm.set_adjust(1.0 + (error * 0.05).clamp(-0.005, 0.005));
        self.fm.push(fm);
        fm.clear();
        self.mono.clear();
        self.fm.pull(&mut self.mono);
        // The chip's full-scale sum is ±32767 but a single voice peaks near ±4084; this gain
        // keeps typical game music well inside full scale without clipping the loudest chords.
        let gain = if self.muted { 0.0 } else { self.volume * 2.0 };
        out.reserve(self.mono.len() * 2);
        for &s in &self.mono {
            let v = (s * gain).clamp(-1.0, 1.0);
            out.push(v);
            out.push(v);
        }
    }
}

#[cfg(target_arch = "wasm32")]
type Backend = crate::web::WebAudio;
#[cfg(not(target_arch = "wasm32"))]
type Backend = native::NativeAudio;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SizedSample};

    /// Interleaved stereo frames waiting for the device callback.
    type Queue = Arc<Mutex<VecDeque<f32>>>;

    /// Never queue more than this (seconds); older audio is dropped first.
    const MAX_QUEUE: f64 = 0.5;

    pub struct NativeAudio {
        _stream: cpal::Stream,
        queue: Queue,
        rate: f64,
    }

    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        queue: Queue,
    ) -> Result<cpal::Stream, cpal::BuildStreamError> {
        let channels = config.channels as usize;
        device.build_output_stream(
            config,
            move |data: &mut [T], _| {
                let mut q = queue.lock().unwrap();
                for frame in data.chunks_mut(channels) {
                    // On underrun play silence rather than stall.
                    let l = q.pop_front().unwrap_or(0.0);
                    let r = q.pop_front().unwrap_or(0.0);
                    for (c, out) in frame.iter_mut().enumerate() {
                        let v = match (c, channels) {
                            (_, 1) => 0.5 * (l + r),
                            (0, _) => l,
                            (1, _) => r,
                            _ => 0.0,
                        };
                        *out = T::from_sample(v);
                    }
                }
            },
            |e| log::warn!("audio: {e}"),
            None,
        )
    }

    impl NativeAudio {
        pub fn new() -> Option<NativeAudio> {
            let device = cpal::default_host().default_output_device()?;
            let supported = device.default_output_config().map_err(|e| log::warn!("audio: {e}")).ok()?;
            let format = supported.sample_format();
            let config: cpal::StreamConfig = supported.into();
            let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
            let q = queue.clone();
            let stream = match format {
                cpal::SampleFormat::F32 => build::<f32>(&device, &config, q),
                cpal::SampleFormat::I16 => build::<i16>(&device, &config, q),
                cpal::SampleFormat::U16 => build::<u16>(&device, &config, q),
                cpal::SampleFormat::I32 => build::<i32>(&device, &config, q),
                other => {
                    log::warn!("audio: unsupported sample format {other}");
                    return None;
                }
            }
            .map_err(|e| log::warn!("audio: {e}"))
            .ok()?;
            stream.play().map_err(|e| log::warn!("audio: {e}")).ok()?;
            Some(NativeAudio { _stream: stream, queue, rate: config.sample_rate.0 as f64 })
        }

        pub fn rate(&self) -> f64 {
            self.rate
        }

        pub fn resume(&self) {}

        pub fn queued(&self) -> f64 {
            self.queue.lock().unwrap().len() as f64 / 2.0 / self.rate
        }

        pub fn play(&mut self, frames: &[f32]) {
            let mut q = self.queue.lock().unwrap();
            q.extend(frames);
            let max = (MAX_QUEUE * self.rate) as usize * 2;
            if q.len() > max {
                let excess = q.len() - max;
                q.drain(..excess);
            }
        }
    }
}

/// The output device: the mixer feeding the platform backend.
pub struct Output {
    mixer: Mixer,
    backend: Backend,
    frames: Vec<f32>,
}

impl Output {
    pub fn new() -> Option<Output> {
        let backend = Backend::new()?;
        log::info!("audio: {} Hz stereo", backend.rate());
        Some(Output { mixer: Mixer::new(backend.rate()), backend, frames: Vec::new() })
    }

    /// Browsers only start audio after a user gesture; call this from input events.
    pub fn resume(&self) {
        self.backend.resume();
    }

    /// Mix and queue what the machine produced this frame.
    pub fn pump(&mut self, fm: &mut Vec<i16>) {
        self.frames.clear();
        self.mixer.process(fm, self.backend.queued(), &mut self.frames);
        self.backend.play(&self.frames);
    }
}
