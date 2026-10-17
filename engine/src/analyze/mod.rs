//! Track analysis (own implementation, SPEC §5j): waveforms, beat grid, key, seek tables, ANLZ output.
//!
//! [`analyze_file`] runs everything on one audio file; [`write_anlz`] stores the result as
//! `ANLZ0000.DAT/.EXT/.2EX` (rekordbox tag order) plus a small side-car `rbx-analysis.txt` with the
//! key, tempo and confidence.

pub mod beats;
pub mod key;
pub mod seek;
pub mod wave;

use std::path::{Path, PathBuf};

use rekordbox_formats::anlz::tags::{Beat, BeatGrid, CueList, CueListType, Tag, Vbr, Vbr2, PVBR_ENTRIES};
use rekordbox_formats::anlz::waveform::{MonoColumn, WavePreview, Waveform};
use rekordbox_formats::anlz::{AnlzFile, FourCC, RawTag};

use crate::pcm::{decode_file, DecodeError, Pcm};

/// Everything the analysis produced for one file.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub path: PathBuf,
    pub sample_rate: u32,
    pub samples: u64,
    pub grid: Option<beats::GridResult>,
    pub beats: Vec<Beat>,
    pub key: Option<(key::Key, f64)>,
    pub waveforms: wave::Waveforms,
    /// FLAC seek table (`PVB2` payload).
    pub flac_seek: Option<Vec<u8>>,
    pub is_mp3: bool,
}

/// Analyse decoded audio (the path is only stored, and used for the FLAC seek table).
pub fn analyze_pcm(path: &Path, pcm: &Pcm) -> AnalysisResult {
    let ons = beats::onsets(pcm);
    let grid = beats::detect_grid_with(pcm, &ons);
    let beats = grid.as_ref().map(|g| beats::grid_beats(g, pcm.duration_s())).unwrap_or_default();
    let key = key::detect_key(&key::key_chroma(pcm));
    let rec = wave::ms_records(pcm);
    let waveforms = wave::waveforms(&rec, pcm.len(), pcm.sample_rate);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let flac_seek = (ext == "flac")
        .then(|| seek::flac_frames(path))
        .flatten()
        .filter(|f| !f.is_empty())
        .map(|f| seek::flac_pvb2(&f, pcm.len() as u64));
    AnalysisResult {
        path: path.to_path_buf(),
        sample_rate: pcm.sample_rate,
        samples: pcm.len() as u64,
        grid,
        beats,
        key,
        waveforms,
        flac_seek,
        is_mp3: ext == "mp3",
    }
}

/// Decode and analyse an audio file.
pub fn analyze_file(path: &Path) -> Result<AnalysisResult, DecodeError> {
    let pcm = decode_file(path)?;
    Ok(analyze_pcm(path, &pcm))
}

fn waveform(kind: FourCC, entry_size: u16, extra: Option<[u8; 4]>, data: Vec<u8>) -> RawTag {
    Tag::Waveform(Waveform { kind, entry_size, extra, data }).to_raw()
}

fn empty_pco2(hot: bool) -> RawTag {
    let mut h = Vec::new();
    h.extend_from_slice(&u32::from(hot).to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    RawTag::new(FourCC::PCO2, h, Vec::new())
}

fn mono(cols: &[MonoColumn]) -> Vec<u8> {
    cols.iter().map(|c| c.to_byte()).collect()
}

/// The three analysis files of a result: (DAT, EXT, 2EX) as bytes. `stored_path` goes into `PPTH`.
pub fn anlz_files(r: &AnalysisResult, stored_path: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = &r.waveforms;
    let path_tag = Tag::Path(stored_path.to_string()).to_raw();
    // DAT: PPTH PVBR PQTZ PWAV PWV2 PCOB PCOB
    let mut dat = AnlzFile::new();
    dat.tags.push(path_tag.clone());
    let table = [0u32; PVBR_ENTRIES];
    let extra = if r.is_mp3 { r.samples as u32 } else { 0 };
    dat.tags.push(Tag::Vbr(Vbr::new(&table, extra)).to_raw());
    dat.tags.push(Tag::BeatGrid(BeatGrid::new(r.beats.clone())).to_raw());
    dat.tags.push(Tag::WavePreview(WavePreview::new(FourCC::PWAV, mono(&w.preview))).to_raw());
    dat.tags.push(Tag::WavePreview(WavePreview::new(FourCC::PWV2, mono(&w.tiny))).to_raw());
    dat.tags.push(Tag::CueList(CueList::new(CueListType::HotCue)).to_raw());
    dat.tags.push(Tag::CueList(CueList::new(CueListType::Memory)).to_raw());
    // EXT: PPTH PWV3 PCOB PCOB PCO2 PCO2 PWV5 PWV4 [PVB2]
    let mut ext = AnlzFile::new();
    ext.tags.push(path_tag.clone());
    ext.tags.push(Tag::Waveform(Waveform::mono_detail(&w.detail)).to_raw());
    ext.tags.push(Tag::CueList(CueList::new(CueListType::HotCue)).to_raw());
    ext.tags.push(Tag::CueList(CueList::new(CueListType::Memory)).to_raw());
    ext.tags.push(empty_pco2(true));
    ext.tags.push(empty_pco2(false));
    ext.tags.push(Tag::Waveform(Waveform::color_detail(&w.color)).to_raw());
    ext.tags.push(waveform(FourCC::PWV4, 6, Some([0; 4]), w.color_preview.iter().flatten().copied().collect()));
    if let Some(seek) = &r.flac_seek {
        ext.tags.push(Tag::Vbr2(Vbr2 { unknown_0e: 0, sample_count: r.samples, entry_size: 20, data: seek.clone() }).to_raw());
    }
    // 2EX: PPTH PWV7 PWV6 PWVC
    let mut ex2 = AnlzFile::new();
    ex2.tags.push(path_tag);
    ex2.tags.push(waveform(FourCC::PWV7, 3, Some([0x00, 0x96, 0x00, 0x00]), w.bands.iter().flatten().copied().collect()));
    ex2.tags.push(waveform(FourCC::PWV6, 3, None, w.bands_preview.iter().flatten().copied().collect()));
    let mut pwvc = Vec::new();
    for v in w.pwvc {
        pwvc.extend_from_slice(&v.to_be_bytes());
    }
    ex2.tags.push(RawTag::new(FourCC::PWVC, vec![0, 0], pwvc));
    (dat.to_bytes(), ext.to_bytes(), ex2.to_bytes())
}

/// Folder for our own analyses: `%APPDATA%\rekordbox-rs\ANLZ`.
pub fn default_output_root() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("rekordbox-rs").join("ANLZ"))
}

/// Stable folder for a file path (FNV-1a hash of the lower-case path, on Windows with `/` read as
/// `\`), split like rekordbox (`<3 hex>/<rest>`).
pub fn folder_for(path: &Path) -> PathBuf {
    let mut s = path.to_string_lossy().to_lowercase();
    if cfg!(windows) {
        s = s.replace('/', "\\");
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let s = format!("{h:016x}");
    PathBuf::from(&s[..3]).join(&s[3..])
}

/// Write the analysis below `root`; returns the folder. `PPTH` holds the full path of the audio file.
pub fn write_anlz(r: &AnalysisResult, root: &Path) -> std::io::Result<PathBuf> {
    let dir = root.join(folder_for(&r.path));
    std::fs::create_dir_all(&dir)?;
    let (dat, ext, ex2) = anlz_files(r, &r.path.to_string_lossy());
    std::fs::write(dir.join("ANLZ0000.DAT"), dat)?;
    std::fs::write(dir.join("ANLZ0000.EXT"), ext)?;
    std::fs::write(dir.join("ANLZ0000.2EX"), ex2)?;
    let mut side = format!("path={}\nsamples={}\nsample_rate={}\n", r.path.display(), r.samples, r.sample_rate);
    if let Some(g) = &r.grid {
        side.push_str(&format!("bpm={:.2}\nconfidence={:.3}\n", g.bpm, g.confidence));
    }
    if let Some((k, c)) = r.key {
        side.push_str(&format!("key={}\ncamelot={}\nkey_confidence={c:.3}\n", k.name(), k.camelot()));
    }
    std::fs::write(dir.join("rbx-analysis.txt"), side)?;
    Ok(dir)
}
