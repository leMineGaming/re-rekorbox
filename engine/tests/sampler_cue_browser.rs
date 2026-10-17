//! Sampler, headphone cue bus, crossfader curve and the file browser (temporary folder with WAVs
//! written by the test itself).

use std::path::PathBuf;
use std::sync::Arc;

use rbx_engine::browser::{Browser, Entry, View};
use rbx_engine::mixer::{crossfader_gains_curve, Command, Mixer};
use rbx_engine::pcm::{write_wav, Frame, Pcm};
use rbx_engine::sampler::{builtin_sounds, Sampler, BUILTIN_NAMES, SLOTS};

const SR: u32 = 44100;

fn rms(x: &[Frame]) -> f32 {
    (x.iter().map(|f| f[0] * f[0]).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn builtin_sounds_are_usable() {
    let sounds = builtin_sounds(SR);
    assert_eq!((sounds.len(), BUILTIN_NAMES.len()), (SLOTS, SLOTS));
    for (s, name) in sounds.iter().zip(BUILTIN_NAMES) {
        assert!(s.len() > 1000, "{name} has content");
        assert!(s.frames.iter().all(|f| f[0].abs() <= 1.0), "{name} within full scale");
        assert!(rms(&s.frames) > 0.001, "{name} audible");
    }
}

#[test]
fn sampler_one_shots() {
    let mut s = Sampler::default();
    s.load(3, Arc::new(Pcm::sine(SR, 440.0, 0.1, 0.5)));
    assert!(s.is_loaded(3) && !s.is_loaded(4));
    s.start(4); // empty: nothing
    s.start(3);
    assert_eq!(s.playing_mask(), 1 << 3);
    let mut out = vec![[0.0; 2]; 2205];
    s.render(&mut out, SR);
    assert!(rms(&out) > 0.1);
    let mut out = vec![[0.0; 2]; 4410];
    s.render(&mut out, SR);
    assert!(!s.is_playing(3), "stops at the end of the sample");
    s.start(3);
    s.stop(3);
    assert_eq!(s.playing_mask(), 0);
}

#[test]
fn sampler_bus_in_the_mixer_ignores_the_crossfader() {
    let mut m = Mixer::new(SR);
    m.apply(Command::SamplerLoad(0, Arc::new(Pcm::sine(SR, 440.0, 1.0, 0.5))));
    m.apply(Command::Crossfader(1.0));
    m.apply(Command::SamplerStart(0));
    let full = rms(&m.render_offline(0.2).frames);
    assert!(full > 0.2);
    m.apply(Command::SamplerVolume(0.0));
    assert!(rms(&m.render_offline(0.2).frames) < 1e-6);
    let s = m.state();
    assert_eq!((s.sampler_loaded & 1, s.sampler_playing & 1), (1, 1));
}

#[test]
fn headphone_cue_bus() {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::sine(SR, 300.0, 5.0, 0.5)), grid: None });
    m.apply(Command::Play(0));
    m.apply(Command::Volume(0, 0.0)); // channel fader down: silent on master
    let n = SR as usize / 5;
    let (mut out, mut cue) = (vec![[0.0; 2]; n], vec![[0.0; 2]; n]);
    m.render_with_cue(&mut out, &mut cue);
    assert!(rms(&out[n / 2..]) < 1e-3 && rms(&cue) < 1e-6, "no cue selected");
    m.apply(Command::HeadphoneCue(0, true));
    m.render_with_cue(&mut out, &mut cue);
    assert!(rms(&cue[n / 2..]) > 0.3, "pre-fader cue: audible in the headphones only");
    assert!(rms(&out[n / 2..]) < 1e-3);
    // cue link: selecting deck 2 clears deck 1
    m.apply(Command::CueLink(true));
    m.apply(Command::HeadphoneCue(1, true));
    assert_eq!(m.state().cue, [false, true]);
    // master cue
    m.apply(Command::Volume(0, 1.0));
    m.apply(Command::MasterCue(true));
    m.render_with_cue(&mut out, &mut cue);
    assert!(rms(&cue[n / 2..]) > 0.2);
}

#[test]
fn crossfader_curve() {
    let (a, b) = crossfader_gains_curve(0.0, 0.0);
    assert!((a - b).abs() < 1e-6 && (a * a + b * b - 1.0).abs() < 1e-6, "smooth: constant power");
    let (a, b) = crossfader_gains_curve(0.0, 1.0);
    assert_eq!((a, b), (1.0, 1.0), "sharp: both full in the middle");
    let (a, _) = crossfader_gains_curve(0.95, 1.0);
    assert!((a - 0.5).abs() < 1e-3, "sharp cut in the last 5 %");
    assert_eq!(crossfader_gains_curve(1.0, 1.0), (0.0, 1.0));
}

fn temp_library() -> PathBuf {
    let root = std::env::temp_dir().join(format!("rbx-browser-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("House")).unwrap();
    std::fs::create_dir_all(root.join("Techno")).unwrap();
    let tone = Pcm::sine(SR, 440.0, 0.2, 0.3);
    for f in ["House/Artist A - One.wav", "House/Artist A - Two.wav", "House/Artist B - Three.wav", "Techno/X - Y.wav"] {
        write_wav(&root.join(f), &tone).unwrap();
    }
    std::fs::write(root.join("House/notes.txt"), "not audio").unwrap();
    root
}

#[test]
fn browser_navigation_tags_and_related() {
    let root = temp_library();
    let mut b = Browser::new(root.clone());
    assert_eq!(b.entries().iter().map(Entry::name).collect::<Vec<_>>(), ["House", "Techno"]);
    assert!(b.selected_track().is_none());
    assert!(b.enter());
    assert_eq!(b.entries().len(), 3, "audio files only");
    b.scroll(10);
    assert_eq!(b.selected, 2, "clamped at the end");
    b.scroll(-2);
    assert_eq!(b.selected_track().unwrap().file_name().unwrap(), "Artist A - One.wav");
    b.toggle_tag();
    b.scroll(1);
    b.toggle_tag();
    assert_eq!(b.tags.len(), 2);
    b.toggle_tag_view();
    assert_eq!((b.view.clone(), b.entries().len()), (View::TagList, 2));
    b.back();
    assert_eq!(b.view, View::Folder(root.clone()));
    b.enter();
    b.show_related(); // selected: "Artist A - One.wav" -> "Artist A - Two.wav"
    assert_eq!(b.entries().iter().map(Entry::name).collect::<Vec<_>>(), ["Artist A - Two.wav"]);
    b.back();
    b.enter();
    b.back();
    assert_eq!(b.selected_entry().unwrap().name(), "House", "back selects the folder we came from");
    b.back();
    assert_eq!(b.view, View::Folder(root.clone()), "not above the root");
    let two = root.join("House/Artist A - Two.wav");
    assert_eq!(Browser::neighbour(&two, true).unwrap().file_name().unwrap(), "Artist B - Three.wav");
    assert_eq!(Browser::neighbour(&two, false).unwrap().file_name().unwrap(), "Artist A - One.wav");
    assert!(Browser::neighbour(&root.join("Techno/X - Y.wav"), true).is_none());
    assert!(b.describe().contains("House/"));
    let _ = std::fs::remove_dir_all(&root);
}
