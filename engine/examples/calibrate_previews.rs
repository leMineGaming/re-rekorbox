//! Calibration of the preview waveforms (PWAV, PWV2, PWV4, PWV6) against our per-column features:
//! `cargo run --release -p rbx-engine --example calibrate_previews -- [tracks]`.

use std::path::Path;

use rbx_engine::analyze::wave::{column_features, detail_columns, ms_records};
use rbx_engine::pcm::decode_file;
use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn tags(p: &Path) -> Vec<Tag> {
    let Ok(b) = std::fs::read(p) else { return vec![] };
    let Ok(f) = AnlzFile::parse(&b) else { return vec![] };
    f.tags.iter().filter_map(|r| Tag::from_raw(r).ok()).collect()
}

const NAMES: [&str; 11] = ["peak", "bass", "aLow", "aMid", "aHigh", "b<100", "b300-3k", "b>1500", "c<300", "c250-1k2", "c3k-9k"];

/// Per bucket: mean and max of each feature (22 values).
fn bucket_feats(f: &[[f32; 11]], n: usize) -> Vec<[f32; 22]> {
    (0..n)
        .map(|j| {
            let s = j * f.len() / n;
            let e = ((j + 1) * f.len() / n).max(s + 1).min(f.len());
            let mut o = [0.0f32; 22];
            for k in 0..11 {
                let sl = &f[s..e];
                o[k] = sl.iter().map(|x| x[k]).sum::<f32>() / sl.len() as f32;
                o[11 + k] = sl.iter().map(|x| x[k]).fold(0.0f32, f32::max);
            }
            o
        })
        .collect()
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

fn report(name: &str, rows: &[([f32; 22], f32)]) {
    let ys: Vec<f64> = rows.iter().map(|r| f64::from(r.1)).collect();
    let (fi, r) = (0..22)
        .map(|fi| (fi, pearson(&rows.iter().map(|r| f64::from(r.0[fi])).collect::<Vec<_>>(), &ys)))
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    let fname = format!("{}({})", if fi < 11 { "mean" } else { "max" }, NAMES[fi % 11]);
    let mut v: Vec<(f32, f32)> = rows.iter().map(|r| (r.0[fi], r.1)).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    print!("{name:<10} best {fname:<16} r={r:.3}: ");
    for q in 0..12 {
        let s = &v[q * v.len() / 12..(q + 1) * v.len() / 12];
        if !s.is_empty() {
            print!("{:.3}->{:.1} ", s.iter().map(|x| x.0).sum::<f32>() / s.len() as f32, s.iter().map(|x| x.1).sum::<f32>() / s.len() as f32);
        }
    }
    println!();
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let mut targets: Vec<Vec<([f32; 22], f32)>> = vec![Vec::new(); 3 + 3 + 6];
    let mut used = 0;
    for t in lib.playable().step_by(89) {
        if used >= n {
            break;
        }
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let dat = tags(&t.anlz_dir.join("ANLZ0000.DAT"));
        let ext = tags(&t.anlz_dir.join("ANLZ0000.EXT"));
        let ex2 = tags(&t.anlz_dir.join("ANLZ0000.2EX"));
        let (mut pwav, mut pwv2, mut p4, mut p6) = (None, None, None, None);
        for tg in dat.iter().chain(ext.iter()).chain(ex2.iter()) {
            match tg {
                Tag::WavePreview(w) if w.columns().len() == 400 => pwav = Some(w.columns()),
                Tag::WavePreview(w) if w.columns().len() == 100 => pwv2 = Some(w.columns()),
                Tag::Waveform(w) if w.entry_size == 6 && p4.is_none() => p4 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                Tag::Waveform(w) if w.entry_size == 3 && w.entries_per_second() != Some(150) && p6.is_none() => {
                    p6 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>())
                }
                _ => {}
            }
        }
        let (Some(pwav), Some(pwv2), Some(p4), Some(p6)) = (pwav, pwv2, p4, p6) else { continue };
        let rec = ms_records(&pcm);
        let feats = column_features(&rec, detail_columns(pcm.len(), pcm.sample_rate));
        let b400 = bucket_feats(&feats, 400);
        let b100 = bucket_feats(&feats, 100);
        let b1200 = bucket_feats(&feats, 1200);
        for j in 0..400 {
            targets[0].push((b400[j], f32::from(pwav[j].height)));
            targets[1].push((b400[j], f32::from(pwav[j].value)));
        }
        for j in 0..100 {
            targets[2].push((b100[j], f32::from(pwv2[j].height)));
        }
        for j in 0..1200.min(p6.len()) {
            for b in 0..3 {
                targets[3 + b].push((b1200[j], f32::from(p6[j][b])));
            }
        }
        for j in 0..1200.min(p4.len()) {
            for b in 0..6 {
                targets[6 + b].push((b1200[j], f32::from(p4[j][b])));
            }
        }
        used += 1;
        println!("{}", t.title);
    }
    for (nm, idx, fi) in [("PWAV h", 0usize, 0usize), ("PWAV h", 0, 11), ("PWV2 h", 2, 0), ("PWV2 h", 2, 11), ("PWAV w", 1, 16), ("PWAV w", 1, 5)] {
        let rows: Vec<([f32; 22], f32)> = targets[idx].iter().map(|r| { let mut o = r.0; o[0] = r.0[fi]; (o, r.1) }).collect();
        let mut v: Vec<(f32, f32)> = rows.iter().map(|r| (r.0[0], r.1)).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        print!("{nm} vs feature {fi}: ");
        for q in 0..12 {
            let sl = &v[q * v.len() / 12..(q + 1) * v.len() / 12];
            print!("{:.3}->{:.1} ", sl.iter().map(|x| x.0).sum::<f32>() / sl.len() as f32, sl.iter().map(|x| x.1).sum::<f32>() / sl.len() as f32);
        }
        println!();
    }
    let names = ["PWAV h", "PWAV w", "PWV2 h", "PWV6 b1", "PWV6 b2", "PWV6 b3", "PWV4 0", "PWV4 1", "PWV4 2", "PWV4 3", "PWV4 4", "PWV4 5"];
    for (nm, rows) in names.iter().zip(&targets) {
        report(nm, rows);
    }
}
