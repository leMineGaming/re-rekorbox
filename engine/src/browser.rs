//! File browser for the controller's BROWSE / LOAD / BACK / TRACK ◀▶ / TAG controls. It works on
//! folders of audio files (the rekordbox library itself is a later MVP step).

use std::path::{Path, PathBuf};

const AUDIO_EXT: [&str; 9] = ["flac", "mp3", "wav", "aif", "aiff", "m4a", "aac", "ogg", "mp4"];

/// True for files with a playable audio extension.
pub fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| AUDIO_EXT.contains(&e.to_lowercase().as_str()))
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Folder(PathBuf),
    Track(PathBuf),
    /// Virtual folder with the library tracks (shown at the root once a library is set).
    Library,
}

impl Entry {
    pub fn path(&self) -> &Path {
        match self {
            Entry::Folder(p) | Entry::Track(p) => p,
            Entry::Library => Path::new(""),
        }
    }

    pub fn name(&self) -> String {
        if *self == Entry::Library {
            return "[Library]".into();
        }
        self.path().file_name().map_or_else(|| self.path().display().to_string(), |n| n.to_string_lossy().into_owned())
    }
}

/// What the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum View {
    Folder(PathBuf),
    TagList,
    /// Tracks related to a file (same artist prefix "Artist - Title").
    Related(PathBuf),
    /// All library tracks.
    Library,
}

#[derive(Debug, Clone)]
pub struct Browser {
    pub root: PathBuf,
    pub view: View,
    entries: Vec<Entry>,
    pub selected: usize,
    pub tags: Vec<PathBuf>,
    /// Waveform zoom level (BROWSE + SHIFT), 1..=8, kept for a display.
    pub zoom: i32,
    /// Library tracks (audio paths, in display order).
    pub library: Vec<PathBuf>,
}

fn list_dir(dir: &Path) -> Vec<Entry> {
    let mut folders = Vec::new();
    let mut tracks = Vec::new();
    if let Ok(it) = std::fs::read_dir(dir) {
        for e in it.flatten() {
            let p = e.path();
            if p.is_dir() {
                folders.push(p);
            } else if is_audio(&p) {
                tracks.push(p);
            }
        }
    }
    folders.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    tracks.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    folders.into_iter().map(Entry::Folder).chain(tracks.into_iter().map(Entry::Track)).collect()
}

/// Artist part of "Artist - Title.ext", lower case.
fn artist_of(p: &Path) -> Option<String> {
    let stem = p.file_stem()?.to_string_lossy().to_lowercase();
    stem.split_once(" - ").map(|(a, _)| a.trim().to_string())
}

impl Browser {
    pub fn new(root: PathBuf) -> Browser {
        let mut b = Browser {
            view: View::Folder(root.clone()),
            root,
            entries: Vec::new(),
            selected: 0,
            tags: Vec::new(),
            zoom: 4,
            library: Vec::new(),
        };
        b.refresh();
        b
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Re-read the current view.
    pub fn refresh(&mut self) {
        self.entries = match &self.view {
            View::Folder(d) if d == &self.root && !self.library.is_empty() => {
                std::iter::once(Entry::Library).chain(list_dir(d)).collect()
            }
            View::Folder(d) => list_dir(d),
            View::Library => self.library.iter().cloned().map(Entry::Track).collect(),
            View::TagList => self.tags.iter().cloned().map(Entry::Track).collect(),
            View::Related(f) => {
                let artist = artist_of(f);
                let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
                list_dir(&dir)
                    .into_iter()
                    .filter(|e| matches!(e, Entry::Track(p) if p != f && artist.is_some() && artist_of(p) == artist))
                    .collect()
            }
        };
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    /// BROWSE rotary.
    pub fn scroll(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let n = self.entries.len() as i64;
        self.selected = (self.selected as i64 + i64::from(delta)).clamp(0, n - 1) as usize;
    }

    /// BROWSE press: open the selected folder. Returns true when the view changed.
    pub fn enter(&mut self) -> bool {
        match self.selected_entry().cloned() {
            Some(Entry::Folder(p)) => self.view = View::Folder(p),
            Some(Entry::Library) => self.view = View::Library,
            _ => return false,
        }
        self.selected = 0;
        self.refresh();
        true
    }

    /// Set the library tracks (shown as `[Library]` at the root).
    pub fn set_library(&mut self, tracks: Vec<PathBuf>) {
        self.library = tracks;
        self.refresh();
    }

    /// BACK: parent folder (not above the root); from the tag list / related view back to folders.
    pub fn back(&mut self) {
        let next = match &self.view {
            View::Folder(d) if d != &self.root => d.parent().map(Path::to_path_buf),
            View::Folder(_) => None,
            View::TagList | View::Related(_) | View::Library => Some(self.root.clone()),
        };
        if let Some(d) = next {
            let came_from = match &self.view {
                View::Folder(c) => Some(c.clone()),
                _ => None,
            };
            self.view = View::Folder(d);
            self.refresh();
            self.selected = came_from
                .and_then(|c| self.entries.iter().position(|e| e.path() == c))
                .unwrap_or(0);
        }
    }

    /// Selected track, if the selection is a track.
    pub fn selected_track(&self) -> Option<PathBuf> {
        match self.selected_entry() {
            Some(Entry::Track(p)) => Some(p.clone()),
            _ => None,
        }
    }

    /// TAG: add / remove the selected track from the tag list.
    pub fn toggle_tag(&mut self) {
        if let Some(p) = self.selected_track() {
            if let Some(i) = self.tags.iter().position(|t| t == &p) {
                self.tags.remove(i);
            } else {
                self.tags.push(p);
            }
            if self.view == View::TagList {
                self.refresh();
            }
        }
    }

    /// TRACK FILTER / AREA: switch between the folders and the tag list.
    pub fn toggle_tag_view(&mut self) {
        self.view = if self.view == View::TagList { View::Folder(self.root.clone()) } else { View::TagList };
        self.selected = 0;
        self.refresh();
    }

    /// RELATED TRACKS: tracks of the same artist as the selected track.
    pub fn show_related(&mut self) {
        if let Some(p) = self.selected_track() {
            self.view = View::Related(p);
            self.selected = 0;
            self.refresh();
        }
    }

    pub fn zoom(&mut self, delta: i32) {
        self.zoom = (self.zoom + delta).clamp(1, 8);
    }

    /// Next / previous audio file in the folder of `current` (TRACK ◀▶ on a deck).
    pub fn neighbour(current: &Path, forward: bool) -> Option<PathBuf> {
        let tracks: Vec<PathBuf> = list_dir(current.parent()?)
            .into_iter()
            .filter_map(|e| match e {
                Entry::Track(p) => Some(p),
                Entry::Folder(_) | Entry::Library => None,
            })
            .collect();
        let i = tracks.iter().position(|p| p == current)?;
        let j = if forward { i + 1 } else { i.checked_sub(1)? };
        tracks.get(j).cloned()
    }

    /// Short text for a console display: view, selected row and its neighbours.
    pub fn describe(&self) -> String {
        let title = match &self.view {
            View::Folder(d) => d.display().to_string(),
            View::TagList => "[tag list]".into(),
            View::Library => "[Library]".into(),
            View::Related(p) => format!("[related to {}]", p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned())),
        };
        let mut s = format!("{title} ({} entries)", self.entries.len());
        let lo = self.selected.saturating_sub(2);
        for (i, e) in self.entries.iter().enumerate().skip(lo).take(5) {
            let mark = if i == self.selected { ">" } else { " " };
            let kind = if matches!(e, Entry::Folder(_) | Entry::Library) { "/" } else { "" };
            let tag = if self.tags.iter().any(|t| t == e.path()) { " *" } else { "" };
            s.push_str(&format!("\n {mark} {}{kind}{tag}", e.name()));
        }
        s
    }
}
