//! Rows of the track list: analysed library tracks plus audio files of the music folder that have
//! no analysis yet.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rekordbox_formats::library::{split_name, Library, Source};

/// Where a row's analysis comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSource {
    Rekordbox,
    /// Own analyzer (`rbx-analyze` / automatic).
    Own,
    /// Not analysed yet.
    Missing,
}

impl RowSource {
    pub fn label(self) -> &'static str {
        match self {
            RowSource::Rekordbox => "rekordbox",
            RowSource::Own => "eigene",
            RowSource::Missing => "–",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    pub path: PathBuf,
    pub artist: String,
    pub title: String,
    pub bpm: Option<f64>,
    pub key: Option<String>,
    pub source: RowSource,
    pub variable_tempo: bool,
    haystack: String,
}

impl TrackRow {
    fn new(path: PathBuf, artist: Option<String>, title: String, source: RowSource) -> TrackRow {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let haystack = format!("{} {title} {file}", artist.as_deref().unwrap_or("")).to_lowercase();
        TrackRow { path, artist: artist.unwrap_or_default(), title, bpm: None, key: None, source, variable_tempo: false, haystack }
    }

    /// True when every word of `query` occurs in artist, title or file name (case-insensitive).
    pub fn matches(&self, query: &str) -> bool {
        query.to_lowercase().split_whitespace().all(|w| self.haystack.contains(w))
    }
}

fn norm(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

/// Library tracks with an audio file, then the remaining audio files; sorted by artist and title.
pub fn build_rows(lib: Option<&Library>, files: &[PathBuf]) -> Vec<TrackRow> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    if let Some(lib) = lib {
        for t in lib.playable() {
            let Some(p) = &t.path else { continue };
            if !seen.insert(norm(p)) {
                continue;
            }
            let source = match t.source {
                Source::Rekordbox => RowSource::Rekordbox,
                Source::Own => RowSource::Own,
            };
            let mut r = TrackRow::new(p.clone(), t.artist.clone(), t.title.clone(), source);
            r.bpm = t.bpm();
            r.key = t.key.clone();
            r.variable_tempo = t.variable_tempo();
            rows.push(r);
        }
    }
    for f in files {
        if seen.insert(norm(f)) {
            let (artist, title) = split_name(&f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            rows.push(TrackRow::new(f.clone(), artist, title, RowSource::Missing));
        }
    }
    rows.sort_by_cached_key(|r| (r.artist.to_lowercase(), r.title.to_lowercase()));
    rows
}

/// Indices of the rows matching `query`.
pub fn filter(rows: &[TrackRow], query: &str) -> Vec<usize> {
    rows.iter().enumerate().filter(|(_, r)| r.matches(query)).map(|(i, _)| i).collect()
}
