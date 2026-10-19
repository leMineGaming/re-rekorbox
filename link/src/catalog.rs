//! The library as the players see it: tracks with ids 1…n (sorted by title), artists and keys with
//! ids, and the analysis data the database server hands out (waveforms, beat grid, ANLZ tags).

use std::collections::BTreeMap;
use std::path::Path;

use rekordbox_formats::anlz::{AnlzFile, FourCC, RawTag};
use rekordbox_formats::library::{LibTrack, Library};

use crate::packets::LibraryInfo;

/// A snapshot of the library.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    tracks: Vec<LibTrack>,
    /// Artist name → id (ids in order of first appearance in title order, from 1).
    artists: BTreeMap<String, u32>,
    keys: BTreeMap<String, u32>,
}

impl Catalog {
    /// Playable tracks of a library (with a matched audio file).
    pub fn from_library(lib: &Library) -> Catalog {
        Catalog::from_tracks(lib.playable().cloned().collect())
    }

    pub fn from_tracks(mut tracks: Vec<LibTrack>) -> Catalog {
        tracks.sort_by_cached_key(|t| t.title.to_lowercase());
        let mut artists = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for t in &tracks {
            let n = artists.len() as u32 + 1;
            artists.entry(artist_name(t).to_string()).or_insert(n);
            if let Some(k) = &t.key {
                let n = keys.len() as u32 + 1;
                keys.entry(k.clone()).or_insert(n);
            }
        }
        Catalog {
            tracks,
            artists,
            keys,
        }
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Counts for the media response (0x06). There are no playlists yet.
    pub fn info(&self) -> LibraryInfo {
        LibraryInfo {
            tracks: self.tracks.len().min(usize::from(u16::MAX)) as u16,
            playlists: 0,
        }
    }

    pub fn track(&self, id: u32) -> Option<&LibTrack> {
        self.tracks.get((id as usize).checked_sub(1)?)
    }

    /// All tracks with their ids, in title order.
    pub fn tracks(&self) -> impl Iterator<Item = (u32, &LibTrack)> {
        self.tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (i as u32 + 1, t))
    }

    pub fn artist_id(&self, t: &LibTrack) -> u32 {
        self.artists.get(artist_name(t)).copied().unwrap_or(0)
    }

    pub fn key_id(&self, t: &LibTrack) -> u32 {
        t.key
            .as_ref()
            .and_then(|k| self.keys.get(k))
            .copied()
            .unwrap_or(0)
    }

    /// Artists sorted by name: (id, name).
    pub fn artists(&self) -> impl Iterator<Item = (u32, &str)> {
        self.artists.iter().map(|(n, id)| (*id, n.as_str()))
    }

    pub fn keys(&self) -> impl Iterator<Item = (u32, &str)> {
        self.keys.iter().map(|(k, id)| (*id, k.as_str()))
    }
}

/// Artist as shown (empty when unknown).
pub fn artist_name(t: &LibTrack) -> &str {
    t.artist.as_deref().unwrap_or("")
}

/// Duration in whole seconds: sample count, else the colour detail waveform (150 columns per second),
/// else the last beat.
pub fn duration_s(t: &LibTrack) -> u32 {
    if let Some(d) = t.duration_s(44100) {
        return d.round() as u32;
    }
    if let Some(w) = anlz_tag(t, FourCC::PWV3, "EXT") {
        return (w.body.len() as f64 / 150.0).round() as u32;
    }
    t.beats.last().map_or(0, |b| b.time_ms / 1000)
}

/// BPM × 100 of the first beat, 0 when unknown.
pub fn bpm_x100(t: &LibTrack) -> u32 {
    t.bpm().map_or(0, |b| (b * 100.0).round() as u32)
}

/// rekordbox's file type code (SONGDATA_TRACK2): 1 MP3, 4 M4A, 5 FLAC, 0x0B WAV, 0x0C AIFF.
pub fn file_type(path: &Path) -> u32 {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp3" => 1,
        "m4a" | "mp4" | "aac" => 4,
        "flac" => 5,
        "wav" => 0x0B,
        "aif" | "aiff" => 0x0C,
        _ => 0,
    }
}

/// A tag from the track's analysis files (`ext` = `DAT`, `EXT` or `2EX`).
pub fn anlz_tag(t: &LibTrack, fourcc: FourCC, ext: &str) -> Option<RawTag> {
    let bytes = std::fs::read(t.anlz_dir.join(format!("ANLZ0000.{ext}"))).ok()?;
    AnlzFile::parse(&bytes)
        .ok()?
        .tags
        .into_iter()
        .find(|x| x.fourcc == fourcc)
}

/// Beat grid blob: 20 bytes header, then 16 bytes per beat (beat in bar, BPM × 100, ms; little-endian).
pub fn beat_grid_blob(t: &LibTrack) -> Vec<u8> {
    let mut b = vec![0u8; 20];
    for beat in &t.beats {
        b.extend_from_slice(&beat.beat.to_le_bytes());
        b.extend_from_slice(&beat.tempo_x100.to_le_bytes());
        b.extend_from_slice(&beat.time_ms.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
    }
    b
}

/// Waveform preview blob: 400 columns × (height, whiteness), then the 100-byte tiny preview.
pub fn preview_blob(t: &LibTrack) -> Vec<u8> {
    let mut b = Vec::with_capacity(900);
    if let Some(p) = anlz_tag(t, FourCC::PWAV, "DAT") {
        for &x in p.body.iter().take(400) {
            b.push(x & 0x1F);
            b.push(x >> 5);
        }
    }
    b.resize(800, 0);
    if let Some(p) = anlz_tag(t, FourCC::PWV2, "DAT") {
        b.extend(p.body.iter().take(100));
    }
    b.resize(900, 0);
    b
}
