//! Database server: answers byte for byte as rekordbox in capture 1, menus over a small library, and
//! the TCP side (port query, greeting, session) over loopback.

#[path = "fixtures/db.rs"]
mod db;
mod fixtures;

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use fixtures::bytes;
use rbx_link::catalog::Catalog;
use rbx_link::dbserver::{encode, req, Reply, Session};
use rbx_link::server::{DbServer, Shared};
use rekordbox_formats::dbserver::{
    decode_stream, kind, Arg, Message, GREETING, PORT_QUERY_REQUEST,
};
use rekordbox_formats::library::{LibTrack, Source};

fn track(artist: &str, title: &str) -> LibTrack {
    LibTrack {
        anlz_dir: PathBuf::from("does-not-exist"),
        stored_path: format!("C:/Music/{artist} - {title}.mp3"),
        file_name: format!("{artist} - {title}.mp3"),
        path: Some(PathBuf::from(format!("C:/Music/{artist} - {title}.mp3"))),
        candidates: 1,
        artist: Some(artist.into()),
        title: title.into(),
        beats: vec![],
        samples: Some(44100 * 200),
        cues: vec![],
        structure: None,
        key: Some("Am".into()),
        source: Source::Rekordbox,
    }
}

fn catalog() -> Catalog {
    Catalog::from_tracks(vec![
        track("Zed", "Home"),
        track("Abba", "Waterloo"),
        track("Abba", "Dancing Queen"),
        track("Moby", "Porcelain"),
    ])
}

fn decode(hex: &str) -> Message {
    let (mut m, used, err) = decode_stream(&bytes(hex));
    assert!(err.is_none() && used == bytes(hex).len() && m.len() == 1);
    m.remove(0)
}

fn send(s: &mut Session, c: &Catalog, m: &Message) -> Vec<Message> {
    match s.handle(c, 0x11, m) {
        Reply::Send(v) => v,
        Reply::Close => panic!("closed"),
    }
}

fn wire(msgs: &[Message]) -> Vec<u8> {
    msgs.iter().flat_map(encode).collect()
}

#[test]
fn setup_and_root_menu_are_rekordbox_s_bytes() {
    let c = catalog();
    let mut s = Session::new();
    assert_eq!(
        wire(&send(&mut s, &c, &decode(db::PLAYER_ID))),
        bytes(db::PLAYER_ID_REPLY)
    );
    assert_eq!(
        wire(&send(&mut s, &c, &decode(db::ROOT_MENU))),
        bytes(db::ROOT_MENU_REPLY)
    );
    assert_eq!(
        wire(&send(&mut s, &c, &decode(db::LEFT_BUF_ROOT))),
        bytes(db::LEFT_BUF_ROOT_REPLY)
    );
}

fn request(kind: u16, args: Vec<u32>) -> Message {
    Message::new(7, kind, args.into_iter().map(Arg::Num).collect())
}

/// Prepare a menu and fetch `count` entries from `offset`: (total, labels, ids).
fn menu(
    s: &mut Session,
    c: &Catalog,
    m: Message,
    offset: u32,
    count: u32,
) -> (u32, Vec<String>, Vec<u32>) {
    let r = send(s, c, &m);
    assert_eq!(r[0].kind, kind::RET_DWORD);
    let total = r[0].args[1].num().unwrap();
    let page = send(
        s,
        c,
        &request(
            kind::LEFT_BUF,
            vec![0x0b01_0401, offset, count, 0, total, 0],
        ),
    );
    assert_eq!(page[0].kind, kind::RET_LIST_START);
    assert_eq!(page[0].args[1], Arg::Num(offset));
    assert_eq!(page.last().unwrap().kind, kind::RET_LIST_END);
    let rows = &page[1..page.len() - 1];
    let labels = rows
        .iter()
        .map(|r| match &r.args[3] {
            Arg::Str(s) => s.clone(),
            _ => String::new(),
        })
        .collect();
    let ids = rows.iter().map(|r| r.args[1].num().unwrap()).collect();
    (total, labels, ids)
}

#[test]
fn tracks_artists_paging_and_search() {
    let c = catalog();
    let mut s = Session::new();
    let dmst = 0x0b01_0401;
    let (n, titles, _) = menu(&mut s, &c, request(kind::TRACK_ROOT, vec![dmst, 0]), 0, 12);
    assert_eq!(n, 4);
    assert_eq!(titles, ["Dancing Queen", "Home", "Porcelain", "Waterloo"]);
    // second page of two
    let (_, page, _) = menu(&mut s, &c, request(kind::TRACK_ROOT, vec![dmst, 0]), 2, 2);
    assert_eq!(page, ["Porcelain", "Waterloo"]);
    let (_, artists, ids) = menu(&mut s, &c, request(kind::ARTIST_ROOT, vec![dmst, 0]), 0, 12);
    assert_eq!(artists, ["Abba", "Moby", "Zed"]);
    let abba = ids[0];
    let (_, all, all_ids) = menu(
        &mut s,
        &c,
        request(kind::ALBUM_ARTIST, vec![dmst, 0, abba]),
        0,
        12,
    );
    assert_eq!(all, ["\u{FFFA}ALL\u{FFFB}"]);
    let (_, abba_tracks, _) = menu(
        &mut s,
        &c,
        request(kind::TRACK_ALBUM_ARTIST, vec![dmst, 0, abba, all_ids[0]]),
        0,
        12,
    );
    assert_eq!(abba_tracks, ["Dancing Queen", "Waterloo"]);
    let search = Message::new(
        7,
        kind::SEARCH,
        vec![
            Arg::Num(dmst),
            Arg::Num(0),
            Arg::Num(6),
            Arg::Str("PO".into()),
            Arg::Num(1),
        ],
    );
    let (_, found, _) = menu(&mut s, &c, search, 0, 12);
    assert_eq!(found, ["Porcelain"]);
}

#[test]
fn metadata_menu_index_abort_drop_and_unknowns() {
    let c = catalog();
    let mut s = Session::new();
    let (n, _, ids) = menu(
        &mut s,
        &c,
        request(kind::SONGDATA_TRACK, vec![0x0b05_0401, 2]),
        0,
        16,
    );
    assert_eq!(n, 11);
    assert_eq!(ids[3], 200, "duration in seconds at position 3");
    // index of an entry in the prepared root menu: PLAYLIST (5) is the fifth
    send(
        &mut s,
        &c,
        &request(kind::ROOT_MENU, vec![0x0b07_0401, 0, 0x05fd_ffff]),
    );
    let r = send(
        &mut s,
        &c,
        &request(req::MENU_INDEX, vec![0x0b07_0401, 5, 0, 0]),
    );
    assert_eq!(r[0].args, vec![Arg::Num(0x3100), Arg::Num(4)]);
    assert!(send(&mut s, &c, &request(req::ABORT, vec![])).is_empty());
    // HISTORY and the like: empty, like rekordbox
    let r = send(
        &mut s,
        &c,
        &request(req::HISTORY_ROOT, vec![0x0b01_0401, 0]),
    );
    assert_eq!(r[0].args, vec![Arg::Num(0x1012), Arg::Num(0)]);
    let r = send(&mut s, &c, &request(0x7777, vec![]));
    assert_eq!(r[0].args[1], Arg::Num(0));
    let r = send(&mut s, &c, &request(0x2d04, vec![0, 1]));
    assert_eq!(r[0].kind, kind::RET_UNSUPPORTED);
    let r = send(
        &mut s,
        &c,
        &request(kind::GET_IMAGE, vec![0x0b08_0401, 1, 1]),
    );
    assert_eq!(
        r[0].args,
        vec![
            Arg::Num(0x2003),
            Arg::Num(0x32),
            Arg::Num(0),
            Arg::Blob(vec![])
        ]
    );
    assert_eq!(
        s.handle(&c, 0x11, &request(kind::DROP, vec![])),
        Reply::Close
    );
}

fn read_n(s: &mut TcpStream, n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    s.read_exact(&mut b).unwrap();
    b
}

fn port_query(port: u16) -> u16 {
    let mut q = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    // the unit sends the request in two pieces
    q.write_all(&PORT_QUERY_REQUEST[..2]).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    q.write_all(&PORT_QUERY_REQUEST[2..]).unwrap();
    let b = read_n(&mut q, 2);
    u16::from_be_bytes([b[0], b[1]])
}

#[test]
fn tcp_port_query_greeting_and_session() {
    let shared = Arc::new(Shared::default());
    shared.set_subnet(Ipv4Addr::new(127, 0, 0, 1), Ipv4Addr::new(255, 0, 0, 0));
    let (tx, rx) = std::sync::mpsc::channel();
    let srv = DbServer::start(shared.clone(), 0, 0, tx).unwrap();
    // not ready: no library, no number
    assert_eq!(port_query(srv.query_port), 0xFFFF);
    shared.set_catalog(Arc::new(catalog()));
    shared.own.store(0x11, Ordering::Relaxed);
    let port = port_query(srv.query_port);
    assert_eq!(port, srv.db_port);
    let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(&GREETING).unwrap();
    assert_eq!(read_n(&mut s, 5), GREETING);
    s.write_all(&bytes(db::PLAYER_ID)).unwrap();
    assert_eq!(read_n(&mut s, 32), bytes(db::PLAYER_ID_REPLY));
    s.write_all(&bytes(db::ROOT_MENU)).unwrap();
    assert_eq!(read_n(&mut s, 32), bytes(db::ROOT_MENU_REPLY));
    drop(s);
    let kinds: Vec<u16> = rx
        .try_iter()
        .filter_map(|e| match e {
            rbx_link::net::LinkEvent::Db { kind, .. } => Some(kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, [kind::PLAYER_ID, kind::ROOT_MENU]);
    srv.stop();
}

#[test]
fn tcp_outside_the_link_subnet_is_refused() {
    let shared = Arc::new(Shared::default());
    shared.set_subnet(Ipv4Addr::new(169, 254, 1, 1), Ipv4Addr::new(255, 255, 0, 0));
    shared.set_catalog(Arc::new(catalog()));
    shared.own.store(0x11, Ordering::Relaxed);
    let (tx, _rx) = std::sync::mpsc::channel();
    let srv = DbServer::start(shared, 0, 0, tx).unwrap();
    let mut q = TcpStream::connect((Ipv4Addr::LOCALHOST, srv.query_port)).unwrap();
    q.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let _ = q.write_all(PORT_QUERY_REQUEST);
    let mut b = [0u8; 2];
    assert!(q.read_exact(&mut b).is_err(), "no port for strangers");
    srv.stop();
}
