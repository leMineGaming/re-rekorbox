//! Decode the XDJ-RX3 deck status packet (UDP 50002, type 0x0A, 292 bytes) for the live mirror.
//!
//! Field offsets and validity rules come from the capture analysis in `docs/link/PRO_DJ_LINK_SPEC.md`
//! (§4.2), verified against the idle-capture fixture. The capture was idle (no track playing), so the
//! *structure* of every field is confirmed but the *dynamics* of the play state, beat and BPM fields
//! are only cross-checked against public documentation (Deep Symmetry); a capture with a running
//! track is still needed to confirm them. Offsets that only mattered while idle are left out.

use rekordbox_formats::djlink::MAGIC;

use crate::error::PacketError;
use crate::packets::kind;

/// Pitch value that means "nominal tempo, ±0 %".
pub const PITCH_NOMINAL: u32 = 0x0010_0000;
/// BPM×100 field value that means "no track loaded".
pub const BPM_NONE: u16 = 0xFFFF;
/// Beat-counter value that means "no beat / not playing".
pub const BEAT_NONE: u32 = 0xFFFF_FFFF;

/// One deck's status as the unit broadcasts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeckStatus {
    /// Device (deck) number, 0x21.
    pub device: u8,
    /// Media slot the loaded track comes from (0x29): 0 none, 1 CD, 2 SD, 3 USB, 4 rekordbox.
    pub slot: u8,
    /// Track type (0x2A): 1 = rekordbox track.
    pub track_type: u8,
    /// rekordbox track id (0x2C); `None` when no track is loaded (slot 0).
    pub track_id: Option<u32>,
    /// Play-state code (0x7B). Dynamics unverified (idle capture); see module note.
    pub play_state: u8,
    /// Status flags (0x89): play/master/sync/on-air bits per public docs.
    pub flags: u8,
    /// Raw pitch/tempo slider value (0x8C); [`PITCH_NOMINAL`] is ±0 %.
    pub pitch_raw: u32,
    /// BPM×100 (0x92); `None` when the field holds [`BPM_NONE`].
    pub bpm_x100: Option<u16>,
    /// Beat counter since the first beat (0xA0); `None` when [`BEAT_NONE`].
    pub beat: Option<u32>,
    /// Beat within the bar, 1–4 (0xA6); 0 when not playing.
    pub beat_in_bar: u8,
    /// Hardware class (0xCC); the XDJ-RX3 reports 0x1F.
    pub hw_class: u8,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

impl DeckStatus {
    /// Last offset the decoder reads (0xCD), so the packet must be at least this long.
    const MIN_LEN: usize = 0xCD;

    /// Decode a type-0x0A packet. Errors if the magic, type or length is wrong.
    pub fn parse(b: &[u8]) -> Result<DeckStatus, PacketError> {
        if b.get(..10) != Some(&MAGIC[..]) {
            return Err(PacketError::NoMagic);
        }
        if b.len() < Self::MIN_LEN {
            return Err(PacketError::TooShort {
                len: b.len(),
                need: Self::MIN_LEN,
            });
        }
        if b[0x0A] != kind::CDJ_STATUS {
            return Err(PacketError::WrongType(b[0x0A]));
        }
        let slot = b[0x29];
        let bpm = u16_at(b, 0x92);
        let beat = u32_at(b, 0xA0);
        Ok(DeckStatus {
            device: b[0x21],
            slot,
            track_type: b[0x2A],
            track_id: (slot != 0).then(|| u32_at(b, 0x2C)),
            play_state: b[0x7B],
            flags: b[0x89],
            pitch_raw: u32_at(b, 0x8C),
            bpm_x100: (bpm != BPM_NONE).then_some(bpm),
            beat: (beat != BEAT_NONE).then_some(beat),
            beat_in_bar: b[0xA6],
            hw_class: b[0xCC],
        })
    }

    /// Pitch as a percentage: 0 at nominal tempo, negative slower, positive faster.
    pub fn pitch_percent(&self) -> f64 {
        (f64::from(self.pitch_raw) - f64::from(PITCH_NOMINAL)) / f64::from(PITCH_NOMINAL) * 100.0
    }

    /// Track BPM at nominal tempo (the written grid value), if a track is loaded.
    pub fn bpm(&self) -> Option<f64> {
        self.bpm_x100.map(|v| f64::from(v) / 100.0)
    }

    /// Actual BPM including the pitch slider, if a track is loaded.
    pub fn effective_bpm(&self) -> Option<f64> {
        self.bpm()
            .map(|b| b * f64::from(self.pitch_raw) / f64::from(PITCH_NOMINAL))
    }

    /// Whether a track is loaded (slot set and a usable BPM present).
    pub fn has_track(&self) -> bool {
        self.slot != 0
    }
}
