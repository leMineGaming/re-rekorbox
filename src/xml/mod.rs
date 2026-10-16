//! `rekordbox.xml` library exchange format (SPEC §5e).
//!
//! * [`Document`] – in-memory model.
//! * [`write::to_string`] – writer following the attribute order and number formats of §5e.
//! * [`read::from_str`] – reader including the tolerance and validation rules of §5e.

pub mod read;
pub mod write;

use crate::anlz::tags::Beat;

pub use read::{from_str, ReadError};
pub use write::to_string;

/// Value of `DJ_PLAYLISTS Version` written by rekordbox (§5e).
pub const FORMAT_VERSION: &str = "1.0.0";
/// Prefix of every `Location` URL (§5e).
pub const LOCATION_PREFIX: &str = "file://localhost/";

/// The eight track colours: index 1…8 → `Colour` attribute value (§5e).
pub const TRACK_COLOURS: [&str; 8] =
    ["0xFF007F", "0xFF0000", "0xFFA500", "0xFFFF00", "0x00FF00", "0x25FDE9", "0x0000FF", "0x660099"];

/// Whole document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub version: String,
    pub product: Product,
    pub tracks: Vec<Track>,
    /// Children of the `ROOT` playlist node.
    pub playlists: Vec<Node>,
}

impl Default for Document {
    fn default() -> Self {
        Document { version: FORMAT_VERSION.into(), product: Product::default(), tracks: Vec::new(), playlists: Vec::new() }
    }
}

/// `PRODUCT` element. `Name`/`Version` are supplied by the caller (§5e); defaults are a design choice.
#[derive(Debug, Clone, PartialEq)]
pub struct Product {
    pub name: String,
    pub version: String,
    pub company: String,
}

impl Default for Product {
    fn default() -> Self {
        Product { name: "rekordbox".into(), version: String::new(), company: "AlphaTheta".into() }
    }
}

/// Audio file format, written as `Kind` (§5e).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FileKind {
    Mp3,
    Aac,
    Mp4,
    M4a,
    Flac,
    Alac,
    Wav,
    Aiff,
    #[default]
    Unknown,
}

impl FileKind {
    /// Internal format code (1…6, 11, 12; 0 for unknown).
    pub fn code(self) -> u8 {
        match self {
            FileKind::Mp3 => 1,
            FileKind::Aac => 2,
            FileKind::Mp4 => 3,
            FileKind::M4a => 4,
            FileKind::Flac => 5,
            FileKind::Alac => 6,
            FileKind::Wav => 11,
            FileKind::Aiff => 12,
            FileKind::Unknown => 0,
        }
    }

    pub fn from_code(c: u8) -> FileKind {
        match c {
            1 => FileKind::Mp3,
            2 => FileKind::Aac,
            3 => FileKind::Mp4,
            4 => FileKind::M4a,
            5 => FileKind::Flac,
            6 => FileKind::Alac,
            11 => FileKind::Wav,
            12 => FileKind::Aiff,
            _ => FileKind::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Mp3 => "MP3 File",
            FileKind::Aac => "AAC File",
            FileKind::Mp4 => "MP4 File",
            FileKind::M4a => "M4A File",
            FileKind::Flac => "FLAC File",
            FileKind::Alac => "ALAC File",
            FileKind::Wav => "WAV File",
            FileKind::Aiff => "AIFF File",
            FileKind::Unknown => "Unknown Format",
        }
    }

    pub fn from_str_value(s: &str) -> FileKind {
        [
            FileKind::Mp3,
            FileKind::Aac,
            FileKind::Mp4,
            FileKind::M4a,
            FileKind::Flac,
            FileKind::Alac,
            FileKind::Wav,
            FileKind::Aiff,
        ]
        .into_iter()
        .find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
        .unwrap_or(FileKind::Unknown)
    }
}

/// One `TRACK` of the collection.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    pub track_id: u32,
    pub name: String,
    pub artist: String,
    pub composer: String,
    pub album: String,
    pub grouping: String,
    pub genre: String,
    pub kind: FileKind,
    pub size: u64,
    /// Seconds (inference, SPEC U20).
    pub total_time: u32,
    pub disc_number: u32,
    pub track_number: u32,
    pub year: u32,
    /// BPM × 100.
    pub bpm_x100: u32,
    /// Copied verbatim (format unknown, SPEC U20).
    pub date_added: String,
    pub bit_rate: u32,
    pub sample_rate: u32,
    pub comments: String,
    pub play_count: u32,
    /// Stars 0…5 (written as stars × 51).
    pub rating: u8,
    /// Local file path; encoded as URL when written.
    pub location: String,
    pub remixer: String,
    pub tonality: String,
    pub label: String,
    pub mix: String,
    /// Colour index 1…8, `None` = no colour.
    pub colour: Option<u8>,
    pub tempos: Vec<Tempo>,
    pub marks: Vec<PositionMark>,
}

/// One `TEMPO` element (beat grid anchor).
#[derive(Debug, Clone, PartialEq)]
pub struct Tempo {
    /// Position in milliseconds (`Inizio` is written in seconds with three decimals).
    pub inizio_ms: u32,
    /// BPM × 100.
    pub bpm_x100: u32,
    /// Time signature, always `"4/4"` when written by rekordbox.
    pub metro: String,
    /// Beat number within the bar, 1…4.
    pub battito: u8,
}

/// `POSITION_MARK` type (§5e): the writer only produces cues (0) and loops (4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkType {
    Cue,
    Loop,
}

impl MarkType {
    pub fn code(self) -> u8 {
        match self {
            MarkType::Cue => 0,
            MarkType::Loop => 4,
        }
    }
}

/// One `POSITION_MARK` element.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionMark {
    pub name: String,
    pub mark_type: MarkType,
    pub start_ms: u32,
    /// Loop end; written only when present and non-zero.
    pub end_ms: Option<u32>,
    /// Hot cue number 0…7 (A–H) or −1 for a memory cue.
    pub num: i32,
    /// Hot cue colour, written only for `num` 0…7.
    pub rgb: Option<(u8, u8, u8)>,
}

/// A node of the playlist tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Folder { name: String, children: Vec<Node> },
    /// `key_type` 0 = keys are `TrackID`s (the only value the writer produces).
    Playlist { name: String, key_type: u32, keys: Vec<String> },
}

// ---------------------------------------------------------------------------------------------
// Cue number mapping (§5e POSITION_MARK)

/// Writer mapping from the internal cue kind to `Num`: 1,2,3 → 0,1,2; 5…9 → 3…7; −1,0,4 → −1.
/// Other kinds are not exported (`None`).
pub fn num_from_cue_kind(kind: i32) -> Option<i32> {
    match kind {
        1..=3 => Some(kind - 1),
        5..=9 => Some(kind - 2),
        -1 | 0 | 4 => Some(-1),
        _ => None,
    }
}

/// Reader mapping from `Num` to the internal cue kind: −1 → 0 (cue) / −1 (loop); 0–2 → Num+1; 3–7 → Num+2.
pub fn cue_kind_from_num(num: i32, mark_type: MarkType) -> Option<i32> {
    match num {
        -1 => Some(if mark_type == MarkType::Loop { -1 } else { 0 }),
        0..=2 => Some(num + 1),
        3..=7 => Some(num + 2),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Beat grid export (§5e TEMPO, setting `bridgeExportBeatGrid`)

/// Value of the `bridgeExportBeatGrid` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatGridExport {
    /// 0 – no `TEMPO` elements.
    None,
    /// 1 – only where the tempo changes or a beat leaves the expected grid.
    Changes,
    /// 2 – one `TEMPO` per beat.
    EveryBeat,
}

/// Build `TEMPO` elements from beat grid entries (same entry format as ANLZ `PQTZ`).
pub fn tempos_from_beat_grid(beats: &[Beat], mode: BeatGridExport) -> Vec<Tempo> {
    let make = |b: &Beat| Tempo {
        inizio_ms: b.time_ms,
        bpm_x100: b.tempo_x100 as u32,
        metro: "4/4".into(),
        battito: b.beat as u8,
    };
    match mode {
        BeatGridExport::None => Vec::new(),
        BeatGridExport::EveryBeat => beats.iter().map(make).collect(),
        BeatGridExport::Changes => {
            let mut out = Vec::new();
            let mut prev_tempo: u16 = 0;
            let mut expected_a = 0.0f64;
            let mut expected_b = 0.0f64;
            for b in beats {
                let t = b.time_ms;
                let hits = |e: f64| t == e as u32 || t == (e + 0.5) as u32;
                let on_grid = hits(expected_a) || hits(expected_b);
                if b.tempo_x100 != prev_tempo || !on_grid {
                    out.push(make(b));
                    prev_tempo = b.tempo_x100;
                    if !hits(expected_a) {
                        expected_a = t as f64;
                    }
                }
                let step = if b.tempo_x100 == 0 { 0.0 } else { 6_000_000.0 / b.tempo_x100 as f64 };
                expected_a += step;
                expected_b = t as f64 + step;
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Location and escaping (§5e)

const URL_SAFE: &[u8] = b"!#$()*+,-./:;=?@_~";

/// Encode a local path as `Location` URL. Bytes that are ASCII alphanumerics or in
/// `!#$()*+,-./:;=?@_~` are kept, all others become `%xx` (lower-case hex), prefix `file://localhost/`.
///
/// Design choices (SPEC U21): `\` is converted to `/` and one leading `/` is removed first.
pub fn encode_location(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.strip_prefix('/').unwrap_or(&p);
    let mut out = String::from(LOCATION_PREFIX);
    for &b in p.as_bytes() {
        if b.is_ascii_alphanumeric() || URL_SAFE.contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
            out.push(char::from_digit((b & 0xF) as u32, 16).unwrap());
        }
    }
    out
}

/// Decode a `Location` URL to a path. Percent escapes of either case are accepted.
/// Returns `None` if the prefix is missing or the bytes are not UTF-8.
/// A path that does not start with a drive letter gets a leading `/` (design choice, SPEC U21).
pub fn decode_location(url: &str) -> Option<String> {
    let rest = url.strip_prefix(LOCATION_PREFIX)?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    let s = String::from_utf8(out).ok()?;
    let has_drive = s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':';
    Some(if has_drive { s } else { format!("/{s}") })
}

/// Escape text for an attribute value: `& " ' < >` become entities, `\n`/`\r` numeric references,
/// other control characters below 0x20 are dropped (SPEC §5e / U22; the newline handling is a design choice).
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}
