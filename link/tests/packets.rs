//! Packet codecs against the bytes of capture 1 (rekordbox 7.2.19 ↔ XDJ-RX3).

mod fixtures;

use std::net::Ipv4Addr;

use fixtures::*;
use rbx_link::packets::{self, LibraryInfo, Me, Request};
use rekordbox_formats::djlink::Announce;

const MAC: [u8; 6] = [0xC8, 0x3D, 0xFC, 0x11, 0x43, 0x61];
const IP: Ipv4Addr = Ipv4Addr::new(169, 254, 226, 215);

fn me() -> Me {
    Me {
        number: 0x11,
        mac: MAC,
        ip: IP,
    }
}

#[test]
fn announce_packets_are_rekordbox_s() {
    assert_eq!(packets::discovery(1, MAC), bytes(REKORDBOX_50000_00));
    assert_eq!(packets::claim(&me(), 1), bytes(REKORDBOX_50000_02));
    assert_eq!(packets::keep_alive(&me(), 1), bytes(REKORDBOX_50000_06));
}

#[test]
fn answers_on_port_50002_are_rekordbox_s() {
    assert_eq!(
        packets::device_info(0x11, "TEST-PC"),
        bytes(REKORDBOX_50002_11)
    );
    assert_eq!(packets::reply_31(0x11), bytes(REKORDBOX_50002_31));
    assert_eq!(packets::reply_47(0x11), bytes(REKORDBOX_50002_47));
    assert_eq!(packets::presence(0x11), bytes(REKORDBOX_50002_16));
    assert_eq!(packets::status_29(0x11, None), bytes(REKORDBOX_50002_29));
    let lib = LibraryInfo {
        tracks: 0x071F,
        playlists: 0x14,
    };
    assert_eq!(
        packets::media_response(0x11, 4, "rekordbox", lib),
        bytes(REKORDBOX_50002_06)
    );
}

#[test]
fn master_tempo_in_status_29() {
    let b = packets::status_29(0x11, Some(135.0));
    assert_eq!(
        &b[0x2C..0x30],
        &[0x80, 0x00, 0x34, 0xBC],
        "as rekordbox sends 135.00 BPM"
    );
    assert_eq!(b.len(), 56);
}

#[test]
fn requests_of_the_xdj_rx3() {
    assert_eq!(
        packets::parse_status_port(&bytes(XDJ_50002_10)).unwrap(),
        Request::DeviceInfo { from: 11 }
    );
    assert_eq!(
        packets::parse_status_port(&bytes(XDJ_50002_30)).unwrap(),
        Request::Query30 { from: 11 }
    );
    assert!(matches!(
        packets::parse_status_port(&bytes(XDJ_50002_46)).unwrap(),
        Request::Query46 { from: 11, .. }
    ));
    assert_eq!(
        packets::parse_status_port(&bytes(XDJ_50002_05)).unwrap(),
        Request::MediaQuery {
            from: 11,
            reply_to: Ipv4Addr::new(169, 254, 67, 96),
            target: 17,
            slot: 4
        }
    );
    assert!(matches!(
        packets::parse_status_port(&bytes(XDJ_50002_1A)).unwrap(),
        Request::Ack1A { from: 11, .. }
    ));
    match packets::parse_status_port(&bytes(XDJ_50002_0A)).unwrap() {
        Request::Status(s) => assert_eq!(s.device, 0x0C),
        other => panic!("{other:?}"),
    }
    let (_, a) = Announce::parse(&bytes(XDJ_50000_06)).unwrap();
    match a {
        Announce::KeepAlive(k) => {
            assert_eq!((k.device, k.kind, k.name.as_str()), (11, 7, "XDJ-RX3"))
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    assert!(packets::parse_status_port(&[]).is_err());
    assert!(packets::parse_status_port(b"Qspt1WmJOL").is_err());
    let mut q = bytes(XDJ_50002_05);
    q.truncate(0x2A);
    assert!(packets::parse_status_port(&q).is_err());
    assert!(packets::describe(50002, &[1, 2, 3]).starts_with('?'));
}
