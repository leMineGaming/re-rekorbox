//! Tempo and beat grid detection (own implementation of public methods, SPEC §8a):
//!
//! 1. onset strength: spectral flux of a log-compressed STFT (2048-point frames, 100 frames/s),
//!    full band and below 150 Hz;
//! 2. coarse tempo: autocorrelation of the onset envelope with a log-normal tempo prior;
//! 3. fine tempo and phase: comb alignment over the whole track (0.01 BPM steps, 1 ms phase steps),
//!    integer BPM preferred when it scores (almost) as well;
//! 4. beat vs offbeat and downbeat: linear decisions on band onset features, fitted against
//!    observed rekordbox grids (`fit_phase`, `fit_downbeat` examples).
//!
//! The result is a constant grid like rekordbox's normal analysis (BPM range 70–180, observed).

use rekordbox_formats::anlz::tags::Beat;

use crate::pcm::Pcm;

/// Onset envelope frames per second.
pub const FPS: f64 = 100.0;
const FRAME: usize = 2048;
/// BPM range of the grid (as observed in rekordbox analyses).
pub const BPM_MIN: f64 = 70.0;
pub const BPM_MAX: f64 = 180.0;
/// Constant latency of the onset envelope relative to the audible beat, seconds (calibrated).
pub const ONSET_OFFSET_S: f64 = -0.006;

/// In-place radix-2 FFT (`re`, `im` length a power of two).
pub fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

/// Onset strength envelopes at [`FPS`] (frame `i` centred at `i / FPS` s), each normalised.
#[derive(Debug, Clone, Default)]
pub struct Onsets {
    pub full: Vec<f32>,
    /// Below 150 Hz.
    pub low: Vec<f32>,
    /// 150–2000 Hz.
    pub mid: Vec<f32>,
    /// Above 2000 Hz.
    pub high: Vec<f32>,
    /// Chroma (12 pitch classes, 80–2000 Hz, linear magnitude, frame-normalised) per frame.
    pub chroma: Vec<[f32; 12]>,
}

/// Full-band and low-band envelopes (kept for callers that need only these two).
pub fn onset_envelopes(pcm: &Pcm) -> (Vec<f32>, Vec<f32>) {
    let o = onsets(pcm);
    (o.full, o.low)
}

/// Onset envelopes in four bands.
pub fn onsets(pcm: &Pcm) -> Onsets {
    let sr = f64::from(pcm.sample_rate.max(1));
    let hop = sr / FPS;
    let frames = (pcm.len() as f64 / hop) as usize;
    let window: Vec<f32> = (0..FRAME).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / FRAME as f32).cos()).collect();
    let low_bins = ((150.0 / sr) * FRAME as f64).ceil() as usize;
    let mid_bins = ((2000.0 / sr) * FRAME as f64).ceil() as usize;
    let bins = FRAME / 2;
    let mut prev = vec![0.0f32; bins];
    let mut full = Vec::with_capacity(frames);
    let mut low = Vec::with_capacity(frames);
    let mut mid = Vec::with_capacity(frames);
    let mut high = Vec::with_capacity(frames);
    let mut chroma = Vec::with_capacity(frames);
    // pitch class of every FFT bin between 80 and 5000 Hz (A4 = 440 Hz → class 9)
    let pc: Vec<Option<usize>> = (0..FRAME / 2)
        .map(|b| {
            let f = b as f64 * sr / FRAME as f64;
            (80.0..2000.0).contains(&f).then(|| ((12.0 * (f / 440.0).log2()).round() as i64 + 9).rem_euclid(12) as usize)
        })
        .collect();
    let (mut re, mut im) = (vec![0.0f32; FRAME], vec![0.0f32; FRAME]);
    for i in 0..frames {
        let centre = (i as f64 * hop) as i64;
        for k in 0..FRAME {
            let idx = centre - (FRAME / 2) as i64 + k as i64;
            let s = if idx >= 0 && (idx as usize) < pcm.len() {
                let f = pcm.frames[idx as usize];
                0.5 * (f[0] + f[1])
            } else {
                0.0
            };
            re[k] = s * window[k];
            im[k] = 0.0;
        }
        fft(&mut re, &mut im);
        let (mut flux, mut lflux, mut mflux, mut hflux) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut ch = [0.0f32; 12];
        for b in 1..bins {
            let lin = (re[b] * re[b] + im[b] * im[b]).sqrt();
            let mag = (1.0 + 100.0 * lin).ln();
            if let Some(c) = pc[b] {
                ch[c] += lin;
            }
            let d = mag - prev[b];
            if d > 0.0 {
                flux += d;
                if b <= low_bins {
                    lflux += d;
                } else if b <= mid_bins {
                    mflux += d;
                } else {
                    hflux += d;
                }
            }
            prev[b] = mag;
        }
        full.push(flux);
        low.push(lflux);
        mid.push(mflux);
        high.push(hflux);
        let norm = ch.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-9);
        chroma.push(ch.map(|v| v / norm));
    }
    Onsets { full: normalise(&full), low: normalise(&low), mid: normalise(&mid), high: normalise(&high), chroma }
}

/// Subtract a moving average (≈ 0.5 s), half-wave rectify, scale to unit maximum.
fn normalise(x: &[f32]) -> Vec<f32> {
    let w = 25usize;
    let mut out = Vec::with_capacity(x.len());
    let mut sum = 0.0f32;
    let mut q = std::collections::VecDeque::new();
    for i in 0..x.len() + w {
        if i < x.len() {
            q.push_back(x[i]);
            sum += x[i];
        }
        if q.len() > 2 * w + 1 || (i >= x.len() && !q.is_empty()) {
            sum -= q.pop_front().unwrap();
        }
        if i >= w {
            let c = i - w;
            let mean = sum / q.len().max(1) as f32;
            out.push((x[c] - mean).max(0.0));
        }
    }
    out.truncate(x.len());
    let m = out.iter().copied().fold(0.0f32, f32::max).max(1e-9);
    out.iter().map(|v| v / m).collect()
}

/// Linear interpolation of the envelope at time `t` seconds.
fn env_at(env: &[f32], t: f64) -> f32 {
    let x = t * FPS;
    if x < 0.0 {
        return 0.0;
    }
    let i = x.floor() as usize;
    if i + 1 >= env.len() {
        return 0.0;
    }
    let f = (x - i as f64) as f32;
    env[i] * (1.0 - f) + env[i + 1] * f
}

/// Log-normal tempo prior around 128 BPM.
fn prior(bpm: f64) -> f64 {
    let z = (bpm / 128.0).log2() / 0.55;
    (-0.5 * z * z).exp()
}

/// Coarse tempo candidates from the autocorrelation (best first).
pub fn coarse_tempi(env: &[f32]) -> Vec<f64> {
    let lag_min = (60.0 * FPS / BPM_MAX * 0.5).floor() as usize;
    let lag_max = (60.0 * FPS / BPM_MIN * 2.0).ceil() as usize;
    let n = env.len();
    let mut acf: Vec<(usize, f64)> = (lag_min..=lag_max.min(n.saturating_sub(1)))
        .map(|lag| {
            let s: f64 = (0..n - lag).map(|i| f64::from(env[i]) * f64::from(env[i + lag])).sum();
            (lag, s / (n - lag) as f64)
        })
        .collect();
    // local maxima, weighted with the prior, folded into the BPM range
    let mut cands: Vec<(f64, f64)> = Vec::new();
    for w in 1..acf.len().saturating_sub(1) {
        let (lag, v) = acf[w];
        if v >= acf[w - 1].1 && v >= acf[w + 1].1 {
            let mut bpm = 60.0 * FPS / lag as f64;
            while bpm < BPM_MIN {
                bpm *= 2.0;
            }
            while bpm > BPM_MAX {
                bpm /= 2.0;
            }
            cands.push((bpm, v * prior(bpm)));
        }
    }
    acf.clear();
    cands.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out: Vec<f64> = Vec::new();
    for (b, _) in cands {
        if out.iter().all(|o| (o / b - 1.0).abs() > 0.02) {
            out.push(b);
        }
        if out.len() >= 4 {
            break;
        }
    }
    out
}

/// Comb score of a grid: mean envelope at the beats, best phase (seconds) returned too.
fn comb(env: &[f32], bpm: f64, duration: f64, phase_step: f64) -> (f64, f64) {
    let beat = 60.0 / bpm;
    let n = (duration / beat) as usize;
    let mut best = (f64::MIN, 0.0);
    let mut ph = 0.0;
    while ph < beat {
        let mut s = 0.0f64;
        for k in 0..n {
            s += f64::from(env_at(env, ph + k as f64 * beat));
        }
        let s = s / n.max(1) as f64;
        if s > best.0 {
            best = (s, ph);
        }
        ph += phase_step;
    }
    best
}

/// Mean envelope at the positions `phase + k · beat`.
fn mean_at(env: &[f32], phase: f64, beat: f64, duration: f64) -> f64 {
    let n = (duration / beat) as usize;
    (0..n).map(|k| f64::from(env_at(env, phase + k as f64 * beat))).sum::<f64>() / n.max(1) as f64
}

/// Features comparing the candidate phase `ph` with `ph + ½ beat` (positive = `ph` looks like the beat):
/// differences of mean full/low/mid/high onset strength and of the backbeat parity of the mid band.
pub fn phase_features(o: &Onsets, bpm: f64, ph: f64, duration: f64) -> [f64; 6] {
    let beat = 60.0 / bpm;
    let alt = ph + beat / 2.0;
    let d = |e: &[f32]| {
        let (a, b) = (mean_at(e, ph, beat, duration), mean_at(e, alt, beat, duration));
        (a - b) / (a + b).max(1e-9)
    };
    let parity = |e: &[f32], p: f64| {
        let n = (duration / beat) as usize;
        let (mut ev, mut od) = (0.0f64, 0.0f64);
        for k in 0..n {
            let v = f64::from(env_at(e, p + k as f64 * beat));
            if k % 2 == 0 {
                ev += v;
            } else {
                od += v;
            }
        }
        (ev - od).abs() / (ev + od).max(1e-9)
    };
    [d(&o.full), d(&o.low), d(&o.mid), d(&o.high), parity(&o.mid, ph) - parity(&o.mid, alt), 1.0]
}

/// Weights of the phase decision (fitted against observed rekordbox grids, see the `fit_phase`
/// example); `phase_features · PHASE_WEIGHTS > 0` keeps the candidate phase.
pub const PHASE_WEIGHTS: [f64; 6] = [-0.702, 0.303, 6.882, -1.46, -0.165, 0.543];

/// Features of the bar offset `k` (beats `phase + (k + 4m)·beat` are downbeats): share of the mean
/// full/low/mid/high onset strength on those beats relative to all beats, and the shares on the
/// beat before (fills / pickups).
pub fn downbeat_features(o: &Onsets, bpm: f64, phase: f64, duration: f64, k: usize) -> [f64; 8] {
    let beat = 60.0 / bpm;
    let n = (duration / beat) as usize;
    let share = |e: &[f32], off: usize| {
        let mut sums = [0.0f64; 4];
        for i in 0..n {
            sums[i % 4] += f64::from(env_at(e, phase + i as f64 * beat));
        }
        let tot: f64 = sums.iter().sum::<f64>().max(1e-9);
        sums[(k + off) % 4] / tot
    };
    // harmonic novelty at each beat: 1 − cosine(chroma of the 2 beats before, chroma of the 2 after)
    let mean_chroma = |t0: f64, t1: f64| {
        let (a, b) = (((t0 * FPS).max(0.0)) as usize, ((t1 * FPS).max(0.0) as usize).min(o.chroma.len()));
        let mut m = [0.0f32; 12];
        for c in o.chroma.get(a..b.max(a)).unwrap_or(&[]) {
            for (x, y) in m.iter_mut().zip(c) {
                *x += y;
            }
        }
        m
    };
    let cos = |a: [f32; 12], b: [f32; 12]| {
        let d: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        f64::from(d / (na * nb).max(1e-9))
    };
    let mut nov = [0.0f64; 4];
    let mut nov_bar = [0.0f64; 4];
    for i in 2..n.saturating_sub(4) {
        let t = phase + i as f64 * beat;
        nov[i % 4] += 1.0 - cos(mean_chroma(t - 2.0 * beat, t), mean_chroma(t, t + 2.0 * beat));
        nov_bar[i % 4] += 1.0 - cos(mean_chroma(t - 4.0 * beat, t), mean_chroma(t, t + 4.0 * beat));
    }
    let nshare = |v: &[f64; 4]| v[k % 4] / v.iter().sum::<f64>().max(1e-9);
    [
        share(&o.full, 0),
        share(&o.low, 0),
        share(&o.mid, 0),
        share(&o.high, 0),
        share(&o.full, 3),
        share(&o.low, 3),
        nshare(&nov),
        nshare(&nov_bar),
    ]
}

/// Weights of the downbeat choice (fitted against observed rekordbox grids, `fit_downbeat` example).
pub const DOWNBEAT_WEIGHTS: [f64; 8] = [0.154, 6.072, 1.242, -0.455, 3.42, -0.477, 11.57, 1.105];

/// Result of the tempo analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct GridResult {
    pub bpm: f64,
    /// Time of a downbeat (first beat of a bar) in seconds, within the first bar.
    pub first_downbeat_s: f64,
    /// Comb score (mean onset strength on the beats, 0..1) – a confidence measure.
    pub confidence: f64,
}

/// Detect tempo, phase and downbeat of a track.
pub fn detect_grid(pcm: &Pcm) -> Option<GridResult> {
    detect_grid_with(pcm, &onsets(pcm))
}

/// [`detect_grid`] with precomputed onset envelopes.
pub fn detect_grid_with(pcm: &Pcm, ons: &Onsets) -> Option<GridResult> {
    let (env, low) = (ons.full.clone(), ons.low.clone());
    let duration = pcm.duration_s();
    if duration < 5.0 {
        return None;
    }
    let cands = coarse_tempi(&env);
    if cands.is_empty() {
        return None;
    }
    // fine search around each candidate (±1.5 %, 0.01 BPM), coarse phase 10 ms
    let mut best: Option<(f64, f64, f64)> = None; // (score·prior, bpm, raw score)
    for c in &cands {
        let lo = (c * 0.985 * 100.0).round() as i64;
        let hi = (c * 1.015 * 100.0).round() as i64;
        for b100 in (lo..=hi).step_by(1) {
            let bpm = b100 as f64 / 100.0;
            if !(BPM_MIN..=BPM_MAX).contains(&bpm) {
                continue;
            }
            let (s, _) = comb(&env, bpm, duration, 0.01);
            let weighted = s * prior(bpm).powf(0.3);
            if best.is_none_or(|b| weighted > b.0) {
                best = Some((weighted, bpm, s));
            }
        }
    }
    let (_, mut bpm, mut score) = best?;
    // prefer the integer tempo when it is nearly as good (compared at a fine phase step, so that
    // very sharp onsets are not missed)
    let int = bpm.round();
    if (int - bpm).abs() <= 0.25 && (BPM_MIN..=BPM_MAX).contains(&int) {
        let (sb, _) = comb(&env, bpm, duration, 0.002);
        let (si, _) = comb(&env, int, duration, 0.002);
        if si >= sb * 0.985 {
            bpm = int;
            score = si;
        }
    }
    // octave: if kicks also sit on the half-beat positions, the tempo is twice as fast
    if bpm * 2.0 <= BPM_MAX {
        let (_, ph) = comb(&low, bpm, duration, 0.005);
        let beat = 60.0 / bpm;
        let on = mean_at(&low, ph, beat, duration);
        let half = mean_at(&low, ph + beat / 2.0, beat, duration);
        if half > 0.55 * on {
            bpm *= 2.0;
            score = comb(&env, bpm, duration, 0.01).0;
        }
    }
    // precise phase at 1 ms; then beat or offbeat by the fitted decision
    let beat = 60.0 / bpm;
    let (_, phase) = comb(&env, bpm, duration, 0.001);
    let f = phase_features(ons, bpm, phase, duration);
    let keep = f.iter().zip(PHASE_WEIGHTS).map(|(a, b)| a * b).sum::<f64>() > 0.0;
    let phase = if keep { phase } else { phase + beat / 2.0 };
    let phase = (phase - ONSET_OFFSET_S).rem_euclid(beat);
    // downbeat: bar offset with the best fitted score
    let score_k = |k: usize| {
        downbeat_features(ons, bpm, phase, duration, k).iter().zip(DOWNBEAT_WEIGHTS).map(|(a, b)| a * b).sum::<f64>()
    };
    let off = (0..4).max_by(|&a, &b| score_k(a).total_cmp(&score_k(b))).unwrap_or(0);
    Some(GridResult { bpm, first_downbeat_s: phase + off as f64 * beat, confidence: score })
}

/// Beat list (PQTZ entries) for a constant grid over `duration` seconds: from the first beat at or
/// after 0 to the end, beat-in-bar 1–4 counted from the downbeat.
pub fn grid_beats(g: &GridResult, duration: f64) -> Vec<Beat> {
    let beat = 60.0 / g.bpm;
    let tempo_x100 = (g.bpm * 100.0).round() as u16;
    let first_index = (-(g.first_downbeat_s) / beat).ceil() as i64;
    let mut out = Vec::new();
    let mut k = first_index;
    loop {
        let t = g.first_downbeat_s + k as f64 * beat;
        if t > duration {
            break;
        }
        if t >= 0.0 {
            out.push(Beat { beat: (k.rem_euclid(4) + 1) as u16, tempo_x100, time_ms: (t * 1000.0).round() as u32 });
        }
        k += 1;
    }
    out
}
