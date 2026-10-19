//! Link Export database protocol (SPEC §5k): typed fields, messages, port query and greeting.
//!
//! Every value on the wire carries a type byte: `0x0F` u8, `0x10` u16, `0x11` u32 (big-endian),
//! `0x14` blob (u32 length + bytes), `0x26` string (u32 length in UTF-16 units incl. NUL, UTF-16BE).
//! A message is: magic, transaction id, type, argument count, a 12-byte blob with the argument
//! types (`06` number, `02` string, `03` blob; 16 bytes for menu items with more than 12 arguments),
//! then the arguments. A blob argument whose preceding number argument is 0 is not transmitted.

use std::fmt;

pub const MAGIC: u32 = 0x8723_49AE;
/// TCP port where a client asks for the database server's port.
pub const PORT_QUERY: u16 = 12523;
/// Port query: u32 length 15, "RemoteDBServer" with NUL.
pub const PORT_QUERY_REQUEST: &[u8] = b"\x00\x00\x00\x0fRemoteDBServer\x00";
/// First bytes on a database connection (sent by the client, echoed by the server): number field 1.
pub const GREETING: [u8; 5] = [0x11, 0, 0, 0, 1];
/// Transaction id of the setup and disconnect messages.
pub const SETUP_TX: u32 = 0xFFFF_FFFE;

pub const TAG_U8: u8 = 0x0F;
pub const TAG_U16: u8 = 0x10;
pub const TAG_U32: u8 = 0x11;
pub const TAG_BLOB: u8 = 0x14;
pub const TAG_STRING: u8 = 0x26;

/// Message types (names as in rekordbox's command table, SPEC §5k).
pub mod kind {
    pub const PLAYER_ID: u16 = 0x0000;
    pub const DROP: u16 = 0x0100;
    pub const ROOT_MENU: u16 = 0x1000;
    pub const GENRE_ROOT: u16 = 0x1001;
    pub const ARTIST_ROOT: u16 = 0x1002;
    pub const ALBUM_ROOT: u16 = 0x1003;
    pub const TRACK_ROOT: u16 = 0x1004;
    pub const BPM_ROOT: u16 = 0x1006;
    pub const KEY_ROOT: u16 = 0x100b;
    pub const ALBUM_ARTIST: u16 = 0x1102;
    pub const TRACK_ALBUM_ARTIST: u16 = 0x1202;
    pub const TRACK_ALBUM: u16 = 0x1103;
    pub const ARTIST_GENRE: u16 = 0x1101;
    pub const TRACK_KEY: u16 = 0x110b;
    pub const TRACK_PLAYLIST: u16 = 0x1105;
    pub const SEARCH: u16 = 0x1300;
    pub const SONGDATA_TRACK: u16 = 0x2002;
    pub const GET_IMAGE: u16 = 0x2003;
    pub const GET_WAVEDATA: u16 = 0x2004;
    pub const GET_USB_CUE: u16 = 0x2104;
    pub const GET_QUANTIZE_DATA: u16 = 0x2204;
    pub const GET_VBR_INFO: u16 = 0x2504;
    pub const GET_DELIVERY_INFO: u16 = 0x2602;
    pub const GET_PAR_WAVE: u16 = 0x2904;
    pub const GET_USB_CUE2: u16 = 0x2b04;
    pub const GET_ATOM_INFO: u16 = 0x2c04;
    pub const LEFT_BUF: u16 = 0x3000;
    pub const RET_DWORD: u16 = 0x4000;
    pub const RET_LIST_START: u16 = 0x4001;
    pub const RET_LIST_SET: u16 = 0x4101;
    pub const RET_LIST_END: u16 = 0x4201;
    pub const RET_IMAGE: u16 = 0x4002;
    pub const RET_UNSUPPORTED: u16 = 0x4003;
    pub const RET_WAVE: u16 = 0x4402;
    pub const RET_VBR: u16 = 0x4502;
    pub const RET_QTZ: u16 = 0x4602;
    pub const RET_CUE: u16 = 0x4702;
    pub const RET_PAR_WAVE: u16 = 0x4a02;
    pub const RET_CUE2: u16 = 0x4e02;
    pub const RET_ATOM: u16 = 0x4f02;

    /// rekordbox's name of a message type, if it has one.
    pub fn name(k: u16) -> &'static str {
        match k {
            PLAYER_ID => "CMD_INFO_PLAYER_ID",
            DROP => "CMD_INFO_DROP",
            ROOT_MENU => "CMD_ROOT_MENU",
            GENRE_ROOT => "CMD_GENRE_ROOT",
            ARTIST_ROOT => "CMD_ARTIST_ROOT",
            ALBUM_ROOT => "CMD_ALBUM_ROOT",
            TRACK_ROOT => "CMD_TRACK_ROOT",
            BPM_ROOT => "CMD_BPM_ROOT",
            KEY_ROOT => "CMD_KEY_ROOT",
            ALBUM_ARTIST => "CMD_ALBUM_ARTIST",
            TRACK_ALBUM_ARTIST => "CMD_TRACK_ALBUM_ARTIST",
            TRACK_ALBUM => "CMD_TRACK_ALBUM",
            ARTIST_GENRE => "CMD_ARTIST_GENRE",
            TRACK_KEY => "CMD_TRACK_KEY",
            TRACK_PLAYLIST => "CMD_TRACK_PLAYLIST",
            0x1200 => "CMD_TRACK_CONTENT",
            SEARCH => "CMD_SEARCH",
            0x1400 => "CMD_RIGHT_MENU",
            SONGDATA_TRACK => "CMD_SONGDATA_TRACK",
            0x2102 => "CMD_SONGDATA_TRACK2",
            GET_IMAGE => "CMD_GET_IMAGE",
            GET_WAVEDATA => "CMD_GET_WAVEDATA",
            GET_USB_CUE => "CMD_GET_USB_CUE",
            GET_QUANTIZE_DATA => "CMD_GET_QUANTIZE_DATA",
            0x2804 => "CMD_GET_QTZ_OFFSET",
            GET_VBR_INFO => "CMD_GET_VBR_INFO",
            GET_DELIVERY_INFO => "CMD_GET_DELIVERY_INFO",
            0x2402 => "CMD_GET_DECODE_INFO",
            0x2302 => "CMD_GET_PCMODE_SONG_INFO",
            GET_PAR_WAVE => "CMD_GET_PAR_WAVE",
            GET_USB_CUE2 => "CMD_GET_USB_CUE2",
            GET_ATOM_INFO => "CMD_GET_SPECIFIED_ATOM_INFO",
            LEFT_BUF => "CMD_LEFT_BUF",
            RET_DWORD => "CMD_RET_DWORD_DATA",
            RET_LIST_START => "CMD_RET_LISTBUF_START",
            RET_LIST_SET => "CMD_RET_LISTBUF_SET",
            RET_LIST_END => "CMD_RET_LISTBUF_END",
            RET_IMAGE => "CMD_RET_IMAGE",
            RET_UNSUPPORTED => "CMD_RET_UNSUPORT_CMD",
            RET_WAVE => "CMD_RET_WAVE",
            RET_VBR => "CMD_RET_VBR_INFO",
            RET_QTZ => "CMD_RET_QTZ_INFO",
            RET_CUE => "CMD_RET_CUE",
            0x4802 => "CMD_RET_DECODE_INFO",
            RET_PAR_WAVE => "CMD_RET_PAR_WAVE",
            RET_CUE2 => "CMD_RET_CUE2",
            RET_ATOM => "CMD_RET_ATOM_INFO",
            _ => "?",
        }
    }
}

/// Menu item types (argument 7 of a menu item, SPEC §5k).
pub mod item {
    pub const FOLDER: u32 = 0x0001;
    pub const ALBUM: u32 = 0x0002;
    pub const TITLE: u32 = 0x0004;
    pub const GENRE: u32 = 0x0006;
    pub const ARTIST: u32 = 0x0007;
    pub const PLAYLIST: u32 = 0x0008;
    pub const RATING: u32 = 0x000a;
    pub const DURATION: u32 = 0x000b;
    pub const TEMPO: u32 = 0x000d;
    pub const KEY: u32 = 0x000f;
    pub const COLOR_NONE: u32 = 0x0013;
    pub const COMMENT: u32 = 0x0023;
    pub const DATE_ADDED: u32 = 0x002e;
    /// Track title with the artist as second column.
    pub const TITLE_ARTIST: u32 = 0x0704;
}

/// One argument of a message.
#[derive(Clone, PartialEq, Eq)]
pub enum Arg {
    Num(u32),
    Str(String),
    Blob(Vec<u8>),
}

impl Arg {
    fn tag(&self) -> u8 {
        match self {
            Arg::Num(_) => 0x06,
            Arg::Str(_) => 0x02,
            Arg::Blob(_) => 0x03,
        }
    }

    pub fn num(&self) -> Option<u32> {
        match self {
            Arg::Num(n) => Some(*n),
            _ => None,
        }
    }
}

impl fmt::Debug for Arg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Arg::Num(n) => write!(f, "{n:#x}"),
            Arg::Str(s) => write!(f, "{s:?}"),
            Arg::Blob(b) => write!(f, "<{} bytes>", b.len()),
        }
    }
}

/// A database message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub tx: u32,
    pub kind: u16,
    pub args: Vec<Arg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbError {
    BadMagic(u32),
    BadField(u8),
    BadArgType(u8),
    TooManyArgs(usize),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::BadMagic(m) => write!(f, "bad magic {m:#x}"),
            DbError::BadField(t) => write!(f, "unexpected field type {t:#x}"),
            DbError::BadArgType(t) => write!(f, "unknown argument type {t:#x}"),
            DbError::TooManyArgs(n) => write!(f, "{n} arguments"),
        }
    }
}

impl std::error::Error for DbError {}

pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.push(TAG_U32);
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_blob(out: &mut Vec<u8>, b: &[u8]) {
    out.push(TAG_BLOB);
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    out.push(TAG_STRING);
    out.extend_from_slice(&(units.len() as u32).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
}

/// Reads fields from a buffer; `None` = not enough data yet.
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }

    fn field(&mut self) -> Result<Option<Arg>, DbError> {
        let Some(&t) = self.b.get(self.p) else { return Ok(None) };
        self.p += 1;
        let v = match t {
            TAG_U8 => self.take(1).map(|s| Arg::Num(u32::from(s[0]))),
            TAG_U16 => self.take(2).map(|s| Arg::Num(u32::from(u16::from_be_bytes([s[0], s[1]])))),
            TAG_U32 => self.take(4).map(|s| Arg::Num(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))),
            TAG_BLOB => match self.take(4) {
                Some(s) => {
                    let n = u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize;
                    self.take(n).map(|d| Arg::Blob(d.to_vec()))
                }
                None => None,
            },
            TAG_STRING => match self.take(4) {
                Some(s) => {
                    let n = u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize;
                    self.take(2 * n).map(|d| {
                        let units: Vec<u16> = d.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
                        Arg::Str(String::from_utf16_lossy(&units))
                    })
                }
                None => None,
            },
            other => return Err(DbError::BadField(other)),
        };
        Ok(v)
    }

    fn number(&mut self) -> Result<Option<u32>, DbError> {
        match self.field()? {
            Some(Arg::Num(n)) => Ok(Some(n)),
            Some(_) => Err(DbError::BadField(0)),
            None => Ok(None),
        }
    }
}

impl Message {
    pub fn new(tx: u32, kind: u16, args: Vec<Arg>) -> Message {
        Message { tx, kind, args }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        put_u32(&mut out, MAGIC);
        put_u32(&mut out, self.tx);
        out.push(TAG_U16);
        out.extend_from_slice(&self.kind.to_be_bytes());
        out.push(TAG_U8);
        out.push(self.args.len() as u8);
        let mut tags = vec![0u8; if self.args.len() > 12 { 16 } else { 12 }];
        for (t, a) in tags.iter_mut().zip(&self.args) {
            *t = a.tag();
        }
        put_blob(&mut out, &tags);
        for a in &self.args {
            match a {
                Arg::Num(n) => put_u32(&mut out, *n),
                Arg::Str(s) => put_string(&mut out, s),
                // an empty blob is not transmitted (its length argument is 0)
                Arg::Blob(b) if b.is_empty() => {}
                Arg::Blob(b) => put_blob(&mut out, b),
            }
        }
        out
    }

    /// Decode one message from the start of `buf`: `Ok(None)` while incomplete, else the message
    /// and the number of bytes it used.
    pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>, DbError> {
        let mut r = Reader { b: buf, p: 0 };
        macro_rules! need {
            ($e:expr) => {
                match $e? {
                    Some(v) => v,
                    None => return Ok(None),
                }
            };
        }
        let magic = need!(r.number());
        if magic != MAGIC {
            return Err(DbError::BadMagic(magic));
        }
        let tx = need!(r.number());
        let kind = need!(r.number()) as u16;
        let n = need!(r.number()) as usize;
        let tags = match need!(r.field()) {
            Arg::Blob(t) => t,
            _ => return Err(DbError::BadField(TAG_BLOB)),
        };
        if n > tags.len() {
            return Err(DbError::TooManyArgs(n));
        }
        let mut args = Vec::with_capacity(n);
        for &t in &tags[..n] {
            let a = match t {
                0x03 if matches!(args.last(), Some(Arg::Num(0))) => Arg::Blob(Vec::new()),
                0x02 | 0x03 | 0x06 => need!(r.field()),
                other => return Err(DbError::BadArgType(other)),
            };
            args.push(a);
        }
        Ok(Some((Message { tx, kind, args }, r.p)))
    }
}

/// DMST argument: requesting player, menu location, slot, track type.
pub fn dmst(device: u8, menu: u8, slot: u8, track_type: u8) -> u32 {
    u32::from_be_bytes([device, menu, slot, track_type])
}

/// Split a DMST argument into (device, menu, slot, track type).
pub fn split_dmst(v: u32) -> (u8, u8, u8, u8) {
    let b = v.to_be_bytes();
    (b[0], b[1], b[2], b[3])
}

/// A menu item message (`CMD_RET_LISTBUF_SET`, 12 arguments).
#[allow(clippy::too_many_arguments)]
pub fn menu_item(tx: u32, parent: u32, id: u32, label1: &str, label2: &str, item_type: u32, artwork: u32, position: u32) -> Message {
    let bytes = |s: &str| (s.encode_utf16().count() as u32 + 1) * 2;
    Message::new(
        tx,
        kind::RET_LIST_SET,
        vec![
            Arg::Num(parent),
            Arg::Num(id),
            Arg::Num(bytes(label1)),
            Arg::Str(label1.into()),
            Arg::Num(bytes(label2)),
            Arg::Str(label2.into()),
            Arg::Num(item_type),
            Arg::Num(0),
            Arg::Num(artwork),
            Arg::Num(position),
            Arg::Num(0),
            Arg::Num(0),
        ],
    )
}

/// Split a byte stream into messages; returns the messages and how many bytes were used.
pub fn decode_stream(buf: &[u8]) -> (Vec<Message>, usize, Option<DbError>) {
    let mut out = Vec::new();
    let mut used = 0;
    loop {
        match Message::decode(&buf[used..]) {
            Ok(Some((m, n))) => {
                out.push(m);
                used += n;
            }
            Ok(None) => return (out, used, None),
            Err(e) => return (out, used, Some(e)),
        }
    }
}

/// Like [`decode_stream`], but after an undecodable stretch (e.g. a gap in a capture) continue at the
/// next message magic. Returns the messages and the number of skipped stretches.
pub fn decode_stream_resync(buf: &[u8]) -> (Vec<Message>, usize) {
    let magic = [TAG_U32, 0x87, 0x23, 0x49, 0xAE];
    let (mut out, mut used, mut skips) = (Vec::new(), 0usize, 0usize);
    while used < buf.len() {
        let (msgs, n, err) = decode_stream(&buf[used..]);
        out.extend(msgs);
        used += n;
        if err.is_none() {
            break;
        }
        skips += 1;
        match buf[used + 1..].windows(5).position(|w| w == magic) {
            Some(i) => used += 1 + i,
            None => break,
        }
    }
    (out, skips)
}
