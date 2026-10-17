//! Dump PVBR / PVB2 of one FLAC and one MP3 analysis: `cargo run --example vbr_dump`.
use rekordbox_formats::anlz::{tags::Tag, AnlzFile};
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};
fn main() {
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    for ext in ["flac", "mp3", "m4a"] {
        let Some(t) = lib.playable().filter(|t| t.anlz_dir.join("ANLZ0000.EXT").exists() && !t.title.starts_with("Demo")).find(|t| t.path.as_ref().unwrap().extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))) else { continue };
        println!("== {ext}: {}", t.title);
        for f in ["DAT", "EXT"] {
            let b = std::fs::read(t.anlz_dir.join(format!("ANLZ0000.{f}"))).unwrap();
            let a = AnlzFile::parse(&b).unwrap();
            for r in &a.tags {
                match Tag::from_raw(r) {
                    Ok(Tag::Vbr(v)) => { let tb = v.table(); println!("  PVBR table[0..6]={:?} [200]={} [399]={} extra={}", &tb[..6], tb[200], tb[399], v.extra()); }
                    Ok(Tag::Vbr2(v)) => {
                        println!("  PVB2 samples={} entry_size={} u0e={} len={}", v.sample_count, v.entry_size, v.unknown_0e, v.data.len());
                        for i in [0usize, 1, 2, 100, 200, 399] {
                            let e = &v.data[i * 20..i * 20 + 20];
                            let w: Vec<u32> = e.chunks(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
                            println!("    entry {i}: {w:?}");
                        }
                    }
                    _ => {}
                }
            }
        }
        let size = std::fs::metadata(t.path.as_ref().unwrap()).map(|m| m.len()).unwrap_or(0);
        println!("  file size {size}");
    }
}
