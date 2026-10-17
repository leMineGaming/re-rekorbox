//! Calibration of the waveform column mapping against rekordbox's own analyses (local data, read
//! only): `cargo run --release -p rbx-engine --example calibrate -- [tracks]`.
//!
//! For each column it pairs our per-column features with rekordbox's stored values and prints,
//! per target, the mean target value for feature quantiles and the best lag.

use std::path::Path;

use rbx_engine::analyze::wave::{column_features, ms_records};
use rbx_engine::pcm::decode_file;
use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn tags(p: &Path) -> Vec<Tag> {
    let Ok(b) = std::fs::read(p) else { return vec![] };
    let Ok(f) = AnlzFile::parse(&b) else { return vec![] };
    f.tags.iter().filter_map(|r| Tag::from_raw(r).ok()).collect()
}

const FEATURES: [&str; 17] = [
    "peak", "bass", "aLow", "aMid", "aHigh", "b<100", "b300-3k", "b>1500", "c<300", "c250-1k2", "c3k-9k",
    "rb1", "rb2", "rb3", "ra1", "ra2", "ra3",
];
const NF: usize = 17;

/// Base features plus ratios (band / max of its group).
fn extend(f: [f32; 11]) -> [f32; NF] {
    let mut o = [0.0f32; NF];
    o[..11].copy_from_slice(&f);
    let mb = f[5].max(f[6]).max(f[7]).max(1e-6);
    let ma = f[2].max(f[3]).max(f[4]).max(1e-6);
    o[11] = f[5] / mb;
    o[12] = f[6] / mb;
    o[13] = f[7] / mb;
    o[14] = f[2] / ma;
    o[15] = f[3] / ma;
    o[16] = f[4] / ma;
    o
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    // pairs (features, targets): targets = [h3, w3, c0, c1, c2, h5, p7a, p7b, p7c]
    let mut rows: Vec<([f32; NF], [f32; 9])> = Vec::new();
    let mut used = 0;
    for t in lib.playable().filter(|t| t.path.is_some()).step_by(97) {
        if used >= n {
            break;
        }
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let ext = tags(&t.anlz_dir.join("ANLZ0000.EXT"));
        let ex2 = tags(&t.anlz_dir.join("ANLZ0000.2EX"));
        let mut pwv3 = None;
        let mut pwv5 = None;
        let mut pwv7 = None;
        for tag in ext.iter().chain(ex2.iter()) {
            if let Tag::Waveform(w) = tag {
                match w.entry_size {
                    1 if pwv3.is_none() => pwv3 = w.mono_columns(),
                    2 if pwv5.is_none() => pwv5 = w.color_columns(),
                    3 if w.entries_per_second() == Some(150) && pwv7.is_none() => {
                        pwv7 = Some(w.entries().map(|e| [e[0], e[1], e[2]]).collect::<Vec<_>>())
                    }
                    _ => {}
                }
            }
        }
        let (Some(p3), Some(p5), Some(p7)) = (pwv3, pwv5, pwv7) else { continue };
        let rec = ms_records(&pcm);
        let feats = column_features(&rec, p3.len());
        // best lag on peak vs height
        let mut best = (0i32, f64::MIN);
        for lag in -8i32..=8 {
            let mut s = 0.0f64;
            for i in 20..p3.len().saturating_sub(20) {
                let j = i as i32 + lag;
                if j >= 0 && (j as usize) < feats.len() {
                    s += f64::from(feats[j as usize][0]) * f64::from(p3[i].height);
                }
            }
            if s > best.1 {
                best = (lag, s);
            }
        }
        let pwvc = ex2.iter().find_map(|t| match t {
            Tag::Raw(r) if r.fourcc.to_string() == "PWVC" => Some(r.body.clone()),
            _ => None,
        });
        let maxc = |k: usize| feats.iter().map(|f| f[k]).fold(0.0f32, f32::max);
        println!(
            "{:<50} cols {} lag {} PWVC {:?} max c {:.3} {:.3} {:.3}",
            t.title, p3.len(), best.0, pwvc, maxc(8), maxc(9), maxc(10)
        );
        for i in 0..p3.len().min(p5.len()).min(p7.len()).min(feats.len()) {
            let c5 = p5[i];
            rows.push((
                extend(feats[i]),
                [
                    f32::from(p3[i].height),
                    f32::from(p3[i].value),
                    f32::from(c5.c0),
                    f32::from(c5.c1),
                    f32::from(c5.c2),
                    f32::from(c5.height),
                    f32::from(p7[i][0]),
                    f32::from(p7[i][1]),
                    f32::from(p7[i][2]),
                ],
            ));
        }
        used += 1;
    }
    let targets = ["h3", "w3", "c0", "c1", "c2", "h5", "p7a", "p7b", "p7c"];
    // correlation matrix target x feature
    println!("\ncorrelation (Pearson) of targets with features ({} columns):", rows.len());
    print!("{:>6}", "");
    for f in FEATURES {
        print!("{f:>9}");
    }
    println!();
    for (ti, tn) in targets.iter().enumerate() {
        print!("{tn:>6}");
        for fi in 0..NF {
            let xs: Vec<f64> = rows.iter().map(|r| f64::from(r.0[fi])).collect();
            let ys: Vec<f64> = rows.iter().map(|r| f64::from(r.1[ti])).collect();
            print!("{:>9.3}", pearson(&xs, &ys));
        }
        println!();
    }
    // mapping tables: mean target per feature decile for the best feature of each target
    for (ti, tn) in targets.iter().enumerate() {
        let (fi, _) = (0..NF)
            .map(|fi| {
                let xs: Vec<f64> = rows.iter().map(|r| f64::from(r.0[fi])).collect();
                let ys: Vec<f64> = rows.iter().map(|r| f64::from(r.1[ti])).collect();
                (fi, pearson(&xs, &ys))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        let mut v: Vec<(f32, f32)> = rows.iter().map(|r| (r.0[fi], r.1[ti])).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        print!("\n{tn} vs {}: ", FEATURES[fi]);
        for q in 0..20 {
            let s = &v[q * v.len() / 20..(q + 1) * v.len() / 20];
            if s.is_empty() {
                continue;
            }
            let fx = s.iter().map(|x| x.0).sum::<f32>() / s.len() as f32;
            let fy = s.iter().map(|x| x.1).sum::<f32>() / s.len() as f32;
            print!("{fx:.3}->{fy:.1} ");
        }
        println!();
    }
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    sxy / (sxx.sqrt() * syy.sqrt()).max(1e-12)
}
