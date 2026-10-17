//! Compare our waveforms with rekordbox's analyses of the same files (other tracks than the
//! calibration set): `cargo run --release -p rbx-engine --example validate_wave -- [tracks]`.

use std::path::Path;

use rbx_engine::analyze::wave::{ms_records, waveforms};
use rbx_engine::pcm::decode_file;
use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

fn tags(p: &Path) -> Vec<Tag> {
    let Ok(b) = std::fs::read(p) else { return vec![] };
    let Ok(f) = AnlzFile::parse(&b) else { return vec![] };
    f.tags.iter().filter_map(|r| Tag::from_raw(r).ok()).collect()
}

#[derive(Default)]
struct Stat {
    sum: f64,
    n: u64,
    within1: u64,
    range: f64,
}

impl Stat {
    fn add(&mut self, ours: f64, theirs: f64, range: f64) {
        let d = (ours - theirs).abs();
        self.sum += d;
        self.n += 1;
        if d <= 1.0 {
            self.within1 += 1;
        }
        self.range = range;
    }
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let names = [
        "PWV3 height", "PWV3 value", "PWV5 c0", "PWV5 c1", "PWV5 c2", "PWV7 band1", "PWV7 band2", "PWV7 band3",
        "PWAV height", "PWAV value", "PWV2 height", "PWV4 b0", "PWV4 b1", "PWV4 b2", "PWV4 b3", "PWV4 b4", "PWV4 b5",
        "PWV6 band1", "PWV6 band2", "PWV6 band3",
    ];
    let ranges = [31.0, 7.0, 7.0, 7.0, 7.0, 127.0, 127.0, 127.0, 31.0, 7.0, 15.0, 127.0, 255.0, 127.0, 127.0, 127.0, 127.0, 127.0, 127.0, 127.0];
    let mut st: Vec<Stat> = (0..names.len()).map(|_| Stat::default()).collect();
    let mut used = 0;
    for t in lib.playable().skip(41).step_by(83) {
        if used >= n {
            break;
        }
        let Ok(pcm) = decode_file(t.path.as_ref().unwrap()) else { continue };
        let mut theirs = (None, None, None, None, None, None, None);
        for d in ["DAT", "EXT", "2EX"] {
            for tg in tags(&t.anlz_dir.join(format!("ANLZ0000.{d}"))) {
                match tg {
                    Tag::WavePreview(w) if w.columns().len() == 400 => theirs.0 = Some(w.columns()),
                    Tag::WavePreview(w) if w.columns().len() == 100 => theirs.1 = Some(w.columns()),
                    Tag::Waveform(w) => match (w.entry_size, w.entries_per_second()) {
                        (1, _) if theirs.2.is_none() => theirs.2 = w.mono_columns(),
                        (2, _) if theirs.3.is_none() => theirs.3 = w.color_columns(),
                        (6, _) if theirs.4.is_none() => theirs.4 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                        (3, Some(150)) if theirs.6.is_none() => theirs.6 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                        (3, _) if theirs.5.is_none() => theirs.5 = Some(w.entries().map(|e| e.to_vec()).collect::<Vec<_>>()),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        let (Some(pwav), Some(pwv2), Some(p3), Some(p5), Some(p4), Some(p6), Some(p7)) = theirs else { continue };
        let ours = waveforms(&ms_records(&pcm), pcm.len(), pcm.sample_rate);
        let mut add = |k: usize, a: u8, b: u8| st[k].add(f64::from(a), f64::from(b), ranges[k]);
        for (o, r) in ours.detail.iter().zip(&p3) {
            add(0, o.height, r.height);
            add(1, o.value, r.value);
        }
        for (o, r) in ours.color.iter().zip(&p5) {
            add(2, o.c0, r.c0);
            add(3, o.c1, r.c1);
            add(4, o.c2, r.c2);
        }
        for (o, r) in ours.bands.iter().zip(&p7) {
            for b in 0..3 {
                add(5 + b, o[b], r[b]);
            }
        }
        for (o, r) in ours.preview.iter().zip(&pwav) {
            add(8, o.height, r.height);
            add(9, o.value, r.value);
        }
        for (o, r) in ours.tiny.iter().zip(&pwv2) {
            add(10, o.height, r.height);
        }
        for (o, r) in ours.color_preview.iter().zip(&p4) {
            for b in 0..6 {
                add(11 + b, o[b], r[b]);
            }
        }
        for (o, r) in ours.bands_preview.iter().zip(&p6) {
            for b in 0..3 {
                add(17 + b, o[b], r[b]);
            }
        }
        used += 1;
        println!("{} ({} columns, ours {})", t.title, p3.len(), ours.detail.len());
    }
    println!("\n{:<12} {:>8} {:>10} {:>10}", "tag", "MAE", "MAE/range", "within ±1");
    for (name, s) in names.iter().zip(&st) {
        let mae = s.sum / s.n.max(1) as f64;
        println!("{name:<12} {mae:>8.2} {:>9.1}% {:>9.1}%", 100.0 * mae / s.range, 100.0 * s.within1 as f64 / s.n.max(1) as f64);
    }
}
