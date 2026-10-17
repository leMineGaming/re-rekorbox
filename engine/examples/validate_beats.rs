//! Compare our tempo/grid detection with rekordbox's grids of the same files:
//! `cargo run --release -p rbx-engine --example validate_beats -- [tracks] [skip]`.

use rbx_engine::analyze::beats::detect_grid;
use rbx_engine::pcm::decode_file;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let skip: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let tracks: Vec<_> = lib.playable().filter(|t| t.bpm().is_some() && !t.variable_tempo()).collect();
    let step = (tracks.len() / n).max(1);
    let (mut exact, mut close, mut octave, mut other) = (0, 0, 0, 0);
    let mut phase_errs: Vec<f64> = Vec::new();
    let (mut down_ok, mut down_n) = (0, 0);
    let t0 = std::time::Instant::now();
    for t in tracks.iter().skip(skip).step_by(step).take(n) {
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let Some(g) = detect_grid(&pcm) else {
            other += 1;
            continue;
        };
        let theirs = t.bpm().unwrap();
        let d = (g.bpm - theirs).abs();
        let ratio = g.bpm / theirs;
        let class = if d < 0.05 {
            exact += 1;
            "exact"
        } else if d < 0.5 {
            close += 1;
            "close"
        } else if (ratio - 2.0).abs() < 0.02 || (ratio - 0.5).abs() < 0.01 || (ratio - 1.5).abs() < 0.015 || (ratio - 2.0 / 3.0).abs() < 0.01 {
            octave += 1;
            "octave"
        } else {
            other += 1;
            "OTHER"
        };
        let mut phase = String::new();
        if d < 0.5 {
            let beat = 60.0 / theirs;
            let their_first = f64::from(t.beats[0].time_ms) / 1000.0;
            let mut e = (their_first - g.first_downbeat_s).rem_euclid(beat);
            if e > beat / 2.0 {
                e -= beat;
            }
            phase_errs.push(e * 1000.0);
            // downbeat: their first downbeat vs ours, in beats, modulo a bar
            if let Some(td) = t.first_downbeat_s() {
                let k = ((td - g.first_downbeat_s) / beat).round() as i64;
                down_n += 1;
                if k.rem_euclid(4) == 0 {
                    down_ok += 1;
                }
            }
            phase = format!("phase {:+.1} ms", e * 1000.0);
        }
        let len_diff = t.samples.map(|s| pcm.len() as i64 - s as i64);
        println!("{class:<6} ours {:>7.2} theirs {:>7.2} conf {:.2} {phase} len-diff {:?} {}  {}", g.bpm, theirs, g.confidence, len_diff, t.path.as_ref().unwrap().extension().unwrap().to_string_lossy(), t.title);
    }
    let total = exact + close + octave + other;
    phase_errs.sort_by(f64::total_cmp);
    let med = phase_errs.get(phase_errs.len() / 2).copied().unwrap_or(0.0);
    let within = |ms: f64| phase_errs.iter().filter(|e| (*e - med).abs() <= ms).count();
    println!(
        "\n{total} tracks in {:.1} s: exact {exact}, close {close}, octave/ratio {octave}, other {other}",
        t0.elapsed().as_secs_f64()
    );
    println!(
        "phase: median {med:+.1} ms; within ±10 ms of the median {} / ±25 ms {} of {}",
        within(10.0),
        within(25.0),
        phase_errs.len()
    );
    println!("downbeat agrees: {down_ok} of {down_n}");
}
