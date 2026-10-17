//! Sampler: 16 one-shot slots mixed into their own bus. Slots can be filled from audio files or
//! with the built-in synthesized sounds (so the sampler works without any sample files).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::dsp::Noise;
use crate::pcm::{decode_file, Frame, Pcm};

pub const SLOTS: usize = 16;

#[derive(Debug, Clone, Default)]
struct Voice {
    pcm: Option<Arc<Pcm>>,
    pos: f64,
    playing: bool,
}

#[derive(Debug, Clone)]
pub struct Sampler {
    voices: Vec<Voice>,
    /// Sampler volume 0..1.
    pub volume: f32,
}

impl Default for Sampler {
    fn default() -> Self {
        Sampler { voices: vec![Voice::default(); SLOTS], volume: 0.8 }
    }
}

impl Sampler {
    pub fn load(&mut self, slot: usize, pcm: Arc<Pcm>) {
        if let Some(v) = self.voices.get_mut(slot) {
            *v = Voice { pcm: Some(pcm), pos: 0.0, playing: false };
        }
    }

    /// Start (or retrigger) a slot from the beginning.
    pub fn start(&mut self, slot: usize) {
        if let Some(v) = self.voices.get_mut(slot) {
            if v.pcm.is_some() {
                v.pos = 0.0;
                v.playing = true;
            }
        }
    }

    pub fn stop(&mut self, slot: usize) {
        if let Some(v) = self.voices.get_mut(slot) {
            v.playing = false;
        }
    }

    pub fn is_loaded(&self, slot: usize) -> bool {
        self.voices.get(slot).is_some_and(|v| v.pcm.is_some())
    }

    pub fn is_playing(&self, slot: usize) -> bool {
        self.voices.get(slot).is_some_and(|v| v.playing)
    }

    /// Bit mask of playing slots.
    pub fn playing_mask(&self) -> u16 {
        self.voices.iter().enumerate().filter(|(_, v)| v.playing).fold(0, |m, (i, _)| m | 1 << i)
    }

    /// Add all playing voices to `out` (resampled to `out_rate`), scaled by the volume.
    pub fn render(&mut self, out: &mut [Frame], out_rate: u32) {
        for v in self.voices.iter_mut().filter(|v| v.playing) {
            let pcm = v.pcm.clone().unwrap();
            let step = f64::from(pcm.sample_rate) / f64::from(out_rate);
            for o in out.iter_mut() {
                if v.pos >= pcm.len() as f64 {
                    v.playing = false;
                    break;
                }
                let f = pcm.sample(v.pos);
                o[0] += f[0] * self.volume;
                o[1] += f[1] * self.volume;
                v.pos += step;
            }
        }
    }

    /// Fill all slots with the built-in sounds.
    pub fn load_builtin(&mut self, sample_rate: u32) {
        for (i, pcm) in builtin_sounds(sample_rate).into_iter().enumerate() {
            self.load(i, Arc::new(pcm));
        }
    }
}

/// Audio files of a folder (sorted by name), decoded; at most [`SLOTS`]. Errors are skipped.
pub fn load_folder(dir: &Path) -> Vec<(PathBuf, Pcm)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| it.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| crate::browser::is_audio(p)).collect())
        .unwrap_or_default();
    files.sort();
    files.into_iter().filter_map(|p| decode_file(&p).ok().map(|pcm| (p, pcm))).take(SLOTS).collect()
}

/// Names of the built-in sounds in slot order.
pub const BUILTIN_NAMES: [&str; SLOTS] = [
    "kick", "snare", "clap", "hihat", "open hat", "tom", "rim", "crash", "bass hit", "stab", "horn", "siren",
    "riser", "downlifter", "zap", "laser",
];

/// Synthesized one-shots (own simple synthesis: sine sweeps, filtered noise, envelopes).
pub fn builtin_sounds(sr: u32) -> Vec<Pcm> {
    let fs = sr as f32;
    let make = |seconds: f32, f: &dyn Fn(f32, &mut Noise) -> f32| {
        let mut noise = Noise::default();
        let n = (seconds * fs) as usize;
        Pcm::new(sr, (0..n).map(|i| {
            let v = f(i as f32 / fs, &mut noise).clamp(-1.0, 1.0) * 0.8;
            [v, v]
        }).collect())
    };
    let sine = |freq: f32, t: f32| (std::f32::consts::TAU * freq * t).sin();
    let env = |t: f32, decay: f32| (-t / decay).exp();
    vec![
        // kick: pitch sweep 150 -> 45 Hz
        make(0.5, &|t, _| (std::f32::consts::TAU * (45.0 * t + 105.0 * 0.04 * (1.0 - (-t / 0.04).exp()))).sin() * env(t, 0.18)),
        make(0.3, &|t, n| (0.6 * n.next() + 0.4 * sine(190.0, t)) * env(t, 0.07)),
        make(0.3, &|t, n| n.next() * (env(t, 0.02) + 0.6 * env((t - 0.012).max(0.0), 0.05))),
        make(0.08, &|t, n| n.next() * env(t, 0.015) * 0.6),
        make(0.4, &|t, n| n.next() * env(t, 0.15) * 0.5),
        make(0.5, &|t, _| sine(110.0 * (1.0 + env(t, 0.1)), t) * env(t, 0.2)),
        make(0.08, &|t, n| (sine(1700.0, t) + 0.5 * n.next()) * env(t, 0.01)),
        make(1.5, &|t, n| n.next() * env(t, 0.6) * 0.6),
        make(0.6, &|t, _| (sine(55.0, t) + 0.3 * sine(110.0, t)) * env(t, 0.3)),
        make(0.4, &|t, _| (sine(440.0, t) + sine(554.4, t) + sine(659.3, t)) * 0.33 * env(t, 0.15)),
        make(1.0, &|t, _| ((sine(233.1, t) + 0.5 * sine(466.2, t)).signum() * 0.4) * (t * 20.0).min(1.0) * env(t, 0.6)),
        make(2.0, &|t, _| sine(600.0 + 300.0 * sine(1.5, t), t) * 0.6),
        make(2.0, &|t, n| n.next() * (t / 2.0) * 0.6),
        make(2.0, &|t, n| n.next() * (1.0 - t / 2.0) * 0.6),
        make(0.3, &|t, _| sine(2000.0 * env(t, 0.05) + 100.0, t) * env(t, 0.1)),
        make(0.6, &|t, _| sine(3000.0 - 4000.0 * t, t) * env(t, 0.25)),
    ]
}
