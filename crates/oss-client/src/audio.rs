//! Minimal sound mixer on cpal: one-shot effects plus a few looping channels.
//!
//! Sounds are decoded once, mixed down to mono and resampled to the device rate at
//! load time. The audio callback owns all voices and receives commands over a channel,
//! so it never blocks on the game thread.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use oss_assets::{AssetDirs, Sound};

/// Most simultaneous one-shot voices; the oldest is dropped beyond this.
const MAX_VOICES: usize = 48;
/// Looping channels (engine thrust, reverse thrust).
pub const LOOP_CHANNELS: usize = 2;

type Samples = Arc<[f32]>;

enum Command {
    Play {
        samples: Samples,
        gain: f32,
        pan: f32,
    },
    Loop {
        channel: usize,
        samples: Option<Samples>,
        gain: f32,
    },
}

struct Voice {
    samples: Samples,
    pos: usize,
    gain: f32,
    pan: f32,
}

#[derive(Default)]
struct LoopVoice {
    samples: Option<Samples>,
    pos: usize,
    gain: f32,
}

struct Mixer {
    rx: Receiver<Command>,
    voices: Vec<Voice>,
    loops: [LoopVoice; LOOP_CHANNELS],
    volume: f32,
}

impl Mixer {
    fn drain(&mut self) {
        while let Ok(cmd) = self.rx.try_recv() {
            match cmd {
                Command::Play { samples, gain, pan } => {
                    if self.voices.len() >= MAX_VOICES {
                        self.voices.remove(0);
                    }
                    self.voices.push(Voice {
                        samples,
                        pos: 0,
                        gain,
                        pan,
                    });
                }
                Command::Loop {
                    channel,
                    samples,
                    gain,
                } => {
                    if let Some(l) = self.loops.get_mut(channel) {
                        let same = match (&l.samples, &samples) {
                            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                            _ => false,
                        };
                        if !same {
                            l.pos = 0;
                        }
                        l.samples = samples;
                        l.gain = gain;
                    }
                }
            }
        }
    }

    fn fill<T: SizedSample + FromSample<f32>>(&mut self, out: &mut [T], channels: usize) {
        self.drain();
        let channels = channels.max(1);
        for frame in out.chunks_mut(channels) {
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for v in &mut self.voices {
                if let Some(&s) = v.samples.get(v.pos) {
                    let s = s * v.gain;
                    l += s * (1.0 - v.pan.max(0.0));
                    r += s * (1.0 + v.pan.min(0.0));
                    v.pos += 1;
                }
            }
            for lp in &mut self.loops {
                if let Some(samples) = &lp.samples
                    && !samples.is_empty()
                {
                    let s = samples[lp.pos % samples.len()] * lp.gain;
                    l += s;
                    r += s;
                    lp.pos = (lp.pos + 1) % samples.len();
                }
            }
            // Soft clip so many overlapping sounds don't wrap or crackle.
            let (l, r) = ((l * self.volume).tanh(), (r * self.volume).tanh());
            if channels == 1 {
                frame[0] = T::from_sample((l + r) * 0.5);
            } else {
                frame[0] = T::from_sample(l);
                frame[1] = T::from_sample(r);
                for s in frame.iter_mut().skip(2) {
                    *s = T::from_sample(0.0);
                }
            }
        }
        self.voices.retain(|v| v.pos < v.samples.len());
    }
}

pub struct Audio {
    _stream: cpal::Stream,
    tx: Sender<Command>,
    rate: u32,
    dir: AssetDirs,
    cache: HashMap<String, Option<Samples>>,
}

impl Audio {
    /// Opens the default output device. Returns `None` (with a log line) when there is
    /// no usable audio device, so the game runs silently.
    pub fn new(dir: &AssetDirs, volume: u32) -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().or_else(|| {
            log::warn!("no audio output device; sound disabled");
            None
        })?;
        let supported = device
            .default_output_config()
            .map_err(|e| log::warn!("audio: {e}; sound disabled"))
            .ok()?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        let (tx, rx) = channel();
        let mixer = Mixer {
            rx,
            voices: Vec::new(),
            loops: Default::default(),
            volume: volume.min(100) as f32 / 100.0,
        };
        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, config, mixer, channels),
            cpal::SampleFormat::I16 => build::<i16>(&device, config, mixer, channels),
            cpal::SampleFormat::I32 => build::<i32>(&device, config, mixer, channels),
            cpal::SampleFormat::U16 => build::<u16>(&device, config, mixer, channels),
            other => {
                log::warn!("audio: unsupported sample format {other}; sound disabled");
                return None;
            }
        }?;
        if let Err(e) = stream.play() {
            log::warn!("audio: {e}; sound disabled");
            return None;
        }
        log::info!("audio: {rate} Hz, {channels} channels, {format}");
        Some(Self {
            _stream: stream,
            tx,
            rate,
            dir: dir.clone(),
            cache: HashMap::new(),
        })
    }

    fn samples(&mut self, name: &str) -> Option<Samples> {
        if let Some(s) = self.cache.get(name) {
            return s.clone();
        }
        let loaded = match self.dir.load_sound(name) {
            Ok(snd) => Some(to_mono_resampled(&snd, self.rate)),
            Err(e) => {
                log::warn!("{e}");
                None
            }
        };
        self.cache.insert(name.to_string(), loaded.clone());
        loaded
    }

    /// Loads sounds ahead of time so the first play doesn't hitch.
    pub fn preload(&mut self, names: &[&str]) {
        for n in names {
            let _ = self.samples(n);
        }
    }

    /// Plays a sound once. `pan` is -1 (left) to 1 (right).
    pub fn play(&mut self, name: &str, gain: f32, pan: f32) {
        if gain <= 0.01 {
            return;
        }
        if let Some(samples) = self.samples(name) {
            let _ = self.tx.send(Command::Play {
                samples,
                gain,
                pan: pan.clamp(-1.0, 1.0),
            });
        }
    }

    /// Sets (or clears, with `None`) a looping channel.
    pub fn set_loop(&mut self, channel: usize, name: Option<&str>, gain: f32) {
        let samples = name.and_then(|n| self.samples(n));
        let _ = self.tx.send(Command::Loop {
            channel,
            samples,
            gain,
        });
    }
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut mixer: Mixer,
    channels: usize,
) -> Option<cpal::Stream> {
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| mixer.fill(data, channels),
            |e| log::warn!("audio stream: {e}"),
            None,
        )
        .map_err(|e| log::warn!("audio: {e}; sound disabled"))
        .ok()
}

/// Mixes to mono f32 and linearly resamples to `rate`.
pub fn to_mono_resampled(snd: &Sound, rate: u32) -> Samples {
    let ch = snd.channels.max(1) as usize;
    let mono: Vec<f32> = snd
        .samples
        .chunks(ch)
        .map(|f| f.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / ch as f32)
        .collect();
    if snd.sample_rate == rate || mono.len() < 2 || rate == 0 {
        return mono.into();
    }
    let step = snd.sample_rate as f64 / rate as f64;
    let out_len = ((mono.len() - 1) as f64 / step).floor() as usize + 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let p = i as f64 * step;
        let i0 = p.floor() as usize;
        let i1 = (i0 + 1).min(mono.len() - 1);
        let t = (p - i0 as f64) as f32;
        out.push(mono[i0] * (1.0 - t) + mono[i1] * t);
    }
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_length_and_endpoints() {
        let snd = Sound {
            sample_rate: 11025,
            channels: 1,
            samples: vec![0, 16384, 32767, 0],
        };
        let out = to_mono_resampled(&snd, 44100);
        assert_eq!(out.len(), 13);
        assert_eq!(out[0], 0.0);
        assert!((out[4] - 0.5).abs() < 1e-3);
        let stereo = Sound {
            sample_rate: 8000,
            channels: 2,
            samples: vec![32767, -32767, 16384, 16384],
        };
        let out = to_mono_resampled(&stereo, 8000);
        assert_eq!(out.len(), 2);
        assert!(out[0].abs() < 1e-3 && (out[1] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn mixer_plays_and_finishes() {
        let (tx, rx) = channel();
        let mut m = Mixer {
            rx,
            voices: Vec::new(),
            loops: Default::default(),
            volume: 1.0,
        };
        tx.send(Command::Play {
            samples: vec![0.5; 4].into(),
            gain: 1.0,
            pan: 0.0,
        })
        .unwrap();
        let mut out = [0.0f32; 12];
        m.fill(&mut out, 2);
        assert!(out[0] > 0.4 && out[1] > 0.4);
        assert_eq!(out[8], 0.0);
        assert!(m.voices.is_empty());
    }
}
