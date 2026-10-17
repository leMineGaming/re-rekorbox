//! Library from analysis folders: synthetic ANLZ files written with this crate's own builders and a
//! fake music folder (dummy audio files) in a temporary directory.

use std::path::{Path, PathBuf};

use rekordbox_formats::anlz::tags::{Beat, BeatGrid, CueList, CueListType, CuePoint, Tag, Vbr2};
use rekordbox_formats::anlz::AnlzFile;
use rekordbox_formats::library::{find_anlz_dirs, split_name, Library, MusicIndex};

fn write_anlz(dir: &Path, stored: &str, beats: &[(u16, u16, u32)], cues: &[(CueListType, u16, u32, Option<u32>)]) {
    std::fs::create_dir_all(dir).unwrap();
    let mut f = AnlzFile::new();
    f.tags.push(Tag::Path(stored.into()).to_raw());
    let beats = beats.iter().map(|&(beat, tempo_x100, time_ms)| Beat { beat, tempo_x100, time_ms }).collect();
    f.tags.push(Tag::BeatGrid(BeatGrid::new(beats)).to_raw());
    for t in [CueListType::HotCue, CueListType::Memory] {
        let mut list = CueList::new(t);
        for (_, slot, start, end) in cues.iter().filter(|c| c.0 == t) {
            list.cues.push(CuePoint::new(*slot, *start, *end));
        }
        f.tags.push(Tag::CueList(list).to_raw());
    }
    std::fs::write(dir.join("ANLZ0000.DAT"), f.to_bytes()).unwrap();
    let mut ext = AnlzFile::new();
    ext.tags.push(Tag::Path(stored.into()).to_raw());
    ext.tags.push(Tag::Vbr2(Vbr2 { unknown_0e: 0, sample_count: 44100 * 180, entry_size: 4, data: vec![0; 16] }).to_raw());
    std::fs::write(dir.join("ANLZ0000.EXT"), ext.to_bytes()).unwrap();
}

fn constant_grid(bpm_x100: u16, first_ms: u32, n: u32, first_beat_in_bar: u16) -> Vec<(u16, u16, u32)> {
    let step = 6_000_000.0 / f64::from(bpm_x100);
    (0..n).map(|i| (((u32::from(first_beat_in_bar) - 1 + i) % 4 + 1) as u16, bpm_x100, first_ms + (f64::from(i) * step) as u32)).collect()
}

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!("rbx-library-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let anlz = root.join("USBANLZ");
    // track A: constant 128 BPM, first beat is beat 3 of a bar, hot cues A and C, a memory cue and a loop
    write_anlz(
        &anlz.join("001/aaa"),
        "?/Artist One - Alpha.flac",
        &constant_grid(12800, 100, 64, 3),
        &[
            (CueListType::HotCue, 1, 1000, None),
            (CueListType::HotCue, 3, 5000, Some(6875)),
            (CueListType::Memory, 0, 30000, None),
        ],
    );
    // track B: tempo change 120 -> 126 BPM, file exists twice (two playlists)
    let mut beats = constant_grid(12000, 0, 32, 1);
    let last = beats.last().unwrap().2;
    beats.extend(constant_grid(12600, last + 500, 32, 1));
    write_anlz(&anlz.join("002/bbb"), "?/Artist Two - Beta.mp3", &beats, &[]);
    // track C: no audio file
    write_anlz(&anlz.join("003/ccc"), "?/Missing - Gamma.wav", &constant_grid(14000, 0, 8, 1), &[]);
    // broken folder: no DAT
    std::fs::create_dir_all(anlz.join("004/ddd")).unwrap();
    let music = root.join("Music");
    for f in ["Set 1/Artist One - Alpha.flac", "Set 1/Artist Two - Beta.mp3", "Set 2/artist two - beta.MP3", "Set 2/notes.txt"] {
        let p = music.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"not really audio").unwrap();
    }
    Fixture { root }
}

#[test]
fn builds_and_matches_the_library() {
    let fx = fixture();
    assert_eq!(find_anlz_dirs(&fx.root.join("USBANLZ")).len(), 3, "folders with ANLZ0000.DAT");
    let lib = Library::build(&fx.root.join("USBANLZ"), &[fx.root.join("Music")]);
    let r = lib.report;
    assert_eq!((r.anlz_dirs, r.tracks, r.matched, r.missing, r.ambiguous), (3, 3, 2, 1, 1));
    assert_eq!((r.with_grid, r.variable_tempo, r.with_cues, r.music_files), (3, 1, 1, 3));
    // sorted by artist, title
    let names: Vec<_> = lib.tracks.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(names, ["Alpha", "Beta", "Gamma"]);

    let a = &lib.tracks[0];
    assert_eq!((a.artist.as_deref(), a.file_name.as_str()), (Some("Artist One"), "Artist One - Alpha.flac"));
    assert_eq!(a.bpm(), Some(128.0));
    assert!(!a.variable_tempo());
    // first beat is beat 3 -> first downbeat is the third entry
    let step = 60.0 / 128.0;
    assert!((a.first_downbeat_s().unwrap() - (0.1 + 2.0 * step)).abs() < 0.002);
    assert_eq!(a.hot_cues_s()[0], Some(1.0));
    assert_eq!(a.hot_cues_s()[2], Some(5.0));
    assert_eq!(a.hot_cues_s()[1], None);
    assert_eq!(a.memory_cues_s(), [30.0]);
    assert!(a.cues.iter().any(|c| c.slot == 3 && c.end_ms == Some(6875)), "loop end kept");
    assert_eq!(a.duration_s(44100), Some(180.0));

    let b = &lib.tracks[1];
    assert!(b.variable_tempo());
    assert_eq!(b.candidates, 2, "same name in two folders (case-insensitive)");
    assert!(lib.tracks[2].path.is_none());
}

#[test]
fn lookup_and_search() {
    let fx = fixture();
    let lib = Library::build(&fx.root.join("USBANLZ"), &[fx.root.join("Music")]);
    let copy = fx.root.join("Music/Set 2/artist two - beta.MP3");
    assert_eq!(lib.by_path(&copy).unwrap().title, "Beta", "every copy finds the analysis");
    assert_eq!(lib.by_path(Path::new("D:/elsewhere/Artist One - Alpha.flac")).unwrap().title, "Alpha", "by name");
    assert!(lib.by_path(Path::new("x/unknown.flac")).is_none());
    assert_eq!(lib.search("artist").len(), 2);
    assert_eq!(lib.search("two BETA").len(), 1, "all words, any case");
    assert_eq!(lib.search("gamma").len(), 1);
    assert_eq!(lib.playable().count(), 2);
    assert_eq!(MusicIndex::build(&[fx.root.join("Music")]).lookup("ARTIST ONE - ALPHA.FLAC").len(), 1);
}

#[test]
fn name_splitting() {
    assert_eq!(split_name("Jonas Blue, Felix Jaehn - Weekends.flac"), (Some("Jonas Blue, Felix Jaehn".into()), "Weekends".into()));
    assert_eq!(split_name("A - B - C.mp3"), (Some("A".into()), "B - C".into()));
    assert_eq!(split_name("Breaks1.wav"), (None, "Breaks1".into()));
    assert_eq!(split_name(" - x.wav"), (None, " - x".into()));
}
