//! `PSSI` song structure / phrase analysis (SPEC §5c, semantics §5d).

use super::tags::expect;
use super::{be_u16, need, AnlzError, FourCC, RawTag, Result};

/// Size of one phrase entry in bytes (12 × u16).
pub const PHRASE_LEN: usize = 24;
/// Required `PSSI` header length.
pub const PSSI_HEADER_LEN: usize = 0x20;

/// Track mood (`PSSI u16@12`, §5d). Selects the label table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    High,
    Mid,
    Low,
    Other(u16),
}

impl Mood {
    pub fn from_code(v: u16) -> Mood {
        match v {
            1 => Mood::High,
            2 => Mood::Mid,
            3 => Mood::Low,
            v => Mood::Other(v),
        }
    }

    pub fn code(self) -> u16 {
        match self {
            Mood::High => 1,
            Mood::Mid => 2,
            Mood::Low => 3,
            Mood::Other(v) => v,
        }
    }
}

/// One phrase entry. All twelve u16 values are kept verbatim; names follow SPEC §5d.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Phrase {
    /// `[0]` running number 1…n (not used by the player).
    pub index: u16,
    /// `[1]` start beat of the phrase.
    pub beat: u16,
    /// `[2]` phrase kind (input of the label mapping).
    pub kind: u16,
    /// `[3]` variant flag k1.
    pub k1: u16,
    /// `[4]` variant flag k2.
    pub k2: u16,
    /// `[5]` passed through by the player, meaning unknown.
    pub b: u16,
    /// `[6]`, `[7]`, `[8]` passed through, beat-like values (meaning unknown).
    pub beat2: u16,
    pub beat3: u16,
    pub beat4: u16,
    /// `[9]` variant flag k3.
    pub k3: u16,
    /// `[10]` fill-in present.
    pub fill: u16,
    /// `[11]` first beat of the fill-in.
    pub fill_beat: u16,
}

impl Phrase {
    fn read(b: &[u8], off: usize) -> Phrase {
        let w = |i: usize| be_u16(b, off + 2 * i);
        Phrase {
            index: w(0),
            beat: w(1),
            kind: w(2),
            k1: w(3),
            k2: w(4),
            b: w(5),
            beat2: w(6),
            beat3: w(7),
            beat4: w(8),
            k3: w(9),
            fill: w(10),
            fill_beat: w(11),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        for v in [
            self.index, self.beat, self.kind, self.k1, self.k2, self.b, self.beat2, self.beat3, self.beat4, self.k3,
            self.fill, self.fill_beat,
        ] {
            out.extend_from_slice(&v.to_be_bytes());
        }
    }

    /// Display label as computed by the player (§5d table "Label-Umrechnung im Player").
    pub fn label(&self, mood: Mood) -> Option<&'static str> {
        phrase_label(mood, self.kind, self.k1 != 0, self.k2 != 0, self.k3 != 0)
    }
}

const HIGH: [&str; 10] =
    ["INTRO 1", "INTRO 2", "UP 1", "UP 2", "UP 3", "CHORUS 1", "CHORUS 2", "DOWN", "OUTRO 1", "OUTRO 2"];
const MID: [&str; 10] =
    ["INTRO", "VERSE1", "VERSE2", "VERSE3", "VERSE4", "VERSE5", "VERSE6", "BRIDGE", "CHORUS", "OUTRO"];
const LOW: [&str; 6] = ["INTRO", "VERSE 1", "VERSE 2", "CHORUS", "BRIDGE", "OUTRO"];

/// Label for a phrase `kind` (1-based as stored in the file) and its variant flags.
pub fn phrase_label(mood: Mood, kind: u16, k1: bool, k2: bool, k3: bool) -> Option<&'static str> {
    let k = kind as i32 - 1;
    match mood {
        Mood::High => {
            let idx = match k {
                0 => if k1 { 0 } else { 1 },
                1 => if k2 { 4 } else if k3 { 3 } else { 2 },
                2 => 7,
                3 => 3,
                4 => if k1 { 5 } else { 6 },
                5 => if k1 { 8 } else { 9 },
                _ => return None,
            };
            Some(HIGH[idx])
        }
        Mood::Mid => (0..10).contains(&k).then(|| MID[k as usize]),
        Mood::Low => {
            let idx = match k {
                0 => 0,
                1..=3 => 1,
                4..=6 => 2,
                7 => 4,
                8 => 3,
                9 => 5,
                _ => return None,
            };
            Some(LOW[idx])
        }
        Mood::Other(_) => None,
    }
}

/// `PSSI` tag (§5c header checks, §5d fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongStructure {
    pub mood: Mood,
    /// Bytes `0x14..0x1A` – always zero in observed files, ignored by the reader.
    pub reserved_14: [u8; 6],
    /// `u16@1A` – end beat (larger than the last phrase start).
    pub end_beat: u16,
    /// `u8@1C` – flag (0/1), meaning unknown.
    pub flag_1c: u8,
    /// `u8@1D` – observed always 1, meaning unknown.
    pub flag_1d: u8,
    /// Bytes `0x1E..0x20`.
    pub reserved_1e: [u8; 2],
    pub phrases: Vec<Phrase>,
}

impl SongStructure {
    pub fn new(mood: Mood, end_beat: u16, phrases: Vec<Phrase>) -> Self {
        SongStructure {
            mood,
            reserved_14: [0; 6],
            end_beat,
            flag_1c: 0,
            flag_1d: 1,
            reserved_1e: [0; 2],
            phrases,
        }
    }

    pub fn from_raw(raw: &RawTag) -> Result<Self> {
        expect(raw, FourCC::PSSI)?;
        let h = raw.full_header();
        if h.len() != PSSI_HEADER_LEN {
            return Err(AnlzError::BadLength(format!("PSSI header_len {:#x}", h.len())));
        }
        if be_u16(&h, 0x0C) != 0 || be_u16(&h, 0x0E) as usize != PHRASE_LEN {
            return Err(AnlzError::BadLength("PSSI requires u16@0C = 0 and u16@0E = 0x18".into()));
        }
        let count = be_u16(&h, 0x10) as usize;
        if raw.total_len() != PSSI_HEADER_LEN + PHRASE_LEN * count {
            return Err(AnlzError::BadLength(format!(
                "PSSI total_len {} != 0x20 + 24 × {count}",
                raw.total_len()
            )));
        }
        need("PSSI entries", &raw.body, PHRASE_LEN * count)?;
        let mut reserved_14 = [0u8; 6];
        reserved_14.copy_from_slice(&h[0x14..0x1A]);
        Ok(SongStructure {
            mood: Mood::from_code(be_u16(&h, 0x12)),
            reserved_14,
            end_beat: be_u16(&h, 0x1A),
            flag_1c: h[0x1C],
            flag_1d: h[0x1D],
            reserved_1e: [h[0x1E], h[0x1F]],
            phrases: (0..count).map(|i| Phrase::read(&raw.body, i * PHRASE_LEN)).collect(),
        })
    }

    pub fn to_raw(&self) -> RawTag {
        let mut h = Vec::with_capacity(PSSI_HEADER_LEN - 12);
        h.extend_from_slice(&0u16.to_be_bytes());
        h.extend_from_slice(&(PHRASE_LEN as u16).to_be_bytes());
        h.extend_from_slice(&(self.phrases.len() as u16).to_be_bytes());
        h.extend_from_slice(&self.mood.code().to_be_bytes());
        h.extend_from_slice(&self.reserved_14);
        h.extend_from_slice(&self.end_beat.to_be_bytes());
        h.push(self.flag_1c);
        h.push(self.flag_1d);
        h.extend_from_slice(&self.reserved_1e);
        let mut body = Vec::with_capacity(self.phrases.len() * PHRASE_LEN);
        for p in &self.phrases {
            p.write(&mut body);
        }
        RawTag::new(FourCC::PSSI, h, body)
    }

    /// Start beat of the phrase after `i`, or the end beat for the last phrase.
    fn next_start(&self, i: usize) -> u16 {
        self.phrases.get(i + 1).map(|p| p.beat).unwrap_or(self.end_beat)
    }

    /// Index of the phrase containing `beat` (`start ≤ beat < next start / end beat`).
    pub fn phrase_at(&self, beat: u16) -> Option<usize> {
        (0..self.phrases.len()).find(|&i| self.phrases[i].beat <= beat && beat < self.next_start(i))
    }

    /// Fill-in step for `beat` (§5d): if the containing phrase has a fill-in starting at `fill_beat`
    /// and `beat` lies in `[fill_beat, next phrase)`, returns `beat − fill_beat + 1` (1…d).
    /// Fill-ins longer than four beats yield `None`, like the player.
    pub fn fill_step(&self, beat: u16) -> Option<u8> {
        let i = self.phrase_at(beat)?;
        let p = &self.phrases[i];
        let next = self.next_start(i);
        if p.fill == 0 || beat < p.fill_beat || beat >= next {
            return None;
        }
        let span = next - p.fill_beat;
        (1..=4).contains(&span).then(|| (beat - p.fill_beat + 1) as u8)
    }
}
