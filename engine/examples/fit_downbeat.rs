//! Fit the downbeat (bar offset) choice against observed rekordbox grids (softmax regression on
//! `downbeat_features`): `cargo run --release -p rbx-engine --example fit_downbeat -- [tracks]`.

use rbx_engine::analyze::beats::{downbeat_features, onsets};
use rbx_engine::pcm::decode_file;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(150);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let tracks: Vec<_> = lib.playable().filter(|t| t.bpm().is_some() && !t.variable_tempo()).collect();
    let step = (tracks.len() / n).max(1);
    // per track: features of the 4 offsets relative to rekordbox's first beat, and the true offset
    let mut data: Vec<([[f64; 8]; 4], usize)> = Vec::new();
    for t in tracks.iter().skip(9).step_by(step).take(n) {
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let bpm = t.bpm().unwrap();
        let o = onsets(&pcm);
        let dur = pcm.duration_s();
        let first = f64::from(t.beats[0].time_ms) / 1000.0;
        // true offset: index of the first downbeat among the beats starting at `first`
        let Some(truth) = t.beats.iter().take(4).position(|b| b.beat == 1) else { continue };
        let feats: [[f64; 8]; 4] = std::array::from_fn(|k| downbeat_features(&o, bpm, first, dur, k));
        data.push((feats, truth));
    }
    let split = data.len() * 2 / 3;
    let (train, test) = data.split_at(split);
    let mut w = [0.0f64; 8];
    for _ in 0..20000 {
        let mut g = [0.0f64; 8];
        for (f, y) in train {
            let s: Vec<f64> = f.iter().map(|x| x.iter().zip(&w).map(|(a, b)| a * b).sum::<f64>()).collect();
            let m = s.iter().copied().fold(f64::MIN, f64::max);
            let e: Vec<f64> = s.iter().map(|v| (v - m).exp()).collect();
            let z: f64 = e.iter().sum();
            for k in 0..4 {
                let p = e[k] / z;
                let t = if k == *y { 1.0 } else { 0.0 };
                for j in 0..8 {
                    g[j] += (p - t) * f[k][j];
                }
            }
        }
        for j in 0..8 {
            w[j] -= 2.0 * g[j] / train.len() as f64 + 0.0005 * w[j];
        }
    }
    let acc = |set: &[([[f64; 8]; 4], usize)], w: &[f64; 8]| {
        set.iter()
            .filter(|(f, y)| {
                let best = (0..4)
                    .max_by(|&a, &b| {
                        let sa: f64 = f[a].iter().zip(w).map(|(x, c)| x * c).sum();
                        let sb: f64 = f[b].iter().zip(w).map(|(x, c)| x * c).sum();
                        sa.total_cmp(&sb)
                    })
                    .unwrap();
                best == *y
            })
            .count() as f64
            / set.len().max(1) as f64
    };
    println!("weights {:?}", w.map(|x| (x * 1000.0).round() / 1000.0));
    println!(
        "train {:.3} test {:.3} (low-only rule {:.3}) on {} / {}",
        acc(train, &w),
        acc(test, &w),
        acc(test, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
        train.len(),
        test.len()
    );
}
