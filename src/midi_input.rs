//! MIDI input processing (SPEC §5f-2): turns incoming MIDI messages into control events using a
//! parsed mapping file ([`crate::midi::MidiMapping`]).
//!
//! ```no_run
//! use rekordbox_formats::{midi, midi_input::{InputProcessor, MidiMessage}};
//! let text = std::fs::read_to_string("XDJ-RX3.midi.csv").unwrap();
//! let mut p = InputProcessor::new(midi::parse(&text));
//! for ev in p.process(&MidiMessage::from_bytes(&[0xB0, 0x22, 0x41], 0.5).unwrap()) {
//!     println!("{} deck {}: {:?}", p.entry(&ev).function, ev.deck, ev.action);
//! }
//! ```
//!
//! Each mapping row with input messages becomes one control per deck; the message key is
//! `status << 8 | data1` and the value is `data2` (0 for shorter messages). 14-bit controls listen on
//! `m` (MSB) and `m + 0x20` (LSB) and report only when the LSB arrives.

use std::collections::HashMap;

use crate::midi::{MappingEntry, MidiMapping};

/// One incoming MIDI message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiMessage {
    pub status: u8,
    pub data1: u8,
    /// Second data byte; `None` for messages with fewer than three bytes.
    pub data2: Option<u8>,
    /// Arrival time in seconds.
    pub time_s: f64,
}

/// Number of bytes of a short message with this status byte (running status is not handled).
pub fn message_len(status: u8) -> usize {
    match status {
        0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 3,
        0xC0..=0xDF | 0xF1 | 0xF3 => 2,
        _ => 1,
    }
}

impl MidiMessage {
    /// Build from raw bytes; `None` when empty or the first byte is not a status byte.
    pub fn from_bytes(bytes: &[u8], time_s: f64) -> Option<MidiMessage> {
        let status = *bytes.first()?;
        if status < 0x80 {
            return None;
        }
        Some(MidiMessage { status, data1: bytes.get(1).copied().unwrap_or(0), data2: bytes.get(2).copied(), time_s })
    }

    /// Build from a packed short message as delivered by WinMM (`status | d1 << 8 | d2 << 16`); the
    /// length follows from the status byte.
    pub fn from_packed(packed: u32, time_s: f64) -> Option<MidiMessage> {
        let b = packed.to_le_bytes();
        let len = message_len(b[0]);
        MidiMessage::from_bytes(&b[..len], time_s)
    }

    /// Lookup key as written in mapping files (`B022` = CC 0x22 on channel 0).
    pub fn key(&self) -> u16 {
        u16::from(self.status) << 8 | u16::from(self.data1)
    }

    /// Value byte (third byte, 0 if absent).
    pub fn value(&self) -> u8 {
        self.data2.unwrap_or(0)
    }

    pub fn time_ms(&self) -> f64 {
        self.time_s * 1000.0
    }
}

/// What a control reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Button and button-like pads: state = raw value ≠ 0, `value` = transformed current value.
    Switch { on: bool, value: u16 },
    /// `DdjRxPad`/`DdjRzPad` (e.g. XDJ-RX3): 0 → released, 0x7F → pressed.
    PadPress { pressed: bool },
    /// `DdjRxPad`/`DdjRzPad`: any other value (velocity/pressure) without a state change.
    PadValue { value: u8 },
    /// JogRotate: `value − 0x40`.
    JogDelta { delta: i32, time_ms: f64 },
    /// Difference (jog search): `value − 0x40`.
    SearchDelta { delta: i32, time_ms: f64 },
    /// JogTouch: touched = (value == 0x7F).
    Touch { touched: bool, time_ms: f64 },
    /// Rotary encoder step (after acceleration).
    Relative { delta: i32 },
    /// Absolute value: KnobSlider/Value (scaled to Min..Max), RawValue, RotaryAsKnob (0..16383),
    /// 14-bit controls (0..16383).
    Absolute { value: u16 },
}

/// A control event: which mapping row (index into `mapping.entries`) and deck (0 = global).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Event {
    pub entry: usize,
    pub deck: u8,
    pub action: Action,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PadTransform {
    /// `DdjSxPad` (0x21): pressed values → 0x7F, else 0.
    Sx,
    /// `DdjSzPad` (0x1F): > 0x40 → 0, else value.
    Sz,
    /// `DdjSx2Pad` (0x20): > 0x40 → 0, plus a remap of 0x1F..0x29.
    Sx2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Button { min: i64, max: i64 },
    ButtonPad(PadTransform),
    RxPad,
    JogRotate,
    Difference,
    JogTouch,
    Rotary { accel: Option<f64> },
    RotaryAsKnob { max: i32 },
    Scaled { min: i32, max: i32, center: f64 },
    Raw,
    HiResMsb,
    /// Index of the MSB control in `controls`.
    HiResLsb { msb: usize },
}

#[derive(Debug, Clone)]
struct Control {
    entry: usize,
    deck: u8,
    kind: Kind,
    /// Current value (object +0x110): last transformed value, 14-bit accumulator or knob position.
    current: i32,
}

/// Values the button "pressed" test accepts: 1..=64 except 63, and 127.
fn is_press_value(v: u8) -> bool {
    v != 0 && v != 0x3F && (v == 0x7F || v.wrapping_sub(1) < 0x40)
}

fn sx2_transform(v: u8) -> u8 {
    match v {
        0x41.. => 0,
        0x1F => 0x20,
        0x20..=0x22 => v + 2,
        0x23..=0x27 => v + 1,
        0x28 | 0x29 => 0x2A,
        _ => v,
    }
}

/// Rotary delta before acceleration [GH:0x14252cbf0].
pub fn rotary_delta(v: u8) -> i32 {
    match v {
        1..=30 => i32::from(v),
        98..=127 => i32::from(v) - 128,
        65..=94 | 34..=63 => i32::from(v) - 64,
        _ => i32::from(v),
    }
}

/// Acceleration curve for Rotary controls (factor `f`, SPEC §5f-2).
pub fn rotary_accelerate(d: i32, f: f64) -> i32 {
    if d < 0 {
        let a = -(f.powi(-d - 1).trunc());
        let a = if a.is_finite() && a >= f64::from(i32::MIN) { a as i32 } else { i32::MIN };
        a.max(-29)
    } else {
        let a = f.powi(d - 1).trunc();
        if !a.is_finite() || a > f64::from(i32::MAX) {
            return 0; // the original's int conversion overflows to a negative value → 0
        }
        (a as i32).min(30)
    }
}

/// KnobSlider/Value scaling of a 7-bit value into `min..=max` [GH:0x14252cac0].
fn scale(v: u8, min: i32, max: i32, center: f64) -> i32 {
    let (v, min_f) = (f64::from(v), f64::from(min));
    let x = if v < 64.0 {
        v * center * (1.0 / 64.0) + min_f
    } else if v == 64.0 {
        min_f + center
    } else {
        (v - 64.0) * (f64::from(max - min) - center) * (1.0 / 63.0) + min_f + center
    };
    x as i32
}

fn opt_or(v: Option<i64>, default: i64) -> i64 {
    v.unwrap_or(default)
}

/// Kind for a type code, or `None` when the type has no input handler here.
fn kind_for(code: u8, e: &MappingEntry) -> Option<Kind> {
    let o = &e.options;
    Some(match code {
        0 => Kind::Button { min: opt_or(o.min(), -1), max: opt_or(o.max(), -1) },
        0x1D | 0x1E => Kind::RxPad,
        0x1F => Kind::ButtonPad(PadTransform::Sz),
        0x20 => Kind::ButtonPad(PadTransform::Sx2),
        0x21 => Kind::ButtonPad(PadTransform::Sx),
        0x16 | 0x17 => Kind::JogRotate,
        0x0C => Kind::Difference,
        0x1A => Kind::JogTouch,
        0x0D => {
            let accel = match o.value() {
                None => Some(2.22),
                Some(v) if v > 0 => Some(v as f64 * 0.01),
                Some(_) => None,
            };
            Kind::Rotary { accel }
        }
        0x0E => {
            let v = opt_or(o.value(), -1);
            Kind::RotaryAsKnob { max: if v > 0 { v.min(0x3FFF) as i32 } else { 0x7F } }
        }
        0x06 | 0x0F => {
            let min = match o.min() {
                Some(m) if (0..=0x3FFF).contains(&m) => m as i32,
                _ => 0,
            };
            let max = match o.max() {
                Some(m) if (0..0x4000).contains(&m) => m as i32,
                _ if code == 0x06 => 0x3FFF,
                _ => 0x7F,
            };
            Kind::Scaled { min, max, center: f64::from((max - min + 1) / 2) }
        }
        0x10 => Kind::Raw,
        _ => return None,
    })
}

/// Routes incoming messages to the controls of a mapping and keeps their state.
#[derive(Debug, Clone)]
pub struct InputProcessor {
    mapping: MidiMapping,
    controls: Vec<Control>,
    routes: HashMap<u16, Vec<usize>>,
    /// Rows whose type has no input handler (index, type code).
    unhandled: Vec<(usize, u8)>,
}

impl InputProcessor {
    pub fn new(mapping: MidiMapping) -> InputProcessor {
        let mut p = InputProcessor { mapping, controls: Vec::new(), routes: HashMap::new(), unhandled: Vec::new() };
        if !p.mapping.is_valid() {
            return p;
        }
        let device = p.mapping.device.clone();
        for (idx, e) in p.mapping.entries.iter().enumerate() {
            let bindings = e.input_bindings(&device);
            if bindings.is_empty() {
                continue;
            }
            let code = e.control_code(&device);
            let fourteen = e.is_14bit(&device);
            let kind = if fourteen { Some(Kind::HiResMsb) } else { kind_for(code, e) };
            let Some(kind) = kind else {
                p.unhandled.push((idx, code));
                continue;
            };
            for b in bindings {
                let msb = p.controls.len();
                let start = if let Kind::RotaryAsKnob { max } = kind { (f64::from(max) * 0.5).round() as i32 } else { 0 };
                p.controls.push(Control { entry: idx, deck: b.deck, kind, current: start });
                p.routes.entry(b.message).or_default().push(msb);
                if fourteen {
                    let lsb = p.controls.len();
                    p.controls.push(Control { entry: idx, deck: b.deck, kind: Kind::HiResLsb { msb }, current: 0 });
                    p.routes.entry(MappingEntry::lsb_message(b.message)).or_default().push(lsb);
                }
            }
        }
        p
    }

    pub fn mapping(&self) -> &MidiMapping {
        &self.mapping
    }

    /// Mapping row of an event.
    pub fn entry(&self, ev: &Event) -> &MappingEntry {
        &self.mapping.entries[ev.entry]
    }

    /// Rows that have input messages but a type without an input handler (index, type code).
    pub fn unhandled(&self) -> &[(usize, u8)] {
        &self.unhandled
    }

    /// True when some control listens to this message key.
    pub fn listens(&self, key: u16) -> bool {
        self.routes.contains_key(&key)
    }

    /// Messages the processor listens to, sorted.
    pub fn listened_keys(&self) -> Vec<u16> {
        let mut k: Vec<u16> = self.routes.keys().copied().collect();
        k.sort_unstable();
        k
    }

    /// Process one message; returns one event per control that reports something, in mapping order.
    pub fn process(&mut self, msg: &MidiMessage) -> Vec<Event> {
        let Some(targets) = self.routes.get(&msg.key()).cloned() else {
            return Vec::new();
        };
        let v = msg.value();
        let t = msg.time_ms();
        let mut out = Vec::new();
        for ci in targets {
            let c = &mut self.controls[ci];
            let (entry, deck) = (c.entry, c.deck);
            let action = match c.kind {
                Kind::Button { min, max } => {
                    if v > 0x7F {
                        continue;
                    }
                    let value = if is_press_value(v) {
                        if max == -1 { 0x7F } else { max as u8 }
                    } else if min == -1 {
                        0
                    } else {
                        min as u8
                    };
                    c.current = i32::from(value);
                    Some(Action::Switch { on: v != 0, value: u16::from(value) })
                }
                Kind::ButtonPad(tr) => {
                    if v > 0x7F {
                        continue;
                    }
                    let value = match tr {
                        PadTransform::Sx => {
                            if is_press_value(v) {
                                0x7F
                            } else {
                                0
                            }
                        }
                        PadTransform::Sz => {
                            if v > 0x40 {
                                0
                            } else {
                                v
                            }
                        }
                        PadTransform::Sx2 => sx2_transform(v),
                    };
                    c.current = i32::from(value);
                    Some(Action::Switch { on: v != 0, value: u16::from(value) })
                }
                Kind::RxPad => {
                    c.current = i32::from(v);
                    Some(match v {
                        0 => Action::PadPress { pressed: false },
                        0x7F => Action::PadPress { pressed: true },
                        _ => Action::PadValue { value: v },
                    })
                }
                Kind::JogRotate => Some(Action::JogDelta { delta: i32::from(v) - 0x40, time_ms: t }),
                Kind::Difference => Some(Action::SearchDelta { delta: i32::from(v) - 0x40, time_ms: t }),
                Kind::JogTouch => Some(Action::Touch { touched: v == 0x7F, time_ms: t }),
                Kind::Rotary { accel } => {
                    let d = rotary_delta(v);
                    Some(Action::Relative { delta: accel.map_or(d, |f| rotary_accelerate(d, f)) })
                }
                Kind::RotaryAsKnob { max } => {
                    let acc = match v {
                        1..=63 => c.current + i32::from(v),
                        64..=127 => c.current - 128 + i32::from(v),
                        _ => c.current,
                    };
                    c.current = acc.clamp(0, max);
                    let value = (f64::from(c.current) / f64::from(max) * 16383.0) as u16;
                    Some(Action::Absolute { value })
                }
                Kind::Scaled { min, max, center } => {
                    c.current = scale(v, min, max, center);
                    Some(Action::Absolute { value: c.current as u16 })
                }
                Kind::Raw => {
                    c.current = i32::from(v);
                    Some(Action::Absolute { value: u16::from(v) })
                }
                Kind::HiResMsb => {
                    c.current = i32::from(v) << 7;
                    None
                }
                Kind::HiResLsb { msb } => {
                    let m = &mut self.controls[msb];
                    m.current |= i32::from(v);
                    Some(Action::Absolute { value: m.current as u16 })
                }
            };
            if let Some(action) = action {
                out.push(Event { entry, deck, action });
            }
        }
        out
    }
}
