//! Waveform analysis (SPEC §5j "Wellenform-DSP"): mono mix, the analysis filter bank and the
//! per-millisecond records A/B/C, then 150-columns-per-second detail data and the previews.
//!
//! The per-millisecond stage follows the specification; the mapping of levels onto the 5-bit heights
//! and 3-bit colour values is calibrated against observed rekordbox analyses (see `calibrate`).

use crate::pcm::Pcm;

/// Second-order section, RBJ cookbook, Q = 1/√2 (mono, f64).
#[derive(Debug, Clone, Copy)]
struct Bq {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Bq {
    fn new(highpass: bool, f: f64, fs: f64) -> Bq {
        let w = 2.0 * std::f64::consts::PI * f / fs;
        let (s, c) = w.sin_cos();
        let alpha = s * std::f64::consts::FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        let (b0, b1) = if highpass { ((1.0 + c) * 0.5, -(1.0 + c)) } else { ((1.0 - c) * 0.5, 1.0 - c) };
        Bq { b0: b0 / a0, b1: b1 / a0, b2: b0 / a0, a1: -2.0 * c / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    fn run(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn chain(spec: &[(bool, f64)], fs: f64) -> Vec<Bq> {
    spec.iter().map(|&(hp, f)| Bq::new(hp, f, fs)).collect()
}

fn run_chain(c: &mut [Bq], x: f64) -> f64 {
    c.iter_mut().fold(x, |v, f| f.run(v))
}

const LP: bool = false;
const HP: bool = true;

/// Mono mix as specified: average, unless both magnitudes are within 0.001, then one channel.
pub fn mono(l: f32, r: f32) -> f64 {
    let (l, r) = (f64::from(l), f64::from(r));
    if (r.abs() - l.abs()).abs() >= 0.001 {
        (l + r) * 0.5
    } else if l.abs() < r.abs() {
        r
    } else {
        l
    }
}

/// i16 value of a level as stored in the records.
fn q(v: f64) -> i16 {
    if v >= 1.0 {
        0x7FFF
    } else if v <= -1.0 {
        i16::MIN
    } else {
        (v * 32768.0) as i16
    }
}

/// Per-millisecond analysis records.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MsRecords {
    /// max, min, max|LP400|, low·share, mid·share, high·share (bands 200 / 2000 Hz).
    pub a: Vec<[i16; 6]>,
    /// max, min, max|<100|, max|300–3000|, max|>1500|.
    pub b: Vec<[i16; 5]>,
    /// max|<300|, max|250–1200|, max|3000–9000|.
    pub c: Vec<[i16; 3]>,
}

/// Compute the per-millisecond records of a track.
pub fn ms_records(pcm: &Pcm) -> MsRecords {
    let fs = f64::from(pcm.sample_rate.max(1));
    let n = pcm.len();
    let ms = (n as u64 * 1000).div_ceil(u64::from(pcm.sample_rate.max(1))) as usize;
    let mut lp400 = chain(&[(LP, 400.0)], fs);
    let mut a_low = chain(&[(LP, 200.0), (LP, 200.0)], fs);
    let mut a_mid = chain(&[(HP, 200.0), (HP, 200.0), (LP, 2000.0), (LP, 2000.0)], fs);
    let mut a_high = chain(&[(HP, 2000.0), (HP, 2000.0)], fs);
    let mut b1 = chain(&[(LP, 100.0)], fs);
    let mut b2 = chain(&[(HP, 300.0), (LP, 3000.0)], fs);
    let mut b3 = chain(&[(HP, 1500.0)], fs);
    let mut c1 = chain(&[(LP, 300.0)], fs);
    let mut c2 = chain(&[(HP, 250.0), (LP, 1200.0)], fs);
    let mut c3 = chain(&[(HP, 3000.0), (LP, 9000.0)], fs);
    let mut out = MsRecords { a: Vec::with_capacity(ms), b: Vec::with_capacity(ms), c: Vec::with_capacity(ms) };
    let mut start = 0usize;
    for m in 0..ms {
        // samples of millisecond m: [ceil(m·fs/1000), ceil((m+1)·fs/1000))
        let end = (((m as u64 + 1) * u64::from(pcm.sample_rate)).div_ceil(1000) as usize).min(n);
        let (mut mx, mut mn) = (f64::MIN, f64::MAX);
        let (mut bass, mut lo, mut mid, mut hi) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let (mut cnt_lo, mut cnt_mid, mut cnt_hi) = (0u32, 0u32, 0u32);
        let (mut bb1, mut bb2, mut bb3, mut cc1, mut cc2, mut cc3) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for f in &pcm.frames[start..end] {
            let x = mono(f[0], f[1]);
            mx = mx.max(x);
            mn = mn.min(x);
            bass = bass.max(run_chain(&mut lp400, x).abs());
            let (l, md, h) = (run_chain(&mut a_low, x), run_chain(&mut a_mid, x), run_chain(&mut a_high, x));
            lo = lo.max(l.abs());
            mid = mid.max(md.abs());
            hi = hi.max(h.abs());
            // dominant band (signed comparison as in the original)
            let top = if md <= l { l.max(h) } else { md.max(h) };
            if top == l {
                cnt_lo += 1;
            } else if top == h {
                cnt_hi += 1;
            } else {
                cnt_mid += 1;
            }
            bb1 = bb1.max(run_chain(&mut b1, x).abs());
            bb2 = bb2.max(run_chain(&mut b2, x).abs());
            bb3 = bb3.max(run_chain(&mut b3, x).abs());
            cc1 = cc1.max(run_chain(&mut c1, x).abs());
            cc2 = cc2.max(run_chain(&mut c2, x).abs());
            cc3 = cc3.max(run_chain(&mut c3, x).abs());
        }
        let len = (end - start).max(1) as f64;
        if end <= start {
            mx = 0.0;
            mn = 0.0;
        }
        out.a.push([
            q(mx),
            q(mn),
            q(bass),
            q(lo * f64::from(cnt_lo) / len),
            q(mid * f64::from(cnt_mid) / len),
            q(hi * f64::from(cnt_hi) / len),
        ]);
        out.b.push([q(mx), q(mn), q(bb1), q(bb2), q(bb3)]);
        out.c.push([q(cc1), q(cc2), q(cc3)]);
        start = end;
    }
    out
}

/// Number of 150/s detail columns for a track (round(samples / rate · 150)).
pub fn detail_columns(samples: usize, rate: u32) -> usize {
    (samples as f64 / f64::from(rate.max(1)) * 150.0).round() as usize
}

/// Millisecond range `[start, end)` covered by detail column `i` (150 columns per second).
pub fn column_ms(i: usize) -> (usize, usize) {
    ((i as f64 * 1000.0 / 150.0).floor() as usize, ((i + 1) as f64 * 1000.0 / 150.0).floor() as usize)
}

/// Per-column maxima of the record fields, as floats 0..1 (absolute values), for columns at
/// `columns` per track: returns one array per column with
/// [peak, bass, a_low, a_mid, a_high, b1, b2, b3, c1, c2, c3].
pub fn column_features(r: &MsRecords, columns: usize) -> Vec<[f32; 11]> {
    let ms = r.a.len();
    (0..columns)
        .map(|i| {
            let (s, e) = column_ms(i);
            let (s, e) = (s.min(ms), e.clamp(s.min(ms) + 1, ms.max(1)));
            let mut f = [0.0f32; 11];
            for m in s..e.min(ms) {
                let a = r.a[m];
                let b = r.b[m];
                let c = r.c[m];
                let v = |x: i16| (f32::from(x) / 32768.0).abs();
                let vals = [
                    v(a[0]).max(v(a[1])),
                    v(a[2]),
                    v(a[3]),
                    v(a[4]),
                    v(a[5]),
                    v(b[2]),
                    v(b[3]),
                    v(b[4]),
                    v(c[0]),
                    v(c[1]),
                    v(c[2]),
                ];
                for (dst, x) in f.iter_mut().zip(vals) {
                    *dst = dst.max(x);
                }
            }
            f
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Quantisation (calibrated against observed rekordbox analyses, SPEC §5j / §8a)

use rekordbox_formats::anlz::waveform::{ColorColumn, MonoColumn};

fn clamp_round(v: f32, max: u8) -> u8 {
    v.round().clamp(0.0, f32::from(max)) as u8
}

/// 5-bit height of a detail column from its peak level.
pub fn detail_height(peak: f32) -> u8 {
    let x = ((peak - 0.12) / 0.88).max(0.0);
    clamp_round(31.0 * x.powf(1.6), 31)
}

/// All waveform tags of a track.
#[derive(Debug, Clone, PartialEq)]
pub struct Waveforms {
    /// `PWAV`, 400 columns.
    pub preview: Vec<MonoColumn>,
    /// `PWV2`, 100 columns (heights 0..15).
    pub tiny: Vec<MonoColumn>,
    /// `PWV3`, 150 columns per second.
    pub detail: Vec<MonoColumn>,
    /// `PWV5`, 150 columns per second.
    pub color: Vec<ColorColumn>,
    /// `PWV4`, 1200 entries.
    pub color_preview: Vec<[u8; 6]>,
    /// `PWV6`, 1200 entries.
    pub bands_preview: Vec<[u8; 3]>,
    /// `PWV7`, 150 columns per second.
    pub bands: Vec<[u8; 3]>,
    /// `PWVC` values (three u16).
    pub pwvc: [u16; 3],
}

/// Mean and max of a feature over buckets of `n` equal parts.
fn bucket<T>(len: usize, n: usize, mut f: impl FnMut(usize, usize) -> T) -> Vec<T> {
    (0..n)
        .map(|j| {
            let s = j * len / n;
            let e = ((j + 1) * len / n).max(s + 1).min(len.max(1));
            f(s.min(len.saturating_sub(1)), e)
        })
        .collect()
}

/// Build all waveform tags from the per-millisecond records of a track with `samples` frames.
pub fn waveforms(rec: &MsRecords, samples: usize, rate: u32) -> Waveforms {
    let cols = detail_columns(samples, rate);
    let feats = column_features(rec, cols);
    let mut detail = Vec::with_capacity(cols);
    let mut color = Vec::with_capacity(cols);
    let mut bands = Vec::with_capacity(cols);
    for f in &feats {
        let h = detail_height(f[0]);
        let mb = f[5].max(f[6]).max(f[7]).max(1e-6);
        let (rb1, rb2, rb3) = (f[5] / mb, f[6] / mb, f[7] / mb);
        detail.push(MonoColumn { value: clamp_round(5.0 * rb2, 7), height: h });
        color.push(ColorColumn {
            c0: clamp_round(7.0 * rb1.powf(0.65), 7),
            c1: clamp_round(5.0 * rb2.powf(1.2), 7),
            c2: clamp_round(7.0 * rb3.powf(0.7), 7),
            height: h,
        });
        bands.push([
            clamp_round(100.0 * f[8].powf(0.75), 127),
            clamp_round(100.0 * f[9].powf(0.75), 127),
            clamp_round(130.0 * f[10].powf(1.1), 127),
        ]);
    }
    let mean = |s: usize, e: usize, k: usize| feats[s..e].iter().map(|x| x[k]).sum::<f32>() / (e - s).max(1) as f32;
    let preview = if feats.is_empty() {
        vec![MonoColumn { value: 0, height: 0 }; 400]
    } else {
        bucket(feats.len(), 400, |s, e| MonoColumn {
            height: clamp_round(30.0 * mean(s, e, 0) - 1.5, 31),
            value: clamp_round(4.8 - 4.5 * mean(s, e, 5), 7),
        })
    };
    let tiny = if feats.is_empty() {
        vec![MonoColumn { value: 0, height: 0 }; 100]
    } else {
        bucket(feats.len(), 100, |s, e| MonoColumn { height: clamp_round(21.0 * mean(s, e, 0), 15), value: 0 })
    };
    let bands_preview = if feats.is_empty() {
        vec![[0; 3]; 1200]
    } else {
        bucket(feats.len(), 1200, |s, e| {
            [
                clamp_round(64.0 * mean(s, e, 8).powf(0.85), 127),
                clamp_round(60.0 * mean(s, e, 9).powf(0.7), 127),
                clamp_round(120.0 * mean(s, e, 10).powf(0.85), 127),
            ]
        })
    };
    // PWV4: record A aggregated per bucket (max of max, min of min, max of the others) × 127
    let color_preview = if rec.a.is_empty() {
        vec![[0, 0, 0, 0, 0, 0]; 1200]
    } else {
        bucket(rec.a.len(), 1200, |s, e| {
            let sl = &rec.a[s..e];
            let v = |x: i16| f32::from(x) / 32768.0;
            let max = |k: usize| sl.iter().map(|a| v(a[k])).fold(0.0f32, f32::max);
            let min = sl.iter().map(|a| v(a[1])).fold(0.0f32, f32::min);
            [
                clamp_round(127.0 * max(0), 127),
                ((127.0 * min).round().clamp(-128.0, 0.0) as i8) as u8,
                clamp_round(127.0 * max(2), 127),
                clamp_round(127.0 * max(3), 127),
                clamp_round(127.0 * max(4), 127),
                clamp_round(127.0 * max(5), 127),
            ]
        })
    };
    let maxc = |k: usize| feats.iter().map(|f| f[k]).fold(0.0f32, f32::max).max(1e-3);
    let pwvc = [80, (100.0 / maxc(9)).round().clamp(100.0, 200.0) as u16, (90.0 / maxc(10)).round().clamp(100.0, 200.0) as u16];
    Waveforms { preview, tiny, detail, color, color_preview, bands_preview, bands, pwvc }
}
