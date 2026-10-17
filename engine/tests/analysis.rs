//! Beat maps (tracks with tempo changes), applying a library analysis to a deck, library browser view.

use std::path::PathBuf;
use std::sync::Arc;

use rbx_engine::analysis::TrackAnalysis;
use rbx_engine::browser::{Browser, Entry, View};
use rbx_engine::deck::{BeatMap, Grid};
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::pcm::Pcm;
use rekordbox_formats::anlz::tags::Beat;
use rekordbox_formats::library::{Cue, LibTrack};

const SR: u32 = 44100;

/// 16 beats at 120 BPM from 0.25 s (first entry is beat 4 of a bar), then 16 at 150 BPM.
fn tempo_change() -> (Vec<f64>, Vec<u16>, Vec<f64>) {
    let mut times = Vec::new();
    let mut t = 0.25;
    for i in 0..32 {
        times.push(t);
        t += if i < 16 { 0.5 } else { 0.4 };
    }
    let numbers = (0..32).map(|i| ((i + 3) % 4 + 1) as u16).collect();
    let bpms = (0..32).map(|i| if i < 16 { 120.0 } else { 150.0 }).collect();
    (times, numbers, bpms)
}

fn map() -> BeatMap {
    let (t, n, b) = tempo_change();
    BeatMap::new(t, &n).unwrap().with_bpms(b)
}

#[test]
fn beat_map_math() {
    let m = map();
    assert_eq!(m.downbeat_index, 1, "second entry is beat 1");
    assert!((m.beat_at(0.75) - 0.0).abs() < 1e-9, "beat 0 = first downbeat at 0.75 s");
    assert!((m.beat_at(1.0) - 0.5).abs() < 1e-9);
    assert!((m.time_of(15.0) - 8.25).abs() < 1e-9, "entry 16 = first 150 BPM beat");
    assert!((m.time_of(16.0) - 8.65).abs() < 1e-9);
    assert!((m.beat_at(m.time_of(20.3)) - 20.3).abs() < 1e-9, "inverse");
    assert_eq!((m.bpm_at(2.0), m.bpm_at(9.0)), (120.0, 150.0));
    assert!((m.beat_len_at(9.0) - 0.4).abs() < 1e-9);
    assert!((m.beat_at(0.0) + 1.5).abs() < 1e-9, "extrapolated before the first beat");
    assert!(m.time_of(100.0) > m.times[31], "extrapolated after the last beat");
    assert!(BeatMap::new(vec![1.0], &[1]).is_none());
}

fn deck_mixer() -> Mixer {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::sine(SR, 220.0, 30.0, 0.2)), grid: Some(Grid { bpm: 120.0, first_beat_s: 0.75 }) });
    let analysis = TrackAnalysis {
        grid: Some(Grid { bpm: 120.0, first_beat_s: 0.75 }),
        beats: Some(Arc::new(map())),
        hot_cues: [Some(2.0), None, Some(4.5), None, None, None, None, None],
        memory_cues: vec![10.0, 5.0],
        title: "Test".into(),
        variable_tempo: true,
    };
    m.apply(Command::ApplyAnalysis { deck: 0, analysis: Arc::new(analysis) });
    m
}

#[test]
fn deck_follows_the_beat_map() {
    let mut m = deck_mixer();
    let s = m.state().decks[0];
    assert_eq!(s.hot_cues[..3], [true, false, true]);
    assert_eq!(m.decks[0].memory_cues_s(), [5.0, 10.0]);
    // in the 150 BPM part the deck reports 150 BPM and beat jumps use 0.4 s beats
    m.apply(Command::SeekSeconds(0, 9.05));
    assert_eq!(m.decks[0].bpm(), Some(150.0));
    m.apply(Command::BeatJump(0, -4.0));
    // 9.05 s = entry 18; four beats back = entry 14 at 7.25 s (two 0.4 s beats and two 0.5 s beats)
    assert!((m.decks[0].position_s() - 7.25).abs() < 1e-6, "back across the tempo change: {}", m.decks[0].position_s());
    // quantize snaps to the beat map, not the constant grid
    m.apply(Command::Quantize(0, true));
    m.apply(Command::SeekSeconds(0, 9.13));
    m.apply(Command::HotCuePress(0, 1));
    assert!((m.decks[0].hot_cue(1).unwrap() - 9.05).abs() < 1e-6);
    // beat loop of 2 beats at 150 BPM = 0.8 s
    m.apply(Command::BeatLoop(0, 2.0));
    let l = m.decks[0].loop_state().unwrap();
    assert!(((l.end - l.start) / f64::from(SR) - 0.8).abs() < 1e-3);
    let here = m.decks[0].position_s();
    assert!((m.state().decks[0].beat.unwrap() - m.decks[0].beat_at_s(here).unwrap()).abs() < 1e-6);
}

#[test]
fn sync_uses_the_local_tempo() {
    let mut m = deck_mixer();
    m.apply(Command::Load { deck: 1, pcm: Arc::new(Pcm::sine(SR, 330.0, 30.0, 0.2)), grid: Some(Grid { bpm: 125.0, first_beat_s: 0.0 }) });
    m.apply(Command::SeekSeconds(0, 10.0)); // 150 BPM region
    m.apply(Command::Play(0));
    m.apply(Command::Play(1));
    m.render_offline(0.3);
    m.apply(Command::Sync(0));
    assert!((m.decks[0].tempo() - 125.0 / 150.0).abs() < 1e-9, "tempo from the local 150 BPM");
    let own = m.decks[0].beat_at_s(m.decks[0].position_s()).unwrap().rem_euclid(1.0);
    let master = (m.decks[1].position_s() * 125.0 / 60.0).rem_euclid(1.0);
    assert!((own - master).abs() < 0.01 || (own - master).abs() > 0.99, "{own} vs {master}");
}

#[test]
fn analysis_from_a_library_track() {
    let t = LibTrack {
        anlz_dir: PathBuf::new(),
        stored_path: "?/A - B.flac".into(),
        file_name: "A - B.flac".into(),
        path: Some(PathBuf::from("A - B.flac")),
        candidates: 1,
        artist: Some("A".into()),
        title: "B".into(),
        beats: vec![
            Beat { beat: 2, tempo_x100: 12800, time_ms: 50 },
            Beat { beat: 3, tempo_x100: 12800, time_ms: 519 },
            Beat { beat: 4, tempo_x100: 12800, time_ms: 988 },
            Beat { beat: 1, tempo_x100: 12800, time_ms: 1456 },
        ],
        samples: None,
        cues: vec![Cue { slot: 2, start_ms: 3000, end_ms: None }, Cue { slot: 0, start_ms: 8000, end_ms: None }],
        structure: None,
        key: None,
        source: rekordbox_formats::library::Source::Rekordbox,
    };
    let a = TrackAnalysis::from_library(&t);
    assert_eq!(a.title, "A - B");
    assert_eq!(a.grid, Some(Grid { bpm: 128.0, first_beat_s: 1.456 }));
    let m = a.beats.as_ref().unwrap();
    assert_eq!((m.downbeat_index, m.bpm_at(0.6)), (3, 128.0));
    assert_eq!(a.hot_cues[1], Some(3.0));
    assert_eq!(a.memory_cues, [8.0]);
    assert!(!a.variable_tempo);
}

#[test]
fn browser_library_folder() {
    let root = std::env::temp_dir().join(format!("rbx-libview-{}", std::process::id()));
    std::fs::create_dir_all(root.join("Folder")).unwrap();
    let mut b = Browser::new(root.clone());
    assert_eq!(b.entries().len(), 1, "no library yet");
    b.set_library(vec![PathBuf::from("x/A - One.flac"), PathBuf::from("y/B - Two.mp3")]);
    assert_eq!(b.entries()[0], Entry::Library);
    assert_eq!(b.entries()[0].name(), "[Library]");
    assert!(b.enter());
    assert_eq!(b.view, View::Library);
    assert_eq!(b.entries().len(), 2);
    b.scroll(1);
    assert_eq!(b.selected_track(), Some(PathBuf::from("y/B - Two.mp3")));
    b.back();
    assert_eq!(b.view, View::Folder(root.clone()));
    let _ = std::fs::remove_dir_all(&root);
}
