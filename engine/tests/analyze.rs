//! Own track analysis on synthetic audio: tempo/phase, key, waveforms, ANLZ output read back through
//! the library, seek table layout.

use std::path::{Path, PathBuf};

use rbx_engine::analyze::beats::{detect_grid, grid_beats};
use rbx_engine::analyze::key::{detect_key, key_chroma, key_of_chroma, Key};
use rbx_engine::analyze::seek::{flac_pvb2, FlacFrame};
use rbx_engine::analyze::wave::{detail_columns, ms_records, waveforms};
use rbx_engine::analyze::{analyze_pcm, anlz_files, folder_for, write_anlz};
use rbx_engine::pcm::{write_wav, Pcm};
use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{read_track, Library, MusicIndex, Source};

const SR: u32 = 44100;

/// Kick-like clicks at `bpm` starting at `first` seconds, plus quieter hats on the offbeats and a
/// clap on every second beat (so beat and offbeat differ like in real dance music).
fn drum_loop(bpm: f64, first: f64, seconds: f64) -> Pcm {
    let n = (seconds * f64::from(SR)) as usize;
    let beat = 60.0 / bpm;
    let mut frames = vec![[0.0f32; 2]; n];
    let mut add = |t: f64, f: &dyn Fn(f64) -> f32, len: f64| {
        let s = (t * f64::from(SR)) as usize;
        for i in 0..(len * f64::from(SR)) as usize {
            if s + i < n {
                let v = f(i as f64 / f64::from(SR));
                frames[s + i][0] += v;
                frames[s + i][1] += v;
            }
        }
    };
    let mut k = 0usize;
    let mut t = first;
    let noise = std::cell::Cell::new(12345u32);
    while t < seconds {
        add(t, &|x| ((x * 60.0 * std::f64::consts::TAU).sin() * (-x * 25.0).exp() * 0.8) as f32, 0.2);
        if k % 2 == 1 {
            add(t, &|x| {
                noise.set(noise.get().wrapping_mul(1_103_515_245).wrapping_add(12345));
                ((f64::from(noise.get() >> 16) / 32768.0 - 1.0) * (-x * 30.0).exp() * 0.25 + (x * 1200.0 * std::f64::consts::TAU).sin() * (-x * 40.0).exp() * 0.3) as f32
            }, 0.12);
        }
        add(t + beat / 2.0, &|x| ((x * 9000.0 * std::f64::consts::TAU).sin() * (-x * 80.0).exp() * 0.1) as f32, 0.05);
        k += 1;
        t += beat;
    }
    Pcm::new(SR, frames)
}

#[test]
fn tempo_and_phase_of_a_drum_loop() {
    for (bpm, first) in [(126.0, 0.31), (140.0, 0.05), (95.0, 0.4)] {
        let pcm = drum_loop(bpm, first, 40.0);
        let g = detect_grid(&pcm).expect("grid");
        assert!((g.bpm - bpm).abs() < 0.015, "{bpm}: got {}", g.bpm);
        let beat = 60.0 / bpm;
        let mut e = (g.first_downbeat_s - first).rem_euclid(beat);
        if e > beat / 2.0 {
            e -= beat;
        }
        assert!(e.abs() < 0.02, "{bpm}: phase off by {:.1} ms", e * 1000.0);
        let beats = grid_beats(&g, 40.0);
        assert!(beats.windows(2).all(|w| w[1].time_ms > w[0].time_ms));
        assert!(beats.iter().all(|b| (1..=4).contains(&b.beat)));
        assert_eq!(beats[0].tempo_x100, (bpm * 100.0) as u16);
    }
}

/// Chord tones with four harmonics, `seconds_each` per chord.
fn chords(chords: &[&[f64]], seconds_each: f64) -> Pcm {
    let mut frames = Vec::new();
    for c in chords {
        for i in 0..(seconds_each * f64::from(SR)) as usize {
            let t = i as f64 / f64::from(SR);
            let mut v = 0.0;
            for &f in c.iter() {
                for h in [1u32, 2, 3, 4] {
                    v += (t * f * h as f64 * std::f64::consts::TAU).sin() / f64::from(h * h);
                }
            }
            let v = (v * 0.08) as f32;
            frames.push([v, v]);
        }
    }
    Pcm::new(SR, frames)
}

#[test]
fn key_of_chord_progressions() {
    // C – F – G – C (C major)
    let c_major = chords(&[&[261.6, 329.6, 392.0], &[349.2, 440.0, 523.3], &[392.0, 493.9, 587.3], &[261.6, 329.6, 392.0]], 2.0);
    let (k, _) = detect_key(&key_chroma(&c_major)).unwrap();
    assert_eq!(k, Key { tonic: 0, minor: false }, "got {}", k.name());
    // Am – Dm – E – Am (A minor)
    let a_minor = chords(&[&[220.0, 261.6, 329.6], &[293.7, 349.2, 440.0], &[329.6, 415.3, 493.9], &[220.0, 261.6, 329.6]], 2.0);
    let (k, _) = detect_key(&key_chroma(&a_minor)).unwrap();
    assert_eq!(k, Key { tonic: 9, minor: true }, "got {}", k.name());
    // Gm – Eb – F – Gm (G minor)
    let g_minor = chords(&[&[196.0, 233.1, 293.7], &[155.6, 196.0, 233.1], &[174.6, 220.0, 261.6], &[196.0, 233.1, 293.7]], 2.0);
    let (k, _) = detect_key(&key_chroma(&g_minor)).unwrap();
    assert_eq!(k, Key { tonic: 7, minor: true }, "got {}", k.name());
    assert!(detect_key(&[]).is_none());
}

#[test]
fn key_names_and_camelot() {
    assert_eq!((Key { tonic: 0, minor: false }.name(), Key { tonic: 0, minor: false }.camelot()), ("C".into(), "8B".into()));
    assert_eq!(Key { tonic: 9, minor: true }.camelot(), "8A");
    assert_eq!(Key { tonic: 6, minor: true }.camelot(), "11A");
    assert_eq!(Key { tonic: 4, minor: false }.camelot(), "12B");
    assert_eq!(Key { tonic: 3, minor: true }.name(), "Ebm");
    // a pure C-major-scale chroma correlates best with C major or A minor
    let scale = [1.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    let (k, r) = key_of_chroma(&scale);
    assert!(matches!((k.tonic, k.minor), (0, false) | (9, true)) && r > 0.5);
}

#[test]
fn waveform_sizes_and_levels() {
    let mut frames = vec![[0.0f32; 2]; SR as usize]; // 1 s silence
    frames.extend(Pcm::sine(SR, 440.0, 2.0, 0.95).frames); // 2 s loud
    let pcm = Pcm::new(SR, frames);
    let w = waveforms(&ms_records(&pcm), pcm.len(), SR);
    assert_eq!(w.detail.len(), detail_columns(pcm.len(), SR));
    assert_eq!(w.detail.len(), 450, "3 s × 150");
    assert_eq!((w.preview.len(), w.tiny.len(), w.color_preview.len(), w.bands_preview.len()), (400, 100, 1200, 1200));
    assert_eq!(w.bands.len(), w.detail.len());
    assert_eq!(w.detail[50].height, 0, "silence");
    assert!(w.detail[300].height >= 28, "near full scale: {}", w.detail[300].height);
    assert_eq!(w.color_preview[100][0], 0);
    assert!(w.color_preview[1000][0] > 110 && w.color_preview[1000][1] > 128, "max ×127 and min as signed byte");
    assert_eq!(w.pwvc[0], 80);
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rbx-analyze-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn anlz_output_reads_back() {
    let dir = temp("out");
    let audio = dir.join("Test Artist - Loop.wav");
    let pcm = drum_loop(128.0, 0.2, 30.0);
    write_wav(&audio, &pcm).unwrap();
    let r = analyze_pcm(&audio, &pcm);
    let (dat, ext, ex2) = anlz_files(&r, &audio.to_string_lossy());
    let tags = |b: &[u8]| -> Vec<String> { AnlzFile::parse(b).unwrap().tags.iter().map(|t| t.fourcc.to_string()).collect() };
    assert_eq!(tags(&dat), ["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PCOB"]);
    assert_eq!(tags(&ext), ["PPTH", "PWV3", "PCOB", "PCOB", "PCO2", "PCO2", "PWV5", "PWV4"], "no PVB2 for WAV");
    assert_eq!(tags(&ex2), ["PPTH", "PWV7", "PWV6", "PWVC"]);
    for b in [&dat, &ext, &ex2] {
        for t in AnlzFile::parse(b).unwrap().tags {
            assert!(Tag::from_raw(&t).is_ok(), "{} decodes", t.fourcc);
        }
    }
    let out = dir.join("anlz");
    let folder = write_anlz(&r, &out).unwrap();
    let t = read_track(&folder).expect("library reads our analysis");
    assert_eq!(t.source, Source::Own);
    assert_eq!(t.stored_path, audio.to_string_lossy());
    assert_eq!((t.artist.as_deref(), t.title.as_str()), (Some("Test Artist"), "Loop"));
    assert!((t.bpm().unwrap() - 128.0).abs() < 0.015);
    assert!(t.key.is_some());
    // the library finds it by its full path, even without a music index
    let lib = Library::build_multi(&[out.clone()], &[]);
    assert_eq!(lib.by_path(&audio).unwrap().title, "Loop");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rekordbox_analysis_wins_over_own() {
    let dir = temp("merge");
    let audio = dir.join("A - B.wav");
    let pcm = drum_loop(120.0, 0.1, 12.0);
    write_wav(&audio, &pcm).unwrap();
    let r = analyze_pcm(&audio, &pcm);
    let own = read_track(&write_anlz(&r, &dir.join("own")).unwrap()).unwrap();
    let mut rb = own.clone();
    rb.source = Source::Rekordbox;
    rb.title = "rekordbox".into();
    let mut lib = Library::from_tracks(vec![rb], &MusicIndex::build(&[dir.clone()]));
    lib.insert(own.clone());
    assert_eq!(lib.by_path(&audio).unwrap().title, "rekordbox", "own analysis does not replace rekordbox's");
    let mut lib2 = Library::default();
    lib2.insert(own);
    assert_eq!(lib2.by_path(&audio).unwrap().source, Source::Own);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analysis_folder_is_stable() {
    let a = folder_for(Path::new(r"C:\Music\A - B.flac"));
    assert_eq!(a, folder_for(Path::new(r"c:\music\a - b.FLAC")), "case-insensitive");
    if cfg!(windows) {
        assert_eq!(a, folder_for(Path::new("C:/Music/A - B.flac")), "either separator");
    }
    assert_ne!(a, folder_for(Path::new(r"C:\Music\A - C.flac")));
    assert_eq!(a.components().count(), 2);
    assert_eq!(a.iter().next().unwrap().len(), 3);
}

#[test]
fn flac_seek_table_layout() {
    let frames: Vec<FlacFrame> = (0..100).map(|i| FlacFrame { sample: i * 4096, offset: i * 1000, block_size: 4096 }).collect();
    let samples = 100 * 4096 - 500;
    let t = flac_pvb2(&frames, samples);
    assert_eq!(t.len(), 8000);
    let entry = |i: usize| (u64::from_be_bytes(t[i * 20..i * 20 + 8].try_into().unwrap()), u64::from_be_bytes(t[i * 20 + 8..i * 20 + 16].try_into().unwrap()));
    assert_eq!(entry(0), (0, 0));
    // entry i targets i * floor(samples / 400), rounded down to a frame start
    let step = samples / 400;
    for i in [1usize, 37, 399] {
        let target = i as u64 * step;
        let (s, o) = entry(i);
        assert!(s <= target && target < s + 4096 && o == s / 4096 * 1000, "entry {i}");
    }
}
