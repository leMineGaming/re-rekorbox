//! Waveform tags (SPEC §5b, §5c, bit layouts §5d).

use super::{be_u16, be_u32, need, AnlzError, FourCC, RawTag, Result};

/// One byte of a monochrome waveform (`PWAV`, `PWV3`, also `PWV2`):
/// bits 7–5 = `value` (0…7), bits 4–0 = `height` (0…31) (§5d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonoColumn {
    pub value: u8,
    pub height: u8,
}

impl MonoColumn {
    pub fn from_byte(b: u8) -> Self {
        MonoColumn { value: b >> 5, height: b & 0x1F }
    }

    pub fn to_byte(self) -> u8 {
        ((self.value & 7) << 5) | (self.height & 0x1F)
    }
}

/// One entry of the colour detail waveform `PWV5` (big-endian u16, §5d):
/// bits 15–13 `c0`, 12–10 `c1`, 9–7 `c2` (3 bits each), 6–2 `height` (5 bits), 1–0 always 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorColumn {
    pub c0: u8,
    pub c1: u8,
    pub c2: u8,
    pub height: u8,
}

impl ColorColumn {
    pub fn from_u16(v: u16) -> Self {
        ColorColumn {
            c0: (v >> 13) as u8 & 7,
            c1: (v >> 10) as u8 & 7,
            c2: (v >> 7) as u8 & 7,
            height: (v >> 2) as u8 & 0x1F,
        }
    }

    pub fn to_u16(self) -> u16 {
        ((self.c0 as u16 & 7) << 13)
            | ((self.c1 as u16 & 7) << 10)
            | ((self.c2 as u16 & 7) << 7)
            | ((self.height as u16 & 0x1F) << 2)
    }
}

// ---------------------------------------------------------------------------------------------

/// `PWAV` (400 bytes) or `PWV2` (100 bytes) preview waveform.
/// Header (20 bytes): `u16@0C`, `u16@0E` count, `u16@10` (default 1), `u16@12` (§5b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavePreview {
    pub kind: FourCC,
    pub unknown_0c: u16,
    pub count_field: u16,
    pub unknown_10: u16,
    pub unknown_12: u16,
    /// Payload exactly as stored.
    pub data: Vec<u8>,
}

impl WavePreview {
    /// Payload size the original reader accepts: 400 for `PWAV`, 100 for `PWV2`.
    pub fn expected_len(kind: FourCC) -> usize {
        if kind == FourCC::PWV2 {
            100
        } else {
            400
        }
    }

    /// Build a preview with rekordbox's default header fields.
    pub fn new(kind: FourCC, data: Vec<u8>) -> Self {
        WavePreview { kind, unknown_0c: 0, count_field: data.len() as u16, unknown_10: 1, unknown_12: 0, data }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        if raw.fourcc != FourCC::PWAV && raw.fourcc != FourCC::PWV2 {
            return Err(AnlzError::BadMagic { expected: FourCC::PWAV, found: raw.fourcc });
        }
        let h = raw.full_header();
        need("PWAV/PWV2 header", &h, 0x14)?;
        Ok(WavePreview {
            kind: raw.fourcc,
            unknown_0c: be_u16(&h, 0x0C),
            count_field: be_u16(&h, 0x0E),
            unknown_10: be_u16(&h, 0x10),
            unknown_12: be_u16(&h, 0x12),
            data: raw.body.clone(),
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(8);
        for v in [self.unknown_0c, self.count_field, self.unknown_10, self.unknown_12] {
            h.extend_from_slice(&v.to_be_bytes());
        }
        RawTag::new(self.kind, h, self.data.clone())
    }

    /// The payload as the original reader sees it: used only if it has exactly the expected
    /// length, otherwise replaced by zeros (§5b "tolerantes Lesen").
    pub fn normalized(&self) -> Vec<u8> {
        let n = Self::expected_len(self.kind);
        if self.data.len() == n {
            self.data.clone()
        } else {
            vec![0; n]
        }
    }

    pub fn columns(&self) -> Vec<MonoColumn> {
        self.normalized().into_iter().map(MonoColumn::from_byte).collect()
    }
}

// ---------------------------------------------------------------------------------------------

/// Sized waveform tables `PWV3`…`PWV7` (§5c).
///
/// | tag  | header | entry | notes |
/// |------|--------|-------|-------|
/// | PWV3 | 0x18   | 1 B   | mono detail, `u32@10` = byte count, `u16@14` = 150 entries/s |
/// | PWV4 | 0x18   | 6 B   | colour preview (1200 entries) |
/// | PWV5 | 0x18   | 2 B   | colour detail, `u16@14` = 150, `u8@16`/`u8@17` = 3/5 |
/// | PWV6 | 0x14   | 3 B   | 3-band preview (1200 entries) |
/// | PWV7 | 0x18   | 3 B   | 3-band detail, `u16@14` = 150 |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waveform {
    pub kind: FourCC,
    /// `u16@0E` – bytes per entry.
    pub entry_size: u16,
    /// Bytes `0x14..0x18` for tags with a 0x18-byte header.
    pub extra: Option<[u8; 4]>,
    pub data: Vec<u8>,
}

impl Waveform {
    /// Header length the original reader requires for this tag.
    pub fn header_len_for(kind: FourCC) -> usize {
        if kind == FourCC::PWV6 {
            0x14
        } else {
            0x18
        }
    }

    /// Number of entries: for `PWV3` the stored count is a byte count.
    pub fn count(&self) -> u32 {
        if self.kind == FourCC::PWV3 || self.entry_size == 0 {
            self.data.len() as u32
        } else {
            (self.data.len() / self.entry_size as usize) as u32
        }
    }

    /// Entries per second from `u16@14` (150 in all observed detail waveforms); `None` if the tag
    /// has no such field or it is zero (as in `PWV4`).
    pub fn entries_per_second(&self) -> Option<u16> {
        self.extra.map(|e| u16::from_be_bytes([e[0], e[1]])).filter(|&v| v != 0)
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        let kind = raw.fourcc;
        if ![FourCC::PWV3, FourCC::PWV4, FourCC::PWV5, FourCC::PWV6, FourCC::PWV7].contains(&kind) {
            return Err(AnlzError::BadMagic { expected: FourCC::PWV3, found: kind });
        }
        let h = raw.full_header();
        let want = Self::header_len_for(kind);
        if h.len() != want {
            return Err(AnlzError::BadLength(format!("{kind} header_len {:#x}, expected {want:#x}", h.len())));
        }
        if kind != FourCC::PWV3 && be_u16(&h, 0x0C) != 0 {
            return Err(AnlzError::BadLength(format!("{kind} u16@0C must be 0")));
        }
        let entry_size = be_u16(&h, 0x0E);
        let count = be_u32(&h, 0x10) as usize;
        let len = if kind == FourCC::PWV3 { count } else { count * entry_size as usize };
        need("waveform payload", &raw.body, len)?;
        let extra = if want == 0x18 { Some([h[0x14], h[0x15], h[0x16], h[0x17]]) } else { None };
        Ok(Waveform { kind, entry_size, extra, data: raw.body[..len].to_vec() })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(12);
        h.extend_from_slice(&0u16.to_be_bytes());
        h.extend_from_slice(&self.entry_size.to_be_bytes());
        h.extend_from_slice(&self.count().to_be_bytes());
        if Self::header_len_for(self.kind) == 0x18 {
            h.extend_from_slice(&self.extra.unwrap_or([0; 4]));
        }
        RawTag::new(self.kind, h, self.data.clone())
    }

    /// `PWV3` detail waveform from columns, with rekordbox's defaults (entry size 1, 150/s).
    pub fn mono_detail(columns: &[MonoColumn]) -> Self {
        Waveform {
            kind: FourCC::PWV3,
            entry_size: 1,
            extra: Some([0x00, 0x96, 0x00, 0x00]),
            data: columns.iter().map(|c| c.to_byte()).collect(),
        }
    }

    /// `PWV5` colour detail waveform from columns, with rekordbox's defaults (150/s, bytes 3, 5).
    pub fn color_detail(columns: &[ColorColumn]) -> Self {
        let mut data = Vec::with_capacity(columns.len() * 2);
        for c in columns {
            data.extend_from_slice(&c.to_u16().to_be_bytes());
        }
        Waveform { kind: FourCC::PWV5, entry_size: 2, extra: Some([0x00, 0x96, 0x03, 0x05]), data }
    }

    /// Decode `PWV3` entries.
    pub fn mono_columns(&self) -> Option<Vec<MonoColumn>> {
        (self.kind == FourCC::PWV3).then(|| self.data.iter().map(|&b| MonoColumn::from_byte(b)).collect())
    }

    /// Decode `PWV5` entries.
    pub fn color_columns(&self) -> Option<Vec<ColorColumn>> {
        (self.kind == FourCC::PWV5).then(|| {
            self.data.chunks_exact(2).map(|c| ColorColumn::from_u16(u16::from_be_bytes([c[0], c[1]]))).collect()
        })
    }

    /// Raw entries of `entry_size` bytes (useful for `PWV4`, `PWV6`, `PWV7`).
    pub fn entries(&self) -> impl Iterator<Item = &[u8]> {
        let n = if self.kind == FourCC::PWV3 { 1 } else { self.entry_size.max(1) as usize };
        self.data.chunks_exact(n)
    }
}
