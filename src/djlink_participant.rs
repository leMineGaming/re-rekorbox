//! Taking part in a Pro DJ Link network (SPEC §5i "Ablauf der Nummernvergabe"): discovery, device
//! number claim, keep-alive, conflict handling and device aging.
//!
//! [`Participant`] is a pure state machine: feed it received port-50000 datagrams with
//! [`Participant::receive`] and call [`Participant::tick`] regularly (every ≈ 20–50 ms); both return
//! datagrams to send ([`Send`]). Sockets live elsewhere (`djlink_net`).

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use crate::djlink::{Announce, DeviceTable, KeepAlive, TableChange, KIND_REKORDBOX};

/// Device numbers rekordbox cycles through while claiming (bit order of the "taken" mask).
pub const CANDIDATES: [u8; 6] = [0x11, 0x12, 0x29, 0x2A, 0x2B, 0x2C];

/// Timings in seconds (SPEC §5i timers).
pub const DISCOVERY_INTERVAL: f64 = 0.1;
pub const IDUSE_INTERVAL: f64 = 0.1;
pub const KEEPALIVE_INTERVAL: f64 = 2.0;
pub const RETRY_AFTER_FAIL: f64 = 30.0;
pub const RETRY_AFTER_CONFLICT: f64 = 0.3;
pub const AGING: f64 = 10.0;
/// Discovery requests before claiming.
pub const DISCOVERY_COUNT: u8 = 3;
/// Full rounds through [`CANDIDATES`] before choosing (the original stops after more than 5 counts).
pub const IDUSE_ROUNDS: u32 = 6;

/// A datagram for port 50000.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Send {
    Broadcast(Vec<u8>),
    Unicast(Ipv4Addr, Vec<u8>),
}

impl Send {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Send::Broadcast(b) | Send::Unicast(_, b) => b,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Idle,
    Discovery { sent: u8 },
    /// Cycling through the candidates; `sent` counts IdUse requests, `taken` is the mask.
    IdUse { sent: u32, taken: u8 },
    Running,
    /// Waiting before a new attempt (after a failure or a conflict).
    Waiting { until: f64 },
}

/// Configuration of the local device.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub name: String,
    pub mac: [u8; 6],
    pub ip: Ipv4Addr,
    pub kind: u8,
    /// Numbers to choose from, in order of preference (default: rekordbox's 0x11, 0x12, 0x29…0x2C).
    pub candidates: Vec<u8>,
}

impl Config {
    pub fn new(name: &str, mac: [u8; 6], ip: Ipv4Addr) -> Config {
        Config { name: name.to_string(), mac, ip, kind: KIND_REKORDBOX, candidates: CANDIDATES.to_vec() }
    }
}

/// What happened, for logs and UIs.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Claimed(u8),
    NoFreeNumber,
    Conflict(u8),
    Device(TableChange),
    Aged(u8),
    Disconnected(u8),
}

#[derive(Debug, Clone)]
pub struct Participant {
    pub config: Config,
    state: State,
    number: u8,
    next_due: f64,
    table: DeviceTable,
    last_seen: BTreeMap<u8, f64>,
    counter: u8,
    events: Vec<Event>,
}

fn bit_of(number: u8) -> Option<u8> {
    CANDIDATES.iter().position(|&c| c == number).map(|i| 1 << i)
}

impl Participant {
    pub fn new(config: Config) -> Participant {
        let first = config.candidates.first().copied().unwrap_or(0x11);
        Participant {
            config,
            state: State::Idle,
            number: first,
            next_due: 0.0,
            table: DeviceTable::new(None),
            last_seen: BTreeMap::new(),
            counter: 0,
            events: Vec::new(),
        }
    }

    pub fn state(&self) -> State {
        self.state
    }

    /// Our device number once claimed.
    pub fn number(&self) -> Option<u8> {
        (self.state == State::Running).then_some(self.number)
    }

    pub fn devices(&self) -> &DeviceTable {
        &self.table
    }

    /// Events since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn start(&mut self, now: f64) {
        self.state = State::Discovery { sent: 0 };
        self.next_due = now;
    }

    fn keep_alive(&self) -> KeepAlive {
        KeepAlive {
            name: self.config.name.clone(),
            device: self.number,
            b25: 1,
            mac: self.config.mac,
            ip: self.config.ip,
            peers: (self.table.devices().count() + 1).min(255) as u8,
            kind: self.config.kind,
            b35: 8,
        }
    }

    fn id_use(&mut self, number: u8) -> Send {
        self.counter = self.counter.wrapping_add(1);
        Send::Broadcast(
            Announce::IdRequest {
                set: false,
                ip: self.config.ip,
                mac: self.config.mac,
                device: number,
                counter: self.counter,
                kind: self.config.kind,
            }
            .to_bytes(&self.config.name),
        )
    }

    /// Advance timers; returns datagrams to send.
    pub fn tick(&mut self, now: f64) -> Vec<Send> {
        let mut out = Vec::new();
        // aging of other devices
        let aged: Vec<u8> = self.last_seen.iter().filter(|(_, &t)| now - t > AGING).map(|(&n, _)| n).collect();
        for n in aged {
            self.last_seen.remove(&n);
            self.table.remove(n);
            self.events.push(Event::Aged(n));
        }
        if now < self.next_due {
            return out;
        }
        match self.state {
            State::Idle => {}
            State::Discovery { sent } => {
                if sent >= DISCOVERY_COUNT {
                    self.state = State::IdUse { sent: 0, taken: self.taken_by_table() };
                    self.next_due = now;
                    return self.tick(now);
                }
                out.push(Send::Broadcast(
                    Announce::Discovery { counter: sent + 1, kind: self.config.kind, mac: self.config.mac }
                        .to_bytes(&self.config.name),
                ));
                self.state = State::Discovery { sent: sent + 1 };
                self.next_due = now + DISCOVERY_INTERVAL;
            }
            State::IdUse { sent, taken } => {
                let n_cand = CANDIDATES.len() as u32;
                if sent >= IDUSE_ROUNDS * n_cand {
                    let taken = taken | self.taken_by_table();
                    match self.config.candidates.iter().copied().find(|&c| bit_of(c).map_or(true, |b| taken & b == 0) && self.table.get(c).is_none()) {
                        Some(n) => {
                            self.number = n;
                            self.state = State::Running;
                            self.events.push(Event::Claimed(n));
                            out.push(Send::Broadcast(self.keep_alive().to_bytes()));
                            self.next_due = now + KEEPALIVE_INTERVAL;
                        }
                        None => {
                            self.events.push(Event::NoFreeNumber);
                            self.state = State::Waiting { until: now + RETRY_AFTER_FAIL };
                            self.next_due = now + RETRY_AFTER_FAIL;
                        }
                    }
                    return out;
                }
                let cand = CANDIDATES[(sent % n_cand) as usize];
                out.push(self.id_use(cand));
                self.state = State::IdUse { sent: sent + 1, taken };
                self.next_due = now + IDUSE_INTERVAL;
            }
            State::Running => {
                out.push(Send::Broadcast(self.keep_alive().to_bytes()));
                self.next_due = now + KEEPALIVE_INTERVAL;
            }
            State::Waiting { until } => {
                if now >= until {
                    self.start(now);
                    return self.tick(now);
                }
            }
        }
        out
    }

    fn taken_by_table(&self) -> u8 {
        self.table.devices().filter_map(|d| bit_of(d.number)).fold(0, |m, b| m | b)
    }

    /// Handle a datagram received on port 50000 from `from`.
    pub fn receive(&mut self, now: f64, from: Ipv4Addr, bytes: &[u8]) -> Vec<Send> {
        let mut out = Vec::new();
        let Ok((_, packet)) = Announce::parse(bytes) else { return out };
        match packet {
            Announce::KeepAlive(k) => {
                // our own broadcast comes back on some systems
                if k.mac == self.config.mac && k.ip == self.config.ip {
                    return out;
                }
                if self.state == State::Running && k.device == self.number {
                    self.events.push(Event::Conflict(self.number));
                    out.push(Send::Broadcast(
                        Announce::Disconnect { device: self.number, ip: self.config.ip }.to_bytes(&self.config.name),
                    ));
                    self.state = State::Waiting { until: now + RETRY_AFTER_CONFLICT };
                    self.next_due = now + RETRY_AFTER_CONFLICT;
                }
                if let State::IdUse { sent, taken } = self.state {
                    if let Some(b) = bit_of(k.device) {
                        self.state = State::IdUse { sent, taken: taken | b };
                    }
                }
                for change in self.table.apply(&k) {
                    if let TableChange::Added(n) | TableChange::Refreshed(n) | TableChange::Overwritten(n) = change {
                        self.last_seen.insert(n, now);
                    }
                    if !matches!(change, TableChange::Refreshed(_)) {
                        self.events.push(Event::Device(change));
                    }
                }
            }
            Announce::IdResponse { set: false, device, status: 1, .. } => {
                if let State::IdUse { sent, taken } = self.state {
                    if let Some(b) = bit_of(device) {
                        self.state = State::IdUse { sent, taken: taken | b };
                    }
                }
            }
            Announce::IdRequest { set: false, device, counter, .. } => {
                if self.state == State::Running && device == self.number {
                    out.push(Send::Unicast(
                        from,
                        Announce::IdResponse { set: false, device, counter, status: 1 }.to_bytes(&self.config.name),
                    ));
                }
            }
            Announce::Disconnect { device, .. } => {
                if self.table.get(device).is_some() {
                    self.table.remove(device);
                    self.last_seen.remove(&device);
                    self.events.push(Event::Disconnected(device));
                }
            }
            _ => {}
        }
        out
    }

    /// Leave the network: a Disconnect if a number was claimed.
    pub fn stop(&mut self) -> Vec<Send> {
        let was = self.state;
        self.state = State::Idle;
        if was == State::Running {
            vec![Send::Broadcast(Announce::Disconnect { device: self.number, ip: self.config.ip }.to_bytes(&self.config.name))]
        } else {
            vec![]
        }
    }
}
