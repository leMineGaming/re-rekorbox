//! Compare our FLAC PVB2 with rekordbox's for a few tracks: `cargo run --release -p rbx-engine --example check_pvb2`.
use rbx_engine::analyze::seek::{flac_frames, flac_pvb2};
use rekordbox_formats::anlz::{tags::Tag, AnlzFile};
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};
fn main() {
    let lib = Library::build(&default_anlz_root().unwrap(), &[default_music_root().unwrap()]);
    let (mut same, mut total) = (0, 0);
    for t in lib.playable().filter(|t| t.path.as_ref().unwrap().extension().is_some_and(|e| e == "flac")).step_by(100).take(15) {
        let b = std::fs::read(t.anlz_dir.join("ANLZ0000.EXT")).unwrap();
        let theirs = AnlzFile::parse(&b).unwrap().tags.iter().find_map(|r| match Tag::from_raw(r) { Ok(Tag::Vbr2(v)) => Some(v), _ => None });
        let Some(v) = theirs else { continue };
        let frames = flac_frames(t.path.as_ref().unwrap()).unwrap();
        let ours = flac_pvb2(&frames, v.sample_count);
        total += 1;
        if ours == v.data { same += 1; }
        let diff = ours.chunks(20).zip(v.data.chunks(20)).position(|(a, b)| a != b);
        println!("{:<50} frames {} identical {} first diff {:?}", t.title, frames.len(), ours == v.data, diff);
        if let Some(d) = diff {
            let be = |e: &[u8]| (u64::from_be_bytes(e[0..8].try_into().unwrap()), u64::from_be_bytes(e[8..16].try_into().unwrap()), u32::from_be_bytes(e[16..20].try_into().unwrap()));
            for k in d.saturating_sub(1)..(d + 2).min(400) {
                println!("    {k}: ours {:?} theirs {:?}", be(&ours[k * 20..k * 20 + 20]), be(&v.data[k * 20..k * 20 + 20]));
            }
            let last = frames.last().unwrap();
            println!("    last frame ours {:?}; samples {}", last, v.sample_count);
        }
    }
    println!("{same} of {total} PVB2 tables byte-identical");
}
