//! Decoded audio held in memory: stereo `f32` frames at the file's sample rate.
//!
//! [`decode_file`] reads any format symphonia supports (FLAC, MP3, WAV, AAC/M4A, Ogg Vorbis); mono is
//! duplicated to both channels, extra channels are dropped. [`write_wav`] writes 16-bit PCM WAV, used
//! by the offline renderer and the tests.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One stereo frame (left, right).
pub type Frame = [f32; 2];

/// Decoded audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub frames: Vec<Frame>,
}

impl Pcm {
    pub fn new(sample_rate: u32, frames: Vec<Frame>) -> Pcm {
        Pcm { sample_rate, frames }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn duration_s(&self) -> f64 {
        self.frames.len() as f64 / f64::from(self.sample_rate)
    }

    /// Frame at an integer index; silence outside the track.
    pub fn at(&self, i: i64) -> Frame {
        if i < 0 {
            return [0.0; 2];
        }
        self.frames.get(i as usize).copied().unwrap_or([0.0; 2])
    }

    /// Frame at a fractional position, 4-point Hermite interpolation.
    pub fn sample(&self, pos: f64) -> Frame {
        let i = pos.floor();
        let t = (pos - i) as f32;
        let i = i as i64;
        let (y0, y1, y2, y3) = (self.at(i - 1), self.at(i), self.at(i + 1), self.at(i + 2));
        let mut out = [0.0; 2];
        for c in 0..2 {
            let c0 = y1[c];
            let c1 = 0.5 * (y2[c] - y0[c]);
            let c2 = y0[c] - 2.5 * y1[c] + 2.0 * y2[c] - 0.5 * y3[c];
            let c3 = 0.5 * (y3[c] - y0[c]) + 1.5 * (y1[c] - y2[c]);
            out[c] = ((c3 * t + c2) * t + c1) * t + c0;
        }
        out
    }

    /// Sine test tone (both channels).
    pub fn sine(sample_rate: u32, freq: f64, seconds: f64, amplitude: f32) -> Pcm {
        let n = (seconds * f64::from(sample_rate)) as usize;
        let frames = (0..n)
            .map(|i| {
                let v = (std::f64::consts::TAU * freq * i as f64 / f64::from(sample_rate)).sin() as f32 * amplitude;
                [v, v]
            })
            .collect();
        Pcm::new(sample_rate, frames)
    }

    /// Click track: a short decaying burst on every beat, silence in between (handy for sync tests).
    pub fn click_track(sample_rate: u32, bpm: f64, seconds: f64) -> Pcm {
        let n = (seconds * f64::from(sample_rate)) as usize;
        let beat = 60.0 / bpm * f64::from(sample_rate);
        let frames = (0..n)
            .map(|i| {
                let phase = i as f64 % beat;
                let v = if phase < 0.01 * f64::from(sample_rate) {
                    ((phase * 0.05).sin() * (-phase / 200.0).exp()) as f32 * 0.8
                } else {
                    0.0
                };
                [v, v]
            })
            .collect();
        Pcm::new(sample_rate, frames)
    }
}

#[derive(Debug)]
pub enum DecodeError {
    Io(io::Error),
    Format(String),
    NoAudioTrack,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "{e}"),
            DecodeError::Format(e) => write!(f, "decode error: {e}"),
            DecodeError::NoAudioTrack => f.write_str("no audio track found"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<io::Error> for DecodeError {
    fn from(e: io::Error) -> Self {
        DecodeError::Io(e)
    }
}

fn sym(e: SymError) -> DecodeError {
    match e {
        SymError::IoError(e) => DecodeError::Io(e),
        e => DecodeError::Format(e.to_string()),
    }
}

/// Decode a whole audio file into memory.
pub fn decode_file(path: &Path) -> Result<Pcm, DecodeError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(sym)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(sym)?;
    let mut frames = Vec::new();
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(sym(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // corrupt packet: skip it
            Err(e) => return Err(sym(e)),
        };
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let need = decoded.capacity() as u64;
        if buf.as_ref().map_or(true, |b| (b.capacity() as u64) < need * channels as u64) {
            buf = Some(SampleBuffer::new(need, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        for f in b.samples().chunks_exact(channels) {
            frames.push(if channels == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
        }
    }
    Ok(Pcm::new(sample_rate, frames))
}

/// Write 16-bit stereo PCM WAV.
pub fn write_wav(path: &Path, pcm: &Pcm) -> io::Result<()> {
    let mut w = io::BufWriter::new(File::create(path)?);
    let data_len = (pcm.frames.len() * 4) as u32;
    w.write_all(b"RIFF")?;
    w.write_all(&(36 + data_len).to_le_bytes())?;
    w.write_all(b"WAVEfmt ")?;
    w.write_all(&16u32.to_le_bytes())?;
    w.write_all(&1u16.to_le_bytes())?; // PCM
    w.write_all(&2u16.to_le_bytes())?;
    w.write_all(&pcm.sample_rate.to_le_bytes())?;
    w.write_all(&(pcm.sample_rate * 4).to_le_bytes())?;
    w.write_all(&4u16.to_le_bytes())?;
    w.write_all(&16u16.to_le_bytes())?;
    w.write_all(b"data")?;
    w.write_all(&data_len.to_le_bytes())?;
    for f in &pcm.frames {
        for s in f {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            w.write_all(&v.to_le_bytes())?;
        }
    }
    w.flush()
}
