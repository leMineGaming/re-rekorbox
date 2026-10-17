//! Audio engine tests on synthetic audio (sine, click tracks, ramps). No sound card needed.

use std::sync::Arc;

use rbx_engine::control::{parse, parse_script, render_script};
use rbx_engine::deck::{Deck, Grid};
use rbx_engine::mixer::{crossfader_gains, soft_clip, Command, Mixer};
use rbx_engine::pcm::{decode_file, write_wav, Pcm};

const SR: u32 = 44100;

/// Ramp whose sample value encodes its own position: frame i = i / len.
fn ramp(seconds: f64) -> Arc<Pcm> {
    let n = (seconds * f64::from(SR)) as usize;
    Arc::new(Pcm::new(SR, (0..n).map(|i| [i as f32 / n as f32; 2]).collect()))
}

fn mixer_with(deck0: Arc<Pcm>, grid: Option<Grid>) -> Mixer {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: deck0, grid });
    m
}

fn pos(m: &Mixer, d: usize) -> f64 {
    m.decks[d].position_s()
}

#[test]
fn wav_round_trip_through_decoder() {
    let dir = std::env::temp_dir().join("rbx-engine-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sine.wav");
    let src = Pcm::sine(48000, 1000.0, 0.5, 0.5);
    write_wav(&path, &src).unwrap();
    let back = decode_file(&path).unwrap();
    assert_eq!((back.sample_rate, back.len()), (48000, src.len()));
    let max_err = src.frames.iter().zip(&back.frames).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_err < 1.0 / 16000.0, "16-bit quantisation only, got {max_err}");
    assert!(decode_file(&dir.join("missing.flac")).is_err());
}

#[test]
fn interpolation_hits_samples_and_midpoints() {
    let p = Pcm::new(SR, (0..8).map(|i| [i as f32, -(i as f32)]).collect());
    assert_eq!(p.sample(3.0), [3.0, -3.0]);
    let mid = p.sample(3.5);
    assert!((mid[0] - 3.5).abs() < 1e-5, "linear data stays linear: {mid:?}");
    assert_eq!(p.at(-1), [0.0, 0.0]);
    assert_eq!(p.at(100), [0.0, 0.0]);
}

#[test]
fn playback_speed_follows_tempo_and_sample_rate() {
    let mut m = mixer_with(ramp(10.0), None);
    m.apply(Command::Play(0));
    m.render_offline(2.0);
    assert!((pos(&m, 0) - 2.0).abs() < 1e-3, "{}", pos(&m, 0));
    m.apply(Command::TempoRange(0, 1)); // ±10 %
    m.apply(Command::TempoFader(0, 1.0));
    m.render_offline(1.0);
    assert!((pos(&m, 0) - 3.1).abs() < 1e-3, "+10 % for one second: {}", pos(&m, 0));
    // a 48 kHz file on a 44.1 kHz mixer still plays in real time
    let p48 = Arc::new(Pcm::sine(48000, 440.0, 5.0, 0.1));
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: p48, grid: None });
    m.apply(Command::Play(0));
    m.render_offline(1.5);
    assert!((pos(&m, 0) - 1.5).abs() < 1e-3);
}

#[test]
fn output_level_and_end_of_track() {
    let mut m = mixer_with(Arc::new(Pcm::sine(SR, 440.0, 1.0, 0.5)), None);
    m.apply(Command::Play(0));
    let out = m.render_offline(0.5);
    let (g, _) = crossfader_gains(0.0);
    let peak = out.frames[1000..].iter().map(|f| f[0].abs()).fold(0.0f32, f32::max);
    assert!((peak - 0.5 * g).abs() < 0.01, "centre crossfader gain {g}, peak {peak}");
    assert!(out.frames[0][0].abs() < 1e-6, "declick ramp starts at zero");
    m.render_offline(1.0);
    assert!(!m.decks[0].is_playing(), "stops at the end");
    assert!((pos(&m, 0) - 1.0).abs() < 1e-6);
}

#[test]
fn cue_behaviour() {
    let mut d = Deck::new();
    d.load(ramp(10.0), None);
    d.seek_s(2.0);
    d.cue_press(); // paused away from cue: new cue at 2.0, preview
    assert!(d.is_playing());
    assert!((d.cue_s() - 2.0).abs() < 1e-9);
    let mut buf = vec![[0.0; 2]; 4410];
    d.render(&mut buf, SR, 1.0);
    d.cue_release();
    assert!(!d.is_playing());
    assert!((d.position_s() - 2.0).abs() < 1e-9, "back to the cue point");
    d.play();
    d.render(&mut buf, SR, 1.0);
    d.cue_press(); // playing: jump back and stop
    assert!(!d.is_playing() && (d.position_s() - 2.0).abs() < 1e-9);
}

#[test]
fn hot_cues() {
    let mut d = Deck::new();
    d.load(ramp(10.0), None);
    d.seek_s(1.5);
    d.hot_cue_press(0); // empty: store
    assert_eq!(d.hot_cue(0), Some(1.5));
    assert!(!d.is_playing());
    d.seek_s(5.0);
    d.hot_cue_press(0); // paused: jump and preview while held
    assert!(d.is_playing());
    let mut buf = vec![[0.0; 2]; 4410];
    d.render(&mut buf, SR, 1.0);
    d.hot_cue_release(0);
    assert!(!d.is_playing() && (d.position_s() - 1.5).abs() < 1e-9);
    d.play();
    d.hot_cue_press(0);
    d.hot_cue_release(0);
    assert!(d.is_playing(), "release only stops a preview");
    d.hot_cue_delete(0);
    assert_eq!(d.hot_cue(0), None);
}

#[test]
fn quantized_cues_snap_to_the_grid() {
    let mut d = Deck::new();
    d.load(ramp(10.0), Some(Grid { bpm: 120.0, first_beat_s: 0.1 }));
    d.quantize = true;
    d.seek_s(1.3);
    d.hot_cue_press(2);
    assert!((d.hot_cue(2).unwrap() - 1.1).abs() < 1e-6, "nearest beat of 0.1 + k·0.5");
}

#[test]
fn beat_loop_and_manual_loop() {
    let mut m = mixer_with(ramp(20.0), Some(Grid { bpm: 120.0, first_beat_s: 0.0 }));
    m.apply(Command::SeekSeconds(0, 4.0));
    m.apply(Command::Play(0));
    m.apply(Command::BeatLoop(0, 1.0)); // 0.5 s
    m.render_offline(3.0);
    let p = pos(&m, 0);
    assert!((4.0..4.5).contains(&p), "stays in the loop: {p}");
    m.apply(Command::LoopScale(0, 2.0));
    assert!((m.decks[0].loop_state().unwrap().end / f64::from(SR) - 5.0).abs() < 1e-6);
    m.apply(Command::ReloopExit(0)); // exit
    m.render_offline(2.0);
    assert!(pos(&m, 0) > 5.0, "continues after exit");
    m.apply(Command::ReloopExit(0)); // re-enter: jump to start
    assert!((pos(&m, 0) - 4.0).abs() < 1e-6);
    // manual loop
    m.apply(Command::SeekSeconds(0, 10.0));
    m.apply(Command::LoopIn(0));
    m.render_offline(0.25);
    m.apply(Command::LoopOut(0));
    m.render_offline(1.0);
    assert!((10.0..10.26).contains(&pos(&m, 0)), "{}", pos(&m, 0));
}

#[test]
fn sync_matches_tempo_and_phase() {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::click_track(SR, 128.0, 30.0)), grid: Some(Grid { bpm: 128.0, first_beat_s: 0.0 }) });
    m.apply(Command::Load { deck: 1, pcm: Arc::new(Pcm::click_track(SR, 124.0, 30.0)), grid: Some(Grid { bpm: 124.0, first_beat_s: 0.0 }) });
    m.apply(Command::Play(0));
    m.render_offline(1.3);
    m.apply(Command::SeekSeconds(1, 0.2));
    m.apply(Command::Play(1));
    m.apply(Command::Sync(1));
    assert!((m.decks[1].bpm().unwrap() - 128.0).abs() < 1e-9);
    assert!((m.decks[1].tempo() - 128.0 / 124.0).abs() < 1e-9);
    let phase = |m: &Mixer, d: usize, bpm: f64| (m.decks[d].position_s() * bpm / 60.0).rem_euclid(1.0);
    assert!((phase(&m, 0, 128.0) - phase(&m, 1, 124.0)).abs() < 1e-3, "beats aligned");
    m.render_offline(5.0);
    let diff = (phase(&m, 0, 128.0) - phase(&m, 1, 124.0)).abs();
    assert!(diff < 0.01 || diff > 0.99, "still aligned after 5 s: {diff}");
    // without a grid nothing happens
    let mut m2 = mixer_with(ramp(5.0), None);
    m2.apply(Command::Sync(0));
    assert_eq!(m2.decks[0].tempo(), 1.0);
}

#[test]
fn sync_widens_tempo_range_when_needed() {
    let mut d = Deck::new();
    d.set_tempo_range(0); // ±6 %
    d.set_tempo(1.12);
    assert!((d.tempo() - 1.12).abs() < 1e-9);
    assert!((d.tempo_range() - 0.16).abs() < 1e-9);
}

#[test]
fn jog_scratch_and_bend() {
    let mut m = mixer_with(ramp(10.0), None);
    m.apply(Command::SeekSeconds(0, 5.0));
    m.apply(Command::JogTouch(0, true));
    m.apply(Command::Jog(0, 2048)); // one revolution forward = 1.8 s
    m.render_offline(1.0);
    assert!((pos(&m, 0) - 6.8).abs() < 0.01, "platter followed: {}", pos(&m, 0));
    m.apply(Command::Jog(0, -1024));
    m.render_offline(1.0);
    assert!((pos(&m, 0) - 5.9).abs() < 0.01, "backwards: {}", pos(&m, 0));
    m.apply(Command::JogTouch(0, false));
    // pitch bend while playing: faster for a moment, then back to normal
    m.apply(Command::Play(0));
    m.apply(Command::Jog(0, 50));
    assert!(m.decks[0].speed() > 1.05);
    m.render_offline(1.0);
    assert!((m.decks[0].speed() - 1.0).abs() < 1e-3, "bend decayed");
    // fine seek while paused
    m.apply(Command::Pause(0));
    let before = pos(&m, 0);
    m.apply(Command::Jog(0, 100));
    assert!(pos(&m, 0) > before);
}

#[test]
fn crossfader_and_clipping() {
    for x in [-1.0f32, -0.5, 0.0, 0.3, 1.0] {
        let (a, b) = crossfader_gains(x);
        assert!((a * a + b * b - 1.0).abs() < 1e-6, "constant power at {x}");
    }
    assert!(crossfader_gains(-1.0).1.abs() < 1e-6);
    assert!(crossfader_gains(1.0).0.abs() < 1e-6);
    assert_eq!(soft_clip(0.5), 0.5);
    assert!(soft_clip(5.0) <= 1.0 && soft_clip(-5.0) >= -1.0);
    // two loud decks never exceed full scale
    let mut m = Mixer::new(SR);
    for d in 0..2 {
        m.apply(Command::Load { deck: d, pcm: Arc::new(Pcm::sine(SR, 100.0, 1.0, 1.0)), grid: None });
        m.apply(Command::Play(d));
        m.apply(Command::Trim(d, 2.0));
    }
    let out = m.render_offline(0.5);
    assert!(out.frames.iter().all(|f| f[0].abs() <= 1.0));
}

#[test]
fn commands_and_scripts() {
    assert!(matches!(parse("play 1"), Ok(Command::Play(0))));
    assert!(matches!(parse("hot 2 8"), Ok(Command::HotCuePress(1, 7))));
    assert!(matches!(parse("range 1 100"), Ok(Command::TempoRange(0, 3))));
    assert!(matches!(parse("bpm 2 128 0.25"), Ok(Command::SetGrid { deck: 1, grid: Some(g) }) if g.bpm == 128.0 && g.first_beat_s == 0.25));
    assert!(parse("play 3").is_err());
    assert!(parse("hot 1 9").is_err());
    assert!(parse("bpm 1 -5").is_err());
    assert!(parse("dance 1").is_err());
    let s = parse_script("2:play 2; 0:play 1").unwrap();
    assert_eq!(s.iter().map(|e| e.0).collect::<Vec<_>>(), [0.0, 2.0], "sorted by time");
    assert!(parse_script("play 1").is_err());

    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: ramp(10.0), grid: None });
    m.apply(Command::Load { deck: 1, pcm: ramp(10.0), grid: None });
    let out = render_script(&mut m, "0:play 1; 1.5:play 2; 2:pause 1", 3.0).unwrap();
    assert_eq!(out.len(), 3 * SR as usize);
    assert!((pos(&m, 0) - 2.0).abs() < 1e-3, "deck 1 paused at 2 s");
    assert!((pos(&m, 1) - 1.5).abs() < 1e-3, "deck 2 played 1.5 s");
}
