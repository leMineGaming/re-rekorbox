//! NFS v2 server (ONC-RPC over UDP 2049) so the unit can read the audio file it wants to play.
//!
//! The protocol is described in `docs/link/PRO_DJ_LINK_SPEC.md` §7 (no authentication; `AUTH_UNIX`/
//! `AUTH_NULL`, cleartext). We serve read-only: `NULL`, `GETATTR`, `LOOKUP`, `READ`, plus `STATFS`
//! and `READLINK` for safety. Paths/filenames in the XDR strings are **UTF-16LE** (a Windows NFS
//! server quirk); file handles are opaque 32-byte tokens we choose and the unit echoes back.
//!
//! Root handles match [`crate::rpc::root_handle`] (so a handle from a mountd `MNT` resolves here);
//! handles for files and subdirectories are minted on `LOOKUP` and kept in a table.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::rpc::{opaque_at, put_opaque, put_u32, reply, root_handle, u32_at, Call, SUCCESS};

/// NFS v2 procedures we handle.
mod proc {
    pub const NULL: u32 = 0;
    pub const GETATTR: u32 = 1;
    pub const LOOKUP: u32 = 4;
    pub const READLINK: u32 = 5;
    pub const READ: u32 = 6;
    pub const STATFS: u32 = 17;
}

/// NFSv2 status codes.
const NFS_OK: u32 = 0;
const NFSERR_NOENT: u32 = 2;
const NFSERR_IO: u32 = 5;
const NFSERR_INVAL: u32 = 22;

/// ftype.
const NFREG: u32 = 1;
const NFDIR: u32 = 2;

/// Largest READ we serve (the unit asks for 8192).
const MAX_READ: usize = 8192;

/// A chosen 32-byte file handle.
type Handle = [u8; 32];

/// The exported read-only filesystem: the drives offered over mountd, plus a table of handles minted
/// for files and directories reached by `LOOKUP`.
pub struct Fs {
    /// Drive base paths in the same order as the mountd export list (index = mountd handle index).
    roots: Vec<PathBuf>,
    table: Mutex<HashMap<Handle, PathBuf>>,
    next: AtomicU32,
}

impl Fs {
    pub fn new(drives: &[char]) -> Fs {
        Fs::with_roots(
            drives
                .iter()
                .map(|d| PathBuf::from(format!("{d}:\\")))
                .collect(),
        )
    }

    /// Build from explicit root paths (root handle index = position). Used by tests.
    pub fn with_roots(roots: Vec<PathBuf>) -> Fs {
        Fs {
            roots,
            table: Mutex::new(HashMap::new()),
            next: AtomicU32::new(0),
        }
    }

    /// Resolve a handle to a path: a root handle by position, else the minted table.
    fn resolve(&self, fh: &Handle) -> Option<PathBuf> {
        for i in 0..self.roots.len() {
            if *fh == root_handle(i) {
                return Some(self.roots[i].clone());
            }
        }
        self.table.lock().ok()?.get(fh).cloned()
    }

    /// Mint a fresh handle for `path` (high bit set so it never collides with a root handle).
    fn mint(&self, path: PathBuf) -> Handle {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&(0x8000_0000 | id).to_be_bytes());
        if let Ok(mut t) = self.table.lock() {
            t.insert(h, path);
        }
        h
    }
}

/// Decode an XDR string of UTF-16LE bytes to a `String`.
fn utf16le(b: &[u8]) -> String {
    let units: Vec<u16> = (0..b.len() / 2)
        .map(|i| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// fattr (17 words, 68 bytes) for a path, or `None` if it cannot be read.
fn fattr(path: &std::path::Path) -> Option<Vec<u8>> {
    let m = std::fs::metadata(path).ok()?;
    let dir = m.is_dir();
    let size = m.len().min(u64::from(u32::MAX)) as u32;
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs().min(u64::from(u32::MAX)) as u32);
    let mut v = Vec::with_capacity(68);
    let fields = [
        if dir { NFDIR } else { NFREG },         // type
        if dir { 0o040_666 } else { 0o100_644 }, // mode
        1,                                       // nlink
        0,                                       // uid
        0,                                       // gid
        size,                                    // size
        MAX_READ as u32,                         // blocksize
        0,                                       // rdev
        size.div_ceil(512),                      // blocks
        1,                                       // fsid
        1,                                       // fileid
        mtime,                                   // atime sec
        0,                                       // atime usec
        mtime,                                   // mtime sec
        0,                                       // mtime usec
        mtime,                                   // ctime sec
        0,                                       // ctime usec
    ];
    for f in fields {
        put_u32(&mut v, f);
    }
    Some(v)
}

/// Read a handle (32 bytes) at the start of `args`.
fn handle_at(args: &[u8], off: usize) -> Option<Handle> {
    args.get(off..off + 32)?.try_into().ok()
}

/// Answer an NFS call. Returns the reply datagram and a short label for logging.
pub fn answer(fs: &Fs, c: &Call) -> (Vec<u8>, &'static str) {
    match c.procedure {
        proc::NULL => (reply(c.xid, SUCCESS, &[]), "NULL"),
        proc::GETATTR => match handle_at(c.args, 0).and_then(|h| fs.resolve(&h)) {
            Some(p) => match fattr(&p) {
                Some(a) => (ok(c.xid, &a), "GETATTR"),
                None => (err(c.xid, NFSERR_IO), "GETATTR io"),
            },
            None => (err(c.xid, NFSERR_NOENT), "GETATTR noent"),
        },
        proc::LOOKUP => lookup(fs, c),
        proc::READ => read(fs, c),
        proc::READLINK => (err(c.xid, NFSERR_INVAL), "READLINK"),
        proc::STATFS => {
            // tsize, bsize, blocks, bfree, bavail
            let mut body = Vec::new();
            put_u32(&mut body, NFS_OK);
            for v in [MAX_READ as u32, 512, 0x10_0000, 0, 0] {
                put_u32(&mut body, v);
            }
            (reply(c.xid, SUCCESS, &body), "STATFS")
        }
        _ => (err(c.xid, NFSERR_NOENT), "proc?"),
    }
}

/// `status(0) | <body>` accepted reply.
fn ok(xid: u32, body: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + body.len());
    put_u32(&mut v, NFS_OK);
    v.extend_from_slice(body);
    reply(xid, SUCCESS, &v)
}

/// `status(err)` accepted reply.
fn err(xid: u32, status: u32) -> Vec<u8> {
    reply(xid, SUCCESS, &status.to_be_bytes())
}

fn lookup(fs: &Fs, c: &Call) -> (Vec<u8>, &'static str) {
    let Some(dir) = handle_at(c.args, 0).and_then(|h| fs.resolve(&h)) else {
        return (err(c.xid, NFSERR_NOENT), "LOOKUP noent");
    };
    let Ok((name, _)) = opaque_at(c.args, 32) else {
        return (err(c.xid, NFSERR_IO), "LOOKUP args");
    };
    let child = dir.join(utf16le(name));
    match fattr(&child) {
        Some(a) => {
            let fh = fs.mint(child);
            let mut body = Vec::with_capacity(4 + 32 + a.len());
            put_u32(&mut body, NFS_OK);
            body.extend_from_slice(&fh);
            body.extend_from_slice(&a);
            (reply(c.xid, SUCCESS, &body), "LOOKUP")
        }
        None => (err(c.xid, NFSERR_NOENT), "LOOKUP miss"),
    }
}

fn read(fs: &Fs, c: &Call) -> (Vec<u8>, &'static str) {
    let (Some(path), Ok(offset), Ok(count)) = (
        handle_at(c.args, 0).and_then(|h| fs.resolve(&h)),
        u32_at(c.args, 32),
        u32_at(c.args, 36),
    ) else {
        return (err(c.xid, NFSERR_NOENT), "READ noent");
    };
    let Some(attr) = fattr(&path) else {
        return (err(c.xid, NFSERR_IO), "READ attr");
    };
    let want = (count as usize).min(MAX_READ);
    let mut buf = vec![0u8; want];
    let n = match std::fs::File::open(&path).and_then(|mut f| {
        f.seek(SeekFrom::Start(u64::from(offset)))?;
        read_fully(&mut f, &mut buf)
    }) {
        Ok(n) => n,
        Err(_) => return (err(c.xid, NFSERR_IO), "READ io"),
    };
    let mut body = Vec::with_capacity(4 + attr.len() + 4 + n);
    put_u32(&mut body, NFS_OK);
    body.extend_from_slice(&attr);
    put_opaque(&mut body, &buf[..n]);
    (reply(c.xid, SUCCESS, &body), "READ")
}

/// Read until the buffer is full or EOF; returns the number of bytes read.
fn read_fully(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match f.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(got)
}
