//! Pro DJ Link packets as rekordbox sends and receives them in LINK EXPORT mode – pure byte ↔ value
//! code, no I/O. Layouts from capture 1 (`docs/link/CAPTURE1.md`) and SPEC §5i; the port-50000 codecs
//! and the CDJ status come from `rekordbox_formats::djlink`.
//!
//! Port 50001/50002 header ("long"): magic, 0x0A type, 0x0B–0x1E name, 0x1F, 0x20, 0x21 device number,
//! 0x22–0x23 length; payload from 0x24.

use std::net::Ipv4Addr;

use rekordbox_formats::djlink::{self, Announce, KeepAlive, MAGIC};

use crate::status::DeckStatus;

use crate::error::PacketError;

/// Name rekordbox announces.
pub const REKORDBOX: &str = "rekordbox";
/// Device kind of rekordbox in keep-alives (0x34).
pub const KIND_REKORDBOX: u8 = 4;
/// Slot of the rekordbox collection in media queries / responses.
pub const SLOT_COLLECTION: u32 = 4;

pub const PORT_ANNOUNCE: u16 = djlink::PORT_ANNOUNCE;
pub const PORT_BEAT: u16 = djlink::PORT_BEAT;
pub const PORT_STATUS: u16 = djlink::PORT_STATUS;

/// Packet types on port 50002.
pub mod kind {
    pub const MEDIA_QUERY: u8 = 0x05;
    pub const MEDIA_RESPONSE: u8 = 0x06;
    pub const CDJ_STATUS: u8 = 0x0A;
    pub const DEVICE_INFO_QUERY: u8 = 0x10;
    pub const DEVICE_INFO: u8 = 0x11;
    pub const PRESENCE: u8 = 0x16;
    pub const ACK_1A: u8 = 0x1A;
    pub const STATUS_29: u8 = 0x29;
    pub const QUERY_30: u8 = 0x30;
    pub const REPLY_31: u8 = 0x31;
    pub const QUERY_46: u8 = 0x46;
    pub const REPLY_47: u8 = 0x47;
}

fn need(b: &[u8], n: usize) -> Result<(), PacketError> {
    if b.len() < n {
        Err(PacketError::TooShort {
            len: b.len(),
            need: n,
        })
    } else {
        Ok(())
    }
}

/// A port 50001/50002 packet: magic, type, name, 0x1F, 0x20, device, length, `payload`.
/// `length` is what goes into 0x22 (most packets: payload length; type 0x29: total length).
pub fn long_packet(
    kind: u8,
    name: &str,
    b1f: u8,
    b20: u8,
    device: u8,
    length: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut b = vec![0u8; 0x24];
    b[..10].copy_from_slice(&MAGIC);
    b[0x0A] = kind;
    for (i, c) in name.bytes().take(20).enumerate() {
        b[0x0B + i] = c;
    }
    b[0x1F] = b1f;
    b[0x20] = b20;
    b[0x21] = device;
    b[0x22..0x24].copy_from_slice(&length.to_be_bytes());
    b.extend_from_slice(payload);
    b
}

/// Header fields of a port 50001/50002 packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub kind: u8,
    pub name: String,
    pub b1f: u8,
    pub b20: u8,
    pub device: u8,
    pub length: u16,
}

pub fn head(b: &[u8]) -> Result<Head, PacketError> {
    need(b, 0x24)?;
    if b[..10] != MAGIC {
        return Err(PacketError::NoMagic);
    }
    let name = String::from_utf8_lossy(&b[0x0B..0x1F])
        .trim_end_matches('\0')
        .to_string();
    Ok(Head {
        kind: b[0x0A],
        name,
        b1f: b[0x1F],
        b20: b[0x20],
        device: b[0x21],
        length: u16::from_be_bytes([b[0x22], b[0x23]]),
    })
}

fn utf16be(s: &str, max_bytes: usize) -> Vec<u8> {
    let mut v: Vec<u8> = s
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .take(max_bytes)
        .collect();
    v.resize(max_bytes, 0);
    v
}

/// What a player asks rekordbox on port 50002.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// 0x10: who are you (answered with the computer name, 0x11).
    DeviceInfo {
        from: u8,
    },
    /// 0x30 (answered with 0x31).
    Query30 {
        from: u8,
    },
    /// 0x46 (answered with 0x47).
    Query46 {
        from: u8,
        payload: Vec<u8>,
    },
    /// 0x05: what is in `slot` of device `target`; answer to `reply_to`.
    MediaQuery {
        from: u8,
        reply_to: Ipv4Addr,
        target: u32,
        slot: u32,
    },
    /// 0x1A from a player (meaning not determined).
    Ack1A {
        from: u8,
        payload: Vec<u8>,
    },
    /// 0x0A deck status.
    Status(Box<DeckStatus>),
    Other {
        kind: u8,
        from: u8,
    },
}

/// Decode a port-50002 packet.
pub fn parse_status_port(b: &[u8]) -> Result<Request, PacketError> {
    let h = head(b)?;
    let payload = b.get(0x24..).unwrap_or_default().to_vec();
    Ok(match h.kind {
        kind::DEVICE_INFO_QUERY => Request::DeviceInfo { from: h.device },
        kind::QUERY_30 => Request::Query30 { from: h.device },
        kind::QUERY_46 => Request::Query46 {
            from: h.device,
            payload,
        },
        kind::MEDIA_QUERY => {
            need(b, 0x30)?;
            let ip = Ipv4Addr::new(b[0x24], b[0x25], b[0x26], b[0x27]);
            let u32_at = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
            Request::MediaQuery {
                from: h.device,
                reply_to: ip,
                target: u32_at(0x28),
                slot: u32_at(0x2C),
            }
        }
        kind::ACK_1A => Request::Ack1A {
            from: h.device,
            payload,
        },
        kind::CDJ_STATUS => Request::Status(Box::new(DeckStatus::parse(b)?)),
        k => Request::Other {
            kind: k,
            from: h.device,
        },
    })
}

/// What rekordbox announces about its collection in a media response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LibraryInfo {
    pub tracks: u16,
    pub playlists: u16,
}

/// 0x11: the computer's name (UTF-16BE, 256 bytes).
pub fn device_info(own: u8, hostname: &str) -> Vec<u8> {
    let mut p = vec![own, 0x01, 0, 0];
    p.extend(utf16be(hostname, 256));
    long_packet(kind::DEVICE_INFO, REKORDBOX, 1, 1, own, p.len() as u16, &p)
}

/// 0x31: answer to 0x30 (fixed content as rekordbox sends it).
pub fn reply_31(own: u8) -> Vec<u8> {
    let p = [0x06, 0, 0, 0, 0, 0, 0, 0];
    long_packet(kind::REPLY_31, REKORDBOX, 1, 3, own, p.len() as u16, &p)
}

/// 0x47: answer to 0x46 (fixed content as rekordbox sends it).
pub fn reply_47(own: u8) -> Vec<u8> {
    let mut p = vec![
        own, 0x04, 0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0x01, 0x01, 0x01, 0x03, 0x01, 0x01, 0x01,
    ];
    p.resize(0x24, 0);
    long_packet(kind::REPLY_47, REKORDBOX, 1, 1, own, p.len() as u16, &p)
}

/// 0x06: media response for the rekordbox collection (slot 4), 0x9C bytes of payload.
pub fn media_response(own: u8, slot: u32, name: &str, lib: LibraryInfo) -> Vec<u8> {
    let mut p = Vec::with_capacity(0x9C);
    p.extend_from_slice(&u32::from(own).to_be_bytes());
    p.extend_from_slice(&slot.to_be_bytes());
    p.extend(utf16be(name, 0x40)); // 0x2C name
    p.extend(utf16be("", 0x28)); // 0x6C creation date
    p.resize(0xA6 - 0x24, 0);
    p.extend_from_slice(&lib.tracks.to_be_bytes()); // 0xA6
    p.push(0); // 0xA8 colour
    p.push(0); // 0xA9
    p.push(1); // 0xAA track type: rekordbox database
    p.push(1); // 0xAB
    p.extend_from_slice(&[0, 0]); // 0xAC
    p.extend_from_slice(&lib.playlists.to_be_bytes()); // 0xAE
    p.resize(0x9C, 0); // 0xB0 total size, 0xB8 free size: 0
    long_packet(
        kind::MEDIA_RESPONSE,
        REKORDBOX,
        1,
        1,
        own,
        p.len() as u16,
        &p,
    )
}

/// 0x16, sent to each player (12 zero bytes, length field 0).
pub fn presence(own: u8) -> Vec<u8> {
    long_packet(kind::PRESENCE, REKORDBOX, 1, 1, own, 0, &[0u8; 12])
}

/// 0x29, broadcast every 10 ms: pitch 0 %, and the master tempo when one is known.
pub fn status_29(own: u8, master_bpm: Option<f64>) -> Vec<u8> {
    let bpm = master_bpm.map_or(0, |b| 0x8000_0000 | ((b * 100.0).round() as u32 & 0xFFFF));
    let mut p = vec![own, 0, 0, 0xC0];
    p.extend_from_slice(&0x0010_0000u32.to_be_bytes());
    p.extend_from_slice(&bpm.to_be_bytes());
    p.extend_from_slice(&0x0010_0000u32.to_be_bytes());
    p.extend_from_slice(&[0x00, 0x09, 0xFF, 0x00]);
    long_packet(
        kind::STATUS_29,
        REKORDBOX,
        1,
        1,
        own,
        (0x24 + p.len()) as u16,
        &p,
    )
}

/// rekordbox's port-50000 packets: discovery (0x00), number claim (0x02), keep-alive (0x06).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub number: u8,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
}

pub fn discovery(counter: u8, mac: [u8; 6]) -> Vec<u8> {
    Announce::Discovery {
        counter,
        kind: KIND_REKORDBOX,
        mac,
    }
    .to_bytes(REKORDBOX)
}

/// Number claim for `me.number` in round `counter` (rekordbox claims all its candidates in turn,
/// six rounds).
pub fn claim(me: &Me, counter: u8) -> Vec<u8> {
    Announce::IdRequest {
        set: false,
        ip: me.ip,
        mac: me.mac,
        device: me.number,
        counter,
        kind: KIND_REKORDBOX,
    }
    .to_bytes(REKORDBOX)
}

/// Keep-alive with `peers` known devices (rekordbox counts itself).
pub fn keep_alive(me: &Me, peers: u8) -> Vec<u8> {
    let mut b = KeepAlive {
        name: REKORDBOX.into(),
        device: me.number,
        b25: 1,
        mac: me.mac,
        ip: me.ip,
        peers,
        kind: KIND_REKORDBOX,
        b35: 8,
    }
    .to_bytes();
    b[0x31] = 1;
    b
}

/// Disconnect (0x08) when leaving.
pub fn disconnect(me: &Me) -> Vec<u8> {
    Announce::Disconnect {
        device: me.number,
        ip: me.ip,
    }
    .to_bytes(REKORDBOX)
}

/// One line describing a packet (for `link-sniff` and logs).
pub fn describe(port: u16, b: &[u8]) -> String {
    if port == PORT_ANNOUNCE {
        return match Announce::parse(b) {
            Ok((h, a)) => format!("{:<10} {a:?}", h.name),
            Err(e) => format!("? {e}"),
        };
    }
    let Ok(h) = head(b) else {
        return format!("? {} bytes", b.len());
    };
    if port == PORT_BEAT {
        return format!(
            "{:<10} #{:<3} beat-port type {:#04x}, {} bytes",
            h.name,
            h.device,
            h.kind,
            b.len()
        );
    }
    let what = match parse_status_port(b) {
        Ok(Request::Status(s)) => format!(
            "deck {} status play {:#04x} slot {} track {:?} bpm {:?} pitch {:+.2}% beat {:?}",
            s.device,
            s.play_state,
            s.slot,
            s.track_id,
            s.bpm(),
            s.pitch_percent(),
            s.beat
        ),
        Ok(Request::MediaQuery {
            reply_to,
            target,
            slot,
            ..
        }) => format!("media query: device {target} slot {slot}, reply to {reply_to}"),
        Ok(Request::Other { kind: k, .. }) => match k {
            kind::MEDIA_RESPONSE => "media response".into(),
            kind::DEVICE_INFO => "device info (answer to 0x10)".into(),
            kind::PRESENCE => "presence 0x16".into(),
            kind::STATUS_29 => format!("status 0x29, {} bytes", b.len()),
            kind::REPLY_31 => "answer to 0x30".into(),
            kind::REPLY_47 => "answer to 0x46".into(),
            k => format!("type {k:#04x}, {} bytes", b.len()),
        },
        Ok(r) => format!("{r:?}"),
        Err(e) => format!("type {:#04x}: {e}", h.kind),
    };
    format!("{:<10} #{:<3} {what}", h.name, h.device)
}
