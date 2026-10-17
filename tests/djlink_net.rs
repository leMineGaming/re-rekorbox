//! Two DJ Link participants talking over the loopback interface on private ports (no traffic on the
//! real network): number claim, mutual discovery, beat packets, disconnect. Plus the beat clock.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use rekordbox_formats::djlink::Beat;
use rekordbox_formats::djlink_net::{BeatClock, Link, LinkConfig};
use rekordbox_formats::djlink_participant::Config;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

fn config(name: &str, last: u8, base: u16, peer: u16) -> LinkConfig {
    let mut c = LinkConfig::new(Config::new(name, [2, 0, 0, 0, 0, last], Ipv4Addr::LOCALHOST), Ipv4Addr::LOCALHOST);
    c.bind = Ipv4Addr::LOCALHOST;
    c.announce_port = base;
    c.beat_port = base + 1;
    c.status_port = base + 2;
    c.targets = Some((vec![addr(peer)], vec![addr(peer + 1)]));
    c
}

fn wait_for(mut f: impl FnMut() -> bool, secs: u64) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn two_links_on_loopback() {
    let base = 52000 + (std::process::id() % 500) as u16 * 20;
    let (pa, pb) = (base, base + 10);
    let mut a = Link::start(config("rbx-a", 1, pa, pb)).expect("bind a");
    std::thread::sleep(Duration::from_millis(500));
    let b = Link::start(config("rbx-b", 2, pb, pa)).expect("bind b");
    assert!(wait_for(|| a.number().is_some() && b.number().is_some(), 15), "both claim a number");
    assert_ne!(a.number(), b.number());
    assert!(wait_for(|| b.state().devices.iter().any(|d| d.name == "rbx-a"), 5), "b sees a");
    assert!(wait_for(|| a.state().devices.iter().any(|d| d.name == "rbx-b"), 5), "a sees b");
    // beat from a arrives at b, tagged with a's number
    let beat = Beat::for_tempo("rbx-a", 0, 124.0, 2);
    assert!(a.send_beat(&beat));
    let na = a.number().unwrap();
    assert!(wait_for(|| b.state().beats.get(&na).is_some_and(|x| x.bpm100 == 12400 && x.beat_in_bar == 2), 5));
    // a leaves: b drops it from its table
    a.stop();
    assert!(wait_for(|| b.state().devices.iter().all(|d| d.name != "rbx-a"), 5), "disconnect seen");
}

#[test]
fn beat_clock_emits_once_per_beat() {
    let mut c = BeatClock::default();
    let first = c.update("rbx", Some(4.25), 120.0).expect("new beat 4");
    assert_eq!(first.beat_in_bar, 1, "beat 4 (0-based) is the downbeat of bar 2");
    assert_eq!(first.next_beat_ms, 375, "a quarter of the beat has passed");
    assert!(c.update("rbx", Some(4.9), 120.0).is_none());
    let next = c.update("rbx", Some(5.01), 120.0).unwrap();
    assert_eq!((next.beat_in_bar, next.next_bar_ms), (2, 1496), "1500 ms minus the truncated 4 ms already passed");
    assert!(c.update("rbx", None, 120.0).is_none(), "stopped deck");
    assert!(c.update("rbx", Some(5.5), 120.0).is_some(), "restarting emits again");
}
