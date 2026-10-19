//! Presence on the network like rekordbox in LINK EXPORT mode: claiming a device number, keep-alives,
//! the device table with timeouts, and the answers players expect on port 50002. Pure and
//! time-driven (the caller passes `now` in seconds), so the timing is testable without sockets.
//!
//! Timing and content as in capture 1 (`docs/link/CAPTURE1.md`): three discoveries 0.22 s apart,
//! then six rounds of number claims over all candidates 0.1 s apart, then keep-alives every 2 s;
//! status 0x29 broadcast every 0.1 s; 0x16 to every device number of a player (both decks of an
//! XDJ-RX3) when it appears, 4.3 s later and then every 137 s – a player already known before our
//! number is claimed gets its first 0x16 on its next keep-alive, as rekordbox does; answers to 0x10 always, to 0x30 and 0x46 once the player is in the table (as
//! rekordbox: the player asks again), and to media queries.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use rekordbox_formats::djlink::{Announce, Device, DeviceTable, TableChange};

use crate::packets::{self, kind, LibraryInfo, Me, Request, PORT_ANNOUNCE, PORT_STATUS};

pub const DISCOVERY_COUNT: u8 = 3;
pub const DISCOVERY_INTERVAL: f64 = 0.22;
/// Six rounds over the six candidates.
pub const CLAIM_COUNT: u32 = 36;
pub const CLAIM_INTERVAL: f64 = 0.1;
pub const KEEPALIVE_INTERVAL: f64 = 2.0;
pub const STATUS_INTERVAL: f64 = 0.1;
/// The second 0x16 after a player appeared.
pub const PRESENCE_SECOND: f64 = 4.3;
pub const PRESENCE_INTERVAL: f64 = 137.0;
/// A device not heard from for this long is dropped (players keep alive every 2 s).
pub const DEVICE_TIMEOUT: f64 = 7.0;
/// Numbers rekordbox tries, in order (SPEC §5i).
pub const CANDIDATES: [u8; 6] = [0x11, 0x12, 0x29, 0x2A, 0x2B, 0x2C];

/// A datagram to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Broadcast {
        port: u16,
        bytes: Vec<u8>,
    },
    Unicast {
        ip: Ipv4Addr,
        port: u16,
        bytes: Vec<u8>,
    },
}

/// What happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Claimed(u8),
    /// Another device uses the number we wanted or had; we move on to `next`.
    Conflict {
        number: u8,
        next: u8,
    },
    DeviceAdded(Device),
    DeviceRemoved(u8),
    /// A request we answered (or not) on port 50002.
    Request {
        from: u8,
        kind: u8,
    },
    /// CDJ status of a player deck.
    Status(Box<rekordbox_formats::djlink::CdjStatus>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Discovery { sent: u8 },
    Claim { sent: u32 },
    Running,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    /// Computer name for the 0x11 answer.
    pub hostname: String,
    pub library: LibraryInfo,
}

pub struct Presence {
    cfg: Config,
    phase: Phase,
    candidate: usize,
    next_announce: f64,
    next_status: f64,
    /// Per player: when the next 0x16 is due and how many were sent.
    next_presence: BTreeMap<u8, (f64, u32)>,
    table: DeviceTable,
    seen: BTreeMap<u8, f64>,
    master_bpm: Option<f64>,
    events: Vec<Event>,
}

impl Presence {
    pub fn new(cfg: Config) -> Presence {
        Presence {
            cfg,
            phase: Phase::Idle,
            candidate: 0,
            next_announce: 0.0,
            next_status: 0.0,
            next_presence: BTreeMap::new(),
            table: DeviceTable::new(None),
            seen: BTreeMap::new(),
            master_bpm: None,
            events: Vec::new(),
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Our number once claimed.
    pub fn number(&self) -> Option<u8> {
        (self.phase == Phase::Running).then(|| CANDIDATES[self.candidate])
    }

    pub fn devices(&self) -> impl Iterator<Item = &Device> {
        self.table.devices()
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn set_library(&mut self, lib: LibraryInfo) {
        self.cfg.library = lib;
    }

    pub fn set_master_bpm(&mut self, bpm: Option<f64>) {
        self.master_bpm = bpm;
    }

    fn me(&self) -> Me {
        Me {
            number: CANDIDATES[self.candidate],
            mac: self.cfg.mac,
            ip: self.cfg.ip,
        }
    }

    pub fn start(&mut self, now: f64) {
        self.phase = Phase::Discovery { sent: 0 };
        self.next_announce = now;
    }

    /// Packets due at `now`.
    pub fn tick(&mut self, now: f64) -> Vec<Out> {
        let mut out = Vec::new();
        let b = |bytes: Vec<u8>, port: u16| Out::Broadcast { port, bytes };
        while now >= self.next_announce && self.phase != Phase::Idle {
            match self.phase {
                Phase::Idle => break,
                Phase::Discovery { sent } => {
                    out.push(b(packets::discovery(sent + 1, self.cfg.mac), PORT_ANNOUNCE));
                    self.next_announce += DISCOVERY_INTERVAL;
                    self.phase = if sent + 1 >= DISCOVERY_COUNT {
                        Phase::Claim { sent: 0 }
                    } else {
                        Phase::Discovery { sent: sent + 1 }
                    };
                }
                Phase::Claim { sent } => {
                    let me = Me {
                        number: CANDIDATES[sent as usize % CANDIDATES.len()],
                        ..self.me()
                    };
                    let round = (sent as usize / CANDIDATES.len() + 1) as u8;
                    out.push(b(packets::claim(&me, round), PORT_ANNOUNCE));
                    self.next_announce += CLAIM_INTERVAL;
                    if sent + 1 >= CLAIM_COUNT {
                        self.phase = Phase::Running;
                        self.table.own_number = Some(CANDIDATES[self.candidate]);
                        self.events.push(Event::Claimed(CANDIDATES[self.candidate]));
                        self.next_status = self.next_announce + STATUS_INTERVAL;
                    } else {
                        self.phase = Phase::Claim { sent: sent + 1 };
                    }
                }
                Phase::Running => {
                    let peers = 1 + self.table.devices().count() as u8;
                    out.push(b(packets::keep_alive(&self.me(), peers), PORT_ANNOUNCE));
                    self.next_announce += KEEPALIVE_INTERVAL;
                }
            }
        }
        if let Some(own) = self.number() {
            if now >= self.next_status {
                out.push(b(packets::status_29(own, self.master_bpm), PORT_STATUS));
                // catch up without bursts after a stall
                self.next_status = (self.next_status + STATUS_INTERVAL).max(now);
            }
            let due: Vec<(u8, u32)> = self
                .next_presence
                .iter()
                .filter(|(_, &(t, _))| now >= t)
                .map(|(&n, &(_, sent))| (n, sent))
                .collect();
            for (n, sent) in due {
                // rekordbox notifies every device number, also the second deck of a two-deck unit
                if let Some(d) = self.table.get(n) {
                    out.push(Out::Unicast {
                        ip: d.ip,
                        port: PORT_STATUS,
                        bytes: packets::presence(own),
                    });
                }
                let wait = if sent == 0 {
                    PRESENCE_SECOND
                } else {
                    PRESENCE_INTERVAL
                };
                self.next_presence.insert(n, (now + wait, sent + 1));
            }
        }
        // aging
        let gone: Vec<u8> = self
            .seen
            .iter()
            .filter(|(_, &t)| now - t > DEVICE_TIMEOUT)
            .map(|(&n, _)| n)
            .collect();
        for n in gone {
            self.seen.remove(&n);
            self.next_presence.remove(&n);
            self.table.remove(n);
            self.events.push(Event::DeviceRemoved(n));
        }
        out
    }

    fn conflict(&mut self, now: f64) {
        let number = CANDIDATES[self.candidate];
        self.candidate = (self.candidate + 1) % CANDIDATES.len();
        self.events.push(Event::Conflict {
            number,
            next: CANDIDATES[self.candidate],
        });
        self.table.own_number = None;
        self.phase = Phase::Claim { sent: 0 };
        self.next_announce = now;
    }

    /// A datagram from `from` on `port`; returns the answers.
    pub fn receive(&mut self, now: f64, from: Ipv4Addr, port: u16, b: &[u8]) -> Vec<Out> {
        if from == self.cfg.ip {
            return Vec::new();
        }
        if port == PORT_ANNOUNCE {
            let wanted = CANDIDATES[self.candidate];
            match Announce::parse(b) {
                Ok((_, Announce::KeepAlive(k))) => {
                    if k.device == wanted
                        && matches!(
                            self.phase,
                            Phase::Claim { .. } | Phase::Running | Phase::Discovery { .. }
                        )
                    {
                        self.conflict(now);
                        return Vec::new();
                    }
                    for c in self.table.apply(&k) {
                        match c {
                            TableChange::Added(n) => {
                                self.seen.insert(n, now);
                                if let Some(d) = self.table.get(n) {
                                    self.events.push(Event::DeviceAdded(d.clone()));
                                }
                                // before our number is claimed the player does not know us yet: the
                                // first 0x16 waits for its next keep-alive
                                let due = if self.phase == Phase::Running {
                                    now
                                } else {
                                    f64::INFINITY
                                };
                                self.next_presence.insert(n, (due, 0));
                            }
                            TableChange::Overwritten(n) | TableChange::Refreshed(n) => {
                                self.seen.insert(n, now);
                            }
                            TableChange::Conflict(_) | TableChange::Invalid => {}
                        }
                    }
                    if self.phase == Phase::Running {
                        let waiting: Vec<u8> = self
                            .next_presence
                            .iter()
                            .filter(|(n, (t, _))| {
                                t.is_infinite() && self.table.get(**n).is_some_and(|d| d.ip == from)
                            })
                            .map(|(n, _)| *n)
                            .collect();
                        for n in waiting {
                            self.next_presence.insert(n, (now, 0));
                        }
                    }
                }
                Ok((_, Announce::IdRequest { device, ip, .. }))
                    if device == wanted && ip != self.cfg.ip && self.phase != Phase::Running =>
                {
                    self.conflict(now);
                }
                Ok((_, Announce::Disconnect { device, .. })) => {
                    self.table.remove(device);
                    self.seen.remove(&device);
                    self.next_presence.remove(&device);
                    self.events.push(Event::DeviceRemoved(device));
                }
                _ => {}
            }
            return Vec::new();
        }
        if port != PORT_STATUS {
            return Vec::new();
        }
        let Ok(req) = packets::parse_status_port(b) else {
            return Vec::new();
        };
        let Some(own) = self.number() else {
            return Vec::new();
        };
        let to = |bytes: Vec<u8>| {
            vec![Out::Unicast {
                ip: from,
                port: PORT_STATUS,
                bytes,
            }]
        };
        match req {
            Request::Status(s) => {
                self.events.push(Event::Status(s));
                Vec::new()
            }
            Request::DeviceInfo { from: dev } => {
                self.events.push(Event::Request {
                    from: dev,
                    kind: kind::DEVICE_INFO_QUERY,
                });
                to(packets::device_info(own, &self.cfg.hostname))
            }
            Request::Query30 { from: dev } => {
                self.events.push(Event::Request {
                    from: dev,
                    kind: kind::QUERY_30,
                });
                match self.table.get(dev) {
                    Some(_) => to(packets::reply_31(own)),
                    None => Vec::new(),
                }
            }
            Request::Query46 { from: dev, .. } => {
                self.events.push(Event::Request {
                    from: dev,
                    kind: kind::QUERY_46,
                });
                match self.table.get(dev) {
                    Some(_) => to(packets::reply_47(own)),
                    None => Vec::new(),
                }
            }
            Request::MediaQuery {
                from: dev,
                reply_to,
                target,
                slot,
            } => {
                self.events.push(Event::Request {
                    from: dev,
                    kind: kind::MEDIA_QUERY,
                });
                if target != u32::from(own) {
                    return Vec::new();
                }
                vec![Out::Unicast {
                    ip: reply_to,
                    port: PORT_STATUS,
                    bytes: packets::media_response(own, slot, packets::REKORDBOX, self.cfg.library),
                }]
            }
            Request::Ack1A { from: dev, .. } | Request::Other { from: dev, kind: _ } => {
                self.events.push(Event::Request {
                    from: dev,
                    kind: b.get(0x0A).copied().unwrap_or(0),
                });
                Vec::new()
            }
        }
    }

    /// Packets for leaving the network.
    pub fn stop(&mut self) -> Vec<Out> {
        let out = match self.number() {
            Some(_) => vec![Out::Broadcast {
                port: PORT_ANNOUNCE,
                bytes: packets::disconnect(&self.me()),
            }],
            None => Vec::new(),
        };
        self.phase = Phase::Idle;
        out
    }
}
