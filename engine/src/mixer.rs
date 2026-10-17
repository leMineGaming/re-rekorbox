//! Two decks, channel faders, crossfader, master gain, beat sync and an offline renderer.
//!
//! All control goes through [`Command`] so the same messages can come from the CLI, a MIDI
//! controller or a UI thread; [`Mixer::render`] produces the stereo output.

use std::sync::Arc;

use crate::deck::{Deck, DeckState, Grid};
use crate::dsp::{ColorFx, ColorProcessor, Eq3, Smooth};
use crate::fx::{BeatFx, BeatFxKind, FxAssign, PAD_FX};
use crate::pcm::{Frame, Pcm};

/// Number of decks.
pub const DECKS: usize = 2;

/// Control messages. `deck` is 0-based.
#[derive(Debug, Clone)]
pub enum Command {
    Load { deck: usize, pcm: Arc<Pcm>, grid: Option<Grid> },
    SetGrid { deck: usize, grid: Option<Grid> },
    Play(usize),
    Pause(usize),
    TogglePlay(usize),
    CuePress(usize),
    CueRelease(usize),
    HotCuePress(usize, usize),
    HotCueRelease(usize, usize),
    HotCueDelete(usize, usize),
    LoopIn(usize),
    LoopOut(usize),
    ReloopExit(usize),
    BeatLoop(usize, f64),
    LoopScale(usize, f64),
    /// Tempo fader −1..1.
    TempoFader(usize, f64),
    TempoRange(usize, usize),
    JogTouch(usize, bool),
    Jog(usize, i32),
    SeekFraction(usize, f64),
    SeekSeconds(usize, f64),
    /// Move by seconds relative to the current position.
    SeekBy(usize, f64),
    /// Jump by beats (negative = back).
    BeatJump(usize, f64),
    /// Channel fader 0..1.
    Volume(usize, f32),
    /// Trim gain (linear, 1.0 = 0 dB).
    Trim(usize, f32),
    Quantize(usize, bool),
    /// Match tempo (and beat phase) of `deck` to the other deck.
    Sync(usize),
    /// EQ knob 0..1 (0.5 = flat) for band 0 = low, 1 = mid, 2 = high.
    Eq(usize, usize, f32),
    /// Sound Color FX knob of a channel 0..1 (0.5 = off).
    ColorKnob(usize, f32),
    /// Selected Sound Color FX for all channels (`None` = off).
    ColorFxSelect(Option<ColorFx>),
    /// Shared Color FX sub-parameter 0..1.
    ColorSub(f32),
    Reverse(usize, bool),
    Slip(usize, bool),
    MasterTempo(usize, bool),
    /// Key shift in semitones.
    Key(usize, i32),
    /// Vinyl brake/start time in seconds.
    Brake(usize, f64),
    MemoryStore(usize),
    /// Call next (`true`) or previous memory cue.
    MemoryCall(usize, bool),
    MemoryDelete(usize),
    AutoCue(usize, bool),
    /// Search button held at a speed (e.g. ±4), 0 = released.
    Search(usize, f64),
    KeyboardPress(usize, i32),
    KeyboardRelease(usize),
    /// Copy the other deck (track, position, cues, transport) into this one.
    InstantDouble(usize),
    /// Sync master deck (`None` = automatic: the other deck).
    SetMaster(Option<usize>),
    BeatFxOn(bool),
    BeatFxSelect(BeatFxKind),
    /// Beat FX knob 0..1.
    BeatFxDepth(f32),
    /// Move the beat fraction up (+1) or down (−1).
    BeatFxBeat(i32),
    /// Add milliseconds to the effect time (ADJUST TIME).
    BeatFxAdjust(f32),
    BeatFxAssign(FxAssign),
    /// Tapped tempo, or `None` for automatic tempo from the decks.
    BeatFxBpm(Option<f64>),
    /// Pad FX of a deck: bank (0/1) and slot (0..15) while held, `None` on release.
    PadFx(usize, Option<(usize, usize)>),
    /// Crossfader −1 (deck 1) .. 1 (deck 2).
    Crossfader(f32),
    MasterGain(f32),
}

impl Command {
    /// Deck index a command addresses, `None` for global commands.
    pub fn deck(&self) -> Option<usize> {
        use Command::*;
        match self {
            Load { deck, .. } | SetGrid { deck, .. } => Some(*deck),
            Play(d) | Pause(d) | TogglePlay(d) | CuePress(d) | CueRelease(d) | LoopIn(d) | LoopOut(d)
            | ReloopExit(d) | Sync(d) => Some(*d),
            HotCuePress(d, _) | HotCueRelease(d, _) | HotCueDelete(d, _) | TempoRange(d, _) => Some(*d),
            BeatLoop(d, _) | LoopScale(d, _) | TempoFader(d, _) | SeekFraction(d, _) | SeekSeconds(d, _) => Some(*d),
            SeekBy(d, _) | BeatJump(d, _) => Some(*d),
            JogTouch(d, _) | Quantize(d, _) => Some(*d),
            Jog(d, _) => Some(*d),
            Volume(d, _) | Trim(d, _) | ColorKnob(d, _) | Eq(d, _, _) => Some(*d),
            PadFx(d, _) | Reverse(d, _) | Slip(d, _) | MasterTempo(d, _) | Key(d, _) | Brake(d, _) => Some(*d),
            MemoryStore(d) | MemoryCall(d, _) | MemoryDelete(d) | AutoCue(d, _) | Search(d, _) => Some(*d),
            KeyboardPress(d, _) | KeyboardRelease(d) | InstantDouble(d) => Some(*d),
            SetMaster(_) => None,
            ColorFxSelect(_) | ColorSub(_) | Crossfader(_) | MasterGain(_) => None,
            BeatFxOn(_) | BeatFxSelect(_) | BeatFxDepth(_) | BeatFxBeat(_) | BeatFxAdjust(_) | BeatFxAssign(_)
            | BeatFxBpm(_) => None,
        }
    }
}

/// Snapshot for display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MixerState {
    pub decks: [DeckState; DECKS],
    pub crossfader: f32,
    /// Peak level of the last rendered block (after master gain).
    pub peak: f32,
    /// Peak level per channel after the fader (for level meters).
    pub channel_peak: [f32; DECKS],
    pub color_fx: Option<ColorFx>,
    pub eq: [[f32; 3]; DECKS],
    pub beat_fx: BeatFxState,
    /// Pad FX held per deck (bank, slot).
    pub pad_fx: [Option<(usize, usize)>; DECKS],
    /// Sync master deck chosen with MASTER (`None` = automatic).
    pub master: Option<usize>,
}

/// Beat FX section for display / LEDs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeatFxState {
    pub on: bool,
    pub kind: BeatFxKind,
    pub beats: f64,
    pub assign: FxAssign,
    /// Tempo the effect runs at.
    pub bpm: f64,
    /// True when the tempo comes from the decks (not tapped).
    pub auto_bpm: bool,
}

impl Default for BeatFxState {
    fn default() -> Self {
        BeatFxState { on: false, kind: BeatFxKind::Delay, beats: 1.0, assign: FxAssign::Master, bpm: 120.0, auto_bpm: true }
    }
}

#[derive(Debug, Clone)]
pub struct Mixer {
    pub sample_rate: u32,
    pub decks: [Deck; DECKS],
    channels: [Channel; DECKS],
    crossfader: f32,
    master: f32,
    peak: f32,
    color_fx: Option<ColorFx>,
    color_sub: f32,
    beat_fx: BeatFx,
    fx_assign: FxAssign,
    fx_bpm: Option<f64>,
    pad_fx: [Option<(usize, usize)>; DECKS],
    sync_master: Option<usize>,
    scratch: Vec<Frame>,
}

/// Channel strip after the deck: trim → EQ → Sound Color FX → channel fader × crossfader.
#[derive(Debug, Clone)]
struct Channel {
    eq: Eq3,
    color: ColorProcessor,
    color_knob: f32,
    gain: Smooth,
    peak: f32,
    pad_fx: BeatFx,
}

impl Channel {
    fn new(sample_rate: u32) -> Channel {
        Channel {
            eq: Eq3::new(sample_rate),
            color: ColorProcessor::new(sample_rate),
            color_knob: 0.5,
            gain: Smooth::new(1.0, 0.005, sample_rate),
            peak: 0.0,
            pad_fx: BeatFx::new(sample_rate, BeatFxKind::Roll),
        }
    }
}

/// Constant-power crossfader gains for (deck 1, deck 2) at position −1..1.
pub fn crossfader_gains(x: f32) -> (f32, f32) {
    let a = (x.clamp(-1.0, 1.0) + 1.0) * 0.25 * std::f32::consts::PI;
    (a.cos(), a.sin())
}

impl Mixer {
    pub fn new(sample_rate: u32) -> Mixer {
        Mixer {
            sample_rate,
            decks: [Deck::new(), Deck::new()],
            channels: [Channel::new(sample_rate), Channel::new(sample_rate)],
            crossfader: 0.0,
            master: 1.0,
            peak: 0.0,
            color_fx: None,
            color_sub: 0.5,
            beat_fx: BeatFx::new(sample_rate, BeatFxKind::Delay),
            fx_assign: FxAssign::Master,
            fx_bpm: None,
            pad_fx: [None; DECKS],
            sync_master: None,
            scratch: Vec::new(),
        }
    }

    pub fn state(&self) -> MixerState {
        MixerState {
            decks: [self.decks[0].state(), self.decks[1].state()],
            crossfader: self.crossfader,
            peak: self.peak,
            channel_peak: [self.channels[0].peak, self.channels[1].peak],
            color_fx: self.color_fx,
            eq: [self.channels[0].eq.knobs, self.channels[1].eq.knobs],
            beat_fx: BeatFxState {
                on: self.beat_fx.on,
                kind: self.beat_fx.kind,
                beats: self.beat_fx.beats(),
                assign: self.fx_assign,
                bpm: self.fx_tempo(),
                auto_bpm: self.fx_bpm.is_none(),
            },
            pad_fx: self.pad_fx,
            master: self.sync_master,
        }
    }

    /// Apply a command; commands for a deck index ≥ [`DECKS`] are ignored.
    pub fn apply(&mut self, cmd: Command) {
        use Command::*;
        if cmd.deck().is_some_and(|d| d >= DECKS) {
            return;
        }
        match cmd {
            Load { deck, pcm, grid } => self.decks[deck].load(pcm, grid),
            SetGrid { deck, grid } => self.decks[deck].grid = grid,
            Play(d) => self.decks[d].play(),
            Pause(d) => self.decks[d].pause(),
            TogglePlay(d) => self.decks[d].toggle_play(),
            CuePress(d) => self.decks[d].cue_press(),
            CueRelease(d) => self.decks[d].cue_release(),
            HotCuePress(d, n) => self.decks[d].hot_cue_press(n),
            HotCueRelease(d, n) => self.decks[d].hot_cue_release(n),
            HotCueDelete(d, n) => self.decks[d].hot_cue_delete(n),
            LoopIn(d) => self.decks[d].loop_in(),
            LoopOut(d) => self.decks[d].loop_out(),
            ReloopExit(d) => self.decks[d].reloop_exit(),
            BeatLoop(d, b) => self.decks[d].beat_loop(b),
            LoopScale(d, f) => self.decks[d].loop_scale(f),
            TempoFader(d, v) => self.decks[d].set_tempo_fader(v),
            TempoRange(d, r) => self.decks[d].set_tempo_range(r),
            JogTouch(d, t) => self.decks[d].jog_touch(t),
            Jog(d, delta) => self.decks[d].jog(delta),
            SeekFraction(d, f) => self.decks[d].seek_fraction(f),
            SeekSeconds(d, s) => self.decks[d].seek_s(s),
            SeekBy(d, s) => self.decks[d].seek_by(s),
            BeatJump(d, b) => self.decks[d].beat_jump(b),
            Volume(d, v) => self.decks[d].volume = v.clamp(0.0, 1.0),
            Trim(d, v) => self.decks[d].trim = v.clamp(0.0, 4.0),
            Quantize(d, q) => self.decks[d].quantize = q,
            Sync(d) => self.sync(d),
            Eq(d, band, v) => {
                if band < 3 {
                    self.channels[d].eq.knobs[band] = v.clamp(0.0, 1.0);
                }
            }
            ColorKnob(d, v) => self.channels[d].color_knob = v.clamp(0.0, 1.0),
            ColorFxSelect(fx) => {
                if fx != self.color_fx {
                    for c in &mut self.channels {
                        c.color.reset();
                    }
                }
                self.color_fx = fx;
            }
            ColorSub(v) => self.color_sub = v.clamp(0.0, 1.0),
            Reverse(d, on) => self.decks[d].set_reverse(on),
            Slip(d, on) => self.decks[d].set_slip(on),
            MasterTempo(d, on) => self.decks[d].set_master_tempo(on),
            Key(d, k) => self.decks[d].set_key(k),
            Brake(d, s) => self.decks[d].set_brake(s),
            MemoryStore(d) => self.decks[d].memory_store(),
            MemoryCall(d, fwd) => self.decks[d].memory_call(fwd),
            MemoryDelete(d) => self.decks[d].memory_delete(),
            AutoCue(d, on) => self.decks[d].set_auto_cue(on),
            Search(d, v) => self.decks[d].set_search(v),
            KeyboardPress(d, k) => self.decks[d].keyboard_press(k),
            KeyboardRelease(d) => self.decks[d].keyboard_release(),
            InstantDouble(d) => {
                let other = self.decks[1 - d].clone();
                self.decks[d].copy_from(&other);
            }
            SetMaster(m) => self.sync_master = m.filter(|&m| m < DECKS),
            BeatFxOn(on) => self.beat_fx.on = on,
            BeatFxSelect(k) => {
                if k != self.beat_fx.kind {
                    self.beat_fx.reset();
                    self.beat_fx.kind = k;
                }
            }
            BeatFxDepth(v) => self.beat_fx.depth = v.clamp(0.0, 1.0),
            BeatFxBeat(step) => {
                let i = self.beat_fx.beat_index as i32 + step;
                self.beat_fx.beat_index = i.clamp(0, crate::fx::BEATS.len() as i32 - 1) as usize;
            }
            BeatFxAdjust(ms) => self.beat_fx.adjust_ms = (self.beat_fx.adjust_ms + ms).clamp(-500.0, 2000.0),
            BeatFxAssign(a) => {
                if a != self.fx_assign {
                    self.beat_fx.reset();
                }
                self.fx_assign = a;
            }
            BeatFxBpm(b) => self.fx_bpm = b.filter(|b| *b > 20.0 && *b < 400.0),
            PadFx(d, held) => {
                let fx = &mut self.channels[d].pad_fx;
                match held.and_then(|(bank, slot)| PAD_FX.get(bank).and_then(|b| b.get(slot))) {
                    Some(preset) => {
                        if fx.kind != preset.kind {
                            fx.reset();
                        }
                        fx.kind = preset.kind;
                        fx.beat_index = preset.beat_index;
                        fx.depth = preset.depth;
                        fx.on = true;
                        self.pad_fx[d] = held;
                    }
                    None => {
                        fx.on = false;
                        self.pad_fx[d] = None;
                    }
                }
            }
            Crossfader(x) => self.crossfader = x.clamp(-1.0, 1.0),
            MasterGain(g) => self.master = g.clamp(0.0, 2.0),
        }
    }

    /// Tempo for the Beat FX: tapped, else the assigned deck, else the loudest playing deck, else 120.
    pub fn fx_tempo(&self) -> f64 {
        if let Some(b) = self.fx_bpm {
            return b;
        }
        if let FxAssign::Channel(c) = self.fx_assign {
            if let Some(b) = self.decks.get(c).and_then(|d| d.bpm()) {
                return b;
            }
        }
        self.master_deck().and_then(|d| self.decks[d].bpm()).unwrap_or(120.0)
    }

    /// The playing deck that is loudest in the mix (fader × crossfader), if any.
    pub fn master_deck(&self) -> Option<usize> {
        let (g1, g2) = crossfader_gains(self.crossfader);
        let xf = [g1, g2];
        (0..DECKS)
            .filter(|&d| self.decks[d].is_playing())
            .max_by(|&a, &b| (self.decks[a].volume * xf[a]).total_cmp(&(self.decks[b].volume * xf[b])))
    }

    /// Match the tempo of `deck` to the other deck and, if both are playing, align the beat phase
    /// (shift by less than half a beat). Needs grids on both decks; otherwise nothing happens.
    pub fn sync(&mut self, deck: usize) {
        let other = match self.sync_master {
            Some(m) if m != deck => m,
            _ => 1 - deck,
        };
        let (Some(g_self), Some(g_master), Some(master_bpm)) =
            (self.decks[deck].grid, self.decks[other].grid, self.decks[other].bpm())
        else {
            return;
        };
        self.decks[deck].set_tempo(master_bpm / g_self.bpm);
        if self.decks[deck].is_playing() && self.decks[other].is_playing() {
            let master_phase = g_master.beat_at(self.decks[other].position_s()).rem_euclid(1.0);
            let t = self.decks[deck].position_s();
            let own_phase = g_self.beat_at(t).rem_euclid(1.0);
            let mut diff = master_phase - own_phase;
            if diff > 0.5 {
                diff -= 1.0;
            } else if diff < -0.5 {
                diff += 1.0;
            }
            self.decks[deck].seek_s(t + diff * g_self.beat_s());
        }
    }

    /// Render the next `out.len()` frames.
    pub fn render(&mut self, out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (g1, g2) = crossfader_gains(self.crossfader);
        let xf = [g1, g2];
        let mut buf = std::mem::take(&mut self.scratch);
        buf.resize(out.len(), [0.0; 2]);
        let fx_bpm = self.fx_tempo();
        let deck_bpm: [f64; DECKS] = std::array::from_fn(|d| self.decks[d].bpm().unwrap_or(120.0));
        for (i, (deck, ch)) in self.decks.iter_mut().zip(self.channels.iter_mut()).enumerate() {
            buf.fill([0.0; 2]);
            deck.render(&mut buf, self.sample_rate, 1.0);
            // DUB ECHO follows the deck tempo (3/4 beat), 120 BPM without a grid
            ch.color.echo_s = (0.75 * 60.0 / deck.bpm().unwrap_or(120.0)) as f32;
            let target = deck.volume * xf[i];
            let trim = deck.trim;
            let mut peak = 0.0f32;
            for (o, b) in out.iter_mut().zip(&buf) {
                let mut x = [b[0] * trim, b[1] * trim];
                if !ch.eq.is_flat() {
                    x = ch.eq.process(x);
                }
                x = ch.color.process(self.color_fx, ch.color_knob, self.color_sub, x);
                if ch.pad_fx.active() {
                    x = ch.pad_fx.process(x, deck_bpm[i]);
                }
                if self.fx_assign == FxAssign::Channel(i) {
                    x = self.beat_fx.process(x, fx_bpm);
                }
                let g = ch.gain.next(target);
                let y = [x[0] * g, x[1] * g];
                peak = peak.max(y[0].abs()).max(y[1].abs());
                o[0] += y[0];
                o[1] += y[1];
            }
            ch.peak = peak;
        }
        self.scratch = buf;
        if self.fx_assign == FxAssign::Master {
            for o in out.iter_mut() {
                *o = self.beat_fx.process(*o, fx_bpm);
            }
        }
        let mut peak = 0.0f32;
        for o in out.iter_mut() {
            for s in o.iter_mut() {
                *s = soft_clip(*s * self.master);
                peak = peak.max(s.abs());
            }
        }
        self.peak = peak;
    }

    /// Render `seconds` of output into a new buffer (offline, e.g. for tests or `--render`).
    pub fn render_offline(&mut self, seconds: f64) -> Pcm {
        let n = (seconds * f64::from(self.sample_rate)) as usize;
        let mut out = vec![[0.0; 2]; n];
        for chunk in out.chunks_mut(512) {
            self.render(chunk);
        }
        Pcm::new(self.sample_rate, out)
    }
}

/// Transparent below ±0.9, smooth saturation above (never exceeds ±1).
pub fn soft_clip(x: f32) -> f32 {
    const K: f32 = 0.9;
    if x.abs() <= K {
        x
    } else {
        let s = x.signum();
        let over = (x.abs() - K) / (1.0 - K);
        s * (K + (1.0 - K) * over.tanh())
    }
}
