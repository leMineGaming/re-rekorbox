//! NFS v2 server: LOOKUP/READ/GETATTR against a temp directory, UTF-16LE names, 32-byte handles.

use std::path::PathBuf;

use rbx_link::nfs::{answer, Fs};
use rbx_link::rpc::{parse_call, root_handle};

/// Build an AUTH_NULL RPC call datagram for NFS (prog 100003 v2).
fn call(xid: u32, procedure: u32, args: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [xid, 0, 2, 100_003, 2, procedure, 0, 0, 0, 0] {
        v.extend_from_slice(&x.to_be_bytes());
    }
    v.extend_from_slice(args);
    v
}

/// XDR string of a UTF-16LE filename.
fn name(s: &str) -> Vec<u8> {
    let bytes: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut v = (bytes.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(&bytes);
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
    v
}

/// Run one call and return the reply body after the RPC accepted-reply header (24 bytes).
fn run(fs: &Fs, xid: u32, procedure: u32, args: &[u8]) -> Vec<u8> {
    let dg = call(xid, procedure, args);
    let c = parse_call(&dg).unwrap();
    let (reply, _) = answer(fs, &c);
    assert_eq!(&reply[..4], &xid.to_be_bytes(), "reply echoes xid");
    reply[24..].to_vec()
}

fn u32_be(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn tempdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rbxnfs-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("Music")).unwrap();
    std::fs::write(
        d.join("Music").join("Track.flac"),
        b"fLaC\x00\x01\x02\x03hello world",
    )
    .unwrap();
    d
}

#[test]
fn lookup_read_roundtrip() {
    let dir = tempdir("rt");
    let fs = Fs::with_roots(vec![dir.clone()]);
    let root = root_handle(0);

    // GETATTR on the root → directory (type 2)
    let a = run(&fs, 1, 1, &root);
    assert_eq!(u32_be(&a, 0), 0, "status ok");
    assert_eq!(u32_be(&a, 4), 2, "root is a directory");

    // LOOKUP "Music" under root
    let mut args = root.to_vec();
    args.extend_from_slice(&name("Music"));
    let a = run(&fs, 2, 4, &args);
    assert_eq!(u32_be(&a, 0), 0, "found Music");
    let music_fh = &a[4..36];
    assert_eq!(u32_be(&a[36..], 0), 2, "Music is a directory");

    // LOOKUP "Track.flac" under Music
    let mut args = music_fh.to_vec();
    args.extend_from_slice(&name("Track.flac"));
    let a = run(&fs, 3, 4, &args);
    assert_eq!(u32_be(&a, 0), 0, "found the track");
    let file_fh = a[4..36].to_vec();
    assert_eq!(u32_be(&a[36..], 0), 1, "it is a regular file");
    let size = u32_be(&a[36..], 20); // fattr word 5 = size
    assert_eq!(size as usize, b"fLaC\x00\x01\x02\x03hello world".len());

    // READ the file from offset 0
    let mut args = file_fh.clone();
    args.extend_from_slice(&0u32.to_be_bytes()); // offset
    args.extend_from_slice(&8192u32.to_be_bytes()); // count
    args.extend_from_slice(&0u32.to_be_bytes()); // total
    let a = run(&fs, 4, 6, &args);
    assert_eq!(u32_be(&a, 0), 0, "read ok");
    // after status(4) + fattr(68) comes the data opaque (len + bytes)
    let dlen = u32_be(&a, 4 + 68) as usize;
    let data = &a[4 + 68 + 4..4 + 68 + 4 + dlen];
    assert_eq!(data, b"fLaC\x00\x01\x02\x03hello world");
    assert_eq!(&data[..4], b"fLaC", "starts with the FLAC header");

    // READ from a non-zero offset
    let mut args = file_fh;
    args.extend_from_slice(&8u32.to_be_bytes());
    args.extend_from_slice(&5u32.to_be_bytes());
    args.extend_from_slice(&0u32.to_be_bytes());
    let a = run(&fs, 5, 6, &args);
    let dlen = u32_be(&a, 4 + 68) as usize;
    assert_eq!(&a[4 + 68 + 4..4 + 68 + 4 + dlen], b"hello");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_name_is_noent() {
    let dir = tempdir("miss");
    let fs = Fs::with_roots(vec![dir.clone()]);
    let mut args = root_handle(0).to_vec();
    args.extend_from_slice(&name("Nope"));
    let a = run(&fs, 1, 4, &args);
    assert_eq!(u32_be(&a, 0), 2, "NFSERR_NOENT");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn null_and_statfs() {
    let fs = Fs::with_roots(vec![std::env::temp_dir()]);
    assert!(run(&fs, 1, 0, &[]).is_empty(), "NULL has an empty body");
    let a = run(&fs, 2, 17, &root_handle(0));
    assert_eq!(u32_be(&a, 0), 0, "statfs ok");
}
