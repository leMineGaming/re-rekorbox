//! `rekordbox.xml` library exchange format (SPEC §5e).
//!
//! * [`Document`] – in-memory model.
//! * [`write::to_string`] – writer following the attribute order and number formats of §5e.
//! * [`read::from_str`] – reader including the tolerance and validation rules of §5e.

pub mod read;
pub mod write;

use crate::anlz::tags::Beat;

pub use read::{from_str, ReadError};
pub use write::{to_string, Element};

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
// Hot cue colour palette (§5e POSITION_MARK reader; table initialised at start-up by
// GH:0x140c9c4d0, looked up by GH:0x1420426d0 for indices 1…64)

/// Hot cue colours, index 1…64 → `HOT_CUE_PALETTE[index - 1]` as (R, G, B).
pub const HOT_CUE_PALETTE: [(u8, u8, u8); 64] = [
    (0x30, 0x5a, 0xff), (0x50, 0x73, 0xff), (0x50, 0x8c, 0xff), (0x50, 0xa0, 0xff),
    (0x50, 0xb4, 0xff), (0x50, 0xb0, 0xf2), (0x50, 0xae, 0xe8), (0x45, 0xac, 0xdb),
    (0x00, 0xe0, 0xff), (0x19, 0xda, 0xf0), (0x32, 0xd2, 0xe6), (0x21, 0xb4, 0xb9),
    (0x20, 0xaa, 0xa0), (0x1f, 0xa3, 0x92), (0x19, 0xa0, 0x8c), (0x14, 0xa5, 0x84),
    (0x14, 0xaa, 0x7d), (0x10, 0xb1, 0x76), (0x30, 0xd2, 0x6e), (0x37, 0xde, 0x5a),
    (0x3c, 0xeb, 0x50), (0x28, 0xe2, 0x14), (0x7d, 0xc1, 0x3d), (0x8c, 0xc8, 0x32),
    (0x9b, 0xd7, 0x23), (0xa5, 0xe1, 0x16), (0xa5, 0xdc, 0x0a), (0xaa, 0xd2, 0x08),
    (0xb4, 0xc8, 0x05), (0xb4, 0xbe, 0x04), (0xba, 0xb4, 0x04), (0xc3, 0xaf, 0x04),
    (0xe1, 0xaa, 0x00), (0xff, 0xa0, 0x00), (0xff, 0x96, 0x00), (0xff, 0x8c, 0x00),
    (0xff, 0x75, 0x00), (0xe0, 0x64, 0x1b), (0xe0, 0x46, 0x1e), (0xe0, 0x30, 0x1e),
    (0xe0, 0x28, 0x23), (0xe6, 0x28, 0x28), (0xff, 0x37, 0x6f), (0xff, 0x2d, 0x6f),
    (0xff, 0x12, 0x7b), (0xf5, 0x1e, 0x8c), (0xeb, 0x2d, 0xa0), (0xe6, 0x37, 0xb4),
    (0xde, 0x44, 0xcf), (0xde, 0x44, 0x8d), (0xe6, 0x30, 0xb4), (0xe6, 0x19, 0xdc),
    (0xe6, 0x00, 0xff), (0xdc, 0x00, 0xff), (0xcc, 0x00, 0xff), (0xb4, 0x32, 0xff),
    (0xb9, 0x3c, 0xff), (0xc5, 0x42, 0xff), (0xaa, 0x5a, 0xff), (0xaa, 0x72, 0xff),
    (0x82, 0x72, 0xff), (0x64, 0x73, 0xff), (0x00, 0x00, 0x00), (0xff, 0xff, 0xff),
];

/// Palette index (1…64) closest to an RGB colour by Euclidean distance; the first index wins ties.
pub fn nearest_hot_cue_colour(rgb: (u8, u8, u8)) -> u8 {
    let mut best = (f64::MAX, 0u8);
    for (i, &(r, g, b)) in HOT_CUE_PALETTE.iter().enumerate() {
        let d = ((rgb.0 as f64 - r as f64).powi(2) + (rgb.1 as f64 - g as f64).powi(2) + (rgb.2 as f64 - b as f64).powi(2))
            .sqrt();
        if d < best.0 {
            best = (d, i as u8 + 1);
        }
    }
    best.1
}

impl PositionMark {
    /// Palette index the importer would assign to this mark's colour (only hot cues carry colours).
    pub fn palette_index(&self) -> Option<u8> {
        self.rgb.map(nearest_hot_cue_colour)
    }
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
/// rekordbox stores paths internally with `/` (its reader converts `\` to `/`, §5e), so `\` is
/// converted first; otherwise the path is appended unchanged (no slash is added or removed).
pub fn encode_location(path: &str) -> String {
    let p = path.replace('\\', "/");
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

/// Decode a `Location` URL like the original reader (§5e): first `%xx` sequences are decoded
/// (hex digits of either case; malformed sequences stay as they are), then the prefix
/// `file://localhost/` is removed (case-insensitive), then `\` is replaced by `/`.
/// Invalid UTF-8 after decoding is replaced lossily.
pub fn decode_location(url: &str) -> String {
    let bytes = url.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).into_owned();
    let s = if s.len() >= LOCATION_PREFIX.len() && s[..LOCATION_PREFIX.len()].eq_ignore_ascii_case(LOCATION_PREFIX) {
        s[LOCATION_PREFIX.len()..].to_string()
    } else {
        s
    };
    s.replace('\\', "/")
}

/// Escape an attribute value like the original writer (§5e): `& " ' < >` become entities,
/// all characters below 0x20 (including `\n`, `\r`, `\t`) are removed, everything else is kept.
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Beat grid import (§5e "TEMPO-Import")

/// Expand `TEMPO` anchors into a complete beat grid like the original importer:
///
/// * an anchor is used if `Bpm > 0`, `Battito > 0` and the numerator of `Metro` (text before `/`) is > 0;
///   it is emitted if it lies after the previous beat and not after `total_time_s`
///   (the very first entry may also lie at the previous position, i.e. 0);
/// * between two anchors, beats are generated every `60 / bpm` seconds with the beat number counting
///   `beat % numerator + 1`, stopping when the next anchor is within half a beat (`30 / bpm`);
/// * after the last anchor, beats continue up to `total_time_s`;
/// * tempo = `(int)(bpm · 100 + 0.5)`, time = `(int)(seconds · 1000 + 0.5)`.
pub fn beat_grid_from_tempos(tempos: &[TempoAnchor], total_time_s: f64) -> Vec<Beat> {
    let mut out: Vec<Beat> = Vec::new();
    let mut prev_t = 0.0f64;
    let mut bpm = 0.0f64;
    let mut beat: i64 = 0;
    let mut num: i64 = 0;
    let emit = |out: &mut Vec<Beat>, beat: i64, bpm: f64, t: f64| {
        out.push(Beat {
            beat: beat as u16,
            tempo_x100: (bpm * 100.0 + 0.5) as i64 as u16,
            time_ms: (t * 1000.0 + 0.5) as i64 as u32,
        })
    };
    for a in tempos {
        let inizio = a.inizio_s;
        if bpm > 0.0 && beat > 0 && num > 0 && prev_t < inizio {
            let spb = 1.0 / bpm;
            loop {
                let t = spb * 60.0 + prev_t;
                beat = beat % num + 1;
                if t <= prev_t || total_time_s < t || inizio <= spb * 30.0 + t {
                    break;
                }
                emit(&mut out, beat, bpm, t);
                prev_t = t;
                if prev_t >= inizio {
                    break;
                }
            }
        }
        bpm = a.bpm;
        beat = a.battito;
        num = a.metro_numerator();
        if bpm > 0.0 && beat > 0 && num > 0 {
            if inizio <= prev_t || total_time_s < inizio {
                if !out.is_empty() || inizio < 0.0 || total_time_s < inizio {
                    continue;
                }
            }
            emit(&mut out, beat, bpm, inizio);
            prev_t = inizio;
        }
    }
    if bpm > 0.0 && beat > 0 && num > 0 && prev_t < total_time_s {
        let step = 60.0 / bpm;
        loop {
            let t = step + prev_t;
            beat = beat % num + 1;
            if t <= prev_t || total_time_s < t {
                break;
            }
            emit(&mut out, beat, bpm, t);
            prev_t = t;
            if prev_t >= total_time_s {
                break;
            }
        }
    }
    out
}

/// A `TEMPO` element as read from XML, before conversion (values exactly as parsed).
#[derive(Debug, Clone, PartialEq)]
pub struct TempoAnchor {
    pub inizio_s: f64,
    pub bpm: f64,
    pub metro: String,
    pub battito: i64,
}

impl TempoAnchor {
    /// Numerator of `Metro` (text before `/`); 0 if missing or not a number.
    pub fn metro_numerator(&self) -> i64 {
        match self.metro.find('/') {
            Some(i) => self.metro[..i].trim().parse().unwrap_or(0),
            None => 0,
        }
    }

    pub fn from_tempo(t: &Tempo) -> Self {
        TempoAnchor {
            inizio_s: t.inizio_ms as f64 / 1000.0,
            bpm: t.bpm_x100 as f64 / 100.0,
            metro: t.metro.clone(),
            battito: t.battito as i64,
        }
    }
}
