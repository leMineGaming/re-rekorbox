//! Pro DJ Link packet tests on synthetic packets. Rules come from SPEC.md §5i.

use rekordbox_formats::djlink::*;
use std::net::Ipv4Addr;

const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0xAA, 0xBB, 0xCC];

fn keep_alive(device: u8, kind: u8, ip: [u8; 4]) -> KeepAlive {
    KeepAlive { name: "TEST-DEV".into(), device, b25: 1, mac: MAC, ip: Ipv4Addr::from(ip), peers: 2, kind, b35: 8 }
}

/// Hand-made long-header packet: magic, type, name, version 1, sub-version, device, length.
fn long_packet(kind: u8, subversion: u8, device: u8, len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    b[..10].copy_from_slice(b"Qspt1WmJOL");
    b[0x0A] = kind;
    b[0x0B..0x0B + 7].copy_from_slice(b"TESTCDJ");
    b[0x1F] = 1;
    b[0x20] = subversion;
    b[0x21] = device;
    b[0x22..0x24].copy_from_slice(&((len - 0x24) as u16).to_be_bytes());
    b
}

#[test]
fn keep_alive_layout() {
    let k = keep_alive(9, 3, [192, 168, 1, 50]);
    let b = k.to_bytes();
    assert_eq!(b.len(), 0x36);
    assert_eq!(&b[..10], b"Qspt1WmJOL");
    assert_eq!((b[0x0A], b[0x0B]), (0x06, 0x00));
    assert_eq!(&b[0x0C..0x14], b"TEST-DEV");
    assert_eq!((b[0x20], b[0x21]), (0x01, 0x03));
    assert_eq!(u16::from_be_bytes([b[0x22], b[0x23]]), 0x36, "port 50000: total length");
    assert_eq!(b[0x24], 9);
    assert_eq!(&b[0x26..0x2C], &MAC);
    assert_eq!(&b[0x2C..0x30], &[192, 168, 1, 50]);
    assert_eq!((b[0x30], b[0x34], b[0x35]), (2, 3, 8));
    let (h, a) = Announce::parse(&b).unwrap();
    assert_eq!(h.name, "TEST-DEV");
    assert_eq!(a, Announce::KeepAlive(k));
}

#[test]
fn announce_round_trips() {
    let ip = Ipv4Addr::new(10, 0, 0, 7);
    for a in [
        Announce::Discovery { counter: 2, kind: KIND_REKORDBOX, mac: MAC },
        Announce::IdRequest { set: false, ip, mac: MAC, device: 0x11, counter: 1, kind: 4 },
        Announce::IdRequest { set: true, ip, mac: MAC, device: 0x11, counter: 3, kind: 4 },
        Announce::IdResponse { set: false, device: 0x11, counter: 1, status: 1 },
        Announce::Disconnect { device: 0x11, ip },
        Announce::CompatiRequest { kind: 4 },
    ] {
        let b = a.to_bytes("rekordbox");
        assert_eq!(usize::from(u16::from_be_bytes([b[0x22], b[0x23]])), b.len());
        let (h, back) = Announce::parse(&b).unwrap();
        assert_eq!((h.name.as_str(), back), ("rekordbox", a));
    }
    let b = Announce::IdRequest { set: true, ip, mac: MAC, device: 0x11, counter: 3, kind: 4 }.to_bytes("rekordbox");
    assert_eq!((b.len(), b[0x0A], b[0x0B], b[0x31]), (0x32, 2, 1, 1));
}

#[test]
fn header_checks() {
    let mut b = keep_alive(1, 1, [1, 2, 3, 4]).to_bytes();
    b[0] = b'X';
    assert_eq!(Announce::parse(&b), Err(Error::BadMagic));
    assert!(matches!(Announce::parse(&b[..0x20]), Err(Error::TooShort { .. })));
    let mut s = long_packet(0x0A, 0, 2, 0x8C);
    s[0x1F] = 2;
    assert_eq!(CdjStatus::parse(&s), Err(Error::UnsupportedVersion(2)), "receivers drop version >= 2");
}

#[test]
fn beat_packet() {
    // 128 BPM, third beat of the bar
    let beat = Beat::for_tempo("rekordbox", 0x11, 128.0, 3);
    assert_eq!(beat.next_beat_ms, 468, "60000 / 128 = 468.75");
    assert_eq!(beat.second_beat_ms, 937);
    assert_eq!(beat.next_bar_ms, 937, "two beats to the next bar");
    assert_eq!(beat.second_bar_ms, 2812, "six beats");
    assert_eq!((beat.fourth_beat_ms, beat.eighth_beat_ms), (1875, 3750));
    assert_eq!((beat.pitch, beat.bpm100), (PITCH_NORMAL, 12800));
    let b = beat.to_bytes();
    assert_eq!((b.len(), b[0x0A], b[0x21], b[0x5F], b[0x5C]), (0x60, 0x28, 0x11, 0x11, 3));
    assert_eq!(u16::from_be_bytes([b[0x22], b[0x23]]), 0x3C, "long header: length after 0x24");
    assert_eq!(Beat::parse(&b).unwrap(), beat);
    assert_eq!(decode(PORT_BEAT, &b).unwrap(), Packet::Beat(beat));
}

#[test]
fn tempo_encoding() {
    assert_eq!(effective_bpm100(PITCH_NORMAL, 12800), 12800);
    assert_eq!(effective_bpm100(0x12_0000, 12000), 13500, "+12.5 %");
    assert_eq!(effective_bpm100(PITCH_NORMAL + PITCH_NORMAL / 10, 12000), 13199, "truncated, not rounded");
    assert!((pitch_percent(PITCH_NORMAL - PITCH_NORMAL / 20) + 5.0).abs() < 1e-3);
    let (p, bpm) = encode_tempo(10.0);
    assert_eq!(bpm, 2001, "slow tempos use a fixed 20.01 BPM");
    assert_eq!(effective_bpm100(p, bpm), 999, "10 BPM, truncated");
    assert_eq!(encode_tempo(0.0), (0, 2001));
}

#[test]
fn cdj_status_fields_and_subversions() {
    let mut b = long_packet(0x0A, 0, 3, 0x8C);
    b[0x28..0x2B].copy_from_slice(&[3, 2, 1]);
    b[0x2C..0x30].copy_from_slice(&1234u32.to_be_bytes());
    b[0x78..0x7C].copy_from_slice(&5u32.to_be_bytes());
    b[0x89] = 0x30;
    let s = CdjStatus::parse(&b).unwrap();
    assert_eq!((s.device, s.load_info, s.track_ref, s.play_state, s.flags), (3, [3, 2, 1], 1234, 5, 0x30));
    assert_eq!((s.tempo, s.master_handoff, s.ext2), (None, None, None), "sub-version 0: no optional fields");

    let mut b = long_packet(0x0A, 2, 3, 0xB8);
    b[0x8C..0x90].copy_from_slice(&PITCH_NORMAL.to_be_bytes());
    b[0x90..0x94].copy_from_slice(&(0x8000_0000u32 | 12450).to_be_bytes());
    b[0x9F] = 2;
    b[0xA0..0xA4].copy_from_slice(&7u32.to_be_bytes());
    b[0xB4..0xB6].copy_from_slice(&0x0102u16.to_be_bytes());
    let s = CdjStatus::parse(&b).unwrap();
    assert_eq!(s.tempo, Some((PITCH_NORMAL, 12450)), "bit 31 of the BPM is masked");
    assert_eq!(s.effective_bpm100(), Some(12450));
    assert_eq!((s.master_handoff, s.ext2), (Some(2), Some((7, 0x0102))));
    b[0x9F] = 0xFF;
    b[0x90..0x94].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
    let s = CdjStatus::parse(&b).unwrap();
    assert_eq!((s.master_handoff, s.effective_bpm100()), (None, None));
    assert!(matches!(CdjStatus::parse(&b[..0x80]), Err(Error::TooShort { .. })));
}

#[test]
fn mixer_status_two_decks() {
    let mut b = long_packet(0x29, 0, 9, 0x50);
    b[0x3C..0x40].copy_from_slice(&3u32.to_be_bytes());
    b[0x4C..0x50].copy_from_slice(&5u32.to_be_bytes());
    assert_eq!(MixerStatus::play_state_offset(9), Some(0x3C));
    match decode(PORT_STATUS, &b).unwrap() {
        Packet::MixerStatus(m) => assert_eq!(m.decks, vec![(9, 3), (10, 5)]),
        other => panic!("{other:?}"),
    }
    // a device number whose deck offsets fall outside the packet yields no decks
    let b = long_packet(0x29, 0, 0x21, 0x50);
    assert!(MixerStatus::parse(&b).unwrap().decks.is_empty());
}

#[test]
fn play_state_grouping() {
    assert_eq!(play_state_group(4), Some(3));
    assert_eq!(play_state_group(0x12), Some(3));
    assert_eq!(play_state_group(0x0E), Some(0x0A));
    assert_eq!(play_state_group(5), Some(5));
    assert_eq!(play_state_group(0), Some(0));
    assert_eq!(play_state_group(2), None, "not passed on");
}

#[test]
fn device_table_rules() {
    let mut t = DeviceTable::new(Some(0x11));
    assert_eq!(t.apply(&keep_alive(2, 1, [10, 0, 0, 2])), vec![TableChange::Added(2)]);
    assert_eq!(t.apply(&keep_alive(2, 1, [10, 0, 0, 2])), vec![TableChange::Refreshed(2)]);
    assert_eq!(t.apply(&keep_alive(2, 1, [10, 0, 0, 3])), vec![TableChange::Overwritten(2)]);
    // two-deck unit on number 9 also occupies 10
    assert_eq!(t.apply(&keep_alive(9, 3, [10, 0, 0, 9])), vec![TableChange::Added(9), TableChange::Added(10)]);
    assert!(t.get(10).unwrap().second_deck);
    assert_eq!(t.apply(&keep_alive(0x11, 1, [10, 0, 0, 4])), vec![TableChange::Conflict(0x11)]);
    assert_eq!(t.apply(&keep_alive(0, 1, [10, 0, 0, 4])), vec![TableChange::Invalid], "number 0");
    assert_eq!(t.apply(&keep_alive(3, 10, [10, 0, 0, 4])), vec![TableChange::Invalid], "kind 10");
    assert_eq!(t.devices().count(), 3);
    t.remove(9);
    assert!(t.get(9).is_none() && t.get(10).is_none());
}

#[test]
fn decode_dispatch() {
    let b = Announce::Disconnect { device: 3, ip: Ipv4Addr::new(1, 2, 3, 4) }.to_bytes("CDJ");
    assert!(matches!(decode(PORT_ANNOUNCE, &b).unwrap(), Packet::Announce(_, Announce::Disconnect { device: 3, .. })));
    let b = long_packet(0x56, 0, 3, 0x30);
    assert!(matches!(decode(PORT_STATUS, &b).unwrap(), Packet::Other(LongHeader { kind: 0x56, .. })));
    assert_eq!(packet_type(&b), Some(0x56));
    assert_eq!(packet_type(b"nope"), None);
}
