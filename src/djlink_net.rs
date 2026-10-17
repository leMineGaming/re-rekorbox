//! Running a [`Participant`] on real UDP sockets, receiving beat and status packets of other devices
//! and sending our own beat packets.
//!
//! [`Link::start`] binds the three DJ Link ports, runs a background thread and exposes a
//! [`LinkState`] snapshot. Ports and targets are configurable so the whole thing can be exercised on
//! the loopback interface without any traffic on the real network.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::djlink::{decode, Beat, CdjStatus, Device, MixerStatus, Packet, PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS};
use crate::djlink_participant::{Config, Event, Participant, Send, State};

/// Where to bind and where to send.
#[derive(Debug, Clone)]
pub struct LinkConfig {
    pub participant: Config,
    /// Local address to bind (0.0.0.0 = all interfaces).
    pub bind: Ipv4Addr,
    /// Broadcast address of the chosen interface.
    pub broadcast: Ipv4Addr,
    pub announce_port: u16,
    pub beat_port: u16,
    pub status_port: u16,
    /// Replace the broadcast targets (announce, beat) – used for loopback tests.
    pub targets: Option<(Vec<SocketAddr>, Vec<SocketAddr>)>,
}

impl LinkConfig {
    /// Standard ports, binding all interfaces, broadcasting on `broadcast`.
    pub fn new(participant: Config, broadcast: Ipv4Addr) -> LinkConfig {
        LinkConfig {
            participant,
            bind: Ipv4Addr::UNSPECIFIED,
            broadcast,
            announce_port: PORT_ANNOUNCE,
            beat_port: PORT_BEAT,
            status_port: PORT_STATUS,
            targets: None,
        }
    }

    fn announce_targets(&self) -> Vec<SocketAddr> {
        match &self.targets {
            Some((a, _)) => a.clone(),
            None => vec![SocketAddr::V4(SocketAddrV4::new(self.broadcast, self.announce_port))],
        }
    }

    fn beat_targets(&self) -> Vec<SocketAddr> {
        match &self.targets {
            Some((_, b)) => b.clone(),
            None => vec![SocketAddr::V4(SocketAddrV4::new(self.broadcast, self.beat_port))],
        }
    }
}

/// Snapshot of the link for display.
#[derive(Debug, Clone, Default)]
pub struct LinkState {
    pub number: Option<u8>,
    pub phase: Option<State>,
    pub devices: Vec<Device>,
    /// Last beat packet per device number.
    pub beats: BTreeMap<u8, Beat>,
    pub status: BTreeMap<u8, CdjStatus>,
    pub mixers: BTreeMap<u8, MixerStatus>,
    /// Recent events (oldest first, at most 50).
    pub events: Vec<Event>,
    /// Datagrams that could not be decoded.
    pub errors: u64,
}

pub struct Link {
    state: Arc<Mutex<LinkState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    beat_socket: UdpSocket,
    beat_targets: Vec<SocketAddr>,
}

fn bind(ip: Ipv4Addr, port: u16) -> io::Result<UdpSocket> {
    let s = UdpSocket::bind((ip, port)).map_err(|e| io::Error::new(e.kind(), format!("UDP port {port}: {e}")))?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    Ok(s)
}

impl Link {
    pub fn start(cfg: LinkConfig) -> io::Result<Link> {
        let announce = bind(cfg.bind, cfg.announce_port)?;
        let beat = bind(cfg.bind, cfg.beat_port)?;
        let status = bind(cfg.bind, cfg.status_port)?;
        let beat_socket = beat.try_clone()?;
        let state = Arc::new(Mutex::new(LinkState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let announce_targets = cfg.announce_targets();
        let announce_port = cfg.announce_port;
        let mut participant = Participant::new(cfg.participant.clone());
        let thread = std::thread::spawn(move || {
            let t0 = Instant::now();
            let now = || t0.elapsed().as_secs_f64();
            participant.start(now());
            let mut ports: HashMap<Ipv4Addr, u16> = HashMap::new();
            let mut buf = [0u8; 0x5C0];
            let send = |out: Vec<Send>, ports: &HashMap<Ipv4Addr, u16>| {
                for s in out {
                    match s {
                        Send::Broadcast(b) => {
                            for t in &announce_targets {
                                let _ = announce.send_to(&b, t);
                            }
                        }
                        Send::Unicast(ip, b) => {
                            let port = ports.get(&ip).copied().unwrap_or(announce_port);
                            let _ = announce.send_to(&b, (ip, port));
                        }
                    }
                }
            };
            while !sp.load(Ordering::Relaxed) {
                let mut busy = false;
                while let Ok((n, from)) = announce.recv_from(&mut buf) {
                    busy = true;
                    if let SocketAddr::V4(v4) = from {
                        ports.insert(*v4.ip(), v4.port());
                        let out = participant.receive(now(), *v4.ip(), &buf[..n]);
                        send(out, &ports);
                    }
                }
                for (sock, port) in [(&beat, PORT_BEAT), (&status, PORT_STATUS)] {
                    while let Ok((n, _)) = sock.recv_from(&mut buf) {
                        busy = true;
                        let mut s = st.lock().unwrap();
                        match decode(port, &buf[..n]) {
                            Ok(Packet::Beat(b)) => {
                                s.beats.insert(b.device, b);
                            }
                            Ok(Packet::CdjStatus(c)) => {
                                s.status.insert(c.device, c);
                            }
                            Ok(Packet::MixerStatus(m)) => {
                                s.mixers.insert(m.device, m);
                            }
                            Ok(_) => {}
                            Err(_) => s.errors += 1,
                        }
                    }
                }
                let out = participant.tick(now());
                send(out, &ports);
                {
                    let mut s = st.lock().unwrap();
                    s.number = participant.number();
                    s.phase = Some(participant.state());
                    s.devices = participant.devices().devices().cloned().collect();
                    let own = participant.number();
                    if let Some(n) = own {
                        s.beats.remove(&n);
                    }
                    s.events.extend(participant.take_events());
                    let excess = s.events.len().saturating_sub(50);
                    s.events.drain(..excess);
                }
                if !busy {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            send(participant.stop(), &ports);
        });
        Ok(Link { state, stop, thread: Some(thread), beat_socket, beat_targets: cfg.beat_targets() })
    }

    pub fn state(&self) -> LinkState {
        self.state.lock().unwrap().clone()
    }

    /// Our device number once claimed.
    pub fn number(&self) -> Option<u8> {
        self.state.lock().unwrap().number
    }

    /// Send a beat packet with our device number (nothing before a number is claimed).
    pub fn send_beat(&self, beat: &Beat) -> bool {
        let Some(n) = self.number() else { return false };
        let mut b = beat.clone();
        b.device = n;
        let bytes = b.to_bytes();
        for t in &self.beat_targets {
            let _ = self.beat_socket.send_to(&bytes, t);
        }
        true
    }

    /// Leave the network (sends a disconnect) and stop the thread.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Tracks the beat position of a playing deck and produces a beat packet on every new beat.
#[derive(Debug, Clone, Default)]
pub struct BeatClock {
    last_beat: Option<i64>,
}

impl BeatClock {
    /// `beat` = fractional beat number since the grid's first beat (beat 0 = downbeat), `bpm` =
    /// effective tempo. Returns a packet when a new beat started (also when jumping).
    pub fn update(&mut self, name: &str, beat: Option<f64>, bpm: f64) -> Option<Beat> {
        let Some(b) = beat else {
            self.last_beat = None;
            return None;
        };
        let whole = b.floor() as i64;
        if self.last_beat == Some(whole) {
            return None;
        }
        self.last_beat = Some(whole);
        let in_bar = (whole.rem_euclid(4) + 1) as u8;
        let mut packet = Beat::for_tempo(name, 0, bpm, in_bar);
        // the times count from now: shorten them by the part of the beat that already passed
        let passed_ms = ((b - b.floor()) * 60000.0 / bpm) as u32;
        for t in [
            &mut packet.next_beat_ms,
            &mut packet.second_beat_ms,
            &mut packet.next_bar_ms,
            &mut packet.fourth_beat_ms,
            &mut packet.second_bar_ms,
            &mut packet.eighth_beat_ms,
        ] {
            *t = t.saturating_sub(passed_ms);
        }
        Some(packet)
    }
}
