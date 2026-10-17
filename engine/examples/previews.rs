//! How rekordbox's preview waveforms relate to its detail waveforms (observation on local analyses,
//! no audio needed): `cargo run --release -p rbx-engine --example previews -- [tracks]`.

use std::path::Path;

use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{default_anlz_root, find_anlz_dirs};

fn tags(p: &Path) -> Vec<Tag> {
    let Ok(b) = std::fs::read(p) else { return vec![] };
    let Ok(f) = AnlzFile::parse(&b) else { return vec![] };
    f.tags.iter().filter_map(|r| Tag::from_raw(r).ok()).collect()
}

/// Aggregate `src` (per detail column) into `n` buckets with `agg`.
fn buckets(src: &[f32], n: usize, agg: &dyn Fn(&[f32]) -> f32) -> Vec<f32> {
    (0..n)
        .map(|j| {
            let s = j * src.len() / n;
            let e = ((j + 1) * src.len() / n).max(s + 1).min(src.len());
            agg(&src[s.min(src.len().saturating_sub(1))..e])
        })
        .collect()
}

fn mae(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len().max(1) as f32
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(200);
    let dirs = find_anlz_dirs(&default_anlz_root().unwrap());
    let max = |s: &[f32]| s.iter().copied().fold(0.0f32, f32::max);
    let mean = |s: &[f32]| s.iter().sum::<f32>() / s.len().max(1) as f32;
    let rms = |s: &[f32]| (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt();
    let aggs: [(&str, &dyn Fn(&[f32]) -> f32); 3] = [("max", &max), ("mean", &mean), ("rms", &rms)];
    let mut err: Vec<[f32; 3]> = vec![[0.0; 3]; 8];
    let mut count = 0;
    let mut pwv4_samples = Vec::new();
    for d in dirs.iter().step_by((dirs.len() / n).max(1)) {
        let dat = tags(&d.join("ANLZ0000.DAT"));
        let ext = tags(&d.join("ANLZ0000.EXT"));
        let ex2 = tags(&d.join("ANLZ0000.2EX"));
        let mut pwav = None;
        let mut pwv2 = None;
        for t in &dat {
            if let Tag::WavePreview(w) = t {
                let cols: Vec<_> = w.columns();
                if cols.len() == 400 {
                    pwav = Some(cols);
                } else if cols.len() == 100 {
                    pwv2 = Some(cols);
                }
            }
        }
        let (mut p3, mut p4, mut p6, mut p7) = (None, None, None, None);
        for t in ext.iter().chain(ex2.iter()) {
            if let Tag::Waveform(w) = t {
                match (w.entry_size, w.entries_per_second()) {
                    (1, _) if p3.is_none() => p3 = w.mono_columns(),
                    (6, _) if p4.is_none() => p4 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                    (3, Some(150)) if p7.is_none() => p7 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                    (3, _) if p6.is_none() => p6 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                    _ => {}
                }
            }
        }
        let (Some(pwav), Some(pwv2), Some(p3), Some(p4), Some(p6), Some(p7)) = (pwav, pwv2, p3, p4, p6, p7) else { continue };
        let h: Vec<f32> = p3.iter().map(|c| f32::from(c.height)).collect();
        let w: Vec<f32> = p3.iter().map(|c| f32::from(c.value)).collect();
        let tgt_h: Vec<f32> = pwav.iter().map(|c| f32::from(c.height)).collect();
        let tgt_w: Vec<f32> = pwav.iter().map(|c| f32::from(c.value)).collect();
        let tgt2: Vec<f32> = pwv2.iter().map(|c| f32::from(c.height)).collect();
        for (k, (_, a)) in aggs.iter().enumerate() {
            err[0][k] += mae(&buckets(&h, 400, a), &tgt_h);
            err[1][k] += mae(&buckets(&w, 400, a), &tgt_w);
            // PWV2 heights are 0..15: compare with half of the 100-bucket aggregate
            err[2][k] += mae(&buckets(&h, 100, a).iter().map(|x| x / 2.0).collect::<Vec<_>>(), &tgt2);
            for band in 0..3 {
                let src: Vec<f32> = p7.iter().map(|e| f32::from(e[band])).collect();
                let tgt: Vec<f32> = p6.iter().map(|e| f32::from(e[band])).collect();
                err[3 + band][k] += mae(&buckets(&src, tgt.len(), a), &tgt);
            }
        }
        if pwv4_samples.len() < 3 {
            pwv4_samples.push((p4.len(), p4[600].clone(), p4[300].clone()));
        }
        count += 1;
    }
    println!("{count} tracks; mean absolute error per aggregation (max / mean / rms):");
    for (name, e) in ["PWAV height", "PWAV value", "PWV2 height/2", "PWV6 band 1", "PWV6 band 2", "PWV6 band 3"].iter().zip(&err) {
        println!("  {name:<16} {:>7.3} {:>7.3} {:>7.3}", e[0] / count as f32, e[1] / count as f32, e[2] / count as f32);
    }
    println!("PWV4 samples (entries, entry 600, entry 300): {pwv4_samples:?}");
}
