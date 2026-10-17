//! Building blocks of the embedded DeviceSQL database used for device libraries (SPEC §5h):
//! the string encoding of rows and the container of the small `.edb` files
//! (`datafile.edb`, `ExtData.edb`).
//!
//! `export.pdb` uses the same engine; its page format is not implemented yet (SPEC U23).

use std::fmt;

/// Errors while decoding DeviceSQL data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsqlError {
    TooShort { what: &'static str, needed: usize, available: usize },
    UnknownStringKind(u8),
    BadLength(String),
}

impl fmt::Display for DsqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DsqlError::TooShort { what, needed, available } => write!(f, "{what}: need {needed} bytes, have {available}"),
            DsqlError::UnknownStringKind(k) => write!(f, "unknown DeviceSQL string kind {k:#04x}"),
            DsqlError::BadLength(m) => write!(f, "invalid length: {m}"),
        }
    }
}

impl std::error::Error for DsqlError {}

pub type Result<T> = std::result::Result<T, DsqlError>;

/// Marker placed before UTF-16 text by rekordbox (SPEC §5h).
pub const TEXT_START: u16 = 0xFFFA;
/// Marker placed after UTF-16 text by rekordbox.
pub const TEXT_END: u16 = 0xFFFB;
/// Kind byte of a long UTF-16LE string.
pub const LONG_UTF16: u8 = 0x90;

/// A decoded DeviceSQL string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DsqlString {
    /// Short ASCII string: kind byte `((len + 1) << 1) | 1`, then `len` bytes (len ≤ 126).
    ShortAscii(String),
    /// Long UTF-16LE string: `0x90`, `u16` total length (header included), one pad byte, then the code units.
    /// Code units are kept as stored, including the `0xFFFA`/`0xFFFB` markers if present.
    LongUtf16(Vec<u16>),
}

impl DsqlString {
    /// Decode a string at the start of `b`; returns the string and its encoded length.
    pub fn decode(b: &[u8]) -> Result<(DsqlString, usize)> {
        let kind = *b.first().ok_or(DsqlError::TooShort { what: "string kind", needed: 1, available: 0 })?;
        if kind & 1 == 1 {
            let n = (kind >> 1) as usize;
            if n == 0 {
                return Err(DsqlError::BadLength("short string kind 0x01".into()));
            }
            let len = n - 1;
            if b.len() < 1 + len {
                return Err(DsqlError::TooShort { what: "short string", needed: 1 + len, available: b.len() });
            }
            let s = b[1..1 + len].iter().map(|&c| c as char).collect();
            return Ok((DsqlString::ShortAscii(s), 1 + len));
        }
        if kind == LONG_UTF16 {
            if b.len() < 4 {
                return Err(DsqlError::TooShort { what: "long string header", needed: 4, available: b.len() });
            }
            let total = u16::from_le_bytes([b[1], b[2]]) as usize;
            if total < 4 || (total - 4) % 2 != 0 {
                return Err(DsqlError::BadLength(format!("long string total {total}")));
            }
            if b.len() < total {
                return Err(DsqlError::TooShort { what: "long string", needed: total, available: b.len() });
            }
            let units = b[4..total].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Ok((DsqlString::LongUtf16(units), total));
        }
        Err(DsqlError::UnknownStringKind(kind))
    }

    /// Encode the string.
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            DsqlString::ShortAscii(s) => {
                if !s.chars().all(|c| (c as u32) < 0x100) || s.len() > 126 {
                    return Err(DsqlError::BadLength("short ASCII string must be ≤ 126 single-byte characters".into()));
                }
                let mut v = vec![(((s.len() + 1) << 1) | 1) as u8];
                v.extend(s.chars().map(|c| c as u8));
                Ok(v)
            }
            DsqlString::LongUtf16(units) => {
                let total = 4 + units.len() * 2;
                if total > 0xFFFF {
                    return Err(DsqlError::BadLength("long string too long".into()));
                }
                let mut v = vec![LONG_UTF16];
                v.extend_from_slice(&(total as u16).to_le_bytes());
                v.push(0);
                for u in units {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                Ok(v)
            }
        }
    }

    /// Long string from text, wrapped in the `0xFFFA`…`0xFFFB` markers like rekordbox does.
    pub fn marked_utf16(text: &str) -> DsqlString {
        let mut units = vec![TEXT_START];
        units.extend(text.encode_utf16());
        units.push(TEXT_END);
        DsqlString::LongUtf16(units)
    }

    /// Text content (markers removed; invalid UTF-16 replaced).
    pub fn text(&self) -> String {
        match self {
            DsqlString::ShortAscii(s) => s.clone(),
            DsqlString::LongUtf16(units) => {
                let inner: Vec<u16> = units.iter().copied().filter(|&u| u != TEXT_START && u != TEXT_END).collect();
                String::from_utf16_lossy(&inner)
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// .edb container (SPEC §5h)

/// Size of the fixed `.edb` header before the row offset table.
pub const EDB_TABLE_OFFSET: usize = 0x38;
/// Row offsets are relative to this position.
pub const EDB_ROW_BASE: usize = 0x14;

/// One row of an `.edb` file: raw bytes from its offset up to the next row (or end of file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdbRow {
    pub offset: usize,
    pub data: Vec<u8>,
}

impl EdbRow {
    /// First u16: constant per table (low five bits zero), used here as table key.
    pub fn kind(&self) -> u16 {
        u16::from_le_bytes([self.data[0], self.data[1]])
    }

    /// Second u16: running row number × 0x20.
    pub fn index_shift(&self) -> u16 {
        u16::from_le_bytes([self.data[2], self.data[3]])
    }

    /// All DeviceSQL strings that decode cleanly starting at `from`, scanning forward (diagnostic helper).
    pub fn string_at(&self, at: usize) -> Option<DsqlString> {
        DsqlString::decode(self.data.get(at..)?).ok().map(|(s, _)| s)
    }
}

/// A parsed `.edb` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdbFile {
    /// The first 0x38 bytes as little-endian u32 words (meaning mostly unknown; word 3 = file size).
    pub header: [u32; 14],
    pub rows: Vec<EdbRow>,
}

impl EdbFile {
    /// Parse an `.edb` file: header, zero-terminated u16 offset table at 0x38, rows at `0x14 + offset`.
    pub fn parse(b: &[u8]) -> Result<EdbFile> {
        if b.len() < EDB_TABLE_OFFSET + 2 {
            return Err(DsqlError::TooShort { what: "edb header", needed: EDB_TABLE_OFFSET + 2, available: b.len() });
        }
        let mut header = [0u32; 14];
        for (i, w) in header.iter_mut().enumerate() {
            *w = u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
        if header[3] as usize != b.len() {
            return Err(DsqlError::BadLength(format!("header file size {} != {}", header[3], b.len())));
        }
        let mut offsets = Vec::new();
        let mut i = EDB_TABLE_OFFSET;
        loop {
            if i + 2 > b.len() {
                return Err(DsqlError::BadLength("unterminated row offset table".into()));
            }
            let v = u16::from_le_bytes([b[i], b[i + 1]]) as usize;
            if v == 0 {
                break;
            }
            offsets.push(EDB_ROW_BASE + v);
            i += 2;
        }
        let mut rows = Vec::with_capacity(offsets.len());
        for (k, &o) in offsets.iter().enumerate() {
            let end = offsets.get(k + 1).copied().unwrap_or(b.len());
            if o >= end || end > b.len() || end - o < 4 {
                return Err(DsqlError::BadLength(format!("row {k} at {o:#x}..{end:#x}")));
            }
            rows.push(EdbRow { offset: o, data: b[o..end].to_vec() });
        }
        Ok(EdbFile { header, rows })
    }
}
