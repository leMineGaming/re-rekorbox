//! Link Export database protocol: field encodings and messages (SPEC §5k).

use rekordbox_formats::dbserver::{decode_stream, dmst, kind, menu_item, split_dmst, Arg, DbError, Message, GREETING, MAGIC, PORT_QUERY_REQUEST};

#[test]
fn header_layout() {
    let m = Message::new(7, kind::RET_DWORD, vec![Arg::Num(0x2002), Arg::Num(11)]);
    let b = m.encode();
    // magic, tx, type, argc, 12-byte tag blob
    assert_eq!(&b[..5], &[0x11, 0x87, 0x23, 0x49, 0xAE]);
    assert_eq!(&b[5..10], &[0x11, 0, 0, 0, 7]);
    assert_eq!(&b[10..13], &[0x10, 0x40, 0x00]);
    assert_eq!(&b[13..15], &[0x0F, 2]);
    assert_eq!(&b[15..20], &[0x14, 0, 0, 0, 12]);
    assert_eq!(&b[20..32], &[6, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&b[32..], &[0x11, 0, 0, 0x20, 0x02, 0x11, 0, 0, 0, 11]);
    assert_eq!(Message::decode(&b).unwrap(), Some((m, b.len())));
    assert_eq!(MAGIC, 0x8723_49AE);
}

#[test]
fn strings_are_utf16be_with_nul() {
    let m = menu_item(3, 1, 42, "Käse", "Artist", 0x0704, 0, 5);
    let b = m.encode();
    // label 1: type 0x26, 5 units incl. NUL, UTF-16BE
    let i = b.windows(5).position(|w| w == [0x26, 0, 0, 0, 5]).unwrap();
    assert_eq!(&b[i + 5..i + 15], &[0, b'K', 0, 0xE4, 0, b's', 0, b'e', 0, 0]);
    // its byte length argument
    assert_eq!(m.args[2], Arg::Num(10));
    let (back, n) = Message::decode(&b).unwrap().unwrap();
    assert_eq!(n, b.len());
    assert_eq!(back, m);
    assert_eq!(back.args.len(), 12);
}

#[test]
fn empty_blob_is_not_sent() {
    let m = Message::new(9, kind::RET_QTZ, vec![Arg::Num(0x2204), Arg::Num(0), Arg::Num(0), Arg::Blob(vec![]), Arg::Num(0)]);
    let b = m.encode();
    assert!(!b[32..].contains(&0x14), "no blob field after the header");
    assert_eq!(Message::decode(&b).unwrap().unwrap().0, m);
    let full = Message::new(9, kind::RET_QTZ, vec![Arg::Num(0x2204), Arg::Num(0), Arg::Num(3), Arg::Blob(vec![1, 2, 3]), Arg::Num(0)]);
    assert_eq!(Message::decode(&full.encode()).unwrap().unwrap().0, full);
}

#[test]
fn stream_split_and_partial_input() {
    let a = Message::new(1, kind::ROOT_MENU, vec![Arg::Num(dmst(2, 1, 4, 1)), Arg::Num(0)]);
    let b = Message::new(2, kind::LEFT_BUF, (0..6).map(Arg::Num).collect());
    let mut bytes = a.encode();
    bytes.extend(b.encode());
    let (msgs, used, err) = decode_stream(&bytes);
    assert_eq!((msgs, used, err), (vec![a.clone(), b], bytes.len(), None));
    // incomplete input: nothing decoded yet
    let half = &bytes[..10];
    assert_eq!(Message::decode(half), Ok(None));
    let (msgs, used, _) = decode_stream(&bytes[..bytes.len() - 3]);
    assert_eq!((msgs.len(), used), (1, a.encode().len()));
    // wrong magic
    assert!(matches!(Message::decode(&[0x11, 1, 2, 3, 4]), Err(DbError::BadMagic(_))));
}

#[test]
fn dmst_and_constants() {
    let v = dmst(3, 8, 4, 1);
    assert_eq!(v, 0x0308_0401);
    assert_eq!(split_dmst(v), (3, 8, 4, 1));
    assert_eq!(GREETING, [0x11, 0, 0, 0, 1]);
    assert_eq!(&PORT_QUERY_REQUEST[4..18], b"RemoteDBServer");
    assert_eq!(kind::name(0x2c04), "CMD_GET_SPECIFIED_ATOM_INFO");
}

/// Classic pcap with Ethernet/IPv4/TCP frames carrying `payloads` from 10.0.0.2:5000 to 10.0.0.1:1051.
fn pcap(payloads: &[&[u8]]) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&0xA1B2_C3D4u32.to_le_bytes());
    f.extend_from_slice(&[2, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 1, 0, 0, 0]);
    let mut seq = 1000u32;
    for (i, p) in payloads.iter().enumerate() {
        let mut tcp = vec![0x13, 0x88, 0x04, 0x1B];
        tcp.extend_from_slice(&seq.to_be_bytes());
        tcp.extend_from_slice(&[0, 0, 0, 0, 0x50, 0x18, 0xff, 0xff, 0, 0, 0, 0]);
        tcp.extend_from_slice(p);
        let total = 20 + tcp.len();
        let mut ip = vec![0x45, 0, (total >> 8) as u8, total as u8, 0, 0, 0, 0, 64, 6, 0, 0, 10, 0, 0, 2, 10, 0, 0, 1];
        ip.extend_from_slice(&tcp);
        let mut eth = vec![0u8; 12];
        eth.extend_from_slice(&[0x08, 0x00]);
        eth.extend_from_slice(&ip);
        f.extend_from_slice(&(i as u32).to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&(eth.len() as u32).to_le_bytes());
        f.extend_from_slice(&(eth.len() as u32).to_le_bytes());
        f.extend_from_slice(&eth);
        seq += p.len() as u32;
    }
    f
}

#[test]
fn capture_report_decodes_database_streams() {
    let setup = Message::new(0xFFFF_FFFE, kind::PLAYER_ID, vec![Arg::Num(2)]).encode();
    let menu = Message::new(1, kind::TRACK_ROOT, vec![Arg::Num(dmst(2, 1, 4, 1)), Arg::Num(0)]).encode();
    // the message split over two segments
    let (a, b) = menu.split_at(9);
    let file = pcap(&[&GREETING, &setup, a, b]);
    let packets = rekordbox_formats::capture::read(&file).unwrap();
    assert_eq!(packets.len(), 4);
    let r = rekordbox_formats::capture::report(&packets);
    assert!(r.contains("database connection"), "{r}");
    assert!(r.contains("CMD_INFO_PLAYER_ID") && r.contains("CMD_TRACK_ROOT"), "{r}");
}
