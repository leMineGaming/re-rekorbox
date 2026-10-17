//! DSP building blocks (own implementations of standard techniques): biquad filters (RBJ cookbook
//! formulas), a 3-band isolator EQ with Linkwitz-Riley crossovers, delay line, Schroeder reverb,
//! parameter smoothing, white noise, and the six Sound Color FX.

use std::f32::consts::PI;

use crate::pcm::Frame;

/// One-pole smoothing towards a target (avoids zipper noise on knob moves).
#[derive(Debug, Clone, Copy)]
pub struct Smooth {
    pub value: f32,
    coeff: f32,
}

impl Smooth {
    /// `time_s` ≈ time constant.
    pub fn new(value: f32, time_s: f32, sample_rate: u32) -> Smooth {
        Smooth { value, coeff: 1.0 - (-1.0 / (time_s * sample_rate as f32)).exp() }
    }

    pub fn next(&mut self, target: f32) -> f32 {
        self.value += (target - self.value) * self.coeff;
        self.value
    }
}

/// Stereo biquad, transposed direct form II.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
}

impl Default for Biquad {
    fn default() -> Self {
        Biquad { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0, z1: [0.0; 2], z2: [0.0; 2] }
    }
}

/// Filter shapes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    LowPass,
    HighPass,
    BandPass,
    Peak(f32),
    LowShelf(f32),
    HighShelf(f32),
}

impl Biquad {
    pub fn new(shape: Shape, fs: f32, f: f32, q: f32) -> Biquad {
        let mut b = Biquad::default();
        b.set(shape, fs, f, q);
        b
    }

    /// Change coefficients, keeping the state (for swept filters).
    pub fn set(&mut self, shape: Shape, fs: f32, f: f32, q: f32) {
        let f = f.clamp(10.0, fs * 0.49);
        let w = 2.0 * PI * f / fs;
        let (sw, cw) = w.sin_cos();
        let alpha = sw / (2.0 * q.max(0.05));
        let (b0, b1, b2, a0, a1, a2) = match shape {
            Shape::LowPass => ((1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::HighPass => ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
            Shape::Peak(db) => {
                let a = 10f32.powf(db / 40.0);
                (1.0 + alpha * a, -2.0 * cw, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cw, 1.0 - alpha / a)
            }
            Shape::LowShelf(db) | Shape::HighShelf(db) => {
                let a = 10f32.powf(db / 40.0);
                let s = 2.0 * a.sqrt() * alpha;
                let sign = if matches!(shape, Shape::LowShelf(_)) { 1.0 } else { -1.0 };
                (
                    a * ((a + 1.0) - sign * (a - 1.0) * cw + s),
                    sign * 2.0 * a * ((a - 1.0) - sign * (a + 1.0) * cw),
                    a * ((a + 1.0) - sign * (a - 1.0) * cw - s),
                    (a + 1.0) + sign * (a - 1.0) * cw + s,
                    -sign * 2.0 * ((a - 1.0) + sign * (a + 1.0) * cw),
                    (a + 1.0) + sign * (a - 1.0) * cw - s,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }

    pub fn process(&mut self, x: Frame) -> Frame {
        let mut y = [0.0; 2];
        for c in 0..2 {
            y[c] = self.b0 * x[c] + self.z1[c];
            self.z1[c] = self.b1 * x[c] - self.a1 * y[c] + self.z2[c];
            self.z2[c] = self.b2 * x[c] - self.a2 * y[c];
        }
        y
    }

    pub fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}

/// 4th-order Linkwitz-Riley section (two Butterworth biquads, Q = 1/√2).
#[derive(Debug, Clone, Copy)]
struct Lr4([Biquad; 2]);

impl Lr4 {
    fn new(shape: Shape, fs: f32, f: f32) -> Lr4 {
        let b = Biquad::new(shape, fs, f, std::f32::consts::FRAC_1_SQRT_2);
        Lr4([b, b])
    }

    fn process(&mut self, x: Frame) -> Frame {
        let y = self.0[0].process(x);
        self.0[1].process(y)
    }
}

/// EQ knob position 0..1 → linear band gain: 0 = kill, 0.5 = 0 dB, 1 = +6 dB; below the centre
/// the gain falls to −26 dB at 0.05 and then to silence.
pub fn eq_gain(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v >= 0.5 {
        10f32.powf(6.0 * (v - 0.5) * 2.0 / 20.0)
    } else if v > 0.05 {
        10f32.powf(-26.0 * (0.5 - v) / 0.45 / 20.0)
    } else {
        10f32.powf(-26.0 / 20.0) * v / 0.05
    }
}

/// 3-band isolator EQ: low < 300 Hz < mid < 4 kHz < high.
#[derive(Debug, Clone)]
pub struct Eq3 {
    lp_low: Lr4,
    hp_low: Lr4,
    lp_high: Lr4,
    hp_high: Lr4,
    gains: [Smooth; 3],
    /// Knob positions 0..1 (low, mid, high).
    pub knobs: [f32; 3],
}

impl Eq3 {
    pub const LOW_HZ: f32 = 300.0;
    pub const HIGH_HZ: f32 = 4000.0;

    pub fn new(sample_rate: u32) -> Eq3 {
        let fs = sample_rate as f32;
        Eq3 {
            lp_low: Lr4::new(Shape::LowPass, fs, Self::LOW_HZ),
            hp_low: Lr4::new(Shape::HighPass, fs, Self::LOW_HZ),
            lp_high: Lr4::new(Shape::LowPass, fs, Self::HIGH_HZ),
            hp_high: Lr4::new(Shape::HighPass, fs, Self::HIGH_HZ),
            gains: [Smooth::new(1.0, 0.01, sample_rate); 3],
            knobs: [0.5; 3],
        }
    }

    pub fn is_flat(&self) -> bool {
        self.knobs.iter().all(|&k| (k - 0.5).abs() < 1e-4) && self.gains.iter().all(|g| (g.value - 1.0).abs() < 1e-4)
    }

    pub fn process(&mut self, x: Frame) -> Frame {
        let low = self.lp_low.process(x);
        let rest = self.hp_low.process(x);
        let mid = self.lp_high.process(rest);
        let high = self.hp_high.process(rest);
        let g: [f32; 3] = std::array::from_fn(|i| self.gains[i].next(eq_gain(self.knobs[i])));
        [
            low[0] * g[0] + mid[0] * g[1] + high[0] * g[2],
            low[1] * g[0] + mid[1] * g[1] + high[1] * g[2],
        ]
    }
}

/// Stereo delay line with fractional read.
#[derive(Debug, Clone)]
pub struct DelayLine {
    buf: Vec<Frame>,
    w: usize,
}

impl DelayLine {
    pub fn new(max_frames: usize) -> DelayLine {
        DelayLine { buf: vec![[0.0; 2]; max_frames.max(2)], w: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn write(&mut self, x: Frame) {
        self.buf[self.w] = x;
        self.w = (self.w + 1) % self.buf.len();
    }

    /// Sample written `delay` frames ago (linear interpolation; 1 = the last written frame).
    pub fn read(&self, delay: f32) -> Frame {
        let n = self.buf.len();
        let d = delay.clamp(1.0, (n - 1) as f32);
        let i = d.floor() as usize;
        let t = d - i as f32;
        let a = self.buf[(self.w + n - i) % n];
        let b = self.buf[(self.w + n - i - 1) % n];
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    pub fn clear(&mut self) {
        self.buf.fill([0.0; 2]);
    }
}

/// Small Schroeder reverb: 4 parallel damped combs and 2 series all-passes per channel.
#[derive(Debug, Clone)]
pub struct Reverb {
    combs: Vec<(Vec<f32>, usize, f32)>,
    allpass: Vec<(Vec<f32>, usize)>,
    /// Feedback of the combs (room size), 0..0.98.
    pub feedback: f32,
    /// High-frequency damping 0..1.
    pub damp: f32,
}

impl Reverb {
    pub fn new(sample_rate: u32) -> Reverb {
        let s = sample_rate as f32 / 44100.0;
        let mut combs = Vec::new();
        let mut allpass = Vec::new();
        for (c, spread) in [(0usize, 0usize), (1, 23)] {
            let _ = c;
            for len in [1116, 1188, 1277, 1356] {
                combs.push((vec![0.0; ((len + spread) as f32 * s) as usize], 0, 0.0));
            }
            for len in [556, 441] {
                allpass.push((vec![0.0; ((len + spread) as f32 * s) as usize], 0));
            }
        }
        Reverb { combs, allpass, feedback: 0.84, damp: 0.3 }
    }

    /// Wet signal only.
    pub fn process(&mut self, x: Frame) -> Frame {
        let mut out = [0.0; 2];
        for ch in 0..2 {
            let input = x[ch] * 0.25;
            let mut acc = 0.0;
            for (buf, pos, store) in &mut self.combs[ch * 4..ch * 4 + 4] {
                let y = buf[*pos];
                *store = y * (1.0 - self.damp) + *store * self.damp;
                buf[*pos] = input + *store * self.feedback;
                *pos = (*pos + 1) % buf.len();
                acc += y;
            }
            for (buf, pos) in &mut self.allpass[ch * 2..ch * 2 + 2] {
                let b = buf[*pos];
                buf[*pos] = acc + b * 0.5;
                acc = b - acc;
                *pos = (*pos + 1) % buf.len();
            }
            out[ch] = acc;
        }
        out
    }

    pub fn clear(&mut self) {
        for (b, _, s) in &mut self.combs {
            b.fill(0.0);
            *s = 0.0;
        }
        for (b, _) in &mut self.allpass {
            b.fill(0.0);
        }
    }
}

/// Deterministic white noise (xorshift), −1..1.
#[derive(Debug, Clone, Copy)]
pub struct Noise(u32);

impl Default for Noise {
    fn default() -> Self {
        Noise(0x1234_5678)
    }
}

impl Noise {
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Sound Color FX types (XDJ-RX3 set).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorFx {
    Filter,
    Space,
    DubEcho,
    Sweep,
    Noise,
    Crush,
}

impl ColorFx {
    pub const ALL: [ColorFx; 6] =
        [ColorFx::Filter, ColorFx::Space, ColorFx::DubEcho, ColorFx::Sweep, ColorFx::Noise, ColorFx::Crush];
}

/// Per-channel Sound Color FX processor. Knob 0..1 with 0.5 = off; `sub` (0..1) is the shared
/// sub-parameter knob (resonance / feedback / character depending on the type).
#[derive(Debug, Clone)]
pub struct ColorProcessor {
    fs: f32,
    filter: Biquad,
    filter2: Biquad,
    delay: DelayLine,
    reverb: Reverb,
    noise: Noise,
    crush_hold: Frame,
    crush_count: f32,
    knob: Smooth,
    /// Delay time for DUB ECHO in seconds (set from the deck tempo).
    pub echo_s: f32,
}

impl ColorProcessor {
    pub fn new(sample_rate: u32) -> ColorProcessor {
        ColorProcessor {
            fs: sample_rate as f32,
            filter: Biquad::default(),
            filter2: Biquad::default(),
            delay: DelayLine::new(sample_rate as usize * 2),
            reverb: Reverb::new(sample_rate),
            noise: Noise::default(),
            crush_hold: [0.0; 2],
            crush_count: 0.0,
            knob: Smooth::new(0.5, 0.01, sample_rate),
            echo_s: 0.375,
        }
    }

    /// Cutoff for a filter knob side: amount 0..1 → frequency (log).
    fn sweep(from: f32, to: f32, amount: f32) -> f32 {
        from * (to / from).powf(amount.clamp(0.0, 1.0))
    }

    pub fn process(&mut self, fx: Option<ColorFx>, knob: f32, sub: f32, x: Frame) -> Frame {
        let k = self.knob.next(knob.clamp(0.0, 1.0));
        let Some(fx) = fx else { return x };
        let dev = k - 0.5; // −0.5 .. 0.5
        let amount = (dev.abs() * 2.0).min(1.0);
        if amount < 0.01 && !matches!(fx, ColorFx::Space | ColorFx::DubEcho) {
            return x;
        }
        let q = 0.7 + sub * 6.0;
        match fx {
            ColorFx::Filter => {
                if dev < 0.0 {
                    self.filter.set(Shape::LowPass, self.fs, Self::sweep(20000.0, 120.0, amount), q);
                } else {
                    self.filter.set(Shape::HighPass, self.fs, Self::sweep(20.0, 8000.0, amount), q);
                }
                self.filter.process(x)
            }
            ColorFx::Space => {
                self.reverb.feedback = 0.75 + 0.22 * sub;
                let wet = self.reverb.process(x);
                let shaped = if dev < 0.0 {
                    self.filter.set(Shape::LowPass, self.fs, Self::sweep(12000.0, 800.0, amount), 0.7);
                    self.filter.process(wet)
                } else {
                    self.filter.set(Shape::HighPass, self.fs, Self::sweep(100.0, 2500.0, amount), 0.7);
                    self.filter.process(wet)
                };
                [x[0] + shaped[0] * amount, x[1] + shaped[1] * amount]
            }
            ColorFx::DubEcho => {
                let d = (self.echo_s * self.fs).min(self.delay.len() as f32 - 2.0);
                let echo = self.delay.read(d);
                // filtered feedback: darker below centre, thinner above
                let shaped = if dev < 0.0 {
                    self.filter.set(Shape::LowPass, self.fs, 1800.0, 0.7);
                    self.filter.process(echo)
                } else {
                    self.filter.set(Shape::HighPass, self.fs, 600.0, 0.7);
                    self.filter.process(echo)
                };
                let fb = 0.35 + 0.5 * sub;
                self.delay.write([x[0] * amount + shaped[0] * fb, x[1] * amount + shaped[1] * fb]);
                [x[0] + shaped[0], x[1] + shaped[1]]
            }
            ColorFx::Sweep => {
                // below centre: band-reject sweeping down, above: band-pass sweeping up
                let f = if dev < 0.0 { Self::sweep(4000.0, 150.0, amount) } else { Self::sweep(150.0, 6000.0, amount) };
                self.filter.set(Shape::BandPass, self.fs, f, 1.0 + sub * 4.0);
                let band = self.filter.process(x);
                if dev < 0.0 {
                    [x[0] - band[0] * amount, x[1] - band[1] * amount]
                } else {
                    [x[0] * (1.0 - amount) + band[0] * amount * 2.0, x[1] * (1.0 - amount) + band[1] * amount * 2.0]
                }
            }
            ColorFx::Noise => {
                let n = self.noise.next() * 0.3 * amount;
                if dev < 0.0 {
                    self.filter.set(Shape::LowPass, self.fs, Self::sweep(12000.0, 300.0, amount), q);
                } else {
                    self.filter.set(Shape::HighPass, self.fs, Self::sweep(300.0, 9000.0, amount), q);
                }
                let shaped = self.filter.process([n, n]);
                [x[0] + shaped[0], x[1] + shaped[1]]
            }
            ColorFx::Crush => {
                // sample-rate reduction with the knob, bit depth with the sub-parameter
                let hold = 1.0 + amount * 30.0;
                self.crush_count += 1.0;
                if self.crush_count >= hold {
                    self.crush_count -= hold;
                    let levels = 2f32.powf(16.0 - (6.0 + sub * 6.0) * amount);
                    self.crush_hold = [(x[0] * levels).round() / levels, (x[1] * levels).round() / levels];
                }
                let y = self.crush_hold;
                if dev < 0.0 {
                    self.filter2.set(Shape::LowPass, self.fs, Self::sweep(16000.0, 2000.0, amount), 0.7);
                    self.filter2.process(y)
                } else {
                    y
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.filter.reset();
        self.filter2.reset();
        self.delay.clear();
        self.reverb.clear();
    }
}
