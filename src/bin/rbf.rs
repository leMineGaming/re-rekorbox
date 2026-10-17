//! `rbf` – inspect and validate rekordbox files with the clean-room implementation.
//!
//! ```text
//! rbf anlz  <ANLZ file>          show tags of a .DAT/.EXT/.2EX (or decode a .3EX)
//! rbf check <directory>          parse every ANLZ file below <directory>, verify byte-exact round trips
//! rbf xml   <rekordbox.xml>      summarise a rekordbox.xml
//! rbf xml-rewrite <in> <out>     read and write a rekordbox.xml with this implementation
//! ```

use rekordbox_formats::anlz::waveform::WavePreview;
use rekordbox_formats::anlz::{threex, AnlzFile, Tag};
use rekordbox_formats::xml;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["anlz", f] => show_anlz(Path::new(f)),
        ["check", d] => check(Path::new(d)),
        ["xml", f] => show_xml(Path::new(f)),
        ["xml-rewrite", i, o] => rewrite_xml(Path::new(i), Path::new(o)),
        ["midi", p] => check_midi(Path::new(p)),
        ["pad", d] => check_pads(Path::new(d)),
        _ => Err(
            "usage: rbf anlz <file> | rbf check <dir> | rbf xml <file> | rbf xml-rewrite <in> <out> | rbf midi <file|dir>"
                .into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

type R = Result<(), String>;

fn is_3ex(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("3EX")).unwrap_or(false)
}

fn show_anlz(path: &Path) -> R {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if is_3ex(path) {
        let v = threex::decode(&data).map_err(|e| e.to_string())?;
        println!("{v:#?}");
        return Ok(());
    }
    let f = AnlzFile::parse(&data).map_err(|e| e.to_string())?;
    println!("PMAI fields={:?} tail={} tags={} size={}", f.header.fields, f.header.tail, f.tags.len(), data.len());
    for raw in &f.tags {
        print!("  {} header={} total={}", raw.fourcc, raw.header_len(), raw.total_len());
        match raw.decode() {
            Ok(Tag::Path(p)) => println!("  path={p:?}"),
            Ok(Tag::BeatGrid(g)) => {
                let first = g.beats.first().map(|b| format!("{:.2} BPM @ {} ms", b.bpm(), b.time_ms));
                println!("  beats={} first={}", g.beats.len(), first.unwrap_or_default());
            }
            Ok(Tag::WavePreview(w)) => {
                println!("  bytes={} (expected {})", w.data.len(), WavePreview::expected_len(w.kind))
            }
            Ok(Tag::Waveform(w)) => {
                println!("  entries={} entry_size={} per_second={:?}", w.count(), w.entry_size, w.entries_per_second())
            }
            Ok(Tag::CueList(c)) => println!("  type={:?} cues={}", c.list_type, c.cues.len()),
            Ok(Tag::ExtBeatGrid(q)) => println!(
                "  version={} entries={} first={:?} last={:?}",
                q.version,
                q.count(),
                q.first_beat,
                q.last_beat
            ),
            Ok(Tag::Vbr2(v)) => println!("  samples={} entries={}", v.sample_count, v.count()),
            Ok(Tag::SongStructure(s)) => {
                println!("  mood={:?} end_beat={} phrases={}", s.mood, s.end_beat, s.phrases.len());
                for p in &s.phrases {
                    let fill = if p.fill != 0 { format!(" fill@{}", p.fill_beat) } else { String::new() };
                    println!("      #{:<3} beat {:>4}  {:<9}{}", p.index, p.beat, p.label(s.mood).unwrap_or("?"), fill);
                }
            }
            Ok(Tag::Raw(_)) => println!("  (raw)"),
            Err(e) => println!("  DECODE ERROR: {e}"),
        }
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("ANLZ")).unwrap_or(false) {
            out.push(p);
        }
    }
    Ok(())
}

/// Parse all ANLZ files, check container and typed round trips, and count labels/errors.
fn check(dir: &Path) -> R {
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| e.to_string())?;
    let mut stats: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for p in &files {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
        *stats.entry(format!("files .{ext}")).or_default() += 1;
        let data = match std::fs::read(p) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        if is_3ex(p) {
            match threex::decode(&data) {
                Ok(v) if v.get("embedding").is_some() => *stats.entry("3EX decoded".into()).or_default() += 1,
                Ok(_) => failures.push(format!("{}: no embedding key", p.display())),
                Err(e) => failures.push(format!("{}: {e}", p.display())),
            }
            continue;
        }
        let f = match AnlzFile::parse(&data) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        if f.to_bytes() == data {
            *stats.entry("container round trip ok".into()).or_default() += 1;
        } else {
            failures.push(format!("{}: container round trip differs", p.display()));
        }
        if f.check_reader_order().is_err() {
            failures.push(format!("{}: first tag is not PPTH", p.display()));
        }
        for raw in &f.tags {
            match raw.decode() {
                Ok(tag) => {
                    let kind = if matches!(tag, Tag::Raw(_)) { "raw" } else { "typed" };
                    *stats.entry(format!("tag {} {kind}", raw.fourcc)).or_default() += 1;
                    if tag.to_raw() != *raw {
                        failures.push(format!("{}: typed round trip of {} differs", p.display(), raw.fourcc));
                    }
                    if let Tag::SongStructure(s) = &tag {
                        for ph in &s.phrases {
                            match ph.label(s.mood) {
                                Some(_) => *stats.entry("phrase labelled".into()).or_default() += 1,
                                None => *stats.entry("phrase WITHOUT label".into()).or_default() += 1,
                            }
                        }
                    }
                }
                Err(e) => failures.push(format!("{}: {} decode error: {e}", p.display(), raw.fourcc)),
            }
        }
    }
    for (k, v) in &stats {
        println!("{k:<32} {v}");
    }
    println!("failures                         {}", failures.len());
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

fn show_xml(path: &Path) -> R {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (doc, report) = xml::read::from_str_with_report(&s).map_err(|e| e.to_string())?;
    println!("version {} product {:?}", doc.version, doc.product);
    println!("tracks {}  playlists (top level) {}", doc.tracks.len(), doc.playlists.len());
    let tempos: usize = doc.tracks.iter().map(|t| t.tempos.len()).sum();
    let marks: usize = doc.tracks.iter().map(|t| t.marks.len()).sum();
    println!(
        "tempos {tempos}  marks {marks}  skipped marks {}  skipped tempos {}",
        report.skipped_marks, report.skipped_tempos
    );
    Ok(())
}

/// Parse MIDI mapping CSV files, compute bindings, verify write/parse round trips.
fn check_midi(path: &Path) -> R {
    use rekordbox_formats::midi;
    let files: Vec<PathBuf> = if path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().ends_with(".midi.csv"))
            .collect();
        v.sort();
        v
    } else {
        vec![path.to_path_buf()]
    };
    let mut stats: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for p in &files {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let m = midi::parse(&text);
        *stats.entry("files".into()).or_default() += 1;
        if !m.is_valid() {
            failures.push(format!("{}: not valid (version {})", p.display(), m.version));
        }
        *stats.entry("files with BOM".into()).or_default() += m.report.had_bom as usize;
        *stats.entry("entries".into()).or_default() += m.entries.len();
        *stats.entry("skipped comment lines".into()).or_default() += m.report.comment_lines;
        *stats.entry("skipped short rows".into()).or_default() += m.report.short_rows;
        *stats.entry("skipped empty name".into()).or_default() += m.report.empty_name;
        *stats.entry("skipped special (@) rows".into()).or_default() += m.report.special_rows;
        for e in &m.entries {
            let code = e.control_code(&m.device);
            if code == 0 && e.type_text != "Button" {
                *stats.entry(format!("type {:?} -> Button (unknown text)", e.type_text)).or_default() += 1;
            }
            *stats.entry("input bindings".into()).or_default() += e.input_bindings(&m.device).len();
            *stats.entry("output bindings".into()).or_default() += e.output_bindings().len();
        }
        match midi::write(&m) {
            Ok(t) => {
                let back = midi::parse(&t);
                let same = back.entries.len() == m.entries.len()
                    && back.entries.iter().zip(&m.entries).all(|(a, b)| {
                        let (mut a, mut b) = (a.clone(), b.clone());
                        a.line = 0;
                        b.line = 0;
                        a == b
                    });
                if same {
                    *stats.entry("write/parse round trip ok".into()).or_default() += 1;
                } else {
                    failures.push(format!("{}: round trip differs", p.display()));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", p.display())),
        }
    }
    for (k, v) in &stats {
        println!("{k:<40} {v}");
    }
    println!("failures                                 {}", failures.len());
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

/// Parse a `pad` directory: version, mode list, every pad file, and write round trips.
fn check_pads(dir: &Path) -> R {
    use rekordbox_formats::pad::{self, Pad};
    let version = std::fs::read_to_string(dir.join("Version")).unwrap_or_default();
    println!("version {}", version.trim());
    let modes = pad::parse_modes(&std::fs::read_to_string(dir.join("Mode.pad.csv")).map_err(|e| e.to_string())?);
    println!("modes {} ({} with a pad file)", modes.len(), modes.iter().filter(|m| m.pad_file_name().is_some()).count());
    let mut failures = Vec::new();
    let (mut files, mut pads, mut user) = (0, 0, 0);
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if !p.to_string_lossy().ends_with(".pad.csv") {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let list = pad::parse(&text);
        files += 1;
        pads += list.len();
        user += list.iter().filter(|p| Pad::clone(p).apply_user_variant()).count();
        let normalized: String = text.lines().filter(|l| !l.is_empty()).map(|l| format!("{l}\n")).collect();
        if pad::write(&list) != normalized {
            failures.push(format!("{}: round trip differs", p.display()));
        }
    }
    let missing: Vec<String> =
        modes.iter().filter_map(|m| m.pad_file_name()).filter(|f| !dir.join(f).exists()).collect();
    println!("pad files {files}, pads {pads}, pads with user variant {user}");
    println!("modes without pad file: {missing:?}");
    println!("failures {}", failures.len());
    for f in &failures {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

fn rewrite_xml(input: &Path, output: &Path) -> R {
    let s = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
    let doc = xml::from_str(&s).map_err(|e| e.to_string())?;
    std::fs::write(output, xml::to_string(&doc)).map_err(|e| e.to_string())
}
