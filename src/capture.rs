//! Reading network captures (pcap / pcapng, Ethernet + IPv4) and summarising Link Export traffic:
//! the database port query and messages (SPEC §5k), ONC RPC calls (portmap, mount, NFS) and Pro DJ
//! Link packets. Used to compare rekordbox ↔ player traffic with the own implementation (U33).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::Ipv4Addr;

use crate::dbserver::{self, Message};

/// One IPv4 packet with a TCP or UDP payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    /// Seconds since the first packet.
    pub time: f64,
    pub src: (Ipv4Addr, u16),
    pub dst: (Ipv4Addr, u16),
    pub tcp: bool,
    /// TCP sequence number (0 for UDP).
    pub seq: u32,
    pub syn: bool,
    pub payload: Vec<u8>,
}

fn u16be(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

fn u32be(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Ethernet (optionally VLAN) → IPv4 → TCP/UDP.
fn parse_frame(f: &[u8], linktype: u32, time: f64) -> Option<Packet> {
    let ip = match linktype {
        1 => {
            let mut et = u16be(f, 12)?;
            let mut off = 14;
            if et == 0x8100 {
                et = u16be(f, 16)?;
                off = 18;
            }
            if et != 0x0800 {
                return None;
            }
            f.get(off..)?
        }
        101 | 228 => f, // raw IPv4
        _ => return None,
    };
    if ip.first()? >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(ip[0] & 0x0F) * 4;
    let total = usize::from(u16be(ip, 2)?).min(ip.len());
    let proto = *ip.get(9)?;
    let src = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
    let dst = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
    let l4 = ip.get(ihl..total)?;
    match proto {
        6 => {
            let off = usize::from(l4.get(12)? >> 4) * 4;
            Some(Packet {
                time,
                src: (src, u16be(l4, 0)?),
                dst: (dst, u16be(l4, 2)?),
                tcp: true,
                seq: u32be(l4, 4)?,
                syn: l4.get(13)? & 0x02 != 0,
                payload: l4.get(off..)?.to_vec(),
            })
        }
        17 => Some(Packet { time, src: (src, u16be(l4, 0)?), dst: (dst, u16be(l4, 2)?), tcp: false, seq: 0, syn: false, payload: l4.get(8..)?.to_vec() }),
        _ => None,
    }
}

/// Packets of a pcap or pcapng file.
pub fn read(data: &[u8]) -> Result<Vec<Packet>, String> {
    let magic = u32::from_le_bytes(data.get(..4).ok_or("empty file")?.try_into().unwrap());
    let mut out = Vec::new();
    let mut t0: Option<f64> = None;
    let mut push = |raw: &[u8], linktype: u32, t: f64, out: &mut Vec<Packet>| {
        let base = *t0.get_or_insert(t);
        if let Some(p) = parse_frame(raw, linktype, t - base) {
            out.push(p);
        }
    };
    match magic {
        0xA1B2_C3D4 | 0xA1B2_3C4D => {
            let nano = magic == 0xA1B2_3C4D;
            let linktype = u32::from_le_bytes(data.get(20..24).ok_or("short header")?.try_into().unwrap());
            let mut p = 24;
            while p + 16 <= data.len() {
                let sec = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
                let frac = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap());
                let len = u32::from_le_bytes(data[p + 8..p + 12].try_into().unwrap()) as usize;
                let t = f64::from(sec) + f64::from(frac) / if nano { 1e9 } else { 1e6 };
                let raw = data.get(p + 16..p + 16 + len).ok_or("truncated packet")?;
                push(raw, linktype, t, &mut out);
                p += 16 + len;
            }
        }
        0x0A0D_0D0A => {
            let mut p = 0;
            let mut linktypes: Vec<(u32, f64)> = Vec::new();
            while p + 12 <= data.len() {
                let btype = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
                let blen = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap()) as usize;
                if blen < 12 || p + blen > data.len() {
                    break;
                }
                let body = &data[p + 8..p + blen - 4];
                match btype {
                    1 => {
                        // interface: link type, options (if_tsresol)
                        let lt = u32::from(u16::from_le_bytes([body[0], body[1]]));
                        let mut res = 1e-6;
                        let mut o = 8;
                        while o + 4 <= body.len() {
                            let code = u16::from_le_bytes([body[o], body[o + 1]]);
                            let len = usize::from(u16::from_le_bytes([body[o + 2], body[o + 3]]));
                            if code == 0 {
                                break;
                            }
                            if code == 9 && len >= 1 {
                                let v = body[o + 4];
                                res = if v & 0x80 == 0 { 10f64.powi(-i32::from(v)) } else { 2f64.powi(-i32::from(v & 0x7F)) };
                            }
                            o += 4 + len.div_ceil(4) * 4;
                        }
                        linktypes.push((lt, res));
                    }
                    6 => {
                        let iface = u32::from_le_bytes(body[0..4].try_into().unwrap()) as usize;
                        let ts = (u64::from(u32::from_le_bytes(body[4..8].try_into().unwrap())) << 32) | u64::from(u32::from_le_bytes(body[8..12].try_into().unwrap()));
                        let caplen = u32::from_le_bytes(body[12..16].try_into().unwrap()) as usize;
                        let (lt, res) = linktypes.get(iface).copied().unwrap_or((1, 1e-6));
                        if let Some(raw) = body.get(20..20 + caplen) {
                            push(raw, lt, ts as f64 * res, &mut out);
                        }
                    }
                    _ => {}
                }
                p += blen;
            }
        }
        _ => return Err(format!("not a pcap/pcapng file (magic {magic:#x})")),
    }
    Ok(out)
}

/// One direction of a TCP connection, reassembled by sequence number.
#[derive(Debug, Clone, Default)]
pub struct Stream {
    pub start: f64,
    pub data: Vec<u8>,
}

/// Reassemble TCP payloads per (src, dst).
pub fn tcp_streams(packets: &[Packet]) -> BTreeMap<((Ipv4Addr, u16), (Ipv4Addr, u16)), Stream> {
    let mut segs: BTreeMap<_, Vec<&Packet>> = BTreeMap::new();
    for p in packets.iter().filter(|p| p.tcp) {
        segs.entry((p.src, p.dst)).or_default().push(p);
    }
    let mut out = BTreeMap::new();
    for (k, ps) in segs {
        let Some(first) = ps.iter().find(|p| p.syn).or(ps.first()) else { continue };
        let base = first.seq.wrapping_add(u32::from(first.syn));
        let mut chunks: Vec<(u32, &[u8])> = ps.iter().filter(|p| !p.payload.is_empty()).map(|p| (p.seq.wrapping_sub(base), p.payload.as_slice())).collect();
        chunks.sort_by_key(|c| c.0);
        let mut data: Vec<u8> = Vec::new();
        for (off, d) in chunks {
            let off = off as usize;
            if off > data.len() + 1_000_000 {
                continue; // a different connection reusing the ports, or garbage
            }
            if off + d.len() <= data.len() {
                continue; // retransmission
            }
            if off > data.len() {
                data.resize(off, 0); // missing segment
            }
            data.extend_from_slice(&d[data.len() - off..]);
        }
        out.insert(k, Stream { start: ps[0].time, data });
    }
    out
}

/// ONC RPC call summary (portmap 100000, NFS 100003, mount 100005).
fn rpc_summary(p: &[u8]) -> Option<String> {
    let xid = u32be(p, 0)?;
    let mtype = u32be(p, 4)?;
    if mtype == 1 {
        let stat = u32be(p, 8)?;
        return Some(format!("RPC reply xid {xid:#x} status {stat}"));
    }
    if mtype != 0 || u32be(p, 8)? != 2 {
        return None;
    }
    let (prog, vers, proc_) = (u32be(p, 12)?, u32be(p, 16)?, u32be(p, 20)?);
    // skip credentials and verifier
    let cred_len = u32be(p, 28)? as usize;
    let verf = 32 + cred_len.div_ceil(4) * 4;
    let args = p.get(verf + 8 + (u32be(p, verf + 4)? as usize).div_ceil(4) * 4..)?;
    let string_at = |b: &[u8], i: usize| -> Option<String> {
        let n = u32be(b, i)? as usize;
        Some(String::from_utf8_lossy(b.get(i + 4..i + 4 + n)?).into_owned())
    };
    let what = match (prog, proc_) {
        (100000, 3) => format!("PORTMAP GETPORT prog {} vers {}", u32be(args, 0)?, u32be(args, 4)?),
        (100000, p) => format!("PORTMAP proc {p}"),
        (100005, 1) => format!("MOUNT MNT {:?}", string_at(args, 0)?),
        (100005, p) => format!("MOUNT proc {p}"),
        (100003, 4) => format!("NFS LOOKUP {:?}", string_at(args, 32).unwrap_or_default()),
        (100003, 6) => format!("NFS READ offset {} count {}", u32be(args, 32)?, u32be(args, 36)?),
        (100003, 1) => "NFS GETATTR".into(),
        (100003, 16) => "NFS READDIR".into(),
        (100003, p) => format!("NFS proc {p}"),
        (prog, p) => format!("RPC prog {prog} proc {p}"),
    };
    Some(format!("{what} (v{vers}, xid {xid:#x})"))
}

/// Text report of a capture: TCP streams (port query, database messages), RPC calls, DJ Link packet
/// types per sender.
pub fn report(packets: &[Packet]) -> String {
    let mut r = String::new();
    let _ = writeln!(r, "{} IPv4 packets", packets.len());
    let streams = tcp_streams(packets);
    let mut db_streams = 0;
    for ((src, dst), s) in &streams {
        if s.data.is_empty() {
            continue;
        }
        let head = format!("\n== TCP {}:{} -> {}:{} at {:.3} s, {} bytes", src.0, src.1, dst.0, dst.1, s.start, s.data.len());
        if s.data.starts_with(dbserver::PORT_QUERY_REQUEST) {
            let _ = writeln!(r, "{head}\nport query (RemoteDBServer)");
        } else if dst.1 == dbserver::PORT_QUERY || src.1 == dbserver::PORT_QUERY {
            let _ = writeln!(r, "{head}\nport query/answer: {:02X?}", &s.data[..s.data.len().min(32)]);
        } else if s.data.starts_with(&dbserver::GREETING) {
            db_streams += 1;
            let _ = writeln!(r, "{head}\ndatabase connection (greeting)");
            let (msgs, used, err) = dbserver::decode_stream(&s.data[5..]);
            for m in &msgs {
                let _ = writeln!(r, "{}", describe(m));
            }
            if let Some(e) = err {
                let _ = writeln!(r, "!! decode error after {used} bytes: {e}");
            }
        } else {
            let (msgs, _, _) = dbserver::decode_stream(&s.data);
            if msgs.is_empty() {
                let _ = writeln!(r, "{head}\nfirst bytes: {:02X?}", &s.data[..s.data.len().min(24)]);
            } else {
                db_streams += 1;
                let _ = writeln!(r, "{head}\ndatabase messages");
                for m in &msgs {
                    let _ = writeln!(r, "{}", describe(m));
                }
            }
        }
    }
    let _ = writeln!(r, "\n{db_streams} database streams");
    let _ = writeln!(r, "\n== UDP");
    let mut djlink: BTreeMap<(Ipv4Addr, u16, u8), usize> = BTreeMap::new();
    for p in packets.iter().filter(|p| !p.tcp) {
        if (50000..=50004).contains(&p.dst.1) && p.payload.starts_with(b"Qspt1WmJOL") {
            *djlink.entry((p.src.0, p.dst.1, p.payload.get(10).copied().unwrap_or(0))).or_default() += 1;
        } else if let Some(s) = rpc_summary(&p.payload) {
            let _ = writeln!(r, "{:9.3} {}:{} -> {}:{}  {s}", p.time, p.src.0, p.src.1, p.dst.0, p.dst.1);
        }
    }
    let _ = writeln!(r, "\nDJ Link packets (sender, port, type: count):");
    for ((ip, port, t), n) in djlink {
        let _ = writeln!(r, "  {ip} {port} {t:#04x}: {n}");
    }
    r
}

/// One database message as a line: transaction, type name, arguments.
pub fn describe(m: &Message) -> String {
    format!("  tx {:#010x} {:#06x} {:<30} {:?}", m.tx, m.kind, dbserver::kind::name(m.kind), m.args)
}
