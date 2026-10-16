//! MIDI mapping CSV files (`MidiMappings/*.midi.csv`, SPEC §5f).
//!
//! ```text
//! @file,1,DDJ-FLX4
//! #name,function,type,input,deck1,deck2,deck3,deck4,output,deck1,deck2,deck3,deck4,option,comment
//! TempoSlider,TempoSlider,KnobSliderHiRes,B000,0,1,2,3,,,,,,Fast,Tempo Control
//! ```
//!
//! [`parse`] follows the original loader: comment/empty lines skipped, rows need 15 fields, only
//! files with `@file` version 1 are used. [`MappingEntry::input_bindings`] computes the MIDI message
//! per deck (hex message or channel substitution, `Dual` pairing).

use std::fmt;

/// Column names in file order (§5f "Zeile").
pub const COLUMNS: [&str; 15] = [
    "Name", "Function", "Type", "Input", "InputDeck1", "InputDeck2", "InputDeck3", "InputDeck4", "Output",
    "OutputDeck1", "OutputDeck2", "OutputDeck3", "OutputDeck4", "Option", "Comment",
];

/// Header line written by rekordbox's own files.
pub const HEADER_LINE: &str = "#name,function,type,input,deck1,deck2,deck3,deck4,output,deck1,deck2,deck3,deck4,option,comment";

/// Option keys the loader recognises (§5f "Optionen").
pub const OPTION_KEYS: [&str; 8] = ["RO", "Fast", "Blink", "Value", "Min", "Max", "Priority", "Dual"];

/// Internal code of the `JogTimestamp` type (deck values are not shifted for it).
pub const JOG_TIMESTAMP: u8 = 0x19;

// ---------------------------------------------------------------------------------------------
// Number parsing (§5f "Zahlen")

/// Hex value of a field: every hex digit is appended, every other character is skipped.
pub fn hex_value(s: &str) -> u32 {
    s.chars().filter_map(|c| c.to_digit(16)).fold(0u32, |v, d| v.wrapping_shl(4) | d)
}

/// Decimal value like `strtol(s, 10)`: leading whitespace, optional sign, digits up to the first
/// non-digit; empty or non-numeric → 0; clamped to the 32-bit range.
pub fn int_value(s: &str) -> i64 {
    let t = s.trim_start();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for c in digits.chars() {
        match c.to_digit(10) {
            Some(d) => v = (v * 10 + d as i64).min(i64::from(i32::MAX) + 1),
            None => break,
        }
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX))
}

// ---------------------------------------------------------------------------------------------
// Control types (§5f "Typen")

const PAD_RZ: [&str; 2] = ["DDJ-RZ", "DDJ-RZX"];
const PAD_RX: [&str; 18] = [
    "DDJ-RX", "DDJ_RX 2IN2OUT", "DDJ-RR", "XDJ-XZ (1)", "XDJ-AZ", "Pioneer DJ XDJ-RX2", "XDJ-RX3", "OPUS-QUAD",
    "DDJ-XP1", "DDJ-XP2", "DDJ-1000", "DDJ-800", "DDJ-FLX10", "DDJ-REV5", "DDJ-REV7", "DJM-S11", "DJM-S7",
    "PLX-CRSS12",
];
const PAD_SX2: [&str; 3] = ["PIONEER DDJ-SX2", "PIONEER DDJ-SX2 2IN2OUT", "Pioneer DJ DJM-S9"];

/// Internal type code for a `Type` text on a given device; unknown text → 0 (Button).
pub fn control_type_code(type_text: &str, device: &str) -> u8 {
    match type_text {
        "Button" => 0,
        "Toggle" => 0x1b,
        "Slider" => 4,
        "Knob" => 5,
        "KnobSliderHiRes" => {
            if device == "PIONEER DDJ-SX2" || device == "PIONEER DDJ-SX2 2IN2OUT" {
                8
            } else {
                7
            }
        }
        "Value" => 0x0f,
        "RawValue" => 0x10,
        "KnobSlider" => 6,
        "Difference" => 0x0c,
        "Rotary" => 0x0d,
        "RotaryAsKnob" => 0x0e,
        "JogRotate" => 0x17,
        "JogRotateAbsPos" => 0x18,
        "JogTimestamp" => JOG_TIMESTAMP,
        "JogTouch" => 0x1a,
        "Indicator" => 0x11,
        "IndicatorHiRes" => 0x12,
        "JogIndicator" => 0x13,
        "JogIndicatorHiRes" => 0x14,
        "Parameter" => 0x1c,
        "Pad" => {
            if PAD_RZ.contains(&device) {
                0x1d
            } else if PAD_RX.contains(&device) {
                0x1e
            } else if device == "DDJ-SZ" {
                0x1f
            } else if PAD_SX2.contains(&device) {
                0x20
            } else {
                0x21
            }
        }
        "DdjRzPad" => 0x1d,
        "DdjRxPad" => 0x1e,
        "DdjSzPad" => 0x1f,
        "DdjSx2Pad" => 0x20,
        "DdjSxPad" => 0x21,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------------------------

/// Parsed `Option` column: all `key=value` parts in order (value `"1"` when no `=` was given).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    pub parts: Vec<(String, String)>,
}

impl Options {
    pub fn parse(s: &str) -> Options {
        let parts = s
            .split(';')
            .filter(|p| !p.is_empty())
            .map(|p| match p.split_once('=') {
                Some((k, v)) if !v.is_empty() => (k.to_string(), v.to_string()),
                Some((k, _)) => (k.to_string(), "1".to_string()),
                None => (p.to_string(), "1".to_string()),
            })
            .collect();
        Options { parts }
    }

    /// Value of a recognised key (last occurrence wins); unknown keys are ignored by the loader.
    pub fn get(&self, key: &str) -> Option<&str> {
        if !OPTION_KEYS.contains(&key) {
            return None;
        }
        self.parts.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn int(&self, key: &str) -> i64 {
        self.get(key).map(int_value).unwrap_or(0)
    }

    fn opt(&self, key: &str) -> Option<i64> {
        self.get(key).filter(|v| !v.is_empty()).map(int_value)
    }

    pub fn read_only(&self) -> bool {
        self.int("RO") != 0
    }
    pub fn fast(&self) -> bool {
        self.int("Fast") != 0
    }
    pub fn dual(&self) -> bool {
        self.int("Dual") != 0
    }
    pub fn blink(&self) -> i64 {
        self.int("Blink")
    }
    pub fn priority(&self) -> i64 {
        self.int("Priority")
    }
    /// `Value`, or `None` (the loader uses −1) when absent.
    pub fn value(&self) -> Option<i64> {
        self.opt("Value")
    }
    pub fn min(&self) -> Option<i64> {
        self.opt("Min")
    }
    pub fn max(&self) -> Option<i64> {
        self.opt("Max")
    }

    /// Text form for writing (`key` alone when the value is `"1"` and the key is a flag).
    pub fn to_text(&self) -> String {
        self.parts
            .iter()
            .map(|(k, v)| {
                if v == "1" && matches!(k.as_str(), "RO" | "Fast" | "Dual") {
                    k.clone()
                } else {
                    format!("{k}={v}")
                }
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// MIDI message assigned to one deck (deck 0 = global).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub deck: u8,
    pub message: u16,
    /// Message of the partner deck when `Dual` is set (1↔3, 2↔4), otherwise equal to `message`.
    pub partner_message: u16,
}

/// One data row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MappingEntry {
    /// 1-based line number in the source text.
    pub line: usize,
    pub name: String,
    pub function: String,
    pub type_text: String,
    pub input: String,
    pub input_decks: [String; 4],
    pub output: String,
    pub output_decks: [String; 4],
    pub options: Options,
    /// Raw option text as stored.
    pub option_text: String,
    pub comment: String,
}

impl MappingEntry {
    pub fn control_code(&self, device: &str) -> u8 {
        control_type_code(&self.type_text, device)
    }

    pub fn input_value(&self) -> u16 {
        hex_value(&self.input) as u16
    }

    pub fn output_value(&self) -> u16 {
        hex_value(&self.output) as u16
    }

    fn bindings(&self, base: u16, decks: &[String; 4], no_shift: bool, missing: u16) -> Vec<Binding> {
        let msgs: [Option<u16>; 4] = std::array::from_fn(|i| {
            let raw = &decks[i];
            if raw.is_empty() {
                None
            } else if base == 0 {
                Some(hex_value(raw) as u16)
            } else {
                let k = int_value(raw) as u16;
                Some(if no_shift { k | base } else { (k << 8) | base })
            }
        });
        let dual = self.options.dual();
        let mut out = Vec::new();
        for i in 0..4 {
            if let Some(m) = msgs[i] {
                let partner = if dual { msgs[(i + 2) % 4].unwrap_or(missing) } else { m };
                out.push(Binding { deck: i as u8 + 1, message: m, partner_message: partner });
            }
        }
        if out.is_empty() && base != 0 {
            out.push(Binding { deck: 0, message: base, partner_message: base });
        }
        out
    }

    /// Input messages per deck (§5f "Nachrichten je Deck").
    pub fn input_bindings(&self, device: &str) -> Vec<Binding> {
        let no_shift = self.control_code(device) == JOG_TIMESTAMP;
        self.bindings(self.input_value(), &self.input_decks, no_shift, 0)
    }

    /// Output (LED/indicator) messages per deck; same rule without the JogTimestamp exception.
    pub fn output_bindings(&self) -> Vec<Binding> {
        self.bindings(self.output_value(), &self.output_decks, false, 0xFFFF)
    }

    fn fields(&self) -> Vec<&str> {
        let mut f = vec![self.name.as_str(), &self.function, &self.type_text, &self.input];
        f.extend(self.input_decks.iter().map(String::as_str));
        f.push(&self.output);
        f.extend(self.output_decks.iter().map(String::as_str));
        f.push(&self.option_text);
        f.push(&self.comment);
        f
    }
}

/// Counts of lines the loader skips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseReport {
    pub comment_lines: usize,
    pub empty_lines: usize,
    pub short_rows: usize,
    pub empty_name: usize,
    pub special_rows: usize,
    pub rows_before_version: usize,
    pub had_bom: bool,
}

/// A parsed mapping file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MidiMapping {
    pub version: i64,
    pub device: String,
    pub entries: Vec<MappingEntry>,
    pub report: ParseReport,
}

impl MidiMapping {
    /// The original loader accepts the file only with `@file` version 1.
    pub fn is_valid(&self) -> bool {
        self.version == 1
    }
}

/// Parse a mapping file. A leading UTF-8 BOM is removed (design choice, SPEC U24).
pub fn parse(text: &str) -> MidiMapping {
    let mut m = MidiMapping::default();
    let text = match text.strip_prefix('\u{feff}') {
        Some(t) => {
            m.report.had_bom = true;
            t
        }
        None => text,
    };
    for (i, line) in text.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            m.report.empty_lines += 1;
            continue;
        }
        if line.starts_with('#') {
            m.report.comment_lines += 1;
            continue;
        }
        if line.starts_with("@file") {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() > 1 {
                m.version = int_value(parts[1]);
            }
            if parts.len() > 2 {
                m.device = parts[2].to_string();
            }
            continue;
        }
        if m.version != 1 {
            m.report.rows_before_version += 1;
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 15 {
            m.report.short_rows += 1;
            continue;
        }
        if f[0].is_empty() {
            m.report.empty_name += 1;
            continue;
        }
        if f[0].contains('@') {
            m.report.special_rows += 1;
            continue;
        }
        let s = |k: usize| f[k].to_string();
        m.entries.push(MappingEntry {
            line: i + 1,
            name: s(0),
            function: s(1),
            type_text: s(2),
            input: s(3),
            input_decks: [s(4), s(5), s(6), s(7)],
            output: s(8),
            output_decks: [s(9), s(10), s(11), s(12)],
            options: Options::parse(f[13]),
            option_text: s(13),
            comment: s(14),
        });
    }
    m
}

/// Error when a field cannot be written (the format has no quoting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteError {
    pub line: usize,
    pub field: &'static str,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entry at line {}: field {} contains ',' or a line break", self.line, self.field)
    }
}

impl std::error::Error for WriteError {}

/// Write a mapping file (`@file` line, header line, one row per entry, CRLF line endings).
pub fn write(m: &MidiMapping) -> Result<String, WriteError> {
    let mut out = format!("@file,{},{}\r\n{HEADER_LINE}\r\n", m.version, m.device);
    for e in &m.entries {
        let fields = e.fields();
        for (k, v) in fields.iter().enumerate() {
            if v.contains(',') || v.contains('\n') || v.contains('\r') {
                return Err(WriteError { line: e.line, field: COLUMNS[k] });
            }
        }
        out.push_str(&fields.join(","));
        out.push_str("\r\n");
    }
    Ok(out)
}
