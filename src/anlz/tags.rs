//! Typed views of ANLZ tags (SPEC §5b–§5d).
//!
//! Every typed struct keeps the header fields whose meaning is unknown, so that
//! `Tag::from_raw(raw)?.to_raw() == raw` for well-formed input.

use super::phrase::SongStructure;
use super::waveform::{WavePreview, Waveform};
use super::{be_u16, be_u32, be_u64, need, AnlzError, FourCC, RawTag, Result};

/// A decoded ANLZ tag.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    /// `PPTH` – path of the audio file (UTF-16BE, §5b).
    Path(String),
    /// `PQTZ` – beat grid (§5b, semantics §5a).
    BeatGrid(BeatGrid),
    /// `PVBR` – 400-entry u32 table plus one extra u32.
    Vbr(Vbr),
    /// `PWAV` / `PWV2` – fixed-size preview waveforms.
    WavePreview(WavePreview),
    /// `PWV3`…`PWV7` – sized waveform tables.
    Waveform(Waveform),
    /// `PCOB` – legacy cue list with `PCPT` entries.
    CueList(CueList),
    /// `PQT2` – extended beat grid.
    ExtBeatGrid(ExtBeatGrid),
    /// `PVB2` – sample-position table.
    Vbr2(Vbr2),
    /// `PSSI` – song structure / phrases.
    SongStructure(SongStructure),
    /// Any other tag (`PVBR`, `PCO2`, `PWVC`, `PKEY`, `PVDI`, unknown) kept verbatim.
    Raw(RawTag),
}

impl Tag {
    /// Decode a raw tag. Unknown or not-yet-typed tags become [`Tag::Raw`].
    pub fn from_raw(raw: &RawTag) -> Result<Tag> {
        Ok(match raw.fourcc {
            FourCC::PPTH => Tag::Path(decode_path(raw)?),
            FourCC::PQTZ => Tag::BeatGrid(BeatGrid::from_raw(raw)?),
            FourCC::PVBR => Tag::Vbr(Vbr::from_raw(raw)?),
            FourCC::PWAV | FourCC::PWV2 => Tag::WavePreview(WavePreview::from_raw(raw)?),
            FourCC::PWV3 | FourCC::PWV4 | FourCC::PWV5 | FourCC::PWV6 | FourCC::PWV7 => {
                Tag::Waveform(Waveform::from_raw(raw)?)
            }
            FourCC::PCOB => Tag::CueList(CueList::from_raw(raw)?),
            FourCC::PQT2 => Tag::ExtBeatGrid(ExtBeatGrid::from_raw(raw)?),
            FourCC::PVB2 => Tag::Vbr2(Vbr2::from_raw(raw)?),
            FourCC::PSSI => Tag::SongStructure(SongStructure::from_raw(raw)?),
            _ => Tag::Raw(raw.clone()),
        })
    }

    /// Encode back into a raw tag.
    pub fn to_raw(&self) -> RawTag {
        match self {
            Tag::Path(p) => encode_path(p),
            Tag::BeatGrid(b) => b.to_raw(),
            Tag::Vbr(v) => v.to_raw(),
            Tag::WavePreview(w) => w.to_raw(),
            Tag::Waveform(w) => w.to_raw(),
            Tag::CueList(c) => c.to_raw(),
            Tag::ExtBeatGrid(q) => q.to_raw(),
            Tag::Vbr2(v) => v.to_raw(),
            Tag::SongStructure(s) => s.to_raw(),
            Tag::Raw(r) => r.clone(),
        }
    }
}

/// Fail unless the tag has the expected code.
pub(crate) fn expect(raw: &RawTag, fourcc: FourCC) -> Result<()> {
    if raw.fourcc == fourcc {
        Ok(())
    } else {
        Err(AnlzError::BadMagic { expected: fourcc, found: raw.fourcc })
    }
}

// ---------------------------------------------------------------------------------------------
// PPTH

/// Decode a `PPTH` tag: `u16@0E` = byte length of the UTF-16BE path (§5b). Trailing NULs are removed.
pub fn decode_path(raw: &RawTag) -> Result<String> {
    expect(raw, FourCC::PPTH)?;
    let h = raw.full_header();
    need("PPTH header", &h, 16)?;
    let len = be_u16(&h, 0x0E) as usize;
    need("PPTH body", &raw.body, len)?;
    let units: Vec<u16> = raw.body[..len - len % 2].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    let s = String::from_utf16(&units).map_err(|_| AnlzError::BadLength("PPTH: invalid UTF-16".into()))?;
    Ok(s.trim_end_matches('\0').to_string())
}

/// Encode a path as `PPTH`, NUL-terminated like the files written by rekordbox.
pub fn encode_path(path: &str) -> RawTag {
    let mut body = Vec::new();
    for u in path.encode_utf16().chain(std::iter::once(0)) {
        body.extend_from_slice(&u.to_be_bytes());
    }
    let mut header = Vec::with_capacity(4);
    header.extend_from_slice(&0u16.to_be_bytes());
    header.extend_from_slice(&(body.len() as u16).to_be_bytes());
    RawTag::new(FourCC::PPTH, header, body)
}

// ---------------------------------------------------------------------------------------------
// PQTZ

/// One beat grid entry: beat within the bar, tempo × 100, position in milliseconds (§5a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Beat {
    /// Beat number within the bar, 1…4.
    pub beat: u16,
    /// Tempo in BPM × 100 (e.g. 12800 = 128.00 BPM).
    pub tempo_x100: u16,
    /// Position in milliseconds.
    pub time_ms: u32,
}

impl Beat {
    pub fn bpm(&self) -> f64 {
        self.tempo_x100 as f64 / 100.0
    }

    pub(crate) fn read(b: &[u8], off: usize) -> Beat {
        Beat { beat: be_u16(b, off), tempo_x100: be_u16(b, off + 2), time_ms: be_u32(b, off + 4) }
    }

    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.beat.to_be_bytes());
        out.extend_from_slice(&self.tempo_x100.to_be_bytes());
        out.extend_from_slice(&self.time_ms.to_be_bytes());
    }
}

/// `PQTZ` beat grid. Header (24 bytes): `u16@0C`, `u16@0E`, `u16@10` (entry size, default 8),
/// `u16@12`, `u32@14` = number of entries (§5b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeatGrid {
    pub unknown_0c: u16,
    pub unknown_0e: u16,
    pub entry_size: u16,
    pub unknown_12: u16,
    pub beats: Vec<Beat>,
}

impl BeatGrid {
    pub fn new(beats: Vec<Beat>) -> Self {
        BeatGrid { unknown_0c: 0, unknown_0e: 0, entry_size: 8, unknown_12: 0, beats }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PQTZ)?;
        let h = raw.full_header();
        need("PQTZ header", &h, 0x18)?;
        let count = be_u32(&h, 0x14) as usize;
        // The original reader always consumes 8 bytes per entry, independent of u16@10.
        need("PQTZ entries", &raw.body, count * 8)?;
        let beats = (0..count).map(|i| Beat::read(&raw.body, i * 8)).collect();
        Ok(BeatGrid {
            unknown_0c: be_u16(&h, 0x0C),
            unknown_0e: be_u16(&h, 0x0E),
            entry_size: be_u16(&h, 0x10),
            unknown_12: be_u16(&h, 0x12),
            beats,
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut header = Vec::with_capacity(12);
        for v in [self.unknown_0c, self.unknown_0e, self.entry_size, self.unknown_12] {
            header.extend_from_slice(&v.to_be_bytes());
        }
        header.extend_from_slice(&(self.beats.len() as u32).to_be_bytes());
        let mut body = Vec::with_capacity(self.beats.len() * 8);
        for b in &self.beats {
            b.write(&mut body);
        }
        RawTag::new(FourCC::PQTZ, header, body)
    }
}

// ---------------------------------------------------------------------------------------------
// PVBR

/// Number of u32 entries in the `PVBR` table.
pub const PVBR_ENTRIES: usize = 400;

/// `PVBR` (§5b/§5h): header 16 bytes (`u16@0C`, `u16@0E`); payload normally 0x644 bytes =
/// 400 × `be_u32` table + one extra `be_u32`. The payload is kept verbatim; [`Vbr::table`] and
/// [`Vbr::extra`] apply the reader's length rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vbr {
    pub unknown_0c: u16,
    pub unknown_0e: u16,
    pub data: Vec<u8>,
}

impl Vbr {
    pub fn new(table: &[u32; PVBR_ENTRIES], extra: u32) -> Self {
        let mut data = Vec::with_capacity(0x644);
        for v in table {
            data.extend_from_slice(&v.to_be_bytes());
        }
        data.extend_from_slice(&extra.to_be_bytes());
        Vbr { unknown_0c: 0, unknown_0e: 0, data }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PVBR)?;
        let h = raw.full_header();
        need("PVBR header", &h, 0x10)?;
        Ok(Vbr { unknown_0c: be_u16(&h, 0x0C), unknown_0e: be_u16(&h, 0x0E), data: raw.body.clone() })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(4);
        h.extend_from_slice(&self.unknown_0c.to_be_bytes());
        h.extend_from_slice(&self.unknown_0e.to_be_bytes());
        RawTag::new(FourCC::PVBR, h, self.data.clone())
    }

    /// Table as the reader sees it: read when the payload is 0x640 or 0x644 bytes, otherwise zeros.
    pub fn table(&self) -> Vec<u32> {
        let n = self.data.len();
        if n == 0x640 || n == 0x644 {
            (0..PVBR_ENTRIES).map(|i| be_u32(&self.data, i * 4)).collect()
        } else {
            vec![0; PVBR_ENTRIES]
        }
    }

    /// Extra value: last u32 when the payload is 0x644 bytes, the only u32 when it is 4 bytes, else 0.
    pub fn extra(&self) -> u32 {
        match self.data.len() {
            0x644 => be_u32(&self.data, 0x640),
            4 => be_u32(&self.data, 0),
            _ => 0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// PCOB / PCPT

/// Cue list type (`PCOB u16@0E`, §5b): 1 = hot cues, 0 = memory cues (inference from slot logic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueListType {
    Memory,
    HotCue,
}

impl CueListType {
    pub fn code(self) -> u16 {
        match self {
            CueListType::Memory => 0,
            CueListType::HotCue => 1,
        }
    }

    /// Capacity rekordbox allocates for a list of this type (§5d): 3 hot cues, 10 memory cues.
    pub fn default_capacity(self) -> usize {
        match self {
            CueListType::Memory => 10,
            CueListType::HotCue => 3,
        }
    }
}

/// `PCOB` cue list. Header (24 bytes): `u16@0C`, `u16@0E` type, `u16@10`, `u16@12` count,
/// `u32@14` (0xFFFFFFFF when written by rekordbox). Body: `count` × `PCPT` (§5b, §5d).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueList {
    pub unknown_0c: u16,
    pub list_type: CueListType,
    pub unknown_10: u16,
    pub unknown_14: u32,
    pub cues: Vec<CuePoint>,
}

impl CueList {
    pub fn new(list_type: CueListType) -> Self {
        CueList { unknown_0c: 0, list_type, unknown_10: 0, unknown_14: 0xFFFF_FFFF, cues: Vec::new() }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PCOB)?;
        let h = raw.full_header();
        need("PCOB header", &h, 0x18)?;
        let list_type = match be_u16(&h, 0x0E) {
            0 => CueListType::Memory,
            1 => CueListType::HotCue,
            v => return Err(AnlzError::UnexpectedType { what: "PCOB list type", value: v as u32 }),
        };
        let count = be_u16(&h, 0x12) as usize;
        let mut cues = Vec::with_capacity(count);
        let mut pos = 0;
        for _ in 0..count {
            let (cue, len) = CuePoint::parse(&raw.body[pos..])?;
            cues.push(cue);
            pos += len;
        }
        Ok(CueList {
            unknown_0c: be_u16(&h, 0x0C),
            list_type,
            unknown_10: be_u16(&h, 0x10),
            unknown_14: be_u32(&h, 0x14),
            cues,
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut header = Vec::with_capacity(12);
        header.extend_from_slice(&self.unknown_0c.to_be_bytes());
        header.extend_from_slice(&self.list_type.code().to_be_bytes());
        header.extend_from_slice(&self.unknown_10.to_be_bytes());
        header.extend_from_slice(&(self.cues.len() as u16).to_be_bytes());
        header.extend_from_slice(&self.unknown_14.to_be_bytes());
        let mut body = Vec::new();
        for c in &self.cues {
            c.write(&mut body);
        }
        RawTag::new(FourCC::PCOB, header, body)
    }
}

/// Minimum size of a `PCPT` entry (§5b).
pub const PCPT_LEN: usize = 0x38;

/// A `PCPT` cue entry. Field meanings beyond `slot` are unknown (SPEC U15); all fields are kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CuePoint {
    pub header_len: u32,
    pub unknown_0c: u16,
    /// Hot cue slot: 1…3 go to fixed slots, other values to the secondary list (§5b).
    pub slot: u16,
    pub unknown_10: u32,
    pub unknown_14: [u16; 4],
    pub unknown_1c: u8,
    pub unknown_1d: u8,
    pub unknown_1e: u16,
    /// `u32@20…@34` (inference: positions in ms).
    pub unknown_20: [u32; 6],
    /// Bytes beyond offset 0x38, if any.
    pub tail: Vec<u8>,
}

impl Default for CuePoint {
    fn default() -> Self {
        CuePoint {
            header_len: 0x1C,
            unknown_0c: 0,
            slot: 0,
            unknown_10: 0,
            unknown_14: [0; 4],
            unknown_1c: 0,
            unknown_1d: 0,
            unknown_1e: 0,
            unknown_20: [0; 6],
            tail: Vec::new(),
        }
    }
}

impl CuePoint {
    /// Parse one entry at the start of `b`; returns the entry and its total length.
    pub fn parse(b: &[u8]) -> Result<(CuePoint, usize)> {
        need("PCPT header", b, 0x1C)?;
        let magic = FourCC([b[0], b[1], b[2], b[3]]);
        if magic != FourCC::PCPT {
            return Err(AnlzError::BadMagic { expected: FourCC::PCPT, found: magic });
        }
        let hl = be_u32(b, 4);
        let tl = be_u32(b, 8) as usize;
        if tl < PCPT_LEN || (hl as usize) < 0x1C || hl as usize > tl {
            return Err(AnlzError::BadLength(format!("PCPT header_len {hl}, total_len {tl}")));
        }
        need("PCPT entry", b, tl)?;
        let mut unknown_14 = [0u16; 4];
        for (i, v) in unknown_14.iter_mut().enumerate() {
            *v = be_u16(b, 0x14 + 2 * i);
        }
        let mut unknown_20 = [0u32; 6];
        for (i, v) in unknown_20.iter_mut().enumerate() {
            *v = be_u32(b, 0x20 + 4 * i);
        }
        Ok((
            CuePoint {
                header_len: hl,
                unknown_0c: be_u16(b, 0x0C),
                slot: be_u16(b, 0x0E),
                unknown_10: be_u32(b, 0x10),
                unknown_14,
                unknown_1c: b[0x1C],
                unknown_1d: b[0x1D],
                unknown_1e: be_u16(b, 0x1E),
                unknown_20,
                tail: b[PCPT_LEN..tl].to_vec(),
            },
            tl,
        ))
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        let tl = PCPT_LEN + self.tail.len();
        out.extend_from_slice(&FourCC::PCPT.0);
        out.extend_from_slice(&self.header_len.to_be_bytes());
        out.extend_from_slice(&(tl as u32).to_be_bytes());
        out.extend_from_slice(&self.unknown_0c.to_be_bytes());
        out.extend_from_slice(&self.slot.to_be_bytes());
        out.extend_from_slice(&self.unknown_10.to_be_bytes());
        for v in self.unknown_14 {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.push(self.unknown_1c);
        out.push(self.unknown_1d);
        out.extend_from_slice(&self.unknown_1e.to_be_bytes());
        for v in self.unknown_20 {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&self.tail);
    }
}

// ---------------------------------------------------------------------------------------------
// PQT2

/// `PQT2` extended beat grid (§5c). Header length 0x38 (or 0x30 without the last two words).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtBeatGrid {
    pub unknown_0e: u16,
    /// `u8@10`, observed 1 or 2.
    pub version: u8,
    pub unknown_11: u8,
    /// `u16@12` – size of one payload entry (observed 2).
    pub entry_size: u16,
    pub unknown_14: u16,
    pub unknown_16: u16,
    /// `@18..@1F` – inference: first beat of the grid.
    pub first_beat: Beat,
    /// `@20..@27` – inference: last beat of the grid.
    pub last_beat: Beat,
    pub unknown_2c: u32,
    /// `u32@30`, `u32@34` – present only when header_len is 0x38.
    pub unknown_30: Option<(u32, u32)>,
    /// `entry_size × count` payload bytes (meaning unknown, SPEC U19).
    pub data: Vec<u8>,
}

impl ExtBeatGrid {
    pub fn count(&self) -> u32 {
        if self.entry_size == 0 {
            0
        } else {
            (self.data.len() / self.entry_size as usize) as u32
        }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PQT2)?;
        let h = raw.full_header();
        let hl = h.len();
        if hl != 0x38 && hl != 0x30 {
            return Err(AnlzError::BadLength(format!("PQT2 header_len {hl:#x}")));
        }
        if be_u16(&h, 0x0C) != 0 {
            return Err(AnlzError::BadLength("PQT2 u16@0C must be 0".into()));
        }
        let entry_size = be_u16(&h, 0x12);
        let count = be_u32(&h, 0x28) as usize;
        let len = entry_size as usize * count;
        need("PQT2 payload", &raw.body, len)?;
        Ok(ExtBeatGrid {
            unknown_0e: be_u16(&h, 0x0E),
            version: h[0x10],
            unknown_11: h[0x11],
            entry_size,
            unknown_14: be_u16(&h, 0x14),
            unknown_16: be_u16(&h, 0x16),
            first_beat: Beat::read(&h, 0x18),
            last_beat: Beat::read(&h, 0x20),
            unknown_2c: be_u32(&h, 0x2C),
            unknown_30: if hl == 0x38 { Some((be_u32(&h, 0x30), be_u32(&h, 0x34))) } else { None },
            data: raw.body[..len].to_vec(),
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(0x2C);
        h.extend_from_slice(&0u16.to_be_bytes());
        h.extend_from_slice(&self.unknown_0e.to_be_bytes());
        h.push(self.version);
        h.push(self.unknown_11);
        h.extend_from_slice(&self.entry_size.to_be_bytes());
        h.extend_from_slice(&self.unknown_14.to_be_bytes());
        h.extend_from_slice(&self.unknown_16.to_be_bytes());
        self.first_beat.write(&mut h);
        self.last_beat.write(&mut h);
        h.extend_from_slice(&self.count().to_be_bytes());
        h.extend_from_slice(&self.unknown_2c.to_be_bytes());
        if let Some((a, b)) = self.unknown_30 {
            h.extend_from_slice(&a.to_be_bytes());
            h.extend_from_slice(&b.to_be_bytes());
        }
        RawTag::new(FourCC::PQT2, h, self.data.clone())
    }
}

// ---------------------------------------------------------------------------------------------
// PVB2

/// `PVB2` table (§5c): `u64@10` (inference: total sample count), `u32@18` count, `u32@1C` entry size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vbr2 {
    pub unknown_0e: u16,
    pub sample_count: u64,
    pub entry_size: u32,
    pub data: Vec<u8>,
}

impl Vbr2 {
    pub fn count(&self) -> u32 {
        if self.entry_size == 0 {
            0
        } else {
            (self.data.len() / self.entry_size as usize) as u32
        }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PVB2)?;
        let h = raw.full_header();
        if h.len() != 0x20 {
            return Err(AnlzError::BadLength(format!("PVB2 header_len {:#x}", h.len())));
        }
        if be_u16(&h, 0x0C) != 0 {
            return Err(AnlzError::BadLength("PVB2 u16@0C must be 0".into()));
        }
        let count = be_u32(&h, 0x18) as usize;
        let entry_size = be_u32(&h, 0x1C);
        let len = count * entry_size as usize;
        need("PVB2 payload", &raw.body, len)?;
        Ok(Vbr2 {
            unknown_0e: be_u16(&h, 0x0E),
            sample_count: be_u64(&h, 0x10),
            entry_size,
            data: raw.body[..len].to_vec(),
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(0x14);
        h.extend_from_slice(&0u16.to_be_bytes());
        h.extend_from_slice(&self.unknown_0e.to_be_bytes());
        h.extend_from_slice(&self.sample_count.to_be_bytes());
        h.extend_from_slice(&self.count().to_be_bytes());
        h.extend_from_slice(&self.entry_size.to_be_bytes());
        RawTag::new(FourCC::PVB2, h, self.data.clone())
    }
}
