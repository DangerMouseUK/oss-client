//! Sound decoding. Continuum's `.wa2` files are plain RIFF/WAVE PCM (8-bit unsigned or
//! 16-bit signed, mono, 11025–44100 Hz); user `.wav` files may also be stereo.

use thiserror::Error;

/// Largest decoded sample count we accept (about 6 minutes of 44.1 kHz stereo).
pub const MAX_SAMPLES: usize = 32 * 1024 * 1024;

/// Decoded PCM, interleaved signed 16-bit samples.
#[derive(Clone, PartialEq, Eq)]
pub struct Sound {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved samples (`frames * channels`).
    pub samples: Vec<i16>,
}

impl std::fmt::Debug for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sound")
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames())
            .finish()
    }
}

impl Sound {
    /// Number of sample frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Duration in seconds.
    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.sample_rate.max(1) as f64
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SoundError {
    #[error("not a RIFF/WAVE file")]
    NotWave,
    #[error("truncated wave data")]
    Truncated,
    #[error("missing {0} chunk")]
    MissingChunk(&'static str),
    #[error("unsupported wave: {0}")]
    Unsupported(&'static str),
    #[error("wave too large")]
    TooLarge,
}

const WAVE_FORMAT_PCM: u16 = 1;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Decodes a RIFF/WAVE PCM file (`.wav` or `.wa2`).
pub fn decode_wave(data: &[u8]) -> Result<Sound, SoundError> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(SoundError::NotWave);
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut pcm: Option<&[u8]> = None;
    let mut i = 12usize;
    while i + 8 <= data.len() {
        let id = &data[i..i + 4];
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let body_start = i + 8;
        // Tolerate a data chunk whose declared length overruns the file (common in old tools).
        let body_end = body_start.saturating_add(len).min(data.len());
        let body = &data[body_start..body_end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(SoundError::Truncated);
                }
                let mut tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                if tag == WAVE_FORMAT_EXTENSIBLE && body.len() >= 26 {
                    tag = u16::from_le_bytes([body[24], body[25]]);
                }
                fmt = Some((tag, channels, rate, bits));
            }
            b"data" => pcm = Some(body),
            _ => {}
        }
        if body_start.saturating_add(len) >= data.len() {
            break;
        }
        i = body_start + len + (len & 1);
    }

    let (tag, channels, sample_rate, bits) = fmt.ok_or(SoundError::MissingChunk("fmt"))?;
    let pcm = pcm.ok_or(SoundError::MissingChunk("data"))?;
    if tag != WAVE_FORMAT_PCM {
        return Err(SoundError::Unsupported("non-PCM encoding"));
    }
    if channels == 0 || channels > 8 {
        return Err(SoundError::Unsupported("channel count"));
    }
    if sample_rate == 0 || sample_rate > 384_000 {
        return Err(SoundError::Unsupported("sample rate"));
    }
    let samples: Vec<i16> = match bits {
        8 => {
            if pcm.len() > MAX_SAMPLES {
                return Err(SoundError::TooLarge);
            }
            pcm.iter().map(|&b| ((b as i16) - 128) << 8).collect()
        }
        16 => {
            if pcm.len() / 2 > MAX_SAMPLES {
                return Err(SoundError::TooLarge);
            }
            pcm.as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect()
        }
        24 => {
            if pcm.len() / 3 > MAX_SAMPLES {
                return Err(SoundError::TooLarge);
            }
            pcm.as_chunks::<3>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes([c[1], c[2]]))
                .collect()
        }
        _ => return Err(SoundError::Unsupported("bit depth")),
    };
    // Drop a trailing partial frame.
    let whole = samples.len() - samples.len() % channels as usize;
    let mut samples = samples;
    samples.truncate(whole);
    Ok(Sound {
        sample_rate,
        channels,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(channels: u16, rate: u32, bits: u16, pcm: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&((4 + 8 + 16 + 8 + pcm.len()) as u32).to_le_bytes());
        v.extend_from_slice(b"WAVE");
        v.extend_from_slice(b"fmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        let align = channels * bits / 8;
        v.extend_from_slice(&(rate * align as u32).to_le_bytes());
        v.extend_from_slice(&align.to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"LIST");
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(&[1, 2, 3, 0]); // odd chunk + pad byte
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        v.extend_from_slice(pcm);
        v
    }

    #[test]
    fn decodes_8bit_mono() {
        let s = decode_wave(&wave(1, 11025, 8, &[128, 255, 0])).unwrap();
        assert_eq!(s.sample_rate, 11025);
        assert_eq!(s.samples, vec![0, 127 << 8, -128 << 8]);
        assert_eq!(s.frames(), 3);
    }

    #[test]
    fn decodes_16bit_stereo() {
        let s = decode_wave(&wave(2, 44100, 16, &[1, 0, 0xFF, 0xFF, 5, 0])).unwrap();
        assert_eq!(s.channels, 2);
        // Trailing half frame dropped.
        assert_eq!(s.samples, vec![1, -1]);
    }

    #[test]
    fn rejects_and_survives_garbage() {
        assert_eq!(decode_wave(b"RIFF\0\0\0\0WAVX"), Err(SoundError::NotWave));
        let good = wave(1, 22050, 16, &[1, 2, 3, 4]);
        for len in 0..good.len() {
            let _ = decode_wave(&good[..len]);
        }
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0xFF;
            let _ = decode_wave(&b);
        }
    }
}
