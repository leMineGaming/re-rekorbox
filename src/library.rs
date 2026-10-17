//! Local track library from rekordbox's analysis folder (no database needed).
//!
//! rekordbox keeps one analysis folder per track below `share/PIONEER/USBANLZ` (`ANLZ0000.DAT`,
//! `.EXT`, `.2EX`, `.3EX`; SPEC §5a–§5d). The `PPTH` tag of local analysis files holds only the file
//! name (`?/Artist - Title.flac`), so audio files are found by name in a [`MusicIndex`] built from
//! music folders. Each [`LibTrack`] carries what the analysis provides: beat grid, cues (if stored),
//! song structure (phrases), track length in samples, and the analysis folder for waveforms.
//!
//! The encrypted `master.db` is not read (SPEC §6); playlists, ratings and cues stored only in the
//! database are therefore not available.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::anlz::phrase::SongStructure;
use crate::anlz::tags::{Beat, CueListType, CueType, Tag};
use crate::anlz::AnlzFile;

const AUDIO_EXT: [&str; 9] = ["flac", "mp3", "wav", "aif", "aiff", "m4a", "aac", "ogg", "mp4"];

/// True for files with an audio extension.
pub fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXT.contains(&e.to_lowercase().as_str()))
}

/// A cue point from the analysis file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    /// 0 = memory cue, 1… = hot cue A…
    pub slot: u16,
    pub start_ms: u32,
    /// Loop end, if the cue is a loop.
    pub end_ms: Option<u32>,
}

/// Where an analysis comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    Rekordbox,
    /// Written by this project's analyzer (full path in `PPTH`, side-car `rbx-analysis.txt`).
    Own,
}

/// One analysed track.
#[derive(Debug, Clone, PartialEq)]
pub struct LibTrack {
    pub anlz_dir: PathBuf,
    /// Path as stored in `PPTH`.
    pub stored_path: String,
    /// File name part of the stored path.
    pub file_name: String,
    /// Matched audio file.
    pub path: Option<PathBuf>,
    /// Number of audio files with this name in the index (> 1 = ambiguous, the first is used).
    pub candidates: usize,
    pub artist: Option<String>,
    pub title: String,
    /// `PQTZ` beat grid.
    pub beats: Vec<Beat>,
    /// Length in samples (`PVB2`), if known.
    pub samples: Option<u64>,
    pub cues: Vec<Cue>,
    pub structure: Option<SongStructure>,
    /// Musical key (e.g. `Am`) from an own analysis.
    pub key: Option<String>,
    pub source: Source,
}

impl LibTrack {
    /// Tempo of the first beat in BPM.
    pub fn bpm(&self) -> Option<f64> {
        self.beats.first().map(|b| b.bpm()).filter(|&b| b > 0.0)
    }

    /// True when the grid changes tempo by more than 0.5 %.
    pub fn variable_tempo(&self) -> bool {
        let Some(first) = self.beats.first() else { return false };
        let t0 = f64::from(first.tempo_x100);
        self.beats.iter().any(|b| (f64::from(b.tempo_x100) - t0).abs() > t0 * 0.005)
    }

    /// Time of the first downbeat (beat 1 of a bar) in seconds.
    pub fn first_downbeat_s(&self) -> Option<f64> {
        self.beats.iter().find(|b| b.beat == 1).or(self.beats.first()).map(|b| f64::from(b.time_ms) / 1000.0)
    }

    /// Duration estimate from the sample count (44.1 kHz analysis rate assumed when unknown).
    pub fn duration_s(&self, sample_rate: u32) -> Option<f64> {
        self.samples.map(|s| s as f64 / f64::from(sample_rate.max(1)))
    }

    /// Hot cues A–H (slot 1–8): start in seconds.
    pub fn hot_cues_s(&self) -> [Option<f64>; 8] {
        let mut out = [None; 8];
        for c in &self.cues {
            if (1..=8).contains(&c.slot) {
                out[usize::from(c.slot - 1)] = Some(f64::from(c.start_ms) / 1000.0);
            }
        }
        out
    }

    /// Memory cues (slot 0) in seconds, sorted.
    pub fn memory_cues_s(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.cues.iter().filter(|c| c.slot == 0).map(|c| f64::from(c.start_ms) / 1000.0).collect();
        v.sort_by(f64::total_cmp);
        v
    }

    /// Text used for searching: artist, title and file name, lower case.
    fn haystack(&self) -> String {
        format!("{} {} {}", self.artist.as_deref().unwrap_or(""), self.title, self.file_name).to_lowercase()
    }
}

/// "Artist - Title.ext" → (Some(artist), title); otherwise (None, stem).
pub fn split_name(file_name: &str) -> (Option<String>, String) {
    let stem = Path::new(file_name).file_stem().map_or(file_name.to_string(), |s| s.to_string_lossy().into_owned());
    match stem.split_once(" - ") {
        Some((a, t)) if !a.trim().is_empty() => (Some(a.trim().to_string()), t.trim().to_string()),
        _ => (None, stem),
    }
}

fn read_anlz(p: &Path) -> Option<Vec<Tag>> {
    let bytes = std::fs::read(p).ok()?;
    let f = AnlzFile::parse(&bytes).ok()?;
    Some(f.tags.iter().filter_map(|r| Tag::from_raw(r).ok()).collect())
}

/// Read one analysis folder (`ANLZ0000.DAT` required, `.EXT` optional).
pub fn read_track(dir: &Path) -> Option<LibTrack> {
    let dat = read_anlz(&dir.join("ANLZ0000.DAT"))?;
    let ext = read_anlz(&dir.join("ANLZ0000.EXT")).unwrap_or_default();
    let mut t = LibTrack {
        anlz_dir: dir.to_path_buf(),
        stored_path: String::new(),
        file_name: String::new(),
        path: None,
        candidates: 0,
        artist: None,
        title: String::new(),
        beats: Vec::new(),
        samples: None,
        cues: Vec::new(),
        structure: None,
        key: None,
        source: Source::Rekordbox,
    };
    if let Ok(side) = std::fs::read_to_string(dir.join("rbx-analysis.txt")) {
        t.source = Source::Own;
        t.key = side.lines().find_map(|l| l.strip_prefix("key=")).map(str::to_string);
    }
    for tag in dat.iter().chain(ext.iter()) {
        match tag {
            Tag::Path(p) if t.stored_path.is_empty() => t.stored_path = p.clone(),
            Tag::BeatGrid(g) if t.beats.is_empty() => t.beats = g.beats.clone(),
            Tag::Vbr2(v) => t.samples = Some(v.sample_count),
            Tag::SongStructure(s) => t.structure = Some(s.clone()),
            Tag::CueList(list) => {
                for c in &list.cues {
                    let end_ms = if c.cue_type() == CueType::Loop { c.end_ms() } else { None };
                    let slot = if list.list_type == CueListType::Memory { 0 } else { c.slot };
                    let cue = Cue { slot, start_ms: c.start_ms(), end_ms };
                    if !t.cues.contains(&cue) {
                        t.cues.push(cue);
                    }
                }
            }
            _ => {}
        }
    }
    if t.stored_path.is_empty() {
        return None;
    }
    t.file_name = t.stored_path.rsplit(['/', '\\']).next().unwrap_or(&t.stored_path).to_string();
    let (artist, title) = split_name(&t.file_name);
    t.artist = artist;
    t.title = title;
    Some(t)
}

/// All analysis folders below `root` (folders containing `ANLZ0000.DAT`).
pub fn find_anlz_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(it) = std::fs::read_dir(&d) else { continue };
        let mut has_dat = false;
        for e in it.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == "ANLZ0000.DAT") {
                has_dat = true;
            }
        }
        if has_dat {
            out.push(d);
        }
    }
    out.sort();
    out
}

/// Audio files of some folders, indexed by lower-case file name.
#[derive(Debug, Clone, Default)]
pub struct MusicIndex {
    by_name: HashMap<String, Vec<PathBuf>>,
    pub files: usize,
}

impl MusicIndex {
    pub fn build(roots: &[PathBuf]) -> MusicIndex {
        let mut idx = MusicIndex::default();
        for root in roots {
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                let Ok(it) = std::fs::read_dir(&d) else { continue };
                for e in it.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if is_audio(&p) {
                        if let Some(n) = p.file_name() {
                            idx.by_name.entry(n.to_string_lossy().to_lowercase()).or_default().push(p);
                            idx.files += 1;
                        }
                    }
                }
            }
        }
        for v in idx.by_name.values_mut() {
            v.sort();
        }
        idx
    }

    pub fn lookup(&self, file_name: &str) -> &[PathBuf] {
        self.by_name.get(&file_name.to_lowercase()).map_or(&[], Vec::as_slice)
    }
}

/// Counts from building a library.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LibraryReport {
    pub anlz_dirs: usize,
    pub unreadable: usize,
    pub tracks: usize,
    pub matched: usize,
    pub ambiguous: usize,
    pub missing: usize,
    pub with_grid: usize,
    pub variable_tempo: usize,
    pub with_cues: usize,
    pub with_phrases: usize,
    pub music_files: usize,
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    pub tracks: Vec<LibTrack>,
    pub report: LibraryReport,
    by_path: HashMap<PathBuf, usize>,
}

/// rekordbox's local analysis folder for the current user (`%APPDATA%\Pioneer\rekordbox\share\PIONEER\USBANLZ`).
pub fn default_anlz_root() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("APPDATA")?).join("Pioneer").join("rekordbox").join("share").join("PIONEER").join("USBANLZ");
    p.is_dir().then_some(p)
}

/// The user's music folder (`%USERPROFILE%\Music`).
pub fn default_music_root() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("USERPROFILE")?).join("Music");
    p.is_dir().then_some(p)
}

impl Library {
    /// Read all analysis folders below `anlz_root` and match them against `music_roots`.
    pub fn build(anlz_root: &Path, music_roots: &[PathBuf]) -> Library {
        Library::build_multi(&[anlz_root.to_path_buf()], music_roots)
    }

    /// Like [`Library::build`] with several analysis folders (e.g. rekordbox's and our own). When two
    /// analyses describe the same audio file, the first folder wins (rekordbox before own).
    pub fn build_multi(anlz_roots: &[PathBuf], music_roots: &[PathBuf]) -> Library {
        let index = MusicIndex::build(music_roots);
        let mut lib = Library::default();
        lib.report.music_files = index.files;
        for root in anlz_roots {
            let dirs = find_anlz_dirs(root);
            lib.report.anlz_dirs += dirs.len();
            for d in &dirs {
                match read_track(d) {
                    Some(t) => lib.tracks.push(t),
                    None => lib.report.unreadable += 1,
                }
            }
        }
        lib.match_files(&index);
        lib.dedupe();
        lib
    }

    /// Drop later analyses of an audio file that an earlier one already covers.
    fn dedupe(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut order: Vec<(usize, LibTrack)> = self.tracks.drain(..).enumerate().collect();
        order.sort_by_key(|(i, t)| (t.source == Source::Own, *i));
        let mut kept = Vec::new();
        for (_, t) in order {
            let key = t.path.clone().map(|p| p.to_string_lossy().to_lowercase()).unwrap_or_else(|| t.file_name.to_lowercase());
            if seen.insert(key) {
                kept.push(t);
            }
        }
        self.tracks = kept;
        self.tracks.sort_by_key(|t| (t.artist.clone().unwrap_or_default().to_lowercase(), t.title.to_lowercase()));
        self.reindex();
    }

    fn reindex(&mut self) {
        self.by_path.clear();
        for (i, t) in self.tracks.iter().enumerate() {
            if let Some(p) = &t.path {
                self.by_path.entry(p.clone()).or_insert(i);
            }
        }
    }

    /// Add (or replace) one analysis, e.g. right after analysing a file.
    pub fn insert(&mut self, mut t: LibTrack) {
        if t.path.is_none() {
            let p = PathBuf::from(&t.stored_path);
            if p.is_file() {
                t.path = Some(p);
            }
        }
        if let Some(p) = &t.path {
            let rekordbox_has_it = self.tracks.iter().any(|x| x.path.as_ref() == Some(p) && x.source == Source::Rekordbox);
            if rekordbox_has_it && t.source == Source::Own {
                return;
            }
            self.tracks.retain(|x| x.path.as_ref() != Some(p));
        }
        self.tracks.push(t);
        self.tracks.sort_by_key(|t| (t.artist.clone().unwrap_or_default().to_lowercase(), t.title.to_lowercase()));
        self.reindex();
    }

    /// Build from already read tracks (tests, other sources).
    pub fn from_tracks(tracks: Vec<LibTrack>, index: &MusicIndex) -> Library {
        let mut lib = Library { tracks, ..Library::default() };
        lib.report.music_files = index.files;
        lib.match_files(index);
        lib
    }

    fn match_files(&mut self, index: &MusicIndex) {
        let r = &mut self.report;
        for t in self.tracks.iter_mut() {
            // own analyses store the full path; use it when the file is there
            let direct = Path::new(&t.stored_path);
            let found = index.lookup(&t.file_name);
            t.candidates = found.len();
            t.path = if direct.is_absolute() && direct.is_file() { Some(direct.to_path_buf()) } else { found.first().cloned() };
            if t.path.is_some() {
                r.matched += 1;
            } else {
                r.missing += 1;
            }
            if found.len() > 1 {
                r.ambiguous += 1;
            }
            if !t.beats.is_empty() {
                r.with_grid += 1;
            }
            if t.variable_tempo() {
                r.variable_tempo += 1;
            }
            if !t.cues.is_empty() {
                r.with_cues += 1;
            }
            if t.structure.is_some() {
                r.with_phrases += 1;
            }
        }
        r.tracks = self.tracks.len();
        self.tracks.sort_by_key(|t| (t.artist.clone().unwrap_or_default().to_lowercase(), t.title.to_lowercase()));
        // every copy with the same file name points to this analysis
        self.by_path.clear();
        for (i, t) in self.tracks.iter().enumerate() {
            for f in index.lookup(&t.file_name) {
                self.by_path.entry(f.clone()).or_insert(i);
            }
        }
    }

    /// Analysis of an audio file (by exact path, else by file name).
    pub fn by_path(&self, path: &Path) -> Option<&LibTrack> {
        if let Some(&i) = self.by_path.get(path) {
            return self.tracks.get(i);
        }
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        self.tracks.iter().find(|t| t.file_name.to_lowercase() == name)
    }

    /// Tracks whose artist, title or file name contain every word of `query` (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&LibTrack> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        self.tracks.iter().filter(|t| {
            let h = t.haystack();
            words.iter().all(|w| h.contains(w))
        }).collect()
    }

    /// Tracks that have an audio file.
    pub fn playable(&self) -> impl Iterator<Item = &LibTrack> {
        self.tracks.iter().filter(|t| t.path.is_some())
    }
}
