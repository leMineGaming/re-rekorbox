//! Reverse, slip, master tempo, key shift, keyboard, vinyl brake, memory cues, auto cue, search,
//! instant double and sync master – synthetic audio.

use std::sync::Arc;

use rbx_engine::deck::Grid;
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::pcm::{Frame, Pcm};

const SR: u32 = 44100;

fn ramp(seconds: f64) -> Arc<Pcm> {
    let n = (seconds * f64::from(SR)) as usize;
    Arc::new(Pcm::new(SR, (0..n).map(|i| [i as f32 / n as f32; 2]).collect()))
}

fn mixer(pcm: Arc<Pcm>) -> Mixer {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm, grid: Some(Grid { bpm: 120.0, first_beat_s: 0.0 }) });
    m.apply(Command::Crossfader(-1.0));
    m
}

fn pos(m: &Mixer) -> f64 {
    m.decks[0].position_s()
}

fn freq(x: &[Frame]) -> f32 {
    let c = x.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
    c as f32 * SR as f32 / x.len() as f32
}

#[test]
fn reverse_plays_backwards() {
    let mut m = mixer(ramp(10.0));
    m.apply(Command::SeekSeconds(0, 5.0));
    m.apply(Command::Play(0));
    m.apply(Command::Reverse(0, true));
    m.render_offline(1.0);
    assert!((pos(&m) - 4.0).abs() < 0.01, "{}", pos(&m));
    assert!(m.state().decks[0].speed < 0.0);
    m.apply(Command::Reverse(0, false));
    m.render_offline(1.0);
    assert!((pos(&m) - 5.0).abs() < 0.01);
}

#[test]
fn slip_returns_to_the_background_position() {
    let mut m = mixer(ramp(20.0));
    m.apply(Command::SeekSeconds(0, 2.0));
    m.apply(Command::Slip(0, true));
    m.apply(Command::Play(0));
    m.render_offline(1.0); // at 3.0
    m.apply(Command::Reverse(0, true));
    m.render_offline(1.0); // audible position 2.0, background 4.0
    assert!(m.state().decks[0].slip_diverged);
    assert!((pos(&m) - 2.0).abs() < 0.01);
    m.apply(Command::Reverse(0, false));
    m.render_offline(0.01);
    assert!((pos(&m) - 4.01).abs() < 0.02, "back on the background position: {}", pos(&m));
    // slip loop: a 1-beat loop for 2 s, then exit continues where the track would be
    m.apply(Command::BeatLoop(0, 1.0));
    m.render_offline(2.0);
    m.apply(Command::ReloopExit(0));
    m.render_offline(0.01);
    assert!((pos(&m) - 6.02).abs() < 0.03, "{}", pos(&m));
}

#[test]
fn master_tempo_keeps_pitch() {
    let tone = Arc::new(Pcm::sine(SR, 440.0, 5.0, 0.5));
    let mut m = mixer(tone);
    m.apply(Command::Play(0));
    m.apply(Command::TempoRange(0, 2));
    m.apply(Command::TempoFader(0, 1.0)); // +16 %
    let plain = m.render_offline(1.0);
    assert!((freq(&plain.frames[10000..]) - 510.4).abs() < 10.0, "pitch follows tempo without key lock");
    m.apply(Command::MasterTempo(0, true));
    m.render_offline(0.2);
    let locked = m.render_offline(1.0);
    assert!((freq(&locked.frames) - 440.0).abs() < 15.0, "key lock: {}", freq(&locked.frames));
    assert!(m.state().decks[0].master_tempo);
}

#[test]
fn key_shift_and_keyboard() {
    let tone = Arc::new(Pcm::sine(SR, 440.0, 5.0, 0.5));
    let mut m = mixer(tone);
    m.apply(Command::Play(0));
    m.apply(Command::Key(0, 12));
    m.render_offline(0.2);
    let up = m.render_offline(1.0);
    assert!((freq(&up.frames) - 880.0).abs() < 30.0, "+12 = one octave: {}", freq(&up.frames));
    assert_eq!(m.state().decks[0].key, 12);
    m.apply(Command::Key(0, 99));
    assert_eq!(m.state().decks[0].key, 12, "clamped");
    // keyboard from pause: jumps to the first hot cue, plays while held
    m.apply(Command::Pause(0));
    m.apply(Command::SeekSeconds(0, 1.0));
    m.apply(Command::HotCuePress(0, 2));
    m.apply(Command::SeekSeconds(0, 3.0));
    m.apply(Command::KeyboardPress(0, 7));
    assert!(m.decks[0].is_playing());
    assert!((pos(&m) - 1.0).abs() < 1e-6);
    assert_eq!(m.decks[0].key(), 7);
    m.render_offline(0.5);
    m.apply(Command::KeyboardRelease(0));
    assert!(!m.decks[0].is_playing() && (pos(&m) - 1.0).abs() < 1e-6);
}

#[test]
fn vinyl_brake_and_start() {
    let mut m = mixer(ramp(20.0));
    m.apply(Command::Brake(0, 1.0));
    m.apply(Command::Play(0));
    m.render_offline(0.5);
    assert!(pos(&m) < 0.3, "starting up takes a second: {}", pos(&m));
    m.render_offline(2.0);
    let before = pos(&m);
    m.apply(Command::Pause(0));
    assert!(m.decks[0].is_playing(), "still turning while braking");
    m.render_offline(1.5);
    assert!(!m.decks[0].is_playing());
    let moved = pos(&m) - before;
    assert!((moved - 0.5).abs() < 0.02, "linear brake over 1 s covers half a second: {moved}");
    assert_eq!(m.state().decks[0].brake_s, 1.0);
}

#[test]
fn memory_cues_store_call_delete() {
    let mut m = mixer(ramp(30.0));
    for t in [5.0, 15.0, 10.0] {
        m.apply(Command::SeekSeconds(0, t));
        m.apply(Command::MemoryStore(0));
    }
    m.apply(Command::MemoryStore(0)); // duplicate ignored
    assert_eq!(m.decks[0].memory_cues_s(), [5.0, 10.0, 15.0]);
    m.apply(Command::SeekSeconds(0, 0.0));
    m.apply(Command::MemoryCall(0, true));
    assert_eq!((pos(&m), m.decks[0].cue_s()), (5.0, 5.0), "called memory becomes the cue point");
    m.apply(Command::MemoryCall(0, true));
    assert_eq!(pos(&m), 10.0);
    m.apply(Command::MemoryCall(0, false));
    assert_eq!(pos(&m), 5.0);
    m.apply(Command::MemoryDelete(0));
    assert_eq!(m.decks[0].memory_cues_s(), [10.0, 15.0]);
    assert_eq!(m.state().decks[0].memory_cues, 2);
}

#[test]
fn auto_cue_skips_leading_silence() {
    let mut frames = vec![[0.0; 2]; 22050];
    frames.extend((0..44100).map(|_| [0.5, 0.5]));
    let mut m = Mixer::new(SR);
    m.apply(Command::AutoCue(0, true));
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::new(SR, frames)), grid: None });
    assert!((pos(&m) - 0.5).abs() < 1e-6);
    assert!((m.decks[0].cue_s() - 0.5).abs() < 1e-6);
}

#[test]
fn search_buttons() {
    let mut m = mixer(ramp(30.0));
    m.apply(Command::Search(0, 4.0));
    m.render_offline(1.0);
    assert!((pos(&m) - 4.0).abs() < 0.01);
    m.apply(Command::Search(0, -4.0));
    m.render_offline(0.5);
    assert!((pos(&m) - 2.0).abs() < 0.01);
    m.apply(Command::Search(0, 0.0));
    m.render_offline(0.5);
    assert!((pos(&m) - 2.0).abs() < 0.01, "released: stays paused");
}

#[test]
fn instant_double_and_sync_master() {
    let mut m = mixer(ramp(30.0));
    m.apply(Command::SeekSeconds(0, 7.0));
    m.apply(Command::HotCuePress(0, 0));
    m.apply(Command::Play(0));
    m.render_offline(1.0);
    m.apply(Command::InstantDouble(1));
    assert!(m.decks[1].is_playing());
    assert!((m.decks[1].position_s() - pos(&m)).abs() < 1e-9);
    assert_eq!(m.decks[1].hot_cue(0), Some(7.0));
    // master choice: deck 2 as master, deck 1 syncs to it
    m.apply(Command::TempoFader(1, 0.5));
    m.apply(Command::SetMaster(Some(1)));
    m.apply(Command::Sync(0));
    assert!((m.decks[0].bpm().unwrap() - m.decks[1].bpm().unwrap()).abs() < 1e-9);
    assert_eq!(m.state().master, Some(1));
}
