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

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::browser::Browser;

use rekordbox_formats::midi::{MappingEntry, MidiMapping};
use rekordbox_formats::midi_input::{Action, InputProcessor, MidiMessage};

use crate::deck::{HOT_CUES, TEMPO_RANGES};
use crate::dsp::ColorFx;
use crate::fx::{BeatFxKind, FxAssign, TapTempo};
use crate::mixer::{Command, MixerState, DECKS};

/// Built-in mapping for the Pioneer DJ XDJ-RX3 (`engine/mappings/XDJ-RX3.csv`).
pub const XDJ_RX3_MAPPING: &str = include_str!("../mappings/XDJ-RX3.csv");

/// Mappings that ship with the program.
pub fn builtin_mappings() -> Vec<MidiMapping> {
    [XDJ_RX3_MAPPING].iter().map(|t| rekordbox_formats::midi::parse(t)).filter(|m| m.is_valid()).collect()
}

/// The built-in mapping whose device name occurs in a MIDI port name (e.g. "XDJ-RX3" in
/// "2- XDJ-RX3").
pub fn builtin_for_port(port: &str) -> Option<MidiMapping> {
    let p = port.to_lowercase();
    builtin_mappings().into_iter().find(|m| p.contains(&m.device.to_lowercase()))
}

/// Browser operations from the controller when the host shows the browser itself
/// ([`Controller::set_external_browser`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseEvent {
    /// BROWSE turned (steps, positive = down the list).
    Scroll(i32),
    /// SHIFT + BROWSE turned: page steps.
    Page(i32),
    /// BROWSE pressed.
    Press,
    Back,
    /// LOAD of a deck (0-based).
    Load(usize),
    /// TRACK ◀▶ of a deck: next (`true`) or previous track relative to the loaded one.
    Step(usize, bool),
    /// TAG TRACK: add or remove the selected track from the tag list.
    Tag,
    /// Show / leave the tag list.
    TagView,
    /// Tracks related to the selected one.
    Related,
}

/// One received message for the MIDI monitor.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorEntry {
    pub time_s: f64,
    pub bytes: Vec<u8>,
    /// Control functions it triggered (`function@deck`), empty if none.
    pub functions: Vec<String>,
    /// Engine commands it produced (short form).
    pub commands: Vec<String>,
    /// No mapping entry knows this message.
    pub unknown: bool,
}

/// Group of a control function, for the controller test.
pub fn category(function: &str) -> &'static str {
    let f = function.split('+').next().unwrap_or(function);
    if f.starts_with("PAD") || f.starts_with("Parameter") || matches!(f, "HotCue" | "Keyboard" | "BeatLoop" | "KeyShift" | "PadFx1" | "PadFx2" | "BeatJump" | "Sampler") {
        "Pads"
    } else if f.starts_with("FX") || f.starts_with("CFX") {
        "Effekte"
    } else if matches!(f, "Browse" | "Back" | "Load" | "AddToTagList" | "TrackFilter" | "Forward" | "Zoom" | "Area" | "RelatedTracks") {
        "Browser"
    } else if f.starts_with("EQ")
        || f.starts_with("AuxInput")
        || matches!(f, "CrossFader" | "ChannelFader" | "Gain" | "AuxTrim" | "HeadphoneCue" | "MasterCue" | "LinkCue" | "CrossFaderCurve3Pos" | "SamplerVolume" | "InputSelect" | "ChannelLevel")
    {
        "Mixer"
    } else {
        "Deck"
    }
}

/// Parse a MIDI log written by [`Controller::start_recording`]: lines `time_s status data1 [data2]`
/// (hex bytes), `#` comments.
pub fn parse_midilog(text: &str) -> Vec<MidiMessage> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let mut w = l.split_whitespace();
            let t: f64 = w.next()?.parse().ok()?;
            let bytes: Vec<u8> = w.filter_map(|h| u8::from_str_radix(h, 16).ok()).collect();
            MidiMessage::from_bytes(&bytes, t)
        })
        .collect()
}

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
    /// Key shift per KEY SHIFT pad (pads 1..8, then the shift layer 1..8), semitones.
    pub key_shift_pads: [i32; 16],
    /// Speed of the SEARCH buttons.
    pub search_speed: f64,
    /// Longest vinyl brake time (VINYL ADJUST BRAKE knob fully up), seconds.
    pub max_brake_s: f64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            beat_loop_sizes: [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0],
            beat_jump_sizes: [1.0, 2.0, 4.0, 8.0],
            tempo_invert: false,
            gain_range_db: 12.0,
            search_seconds_per_tick: 0.05,
            key_shift_pads: [1, 2, 3, 4, -1, -2, -3, -4, 5, 6, 7, 12, -5, -6, -7, -12],
            search_speed: 4.0,
            max_brake_s: 3.0,
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
    Reverse,
    Slip,
    MasterTempo,
    Brake,
    MemoryStore,
    MemoryCall(bool),
    MemoryDelete,
    TimeMode,
    AutoCue,
    FourBeatLoop,
    Master,
    InstantDouble,
    SearchButton(f64),
    /// KEY SHIFT pad index into `key_shift_pads`.
    KeyShiftPad(usize),
    /// KEYBOARD pad: semitones.
    KeyboardPad(i32),
    PadMode(PadMode),
    /// Output-only indicators.
    SlipFlash,
    Loaded,
    ChannelLevel,
    /// Known function that deliberately triggers nothing (see `ignore_reason`).
    NoOp,
    /// Sampler pad n (0..7) start / stop; the slot is `deck * 8 + n`.
    SamplerStart(usize),
    SamplerStop(usize),
    SamplerVolume,
    HeadphoneCue,
    MasterCue,
    CueLink,
    CrossfaderCurve,
    Browse,
    Zoom,
    BrowseEnter,
    Back,
    TagView,
    Tag,
    Related,
    Load,
    TrackStep(bool),
}

/// Pad modes of the XDJ-RX3 (mode buttons, with SHIFT the second mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PadMode {
    #[default]
    HotCue,
    Keyboard,
    BeatLoop,
    KeyShift,
    PadFx1,
    PadFx2,
    BeatJump,
    Sampler,
}

/// Why a function is mapped to [`Target::NoOp`].
pub fn ignore_reason(function: &str) -> Option<&'static str> {
    Some(match function {
        "JogPitchBend" => "same message as JogScratch; the deck decides by jog touch",
        "JogTouch+Shift" => "with SHIFT the jog sends JogSearch, the touch itself needs no action",
        "JogRatency" | "MidiOutInterval" => "device parameter rows, not controls",
        "DeckState" => "deck 1-4 select state; the engine has two decks",
        "InputSelect" => "USB/LINE input select is handled inside the unit",
        "AuxInputSelect.Off" | "AuxInputSelect.0dB" | "AuxInputSelect.12dB" => "aux input routing is handled inside the unit",
        _ => return None,
    })
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
                | Target::SamplerVolume
                | Target::MasterCue
                | Target::CueLink
                | Target::CrossfaderCurve
                | Target::Browse
                | Target::Zoom
                | Target::BrowseEnter
                | Target::Back
                | Target::TagView
                | Target::Tag
                | Target::Related
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
        "AuxTrim" => SamplerVolume,
        "HeadphoneCue" => HeadphoneCue,
        "MasterCue" => MasterCue,
        "LinkCue" => CueLink,
        "CrossFaderCurve" => CrossfaderCurve,
        "Browse" => Browse,
        "Browse+Shift" => Zoom,
        "Browse+Press" | "Browse+Press+Shift" => BrowseEnter,
        "Back" => Back,
        "Back+Shift" | "TrackFilter" | "TrackFilter+Shift" => TagView,
        "AddToTagList" => Tag,
        "AddToTagList+Shift" => Related,
        "Load" => Load,
        "TrackNext" => TrackStep(true),
        "TrackPrev" => TrackStep(false),
        "Reverse" => Reverse,
        "Slip" => Slip,
        "MasterTempo" => MasterTempo,
        "VinylAdjustBrake" => Brake,
        "Memory" => MemoryStore,
        "CallNext" => MemoryCall(true),
        "CallPrev" => MemoryCall(false),
        "Delete" => MemoryDelete,
        "TimeMode" => TimeMode,
        "TimeMode+LongPress" => AutoCue,
        "LoopIn+LongPress" => FourBeatLoop,
        "Master" => Master,
        "Sync+LongPress" => InstantDouble,
        "SearchFwd" => SearchButton(1.0),
        "SearchRev" => SearchButton(-1.0),
        "HotCue" => PadMode(self::PadMode::HotCue),
        "HotCue+Shift" => PadMode(self::PadMode::Keyboard),
        "BeatLoop" => PadMode(self::PadMode::BeatLoop),
        "BeatLoop+Shift" => PadMode(self::PadMode::KeyShift),
        "SlipLoop" => PadMode(self::PadMode::PadFx1),
        "SlipLoop+Shift" => PadMode(self::PadMode::PadFx2),
        "BeatJump" => PadMode(self::PadMode::BeatJump),
        "BeatJump+Shift" => PadMode(self::PadMode::Sampler),
        "SlipStateForFlashing" => SlipFlash,
        "LoadedIndicator" => Loaded,
        "ChannelLevel" => ChannelLevel,
        f if ignore_reason(f).is_some() => NoOp,
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
            } else if f.ends_with(".Sampler.Start") && (1..=8).contains(&n) {
                SamplerStart(n - 1)
            } else if f.ends_with(".Sampler.Stop") && (1..=8).contains(&n) {
                SamplerStop(n - 1)
            } else if f.ends_with("_KeyShift") && (1..=8).contains(&n) {
                KeyShiftPad(n - 1)
            } else if f.ends_with("_KeyShift+Shift") && (1..=8).contains(&n) {
                KeyShiftPad(n + 7)
            } else if f.ends_with("_Keyboard") && (1..=8).contains(&n) {
                KeyboardPad(n as i32 - 1)
            } else if f.ends_with("_Keyboard+Shift") && (1..=8).contains(&n) {
                KeyboardPad(n as i32 - 9)
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
    reverse: bool,
    slip: bool,
    master_tempo: bool,
    auto_cue: bool,
    time_remaining: bool,
    key: i32,
    pad_mode: PadMode,
}

impl Default for DeckCtl {
    fn default() -> Self {
        DeckCtl {
            vinyl: true,
            quantize: false,
            range: 2,
            touched: false,
            reverse: false,
            slip: false,
            master_tempo: false,
            auto_cue: false,
            time_remaining: false,
            key: 0,
            pad_mode: PadMode::HotCue,
        }
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
    sync_master: Option<usize>,
    cue: [bool; DECKS],
    master_cue: bool,
    cue_link: bool,
    browser: Option<Browser>,
    browser_changed: bool,
    /// Browser controls go to the host as [`BrowseEvent`]s instead of the built-in folder browser.
    external_browser: bool,
    browse_events: Vec<BrowseEvent>,
    /// Recent messages (monitor), controls seen since the last reset, unknown messages.
    monitor: VecDeque<MonitorEntry>,
    seen: BTreeSet<(String, u8)>,
    unknown: BTreeMap<(u8, u8), usize>,
    known_messages: HashSet<u16>,
    recorder: Option<std::io::BufWriter<std::fs::File>>,
    loaded: [Option<PathBuf>; DECKS],
    pending_loads: Vec<(usize, PathBuf)>,
    last_load: [f64; DECKS],
    tap: TapTempo,
    /// Time of the message being handled (seconds).
    now: f64,
    pub config: ControllerConfig,
}

impl Controller {
    pub fn new(mapping: MidiMapping) -> Controller {
        // every input message of the mapping, including the LSB half of 14-bit controls
        let mut known_messages = HashSet::new();
        for e in &mapping.entries {
            for b in e.input_bindings(&mapping.device) {
                known_messages.insert(b.message);
                if e.is_14bit(&mapping.device) {
                    known_messages.insert(MappingEntry::lsb_message(b.message));
                }
            }
        }
        let mut c = Controller::new_inner(mapping);
        c.known_messages = known_messages;
        c
    }

    fn new_inner(mapping: MidiMapping) -> Controller {
        Controller {
            proc: InputProcessor::new(mapping),
            decks: Default::default(),
            color_fx: None,
            fx_on: false,
            fx_kind: BeatFxKind::Delay,
            sync_master: None,
            cue: [false; DECKS],
            master_cue: false,
            cue_link: false,
            browser: None,
            browser_changed: false,
            external_browser: false,
            browse_events: Vec::new(),
            monitor: VecDeque::new(),
            seen: BTreeSet::new(),
            unknown: BTreeMap::new(),
            known_messages: HashSet::new(),
            recorder: None,
            loaded: Default::default(),
            pending_loads: Vec::new(),
            last_load: [f64::NEG_INFINITY; DECKS],
            tap: TapTempo::default(),
            now: 0.0,
            config: ControllerConfig::default(),
        }
    }

    pub fn mapping(&self) -> &MidiMapping {
        self.proc.mapping()
    }

    /// Active pad mode of a deck (0-based).
    pub fn pad_mode(&self, deck: usize) -> PadMode {
        self.decks[deck].pad_mode
    }

    /// Start the file browser at `root` (BROWSE / LOAD / TRACK controls need it).
    pub fn set_browser_root(&mut self, root: PathBuf) {
        self.browser = Some(Browser::new(root));
        self.browser_changed = true;
    }

    /// Let the host show the browser: BROWSE, LOAD, BACK, TAG and TRACK ◀▶ become [`BrowseEvent`]s
    /// ([`Controller::take_browse_events`]).
    pub fn set_external_browser(&mut self, on: bool) {
        self.external_browser = on;
    }

    pub fn take_browse_events(&mut self) -> Vec<BrowseEvent> {
        std::mem::take(&mut self.browse_events)
    }

    pub fn browser(&self) -> Option<&Browser> {
        self.browser.as_ref()
    }

    /// Library tracks for the browser's `[Library]` folder.
    pub fn set_library(&mut self, tracks: Vec<PathBuf>) {
        if let Some(b) = self.browser.as_mut() {
            b.set_library(tracks);
            self.browser_changed = true;
        }
    }

    /// True once after the browser view or selection changed.
    pub fn take_browser_changed(&mut self) -> bool {
        std::mem::take(&mut self.browser_changed)
    }

    /// Loads requested by LOAD / TRACK ◀▶: the host decodes the file and sends `Command::Load`.
    pub fn take_loads(&mut self) -> Vec<(usize, PathBuf)> {
        std::mem::take(&mut self.pending_loads)
    }

    /// Tell the controller which file a deck holds (for TRACK ◀▶ after a load from elsewhere).
    pub fn set_loaded_path(&mut self, deck: usize, path: &Path) {
        if deck < DECKS {
            self.loaded[deck] = Some(path.to_path_buf());
        }
    }

    pub fn loaded_path(&self, deck: usize) -> Option<&Path> {
        self.loaded.get(deck)?.as_deref()
    }

    fn request_load(&mut self, deck: usize, path: PathBuf) {
        self.loaded[deck] = Some(path.clone());
        self.pending_loads.push((deck, path));
    }

    /// TIME MODE: true = remaining time is shown.
    pub fn time_remaining(&self, deck: usize) -> bool {
        self.decks[deck].time_remaining
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
        let mut bytes = vec![msg.status, msg.data1];
        bytes.extend(msg.data2);
        if let Some(r) = self.recorder.as_mut() {
            use std::io::Write;
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
            let _ = writeln!(r, "{:.4} {}", msg.time_s, hex.join(" "));
        }
        let events = self.proc.process(msg);
        let mut out = Vec::new();
        let mut functions = Vec::new();
        for ev in events {
            let f = function_of(self.proc.entry(&ev)).to_string();
            functions.push(format!("{f}@{}", ev.deck));
            self.seen.insert((f.clone(), ev.deck));
            out.extend(self.map(&f, ev.deck, ev.action));
        }
        let key = u16::from(msg.status) << 8 | u16::from(msg.data1);
        let unknown = functions.is_empty() && !self.known_messages.contains(&key) && msg.status < 0xF0;
        if unknown {
            *self.unknown.entry((msg.status, msg.data1)).or_default() += 1;
        }
        let commands = out.iter().map(|c| format!("{c:?}").chars().take(60).collect()).collect();
        if self.monitor.len() >= 200 {
            self.monitor.pop_front();
        }
        self.monitor.push_back(MonitorEntry { time_s: msg.time_s, bytes, functions, commands, unknown });
        out
    }

    /// Recent messages, oldest first.
    pub fn monitor(&self) -> &VecDeque<MonitorEntry> {
        &self.monitor
    }

    /// Controls (function, deck) that sent something since the last [`Controller::reset_test`].
    pub fn seen(&self) -> &BTreeSet<(String, u8)> {
        &self.seen
    }

    /// Messages without mapping entry: (status, data1) → count.
    pub fn unknown(&self) -> &BTreeMap<(u8, u8), usize> {
        &self.unknown
    }

    pub fn reset_test(&mut self) {
        self.seen.clear();
        self.unknown.clear();
        self.monitor.clear();
    }

    /// All controls of the mapping that send something: (category, function, deck), sorted.
    pub fn testable_controls(&self) -> Vec<(&'static str, String, u8)> {
        let m = self.proc.mapping();
        let mut v: BTreeSet<(&'static str, String, u8)> = BTreeSet::new();
        for e in &m.entries {
            let f = function_of(e).to_string();
            if ignore_reason(&f).is_some() {
                continue;
            }
            for b in e.input_bindings(&m.device) {
                if b.deck <= 2 {
                    v.insert((category(&f), f.clone(), b.deck));
                }
            }
        }
        v.into_iter().collect()
    }

    /// Every LED / indicator output of the mapping switched on (for an LED test), with its function.
    pub fn led_outputs(&self) -> Vec<(String, Led)> {
        let m = self.proc.mapping();
        let mut v = Vec::new();
        let mut seen = HashSet::new();
        for e in &m.entries {
            for b in e.output_bindings() {
                if b.message != 0xFFFF && seen.insert(b.message) {
                    v.push((function_of(e).to_string(), Led { status: (b.message >> 8) as u8, data1: b.message as u8, value: 0x7F }));
                }
            }
        }
        v
    }

    /// Append every received message to `path` (a MIDI log, see [`parse_midilog`]).
    pub fn start_recording(&mut self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(f, "# rekordbox-rs MIDI log, device {}", self.proc.mapping().device)?;
        writeln!(f, "# time_s status data1 data2")?;
        self.recorder = Some(f);
        Ok(())
    }

    pub fn stop_recording(&mut self) {
        if let Some(mut r) = self.recorder.take() {
            use std::io::Write;
            let _ = r.flush();
        }
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// Feed recorded messages and collect the engine commands with their times.
    pub fn replay(&mut self, messages: &[MidiMessage]) -> Vec<(f64, Command)> {
        let mut out = Vec::new();
        for m in messages {
            for c in self.handle(m) {
                out.push((m.time_s, c));
            }
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
            (Target::SamplerStart(n), Some(true), _) => vec![C::SamplerStart(d * 8 + n)],
            (Target::SamplerStop(n), Some(true), _) => vec![C::SamplerStop(d * 8 + n)],
            (Target::SamplerVolume, _, Some(v)) => vec![C::SamplerVolume(v as f32)],
            (Target::HeadphoneCue, Some(true), _) => {
                let on = !self.cue[d];
                if on && self.cue_link {
                    self.cue = [false; DECKS];
                }
                self.cue[d] = on;
                vec![C::HeadphoneCue(d, on)]
            }
            (Target::MasterCue, Some(true), _) => {
                self.master_cue = !self.master_cue;
                vec![C::MasterCue(self.master_cue)]
            }
            (Target::CueLink, Some(true), _) => {
                self.cue_link = !self.cue_link;
                vec![C::CueLink(self.cue_link)]
            }
            (Target::CrossfaderCurve, _, Some(v)) => vec![C::CrossfaderCurve(v as f32)],
            (Target::Browse | Target::Zoom, _, _) if self.external_browser => {
                if let Action::Relative { delta } = action {
                    self.browse_events.push(if t == Target::Browse { BrowseEvent::Scroll(delta) } else { BrowseEvent::Page(delta) });
                }
                vec![]
            }
            (Target::BrowseEnter | Target::Back | Target::TagView | Target::Tag | Target::Related, Some(true), _) if self.external_browser => {
                self.browse_events.push(match t {
                    Target::BrowseEnter => BrowseEvent::Press,
                    Target::Back => BrowseEvent::Back,
                    Target::TagView => BrowseEvent::TagView,
                    Target::Tag => BrowseEvent::Tag,
                    _ => BrowseEvent::Related,
                });
                vec![]
            }
            (Target::Browse | Target::Zoom, _, _) => {
                if let (Some(b), Action::Relative { delta }) = (self.browser.as_mut(), action) {
                    if t == Target::Browse {
                        b.scroll(delta);
                    } else {
                        b.zoom(delta.signum());
                    }
                    self.browser_changed = true;
                }
                vec![]
            }
            (Target::BrowseEnter | Target::Back | Target::TagView | Target::Tag | Target::Related, Some(true), _) => {
                if let Some(b) = self.browser.as_mut() {
                    match t {
                        Target::BrowseEnter => {
                            b.enter();
                        }
                        Target::Back => b.back(),
                        Target::TagView => b.toggle_tag_view(),
                        Target::Tag => b.toggle_tag(),
                        _ => b.show_related(),
                    }
                    self.browser_changed = true;
                }
                vec![]
            }
            (Target::Load, Some(true), _) => {
                // a second press within 0.4 s doubles the other deck instead
                if self.now - self.last_load[d] < 0.4 {
                    self.last_load[d] = f64::NEG_INFINITY;
                    self.pending_loads.retain(|(deck, _)| *deck != d);
                    self.loaded[d] = self.loaded[1 - d].clone();
                    return vec![C::InstantDouble(d)];
                }
                self.last_load[d] = self.now;
                if self.external_browser {
                    self.browse_events.push(BrowseEvent::Load(d));
                    return vec![];
                }
                if let Some(p) = self.browser.as_ref().and_then(|b| b.selected_track()) {
                    self.request_load(d, p);
                }
                vec![]
            }
            (Target::TrackStep(fwd), Some(true), _) if self.external_browser => {
                self.browse_events.push(BrowseEvent::Step(d, fwd));
                vec![]
            }
            (Target::TrackStep(fwd), Some(true), _) => {
                if let Some(next) = self.loaded[d].as_deref().and_then(|p| Browser::neighbour(p, fwd)) {
                    self.request_load(d, next);
                }
                vec![]
            }
            (Target::Reverse, Some(true), _) => {
                ctl.reverse = !ctl.reverse;
                vec![C::Reverse(d, ctl.reverse)]
            }
            (Target::Slip, Some(true), _) => {
                ctl.slip = !ctl.slip;
                vec![C::Slip(d, ctl.slip)]
            }
            (Target::MasterTempo, Some(true), _) => {
                ctl.master_tempo = !ctl.master_tempo;
                vec![C::MasterTempo(d, ctl.master_tempo)]
            }
            (Target::Brake, _, Some(v)) => vec![C::Brake(d, v * cfg.max_brake_s)],
            (Target::MemoryStore, Some(true), _) => vec![C::MemoryStore(d)],
            (Target::MemoryCall(fwd), Some(true), _) => vec![C::MemoryCall(d, fwd)],
            (Target::MemoryDelete, Some(true), _) => vec![C::MemoryDelete(d)],
            (Target::TimeMode, Some(true), _) => {
                ctl.time_remaining = !ctl.time_remaining;
                vec![]
            }
            (Target::AutoCue, Some(true), _) => {
                ctl.auto_cue = !ctl.auto_cue;
                vec![C::AutoCue(d, ctl.auto_cue)]
            }
            (Target::FourBeatLoop, Some(true), _) => vec![C::BeatLoop(d, 4.0)],
            (Target::Master, Some(true), _) => {
                self.sync_master = if self.sync_master == Some(d) { None } else { Some(d) };
                vec![C::SetMaster(self.sync_master)]
            }
            (Target::InstantDouble, Some(true), _) => vec![C::InstantDouble(d)],
            (Target::SearchButton(dir), Some(true), _) => vec![C::Search(d, dir * cfg.search_speed)],
            (Target::SearchButton(_), Some(false), _) => vec![C::Search(d, 0.0)],
            (Target::KeyShiftPad(i), Some(true), _) => {
                let k = cfg.key_shift_pads[i];
                ctl.key = if ctl.key == k { 0 } else { k };
                vec![C::Key(d, ctl.key)]
            }
            (Target::KeyboardPad(k), Some(true), _) => {
                ctl.key = k;
                vec![C::KeyboardPress(d, k)]
            }
            (Target::KeyboardPad(_), Some(false), _) => vec![C::KeyboardRelease(d)],
            (Target::PadMode(m), Some(true), _) => {
                ctl.pad_mode = m;
                vec![]
            }
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
                    Target::Reverse => st.reverse,
                    Target::SamplerStart(n) => s.sampler_playing & (1 << (d * 8 + n)) != 0,
                    Target::HeadphoneCue => s.cue[d],
                    Target::Slip => st.slip,
                    Target::SlipFlash => st.slip_diverged,
                    Target::MasterTempo => st.master_tempo,
                    Target::Master => s.master == Some(d),
                    Target::FourBeatLoop => st.looping,
                    Target::Loaded => st.loaded,
                    Target::KeyShiftPad(i) => st.key != 0 && st.key == self.config.key_shift_pads[i],
                    Target::KeyboardPad(k) => st.key == k,
                    Target::PadMode(m) => self.decks[d].pad_mode == m,
                    Target::ChannelLevel => {
                        let v = (s.channel_peak[d].min(1.0) * 127.0).round() as u8;
                        out.push(Led { status: (b.message >> 8) as u8, data1: b.message as u8, value: v });
                        continue;
                    }
                    _ => continue,
                };
                out.push(Led { status: (b.message >> 8) as u8, data1: b.message as u8, value: if on { 0x7F } else { 0 } });
            }
        }
        out
    }
}
