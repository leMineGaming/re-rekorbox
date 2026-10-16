//! ANLZ analysis files (`.DAT`, `.EXT`, `.2EX`) – container format, SPEC §5a–§5c.
//!
//! The container is a sequence of big-endian tagged sections:
//!
//! ```text
//! "PMAI" u32 header_len(=28) u32 total_len(=file size) 6×u16 u32   <- file header
//! fourcc u32 header_len u32 total_len <header rest> <body>        <- repeated tags
//! ```
//!
//! [`AnlzFile`] keeps every tag as a [`RawTag`] so that a parse/serialize round trip is
//! byte-exact, including tags this crate does not understand (rekordbox itself preserves
//! unknown tags, §5c). Typed views are provided by [`tags::Tag`].

pub mod phrase;
pub mod tags;
pub mod threex;
pub mod waveform;

use std::fmt;

pub use tags::Tag;

/// Length of the `PMAI` file header in bytes.
pub const PMAI_HEADER_LEN: usize = 28;
/// Length of the common part of every tag header (fourcc, header_len, total_len).
pub const TAG_COMMON_LEN: usize = 12;

/// Errors reported while decoding ANLZ data. The variants mirror the error classes of the
/// original reader (§5b "Fehlercodes"): too few bytes, wrong magic/length, unexpected list type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnlzError {
    /// Fewer bytes available than the structure requires.
    TooShort { what: &'static str, needed: usize, available: usize },
    /// A four-character code did not match.
    BadMagic { expected: FourCC, found: FourCC },
    /// A length or header field violates the format rules.
    BadLength(String),
    /// A list/type selector has a value the format does not allow.
    UnexpectedType { what: &'static str, value: u32 },
}

impl fmt::Display for AnlzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnlzError::TooShort { what, needed, available } => {
                write!(f, "{what}: need {needed} bytes, only {available} available")
            }
            AnlzError::BadMagic { expected, found } => write!(f, "expected tag {expected}, found {found}"),
            AnlzError::BadLength(msg) => write!(f, "invalid length: {msg}"),
            AnlzError::UnexpectedType { what, value } => write!(f, "unexpected {what} value {value}"),
        }
    }
}

impl std::error::Error for AnlzError {}

pub type Result<T> = std::result::Result<T, AnlzError>;

/// A four-character tag code such as `PQTZ`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FourCC(pub [u8; 4]);

impl FourCC {
    pub const PMAI: FourCC = FourCC(*b"PMAI");
    pub const PPTH: FourCC = FourCC(*b"PPTH");
    pub const PVBR: FourCC = FourCC(*b"PVBR");
    pub const PQTZ: FourCC = FourCC(*b"PQTZ");
    pub const PWAV: FourCC = FourCC(*b"PWAV");
    pub const PWV2: FourCC = FourCC(*b"PWV2");
    pub const PWV3: FourCC = FourCC(*b"PWV3");
    pub const PWV4: FourCC = FourCC(*b"PWV4");
    pub const PWV5: FourCC = FourCC(*b"PWV5");
    pub const PWV6: FourCC = FourCC(*b"PWV6");
    pub const PWV7: FourCC = FourCC(*b"PWV7");
    pub const PWVC: FourCC = FourCC(*b"PWVC");
    pub const PCOB: FourCC = FourCC(*b"PCOB");
    pub const PCO2: FourCC = FourCC(*b"PCO2");
    pub const PCPT: FourCC = FourCC(*b"PCPT");
    pub const PCP2: FourCC = FourCC(*b"PCP2");
    pub const PQT2: FourCC = FourCC(*b"PQT2");
    pub const PVB2: FourCC = FourCC(*b"PVB2");
    pub const PSSI: FourCC = FourCC(*b"PSSI");
    pub const PKEY: FourCC = FourCC(*b"PKEY");
    pub const PVDI: FourCC = FourCC(*b"PVDI");
}

impl fmt::Display for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            let c = if b.is_ascii_graphic() { b as char } else { '?' };
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FourCC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FourCC({self})")
    }
}

// ---------------------------------------------------------------------------------------------
// Big-endian helpers (§5b "Lesehilfen")

pub(crate) fn be_u16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([b[off], b[off + 1]])
}

pub(crate) fn be_u32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

pub(crate) fn be_u64(b: &[u8], off: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[off..off + 8]);
    u64::from_be_bytes(a)
}

pub(crate) fn need(what: &'static str, buf: &[u8], needed: usize) -> Result<()> {
    if buf.len() < needed {
        Err(AnlzError::TooShort { what, needed, available: buf.len() })
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------

/// The fields of the `PMAI` header after magic and lengths (§5b "PMAI").
///
/// Meaning unknown (SPEC U8); rekordbox always writes `fields = [0, 1, 1, 0, 1, 0]`, `tail = 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PmaiHeader {
    pub fields: [u16; 6],
    pub tail: u32,
}

impl Default for PmaiHeader {
    fn default() -> Self {
        PmaiHeader { fields: [0, 1, 1, 0, 1, 0], tail: 0 }
    }
}

/// One tagged section, kept as raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTag {
    pub fourcc: FourCC,
    /// Header bytes following the 12-byte common part (`header_len - 12` bytes).
    pub header: Vec<u8>,
    /// Payload (`total_len - header_len` bytes).
    pub body: Vec<u8>,
}

impl RawTag {
    pub fn new(fourcc: FourCC, header: Vec<u8>, body: Vec<u8>) -> Self {
        RawTag { fourcc, header, body }
    }

    pub fn header_len(&self) -> usize {
        TAG_COMMON_LEN + self.header.len()
    }

    pub fn total_len(&self) -> usize {
        self.header_len() + self.body.len()
    }

    /// The complete header including the common 12 bytes, i.e. bytes `0..header_len` of the tag.
    /// Offsets in SPEC tables (e.g. `u16@0E`) index into this buffer.
    pub fn full_header(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.header_len());
        self.write_common(&mut v);
        v.extend_from_slice(&self.header);
        v
    }

    fn write_common(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.fourcc.0);
        out.extend_from_slice(&(self.header_len() as u32).to_be_bytes());
        out.extend_from_slice(&(self.total_len() as u32).to_be_bytes());
    }

    /// Serialise the tag.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        self.write_common(out);
        out.extend_from_slice(&self.header);
        out.extend_from_slice(&self.body);
    }

    /// Decode into a typed [`Tag`].
    pub fn decode(&self) -> Result<Tag> {
        Tag::from_raw(self)
    }

    /// True if decoding into a typed tag and encoding again yields exactly this tag.
    pub fn typed_round_trip_ok(&self) -> Result<bool> {
        Ok(Tag::from_raw(self)?.to_raw() == *self)
    }
}

/// A parsed ANLZ container file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnlzFile {
    pub header: PmaiHeader,
    /// Bytes between offset 28 and `header_len` if a file declares a longer PMAI header.
    pub header_extra: Vec<u8>,
    pub tags: Vec<RawTag>,
    /// Bytes after the last tag that are too short to form a tag header (< 12 bytes);
    /// the original reader ignores them (§5c step 2), they are kept for exact round trips.
    pub trailing: Vec<u8>,
}

impl Default for AnlzFile {
    fn default() -> Self {
        AnlzFile { header: PmaiHeader::default(), header_extra: Vec::new(), tags: Vec::new(), trailing: Vec::new() }
    }
}

impl AnlzFile {
    /// Create an empty file with the default PMAI header.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a complete `.DAT` / `.EXT` / `.2EX` file (§5a container, §5c loop rules).
    pub fn parse(data: &[u8]) -> Result<Self> {
        need("PMAI header", data, PMAI_HEADER_LEN)?;
        let magic = FourCC([data[0], data[1], data[2], data[3]]);
        if magic != FourCC::PMAI {
            return Err(AnlzError::BadMagic { expected: FourCC::PMAI, found: magic });
        }
        let header_len = be_u32(data, 4) as usize;
        let total_len = be_u32(data, 8) as usize;
        if header_len < PMAI_HEADER_LEN || header_len > data.len() {
            return Err(AnlzError::BadLength(format!("PMAI header_len {header_len}")));
        }
        if total_len != data.len() {
            return Err(AnlzError::BadLength(format!(
                "PMAI total_len {total_len} does not match file size {}",
                data.len()
            )));
        }
        let mut fields = [0u16; 6];
        for (i, f) in fields.iter_mut().enumerate() {
            *f = be_u16(data, 12 + 2 * i);
        }
        let header = PmaiHeader { fields, tail: be_u32(data, 24) };
        let header_extra = data[PMAI_HEADER_LEN..header_len].to_vec();

        let mut tags = Vec::new();
        let mut pos = header_len;
        while pos + TAG_COMMON_LEN <= data.len() {
            let fourcc = FourCC([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            let hl = be_u32(data, pos + 4) as usize;
            let tl = be_u32(data, pos + 8) as usize;
            if tl == 0 {
                return Err(AnlzError::BadLength(format!("tag {fourcc} at {pos}: total_len 0")));
            }
            if hl < TAG_COMMON_LEN || hl > tl {
                return Err(AnlzError::BadLength(format!(
                    "tag {fourcc} at {pos}: header_len {hl}, total_len {tl}"
                )));
            }
            if pos + tl > data.len() {
                return Err(AnlzError::BadLength(format!(
                    "tag {fourcc} at {pos}: total_len {tl} exceeds file size {}",
                    data.len()
                )));
            }
            tags.push(RawTag {
                fourcc,
                header: data[pos + TAG_COMMON_LEN..pos + hl].to_vec(),
                body: data[pos + hl..pos + tl].to_vec(),
            });
            pos += tl;
        }
        let trailing = data[pos..].to_vec();
        Ok(AnlzFile { header, header_extra, tags, trailing })
    }

    /// Serialise the file. Lengths in all headers are recomputed.
    pub fn to_bytes(&self) -> Vec<u8> {
        let header_len = PMAI_HEADER_LEN + self.header_extra.len();
        let total = header_len
            + self.tags.iter().map(RawTag::total_len).sum::<usize>()
            + self.trailing.len();
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&FourCC::PMAI.0);
        out.extend_from_slice(&(header_len as u32).to_be_bytes());
        out.extend_from_slice(&(total as u32).to_be_bytes());
        for f in self.header.fields {
            out.extend_from_slice(&f.to_be_bytes());
        }
        out.extend_from_slice(&self.header.tail.to_be_bytes());
        out.extend_from_slice(&self.header_extra);
        for t in &self.tags {
            t.write_to(&mut out);
        }
        out.extend_from_slice(&self.trailing);
        out
    }

    /// First tag with the given code.
    pub fn find(&self, fourcc: FourCC) -> Option<&RawTag> {
        self.tags.iter().find(|t| t.fourcc == fourcc)
    }

    /// All tags with the given code, in file order.
    pub fn find_all(&self, fourcc: FourCC) -> impl Iterator<Item = &RawTag> {
        self.tags.iter().filter(move |t| t.fourcc == fourcc)
    }

    /// Append a typed tag.
    pub fn push(&mut self, tag: &Tag) {
        self.tags.push(tag.to_raw());
    }

    /// Decode the track path from the `PPTH` tag, if present.
    pub fn path(&self) -> Option<Result<String>> {
        self.find(FourCC::PPTH).map(|t| tags::decode_path(t))
    }

    /// Check the ordering rule of the original `.EXT`/`.2EX` reader: the first two tags must be
    /// `PPTH` (directly after the PMAI header). See §5c step 1.
    pub fn check_reader_order(&self) -> Result<()> {
        match self.tags.first() {
            Some(t) if t.fourcc == FourCC::PPTH => Ok(()),
            Some(t) => Err(AnlzError::BadMagic { expected: FourCC::PPTH, found: t.fourcc }),
            None => Err(AnlzError::TooShort { what: "PPTH tag", needed: TAG_COMMON_LEN, available: 0 }),
        }
    }
}
