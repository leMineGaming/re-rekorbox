//! The presence state machine: timing like rekordbox, answers to the XDJ-RX3, device table, conflicts.

mod fixtures;

use std::net::Ipv4Addr;

use fixtures::*;
use rbx_link::devices::{Config, Event, Out, Phase, Presence, DEVICE_TIMEOUT};
use rbx_link::packets::{LibraryInfo, PORT_ANNOUNCE, PORT_STATUS};

const XDJ: Ipv4Addr = Ipv4Addr::new(169, 254, 67, 96);

fn presence() -> Presence {
    Presence::new(Config {
        mac: [0xC8, 0x3D, 0xFC, 0x11, 0x43, 0x61],
        ip: Ipv4Addr::new(169, 254, 226, 215),
        hostname: "TEST-PC".into(),
        library: LibraryInfo {
            tracks: 0x071F,
            playlists: 0x14,
        },
    })
}

fn kind(o: &Out) -> (u16, u8) {
    match o {
        Out::Broadcast { port, bytes } | Out::Unicast { port, bytes, .. } => (*port, bytes[0x0A]),
    }
}

/// Run from `t0` to `t1` in 1 ms steps, collecting (time, port, bytes).
fn run_full(p: &mut Presence, t0: f64, t1: f64) -> Vec<(f64, u16, Vec<u8>)> {
    let mut v = Vec::new();
    let mut t = t0;
    while t < t1 {
        for o in p.tick(t) {
            let (Out::Broadcast { port, bytes } | Out::Unicast { port, bytes, .. }) = o;
            v.push((t, port, bytes));
        }
        t += 0.001;
    }
    v
}

fn run(p: &mut Presence, t0: f64, t1: f64) {
    run_full(p, t0, t1);
}

/// Time the claim is done.
const CLAIMED: f64 = 4.5;

#[test]
fn announces_with_rekordbox_timing() {
    let mut p = presence();
    p.start(0.0);
    let sent = run_full(&mut p, 0.0, 15.0);
    let at = |port: u16, k: u8| {
        sent.iter()
            .filter(|s| s.1 == port && s.2[0x0A] == k)
            .map(|s| s.0)
            .collect::<Vec<_>>()
    };
    let close = |a: f64, b: f64| (a - b).abs() < 0.002;
    let disc = at(PORT_ANNOUNCE, 0x00);
    assert_eq!(disc.len(), 3);
    assert!(close(disc[1] - disc[0], 0.22) && close(disc[2] - disc[1], 0.22));
    // six rounds over 17, 18, 41, 42, 43, 44 with counters 1..6, 0.1 s apart
    let claims: Vec<_> = sent
        .iter()
        .filter(|s| s.1 == PORT_ANNOUNCE && s.2[0x0A] == 0x02)
        .collect();
    assert_eq!(claims.len(), 36);
    assert!(close(claims[1].0 - claims[0].0, 0.1));
    let order: Vec<(u8, u8)> = claims.iter().map(|c| (c.2[0x2E], c.2[0x2F])).collect();
    assert_eq!(
        &order[..7],
        &[
            (17, 1),
            (18, 1),
            (41, 1),
            (42, 1),
            (43, 1),
            (44, 1),
            (17, 2)
        ]
    );
    assert_eq!(order[35], (44, 6));
    let ka = at(PORT_ANNOUNCE, 0x06);
    assert!(ka.len() >= 5, "keep-alive every 2 s: {}", ka.len());
    assert!(close(ka[1] - ka[0], 2.0));
    assert!(close(ka[0] - claims[35].0, 0.1));
    let status = at(PORT_STATUS, 0x29);
    assert!(close(status[1] - status[0], 0.1));
    assert!(close(status[0] - ka[0], 0.1));
    assert_eq!(p.phase(), Phase::Running);
    assert_eq!(p.number(), Some(17));
    assert!(p.take_events().contains(&Event::Claimed(17)));
}

fn running() -> Presence {
    let mut p = presence();
    p.start(0.0);
    run(&mut p, 0.0, CLAIMED);
    assert_eq!(p.number(), Some(17));
    p.take_events();
    p
}

#[test]
fn answers_the_xdj_rx3_like_rekordbox() {
    let mut p = running();
    // the unit appears: table gets decks 11 and 12, and it gets a 0x16
    assert!(p
        .receive(CLAIMED, XDJ, PORT_ANNOUNCE, &bytes(XDJ_50000_06))
        .is_empty());
    let numbers: Vec<u8> = p.devices().map(|d| d.number).collect();
    assert_eq!(numbers, [11, 12]);
    let out = p.tick(CLAIMED + 0.001);
    assert!(out.iter().any(|o| matches!(o, Out::Unicast { ip, bytes, .. } if *ip == XDJ && *bytes == fixtures::bytes(REKORDBOX_50002_16))));
    let ka = out
        .iter()
        .chain(p.tick(CLAIMED + 2.1).iter())
        .find(|o| kind(o) == (PORT_ANNOUNCE, 0x06))
        .cloned();
    if let Some(Out::Broadcast { bytes, .. }) = ka {
        assert_eq!(bytes[0x30], 3, "rekordbox counts itself and both decks");
    }
    let answer =
        |p: &mut Presence, req: &str| p.receive(CLAIMED + 2.2, XDJ, PORT_STATUS, &bytes(req));
    let one = |v: Vec<Out>| match v.as_slice() {
        [Out::Unicast { ip, port, bytes }] => {
            assert_eq!((*ip, *port), (XDJ, PORT_STATUS));
            bytes.clone()
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(one(answer(&mut p, XDJ_50002_10)), bytes(REKORDBOX_50002_11));
    assert_eq!(one(answer(&mut p, XDJ_50002_30)), bytes(REKORDBOX_50002_31));
    assert_eq!(one(answer(&mut p, XDJ_50002_46)), bytes(REKORDBOX_50002_47));
    assert_eq!(one(answer(&mut p, XDJ_50002_05)), bytes(REKORDBOX_50002_06));
    assert!(answer(&mut p, XDJ_50002_0A).is_empty());
    assert!(p
        .take_events()
        .iter()
        .any(|e| matches!(e, Event::Status(_))));
}

#[test]
fn media_query_for_another_device_is_ignored() {
    let mut p = running();
    let mut q = bytes(XDJ_50002_05);
    q[0x2B] = 0x12;
    assert!(p.receive(CLAIMED, XDJ, PORT_STATUS, &q).is_empty());
}

#[test]
fn nothing_is_answered_before_the_number_is_claimed() {
    let mut p = presence();
    p.start(0.0);
    run(&mut p, 0.0, 0.3);
    assert!(p
        .receive(0.3, XDJ, PORT_STATUS, &bytes(XDJ_50002_10))
        .is_empty());
}

#[test]
fn conflict_moves_to_the_next_number() {
    let mut p = presence();
    p.start(0.0);
    run(&mut p, 0.0, 0.7);
    // another device keeps alive with number 17
    let mut other = bytes(REKORDBOX_50000_06);
    other[0x2C..0x30].copy_from_slice(&[169, 254, 1, 2]);
    other[0x26] = 0x00;
    p.receive(0.7, Ipv4Addr::new(169, 254, 1, 2), PORT_ANNOUNCE, &other);
    assert!(p.take_events().contains(&Event::Conflict {
        number: 17,
        next: 18
    }));
    run(&mut p, 0.7, 0.7 + CLAIMED);
    assert_eq!(p.number(), Some(18));
}

#[test]
fn silent_devices_age_out_and_own_packets_are_ignored() {
    let mut p = running();
    p.receive(CLAIMED, XDJ, PORT_ANNOUNCE, &bytes(XDJ_50000_06));
    p.tick(CLAIMED + 0.1);
    assert_eq!(p.devices().count(), 2);
    p.tick(CLAIMED + DEVICE_TIMEOUT + 0.1);
    assert_eq!(p.devices().count(), 0);
    assert!(p
        .take_events()
        .iter()
        .any(|e| matches!(e, Event::DeviceRemoved(11))));
    // our own broadcast coming back
    assert!(p
        .receive(
            9.0,
            Ipv4Addr::new(169, 254, 226, 215),
            PORT_STATUS,
            &bytes(XDJ_50002_10)
        )
        .is_empty());
}

#[test]
fn leaving_sends_a_disconnect() {
    let mut p = running();
    let out = p.stop();
    assert_eq!(out.len(), 1);
    assert_eq!(kind(&out[0]), (PORT_ANNOUNCE, 0x08));
    assert_eq!(p.phase(), Phase::Idle);
}

#[test]
fn queries_before_the_player_is_known_wait_like_rekordbox() {
    let mut p = running();
    let ask = |p: &mut Presence, req: &str| p.receive(CLAIMED, XDJ, PORT_STATUS, &bytes(req));
    // the device info is answered at once, 0x30 and 0x46 only once the player kept alive
    assert_eq!(ask(&mut p, XDJ_50002_10).len(), 1);
    assert!(ask(&mut p, XDJ_50002_30).is_empty());
    assert!(ask(&mut p, XDJ_50002_46).is_empty());
    p.receive(CLAIMED, XDJ, PORT_ANNOUNCE, &bytes(XDJ_50000_06));
    assert_eq!(ask(&mut p, XDJ_50002_30).len(), 1);
    assert_eq!(ask(&mut p, XDJ_50002_46).len(), 1);
}

#[test]
fn presence_0x16_on_appearance_after_4_3_s_and_then_rarely() {
    let mut p = running();
    p.receive(CLAIMED, XDJ, PORT_ANNOUNCE, &bytes(XDJ_50000_06));
    let mut sent = Vec::new();
    let mut t = CLAIMED;
    while t < CLAIMED + 150.0 {
        // the unit keeps alive every 2 s
        if ((t - CLAIMED) / 2.0).fract() < 0.0005 {
            p.receive(t, XDJ, PORT_ANNOUNCE, &bytes(XDJ_50000_06));
        }
        for o in p.tick(t) {
            if let Out::Unicast { ip, bytes, .. } = o {
                if ip == XDJ && bytes[0x0A] == 0x16 {
                    sent.push(t - CLAIMED);
                }
            }
        }
        t += 0.001;
    }
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(sent[0] < 0.01);
    assert!((sent[1] - 4.3).abs() < 0.01);
    assert!((sent[2] - 141.3).abs() < 0.01);
}
