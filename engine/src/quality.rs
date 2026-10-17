//! Automatic audio checks instead of listening: typical operations are rendered offline with the real
//! mixer and the result is measured (clicks at jumps and fader moves, pitch with key lock, sync drift,
//! echo timing, clipping, routing). Used by the tests and by `rbx-selftest`.

use std::sync::Arc;

use crate::deck::Grid;
use crate::fx::{BeatFxKind, FxAssign};
use crate::mixer::{Command, Mixer, MixerState};
use crate::pcm::{Frame, Pcm};

const SR: u32 = 44100;

/// One measured property.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub group: &'static str,
    pub name: String,
    pub passed: bool,
    pub value: f64,
    pub limit: f64,
    pub unit: &'static str,
    /// What was measured and what it means.
    pub detail: String,
}

impl Check {
    fn at_most(group: &'static str, name: &str, value: f64, limit: f64, unit: &'static str, detail: &str) -> Check {
        Check { group, name: name.into(), passed: value <= limit, value, limit, unit, detail: detail.into() }
    }

    fn near(group: &'static str, name: &str, value: f64, target: f64, tol: f64, unit: &'static str, detail: &str) -> Check {
        Check {
            group,
            name: name.into(),
            passed: (value - target).abs() <= tol,
            value,
            limit: tol,
            unit,
            detail: format!("{detail} (Soll {target:.2} ± {tol})"),
        }
    }
}

/// Rendered master and headphone bus plus the mixer state after every block.
pub struct Render {
    pub master: Vec<Frame>,
    pub cue: Vec<Frame>,
    pub states: Vec<(f64, MixerState)>,
}

/// Render `seconds` while applying timed commands (block size 10 ms, commands at block starts).
pub fn render(m: &mut Mixer, mut events: Vec<(f64, Command)>, seconds: f64) -> Render {
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut events = events.into_iter().peekable();
    let total = (seconds * f64::from(m.sample_rate)) as usize;
    let block = (m.sample_rate / 100) as usize;
    let (mut master, mut cue) = (vec![[0.0f32; 2]; total], vec![[0.0f32; 2]; total]);
    let mut states = Vec::with_capacity(total / block + 1);
    let mut done = 0;
    while done < total {
        let t = done as f64 / f64::from(m.sample_rate);
        while events.peek().is_some_and(|(at, _)| *at <= t + 1e-9) {
            m.apply(events.next().unwrap().1);
        }
        let end = (done + block).min(total);
        m.render_with_cue(&mut master[done..end], &mut cue[done..end]);
        states.push((end as f64 / f64::from(m.sample_rate), m.state()));
        done = end;
    }
    Render { master, cue, states }
}

/// Largest second difference |x[n] − 2x[n−1] + x[n−2]| after `from_s` (both channels) and its time.
/// A 440 Hz sine at 0.5 has about 0.002; a hard jump gives up to the jump size.
pub fn max_step(frames: &[Frame], from_s: f64) -> (f64, f64) {
    let start = ((from_s * f64::from(SR)) as usize).max(2);
    let mut best = (0.0f64, 0.0f64);
    for n in start..frames.len() {
        for c in 0..2 {
            let d = f64::from(frames[n][c] - 2.0 * frames[n - 1][c] + frames[n - 2][c]).abs();
            if d > best.0 {
                best = (d, n as f64 / f64::from(SR));
            }
        }
    }
    best
}

/// Frequency from rising zero crossings in `[a, b]` seconds (left channel).
pub fn frequency(frames: &[Frame], a: f64, b: f64) -> f64 {
    let (s, e) = ((a * f64::from(SR)) as usize, ((b * f64::from(SR)) as usize).min(frames.len()));
    let mut crossings = Vec::new();
    for n in s.max(1)..e {
        let (p, q) = (frames[n - 1][0], frames[n][0]);
        if p < 0.0 && q >= 0.0 {
            crossings.push(n as f64 - 1.0 + f64::from(-p / (q - p)));
        }
    }
    if crossings.len() < 2 {
        return 0.0;
    }
    (crossings.len() - 1) as f64 / ((crossings[crossings.len() - 1] - crossings[0]) / f64::from(SR))
}

fn rms(frames: &[Frame], a: f64, b: f64) -> f64 {
    let (s, e) = ((a * f64::from(SR)) as usize, ((b * f64::from(SR)) as usize).min(frames.len()));
    if e <= s {
        return 0.0;
    }
    (frames[s..e].iter().map(|f| f64::from(f[0]).powi(2)).sum::<f64>() / (e - s) as f64).sqrt()
}

fn peak(frames: &[Frame]) -> f64 {
    frames.iter().map(|f| f64::from(f[0].abs().max(f[1].abs()))).fold(0.0, f64::max)
}

/// Times where the absolute level rises above `threshold` after at least `gap_s` below it.
pub fn onsets(frames: &[Frame], threshold: f32, gap_s: f64) -> Vec<f64> {
    let gap = (gap_s * f64::from(SR)) as usize;
    let mut out = Vec::new();
    let mut quiet = gap;
    for (n, f) in frames.iter().enumerate() {
        if f[0].abs().max(f[1].abs()) >= threshold {
            if quiet >= gap {
                out.push(n as f64 / f64::from(SR));
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

fn sine(freq: f64, seconds: f64) -> Arc<Pcm> {
    Arc::new(Pcm::sine(SR, freq, seconds, 0.5))
}

fn mixer_with(deck: usize, pcm: Arc<Pcm>, grid: Option<Grid>) -> Mixer {
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck, pcm, grid });
    m
}

const CLICK_LIMIT: f64 = 0.05;
const CLICK_DETAIL: &str = "größter Sprung im Signal (2. Differenz) bei einem 440-Hz-Sinus; Sinus allein ≈ 0,002, hörbarer Knackser ≳ 0,05";

/// Click checks: a sine on deck 1 and one operation at 1.0 s.
fn click_checks() -> Vec<Check> {
    // 123 BPM: loop and jump lengths are no whole number of sine periods, so jumps are real discontinuities
    let grid = Some(Grid { bpm: 123.0, first_beat_s: 0.0 });
    let cases: Vec<(&str, Vec<(f64, Command)>)> = vec![
        ("Start/Stop", vec![(0.2, Command::Play(0)), (1.0, Command::Pause(0)), (1.5, Command::Play(0))]),
        ("CUE während der Wiedergabe", vec![(0.2, Command::Play(0)), (1.0, Command::CuePress(0)), (1.1, Command::CueRelease(0))]),
        (
            "Hot-Cue-Sprung",
            vec![
                (0.2, Command::Play(0)),
                (0.537, Command::HotCuePress(0, 0)),
                (0.55, Command::HotCueRelease(0, 0)),
                (1.0, Command::HotCuePress(0, 0)),
                (1.02, Command::HotCueRelease(0, 0)),
            ],
        ),
        ("Loop 1 Beat (Loop-Grenzen)", vec![(0.2, Command::Play(0)), (1.0, Command::BeatLoop(0, 1.0))]),
        ("Beat Jump", vec![(0.2, Command::Play(0)), (1.0, Command::BeatJump(0, 4.0)), (1.5, Command::BeatJump(0, -2.0))]),
        ("Reverse an/aus", vec![(0.2, Command::Play(0)), (1.0, Command::Reverse(0, true)), (1.6, Command::Reverse(0, false))]),
        ("Kanalfader-Sprung", vec![(0.2, Command::Play(0)), (1.0, Command::Volume(0, 0.0)), (1.5, Command::Volume(0, 1.0))]),
        ("Crossfader-Sprung", vec![(0.2, Command::Play(0)), (1.0, Command::Crossfader(1.0)), (1.5, Command::Crossfader(-1.0))]),
        ("EQ-Kill", vec![(0.2, Command::Play(0)), (1.0, Command::Eq(0, 1, 0.0)), (1.5, Command::Eq(0, 1, 0.5))]),
        ("Tempo-Sprung", vec![(0.2, Command::Play(0)), (1.0, Command::TempoFader(0, 0.8)), (1.5, Command::TempoFader(0, -0.8))]),
        ("Beat FX an/aus", vec![(0.2, Command::Play(0)), (1.0, Command::BeatFxOn(true)), (2.0, Command::BeatFxOn(false))]),
        (
            "Color FX (Filter) Sprung",
            vec![(0.2, Command::Play(0)), (0.5, Command::ColorFxSelect(Some(crate::dsp::ColorFx::Filter))), (1.0, Command::ColorKnob(0, 0.1)), (1.5, Command::ColorKnob(0, 0.5))],
        ),
        ("Key Lock an (Tempo +8 %)", vec![(0.2, Command::Play(0)), (0.5, Command::TempoFader(0, 0.5)), (1.0, Command::MasterTempo(0, true))]),
    ];
    let mut out = Vec::new();
    for (name, events) in cases {
        let mut m = mixer_with(0, sine(440.0, 10.0), grid);
        let r = render(&mut m, events, 3.0);
        // the very first start (fade-in) is included from 0.2 s on
        let (v, t) = max_step(&r.master, 0.15);
        out.push(Check::at_most("Knackser", name, v, CLICK_LIMIT, "", &format!("{CLICK_DETAIL}; Maximum bei {t:.3} s")));
    }
    // key lock running: the granular shifter must not click on its own
    let mut m = mixer_with(0, sine(440.0, 10.0), grid);
    let r = render(&mut m, vec![(0.0, Command::Play(0)), (0.0, Command::TempoFader(0, 0.5)), (0.0, Command::MasterTempo(0, true))], 4.0);
    let (v, t) = max_step(&r.master, 1.0);
    out.push(Check::at_most("Knackser", "Key Lock im Dauerbetrieb", v, CLICK_LIMIT, "", &format!("{CLICK_DETAIL}; Maximum bei {t:.3} s")));
    out
}

fn pitch_checks() -> Vec<Check> {
    let mut out = Vec::new();
    // tempo fader 0.5 of ±16 % = +8 %
    for (name, events, target) in [
        ("Tempo +8 % ohne Key Lock", vec![(0.0, Command::Play(0)), (0.0, Command::TempoFader(0, 0.5))], 440.0 * 1.08),
        (
            "Tempo +8 % mit Key Lock",
            vec![(0.0, Command::Play(0)), (0.0, Command::TempoFader(0, 0.5)), (0.0, Command::MasterTempo(0, true))],
            440.0,
        ),
        ("Key Shift +2", vec![(0.0, Command::Play(0)), (0.0, Command::Key(0, 2))], 440.0 * 2f64.powf(2.0 / 12.0)),
    ] {
        let mut m = mixer_with(0, sine(440.0, 10.0), None);
        m.apply(Command::TempoRange(0, 2));
        let r = render(&mut m, events, 3.0);
        let f = frequency(&r.master, 1.0, 2.8);
        out.push(Check::near("Tonhöhe", name, f, target, target * 0.01, "Hz", "gemessene Frequenz eines 440-Hz-Sinus"));
    }
    out
}

fn timing_checks() -> Vec<Check> {
    let mut out = Vec::new();
    // sync: 124 BPM deck follows 128 BPM deck for 3 minutes
    let mut m = Mixer::new(SR);
    m.apply(Command::Load { deck: 0, pcm: Arc::new(Pcm::click_track(SR, 128.0, 200.0)), grid: Some(Grid { bpm: 128.0, first_beat_s: 0.0 }) });
    m.apply(Command::Load { deck: 1, pcm: Arc::new(Pcm::click_track(SR, 124.0, 200.0)), grid: Some(Grid { bpm: 124.0, first_beat_s: 0.0 }) });
    let r = render(&mut m, vec![(0.0, Command::Play(0)), (0.37, Command::Play(1)), (1.0, Command::Sync(1))], 180.0);
    let mut worst: f64 = 0.0;
    for (t, s) in r.states.iter().filter(|(t, _)| *t > 3.0) {
        let _ = t;
        if let (Some(a), Some(b), Some(bpm)) = (s.decks[0].beat, s.decks[1].beat, s.decks[0].bpm) {
            let mut d = (a - b).rem_euclid(1.0);
            if d > 0.5 {
                d -= 1.0;
            }
            worst = worst.max((d * 60.0 / bpm * 1000.0).abs());
        }
    }
    out.push(Check::at_most("Timing", "Sync-Drift über 3 Minuten", worst, 2.0, "ms", "größte Beat-Abweichung zwischen den Decks nach SYNC (124 → 128 BPM)"));
    let bpm2 = r.states.last().and_then(|(_, s)| s.decks[1].bpm).unwrap_or(0.0);
    out.push(Check::near("Timing", "Tempo nach SYNC", bpm2, 128.0, 0.01, "BPM", "Tempo von Deck 2"));
    // tempo accuracy in the audio: click intervals of deck 1 alone
    let mut m = mixer_with(0, Arc::new(Pcm::click_track(SR, 128.0, 40.0)), Some(Grid { bpm: 128.0, first_beat_s: 0.0 }));
    let r = render(&mut m, vec![(0.0, Command::Play(0))], 30.0);
    let on = onsets(&r.master, 0.2, 0.1);
    let interval = if on.len() > 2 { (on[on.len() - 1] - on[0]) / (on.len() - 1) as f64 * 1000.0 } else { 0.0 };
    out.push(Check::near("Timing", "Beat-Abstand bei 128 BPM", interval, 60_000.0 / 128.0, 0.1, "ms", "mittlerer Abstand der Klicks im Ausgang"));
    // echo: one click, Beat FX DELAY 1 beat at 120 BPM → repeat 500 ms later
    let mut frames = vec![[0.0f32; 2]; SR as usize * 3];
    for (i, f) in frames.iter_mut().enumerate().skip(SR as usize / 5).take(200) {
        let v = (1.0 - (i - SR as usize / 5) as f32 / 200.0) * 0.8;
        *f = [v, v];
    }
    let mut m = mixer_with(0, Arc::new(Pcm::new(SR, frames)), None);
    let r = render(
        &mut m,
        vec![
            (0.0, Command::BeatFxSelect(BeatFxKind::Delay)),
            (0.0, Command::BeatFxBpm(Some(120.0))),
            (0.0, Command::BeatFxAssign(FxAssign::Master)),
            (0.0, Command::BeatFxDepth(1.0)),
            (0.0, Command::BeatFxOn(true)),
            (0.0, Command::Play(0)),
        ],
        1.5,
    );
    let on = onsets(&r.master, 0.05, 0.1);
    let echo = if on.len() >= 2 { (on[1] - on[0]) * 1000.0 } else { 0.0 };
    out.push(Check::near("Timing", "Echo-Zeit (DELAY 1 Beat, 120 BPM)", echo, 500.0, 5.0, "ms", "Abstand Original → erste Wiederholung"));
    out
}

fn level_checks() -> Vec<Check> {
    let mut out = Vec::new();
    // everything turned up: soft clip keeps the output within ±1
    let mut m = Mixer::new(SR);
    for d in 0..2 {
        m.apply(Command::Load { deck: d, pcm: Arc::new(Pcm::sine(SR, 100.0 + 50.0 * d as f64, 5.0, 0.99)), grid: None });
        m.apply(Command::Trim(d, 4.0));
        for b in 0..3 {
            m.apply(Command::Eq(d, b, 1.0));
        }
        m.apply(Command::Play(d));
    }
    m.apply(Command::MasterGain(2.0));
    let r = render(&mut m, vec![], 2.0);
    out.push(Check::at_most("Pegel", "Kein Übersteuern bei Vollaussteuerung", peak(&r.master), 1.0, "", "Spitzenpegel mit Trim +12 dB, EQ +6 dB, Master ×2 auf beiden Decks"));
    // nothing playing → silence
    let mut m = mixer_with(0, sine(440.0, 5.0), None);
    let r = render(&mut m, vec![], 1.0);
    out.push(Check::at_most("Pegel", "Stille ohne Wiedergabe", peak(&r.master), 1e-6, "", "Spitzenpegel, wenn kein Deck spielt"));
    // headphone cue: fader down, cue on → only in the headphones
    let mut m = mixer_with(1, sine(1000.0, 5.0), None);
    let r = render(&mut m, vec![(0.0, Command::Play(1)), (0.0, Command::Volume(1, 0.0)), (0.0, Command::HeadphoneCue(1, true))], 2.0);
    out.push(Check { group: "Routing", name: "Kopfhörer-CUE bei geschlossenem Fader".into(), passed: rms(&r.cue, 1.0, 2.0) > 0.05 && rms(&r.master, 1.0, 2.0) < 1e-4, value: rms(&r.cue, 1.0, 2.0), limit: 0.05, unit: "RMS", detail: format!("Kopfhörer-RMS (soll > 0,05), Master-RMS {:.5} (soll ≈ 0)", rms(&r.master, 1.0, 2.0)) });
    // crossfader fully left cuts deck 2
    let mut m = mixer_with(1, sine(1000.0, 5.0), None);
    let r = render(&mut m, vec![(0.0, Command::Play(1)), (0.0, Command::Crossfader(-1.0))], 2.0);
    out.push(Check::at_most("Routing", "Crossfader links trennt Deck 2", rms(&r.master, 1.0, 2.0), 1e-4, "RMS", "Master-RMS von Deck 2 bei Crossfader ganz links"));
    // sampler pad
    let mut m = Mixer::new(SR);
    let pad = crate::sampler::builtin_sounds(SR).remove(0);
    let r = render(&mut m, vec![(0.0, Command::SamplerLoad(0, Arc::new(pad))), (0.1, Command::SamplerStart(0))], 1.0);
    out.push(Check { group: "Routing", name: "Sampler-Pad klingt".into(), passed: peak(&r.master) > 0.05, value: peak(&r.master), limit: 0.05, unit: "", detail: "Spitzenpegel nach Pad 1 (soll > 0,05)".into() });
    out
}

/// All checks (takes a few seconds: about four minutes of audio are rendered).
pub fn run_all() -> Vec<Check> {
    let mut v = click_checks();
    v.extend(pitch_checks());
    v.extend(timing_checks());
    v.extend(level_checks());
    v
}
