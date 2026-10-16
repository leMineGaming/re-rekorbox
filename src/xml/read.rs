//! `rekordbox.xml` reader (SPEC §5e, reader rules from the import code).
//!
//! Tolerances implemented as specified: `,` is accepted as decimal separator; invalid
//! `POSITION_MARK`s are skipped (type ∉ {0,4}, start outside `[0, TotalTime]`, loop end before start
//! or after `TotalTime`, `Num ∉ [−1, 7]`); `Rating` is divided by 51; `Colour` is matched against
//! the eight known values.

use super::{
    decode_location, Document, FileKind, MarkType, Node, PositionMark, Product, Tempo, Track, TRACK_COLOURS,
};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::fmt;

/// Errors while reading a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Malformed XML.
    Xml(String),
    /// No `DJ_PLAYLISTS` root element.
    MissingRoot,
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Xml(m) => write!(f, "XML error: {m}"),
            ReadError::MissingRoot => write!(f, "missing DJ_PLAYLISTS root element"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Counts of elements dropped by the validation rules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadReport {
    pub skipped_marks: usize,
    pub skipped_tempos: usize,
}

/// Parse a document.
pub fn from_str(s: &str) -> Result<Document, ReadError> {
    from_str_with_report(s).map(|(d, _)| d)
}

type Attrs = Vec<(String, String)>;

fn attrs(e: &BytesStart) -> Result<Attrs, ReadError> {
    let mut v = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| ReadError::Xml(e.to_string()))?;
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let val = a.unescape_value().map_err(|e| ReadError::Xml(e.to_string()))?.into_owned();
        v.push((key, val));
    }
    Ok(v)
}

fn get<'a>(a: &'a Attrs, key: &str) -> Option<&'a str> {
    a.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn text(a: &Attrs, key: &str) -> String {
    get(a, key).unwrap_or("").to_string()
}

/// Number with `,` or `.` as decimal separator.
fn num(a: &Attrs, key: &str) -> Option<f64> {
    get(a, key).and_then(|v| v.trim().replace(',', ".").parse::<f64>().ok())
}

fn int(a: &Attrs, key: &str) -> Option<i64> {
    num(a, key).map(|f| f as i64)
}

fn uint(a: &Attrs, key: &str) -> u64 {
    int(a, key).unwrap_or(0).max(0) as u64
}

/// Seconds → milliseconds, rounded (inference: `s · 1000 + 0.5`, truncated).
fn ms(seconds: f64) -> u32 {
    (seconds * 1000.0 + 0.5).max(0.0) as u32
}

fn parse_track(a: &Attrs) -> Track {
    let location = get(a, "Location").map(decode_location).unwrap_or_default();
    let colour = get(a, "Colour")
        .and_then(|c| TRACK_COLOURS.iter().position(|k| k.eq_ignore_ascii_case(c.trim())))
        .map(|i| i as u8 + 1);
    Track {
        track_id: uint(a, "TrackID") as u32,
        name: text(a, "Name"),
        artist: text(a, "Artist"),
        composer: text(a, "Composer"),
        album: text(a, "Album"),
        grouping: text(a, "Grouping"),
        genre: text(a, "Genre"),
        kind: get(a, "Kind").map(FileKind::from_str_value).unwrap_or_default(),
        size: uint(a, "Size"),
        total_time: uint(a, "TotalTime") as u32,
        disc_number: uint(a, "DiscNumber") as u32,
        track_number: uint(a, "TrackNumber") as u32,
        year: uint(a, "Year") as u32,
        // The original importer truncates: (int)(AverageBpm * 100.0) (§5e).
        bpm_x100: num(a, "AverageBpm").map(|b| (b * 100.0).max(0.0) as u32).unwrap_or(0),
        date_added: text(a, "DateAdded"),
        bit_rate: uint(a, "BitRate") as u32,
        sample_rate: uint(a, "SampleRate") as u32,
        comments: text(a, "Comments"),
        play_count: uint(a, "PlayCount") as u32,
        rating: (uint(a, "Rating") / 51).min(5) as u8,
        location,
        remixer: text(a, "Remixer"),
        tonality: text(a, "Tonality"),
        label: text(a, "Label"),
        mix: text(a, "Mix"),
        colour,
        tempos: Vec::new(),
        marks: Vec::new(),
    }
}

/// `TEMPO` → [`Tempo`] with the importer's rounding: `(int)(bpm·100 + 0.5)`, `(int)(s·1000 + 0.5)`.
fn parse_tempo(a: &Attrs) -> Option<Tempo> {
    Some(Tempo {
        inizio_ms: ms(num(a, "Inizio")?),
        bpm_x100: (num(a, "Bpm")? * 100.0 + 0.5).max(0.0) as u32,
        metro: get(a, "Metro").unwrap_or("4/4").to_string(),
        battito: int(a, "Battito").unwrap_or(0).clamp(0, 255) as u8,
    })
}

/// Validate and convert a `POSITION_MARK` (§5e reader rules). `None` = skipped.
pub fn parse_mark(a: &Attrs, total_time_s: f64) -> Option<PositionMark> {
    let ty = int(a, "Type").unwrap_or(-1);
    let start = num(a, "Start").unwrap_or(0.0);
    let end = num(a, "End").unwrap_or(0.0);
    let n = int(a, "Num").unwrap_or(i32::MAX as i64);
    let mark_type = match ty {
        0 => MarkType::Cue,
        4 => MarkType::Loop,
        _ => return None,
    };
    if start < 0.0 || start > total_time_s {
        return None;
    }
    if mark_type == MarkType::Loop && (end < start || end > total_time_s) {
        return None;
    }
    if !(-1..=7).contains(&n) {
        return None;
    }
    let rgb = if (0..8).contains(&n) {
        match (int(a, "Red"), int(a, "Green"), int(a, "Blue")) {
            (Some(r), Some(g), Some(b)) => Some((r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8)),
            _ => None,
        }
    } else {
        None
    };
    let end_ms = ms(end);
    Some(PositionMark {
        name: text(a, "Name"),
        mark_type,
        start_ms: ms(start),
        end_ms: (end_ms != 0).then_some(end_ms),
        num: n as i32,
        rgb,
    })
}

struct NodeBuilder {
    name: String,
    ty: i64,
    key_type: u32,
    children: Vec<Node>,
    keys: Vec<String>,
}

impl NodeBuilder {
    fn from_attrs(a: &Attrs) -> Self {
        NodeBuilder {
            name: text(a, "Name"),
            ty: int(a, "Type").unwrap_or(0),
            key_type: uint(a, "KeyType") as u32,
            children: Vec::new(),
            keys: Vec::new(),
        }
    }

    fn finish(self) -> Node {
        if self.ty == 1 {
            Node::Playlist { name: self.name, key_type: self.key_type, keys: self.keys }
        } else {
            Node::Folder { name: self.name, children: self.children }
        }
    }
}

/// Parse a document and report how many elements were rejected by the validation rules.
pub fn from_str_with_report(s: &str) -> Result<(Document, ReadReport), ReadError> {
    let mut reader = Reader::from_str(s);
    reader.trim_text(true);
    let mut doc = Document::default();
    let mut report = ReadReport::default();
    let mut seen_root = false;
    let mut stack: Vec<String> = Vec::new();
    let mut track: Option<Track> = None;
    let mut nodes: Vec<NodeBuilder> = Vec::new();
    let mut root_done = false;

    // Handles one start (or empty) element; returns true if a TRACK of the collection was opened.
    loop {
        let ev = reader.read_event().map_err(|e| ReadError::Xml(e.to_string()))?;
        let (e, is_empty) = match &ev {
            Event::Start(e) => (e.clone(), false),
            Event::Empty(e) => (e.clone(), true),
            Event::End(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                stack.pop();
                match name.as_str() {
                    "TRACK" => {
                        if let Some(t) = track.take() {
                            doc.tracks.push(t);
                        }
                    }
                    "NODE" => finish_node(&mut nodes, &mut doc, &mut root_done),
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
        let a = attrs(&e)?;
        let parent = stack.last().map(String::as_str).unwrap_or("");
        match name.as_str() {
            "DJ_PLAYLISTS" => {
                seen_root = true;
                doc.version = text(&a, "Version");
            }
            "PRODUCT" => {
                doc.product = Product { name: text(&a, "Name"), version: text(&a, "Version"), company: text(&a, "Company") };
            }
            "TRACK" if parent == "COLLECTION" => {
                let t = parse_track(&a);
                if is_empty {
                    doc.tracks.push(t);
                } else {
                    track = Some(t);
                }
            }
            "TRACK" if parent == "NODE" => {
                if let Some(n) = nodes.last_mut() {
                    n.keys.push(text(&a, "Key"));
                }
            }
            "TEMPO" if parent == "TRACK" => {
                if let Some(t) = track.as_mut() {
                    match parse_tempo(&a) {
                        Some(tempo) => t.tempos.push(tempo),
                        None => report.skipped_tempos += 1,
                    }
                }
            }
            "POSITION_MARK" if parent == "TRACK" => {
                if let Some(t) = track.as_mut() {
                    match parse_mark(&a, t.total_time as f64) {
                        Some(m) => t.marks.push(m),
                        None => report.skipped_marks += 1,
                    }
                }
            }
            "NODE" if parent == "PLAYLISTS" || parent == "NODE" => {
                nodes.push(NodeBuilder::from_attrs(&a));
                if is_empty {
                    finish_node(&mut nodes, &mut doc, &mut root_done);
                }
            }
            _ => {}
        }
        if !is_empty {
            stack.push(name);
        }
    }
    if !seen_root {
        return Err(ReadError::MissingRoot);
    }
    Ok((doc, report))
}

fn finish_node(nodes: &mut Vec<NodeBuilder>, doc: &mut Document, root_done: &mut bool) {
    let Some(b) = nodes.pop() else { return };
    match nodes.last_mut() {
        Some(parent) => parent.children.push(b.finish()),
        None => {
            // The outermost NODE is ROOT; its children are the playlist tree.
            if !*root_done {
                doc.playlists = b.children;
                *root_done = true;
            }
        }
    }
}
