//! Link Export server over loopback with the own client: port query, greeting, setup, menus,
//! metadata, beat grid, waveforms and ANLZ tags of tracks analysed by the own analyzer.

use std::net::{IpAddr, Ipv4Addr};

use rbx_engine::analyze::{analyze_pcm, write_anlz};
use rbx_engine::linkexport::{Catalog, Client, LinkExport};
use rbx_engine::pcm::{write_wav, Pcm};
use rbx_engine::session::Log;
use rekordbox_formats::dbserver::{dmst, item, kind, Arg};
use rekordbox_formats::library::Library;

fn library(dir: &std::path::Path) -> Library {
    let anlz = dir.join("anlz");
    for (name, bpm) in [("Alpha Artist - Zebra.wav", 128.0), ("Beta Artist - Apple.wav", 124.0)] {
        let p = dir.join(name);
        let pcm = Pcm::click_track(44100, bpm, 30.0);
        write_wav(&p, &pcm).unwrap();
        write_anlz(&analyze_pcm(&p, &pcm), &anlz).unwrap();
    }
    Library::build_multi(&[anlz], &[dir.to_path_buf()])
}

#[test]
fn player_browses_and_reads_analysis_data() {
    let dir = std::env::temp_dir().join(format!("rbx-linkexport-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let lib = library(&dir);
    let server = LinkExport::start(Catalog::from_library(&lib), 17, 0, Log::default()).unwrap();
    assert_eq!(server.track_count(), 2);
    let mut c = Client::connect(IpAddr::V4(Ipv4Addr::LOCALHOST), server.query_port, 2).unwrap();
    let d = dmst(2, 1, 4, 1);

    // root menu
    let r = c.request(kind::ROOT_MENU, vec![Arg::Num(d), Arg::Num(0)]).unwrap();
    assert_eq!((r.kind, r.args.clone()), (kind::RET_DWORD, vec![Arg::Num(0x1000), Arg::Num(4)]));
    let items = c.render(d, 4).unwrap();
    assert!(items.iter().all(|m| m.kind == kind::RET_LIST_SET && m.args.len() == 12));
    assert_eq!(items[1].args[3], Arg::Str("TRACK".into()));

    // all tracks, sorted by title: Apple (id 1), Zebra (id 2)
    let r = c.request(kind::TRACK_ROOT, vec![Arg::Num(d), Arg::Num(0)]).unwrap();
    assert_eq!(r.args[1], Arg::Num(2));
    let tracks = c.render(d, 2).unwrap();
    assert_eq!(tracks[0].args[3], Arg::Str("Apple".into()));
    assert_eq!(tracks[0].args[5], Arg::Str("Beta Artist".into()));
    assert_eq!(tracks[0].args[6], Arg::Num(item::TITLE_ARTIST));
    let apple = tracks[0].args[1].num().unwrap();

    // metadata of a track
    let r = c.request(kind::SONGDATA_TRACK, vec![Arg::Num(d), Arg::Num(apple)]).unwrap();
    assert_eq!(r.args[1], Arg::Num(11));
    let meta = c.render(d, 11).unwrap();
    assert_eq!(meta[0].args[3], Arg::Str("Apple".into()));
    let tempo = meta.iter().find(|m| m.args[6] == Arg::Num(item::TEMPO)).unwrap();
    assert!((tempo.args[1].num().unwrap() as i64 - 12400).abs() <= 2, "{:?}", tempo.args[1]);
    assert_eq!(meta.iter().find(|m| m.args[6] == Arg::Num(item::DURATION)).unwrap().args[1], Arg::Num(30));
    // unknown track
    let r = c.request(kind::SONGDATA_TRACK, vec![Arg::Num(d), Arg::Num(99)]).unwrap();
    assert_eq!(r.args[1], Arg::Num(0xFFFF_FFFF));

    // beat grid: 20 bytes header + 16 bytes per beat, beats 1-4 at 124 BPM
    let d8 = dmst(2, 8, 4, 1);
    let r = c.request(kind::GET_QUANTIZE_DATA, vec![Arg::Num(d8), Arg::Num(apple)]).unwrap();
    assert_eq!(r.kind, kind::RET_QTZ);
    let Arg::Blob(grid) = &r.args[3] else { panic!() };
    assert_eq!((grid.len() - 20) % 16, 0);
    let first_tempo = u16::from_le_bytes([grid[22], grid[23]]);
    assert!((i32::from(first_tempo) - 12400).abs() <= 2);
    assert!((grid.len() - 20) / 16 > 55, "about 62 beats in 30 s");

    // waveform preview (900 bytes) and detail (150 per second)
    let r = c.request(kind::GET_WAVEDATA, vec![Arg::Num(d8), Arg::Num(4), Arg::Num(apple), Arg::Num(0)]).unwrap();
    let Arg::Blob(prev) = &r.args[3] else { panic!() };
    assert_eq!(prev.len(), 900);
    let r = c.request(kind::GET_PAR_WAVE, vec![Arg::Num(d8), Arg::Num(apple), Arg::Num(0)]).unwrap();
    let Arg::Blob(detail) = &r.args[3] else { panic!() };
    assert_eq!(detail.len(), 30 * 150);

    // colour detail waveform as ANLZ tag (PWV5 in .EXT)
    let r = c.request(kind::GET_ATOM_INFO, vec![Arg::Num(d8), Arg::Num(apple), Arg::Num(0x3556_5750), Arg::Num(0x0054_5845)]).unwrap();
    assert_eq!(r.kind, kind::RET_ATOM);
    let Arg::Blob(tag) = &r.args[3] else { panic!() };
    assert_eq!(&tag[..4], b"PWV5");
    assert!(tag.len() > 30 * 150 * 2);

    // artists → tracks of an artist; search
    let r = c.request(kind::ARTIST_ROOT, vec![Arg::Num(d), Arg::Num(0)]).unwrap();
    assert_eq!(r.args[1], Arg::Num(2));
    let artists = c.render(d, 2).unwrap();
    let alpha = artists.iter().find(|m| m.args[3] == Arg::Str("Alpha Artist".into())).unwrap().args[1].num().unwrap();
    let r = c.request(kind::TRACK_ALBUM_ARTIST, vec![Arg::Num(d), Arg::Num(0), Arg::Num(alpha), Arg::Num(0xFFFF_FFFF)]).unwrap();
    assert_eq!(r.args[1], Arg::Num(1));
    assert_eq!(c.render(d, 1).unwrap()[0].args[3], Arg::Str("Zebra".into()));
    let r = c.request(kind::SEARCH, vec![Arg::Num(d), Arg::Num(0), Arg::Num(8), Arg::Str("zeb".into())]).unwrap();
    assert_eq!(r.args[1], Arg::Num(1));

    // something the server does not know
    let r = c.request(0x2402, vec![Arg::Num(d8), Arg::Num(apple)]).unwrap();
    assert_eq!(r.kind, kind::RET_UNSUPPORTED);
    assert!(server.connections.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    drop(c);
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
