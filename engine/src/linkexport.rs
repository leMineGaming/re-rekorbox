//! Link Export server (SPEC §5k): offers the own library to DJ players on the Pro DJ Link network
//! like rekordbox does – port query on TCP 12523, database connection with menus (root, tracks,
//! artists, keys, search), track metadata and analysis data (beat grid, waveforms, ANLZ tags) from
//! the analysis files.
//!
//! Not yet implemented: the audio file transfer (NFS) – how the player learns the file path is open
//! (SPEC U33) and needs a capture of rekordbox ↔ player traffic (`Link-Mitschnitt.cmd`). Until then a
//! player can browse and see waveforms but not play the track.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use rekordbox_formats::anlz::{AnlzFile, FourCC};
use rekordbox_formats::dbserver::{self, item, kind, menu_item, Arg, Message};
use rekordbox_formats::library::{LibTrack, Library};

use crate::session::Log;

/// A snapshot of the library as the player sees it: tracks with ids 1…n, artists and keys with ids.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub tracks: Vec<LibTrack>,
    artists: BTreeMap<String, u32>,
    keys: BTreeMap<String, u32>,
}

impl Catalog {
    /// Playable tracks sorted by title (ids are positions + 1).
    pub fn from_library(lib: &Library) -> Catalog {
        let mut tracks: Vec<LibTrack> = lib.playable().cloned().collect();
        tracks.sort_by_cached_key(|t| t.title.to_lowercase());
        let mut artists = BTreeMap::new();
        let mut keys = BTreeMap::new();
        for t in &tracks {
            let a = t.artist.clone().unwrap_or_default();
            let n = artists.len() as u32 + 1;
            artists.entry(a).or_insert(n);
            if let Some(k) = &t.key {
                let n = keys.len() as u32 + 1;
                keys.entry(k.clone()).or_insert(n);
            }
        }
        Catalog { tracks, artists, keys }
    }

    pub fn track(&self, id: u32) -> Option<&LibTrack> {
        self.tracks.get((id as usize).checked_sub(1)?)
    }

    fn artist_id(&self, t: &LibTrack) -> u32 {
        self.artists.get(t.artist.as_deref().unwrap_or("")).copied().unwrap_or(0)
    }

    fn key_id(&self, t: &LibTrack) -> u32 {
        t.key.as_ref().and_then(|k| self.keys.get(k)).copied().unwrap_or(0)
    }
}

/// One entry of a prepared menu (sent on `CMD_LEFT_BUF`).
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub parent: u32,
    pub id: u32,
    pub label1: String,
    pub label2: String,
    pub item_type: u32,
}

fn entry(parent: u32, id: u32, label1: &str, label2: &str, item_type: u32) -> Entry {
    Entry { parent, id, label1: label1.into(), label2: label2.into(), item_type }
}

/// Menu item types of the root menu entries (player menus, SPEC §5k 0x0080…).
const MENU_ARTIST: u32 = 0x0081;
const MENU_TRACK: u32 = 0x0083;
const MENU_KEY: u32 = 0x008b;
const MENU_SEARCH: u32 = 0x0091;

fn track_entries<'a>(c: &Catalog, it: impl Iterator<Item = (u32, &'a LibTrack)>) -> Vec<Entry> {
    it.map(|(id, t)| entry(c.artist_id(t), id, &t.title, t.artist.as_deref().unwrap_or(""), item::TITLE_ARTIST)).collect()
}

/// Duration in seconds: sample count, else the detail waveform (150 columns per second), else the
/// last beat.
fn duration_s(t: &LibTrack) -> u32 {
    if let Some(d) = t.duration_s(44100) {
        return d.round() as u32;
    }
    if let Some(w) = anlz_tag(t, FourCC::PWV3, "EXT") {
        return (w.body.len() as f64 / 150.0).round() as u32;
    }
    t.beats.last().map_or(0, |b| b.time_ms / 1000)
}

/// Answer to a menu request: the entries, or `None` when the request is not a menu.
pub fn menu(c: &Catalog, m: &Message) -> Option<Vec<Entry>> {
    let arg = |i: usize| m.args.get(i).and_then(Arg::num).unwrap_or(0);
    Some(match m.kind {
        kind::ROOT_MENU => vec![
            entry(0, u32::from(kind::ARTIST_ROOT), "ARTIST", "", MENU_ARTIST),
            entry(0, u32::from(kind::TRACK_ROOT), "TRACK", "", MENU_TRACK),
            entry(0, u32::from(kind::KEY_ROOT), "KEY", "", MENU_KEY),
            entry(0, u32::from(kind::SEARCH), "SEARCH", "", MENU_SEARCH),
        ],
        kind::TRACK_ROOT => track_entries(c, c.tracks.iter().enumerate().map(|(i, t)| (i as u32 + 1, t))),
        kind::ARTIST_ROOT => c.artists.iter().map(|(name, id)| entry(0, *id, if name.is_empty() { "(unbekannt)" } else { name }, "", item::ARTIST)).collect(),
        // the library has no albums: one "all tracks" album per artist
        kind::ALBUM_ARTIST => vec![entry(arg(2), 0xFFFF_FFFF, "(alle)", "", item::ALBUM)],
        kind::TRACK_ALBUM_ARTIST | 0xF001 => {
            let artist = arg(2);
            track_entries(c, c.tracks.iter().enumerate().filter(|(_, t)| c.artist_id(t) == artist).map(|(i, t)| (i as u32 + 1, t)))
        }
        kind::KEY_ROOT => c.keys.iter().map(|(k, id)| entry(0, *id, k, "", item::KEY)).collect(),
        kind::TRACK_KEY => {
            let key = arg(2);
            track_entries(c, c.tracks.iter().enumerate().filter(|(_, t)| c.key_id(t) == key).map(|(i, t)| (i as u32 + 1, t)))
        }
        kind::SEARCH => {
            let text = m.args.iter().find_map(|a| if let Arg::Str(s) = a { Some(s.to_lowercase()) } else { None }).unwrap_or_default();
            track_entries(
                c,
                c.tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| format!("{} {}", t.title, t.artist.as_deref().unwrap_or("")).to_lowercase().contains(&text))
                    .map(|(i, t)| (i as u32 + 1, t)),
            )
        }
        _ => return None,
    })
}

/// Metadata entries of a track (`CMD_SONGDATA_TRACK`, rendered with `CMD_LEFT_BUF`).
pub fn metadata(c: &Catalog, id: u32) -> Option<Vec<Entry>> {
    let t = c.track(id)?;
    let bpm100 = t.bpm().map_or(0, |b| (b * 100.0).round() as u32);
    Some(vec![
        entry(c.artist_id(t), id, &t.title, "", item::TITLE),
        entry(0, c.artist_id(t), t.artist.as_deref().unwrap_or(""), "", item::ARTIST),
        entry(0, 0, "", "", item::ALBUM),
        entry(0, duration_s(t), "", "", item::DURATION),
        entry(0, bpm100, "", "", item::TEMPO),
        entry(0, 0, "", "", item::COMMENT),
        entry(0, c.key_id(t), t.key.as_deref().unwrap_or(""), "", item::KEY),
        entry(0, 0, "", "", item::RATING),
        entry(0, 0, "", "", item::COLOR_NONE),
        entry(0, 0, "", "", item::GENRE),
        entry(0, 0, "", "", item::DATE_ADDED),
    ])
}

/// A tag from the track's analysis files (`ext` = `DAT`, `EXT` or `2EX`).
pub fn anlz_tag(t: &LibTrack, fourcc: FourCC, ext: &str) -> Option<rekordbox_formats::anlz::RawTag> {
    let bytes = std::fs::read(t.anlz_dir.join(format!("ANLZ0000.{ext}"))).ok()?;
    AnlzFile::parse(&bytes).ok()?.tags.into_iter().find(|x| x.fourcc == fourcc)
}

/// Beat grid blob: 20 bytes header, then 16 bytes per beat (beat in bar, BPM × 100, ms; little-endian).
pub fn beat_grid_blob(t: &LibTrack) -> Vec<u8> {
    let mut b = vec![0u8; 20];
    for beat in &t.beats {
        b.extend_from_slice(&beat.beat.to_le_bytes());
        b.extend_from_slice(&beat.tempo_x100.to_le_bytes());
        b.extend_from_slice(&beat.time_ms.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
    }
    b
}

/// Waveform preview blob: 400 columns × (height, whiteness), then the 100-byte tiny preview.
pub fn preview_blob(t: &LibTrack) -> Vec<u8> {
    let mut b = Vec::with_capacity(900);
    if let Some(p) = anlz_tag(t, FourCC::PWAV, "DAT") {
        for &x in p.body.iter().take(400) {
            b.push(x & 0x1F);
            b.push(x >> 5);
        }
    }
    b.resize(800, 0);
    if let Some(p) = anlz_tag(t, FourCC::PWV2, "DAT") {
        b.extend(p.body.iter().take(100));
    }
    b.resize(900, 0);
    b
}

/// The database connection loop: greeting, then requests until the player hangs up.
fn serve(mut s: TcpStream, catalog: Arc<RwLock<Catalog>>, own_number: u8, log: Log, requests: Requests) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut greet = [0u8; 5];
    s.read_exact(&mut greet)?;
    s.write_all(&greet)?;
    let peer = s.peer_addr().ok();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut pending: Vec<Entry> = Vec::new();
    loop {
        let (msgs, used, err) = dbserver::decode_stream(&buf);
        if let Some(e) = err {
            log.say(format!("Link Export: Protokollfehler von {peer:?}: {e}"));
            return Ok(());
        }
        buf.drain(..used);
        for m in msgs {
            log.say(format!("Link Export <- {peer:?}{}", rekordbox_formats::capture::describe(&m)));
            *requests.lock().unwrap().entry(m.kind).or_default() += 1;
            let replies = handle(&m, &catalog.read().unwrap(), own_number, &mut pending);
            if m.kind == kind::DROP {
                return Ok(());
            }
            for r in replies {
                s.write_all(&r.encode())?;
            }
        }
        let n = s.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Answer one request (pure function of the catalog; `pending` holds the last prepared menu).
pub fn handle(m: &Message, c: &Catalog, own_number: u8, pending: &mut Vec<Entry>) -> Vec<Message> {
    let tx = m.tx;
    let arg = |i: usize| m.args.get(i).and_then(Arg::num).unwrap_or(0);
    let ok = |count: u32| Message::new(tx, kind::RET_DWORD, vec![Arg::Num(u32::from(m.kind)), Arg::Num(count)]);
    let blob = |ret: u16, data: Vec<u8>, extra: Option<u32>| {
        let mut a = vec![Arg::Num(u32::from(m.kind)), Arg::Num(0), Arg::Num(data.len() as u32), Arg::Blob(data)];
        if let Some(e) = extra {
            a.push(Arg::Num(e));
        }
        Message::new(tx, ret, a)
    };
    match m.kind {
        kind::PLAYER_ID => vec![Message::new(tx, kind::RET_DWORD, vec![Arg::Num(0), Arg::Num(u32::from(own_number))])],
        kind::DROP => vec![],
        kind::LEFT_BUF => {
            let (offset, limit) = (arg(1) as usize, arg(2) as usize);
            let mut out = vec![Message::new(tx, kind::RET_LIST_START, vec![Arg::Num(1), Arg::Num(0)])];
            for (i, e) in pending.iter().enumerate().skip(offset).take(limit.max(1)) {
                out.push(menu_item(tx, e.parent, e.id, &e.label1, &e.label2, e.item_type, 0, i as u32 + 1));
            }
            out.push(Message::new(tx, kind::RET_LIST_END, vec![]));
            out
        }
        kind::SONGDATA_TRACK => match metadata(c, arg(1)) {
            Some(items) => {
                let n = items.len() as u32;
                *pending = items;
                vec![ok(n)]
            }
            None => vec![ok(0xFFFF_FFFF)],
        },
        kind::GET_QUANTIZE_DATA => vec![blob(kind::RET_QTZ, c.track(arg(1)).map(beat_grid_blob).unwrap_or_default(), Some(0))],
        kind::GET_WAVEDATA => vec![blob(kind::RET_WAVE, c.track(arg(2)).map(preview_blob).unwrap_or_default(), None)],
        kind::GET_PAR_WAVE => {
            let data = c.track(arg(1)).and_then(|t| anlz_tag(t, FourCC::PWV3, "EXT")).map(|t| t.body).unwrap_or_default();
            vec![blob(kind::RET_PAR_WAVE, data, None)]
        }
        kind::GET_ATOM_INFO => {
            let fourcc = FourCC(arg(2).to_le_bytes());
            let ext = String::from_utf8_lossy(&arg(3).to_le_bytes()).trim_end_matches('\0').to_string();
            let mut data = Vec::new();
            if let Some(tag) = c.track(arg(1)).and_then(|t| anlz_tag(t, fourcc, &ext)) {
                tag.write_to(&mut data);
            }
            vec![blob(kind::RET_ATOM, data, Some(0))]
        }
        kind::GET_USB_CUE => vec![Message::new(
            tx,
            kind::RET_CUE,
            vec![Arg::Num(u32::from(m.kind)), Arg::Num(0), Arg::Num(0), Arg::Blob(vec![]), Arg::Num(0x24), Arg::Num(0), Arg::Num(0), Arg::Num(0), Arg::Blob(vec![])],
        )],
        kind::GET_USB_CUE2 => vec![blob(kind::RET_CUE2, Vec::new(), Some(0))],
        kind::GET_IMAGE => vec![blob(kind::RET_IMAGE, Vec::new(), None)],
        _ => match menu(c, m) {
            Some(items) => {
                let n = items.len() as u32;
                *pending = items;
                vec![ok(n)]
            }
            None => vec![Message::new(tx, kind::RET_UNSUPPORTED, vec![Arg::Num(u32::from(m.kind))])],
        },
    }
}

/// Requests received per message type.
pub type Requests = Arc<std::sync::Mutex<BTreeMap<u16, usize>>>;

/// The running server.
pub struct LinkExport {
    /// Requests per message type (for diagnostics).
    pub requests: Requests,
    /// Port queries answered.
    pub port_queries: Arc<AtomicUsize>,
    pub db_port: u16,
    /// Port of the port query (12523 unless another was asked for, e.g. 0 in tests).
    pub query_port: u16,
    stop: Arc<AtomicBool>,
    pub connections: Arc<AtomicUsize>,
    catalog: Arc<RwLock<Catalog>>,
}

impl LinkExport {
    /// Start the port query listener (`query_port`, normally 12523) and the database server.
    pub fn start(catalog: Catalog, own_number: u8, query_port: u16, log: Log) -> std::io::Result<LinkExport> {
        let db = TcpListener::bind(("0.0.0.0", 0))?;
        let db_port = db.local_addr()?.port();
        let query = TcpListener::bind(("0.0.0.0", query_port))?;
        let query_port = query.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(AtomicUsize::new(0));
        let catalog = Arc::new(RwLock::new(catalog));
        let requests: Requests = Default::default();
        let port_queries = Arc::new(AtomicUsize::new(0));
        log.say(format!("Link Export: Datenbank-Server auf Port {db_port}, Port-Abfrage auf {query_port}"));
        {
            let (stop, log, pq) = (stop.clone(), log.clone(), port_queries.clone());
            query.set_nonblocking(true)?;
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match query.accept() {
                        Ok((mut s, _)) => {
                            let _ = s.set_nonblocking(false);
                            let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                            let mut req = [0u8; 19];
                            if s.read_exact(&mut req).is_ok() && req.as_slice() == dbserver::PORT_QUERY_REQUEST {
                                log.say(format!("Link Export: Port-Abfrage von {:?}", s.peer_addr().ok()));
                                pq.fetch_add(1, Ordering::Relaxed);
                                let _ = s.write_all(&db_port.to_be_bytes());
                            } else {
                                log.say("Link Export: unbekannte Port-Abfrage");
                            }
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
            });
        }
        {
            let (stop, conns, cat, reqs) = (stop.clone(), connections.clone(), catalog.clone(), requests.clone());
            db.set_nonblocking(true)?;
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match db.accept() {
                        Ok((s, addr)) => {
                            let _ = s.set_nonblocking(false);
                            conns.fetch_add(1, Ordering::Relaxed);
                            log.say(format!("Link Export: Verbindung von {addr}"));
                            let (cat, log, reqs) = (cat.clone(), log.clone(), reqs.clone());
                            std::thread::spawn(move || {
                                if let Err(e) = serve(s, cat, own_number, log.clone(), reqs) {
                                    log.say(format!("Link Export: Verbindung {addr} beendet: {e}"));
                                }
                            });
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(50)),
                    }
                }
            });
        }
        Ok(LinkExport { requests, port_queries, db_port, query_port, stop, connections, catalog })
    }

    /// Replace the offered library (e.g. after new analyses).
    pub fn update(&self, catalog: Catalog) {
        *self.catalog.write().unwrap() = catalog;
    }

    pub fn track_count(&self) -> usize {
        self.catalog.read().unwrap().tracks.len()
    }
}

impl Drop for LinkExport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Minimal client (for tests and the self test): ask for the port, connect, set up as `player`.
pub struct Client {
    s: TcpStream,
    tx: u32,
    buf: Vec<u8>,
}

impl Client {
    pub fn connect(addr: std::net::IpAddr, query_port: u16, player: u8) -> std::io::Result<Client> {
        let mut q = TcpStream::connect_timeout(&SocketAddr::new(addr, query_port), Duration::from_secs(3))?;
        q.write_all(dbserver::PORT_QUERY_REQUEST)?;
        let mut p = [0u8; 2];
        q.read_exact(&mut p)?;
        let mut s = TcpStream::connect_timeout(&SocketAddr::new(addr, u16::from_be_bytes(p)), Duration::from_secs(3))?;
        s.set_read_timeout(Some(Duration::from_secs(5)))?;
        s.write_all(&dbserver::GREETING)?;
        let mut g = [0u8; 5];
        s.read_exact(&mut g)?;
        let mut c = Client { s, tx: 0, buf: Vec::new() };
        c.send(Message::new(dbserver::SETUP_TX, kind::PLAYER_ID, vec![Arg::Num(u32::from(player))]))?;
        c.recv(1)?;
        Ok(c)
    }

    fn send(&mut self, m: Message) -> std::io::Result<()> {
        self.s.write_all(&m.encode())
    }

    /// Read `n` messages.
    pub fn recv(&mut self, n: usize) -> std::io::Result<Vec<Message>> {
        let mut out = Vec::new();
        let mut chunk = [0u8; 8192];
        while out.len() < n {
            let (msgs, used, err) = dbserver::decode_stream(&self.buf);
            if let Some(e) = err {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
            self.buf.drain(..used);
            out.extend(msgs);
            if out.len() >= n {
                break;
            }
            let k = self.s.read(&mut chunk)?;
            if k == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            self.buf.extend_from_slice(&chunk[..k]);
        }
        Ok(out)
    }

    /// Send a request and return the first answer.
    pub fn request(&mut self, kind: u16, args: Vec<Arg>) -> std::io::Result<Message> {
        self.tx += 1;
        self.send(Message::new(self.tx, kind, args))?;
        Ok(self.recv(1)?.remove(0))
    }

    /// Render the prepared menu: header, items, footer.
    pub fn render(&mut self, dmst: u32, count: u32) -> std::io::Result<Vec<Message>> {
        self.tx += 1;
        let args = vec![Arg::Num(dmst), Arg::Num(0), Arg::Num(count), Arg::Num(0), Arg::Num(count), Arg::Num(0)];
        self.send(Message::new(self.tx, kind::LEFT_BUF, args))?;
        let msgs = self.recv(count as usize + 2)?;
        Ok(msgs[1..msgs.len() - 1].to_vec())
    }
}

/// Library snapshot for the server from the loader's library.
pub fn catalog_of(lib: Option<&Library>) -> Catalog {
    lib.map(Catalog::from_library).unwrap_or_default()
}

/// Read only: file name of a track for logs.
pub fn track_name(t: &LibTrack) -> String {
    Path::new(&t.stored_path).file_name().map_or(t.title.clone(), |n| n.to_string_lossy().into_owned())
}
