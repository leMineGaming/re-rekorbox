//! Two decks, channel faders, crossfader, master gain, beat sync and an offline renderer.
//!
//! All control goes through [`Command`] so the same messages can come from the CLI, a MIDI
//! controller or a UI thread; [`Mixer::render`] produces the stereo output.

use std::sync::Arc;

use crate::deck::{Deck, DeckState, Grid};
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
    /// Channel fader 0..1.
    Volume(usize, f32),
    /// Trim gain (linear, 1.0 = 0 dB).
    Trim(usize, f32),
    Quantize(usize, bool),
    /// Match tempo (and beat phase) of `deck` to the other deck.
    Sync(usize),
    /// Crossfader −1 (deck 1) .. 1 (deck 2).
    Crossfader(f32),
    MasterGain(f32),
}

/// Snapshot for display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MixerState {
    pub decks: [DeckState; DECKS],
    pub crossfader: f32,
    /// Peak level of the last rendered block (after master gain).
    pub peak: f32,
}

#[derive(Debug, Clone)]
pub struct Mixer {
    pub sample_rate: u32,
    pub decks: [Deck; DECKS],
    crossfader: f32,
    master: f32,
    peak: f32,
    scratch: Vec<Frame>,
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
            crossfader: 0.0,
            master: 1.0,
            peak: 0.0,
            scratch: Vec::new(),
        }
    }

    pub fn state(&self) -> MixerState {
        MixerState {
            decks: [self.decks[0].state(), self.decks[1].state()],
            crossfader: self.crossfader,
            peak: self.peak,
        }
    }

    /// Apply a command; commands for a deck index ≥ [`DECKS`] are ignored.
    pub fn apply(&mut self, cmd: Command) {
        use Command::*;
        let deck_of = |c: &Command| match c {
            Load { deck, .. } | SetGrid { deck, .. } => Some(*deck),
            Play(d) | Pause(d) | TogglePlay(d) | CuePress(d) | CueRelease(d) | LoopIn(d) | LoopOut(d)
            | ReloopExit(d) | Sync(d) => Some(*d),
            HotCuePress(d, _) | HotCueRelease(d, _) | HotCueDelete(d, _) | TempoRange(d, _) => Some(*d),
            BeatLoop(d, _) | LoopScale(d, _) | TempoFader(d, _) | SeekFraction(d, _) | SeekSeconds(d, _) => Some(*d),
            JogTouch(d, _) | Quantize(d, _) => Some(*d),
            Jog(d, _) => Some(*d),
            Volume(d, _) | Trim(d, _) => Some(*d),
            Crossfader(_) | MasterGain(_) => None,
        };
        if deck_of(&cmd).is_some_and(|d| d >= DECKS) {
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
            Volume(d, v) => self.decks[d].volume = v.clamp(0.0, 1.0),
            Trim(d, v) => self.decks[d].trim = v.clamp(0.0, 4.0),
            Quantize(d, q) => self.decks[d].quantize = q,
            Sync(d) => self.sync(d),
            Crossfader(x) => self.crossfader = x.clamp(-1.0, 1.0),
            MasterGain(g) => self.master = g.clamp(0.0, 2.0),
        }
    }

    /// Match the tempo of `deck` to the other deck and, if both are playing, align the beat phase
    /// (shift by less than half a beat). Needs grids on both decks; otherwise nothing happens.
    pub fn sync(&mut self, deck: usize) {
        let other = 1 - deck;
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
        let gains = [g1, g2];
        let mut buf = std::mem::take(&mut self.scratch);
        buf.resize(out.len(), [0.0; 2]);
        for (i, deck) in self.decks.iter_mut().enumerate() {
            buf.fill([0.0; 2]);
            let gain = deck.volume * deck.trim * gains[i];
            deck.render(&mut buf, self.sample_rate, gain);
            for (o, b) in out.iter_mut().zip(&buf) {
                o[0] += b[0];
                o[1] += b[1];
            }
        }
        self.scratch = buf;
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
