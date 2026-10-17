//! Beat FX / Pad FX tests with synthetic signals.

use std::sync::Arc;

use rbx_engine::deck::Grid;
use rbx_engine::fx::{BeatFx, BeatFxKind, FxAssign, TapTempo, BEATS, BEAT_ONE};
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::pcm::{Frame, Pcm};

const SR: u32 = 44100;

fn run(fx: &mut BeatFx, input: &[Frame], bpm: f64) -> Vec<Frame> {
    input.iter().map(|&x| fx.process(x, bpm)).collect()
}

fn impulse(n: usize) -> Vec<Frame> {
    let mut v = vec![[0.0; 2]; n];
    v[0] = [1.0, 1.0];
    v
}

fn sine(freq: f32, n: usize) -> Vec<Frame> {
    (0..n).map(|i| {
        let v = (std::f32::consts::TAU * freq * i as f32 / SR as f32).sin() * 0.5;
        [v, v]
    }).collect()
}

/// Frequency estimate from rising zero crossings of the left channel.
fn freq(x: &[Frame]) -> f32 {
    let crossings = x.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
    crossings as f32 * SR as f32 / x.len() as f32
}

fn fx(kind: BeatFxKind) -> BeatFx {
    let mut f = BeatFx::new(SR, kind);
    f.on = true;
    f.depth = 1.0;
    f
}

#[test]
fn delay_repeats_after_one_beat() {
    let mut f = fx(BeatFxKind::Delay);
    let y = run(&mut f, &impulse(SR as usize), 120.0);
    // the dry impulse at 0, the first repeat one beat (0.5 s) later
    let (i, _) = y[1000..].iter().enumerate().max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs())).unwrap();
    let at = i + 1000;
    assert!((at as i64 - 22050).abs() < 10, "repeat at {at}");
    assert_eq!(BEATS[BEAT_ONE], 1.0);
}

#[test]
fn echo_tail_continues_after_off() {
    let mut f = fx(BeatFxKind::Echo);
    let burst: Vec<Frame> = (0..SR as usize).map(|i| if i < 1000 { [0.5, 0.5] } else { [0.0, 0.0] }).collect();
    run(&mut f, &burst[..5000], 120.0);
    f.on = false;
    let tail = run(&mut f, &burst[5000..], 120.0);
    let level = tail.iter().map(|x| x[0].abs()).fold(0.0f32, f32::max);
    assert!(level > 0.05, "echo still audible after switching off: {level}");
}

#[test]
fn trans_gates_half_of_each_period() {
    let mut f = fx(BeatFxKind::Trans);
    f.beat_index = 3; // 1/2 beat = 0.25 s at 120 BPM
    let input = vec![[0.5, 0.5]; SR as usize];
    let y = run(&mut f, &input, 120.0);
    let silent = y[2000..].iter().filter(|x| x[0].abs() < 0.01).count() as f32 / (y.len() - 2000) as f32;
    assert!((silent - 0.5).abs() < 0.05, "gate closed {silent:.2} of the time");
}

#[test]
fn roll_repeats_the_captured_slice() {
    let mut f = fx(BeatFxKind::Roll);
    f.beat_index = 2; // 1/4 beat at 120 BPM = 5512 frames
    let ramp: Vec<Frame> = (0..SR as usize).map(|i| [i as f32 / SR as f32; 2]).collect();
    let y = run(&mut f, &ramp, 120.0);
    let len = (0.125 * SR as f64) as usize;
    // after capturing, the output repeats the first slice instead of following the ramp
    assert!((y[len + 100][0] - ramp[100][0]).abs() < 1e-4, "{} {} {}", y[len + 100][0], ramp[100][0], y[len + 99][0]);
    assert!((y[3 * len + 100][0] - ramp[100][0]).abs() < 1e-4);
    f.on = false;
    let after = f.process(ramp[30000], 120.0);
    assert_eq!(after, ramp[30000], "off: dry again");
}

#[test]
fn pitch_shifts_up_an_octave() {
    let mut f = fx(BeatFxKind::Pitch); // depth 1.0 = +12 semitones
    let y = run(&mut f, &sine(440.0, SR as usize), 120.0);
    let est = freq(&y[SR as usize / 2..]);
    assert!((est - 880.0).abs() < 40.0, "{est} Hz");
    f.depth = 0.5;
    let y = run(&mut f, &sine(440.0, SR as usize), 120.0);
    assert!((freq(&y[SR as usize / 2..]) - 440.0).abs() < 10.0, "centre = no shift");
}

#[test]
fn vinyl_brake_slows_to_silence() {
    let mut f = fx(BeatFxKind::VinylBrake);
    let y = run(&mut f, &sine(440.0, SR as usize), 120.0);
    let early = freq(&y[2000..8000]);
    let later = freq(&y[12000..20000]);
    assert!(later < early * 0.8, "pitch falls while braking: {early} -> {later}");
}

#[test]
fn all_effects_are_bounded_and_audible() {
    for kind in BeatFxKind::ALL {
        let mut f = fx(kind);
        let input = sine(220.0, SR as usize);
        let y = run(&mut f, &input, 128.0);
        assert!(y.iter().all(|x| x[0].is_finite() && x[0].abs() < 4.0), "{kind:?} bounded");
        let diff = y.iter().zip(&input).map(|(a, b)| (a[0] - b[0]).abs()).fold(0.0f32, f32::max);
        assert!(diff > 1e-3, "{kind:?} changes the signal");
    }
    assert_eq!(BeatFxKind::from_name("PINGPONG"), Some(BeatFxKind::PingPong));
    assert_eq!(BeatFxKind::VinylBrake.next(), BeatFxKind::Delay);
}

#[test]
fn tap_tempo() {
    let mut t = TapTempo::default();
    assert_eq!(t.tap(10.0), None);
    assert!((t.tap(10.5).unwrap() - 120.0).abs() < 1e-9);
    assert!((t.tap(11.0).unwrap() - 120.0).abs() < 1e-9);
    assert_eq!(t.tap(20.0), None, "long pause restarts");
}

#[test]
fn mixer_beat_fx_section() {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::click_track(SR, 126.0, 10.0)), grid: Some(Grid { bpm: 126.0, first_beat_s: 0.0 }) });
    m.apply(Command::Play(0));
    assert!((m.fx_tempo() - 126.0).abs() < 1e-9, "auto tempo from the playing deck");
    m.apply(Command::BeatFxBpm(Some(100.0)));
    assert_eq!(m.state().beat_fx.bpm, 100.0);
    assert!(!m.state().beat_fx.auto_bpm);
    m.apply(Command::BeatFxBpm(None));
    for _ in 0..20 {
        m.apply(Command::BeatFxBeat(1));
    }
    assert_eq!(m.state().beat_fx.beats, 16.0, "clamped at the top");
    m.apply(Command::BeatFxSelect(BeatFxKind::Trans));
    m.apply(Command::BeatFxAssign(FxAssign::Channel(0)));
    m.apply(Command::BeatFxOn(true));
    let s = m.state().beat_fx;
    assert_eq!((s.on, s.kind, s.assign), (true, BeatFxKind::Trans, FxAssign::Channel(0)));
    let out = m.render_offline(1.0);
    assert!(out.frames.iter().all(|f| f[0].is_finite()));
    // pad FX while held
    m.apply(Command::PadFx(0, Some((0, 3))));
    assert_eq!(m.state().pad_fx[0], Some((0, 3)));
    m.render_offline(0.2);
    m.apply(Command::PadFx(0, None));
    assert_eq!(m.state().pad_fx[0], None);
}
