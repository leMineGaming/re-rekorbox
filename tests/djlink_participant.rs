//! DJ Link participant state machine on simulated time and a simulated network (SPEC §5i).

use std::net::Ipv4Addr;

use rekordbox_formats::djlink::{packet_type, Announce, KeepAlive};
use rekordbox_formats::djlink_participant::*;

const MAC: [u8; 6] = [0x02, 0, 0, 0, 0, 0x01];
const IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);

fn participant() -> Participant {
    Participant::new(Config::new("rbx", MAC, IP))
}

/// Tick every 10 ms from `from` to `to`; returns all sent datagrams with their time.
fn run(p: &mut Participant, from: f64, to: f64) -> Vec<(f64, Send)> {
    let mut out = Vec::new();
    let mut t = from;
    while t < to {
        out.extend(p.tick(t).into_iter().map(|s| (t, s)));
        t += 0.01;
    }
    out
}

fn keep_alive_from(device: u8, last: u8) -> Vec<u8> {
    KeepAlive { name: "XDJ-TEST".into(), device, b25: 1, mac: [2, 0, 0, 0, 0, last], ip: Ipv4Addr::new(10, 0, 0, last), peers: 1, kind: 3, b35: 8 }
        .to_bytes()
}

fn kinds(sent: &[(f64, Send)]) -> Vec<u8> {
    sent.iter().map(|(_, s)| packet_type(s.bytes()).unwrap()).collect()
}

#[test]
fn claims_the_first_number_on_an_empty_network() {
    let mut p = participant();
    p.start(0.0);
    let sent = run(&mut p, 0.0, 7.0);
    let k = kinds(&sent);
    assert_eq!(&k[..3], &[0x00, 0x00, 0x00], "three discovery requests");
    let iduse = k.iter().filter(|&&t| t == 0x02).count();
    assert_eq!(iduse, 36, "six rounds over the six candidates");
    assert_eq!(p.number(), Some(0x11));
    assert_eq!(p.take_events(), [Event::Claimed(0x11)]);
    // keep-alives every 2 s after the claim
    let ka: Vec<f64> = sent.iter().filter(|(_, s)| packet_type(s.bytes()) == Some(0x06)).map(|(t, _)| *t).collect();
    assert!(ka.len() >= 2);
    assert!((ka[1] - ka[0] - 2.0).abs() < 0.02, "{ka:?}");
    // the requested numbers cycle 0x11, 0x12, 0x29 …
    let requested: Vec<u8> = sent
        .iter()
        .filter_map(|(_, s)| match Announce::parse(s.bytes()).unwrap().1 {
            Announce::IdRequest { device, .. } => Some(device),
            _ => None,
        })
        .take(7)
        .collect();
    assert_eq!(requested, [0x11, 0x12, 0x29, 0x2A, 0x2B, 0x2C, 0x11]);
    let (_, ka_packet) = Announce::parse(sent.iter().find(|(_, s)| packet_type(s.bytes()) == Some(6)).unwrap().1.bytes()).unwrap();
    match ka_packet {
        Announce::KeepAlive(k) => assert_eq!((k.device, k.kind, k.ip, k.mac), (0x11, 4, IP, MAC)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn skips_numbers_taken_by_keep_alives_and_responses() {
    let mut p = participant();
    p.start(0.0);
    run(&mut p, 0.0, 0.5);
    p.receive(0.5, Ipv4Addr::new(10, 0, 0, 9), &keep_alive_from(0x11, 9));
    let resp = Announce::IdResponse { set: false, device: 0x12, counter: 1, status: 1 }.to_bytes("rekordbox");
    p.receive(0.6, Ipv4Addr::new(10, 0, 0, 8), &resp);
    run(&mut p, 0.5, 5.0);
    assert_eq!(p.number(), Some(0x29));
    assert_eq!(p.devices().devices().count(), 1);
}

#[test]
fn no_free_number_waits_and_retries() {
    let mut p = Participant::new(Config { candidates: vec![0x11], ..Config::new("rbx", MAC, IP) });
    p.start(0.0);
    p.receive(0.0, Ipv4Addr::new(10, 0, 0, 9), &keep_alive_from(0x11, 9));
    run(&mut p, 0.0, 5.0);
    assert_eq!(p.number(), None);
    assert!(matches!(p.state(), State::Waiting { until } if (until - 4.2).abs() > 0.0));
    assert!(p.take_events().contains(&Event::NoFreeNumber));
    // the other device disappears (aging), the retry after 30 s succeeds
    let sent = run(&mut p, 5.0, 40.0);
    assert!(kinds(&sent).contains(&0x00), "discovery again");
    assert_eq!(p.number(), Some(0x11));
}

#[test]
fn answers_id_use_requests_for_its_number_and_ignores_own_echo() {
    let mut p = participant();
    p.start(0.0);
    run(&mut p, 0.0, 5.0);
    let req = Announce::IdRequest { set: false, ip: Ipv4Addr::new(10, 0, 0, 7), mac: [2; 6], device: 0x11, counter: 4, kind: 1 }
        .to_bytes("CDJ");
    let out = p.receive(5.0, Ipv4Addr::new(10, 0, 0, 7), &req);
    assert_eq!(out.len(), 1);
    match &out[0] {
        Send::Unicast(ip, b) => {
            assert_eq!(*ip, Ipv4Addr::new(10, 0, 0, 7));
            assert_eq!(Announce::parse(b).unwrap().1, Announce::IdResponse { set: false, device: 0x11, counter: 4, status: 1 });
        }
        other => panic!("{other:?}"),
    }
    // our own keep-alive echoed back is not a conflict
    let echo = sent_keep_alive(&mut p, 6.0);
    assert!(p.receive(6.0, IP, &echo).is_empty());
    assert_eq!(p.number(), Some(0x11));
}

fn sent_keep_alive(p: &mut Participant, t: f64) -> Vec<u8> {
    run(p, t, t + 2.1).into_iter().find(|(_, s)| packet_type(s.bytes()) == Some(6)).unwrap().1.bytes().to_vec()
}

#[test]
fn conflict_disconnects_and_reclaims() {
    let mut p = participant();
    p.start(0.0);
    run(&mut p, 0.0, 5.0);
    p.take_events();
    let out = p.receive(5.0, Ipv4Addr::new(10, 0, 0, 9), &keep_alive_from(0x11, 9));
    assert_eq!(kinds(&out.into_iter().map(|s| (0.0, s)).collect::<Vec<_>>()), [0x08], "disconnect");
    assert!(p.take_events().contains(&Event::Conflict(0x11)));
    run(&mut p, 5.0, 10.0);
    assert_eq!(p.number(), Some(0x12), "re-claimed a free number");
}

#[test]
fn aging_disconnect_and_stop() {
    let mut p = participant();
    p.start(0.0);
    p.receive(0.0, Ipv4Addr::new(10, 0, 0, 3), &keep_alive_from(3, 3));
    p.receive(0.0, Ipv4Addr::new(10, 0, 0, 4), &keep_alive_from(4, 4));
    run(&mut p, 0.0, 5.0);
    p.receive(5.0, Ipv4Addr::new(10, 0, 0, 3), &keep_alive_from(3, 3));
    let bye = Announce::Disconnect { device: 4, ip: Ipv4Addr::new(10, 0, 0, 4) }.to_bytes("CDJ");
    p.receive(5.5, Ipv4Addr::new(10, 0, 0, 4), &bye);
    assert!(p.devices().get(4).is_none());
    run(&mut p, 5.0, 14.0);
    assert!(p.devices().get(3).is_some(), "seen 9 s ago");
    run(&mut p, 14.0, 16.0);
    assert!(p.devices().get(3).is_none(), "aged out after 10 s");
    let ev = p.take_events();
    assert!(ev.contains(&Event::Disconnected(4)) && ev.contains(&Event::Aged(3)));
    let out = p.stop();
    assert_eq!(packet_type(out[0].bytes()), Some(0x08));
    assert!(p.stop().is_empty(), "nothing to disconnect twice");
}

#[test]
fn two_participants_on_one_network_get_different_numbers() {
    let mut a = participant();
    let mut b = Participant::new(Config::new("rbx-b", [2, 0, 0, 0, 0, 2], Ipv4Addr::new(10, 0, 0, 6)));
    a.start(0.0);
    b.start(1.0); // b joins later
    let mut t = 0.0;
    while t < 12.0 {
        let from_a = a.tick(t);
        let from_b = b.tick(t);
        for s in from_a {
            if let Send::Broadcast(bytes) = s {
                b.receive(t, IP, &bytes);
            }
        }
        for s in from_b {
            match s {
                Send::Broadcast(bytes) => {
                    a.receive(t, Ipv4Addr::new(10, 0, 0, 6), &bytes);
                }
                Send::Unicast(_, bytes) => {
                    a.receive(t, Ipv4Addr::new(10, 0, 0, 6), &bytes);
                }
            }
        }
        t += 0.01;
    }
    assert_eq!(a.number(), Some(0x11));
    assert_eq!(b.number(), Some(0x12));
    assert!(a.devices().get(0x12).is_some() && b.devices().get(0x11).is_some(), "they see each other");
}
