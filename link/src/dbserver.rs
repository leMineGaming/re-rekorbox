//! The database server side as rekordbox answers the XDJ-RX3 (capture 1, `docs/link/PHASE4.md`):
//! a pure session – one request in, the answers out. Menus are prepared by a menu request (answer:
//! number of entries) and fetched in pages with `CMD_LEFT_BUF` (offset, count).
//!
//! Differences to the CDJ protocol notes in `rekordbox_formats::dbserver`: rekordbox sends the
//! argument-type blob only as long as there are arguments (`encode`), wraps menu labels in
//! U+FFFA/U+FFFB, and its root menu has nine entries whose ids are the menu request types
//! (`0x1000 + id`, PLAYLIST = `0x1105`, SEARCH = `0x1300`).

use rekordbox_formats::anlz::FourCC;
use rekordbox_formats::dbserver::{self as wire, kind, Arg, Message, MAGIC};

use crate::catalog::{self, Catalog};

/// Request types not in `rekordbox_formats::dbserver::kind` (seen in capture 1).
pub mod req {
    /// Abort of the transaction with the same id; no answer.
    pub const ABORT: u16 = 0x0001;
    pub const KEY_ROOT: u16 = 0x100c;
    pub const HISTORY_ROOT: u16 = 0x1012;
    pub const DATE_ADDED_ROOT: u16 = 0x1708;
    pub const DATE_ADDED_MONTH: u16 = 0x1808;
    pub const SONGDATA_TRACK2: u16 = 0x2102;
    pub const MATCHING: u16 = 0x3007;
    /// Index of an entry (by id) in the prepared menu.
    pub const MENU_INDEX: u16 = 0x3100;
    pub const RELATED: u16 = 0x3d03;
}

/// Item types of menu entries (argument 7).
pub mod item {
    pub const PATH: u32 = 0x0000;
    pub const ALBUM: u32 = 0x0002;
    pub const TITLE: u32 = 0x0004;
    pub const GENRE: u32 = 0x0006;
    pub const ARTIST: u32 = 0x0007;
    pub const RATING: u32 = 0x000a;
    pub const DURATION: u32 = 0x000b;
    pub const TEMPO: u32 = 0x000d;
    pub const KEY: u32 = 0x000f;
    pub const COLOR_NONE: u32 = 0x0013;
    pub const COMMENT: u32 = 0x0023;
    pub const DATE_ADDED: u32 = 0x002e;
    pub const FLAG_2F: u32 = 0x002f;
    pub const ALL: u32 = 0x00a0;
}

/// One menu entry with all twelve fields of `CMD_RET_LISTBUF_SET`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub parent: u32,
    pub id: u32,
    pub label1: String,
    pub label2: String,
    pub item_type: u32,
    pub flags: u32,
    pub artwork: u32,
    pub a9: u32,
    pub a10: u32,
}

impl Row {
    pub fn new(parent: u32, id: u32, label1: &str, item_type: u32) -> Row {
        Row {
            parent,
            id,
            label1: label1.into(),
            label2: String::new(),
            item_type,
            flags: 0,
            artwork: 0,
            a9: 0,
            a10: 0,
        }
    }

    fn message(&self, tx: u32) -> Message {
        // byte length of the UTF-16 string including NUL
        let len = |s: &str| (s.encode_utf16().count() as u32 + 1) * 2;
        Message::new(
            tx,
            kind::RET_LIST_SET,
            vec![
                Arg::Num(self.parent),
                Arg::Num(self.id),
                Arg::Num(len(&self.label1)),
                Arg::Str(self.label1.clone()),
                Arg::Num(len(&self.label2)),
                Arg::Str(self.label2.clone()),
                Arg::Num(self.item_type),
                Arg::Num(self.flags),
                Arg::Num(self.artwork),
                Arg::Num(self.a9),
                Arg::Num(self.a10),
                Arg::Num(0),
            ],
        )
    }
}

/// Label of a menu entry as rekordbox writes it.
fn menu_label(s: &str) -> String {
    format!("\u{FFFA}{s}\u{FFFB}")
}

/// The root menu: (id, label, item type) – id = menu request type − 0x1000.
pub const ROOT: [(u32, &str, u32); 9] = [
    (0x02, "ARTIST", 0x81),
    (0x03, "ALBUM", 0x82),
    (0x04, "TRACK", 0x83),
    (0x0c, "KEY", 0x8b),
    (0x05, "PLAYLIST", 0x84),
    (0x16, "HISTORY", 0x95),
    (0x12, "SEARCH", 0x91),
    (0x1a, "MATCHING", 0xaa),
    (0x1b, "DATE ADDED", 0x8c),
];

/// Encode like rekordbox: the argument-type blob has one byte per argument.
pub fn encode(m: &Message) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    wire::put_u32(&mut out, MAGIC);
    wire::put_u32(&mut out, m.tx);
    out.push(wire::TAG_U16);
    out.extend_from_slice(&m.kind.to_be_bytes());
    out.push(wire::TAG_U8);
    out.push(m.args.len() as u8);
    out.push(wire::TAG_BLOB);
    out.extend_from_slice(&(m.args.len() as u32).to_be_bytes());
    for a in &m.args {
        out.push(match a {
            Arg::Num(_) => 0x06,
            Arg::Str(_) => 0x02,
            Arg::Blob(_) => 0x03,
        });
    }
    for a in &m.args {
        match a {
            Arg::Num(n) => wire::put_u32(&mut out, *n),
            Arg::Str(s) => {
                let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
                out.push(wire::TAG_STRING);
                out.extend_from_slice(&(units.len() as u32).to_be_bytes());
                for u in units {
                    out.extend_from_slice(&u.to_be_bytes());
                }
            }
            // an empty blob is not transmitted (its length argument is 0)
            Arg::Blob(b) if b.is_empty() => {}
            Arg::Blob(b) => {
                out.push(wire::TAG_BLOB);
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
                out.extend_from_slice(b);
            }
        }
    }
    out
}

/// What to do after a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Send(Vec<Message>),
    /// The player ends the connection (`CMD_INFO_DROP`).
    Close,
}

/// One database connection (the XDJ-RX3 opens one per deck).
#[derive(Debug, Default)]
pub struct Session {
    /// The last prepared menu.
    pending: Vec<Row>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    /// Answer one request. `own` is our device number.
    pub fn handle(&mut self, c: &Catalog, own: u8, m: &Message) -> Reply {
        let tx = m.tx;
        let arg = |i: usize| m.args.get(i).and_then(Arg::num).unwrap_or(0);
        let dword =
            |a: u32, b: u32| Message::new(tx, kind::RET_DWORD, vec![Arg::Num(a), Arg::Num(b)]);
        let blob = |ret: u16, data: Vec<u8>, extra: Option<u32>| {
            let mut a = vec![
                Arg::Num(u32::from(m.kind)),
                Arg::Num(0),
                Arg::Num(data.len() as u32),
                Arg::Blob(data),
            ];
            a.extend(extra.map(Arg::Num));
            Message::new(tx, ret, a)
        };
        let k = m.kind;
        let out = match k {
            kind::PLAYER_ID => vec![dword(0, u32::from(own))],
            kind::DROP => return Reply::Close,
            req::ABORT => vec![],
            kind::LEFT_BUF => {
                let (offset, count) = (arg(1) as usize, arg(2) as usize);
                let mut out = vec![Message::new(
                    tx,
                    kind::RET_LIST_START,
                    vec![Arg::Num(1), Arg::Num(offset as u32)],
                )];
                out.extend(
                    self.pending
                        .iter()
                        .skip(offset)
                        .take(count)
                        .map(|r| r.message(tx)),
                );
                out.push(Message::new(tx, kind::RET_LIST_END, vec![]));
                out
            }
            req::MENU_INDEX => {
                let idx = self
                    .pending
                    .iter()
                    .position(|r| r.id == arg(1))
                    .unwrap_or(0);
                vec![dword(u32::from(k), idx as u32)]
            }
            kind::GET_IMAGE => vec![Message::new(
                tx,
                kind::RET_IMAGE,
                vec![
                    Arg::Num(u32::from(k)),
                    Arg::Num(0x32),
                    Arg::Num(0),
                    Arg::Blob(vec![]),
                ],
            )],
            kind::GET_QUANTIZE_DATA => vec![blob(
                kind::RET_QTZ,
                c.track(arg(1))
                    .map(catalog::beat_grid_blob)
                    .unwrap_or_default(),
                Some(0),
            )],
            kind::GET_WAVEDATA => vec![blob(
                kind::RET_WAVE,
                c.track(arg(2))
                    .map(catalog::preview_blob)
                    .unwrap_or_default(),
                None,
            )],
            kind::GET_PAR_WAVE => {
                let data = c
                    .track(arg(1))
                    .and_then(|t| catalog::anlz_tag(t, FourCC::PWV3, "EXT"))
                    .map(|t| t.body)
                    .unwrap_or_default();
                vec![blob(kind::RET_PAR_WAVE, data, None)]
            }
            kind::GET_ATOM_INFO => {
                let fourcc = FourCC(arg(2).to_le_bytes());
                let ext = String::from_utf8_lossy(&arg(3).to_le_bytes())
                    .trim_end_matches('\0')
                    .to_string();
                let mut data = Vec::new();
                if let Some(tag) = c
                    .track(arg(1))
                    .and_then(|t| catalog::anlz_tag(t, fourcc, &ext))
                {
                    tag.write_to(&mut data);
                }
                vec![blob(kind::RET_ATOM, data, Some(0))]
            }
            kind::GET_USB_CUE2 => vec![blob(kind::RET_CUE2, Vec::new(), Some(0))],
            _ => match self.menu(c, m) {
                Some(rows) => {
                    let n = rows.len() as u32;
                    self.pending = rows;
                    vec![dword(u32::from(k), n)]
                }
                // other data requests: not offered (the player falls back)
                None if (0x2000..0x3000).contains(&k) => vec![Message::new(
                    tx,
                    kind::RET_UNSUPPORTED,
                    vec![Arg::Num(u32::from(k))],
                )],
                // unknown menus: empty, as rekordbox answers HISTORY without entries
                None => {
                    self.pending.clear();
                    vec![dword(u32::from(k), 0)]
                }
            },
        };
        Reply::Send(out)
    }

    /// Rows of a menu request, `None` if `m` is not one we know.
    fn menu(&self, c: &Catalog, m: &Message) -> Option<Vec<Row>> {
        let arg = |i: usize| m.args.get(i).and_then(Arg::num).unwrap_or(0);
        let tracks = |f: &dyn Fn(&rekordbox_formats::library::LibTrack) -> bool| -> Vec<Row> {
            c.tracks()
                .filter(|(_, t)| f(t))
                .map(|(id, t)| track_row(id, t))
                .collect()
        };
        Some(match m.kind {
            kind::ROOT_MENU => ROOT
                .iter()
                .map(|(id, label, ty)| Row::new(0, *id, &menu_label(label), *ty))
                .collect(),
            kind::TRACK_ROOT => tracks(&|_| true),
            kind::ARTIST_ROOT => c
                .artists()
                .map(|(id, name)| Row::new(0, id, name, item::ARTIST))
                .collect(),
            // no album information: one "ALL" entry per artist
            kind::ALBUM_ARTIST => vec![Row::new(0, u32::MAX, &menu_label("ALL"), item::ALL)],
            kind::TRACK_ALBUM_ARTIST => {
                let artist = arg(2);
                tracks(&|t| c.artist_id(t) == artist)
            }
            kind::ALBUM_ROOT | kind::TRACK_ALBUM => vec![],
            req::KEY_ROOT | kind::KEY_ROOT => c
                .keys()
                .map(|(id, k)| Row::new(0, id, k, item::KEY))
                .collect(),
            0x110b | 0x110c | 0x120c => {
                let key = arg(2);
                tracks(&|t| c.key_id(t) == key)
            }
            kind::SEARCH => {
                let text = m
                    .args
                    .iter()
                    .find_map(|a| match a {
                        Arg::Str(s) => Some(s.to_lowercase()),
                        _ => None,
                    })
                    .unwrap_or_default();
                tracks(&|t| {
                    format!("{} {}", t.title, catalog::artist_name(t))
                        .to_lowercase()
                        .contains(&text)
                })
            }
            // no playlists yet
            kind::TRACK_PLAYLIST => vec![],
            req::HISTORY_ROOT | req::DATE_ADDED_ROOT | req::DATE_ADDED_MONTH | req::MATCHING => {
                vec![]
            }
            kind::SONGDATA_TRACK => metadata(c, arg(1))?,
            req::SONGDATA_TRACK2 => songdata2(c, arg(1))?,
            _ => return None,
        })
    }
}

/// A track in a list, as rekordbox sends it (album track list in capture 1).
fn track_row(id: u32, t: &rekordbox_formats::library::LibTrack) -> Row {
    Row {
        flags: 0x0100_0000,
        a9: 1,
        a10: 0x101,
        ..Row::new(0, id, &t.title, item::TITLE)
    }
}

/// Track metadata (`CMD_SONGDATA_TRACK`): title, artist, album, duration, tempo, comment, key, rating,
/// colour, genre, date added (positions 0, 3 and 6 as in capture 1).
fn metadata(c: &Catalog, id: u32) -> Option<Vec<Row>> {
    let t = c.track(id)?;
    let artist = c.artist_id(t);
    Some(vec![
        Row {
            a10: 0x101,
            ..Row::new(artist, id, &t.title, item::TITLE)
        },
        Row::new(0, artist, catalog::artist_name(t), item::ARTIST),
        Row::new(0, 0, "", item::ALBUM),
        Row::new(0, catalog::duration_s(t), "", item::DURATION),
        Row::new(0, catalog::bpm_x100(t), "", item::TEMPO),
        Row::new(0, 0, "", item::COMMENT),
        Row::new(1, c.key_id(t), t.key.as_deref().unwrap_or(""), item::KEY),
        Row::new(0, 0, "", item::RATING),
        Row::new(0, 0, "", item::COLOR_NONE),
        Row::new(0, 0, "", item::GENRE),
        Row::new(0, 0, "", item::DATE_ADDED),
    ])
}

/// `CMD_SONGDATA_TRACK2` (asked before loading): file type, duration, tempo, comment, the full path
/// (parent = file size, `C:/…` with forward slashes), a flag, the key.
fn songdata2(c: &Catalog, id: u32) -> Option<Vec<Row>> {
    let t = c.track(id)?;
    let path = t.path.as_ref()?;
    let size = std::fs::metadata(path).map_or(0, |m| m.len().min(u64::from(u32::MAX)) as u32);
    let shown = path.to_string_lossy().replace('\\', "/");
    Some(vec![
        Row::new(0, catalog::file_type(path), "", item::TITLE),
        Row::new(0, catalog::duration_s(t), "", item::DURATION),
        Row::new(0, catalog::bpm_x100(t), "", item::TEMPO),
        Row::new(0, id, "", item::COMMENT),
        Row::new(size, id, &shown, item::PATH),
        Row::new(0, 1, "", item::FLAG_2F),
        Row::new(0, 0x7FFF_FFFF, "", item::KEY),
    ])
}
