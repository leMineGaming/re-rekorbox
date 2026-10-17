//! Beat FX unit (XDJ-RX3 set of 14 effects) and Pad FX presets. Own implementations.
//!
//! An effect runs at a beat fraction of the current tempo (auto from the assigned deck, or tapped).
//! `depth` (the FX knob, 0..1) sets the wet amount / intensity. Time-based effects keep their tail
//! after being switched off.

use crate::dsp::{Biquad, DelayLine, PitchShifter, Reverb, Shape, Smooth};
use crate::pcm::Frame;

/// Beat fractions selectable with BEAT ◀/▶.
pub const BEATS: [f64; 10] = [1.0 / 16.0, 1.0 / 8.0, 1.0 / 4.0, 1.0 / 2.0, 3.0 / 4.0, 1.0, 2.0, 4.0, 8.0, 16.0];
/// Index of 1 beat in [`BEATS`].
pub const BEAT_ONE: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatFxKind {
    Delay,
    Echo,
    PingPong,
    Spiral,
    Helix,
    Reverb,
    Flanger,
    Phaser,
    Filter,
    Trans,
    Roll,
    SlipRoll,
    Pitch,
    VinylBrake,
}

impl BeatFxKind {
    pub const ALL: [BeatFxKind; 14] = [
        BeatFxKind::Delay,
        BeatFxKind::Echo,
        BeatFxKind::PingPong,
        BeatFxKind::Spiral,
        BeatFxKind::Helix,
        BeatFxKind::Reverb,
        BeatFxKind::Flanger,
        BeatFxKind::Phaser,
        BeatFxKind::Filter,
        BeatFxKind::Trans,
        BeatFxKind::Roll,
        BeatFxKind::SlipRoll,
        BeatFxKind::Pitch,
        BeatFxKind::VinylBrake,
    ];

    /// Name as used in the mapping file functions (`DELAY`, `ECHO`, …).
    pub fn from_name(s: &str) -> Option<BeatFxKind> {
        Some(match s {
            "DELAY" => BeatFxKind::Delay,
            "ECHO" => BeatFxKind::Echo,
            "PINGPONG" => BeatFxKind::PingPong,
            "SPIRAL" => BeatFxKind::Spiral,
            "HELIX" => BeatFxKind::Helix,
            "REVERB" => BeatFxKind::Reverb,
            "FLANGER" => BeatFxKind::Flanger,
            "PHASER" => BeatFxKind::Phaser,
            "FILTER" => BeatFxKind::Filter,
            "TRANS" => BeatFxKind::Trans,
            "ROLL" => BeatFxKind::Roll,
            "SLIPROLL" => BeatFxKind::SlipRoll,
            "PITCH" => BeatFxKind::Pitch,
            "VINYLBRAKE" => BeatFxKind::VinylBrake,
            _ => return None,
        })
    }

    pub fn next(self) -> BeatFxKind {
        let i = Self::ALL.iter().position(|&k| k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Where the Beat FX is inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxAssign {
    /// Mixer channel (0-based; only channels with a deck have audio).
    Channel(usize),
    Master,
    Sampler,
}

/// One effect processor.
#[derive(Debug, Clone)]
pub struct BeatFx {
    fs: f32,
    pub kind: BeatFxKind,
    pub on: bool,
    /// FX knob 0..1.
    pub depth: f32,
    /// Index into [`BEATS`].
    pub beat_index: usize,
    /// Fine time offset in ms (ADJUST TIME).
    pub adjust_ms: f32,
    line: DelayLine,
    reverb: Reverb,
    filters: [Biquad; 6],
    pitch: PitchShifter,
    lfo: f64,
    /// Roll / brake capture.
    capture: Vec<Frame>,
    cap_pos: f64,
    cap_len: usize,
    capturing: bool,
    brake_speed: f64,
    wet: Smooth,
    was_on: bool,
}

impl BeatFx {
    pub fn new(sample_rate: u32, kind: BeatFxKind) -> BeatFx {
        BeatFx {
            fs: sample_rate as f32,
            kind,
            on: false,
            depth: 0.5,
            beat_index: BEAT_ONE,
            adjust_ms: 0.0,
            line: DelayLine::new(sample_rate as usize * 9),
            reverb: Reverb::new(sample_rate),
            filters: [Biquad::default(); 6],
            pitch: PitchShifter::new(sample_rate),
            lfo: 0.0,
            capture: vec![[0.0; 2]; sample_rate as usize * 8],
            cap_pos: 0.0,
            cap_len: 0,
            capturing: false,
            brake_speed: 1.0,
            wet: Smooth::new(0.0, 0.01, sample_rate),
            was_on: false,
        }
    }

    pub fn beats(&self) -> f64 {
        BEATS[self.beat_index.min(BEATS.len() - 1)]
    }

    /// Effect period in samples for a tempo.
    fn period(&self, bpm: f64) -> f32 {
        let s = 60.0 / bpm.max(20.0) * self.beats() + f64::from(self.adjust_ms) / 1000.0;
        (s.max(0.005) * f64::from(self.fs)) as f32
    }

    /// True while the effect still produces sound (on, or a tail is decaying).
    pub fn active(&self) -> bool {
        self.on || self.wet.value > 1e-4
    }

    pub fn reset(&mut self) {
        self.line.clear();
        self.reverb.clear();
        self.pitch.reset();
        for f in &mut self.filters {
            f.reset();
        }
        self.capturing = false;
        self.cap_len = 0;
    }

    /// Process one frame at tempo `bpm`.
    pub fn process(&mut self, x: Frame, bpm: f64) -> Frame {
        let started = self.on && !self.was_on;
        self.was_on = self.on;
        let period = self.period(bpm);
        let d = self.depth.clamp(0.0, 1.0);
        let on = if self.on { 1.0 } else { 0.0 };
        // input to the effect is gated by on/off; the wet level fades so tails stay audible
        let send = [x[0] * on, x[1] * on];
        let tail_kinds = matches!(
            self.kind,
            BeatFxKind::Delay | BeatFxKind::Echo | BeatFxKind::PingPong | BeatFxKind::Spiral | BeatFxKind::Reverb
        );
        let wet = self.wet.next(if self.on || tail_kinds { 1.0 } else { 0.0 });
        if !self.active() && !tail_kinds {
            return x;
        }
        let lfo_step = 1.0 / f64::from(period.max(1.0));
        self.lfo = (self.lfo + lfo_step).fract();
        let lfo = self.lfo as f32;
        match self.kind {
            BeatFxKind::Delay | BeatFxKind::Echo | BeatFxKind::Spiral => {
                let mut dl = period;
                if self.kind == BeatFxKind::Spiral {
                    // delay time slowly shrinks while feedback stays high: rising spiral
                    dl *= 1.0 - 0.15 * lfo;
                }
                let echo = self.line.read(dl.min(self.line.len() as f32 - 2.0));
                let fb = match self.kind {
                    BeatFxKind::Delay => 0.25,
                    BeatFxKind::Echo => 0.55,
                    _ => 0.8,
                };
                let shaped = self.filters[0].process(echo);
                self.filters[0].set(Shape::LowPass, self.fs, 7000.0, 0.7);
                self.line.write([send[0] + shaped[0] * fb, send[1] + shaped[1] * fb]);
                [x[0] + shaped[0] * d * wet, x[1] + shaped[1] * d * wet]
            }
            BeatFxKind::PingPong => {
                let echo = self.line.read(period.min(self.line.len() as f32 - 2.0));
                // cross-feedback: left repeats come back on the right and vice versa
                self.line.write([send[0] * 0.5 + send[1] * 0.5 + echo[1] * 0.6, echo[0] * 0.6]);
                [x[0] + echo[0] * d * wet, x[1] + echo[1] * d * wet]
            }
            BeatFxKind::Helix => {
                // capture one period, then loop it with the input mixed by (1 - depth)
                if started {
                    self.cap_len = 0;
                    self.capturing = true;
                    self.cap_pos = 0.0;
                }
                self.loop_capture(x, period as usize, d, wet, 1.0)
            }
            BeatFxKind::Reverb => {
                self.reverb.feedback = 0.7 + 0.28 * d;
                let r = self.reverb.process(send);
                [x[0] + r[0] * d * 1.5 * wet, x[1] + r[1] * d * 1.5 * wet]
            }
            BeatFxKind::Flanger => {
                let delay = self.fs * (0.001 + 0.006 * (0.5 - 0.5 * (lfo * std::f32::consts::TAU).cos()));
                let y = self.line.read(delay);
                self.line.write([x[0] + y[0] * 0.5 * d, x[1] + y[1] * 0.5 * d]);
                [x[0] * (1.0 - 0.5 * d * wet) + y[0] * 0.5 * d * wet, x[1] * (1.0 - 0.5 * d * wet) + y[1] * 0.5 * d * wet]
            }
            BeatFxKind::Phaser => {
                let f = 300.0 * (5000.0f32 / 300.0).powf(0.5 - 0.5 * (lfo * std::f32::consts::TAU).cos());
                let mut y = x;
                for flt in self.filters.iter_mut().take(4) {
                    flt.set(Shape::Peak(-18.0), self.fs, f, 2.0);
                    y = flt.process(y);
                }
                let m = d * wet;
                [x[0] * (1.0 - m) + y[0] * m, x[1] * (1.0 - m) + y[1] * m]
            }
            BeatFxKind::Filter => {
                let f = 200.0 * (8000.0f32 / 200.0).powf(0.5 - 0.5 * (lfo * std::f32::consts::TAU).cos());
                self.filters[0].set(Shape::LowPass, self.fs, f, 1.0 + 4.0 * d);
                let y = self.filters[0].process(x);
                let m = wet;
                [x[0] * (1.0 - m) + y[0] * m, x[1] * (1.0 - m) + y[1] * m]
            }
            BeatFxKind::Trans => {
                // gate closed during the second half of each period
                let closed = lfo > 0.5;
                let g = if closed { 1.0 - d * wet } else { 1.0 };
                [x[0] * g, x[1] * g]
            }
            BeatFxKind::Roll | BeatFxKind::SlipRoll => {
                if started {
                    self.cap_len = 0;
                    self.capturing = true;
                    self.cap_pos = 0.0;
                }
                self.loop_capture(x, period as usize, d.max(0.5), wet, 1.0)
            }
            BeatFxKind::Pitch => {
                // knob: 0 = −12, 0.5 = 0, 1 = +12 semitones
                let semis = (d - 0.5) * 24.0;
                let y = self.pitch.process(x, 2f32.powf(semis / 12.0));
                [x[0] * (1.0 - wet) + y[0] * wet, x[1] * (1.0 - wet) + y[1] * wet]
            }
            BeatFxKind::VinylBrake => {
                if started {
                    self.cap_len = 0;
                    self.capturing = true;
                    self.cap_pos = 0.0;
                    self.brake_speed = 1.0;
                }
                if !self.on {
                    return x;
                }
                // record live input, read it back with decreasing speed over one period
                if self.cap_len < self.capture.len() {
                    self.capture[self.cap_len] = x;
                    self.cap_len += 1;
                }
                self.brake_speed = (self.brake_speed - 1.0 / f64::from(period.max(1.0))).max(0.0);
                let i = self.cap_pos as usize;
                let y = if i < self.cap_len { self.capture[i] } else { [0.0; 2] };
                self.cap_pos += self.brake_speed;
                let m = d.max(0.3) * wet;
                [x[0] * (1.0 - m) + y[0] * m, x[1] * (1.0 - m) + y[1] * m]
            }
        }
    }

    /// Shared by ROLL / SLIP ROLL / HELIX: capture `len` frames, then repeat them.
    fn loop_capture(&mut self, x: Frame, len: usize, mix: f32, wet: f32, speed: f64) -> Frame {
        if !self.on {
            self.capturing = false;
            return x;
        }
        let len = len.clamp(16, self.capture.len());
        if self.capturing {
            self.capture[self.cap_len] = x;
            self.cap_len += 1;
            if self.cap_len >= len {
                self.capturing = false;
            }
            return x;
        }
        let i = (self.cap_pos as usize) % self.cap_len.max(1);
        let y = self.capture[i];
        self.cap_pos = (self.cap_pos + speed) % self.cap_len.max(1) as f64;
        let m = mix * wet;
        [x[0] * (1.0 - m) + y[0] * m, x[1] * (1.0 - m) + y[1] * m]
    }
}

/// Pad FX preset: effect, beat fraction index, depth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadFxPreset {
    pub kind: BeatFxKind,
    pub beat_index: usize,
    pub depth: f32,
}

const fn p(kind: BeatFxKind, beat_index: usize, depth: f32) -> PadFxPreset {
    PadFxPreset { kind, beat_index, depth }
}

/// Pad FX banks (own presets): bank 0 = PAD FX1, bank 1 = PAD FX2; 16 slots each.
pub const PAD_FX: [[PadFxPreset; 16]; 2] = [
    [
        p(BeatFxKind::Roll, 2, 1.0),
        p(BeatFxKind::Roll, 1, 1.0),
        p(BeatFxKind::Roll, 0, 1.0),
        p(BeatFxKind::Trans, 2, 1.0),
        p(BeatFxKind::Echo, 3, 0.7),
        p(BeatFxKind::Echo, 5, 0.7),
        p(BeatFxKind::Filter, 6, 0.6),
        p(BeatFxKind::Reverb, 5, 0.8),
        p(BeatFxKind::Delay, 4, 0.7),
        p(BeatFxKind::PingPong, 3, 0.7),
        p(BeatFxKind::Spiral, 3, 0.7),
        p(BeatFxKind::Helix, 2, 0.8),
        p(BeatFxKind::Flanger, 7, 0.8),
        p(BeatFxKind::Phaser, 7, 0.8),
        p(BeatFxKind::Pitch, 5, 0.75),
        p(BeatFxKind::VinylBrake, 5, 1.0),
    ],
    [
        p(BeatFxKind::Roll, 3, 1.0),
        p(BeatFxKind::Roll, 5, 1.0),
        p(BeatFxKind::SlipRoll, 2, 1.0),
        p(BeatFxKind::Trans, 1, 1.0),
        p(BeatFxKind::Echo, 4, 0.8),
        p(BeatFxKind::Echo, 6, 0.8),
        p(BeatFxKind::Filter, 7, 0.9),
        p(BeatFxKind::Reverb, 6, 1.0),
        p(BeatFxKind::Delay, 5, 0.6),
        p(BeatFxKind::PingPong, 4, 0.8),
        p(BeatFxKind::Spiral, 5, 0.8),
        p(BeatFxKind::Helix, 3, 0.9),
        p(BeatFxKind::Flanger, 6, 1.0),
        p(BeatFxKind::Phaser, 6, 1.0),
        p(BeatFxKind::Pitch, 5, 0.25),
        p(BeatFxKind::VinylBrake, 6, 1.0),
    ],
];

/// Tap tempo from the last taps (seconds); taps older than 2 s restart the measurement.
#[derive(Debug, Clone, Default)]
pub struct TapTempo {
    taps: Vec<f64>,
}

impl TapTempo {
    /// Register a tap at time `t`; returns the BPM once at least two taps are known.
    pub fn tap(&mut self, t: f64) -> Option<f64> {
        if self.taps.last().is_some_and(|&l| t - l > 2.0 || t <= l) {
            self.taps.clear();
        }
        self.taps.push(t);
        if self.taps.len() > 5 {
            self.taps.remove(0);
        }
        if self.taps.len() < 2 {
            return None;
        }
        let span = self.taps.last().unwrap() - self.taps[0];
        Some(60.0 * (self.taps.len() - 1) as f64 / span)
    }
}
