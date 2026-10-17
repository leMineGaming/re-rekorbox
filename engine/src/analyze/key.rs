//! Key detection (own implementation of the public Krumhansl–Schmuckler method): the track's mean
//! chroma is correlated with the 24 rotated major/minor key profiles (Krumhansl–Kessler).
//! rekordbox stores the key only in its database (SPEC §5j), so the result goes into the analysis
//! side-car file, not into ANLZ tags.

const MAJOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];
const NAMES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

/// A musical key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    /// Tonic pitch class, C = 0.
    pub tonic: u8,
    pub minor: bool,
}

impl Key {
    /// Name like `Am`, `F#`, `Ebm`.
    pub fn name(&self) -> String {
        format!("{}{}", NAMES[usize::from(self.tonic % 12)], if self.minor { "m" } else { "" })
    }

    /// Camelot code like `8A` (minor) / `8B` (major).
    pub fn camelot(&self) -> String {
        // C major = 8B, A minor = 8A; one step = a fifth (7 semitones)
        let (base, letter) = if self.minor { ((u16::from(self.tonic) + 3) % 12, 'A') } else { (u16::from(self.tonic), 'B') };
        let n = (base * 7 % 12 + 7) % 12 + 1;
        format!("{n}{letter}")
    }
}

fn corr(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let ma = a.iter().sum::<f64>() / 12.0;
    let mb = b.iter().sum::<f64>() / 12.0;
    let (mut s, mut sa, mut sb) = (0.0, 0.0, 0.0);
    for i in 0..12 {
        s += (a[i] - ma) * (b[i] - mb);
        sa += (a[i] - ma).powi(2);
        sb += (b[i] - mb).powi(2);
    }
    s / (sa.sqrt() * sb.sqrt()).max(1e-12)
}

/// Key with the highest profile correlation for a mean chroma vector, and that correlation.
pub fn key_of_chroma(chroma: &[f64; 12]) -> (Key, f64) {
    let mut best = (Key { tonic: 0, minor: false }, f64::MIN);
    for tonic in 0..12u8 {
        for (minor, prof) in [(false, &MAJOR), (true, &MINOR)] {
            let rot: [f64; 12] = std::array::from_fn(|i| prof[(i + 12 - usize::from(tonic)) % 12]);
            let r = corr(chroma, &rot);
            if r > best.1 {
                best = (Key { tonic, minor }, r);
            }
        }
    }
    best
}

/// Key from per-frame chroma vectors.
pub fn detect_key(frames: &[[f32; 12]]) -> Option<(Key, f64)> {
    if frames.is_empty() {
        return None;
    }
    let mut m = [0.0f64; 12];
    for f in frames {
        for (a, b) in m.iter_mut().zip(f) {
            *a += f64::from(*b);
        }
    }
    Some(key_of_chroma(&m))
}

/// Chroma frames for key detection: 8192-point STFT every 0.25 s, reduced to a semitone spectrum
/// (strongest bin per semitone, G2–B6), whitened in the semitone domain (each semitone divided by the
/// mean of its neighbours within an octave, so that the other chord tones average out) and only local peaks counted. Working on semitones keeps the
/// weighting independent of frequency (FFT bins are linear, pitch is logarithmic), and the whitening
/// keeps drums and bass resonances from dominating.
pub fn key_chroma(pcm: &crate::pcm::Pcm) -> Vec<[f32; 12]> {
    const N: usize = 8192;
    const LO: i32 = 43; // MIDI G2 ≈ 98 Hz
    const HI: i32 = 95; // MIDI B6 ≈ 1976 Hz
    let sr = pcm.sample_rate.max(1) as f32;
    let hop = (sr * 0.25) as usize;
    let window: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    let bin = |midi: f32| 440.0 * 2f32.powf((midi - 69.0) / 12.0) / sr * N as f32;
    // bin range of each semitone (at least the nearest bin)
    let ranges: Vec<(usize, usize)> = (LO..=HI)
        .map(|m| {
            let (a, b) = (bin(m as f32 - 0.5).ceil() as usize, bin(m as f32 + 0.5).floor() as usize);
            let c = bin(m as f32).round() as usize;
            (a.min(c), b.max(c))
        })
        .collect();
    let top = ranges.last().map_or(0, |r| r.1) + 1;
    let mut out = Vec::new();
    let (mut re, mut im) = (vec![0.0f32; N], vec![0.0f32; N]);
    let mut start = 0;
    while start + N <= pcm.len() {
        for k in 0..N {
            let f = pcm.frames[start + k];
            re[k] = 0.5 * (f[0] + f[1]) * window[k];
            im[k] = 0.0;
        }
        super::beats::fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..=top).map(|b| (re[b] * re[b] + im[b] * im[b]).sqrt()).collect();
        let semi: Vec<f32> = ranges.iter().map(|&(a, b)| mag[a..=b].iter().fold(0.0f32, |m, &v| m.max(v))).collect();
        let mut ch = [0.0f32; 12];
        for i in 0..semi.len() {
            let (s, e) = (i.saturating_sub(12), (i + 12).min(semi.len() - 1));
            let mean = (semi[s..=e].iter().sum::<f32>() - semi[i]) / (e - s) as f32;
            let v = semi[i] / mean.max(1e-9);
            let peak = (i == 0 || semi[i] >= semi[i - 1]) && (i + 1 == semi.len() || semi[i] >= semi[i + 1]);
            if v > 1.5 && peak {
                ch[((LO + i as i32) % 12) as usize] += v - 1.5;
            }
        }
        out.push(ch);
        start += hop;
    }
    out
}
