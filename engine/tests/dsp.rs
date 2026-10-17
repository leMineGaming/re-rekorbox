//! DSP tests: frequency response measured with sine tones, effects checked by level and timing.

use std::sync::Arc;

use rbx_engine::dsp::{eq_gain, Biquad, ColorFx, ColorProcessor, Eq3, Shape};
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::pcm::{Frame, Pcm};

const SR: u32 = 44100;

fn sine(freq: f32, n: usize) -> Vec<Frame> {
    (0..n).map(|i| {
        let v = (std::f32::consts::TAU * freq * i as f32 / SR as f32).sin() * 0.5;
        [v, v]
    }).collect()
}

/// RMS of the left channel over the second half (steady state).
fn rms(x: &[Frame]) -> f32 {
    let tail = &x[x.len() / 2..];
    (tail.iter().map(|f| f[0] * f[0]).sum::<f32>() / tail.len() as f32).sqrt()
}

fn db(ratio: f32) -> f32 {
    20.0 * ratio.max(1e-9).log10()
}

/// Gain in dB of `f` applied to a sine of `freq`.
fn response(freq: f32, mut f: impl FnMut(Frame) -> Frame) -> f32 {
    let x = sine(freq, SR as usize / 2);
    let y: Vec<Frame> = x.iter().map(|&s| f(s)).collect();
    db(rms(&y) / rms(&x))
}

#[test]
fn eq_knob_curve() {
    assert!((eq_gain(0.5) - 1.0).abs() < 1e-6);
    assert!((db(eq_gain(1.0)) - 6.0).abs() < 0.01);
    assert!((db(eq_gain(0.05)) + 26.0).abs() < 0.01);
    assert_eq!(eq_gain(0.0), 0.0, "kill");
    assert!(eq_gain(0.3) < 1.0 && eq_gain(0.3) > eq_gain(0.1));
}

#[test]
fn biquad_shapes() {
    let mut lp = Biquad::new(Shape::LowPass, SR as f32, 1000.0, 0.707);
    assert!(response(100.0, |x| lp.process(x)).abs() < 0.5);
    let mut lp = Biquad::new(Shape::LowPass, SR as f32, 1000.0, 0.707);
    assert!(response(10000.0, |x| lp.process(x)) < -35.0);
    let mut hp = Biquad::new(Shape::HighPass, SR as f32, 1000.0, 0.707);
    assert!(response(100.0, |x| hp.process(x)) < -35.0);
    let mut pk = Biquad::new(Shape::Peak(6.0), SR as f32, 1000.0, 1.0);
    assert!((response(1000.0, |x| pk.process(x)) - 6.0).abs() < 0.3);
}

#[test]
fn isolator_eq_flat_and_kills() {
    for f in [60.0, 300.0, 1000.0, 4000.0, 12000.0] {
        let mut eq = Eq3::new(SR);
        let g = response(f, |x| eq.process(x));
        assert!(g.abs() < 1.5, "flat EQ at {f} Hz: {g:.2} dB");
    }
    let mut eq = Eq3::new(SR);
    eq.knobs = [0.0, 0.5, 0.5];
    assert!(response(60.0, |x| eq.process(x)) < -30.0, "low kill");
    let mut eq = Eq3::new(SR);
    eq.knobs = [0.0, 0.5, 0.5];
    assert!(response(10000.0, |x| eq.process(x)).abs() < 1.5, "high band untouched");
    let mut eq = Eq3::new(SR);
    eq.knobs = [0.5, 0.0, 0.5];
    assert!(response(1200.0, |x| eq.process(x)) < -20.0, "mid kill");
    let mut eq = Eq3::new(SR);
    eq.knobs = [0.5, 0.5, 1.0];
    assert!((response(12000.0, |x| eq.process(x)) - 6.0).abs() < 1.0, "+6 dB high");
}

#[test]
fn color_filter_both_directions() {
    let mut c = ColorProcessor::new(SR);
    assert_eq!(c.process(Some(ColorFx::Filter), 0.5, 0.5, [0.3, -0.2]), [0.3, -0.2], "centre = bypass");
    let mut c = ColorProcessor::new(SR);
    assert!(response(5000.0, |x| c.process(Some(ColorFx::Filter), 0.0, 0.0, x)) < -30.0, "low-pass side");
    let mut c = ColorProcessor::new(SR);
    assert!(response(80.0, |x| c.process(Some(ColorFx::Filter), 1.0, 0.0, x)) < -30.0, "high-pass side");
    let mut c = ColorProcessor::new(SR);
    assert_eq!(c.process(None, 0.0, 0.5, [0.3, 0.3]), [0.3, 0.3], "no type selected");
}

#[test]
fn time_based_color_fx_leave_a_tail() {
    // a short burst, then silence: SPACE and DUB ECHO keep sounding
    for fx in [ColorFx::Space, ColorFx::DubEcho] {
        let mut c = ColorProcessor::new(SR);
        c.echo_s = 0.25;
        let mut tail = 0.0f32;
        for i in 0..SR as usize {
            let x = if i < 2000 { [0.5, 0.5] } else { [0.0, 0.0] };
            let y = c.process(Some(fx), 1.0, 0.5, x);
            if i > 12000 {
                tail = tail.max(y[0].abs());
            }
        }
        assert!(tail > 0.01, "{fx:?} tail {tail}");
    }
}

#[test]
fn crush_and_noise_stay_bounded() {
    for fx in [ColorFx::Crush, ColorFx::Noise, ColorFx::Sweep] {
        let mut c = ColorProcessor::new(SR);
        let x = sine(440.0, SR as usize / 4);
        let mut changed = false;
        for f in &x {
            let y = c.process(Some(fx), 0.95, 0.8, *f);
            assert!(y[0].is_finite() && y[0].abs() < 4.0, "{fx:?}");
            changed |= (y[0] - f[0]).abs() > 1e-3;
        }
        assert!(changed, "{fx:?} audible");
    }
}

#[test]
fn mixer_channel_strip() {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::sine(SR, 60.0, 2.0, 0.5)), grid: None });
    m.apply(Command::Play(0));
    m.apply(Command::Crossfader(-1.0));
    let full = rms(&m.render_offline(0.5).frames);
    m.apply(Command::Eq(0, 0, 0.0));
    m.render_offline(0.1);
    let killed = rms(&m.render_offline(0.5).frames);
    assert!(db(killed / full) < -30.0, "low kill on deck 1: {:.1} dB", db(killed / full));
    m.apply(Command::Eq(0, 0, 0.5));
    m.apply(Command::ColorFxSelect(Some(ColorFx::Filter)));
    m.apply(Command::ColorKnob(0, 1.0)); // high-pass removes 60 Hz
    m.render_offline(0.1);
    let filtered = rms(&m.render_offline(0.5).frames);
    assert!(db(filtered / full) < -20.0);
    let s = m.state();
    assert_eq!((s.color_fx, s.eq[0]), (Some(ColorFx::Filter), [0.5, 0.5, 0.5]));
    assert!(s.channel_peak[0] < 0.1);
}
