//! Fit the beat/offbeat decision against observed rekordbox grids (logistic regression on
//! `phase_features`): `cargo run --release -p rbx-engine --example fit_phase -- [tracks]`.

use rbx_engine::analyze::beats::{onsets, phase_features, FPS};
use rbx_engine::pcm::decode_file;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn comb_phase(env: &[f32], bpm: f64, dur: f64) -> f64 {
    let beat = 60.0 / bpm;
    let n = (dur / beat) as usize;
    let mut best = (f64::MIN, 0.0);
    let mut ph = 0.0;
    while ph < beat {
        let s: f64 = (0..n)
            .map(|k| {
                let x = (ph + k as f64 * beat) * FPS;
                let i = x as usize;
                if i + 1 < env.len() {
                    let f = (x - i as f64) as f32;
                    f64::from(env[i] * (1.0 - f) + env[i + 1] * f)
                } else {
                    0.0
                }
            })
            .sum();
        if s > best.0 {
            best = (s, ph);
        }
        ph += 0.001;
    }
    best.1
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(120);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let tracks: Vec<_> = lib.playable().filter(|t| t.bpm().is_some() && !t.variable_tempo()).collect();
    let step = (tracks.len() / n).max(1);
    let mut data: Vec<([f64; 6], f64)> = Vec::new();
    for t in tracks.iter().skip(5).step_by(step).take(n) {
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let bpm = t.bpm().unwrap();
        let o = onsets(&pcm);
        let dur = pcm.duration_s();
        let ph = comb_phase(&o.full, bpm, dur);
        let beat = 60.0 / bpm;
        let first = f64::from(t.beats[0].time_ms) / 1000.0;
        let mut e = (first - ph).rem_euclid(beat);
        if e > beat / 2.0 {
            e -= beat;
        }
        let label = if e.abs() < beat / 4.0 { 1.0 } else { 0.0 };
        let f = phase_features(&o, bpm, ph, dur);
        println!("{label} {:?} {}", f.map(|x| (x * 1000.0).round() / 1000.0), t.title);
        data.push((f, label));
    }
    let split = data.len() * 2 / 3;
    let (train, test) = data.split_at(split);
    let mut w = [0.0f64; 6];
    for _ in 0..20000 {
        let mut g = [0.0f64; 6];
        for (x, y) in train {
            let p = sigmoid(x.iter().zip(&w).map(|(a, b)| a * b).sum());
            for k in 0..6 {
                g[k] += (p - y) * x[k];
            }
        }
        for k in 0..6 {
            w[k] -= 0.5 * g[k] / train.len() as f64 + 0.001 * w[k];
        }
    }
    let acc = |set: &[([f64; 6], f64)], w: &[f64; 6]| {
        set.iter().filter(|(x, y)| ((x.iter().zip(w).map(|(a, b)| a * b).sum::<f64>() > 0.0) as u8 as f64) == *y).count() as f64
            / set.len().max(1) as f64
    };
    let always = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let low_rule = [0.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    println!("\nweights {:?}", w.map(|x| (x * 1000.0).round() / 1000.0));
    println!("train acc {:.3}  test acc {:.3}  (always keep: {:.3}, low-flux rule: {:.3}) on {} / {}", acc(train, &w), acc(test, &w), acc(test, &always), acc(test, &low_rule), train.len(), test.len());
}
