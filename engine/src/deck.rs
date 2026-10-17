//! One deck: playback position, transport, cue / hot cues, loops, tempo, jog (scratch and pitch bend).
//!
//! Positions are in source frames (`f64`). [`Deck::render`] mixes the deck into an output buffer at the
//! output sample rate; everything else only changes state, so the deck can be driven from a MIDI
//! thread through [`crate::mixer::Command`]s and rendered from the audio thread.
//!
//! The behaviour is an own design for the MVP (not taken from rekordbox code); where it imitates
//! usual CDJ behaviour this is noted at the method.

use std::sync::Arc;

use crate::dsp::PitchShifter;
use crate::pcm::{Frame, Pcm};

/// Number of hot cue slots.
pub const HOT_CUES: usize = 8;
/// Tempo ranges selectable on the deck (±6 %, ±10 %, ±16 %, wide).
pub const TEMPO_RANGES: [f64; 4] = [0.06, 0.10, 0.16, 1.0];
/// Length of the fade applied when playback starts or stops (avoids clicks).
const DECLICK_S: f64 = 0.004;
/// One full jog revolution corresponds to this much audio while scratching (33⅓ rpm).
const SECONDS_PER_REV: f64 = 1.8;

/// Beat grid: tempo and the position of a downbeat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub bpm: f64,
    /// Position of a first beat in seconds.
    pub first_beat_s: f64,
}

impl Grid {
    /// Length of one beat in seconds.
    pub fn beat_s(&self) -> f64 {
        60.0 / self.bpm
    }

    /// Beat number (fractional) at a time in seconds.
    pub fn beat_at(&self, t: f64) -> f64 {
        (t - self.first_beat_s) / self.beat_s()
    }

    /// Time of the beat closest to `t`.
    pub fn nearest_beat(&self, t: f64) -> f64 {
        self.first_beat_s + self.beat_at(t).round() * self.beat_s()
    }
}

/// Active or prepared loop (frames).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loop {
    pub start: f64,
    pub end: f64,
    pub active: bool,
}

/// Snapshot of a deck for display.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DeckState {
    pub loaded: bool,
    pub playing: bool,
    pub position_s: f64,
    pub duration_s: f64,
    /// Current speed factor (1.0 = normal, includes tempo, bend and scratch).
    pub speed: f64,
    /// Tempo fader setting as factor (1.0 = ±0 %).
    pub tempo: f64,
    /// Effective BPM if a grid is known.
    pub bpm: Option<f64>,
    pub cue_s: f64,
    pub looping: bool,
    /// Paused exactly on the cue point.
    pub at_cue: bool,
    /// Which hot cue slots are set.
    pub hot_cues: [bool; HOT_CUES],
    pub reverse: bool,
    pub slip: bool,
    /// Slip is on and playback currently diverges from the background position.
    pub slip_diverged: bool,
    pub master_tempo: bool,
    /// Key shift in semitones.
    pub key: i32,
    pub memory_cues: usize,
    pub auto_cue: bool,
    /// Vinyl brake/start time in seconds (0 = instant).
    pub brake_s: f64,
}

#[derive(Debug, Clone)]
pub struct Deck {
    pcm: Option<Arc<Pcm>>,
    pos: f64,
    playing: bool,
    /// Gain of the declick ramp (0..1) and its target.
    ramp: f32,
    /// Tempo fader −1..1 and range index.
    tempo_fader: f64,
    tempo_range: usize,
    /// Temporary speed offset from the jog wheel (pitch bend), decays.
    bend: f64,
    /// Scratch: jog touched, target position the platter points at.
    touched: bool,
    scratch_target: f64,
    scratch_speed: f64,
    /// Ticks per jog revolution (controller dependent).
    pub jog_ticks_per_rev: f64,
    cue: f64,
    /// Cue button held while previewing from the cue point.
    cue_preview: bool,
    hot_cues: [Option<f64>; HOT_CUES],
    /// Hot cue held while it started playback from pause (returns on release).
    hot_preview: Option<usize>,
    lp: Option<Loop>,
    pub grid: Option<Grid>,
    /// Channel fader 0..1 and trim gain (linear).
    pub volume: f32,
    pub trim: f32,
    /// Quantize hot cues / loops to the beat grid.
    pub quantize: bool,
    reverse: bool,
    slip: bool,
    /// Background position while slip is on.
    shadow: f64,
    was_diverged: bool,
    /// Hot cue held in slip mode while playing (returns to the background position on release).
    hot_slip: Option<usize>,
    master_tempo: bool,
    key: i32,
    shifter: PitchShifter,
    /// Vinyl brake/start time (s) and motor speed factor 0..1.
    brake_s: f64,
    motor: f64,
    stopping: bool,
    /// Speed while a search button is held (0 = not searching).
    search: f64,
    memory: Vec<f64>,
    auto_cue: bool,
    keyboard_preview: bool,
}

impl Default for Deck {
    fn default() -> Self {
        Deck {
            pcm: None,
            pos: 0.0,
            playing: false,
            ramp: 0.0,
            tempo_fader: 0.0,
            tempo_range: 2,
            bend: 0.0,
            touched: false,
            scratch_target: 0.0,
            scratch_speed: 0.0,
            jog_ticks_per_rev: 2048.0,
            cue: 0.0,
            cue_preview: false,
            hot_cues: [None; HOT_CUES],
            hot_preview: None,
            lp: None,
            grid: None,
            volume: 1.0,
            trim: 1.0,
            quantize: false,
            reverse: false,
            slip: false,
            shadow: 0.0,
            was_diverged: false,
            hot_slip: None,
            master_tempo: false,
            key: 0,
            shifter: PitchShifter::new(48000),
            brake_s: 0.0,
            motor: 0.0,
            stopping: false,
            search: 0.0,
            memory: Vec::new(),
            auto_cue: false,
            keyboard_preview: false,
        }
    }
}

impl Deck {
    pub fn new() -> Deck {
        Deck::default()
    }

    fn sr(&self) -> f64 {
        self.pcm.as_ref().map_or(44100.0, |p| f64::from(p.sample_rate))
    }

    fn len(&self) -> f64 {
        self.pcm.as_ref().map_or(0.0, |p| p.len() as f64)
    }

    fn to_frames(&self, s: f64) -> f64 {
        s * self.sr()
    }

    fn to_secs(&self, f: f64) -> f64 {
        f / self.sr()
    }

    /// Load audio; stops playback and clears cues and loop, keeps tempo settings.
    pub fn load(&mut self, pcm: Arc<Pcm>, grid: Option<Grid>) {
        self.pcm = Some(pcm);
        self.grid = grid;
        self.pos = 0.0;
        self.playing = false;
        self.ramp = 0.0;
        self.cue = 0.0;
        self.hot_cues = [None; HOT_CUES];
        self.lp = None;
        self.cue_preview = false;
        self.hot_preview = None;
        self.hot_slip = None;
        self.memory.clear();
        self.stopping = false;
        self.motor = 0.0;
        self.reverse = false;
        self.shifter = PitchShifter::new(self.pcm.as_ref().map_or(48000, |p| p.sample_rate));
        if self.auto_cue {
            self.cue = self.first_sound();
            self.pos = self.cue;
        }
        self.shadow = self.pos;
        self.scratch_target = self.pos;
    }

    /// First frame louder than −60 dBFS (auto cue), or 0.
    fn first_sound(&self) -> f64 {
        let Some(p) = &self.pcm else { return 0.0 };
        p.frames.iter().position(|f| f[0].abs().max(f[1].abs()) > 0.001).unwrap_or(0) as f64
    }

    /// Copy track, position, cues and transport from another deck (instant double).
    pub fn copy_from(&mut self, other: &Deck) {
        let Some(pcm) = other.pcm.clone() else { return };
        self.load(pcm, other.grid);
        self.pos = other.pos;
        self.shadow = other.pos;
        self.scratch_target = other.pos;
        self.cue = other.cue;
        self.hot_cues = other.hot_cues;
        self.memory = other.memory.clone();
        self.lp = other.lp;
        if other.playing && !other.stopping {
            self.playing = true;
            self.motor = 1.0;
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.pcm.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn position_s(&self) -> f64 {
        self.to_secs(self.pos)
    }

    /// Jump to a time; clamped to the track.
    pub fn seek_s(&mut self, t: f64) {
        self.pos = self.to_frames(t).clamp(0.0, self.len());
        self.scratch_target = self.pos;
    }

    /// Needle search: jump to a fraction 0..1 of the track.
    pub fn seek_fraction(&mut self, f: f64) {
        self.pos = (f.clamp(0.0, 1.0) * self.len()).min(self.len());
        self.scratch_target = self.pos;
    }

    pub fn play(&mut self) {
        if self.is_loaded() {
            if !self.playing || self.brake_s <= 0.0 {
                self.motor = if self.brake_s > 0.0 { self.motor.min(1.0) } else { 1.0 };
            }
            self.playing = true;
            self.stopping = false;
            self.cue_preview = false;
            self.hot_preview = None;
            self.keyboard_preview = false;
        }
    }

    /// Pause; with a brake time the deck slows down first (vinyl brake).
    pub fn pause(&mut self) {
        if self.playing && self.brake_s > 0.0 {
            self.stopping = true;
        } else {
            self.stop_now();
        }
    }

    fn stop_now(&mut self) {
        self.playing = false;
        self.stopping = false;
        self.motor = 0.0;
    }

    pub fn toggle_play(&mut self) {
        if self.playing && !self.stopping {
            self.pause()
        } else {
            self.play()
        }
    }

    /// Vinyl brake / start time in seconds (0 = instant).
    pub fn set_brake(&mut self, seconds: f64) {
        self.brake_s = seconds.clamp(0.0, 5.0);
    }

    pub fn set_reverse(&mut self, on: bool) {
        self.reverse = on;
    }

    pub fn set_slip(&mut self, on: bool) {
        self.slip = on;
        self.shadow = self.pos;
    }

    /// Master tempo (key lock): tempo changes keep the pitch.
    pub fn set_master_tempo(&mut self, on: bool) {
        self.master_tempo = on;
    }

    /// Key shift in semitones (−12..12).
    pub fn set_key(&mut self, semitones: i32) {
        self.key = semitones.clamp(-12, 12);
    }

    pub fn key(&self) -> i32 {
        self.key
    }

    pub fn set_auto_cue(&mut self, on: bool) {
        self.auto_cue = on;
    }

    /// Search button held: fast forward/backward at `speed` (0 = released).
    pub fn set_search(&mut self, speed: f64) {
        self.search = speed;
        self.scratch_target = self.pos;
    }

    // --- memory cues -----------------------------------------------------------------------

    /// Store a memory cue: the cue point while paused on it, otherwise the current position.
    pub fn memory_store(&mut self) {
        if !self.is_loaded() {
            return;
        }
        let at = if !self.playing && (self.pos - self.cue).abs() < 1.0 { self.cue } else { self.quantized(self.pos) };
        if !self.memory.iter().any(|&m| (m - at).abs() < 1.0) {
            self.memory.push(at);
            self.memory.sort_by(f64::total_cmp);
        }
    }

    /// Call the next (`forward`) or previous memory cue: it becomes the cue point and the deck
    /// jumps there.
    pub fn memory_call(&mut self, forward: bool) {
        let next = if forward {
            self.memory.iter().copied().find(|&m| m > self.pos + 1.0)
        } else {
            self.memory.iter().rev().copied().find(|&m| m < self.pos - 1.0)
        };
        if let Some(m) = next {
            self.cue = m;
            self.pos = m;
            self.shadow = m;
            self.scratch_target = m;
        }
    }

    /// Delete the memory cue at the current position (the one just called).
    pub fn memory_delete(&mut self) {
        let pos = self.pos;
        self.memory.retain(|&m| (m - pos).abs() >= 1.0);
    }

    pub fn memory_cues_s(&self) -> Vec<f64> {
        self.memory.iter().map(|&m| self.to_secs(m)).collect()
    }

    // --- keyboard mode ---------------------------------------------------------------------

    /// Keyboard pad: set the key to `semitones` and play from the first hot cue (else the cue
    /// point); when paused it plays while the pad is held.
    pub fn keyboard_press(&mut self, semitones: i32) {
        if !self.is_loaded() {
            return;
        }
        self.set_key(semitones);
        let start = self.hot_cues.iter().flatten().next().copied().unwrap_or(self.cue);
        self.pos = start;
        self.shadow = start;
        self.scratch_target = start;
        if !self.playing {
            self.playing = true;
            self.motor = 1.0;
            self.keyboard_preview = true;
        }
    }

    pub fn keyboard_release(&mut self) {
        if self.keyboard_preview {
            self.keyboard_preview = false;
            self.stop_now();
            self.pos = self.hot_cues.iter().flatten().next().copied().unwrap_or(self.cue);
        }
    }

    // --- tempo -----------------------------------------------------------------------------

    /// Tempo fader position −1..1 (MIDI value 0..16383 maps to −1..1 in the controller layer).
    pub fn set_tempo_fader(&mut self, v: f64) {
        self.tempo_fader = v.clamp(-1.0, 1.0);
    }

    pub fn set_tempo_range(&mut self, index: usize) {
        self.tempo_range = index.min(TEMPO_RANGES.len() - 1);
    }

    pub fn tempo_range(&self) -> f64 {
        TEMPO_RANGES[self.tempo_range]
    }

    /// Tempo factor from the fader (1.0 = ±0 %).
    pub fn tempo(&self) -> f64 {
        1.0 + self.tempo_fader * self.tempo_range()
    }

    /// Set the tempo factor directly (used by sync); widens the range if needed.
    pub fn set_tempo(&mut self, factor: f64) {
        let mut off = factor - 1.0;
        while off.abs() > self.tempo_range() && self.tempo_range < TEMPO_RANGES.len() - 1 {
            self.tempo_range += 1;
        }
        off = off.clamp(-self.tempo_range(), self.tempo_range());
        self.tempo_fader = off / self.tempo_range();
    }

    /// Effective BPM (grid BPM × tempo), if a grid is known.
    pub fn bpm(&self) -> Option<f64> {
        self.grid.map(|g| g.bpm * self.tempo())
    }

    /// Current playback speed without the scratch override.
    pub fn speed(&self) -> f64 {
        self.tempo() + self.bend
    }

    // --- cue -------------------------------------------------------------------------------

    /// CUE pressed. Playing → jump back to the cue point and stop. Paused → if away from the cue
    /// point, set a new cue here (quantized if enabled); then preview from the cue while held.
    pub fn cue_press(&mut self) {
        if !self.is_loaded() {
            return;
        }
        if self.playing && !self.cue_preview {
            self.stop_now();
            self.pos = self.cue;
            return;
        }
        if (self.pos - self.cue).abs() > 1.0 {
            self.cue = self.quantized(self.pos);
        }
        self.pos = self.cue;
        self.playing = true;
        self.motor = 1.0;
        self.cue_preview = true;
    }

    /// CUE released: end of the preview, back to the cue point.
    pub fn cue_release(&mut self) {
        if self.cue_preview {
            self.cue_preview = false;
            self.stop_now();
            self.pos = self.cue;
        }
    }

    pub fn cue_s(&self) -> f64 {
        self.to_secs(self.cue)
    }

    fn quantized(&self, frames: f64) -> f64 {
        match (self.quantize, self.grid) {
            (true, Some(g)) => self.to_frames(g.nearest_beat(self.to_secs(frames))).clamp(0.0, self.len()),
            _ => frames,
        }
    }

    // --- hot cues --------------------------------------------------------------------------

    pub fn hot_cue(&self, n: usize) -> Option<f64> {
        self.hot_cues.get(n).copied().flatten().map(|f| self.to_secs(f))
    }

    /// Hot cue pad pressed: empty slot → store the current position; set slot → jump there. When
    /// paused the deck plays while the pad is held (released → back to the hot cue, paused).
    pub fn hot_cue_press(&mut self, n: usize) {
        if n >= HOT_CUES || !self.is_loaded() {
            return;
        }
        match self.hot_cues[n] {
            None => self.hot_cues[n] = Some(self.quantized(self.pos)),
            Some(p) => {
                self.pos = p;
                self.scratch_target = p;
                if !self.playing {
                    self.playing = true;
                    self.motor = 1.0;
                    self.hot_preview = Some(n);
                } else if self.slip {
                    self.hot_slip = Some(n);
                }
            }
        }
    }

    pub fn hot_cue_release(&mut self, n: usize) {
        if self.hot_preview == Some(n) {
            self.hot_preview = None;
            self.stop_now();
            if let Some(p) = self.hot_cues[n] {
                self.pos = p;
            }
        }
        if self.hot_slip == Some(n) {
            self.hot_slip = None;
        }
    }

    pub fn hot_cue_delete(&mut self, n: usize) {
        if n < HOT_CUES {
            self.hot_cues[n] = None;
        }
    }

    // --- loops -----------------------------------------------------------------------------

    pub fn loop_state(&self) -> Option<Loop> {
        self.lp
    }

    pub fn loop_in(&mut self) {
        let start = self.quantized(self.pos);
        self.lp = Some(Loop { start, end: start, active: false });
    }

    /// Set the loop end here and start looping (needs a loop-in before the current position).
    pub fn loop_out(&mut self) {
        let end = self.quantized(self.pos);
        if let Some(l) = &mut self.lp {
            if end > l.start + 1.0 {
                l.end = end;
                l.active = true;
            }
        }
    }

    /// Exit an active loop, or re-enter the last loop (jumping to its start if outside).
    pub fn reloop_exit(&mut self) {
        if let Some(l) = &mut self.lp {
            if l.end <= l.start {
                return;
            }
            l.active = !l.active;
            if l.active && (self.pos < l.start || self.pos >= l.end) {
                self.pos = l.start;
            }
        }
    }

    /// Loop of `beats` beats starting at the current position (nearest beat with quantize);
    /// without a grid 120 BPM is assumed.
    pub fn beat_loop(&mut self, beats: f64) {
        if !self.is_loaded() || beats <= 0.0 {
            return;
        }
        let beat_s = self.grid.map_or(0.5, |g| g.beat_s());
        let start = self.quantized(self.pos);
        let end = (start + self.to_frames(beat_s * beats)).min(self.len());
        self.lp = Some(Loop { start, end, active: true });
    }

    /// Halve or double the active loop length.
    pub fn loop_scale(&mut self, factor: f64) {
        if let Some(l) = &mut self.lp {
            let len = (l.end - l.start) * factor;
            if len >= 16.0 {
                l.end = l.start + len;
            }
        }
    }

    /// Jump by `beats` (negative = back) using the grid, or 120 BPM without one. An active loop
    /// moves along with the position.
    pub fn beat_jump(&mut self, beats: f64) {
        if !self.is_loaded() {
            return;
        }
        let beat_s = self.grid.map_or(0.5, |g| g.beat_s());
        let delta = self.to_frames(beats * beat_s);
        let new = (self.pos + delta).clamp(0.0, self.len());
        let moved = new - self.pos;
        self.pos = new;
        self.scratch_target = new;
        if let Some(l) = &mut self.lp {
            if l.active {
                l.start += moved;
                l.end += moved;
            }
        }
    }

    /// Move the position by `seconds` (search buttons / search wheel).
    pub fn seek_by(&mut self, seconds: f64) {
        let t = self.position_s() + seconds;
        self.seek_s(t);
    }

    // --- jog -------------------------------------------------------------------------------

    /// Jog top touched / released (vinyl mode: touching holds the platter for scratching).
    pub fn jog_touch(&mut self, touched: bool) {
        self.touched = touched;
        self.scratch_target = self.pos;
        self.scratch_speed = 0.0;
    }

    /// Jog turned by `delta` ticks. Touched → scratch (move the audio with the platter);
    /// otherwise → pitch bend while playing, fine seek while paused.
    pub fn jog(&mut self, delta: i32) {
        if !self.is_loaded() {
            return;
        }
        let frames = f64::from(delta) / self.jog_ticks_per_rev * SECONDS_PER_REV * self.sr();
        if self.touched {
            self.scratch_target = (self.scratch_target + frames).clamp(0.0, self.len());
        } else if self.playing {
            self.bend = (self.bend + f64::from(delta) * 0.002).clamp(-0.5, 0.5);
        } else {
            self.pos = (self.pos + frames * 0.25).clamp(0.0, self.len());
        }
    }

    // --- render ----------------------------------------------------------------------------

    pub fn state(&self) -> DeckState {
        DeckState {
            loaded: self.is_loaded(),
            playing: self.playing,
            position_s: self.position_s(),
            duration_s: self.to_secs(self.len()),
            speed: if self.touched {
                self.scratch_speed
            } else if self.search != 0.0 {
                self.search
            } else if self.playing {
                self.speed() * self.motor * if self.reverse { -1.0 } else { 1.0 }
            } else {
                0.0
            },
            tempo: self.tempo(),
            bpm: self.bpm(),
            cue_s: self.cue_s(),
            looping: self.lp.is_some_and(|l| l.active),
            at_cue: self.is_loaded() && !self.playing && (self.pos - self.cue).abs() < 1.0,
            hot_cues: std::array::from_fn(|i| self.hot_cues[i].is_some()),
            reverse: self.reverse,
            slip: self.slip,
            slip_diverged: self.slip && self.diverged(),
            master_tempo: self.master_tempo,
            key: self.key,
            memory_cues: self.memory.len(),
            auto_cue: self.auto_cue,
            brake_s: self.brake_s,
        }
    }

    /// Playback differs from plain forward play (what slip mode returns from).
    fn diverged(&self) -> bool {
        self.touched || self.reverse || self.lp.is_some_and(|l| l.active) || self.hot_slip.is_some() || self.search != 0.0
    }

    /// Add this deck's audio (before channel fader / crossfader) to `out`, scaled by `gain`.
    pub fn render(&mut self, out: &mut [Frame], out_rate: u32, gain: f32) {
        let Some(pcm) = self.pcm.clone() else { return };
        let ratio = f64::from(pcm.sample_rate) / f64::from(out_rate);
        let ramp_step = (1.0 / (DECLICK_S * f64::from(out_rate))) as f32;
        let len = pcm.len() as f64;
        // bend decays to zero within ~0.2 s
        let bend_decay = (-1.0 / (0.05 * f64::from(out_rate))).exp();
        let motor_step = if self.brake_s > 0.0 { 1.0 / (self.brake_s * f64::from(out_rate)) } else { 1.0 };
        for o in out.iter_mut() {
            // motor: vinyl start/brake ramp
            let motor_target = if self.playing && !self.stopping { 1.0 } else { 0.0 };
            if self.motor < motor_target {
                self.motor = (self.motor + motor_step).min(1.0);
            } else if self.motor > motor_target {
                self.motor = (self.motor - motor_step).max(0.0);
                if self.motor == 0.0 && self.stopping {
                    self.stop_now();
                }
            }
            let forward = self.speed() * self.motor;
            let speed = if self.touched {
                // follow the platter: velocity proportional to the remaining distance
                let v = (self.scratch_target - self.pos) / (0.01 * f64::from(pcm.sample_rate));
                self.scratch_speed += (v - self.scratch_speed) * 0.05;
                self.scratch_speed
            } else if self.search != 0.0 {
                self.search
            } else if self.playing {
                if self.reverse { -forward } else { forward }
            } else {
                0.0
            };
            let audible = self.touched || self.playing || self.search != 0.0;
            let target = if audible { 1.0 } else { 0.0 };
            if self.ramp < target {
                self.ramp = (self.ramp + ramp_step).min(1.0);
            } else if self.ramp > target {
                self.ramp = (self.ramp - ramp_step).max(0.0);
            }
            // key lock / key shift
            let mut pitch = 2f32.powf(self.key as f32 / 12.0);
            if self.master_tempo && speed.abs() > 0.05 && !self.touched {
                pitch /= speed.abs() as f32;
            }
            let shifting = (pitch - 1.0).abs() > 1e-3;
            if self.ramp > 0.0 || shifting {
                let mut f = pcm.sample(self.pos);
                if shifting {
                    f = self.shifter.process(f, pitch);
                }
                let g = gain * self.ramp;
                o[0] += f[0] * g;
                o[1] += f[1] * g;
            }
            self.pos += speed * ratio;
            if let Some(l) = self.lp {
                let span = (l.end - l.start).max(1.0);
                if l.active && speed > 0.0 && self.pos >= l.end {
                    self.pos = l.start + (self.pos - l.end) % span;
                } else if l.active && speed < 0.0 && self.pos < l.start {
                    self.pos = l.end - (l.start - self.pos) % span;
                }
            }
            // slip: the background position runs on as if nothing happened
            if self.playing {
                self.shadow += forward * ratio;
            }
            let diverged = self.diverged();
            if !self.slip || (!diverged && !self.was_diverged) {
                self.shadow = self.pos;
            } else if self.was_diverged && !diverged {
                self.pos = self.shadow.min(len);
                self.scratch_target = self.pos;
            }
            self.was_diverged = self.slip && diverged;
            if self.pos >= len {
                self.pos = len;
                self.stop_now();
                self.search = 0.0;
            } else if self.pos < 0.0 {
                self.pos = 0.0;
                if self.reverse || self.search < 0.0 {
                    self.search = 0.0;
                }
            }
            if !self.touched {
                self.bend *= bend_decay;
            }
        }
    }
}
