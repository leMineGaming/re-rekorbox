//! DJ controller binding: MIDI messages → [`Command`]s, and LED feedback from the mixer state.
//!
//! Input goes through the clean-room MIDI layer (`rekordbox_formats::midi_input`, SPEC §5f-2): the
//! mapping CSV turns a message into a control event (function name + deck + action), and
//! [`Controller`] turns that event into engine commands. Function names are the ones used in
//! rekordbox's mapping files (e.g. `XDJ-RX3.midi.csv`); rows with an empty function column use the
//! row name (`PAD5_BeatLoop`).
//!
//! Which function does what is an own design for the MVP (the engine has no rekordbox spec); where
//! sizes are not given by the file (beat-loop / beat-jump pad sizes, gain range) they are in
//! [`ControllerConfig`].

use std::collections::BTreeSet;

use rekordbox_formats::midi::{MappingEntry, MidiMapping};
use rekordbox_formats::midi_input::{Action, InputProcessor, MidiMessage};

use crate::deck::{HOT_CUES, TEMPO_RANGES};
use crate::dsp::ColorFx;
use crate::fx::{BeatFxKind, FxAssign, TapTempo};
use crate::mixer::{Command, MixerState, DECKS};

/// Tunables that the mapping file does not define.
#[derive(Debug, Clone, PartialEq)]
pub struct ControllerConfig {
    /// Beat-loop pad sizes in beats (pads 1..8 of the beat-loop mode).
    pub beat_loop_sizes: [f64; 8],
    /// Beat-jump sizes for pad pairs (pad 1/2 = first size back/forward, …).
    pub beat_jump_sizes: [f64; 4],
    /// Invert the tempo fader direction.
    pub tempo_invert: bool,
    /// Gain knob range ±dB around the centre.
    pub gain_range_db: f32,
    /// Seconds moved per tick of the search wheel / jog search.
    pub search_seconds_per_tick: f64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            beat_loop_sizes: [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            beat_jump_sizes: [1.0, 2.0, 4.0, 8.0],
            tempo_invert: false,
            gain_range_db: 12.0,
            search_seconds_per_tick: 0.05,
        }
    }
}

/// What a mapping function controls in the engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    PlayPause,
    Cue,
    HotCueCall(usize),
    HotCueDelete(usize),
    BeatLoopPad(usize),
    BeatJumpPad(usize),
    LoopIn,
    LoopOut,
    ReloopExit,
    Sync,
    Quantize,
    TempoRange,
    TempoSlider,
    Jog,
    Search,
    JogTouch,
    Vinyl,
    Needle,
    ChannelFader,
    Gain,
    CrossFader,
    /// EQ band 0 = low, 1 = mid, 2 = high.
    Eq(usize),
    /// Sound Color FX knob of mixer channel n (0-based).
    ColorKnob(usize),
    ColorSub,
    ColorSelect(ColorFx),
    BeatFxOn,
    BeatFxNext,
    BeatFxSelect(BeatFxKind),
    BeatFxDepth,
    BeatFxBeat(i32),
    BeatFxTap,
    BeatFxAuto,
    BeatFxAdjust(f32),
    BeatFxAssign(FxAssign),
    /// Pad FX bank (0/1) and slot (0..15).
    PadFx(usize, usize),
}

impl Target {
    /// Targets that are not tied to a deck column (mixer section).
    pub fn is_global(&self) -> bool {
        matches!(
            self,
            Target::CrossFader
                | Target::ColorKnob(_)
                | Target::ColorSub
                | Target::ColorSelect(_)
                | Target::BeatFxOn
                | Target::BeatFxNext
                | Target::BeatFxSelect(_)
                | Target::BeatFxDepth
                | Target::BeatFxBeat(_)
                | Target::BeatFxTap
                | Target::BeatFxAuto
                | Target::BeatFxAdjust(_)
                | Target::BeatFxAssign(_)
        )
    }
}

fn pad_number(s: &str) -> Option<usize> {
    let digits: String = s.strip_prefix("PAD")?.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Classify a function name (or row name for rows without a function).
pub fn target_of(function: &str) -> Option<Target> {
    use Target::*;
    Some(match function {
        "PlayPause" => PlayPause,
        "Cue" => Cue,
        "LoopIn" => LoopIn,
        "LoopOut" => LoopOut,
        "ReloopExit" => ReloopExit,
        "Sync" => Sync,
        "Quantize" => Quantize,
        "TempoRange" => TempoRange,
        "TempoSlider" => TempoSlider,
        "JogScratch" | "WheelPitchBend" => Jog,
        "JogSearch" | "WheelSearch" => Search,
        "JogTouch" => JogTouch,
        "Vinyl" => Vinyl,
        "NeedleSearch" => Needle,
        "ChannelFader" => ChannelFader,
        "Gain" => Gain,
        "CrossFader" => CrossFader,
        "EQLow" => Eq(0),
        "EQMid" => Eq(1),
        "EQHigh" => Eq(2),
        "CFXSubParameter" => ColorSub,
        // function names as in the mapping file (its comments name the effects)
        "CFX1On" => ColorSelect(ColorFx::Filter),
        "CFX2On" => ColorSelect(ColorFx::Space),
        "CFX3On" => ColorSelect(ColorFx::DubEcho),
        "CFX4On" => ColorSelect(ColorFx::Sweep),
        "CFX5On" => ColorSelect(ColorFx::Noise),
        "CFX6On" => ColorSelect(ColorFx::Crush),
        "FX1-1On" => BeatFxOn,
        "FX1-1On+Shift" => BeatFxNext,
        "FX1-1" => BeatFxDepth,
        "FX1BeatDown" => BeatFxBeat(-1),
        "FX1BeatUp" => BeatFxBeat(1),
        "FX1Tap" => BeatFxTap,
        "FX1Tap+Shift" => BeatFxAuto,
        "FX1ReleaseFx" => BeatFxAdjust(1.0),
        "FX1ReleaseFx+Shift" => BeatFxAdjust(10.0),
        "FX1Assign.CH1" => BeatFxAssign(FxAssign::Channel(0)),
        "FX1Assign.CH2" => BeatFxAssign(FxAssign::Channel(1)),
        "FX1Assign.CH3" => BeatFxAssign(FxAssign::Channel(2)),
        "FX1Assign.CH4" => BeatFxAssign(FxAssign::Channel(3)),
        "FX1Assign.MASTER" => BeatFxAssign(FxAssign::Master),
        "FX1Assign.SAMPLER" => BeatFxAssign(FxAssign::Sampler),
        f if BeatFxKind::from_name(f).is_some() => BeatFxSelect(BeatFxKind::from_name(f)?),
        f if f.starts_with("CFXParameterCH") => {
            let n: usize = f["CFXParameterCH".len()..].parse().ok()?;
            ColorKnob(n.checked_sub(1)?)
        }
        f => {
            let n = pad_number(f)?;
            if f.ends_with(".HotCue.Call") && (1..=HOT_CUES).contains(&n) {
                HotCueCall(n - 1)
            } else if f.ends_with(".HotCue.Delete") && (1..=HOT_CUES).contains(&n) {
                HotCueDelete(n - 1)
            } else if f.ends_with("_BeatLoop") && (5..=12).contains(&n) {
                // beat-loop pads are named PAD5..PAD12 in the file (notes 0x10..0x17, plus shift layer)
                BeatLoopPad(n - 5)
            } else if (f.ends_with(".BeatJump") || f.ends_with("_BeatJump+Shift")) && (1..=8).contains(&n) {
                BeatJumpPad(n - 1)
            } else if (f.ends_with(".PadFx1") || f.ends_with("_PadFx1")) && (1..=16).contains(&n) {
                PadFx(0, n - 1)
            } else if (f.ends_with(".PadFx2") || f.ends_with("_PadFx2")) && (1..=16).contains(&n) {
                PadFx(1, n - 1)
            } else {
                return None;
            }
        }
    })
}

/// Name used for an entry: the function column, or the row name when it is empty.
pub fn function_of(e: &MappingEntry) -> &str {
    if e.function.is_empty() {
        &e.name
    } else {
        &e.function
    }
}

#[derive(Debug, Clone, Copy)]
struct DeckCtl {
    vinyl: bool,
    quantize: bool,
    range: usize,
    touched: bool,
}

impl Default for DeckCtl {
    fn default() -> Self {
        DeckCtl { vinyl: true, quantize: false, range: 2, touched: false }
    }
}

/// LED message to send: packed short message (`status | d1 << 8 | value << 16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Led {
    pub status: u8,
    pub data1: u8,
    pub value: u8,
}

impl Led {
    pub fn packed(&self) -> u32 {
        u32::from(self.status) | u32::from(self.data1) << 8 | u32::from(self.value) << 16
    }
}

pub struct Controller {
    proc: InputProcessor,
    decks: [DeckCtl; DECKS],
    color_fx: Option<ColorFx>,
    fx_on: bool,
    fx_kind: BeatFxKind,
    tap: TapTempo,
    /// Time of the message being handled (seconds).
    now: f64,
    pub config: ControllerConfig,
}

impl Controller {
    pub fn new(mapping: MidiMapping) -> Controller {
        Controller {
            proc: InputProcessor::new(mapping),
            decks: Default::default(),
            color_fx: None,
            fx_on: false,
            fx_kind: BeatFxKind::Delay,
            tap: TapTempo::default(),
            now: 0.0,
            config: ControllerConfig::default(),
        }
    }

    pub fn mapping(&self) -> &MidiMapping {
        self.proc.mapping()
    }

    /// Function names with input messages, split into (handled, not handled).
    pub fn coverage(&self) -> (BTreeSet<String>, BTreeSet<String>) {
        let m = self.proc.mapping();
        let (mut yes, mut no) = (BTreeSet::new(), BTreeSet::new());
        for e in &m.entries {
            if e.input_bindings(&m.device).is_empty() {
                continue;
            }
            let f = function_of(e).to_string();
            if target_of(&f).is_some() {
                yes.insert(f);
            } else {
                no.insert(f);
            }
        }
        (yes, no)
    }

    /// Process one MIDI message into engine commands.
    pub fn handle(&mut self, msg: &MidiMessage) -> Vec<Command> {
        self.now = msg.time_s;
        let events = self.proc.process(msg);
        let mut out = Vec::new();
        for ev in events {
            let f = function_of(self.proc.entry(&ev)).to_string();
            out.extend(self.map(&f, ev.deck, ev.action));
        }
        out
    }

    /// Map one control event. Deck numbers 1 and 2 address the engine decks; deck 0 is global;
    /// decks 3 and 4 are ignored (two-deck engine).
    pub fn map(&mut self, function: &str, deck: u8, action: Action) -> Vec<Command> {
        use Command as C;
        let Some(t) = target_of(function) else { return vec![] };
        let d = match (t, deck) {
            (Target::ColorKnob(n), _) if n < DECKS => n,
            (Target::ColorKnob(_), _) => return vec![],
            (t, _) if t.is_global() => 0,
            (_, 1 | 2) => usize::from(deck - 1),
            _ => return vec![],
        };
        let ctl = &mut self.decks[d];
        let cfg = &self.config;
        let pressed = match action {
            Action::Switch { on, .. } => Some(on),
            Action::PadPress { pressed } => Some(pressed),
            _ => None,
        };
        let abs = match action {
            Action::Absolute { value } => Some(f64::from(value) / 16383.0),
            _ => None,
        };
        match (t, pressed, abs) {
            (Target::PlayPause, Some(true), _) => vec![C::TogglePlay(d)],
            (Target::Cue, Some(true), _) => vec![C::CuePress(d)],
            (Target::Cue, Some(false), _) => vec![C::CueRelease(d)],
            (Target::HotCueCall(n), Some(true), _) => vec![C::HotCuePress(d, n)],
            (Target::HotCueCall(n), Some(false), _) => vec![C::HotCueRelease(d, n)],
            (Target::HotCueDelete(n), Some(true), _) => vec![C::HotCueDelete(d, n)],
            (Target::BeatLoopPad(k), Some(true), _) => vec![C::BeatLoop(d, cfg.beat_loop_sizes[k])],
            (Target::BeatJumpPad(k), Some(true), _) => {
                let beats = cfg.beat_jump_sizes[k / 2];
                vec![C::BeatJump(d, if k % 2 == 0 { -beats } else { beats })]
            }
            (Target::LoopIn, Some(true), _) => vec![C::LoopIn(d)],
            (Target::LoopOut, Some(true), _) => vec![C::LoopOut(d)],
            (Target::ReloopExit, Some(true), _) => vec![C::ReloopExit(d)],
            (Target::Sync, Some(true), _) => vec![C::Sync(d)],
            (Target::Quantize, Some(true), _) => {
                ctl.quantize = !ctl.quantize;
                vec![C::Quantize(d, ctl.quantize)]
            }
            (Target::TempoRange, Some(true), _) => {
                ctl.range = (ctl.range + 1) % TEMPO_RANGES.len();
                vec![C::TempoRange(d, ctl.range)]
            }
            (Target::Vinyl, Some(true), _) => {
                ctl.vinyl = !ctl.vinyl;
                if !ctl.vinyl && ctl.touched {
                    ctl.touched = false;
                    return vec![C::JogTouch(d, false)];
                }
                vec![]
            }
            (Target::TempoSlider, _, Some(v)) => {
                let x = v * 2.0 - 1.0;
                vec![C::TempoFader(d, if cfg.tempo_invert { -x } else { x })]
            }
            (Target::Needle, _, Some(v)) => vec![C::SeekFraction(d, v)],
            (Target::ChannelFader, _, Some(v)) => vec![C::Volume(d, v as f32)],
            (Target::Gain, _, Some(v)) => {
                let db = (v as f32 * 2.0 - 1.0) * cfg.gain_range_db;
                vec![C::Trim(d, 10f32.powf(db / 20.0))]
            }
            (Target::CrossFader, _, Some(v)) => vec![C::Crossfader(v as f32 * 2.0 - 1.0)],
            (Target::Eq(band), _, Some(v)) => vec![C::Eq(d, band, v as f32)],
            (Target::ColorKnob(_), _, Some(v)) => vec![C::ColorKnob(d, v as f32)],
            (Target::ColorSub, _, Some(v)) => vec![C::ColorSub(v as f32)],
            (Target::BeatFxOn, Some(true), _) => {
                self.fx_on = !self.fx_on;
                vec![C::BeatFxOn(self.fx_on)]
            }
            (Target::BeatFxNext, Some(true), _) => {
                self.fx_kind = self.fx_kind.next();
                vec![C::BeatFxSelect(self.fx_kind)]
            }
            (Target::BeatFxSelect(k), Some(true), _) => {
                self.fx_kind = k;
                vec![C::BeatFxSelect(k)]
            }
            (Target::BeatFxDepth, _, Some(v)) => vec![C::BeatFxDepth(v as f32)],
            (Target::BeatFxBeat(step), Some(true), _) => vec![C::BeatFxBeat(step)],
            (Target::BeatFxTap, Some(true), _) => match self.tap.tap(self.now) {
                Some(bpm) => vec![C::BeatFxBpm(Some(bpm))],
                None => vec![],
            },
            (Target::BeatFxAuto, Some(true), _) => vec![C::BeatFxBpm(None)],
            (Target::BeatFxAdjust(ms), _, _) => match action {
                Action::Relative { delta } => vec![C::BeatFxAdjust(delta as f32 * ms)],
                _ => vec![],
            },
            (Target::BeatFxAssign(a), Some(true), _) => vec![C::BeatFxAssign(a)],
            (Target::PadFx(bank, slot), Some(true), _) => vec![C::PadFx(d, Some((bank, slot)))],
            (Target::PadFx(_, _), Some(false), _) => vec![C::PadFx(d, None)],
            (Target::ColorSelect(fx), Some(true), _) => {
                self.color_fx = if self.color_fx == Some(fx) { None } else { Some(fx) };
                vec![C::ColorFxSelect(self.color_fx)]
            }
            (Target::JogTouch, _, _) => match action {
                Action::Touch { touched, .. } if ctl.vinyl => {
                    ctl.touched = touched;
                    vec![C::JogTouch(d, touched)]
                }
                _ => vec![],
            },
            (Target::Jog, _, _) => match action {
                Action::JogDelta { delta, .. } => vec![C::Jog(d, delta)],
                _ => vec![],
            },
            (Target::Search, _, _) => match action {
                Action::JogDelta { delta, .. } | Action::SearchDelta { delta, .. } => {
                    vec![C::SeekBy(d, f64::from(delta) * cfg.search_seconds_per_tick)]
                }
                _ => vec![],
            },
            _ => vec![],
        }
    }

    /// LED messages for the current state: one per output binding of a handled function on decks
    /// 1 and 2 (value 0x7F = on, 0x00 = off).
    pub fn leds(&self, s: &MixerState) -> Vec<Led> {
        let m = self.proc.mapping();
        let mut out = Vec::new();
        for e in &m.entries {
            let Some(t) = target_of(function_of(e)) else { continue };
            for b in e.output_bindings() {
                if b.message == 0xFFFF {
                    continue;
                }
                let global_on = match t {
                    Target::ColorSelect(fx) => Some(s.color_fx == Some(fx)),
                    Target::BeatFxOn => Some(s.beat_fx.on),
                    Target::BeatFxTap => Some(!s.beat_fx.auto_bpm),
                    Target::BeatFxAuto => Some(s.beat_fx.auto_bpm),
                    _ => None,
                };
                if let Some(on) = global_on {
                    out.push(Led { status: (b.message >> 8) as u8, data1: b.message as u8, value: if on { 0x7F } else { 0 } });
                    continue;
                }
                if !(1..=2).contains(&b.deck) {
                    continue;
                }
                let d = usize::from(b.deck - 1);
                let st = &s.decks[d];
                let other = &s.decks[1 - d];
                let on = match t {
                    Target::PlayPause => st.playing,
                    Target::Cue => st.at_cue,
                    Target::HotCueCall(n) => st.hot_cues[n],
                    Target::LoopIn | Target::LoopOut | Target::ReloopExit => st.looping,
                    Target::Sync => matches!((st.bpm, other.bpm), (Some(a), Some(b)) if (a - b).abs() < 0.01),
                    Target::Quantize => self.decks[d].quantize,
                    Target::Vinyl => self.decks[d].vinyl,
                    Target::PadFx(bank, slot) => s.pad_fx[d] == Some((bank, slot)),
                    _ => continue,
                };
                out.push(Led { status: (b.message >> 8) as u8, data1: b.message as u8, value: if on { 0x7F } else { 0 } });
            }
        }
        out
    }
}
