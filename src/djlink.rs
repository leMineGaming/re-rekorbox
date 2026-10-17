//! Pro DJ Link network packets (SPEC §5i): device announcement / keep-alive (port 50000), beat
//! (port 50001) and status (port 50002), plus the device table rules of the original.
//!
//! Every packet starts with the 10-byte ASCII magic [`MAGIC`] followed by the packet type at 0x0A.
//! Port 50000 uses the "short" header ([`ShortHeader`], name at 0x0C, total length at 0x22); ports
//! 50001/50002 use the "long" header ([`LongHeader`], name at 0x0B, device number at 0x21, length of
//! the part after 0x24 at 0x22). All numbers are big-endian.
//!
//! Only parsing and building of packets lives here; [`DeviceTable`] applies the keep-alive rules.
//! Sockets are left to the caller (see `rbf djlink-listen`).

use std::fmt;
use std::net::Ipv4Addr;

/// Packet magic at offset 0.
pub const MAGIC: [u8; 10] = *b"Qspt1WmJOL";
/// Announcement / device numbers / keep-alive.
pub const PORT_ANNOUNCE: u16 = 50000;
/// Beat packets.
pub const PORT_BEAT: u16 = 50001;
/// Status packets.
pub const PORT_STATUS: u16 = 50002;
/// All ports the original checks before starting (2049 = NFS).
pub const PORTS_CHECKED: [u16; 6] = [2049, 50000, 50001, 50002, 50004, 50111];
/// Name length in both header variants.
pub const NAME_LEN: usize = 20;
/// Pitch value for ±0 % (2^20).
pub const PITCH_NORMAL: u32 = 0x10_0000;
/// Device kind rekordbox announces.
pub const KIND_REKORDBOX: u8 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort { need: usize, have: usize },
    BadMagic,
    UnsupportedVersion(u8),
    WrongType(u8),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::TooShort { need, have } => write!(f, "packet too short: {have} bytes, need {need}"),
            Error::BadMagic => f.write_str("missing Qspt1WmJOL magic"),
            Error::UnsupportedVersion(v) => write!(f, "header version {v} (receivers accept only 0 and 1)"),
            Error::WrongType(t) => write!(f, "unexpected packet type 0x{t:02X}"),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

fn need(b: &[u8], n: usize) -> Result<()> {
    if b.len() < n {
        Err(Error::TooShort { need: n, have: b.len() })
    } else {
        Ok(())
    }
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn put_u16(b: &mut [u8], o: usize, v: u16) {
    b[o..o + 2].copy_from_slice(&v.to_be_bytes());
}

fn put_u32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_be_bytes());
}

fn name_from(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

fn put_name(b: &mut [u8], o: usize, name: &str) {
    let n = name.as_bytes();
    let len = n.len().min(NAME_LEN);
    b[o..o + len].copy_from_slice(&n[..len]);
}

fn ip_at(b: &[u8], o: usize) -> Ipv4Addr {
    Ipv4Addr::new(b[o], b[o + 1], b[o + 2], b[o + 3])
}

fn mac_at(b: &[u8], o: usize) -> [u8; 6] {
    b[o..o + 6].try_into().unwrap()
}

/// True when the buffer starts with the magic.
pub fn has_magic(b: &[u8]) -> bool {
    b.len() >= MAGIC.len() && b[..MAGIC.len()] == MAGIC
}

/// Packet type byte (0x0A) of a buffer with magic.
pub fn packet_type(b: &[u8]) -> Option<u8> {
    (has_magic(b) && b.len() > 0x0A).then(|| b[0x0A])
}

// ---------------------------------------------------------------------------------------------
// Headers

/// Port 50000 header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortHeader {
    pub kind: u8,
    pub subtype: u8,
    pub name: String,
    /// 0x20 (rekordbox: 1).
    pub b20: u8,
    /// 0x21 (rekordbox: 3; the CDJ-2000/900 filter checks it for 0).
    pub b21: u8,
    /// Total packet length (0x22).
    pub length: u16,
}

impl ShortHeader {
    pub fn parse(b: &[u8]) -> Result<ShortHeader> {
        need(b, 0x24)?;
        if !has_magic(b) {
            return Err(Error::BadMagic);
        }
        Ok(ShortHeader {
            kind: b[0x0A],
            subtype: b[0x0B],
            name: name_from(&b[0x0C..0x20]),
            b20: b[0x20],
            b21: b[0x21],
            length: u16_at(b, 0x22),
        })
    }

    fn write(kind: u8, subtype: u8, name: &str, total: usize) -> Vec<u8> {
        let mut b = vec![0u8; total];
        b[..10].copy_from_slice(&MAGIC);
        b[0x0A] = kind;
        b[0x0B] = subtype;
        put_name(&mut b, 0x0C, name);
        b[0x20] = 0x01;
        b[0x21] = 0x03;
        put_u16(&mut b, 0x22, total as u16);
        b
    }
}

/// Port 50001/50002 header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LongHeader {
    pub kind: u8,
    pub name: String,
    /// 0x1F; receivers drop packets with a value ≥ 2.
    pub version: u8,
    /// 0x20; selects optional fields of the CDJ status.
    pub subversion: u8,
    /// Device number (0x21).
    pub device: u8,
    /// Length of the part after 0x24 (0x22).
    pub length: u16,
}

impl LongHeader {
    pub fn parse(b: &[u8]) -> Result<LongHeader> {
        need(b, 0x24)?;
        if !has_magic(b) {
            return Err(Error::BadMagic);
        }
        if b[0x1F] >= 2 {
            return Err(Error::UnsupportedVersion(b[0x1F]));
        }
        Ok(LongHeader {
            kind: b[0x0A],
            name: name_from(&b[0x0B..0x1F]),
            version: b[0x1F],
            subversion: b[0x20],
            device: b[0x21],
            length: u16_at(b, 0x22),
        })
    }

    fn write(kind: u8, name: &str, device: u8, total: usize) -> Vec<u8> {
        let mut b = vec![0u8; total];
        b[..10].copy_from_slice(&MAGIC);
        b[0x0A] = kind;
        put_name(&mut b, 0x0B, name);
        b[0x1F] = 0x01;
        b[0x20] = 0x01;
        b[0x21] = device;
        put_u16(&mut b, 0x22, (total - 0x24) as u16);
        b
    }
}

// ---------------------------------------------------------------------------------------------
// Port 50000

/// Decoded port-50000 packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Announce {
    /// 00/00.
    Discovery { counter: u8, kind: u8, mac: [u8; 6] },
    /// 02/00 (`set == false`, broadcast) or 02/01 (`set == true`, unicast).
    IdRequest { set: bool, ip: Ipv4Addr, mac: [u8; 6], device: u8, counter: u8, kind: u8 },
    /// 03/00 (use) or 03/01 (set). `status` (0x26): for set 0 = accepted, 2 = restart; for use 1 = ok.
    IdResponse { set: bool, device: u8, counter: u8, status: u8 },
    /// 06/00 keep-alive.
    KeepAlive(KeepAlive),
    /// 08/00.
    Disconnect { device: u8, ip: Ipv4Addr },
    /// 09: reject ("CDJ AND XDJ EXIST").
    Reject,
    /// 0A/00.
    CompatiRequest { kind: u8 },
    /// Any other type/subtype.
    Other { kind: u8, subtype: u8 },
}

/// Keep-alive (ConfigNotify, 06/00, 0x36 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepAlive {
    pub name: String,
    pub device: u8,
    /// 0x25.
    pub b25: u8,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    /// Number of devices the sender knows (0x30).
    pub peers: u8,
    /// Device kind (0x34); rekordbox 4, receivers accept 1..=9.
    pub kind: u8,
    /// 0x35 (rekordbox: 8).
    pub b35: u8,
}

impl KeepAlive {
    pub const LEN: usize = 0x36;

    /// The receiver's validity rule: device number 1..=0x50 and kind 1..=9.
    pub fn is_valid(&self) -> bool {
        (1..=0x50).contains(&self.device) && (1..=9).contains(&self.kind)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = ShortHeader::write(0x06, 0x00, &self.name, Self::LEN);
        b[0x24] = self.device;
        b[0x25] = self.b25;
        b[0x26..0x2C].copy_from_slice(&self.mac);
        b[0x2C..0x30].copy_from_slice(&self.ip.octets());
        b[0x30] = self.peers;
        b[0x34] = self.kind;
        b[0x35] = self.b35;
        b
    }
}

impl Announce {
    pub fn parse(b: &[u8]) -> Result<(ShortHeader, Announce)> {
        let h = ShortHeader::parse(b)?;
        let a = match (h.kind, h.subtype) {
            (0x00, 0x00) => {
                need(b, 0x2C)?;
                Announce::Discovery { counter: b[0x24], kind: b[0x25], mac: mac_at(b, 0x26) }
            }
            (0x02, s @ (0x00 | 0x01)) => {
                need(b, 0x32)?;
                Announce::IdRequest {
                    set: s == 1,
                    ip: ip_at(b, 0x24),
                    mac: mac_at(b, 0x28),
                    device: b[0x2E],
                    counter: b[0x2F],
                    kind: b[0x30],
                }
            }
            (0x03, s @ (0x00 | 0x01)) => {
                need(b, 0x27)?;
                Announce::IdResponse { set: s == 1, device: b[0x24], counter: b[0x25], status: b[0x26] }
            }
            (0x06, 0x00) => {
                need(b, KeepAlive::LEN)?;
                Announce::KeepAlive(KeepAlive {
                    name: h.name.clone(),
                    device: b[0x24],
                    b25: b[0x25],
                    mac: mac_at(b, 0x26),
                    ip: ip_at(b, 0x2C),
                    peers: b[0x30],
                    kind: b[0x34],
                    b35: b[0x35],
                })
            }
            (0x08, _) => {
                need(b, 0x29)?;
                Announce::Disconnect { device: b[0x24], ip: ip_at(b, 0x25) }
            }
            (0x09, _) => Announce::Reject,
            (0x0A, 0x00) => {
                need(b, 0x25)?;
                Announce::CompatiRequest { kind: b[0x24] }
            }
            (kind, subtype) => Announce::Other { kind, subtype },
        };
        Ok((h, a))
    }

    /// Build the packet as the original sends it (`name` goes into the header).
    pub fn to_bytes(&self, name: &str) -> Vec<u8> {
        match self {
            Announce::Discovery { counter, kind, mac } => {
                let mut b = ShortHeader::write(0x00, 0x00, name, 0x2C);
                b[0x24] = *counter;
                b[0x25] = *kind;
                b[0x26..0x2C].copy_from_slice(mac);
                b
            }
            Announce::IdRequest { set, ip, mac, device, counter, kind } => {
                let mut b = ShortHeader::write(0x02, u8::from(*set), name, 0x32);
                b[0x24..0x28].copy_from_slice(&ip.octets());
                b[0x28..0x2E].copy_from_slice(mac);
                b[0x2E] = *device;
                b[0x2F] = *counter;
                b[0x30] = *kind;
                b[0x31] = 1;
                b
            }
            Announce::IdResponse { set, device, counter, status } => {
                let mut b = ShortHeader::write(0x03, u8::from(*set), name, 0x27);
                b[0x24] = *device;
                b[0x25] = *counter;
                b[0x26] = *status;
                b
            }
            Announce::KeepAlive(k) => {
                let mut b = k.to_bytes();
                b[0x0C..0x20].fill(0);
                put_name(&mut b, 0x0C, name);
                b
            }
            Announce::Disconnect { device, ip } => {
                let mut b = ShortHeader::write(0x08, 0x00, name, 0x29);
                b[0x24] = *device;
                b[0x25..0x29].copy_from_slice(&ip.octets());
                b
            }
            Announce::Reject => ShortHeader::write(0x09, 0x00, name, 0x24),
            Announce::CompatiRequest { kind } => {
                let mut b = ShortHeader::write(0x0A, 0x00, name, 0x25);
                b[0x24] = *kind;
                b
            }
            Announce::Other { kind, subtype } => ShortHeader::write(*kind, *subtype, name, 0x24),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Port 50001: beat

/// Beat packet (type 0x28, 0x60 bytes). Times are milliseconds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Beat {
    pub name: String,
    pub device: u8,
    pub next_beat_ms: u32,
    pub second_beat_ms: u32,
    pub next_bar_ms: u32,
    pub fourth_beat_ms: u32,
    pub second_bar_ms: u32,
    pub eighth_beat_ms: u32,
    /// 0x3C..0x54, not set by rekordbox.
    pub reserved: [u32; 6],
    /// 0x100000 = ±0 %.
    pub pitch: u32,
    /// BPM × 100.
    pub bpm100: u32,
    /// Beat within the bar, 1..=4.
    pub beat_in_bar: u8,
    /// 0x5D.
    pub b5d: u8,
}

impl Beat {
    pub const TYPE: u8 = 0x28;
    pub const LEN: usize = 0x60;

    pub fn parse(b: &[u8]) -> Result<Beat> {
        let h = LongHeader::parse(b)?;
        if h.kind != Self::TYPE {
            return Err(Error::WrongType(h.kind));
        }
        need(b, Self::LEN)?;
        Ok(Beat {
            name: h.name,
            device: h.device,
            next_beat_ms: u32_at(b, 0x24),
            second_beat_ms: u32_at(b, 0x28),
            next_bar_ms: u32_at(b, 0x2C),
            fourth_beat_ms: u32_at(b, 0x30),
            second_bar_ms: u32_at(b, 0x34),
            eighth_beat_ms: u32_at(b, 0x38),
            reserved: std::array::from_fn(|i| u32_at(b, 0x3C + 4 * i)),
            pitch: u32_at(b, 0x54),
            bpm100: u32_at(b, 0x58),
            beat_in_bar: b[0x5C],
            b5d: b[0x5D],
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = LongHeader::write(Self::TYPE, &self.name, self.device, Self::LEN);
        for (o, v) in [
            (0x24, self.next_beat_ms),
            (0x28, self.second_beat_ms),
            (0x2C, self.next_bar_ms),
            (0x30, self.fourth_beat_ms),
            (0x34, self.second_bar_ms),
            (0x38, self.eighth_beat_ms),
            (0x54, self.pitch),
            (0x58, self.bpm100),
        ] {
            put_u32(&mut b, o, v);
        }
        for (i, v) in self.reserved.iter().enumerate() {
            put_u32(&mut b, 0x3C + 4 * i, *v);
        }
        b[0x5C] = self.beat_in_bar;
        b[0x5D] = self.b5d;
        b[0x5F] = self.device;
        b
    }

    /// Effective tempo × 100 (pitch applied).
    pub fn effective_bpm100(&self) -> u32 {
        effective_bpm100(self.pitch, self.bpm100)
    }

    /// The beat packet rekordbox sends for `bpm` on beat `beat_in_bar` (1..=4) of a bar.
    pub fn for_tempo(name: &str, device: u8, bpm: f64, beat_in_bar: u8) -> Beat {
        let (pitch, bpm100) = encode_tempo(bpm);
        let beat = 60000.0 / bpm;
        let b = f64::from(beat_in_bar.clamp(1, 4));
        Beat {
            name: name.to_string(),
            device,
            next_beat_ms: beat as u32,
            second_beat_ms: (beat + beat) as u32,
            next_bar_ms: (beat * (5.0 - b)) as u32,
            fourth_beat_ms: (beat * 4.0) as u32,
            second_bar_ms: (beat * (9.0 - b)) as u32,
            eighth_beat_ms: (beat * 8.0) as u32,
            reserved: [0; 6],
            pitch,
            bpm100,
            beat_in_bar: beat_in_bar.clamp(1, 4),
            b5d: 0,
        }
    }
}

/// trunc(pitch × bpm100 / 2^20).
pub fn effective_bpm100(pitch: u32, bpm100: u32) -> u32 {
    (f64::from(pitch) * f64::from(bpm100) * (1.0 / 1_048_576.0)) as u32
}

/// Pitch in percent (0.0 = normal speed).
pub fn pitch_percent(pitch: u32) -> f64 {
    (f64::from(pitch) / f64::from(PITCH_NORMAL) - 1.0) * 100.0
}

/// (pitch, bpm100) as sent for a tempo: above 20 BPM the tempo goes into bpm100 with normal
/// pitch; at or below 20 BPM bpm100 is fixed at 2001 and the pitch carries the ratio.
pub fn encode_tempo(bpm: f64) -> (u32, u32) {
    if bpm <= 20.0 {
        ((bpm * (1_048_576.0 / 20.01)) as i64 as u32, 2001)
    } else {
        (PITCH_NORMAL, (bpm * 100.0) as u32)
    }
}

// ---------------------------------------------------------------------------------------------
// Port 50002: status

/// CDJ status (type 0x0A). Fields after 0x8B depend on the header sub-version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdjStatus {
    pub name: String,
    pub device: u8,
    pub subversion: u8,
    /// Load info passed on together (bytes 0x28, 0x29, 0x2A).
    pub load_info: [u8; 3],
    /// u32 at 0x2C (passed on with the load info).
    pub track_ref: u32,
    /// u32 at 0x34.
    pub field_34: u32,
    /// Play state code (u32 at 0x78), see [`play_state_group`].
    pub play_state: u32,
    /// Flags (0x89); bits 0x10/0x20 are passed on.
    pub flags: u8,
    /// Sub-version ≥ 1: pitch (0x8C) and BPM × 100 (0x90, bit 31 masked).
    pub tempo: Option<(u32, u32)>,
    /// Sub-version ≥ 1: byte 0x9E.
    pub b9e: Option<u8>,
    /// Sub-version ≥ 1: device number in a sync master handoff (0x9F); `None` for 0 / 0xFF.
    pub master_handoff: Option<u8>,
    /// Sub-version ≥ 2: u32 at 0xA0 and u16 at 0xB4.
    pub ext2: Option<(u32, u16)>,
}

impl CdjStatus {
    pub const TYPE: u8 = 0x0A;
    pub const MIN_LEN: usize = 0x8C;

    pub fn parse(b: &[u8]) -> Result<CdjStatus> {
        let h = LongHeader::parse(b)?;
        if h.kind != Self::TYPE {
            return Err(Error::WrongType(h.kind));
        }
        need(b, Self::MIN_LEN)?;
        let ext1 = h.subversion >= 1 && b.len() >= 0xA0;
        let ext2 = h.subversion >= 2 && b.len() >= 0xB8;
        Ok(CdjStatus {
            name: h.name,
            device: h.device,
            subversion: h.subversion,
            load_info: [b[0x28], b[0x29], b[0x2A]],
            track_ref: u32_at(b, 0x2C),
            field_34: u32_at(b, 0x34),
            play_state: u32_at(b, 0x78),
            flags: b[0x89],
            tempo: ext1.then(|| (u32_at(b, 0x8C), u32_at(b, 0x90) & 0x7FFF_FFFF)),
            b9e: ext1.then(|| b[0x9E]),
            master_handoff: if ext1 && b[0x9F] != 0 && b[0x9F] != 0xFF { Some(b[0x9F]) } else { None },
            ext2: ext2.then(|| (u32_at(b, 0xA0), u16_at(b, 0xB4))),
        })
    }

    /// Effective tempo × 100, unless the BPM field holds the "no value" marker 0x7FFFFFFF.
    pub fn effective_bpm100(&self) -> Option<u32> {
        self.tempo.filter(|&(_, bpm)| bpm != 0x7FFF_FFFF).map(|(p, bpm)| effective_bpm100(p, bpm))
    }
}

/// Mixer / all-in-one status (type 0x29): play states of the two decks n and n + 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixerStatus {
    pub name: String,
    pub device: u8,
    /// (deck number, play state) for decks n and n + 1, when inside the packet.
    pub decks: Vec<(u8, u32)>,
}

impl MixerStatus {
    pub const TYPE: u8 = 0x29;

    /// Offset of the play state of deck `n` (`n · 0x10 − 0x54`).
    pub fn play_state_offset(n: u8) -> Option<usize> {
        (usize::from(n) * 0x10).checked_sub(0x54)
    }

    pub fn parse(b: &[u8]) -> Result<MixerStatus> {
        let h = LongHeader::parse(b)?;
        if h.kind != Self::TYPE {
            return Err(Error::WrongType(h.kind));
        }
        need(b, 0x50)?;
        let decks = [h.device, h.device.wrapping_add(1)]
            .into_iter()
            .filter_map(|n| {
                let o = Self::play_state_offset(n)?;
                (o >= 0x24 && o + 4 <= b.len()).then(|| (n, u32_at(b, o)))
            })
            .collect();
        Ok(MixerStatus { name: h.name, device: h.device, decks })
    }
}

/// Play-state codes the original passes on, with its grouping: {3, 4, 7, 8, 9, 0x12} → 3,
/// {0x0A, 0x0E} → 0x0A, 0/5/6/0x10/0x11 unchanged; other codes → `None` (not passed on).
pub fn play_state_group(code: u32) -> Option<u32> {
    match code {
        3 | 4 | 7 | 8 | 9 | 0x12 => Some(3),
        0x0A | 0x0E => Some(0x0A),
        0 | 5 | 6 | 0x10 | 0x11 => Some(code),
        _ => None,
    }
}

/// Any packet seen on a DJ Link port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    Announce(ShortHeader, Announce),
    Beat(Beat),
    CdjStatus(CdjStatus),
    MixerStatus(MixerStatus),
    /// Long-header packet of a type that is not decoded here.
    Other(LongHeader),
}

/// Decode a datagram received on `port`.
pub fn decode(port: u16, b: &[u8]) -> Result<Packet> {
    if port == PORT_ANNOUNCE {
        let (h, a) = Announce::parse(b)?;
        return Ok(Packet::Announce(h, a));
    }
    let h = LongHeader::parse(b)?;
    Ok(match (port, h.kind) {
        (PORT_BEAT, Beat::TYPE) => Packet::Beat(Beat::parse(b)?),
        (PORT_STATUS, CdjStatus::TYPE) => Packet::CdjStatus(CdjStatus::parse(b)?),
        (PORT_STATUS, MixerStatus::TYPE) => Packet::MixerStatus(MixerStatus::parse(b)?),
        _ => Packet::Other(h),
    })
}

// ---------------------------------------------------------------------------------------------
// Device table

/// One device known from keep-alives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub number: u8,
    pub name: String,
    pub kind: u8,
    pub ip: Ipv4Addr,
    pub mac: [u8; 6],
    /// True for the extra entry of a two-deck unit (number + 1).
    pub second_deck: bool,
}

/// What a keep-alive changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableChange {
    Added(u8),
    Overwritten(u8),
    Refreshed(u8),
    /// Same number as our own device.
    Conflict(u8),
    Invalid,
}

/// Device list built from keep-alives with the original's rules (SPEC §5i "readConfigNotify").
#[derive(Debug, Clone, Default)]
pub struct DeviceTable {
    /// Our own device number, if we take part.
    pub own_number: Option<u8>,
    devices: std::collections::BTreeMap<u8, Device>,
}

impl DeviceTable {
    pub fn new(own_number: Option<u8>) -> DeviceTable {
        DeviceTable { own_number, devices: Default::default() }
    }

    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values()
    }

    pub fn get(&self, number: u8) -> Option<&Device> {
        self.devices.get(&number)
    }

    fn put(&mut self, number: u8, k: &KeepAlive, second_deck: bool) -> TableChange {
        let new = Device { number, name: k.name.clone(), kind: k.kind, ip: k.ip, mac: k.mac, second_deck };
        match self.devices.insert(number, new) {
            None => TableChange::Added(number),
            Some(old) if old.ip != k.ip || old.mac != k.mac => TableChange::Overwritten(number),
            Some(_) => TableChange::Refreshed(number),
        }
    }

    /// Apply a keep-alive; returns one change per touched number (two for numbers 9 and 11).
    pub fn apply(&mut self, k: &KeepAlive) -> Vec<TableChange> {
        if !k.is_valid() {
            return vec![TableChange::Invalid];
        }
        if self.own_number == Some(k.device) {
            return vec![TableChange::Conflict(k.device)];
        }
        let mut out = vec![self.put(k.device, k, false)];
        if k.device == 9 || k.device == 11 {
            out.push(self.put(k.device + 1, k, true));
        }
        out
    }

    /// Remove a device after a disconnect (and its second deck).
    pub fn remove(&mut self, number: u8) {
        self.devices.remove(&number);
        if let Some(d) = self.devices.get(&(number + 1)) {
            if d.second_deck {
                self.devices.remove(&(number + 1));
            }
        }
    }
}
