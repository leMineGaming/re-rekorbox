//! ONC RPC over UDP as the players use it before they ask for a computer's media (capture 1, see
//! `docs/link/PHASE1.md`): the portmapper on **UDP 50111** (not 111) and mountd with its export list
//! and MNT. Pure: a datagram in, the answer out. Reading files (nfsd, port 2049) is Phase 5.
//!
//! Export paths are UTF-16LE (`/C/` = `2F 00 43 00 2F 00`), the group is `ip/netmask` of the adapter.

use std::net::Ipv4Addr;

use crate::error::PacketError;

pub const PORT_PORTMAP: u16 = 50111;
pub const PORT_MOUNTD: u16 = 36251;
pub const PORT_NFS: u16 = 2049;

pub mod prog {
    pub const PORTMAP: u32 = 100_000;
    pub const NFS: u32 = 100_003;
    pub const MOUNT: u32 = 100_005;
}

const CALL: u32 = 0;
const REPLY: u32 = 1;
const RPC_VERSION: u32 = 2;
const IPPROTO_UDP: u32 = 17;

/// accept_stat of an accepted reply.
const SUCCESS: u32 = 0;
const PROG_UNAVAIL: u32 = 1;
const PROC_UNAVAIL: u32 = 3;
const GARBAGE_ARGS: u32 = 4;

/// MNT status for an unknown path (NFSERR_NOENT).
const MNT_NOENT: u32 = 2;

/// An RPC call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call<'a> {
    pub xid: u32,
    pub prog: u32,
    pub vers: u32,
    pub procedure: u32,
    pub args: &'a [u8],
}

fn u32_at(b: &[u8], i: usize) -> Result<u32, PacketError> {
    b.get(i..i + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(PacketError::TooShort {
            len: b.len(),
            need: i + 4,
        })
}

/// Opaque/string of XDR at `i`: (bytes, offset after the padding).
fn opaque_at(b: &[u8], i: usize) -> Result<(&[u8], usize), PacketError> {
    let n = u32_at(b, i)? as usize;
    let end = i + 4 + n;
    let data = b.get(i + 4..end).ok_or(PacketError::TooShort {
        len: b.len(),
        need: end,
    })?;
    Ok((data, i + 4 + n.div_ceil(4) * 4))
}

pub fn parse_call(b: &[u8]) -> Result<Call<'_>, PacketError> {
    if u32_at(b, 4)? != CALL || u32_at(b, 8)? != RPC_VERSION {
        return Err(PacketError::Format("not an RPC call".into()));
    }
    // credentials and verifier: flavor + opaque body each
    let (_, after_cred) = opaque_at(b, 28)?;
    let (_, after_verf) = opaque_at(b, after_cred + 4)?;
    Ok(Call {
        xid: u32_at(b, 0)?,
        prog: u32_at(b, 12)?,
        vers: u32_at(b, 16)?,
        procedure: u32_at(b, 20)?,
        args: &b[after_verf..],
    })
}

fn put_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn put_opaque(v: &mut Vec<u8>, data: &[u8]) {
    put_u32(v, data.len() as u32);
    v.extend_from_slice(data);
    v.resize(v.len() + (4 - data.len() % 4) % 4, 0);
}

/// Accepted reply with AUTH_NULL verifier.
fn reply(xid: u32, accept: u32, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(24 + body.len());
    for x in [xid, REPLY, 0, 0, 0, accept] {
        put_u32(&mut v, x);
    }
    v.extend_from_slice(body);
    v
}

/// `/C/` in UTF-16LE.
pub fn export_path(drive: char) -> Vec<u8> {
    format!("/{drive}/")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The 32-byte handle MNT returns for the export at `index` (as rekordbox: C = zeros, D = 1, 1, 1).
pub fn root_handle(index: usize) -> [u8; 32] {
    let mut h = [0u8; 32];
    for k in 0..3 {
        h[k * 4..k * 4 + 4].copy_from_slice(&(index as u32).to_be_bytes());
    }
    h
}

/// What the computer offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exports {
    /// Drive letters, e.g. `['C', 'D']`.
    pub drives: Vec<char>,
    /// Adapter address and netmask for the export group.
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
}

impl Exports {
    fn export_list(&self) -> Vec<u8> {
        let group = format!("{}/{}", self.ip, self.netmask);
        let mut v = Vec::new();
        for d in &self.drives {
            put_u32(&mut v, 1);
            put_opaque(&mut v, &export_path(*d));
            put_u32(&mut v, 1);
            put_opaque(&mut v, group.as_bytes());
            put_u32(&mut v, 0);
        }
        put_u32(&mut v, 0);
        v
    }

    fn mount(&self, args: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        let Ok((path, _)) = opaque_at(args, 0) else {
            put_u32(&mut v, MNT_NOENT);
            return v;
        };
        match self.drives.iter().position(|d| export_path(*d) == path) {
            Some(i) => {
                put_u32(&mut v, 0);
                v.extend_from_slice(&root_handle(i));
            }
            None => put_u32(&mut v, MNT_NOENT),
        }
        v
    }
}

/// Which RPC a datagram was (for logging).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    GetPort { prog: u32, vers: u32, port: u16 },
    Export,
    Mount { ok: bool },
    Other { prog: u32, procedure: u32 },
}

/// Answer a datagram that came in on the portmapper or mountd port. `None` for anything that is not an
/// RPC call (no answer, like a real server).
pub fn answer(exports: &Exports, b: &[u8]) -> Option<(Vec<u8>, Kind)> {
    let c = parse_call(b).ok()?;
    let other = Kind::Other {
        prog: c.prog,
        procedure: c.procedure,
    };
    let out = match (c.prog, c.procedure) {
        (prog::PORTMAP, 0) | (prog::MOUNT, 0) => (reply(c.xid, SUCCESS, &[]), other),
        (prog::PORTMAP, 3) => {
            let (Ok(p), Ok(vers), Ok(prot)) =
                (u32_at(c.args, 0), u32_at(c.args, 4), u32_at(c.args, 8))
            else {
                return Some((reply(c.xid, GARBAGE_ARGS, &[]), other));
            };
            let port = match (p, vers, prot) {
                (prog::MOUNT, 1, IPPROTO_UDP) => PORT_MOUNTD,
                (prog::NFS, 2, IPPROTO_UDP) => PORT_NFS,
                _ => 0,
            };
            (
                reply(c.xid, SUCCESS, &u32::from(port).to_be_bytes()),
                Kind::GetPort {
                    prog: p,
                    vers,
                    port,
                },
            )
        }
        (prog::PORTMAP, _) => (reply(c.xid, PROC_UNAVAIL, &[]), other),
        (prog::MOUNT, 1) => {
            let body = exports.mount(c.args);
            let ok = body.get(..4) == Some(&[0, 0, 0, 0][..]);
            (reply(c.xid, SUCCESS, &body), Kind::Mount { ok })
        }
        // DUMP: empty list; UMNT, UMNTALL: void
        (prog::MOUNT, 2) => (reply(c.xid, SUCCESS, &[0, 0, 0, 0]), other),
        (prog::MOUNT, 3) | (prog::MOUNT, 4) => (reply(c.xid, SUCCESS, &[]), other),
        (prog::MOUNT, 5) => (reply(c.xid, SUCCESS, &exports.export_list()), Kind::Export),
        (prog::MOUNT, _) => (reply(c.xid, PROC_UNAVAIL, &[]), other),
        _ => (reply(c.xid, PROG_UNAVAIL, &[]), other),
    };
    Some(out)
}
