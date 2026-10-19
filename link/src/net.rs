//! The network side: adapter choice (the DJ unit's USB adapter first), the three DJ Link UDP sockets
//! plus portmapper and mountd ([`crate::rpc`]), a thread that drives [`Presence`], hot-plug and
//! adapter changes, and the database server ([`crate::server`]). Nothing here blocks the caller;
//! events come through a channel. RPC and TCP are answered only for devices in the link adapter's
//! subnet.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rekordbox_formats::netif::{self, Interface};
use tracing::{debug, info, trace, warn};

use crate::catalog::Catalog;
use crate::devices::{Config, Event, Out, Presence};
use crate::error::LinkError;
use crate::nfs::Fs;
use crate::packets::{LibraryInfo, PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS};
use crate::rpc::{self, Exports, PORT_MOUNTD, PORT_NFS, PORT_PORTMAP};
use crate::server::{DbServer, Shared};

/// How often the adapter list is checked for hot-plug.
pub const ADAPTER_CHECK: Duration = Duration::from_secs(2);

/// The adapter the presence runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub ip: Ipv4Addr,
    pub broadcast: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub mac: [u8; 6],
}

impl Adapter {
    fn same_subnet(&self, ip: Ipv4Addr) -> bool {
        let m = u32::from(self.netmask);
        u32::from(ip) & m == u32::from(self.ip) & m
    }
}

impl From<&Interface> for Adapter {
    fn from(i: &Interface) -> Adapter {
        Adapter {
            name: i.name.clone(),
            description: i.description.clone(),
            ip: i.ip,
            broadcast: i.broadcast(),
            netmask: i.netmask(),
            mac: i.mac,
        }
    }
}

/// Settings of the service.
#[derive(Debug, Clone, Default)]
pub struct LinkConfig {
    /// Adapter name, description part or IP; `None` = automatic (Pioneer/AlphaTheta adapter first).
    pub adapter: Option<String>,
    /// Computer name sent to players (0x11).
    pub hostname: String,
    pub library: LibraryInfo,
    /// Drive letters offered to the players over mountd (`/C/`, `/D/`, …).
    pub drives: Vec<char>,
    /// The library offered over the database server; `None` = port query answers "not ready".
    /// When set, its counts replace `library`.
    pub catalog: Option<Arc<Catalog>>,
    /// TCP port of the database server; `None` = [`crate::server::DB_PORT`] (12524).
    pub db_port: Option<u16>,
}

/// Everything the service reports.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    /// Now running on this adapter (`None` = no usable adapter, waiting).
    Adapter(Option<Adapter>),
    Presence(Event),
    /// A portmapper/mountd request was answered.
    Rpc {
        from: Ipv4Addr,
        kind: rpc::Kind,
    },
    /// A database request (message type) on a connection.
    Db {
        from: SocketAddr,
        kind: u16,
    },
}

enum Control {
    Library(LibraryInfo),
    MasterBpm(Option<f64>),
}

/// Handle to the running service; stops it when dropped.
pub struct LinkHandle {
    stop: Arc<AtomicBool>,
    control: Sender<Control>,
    events: Receiver<LinkEvent>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    db: Option<DbServer>,
}

impl LinkHandle {
    /// Events since the last call.
    pub fn events(&self) -> Vec<LinkEvent> {
        self.events.try_iter().collect()
    }

    pub fn set_library(&self, lib: LibraryInfo) {
        let _ = self.control.send(Control::Library(lib));
    }

    /// Offer another library (e.g. after a rescan).
    pub fn set_catalog(&self, c: Arc<Catalog>) {
        let info = c.info();
        self.shared.set_catalog(c);
        self.set_library(info);
    }

    /// TCP port of the database server.
    pub fn db_port(&self) -> Option<u16> {
        self.db.as_ref().map(|d| d.db_port)
    }

    pub fn set_master_bpm(&self, bpm: Option<f64>) {
        let _ = self.control.send(Control::MasterBpm(bpm));
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if let Some(d) = self.db.take() {
            d.stop();
        }
    }
}

impl Drop for LinkHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn bind(port: u16) -> Result<UdpSocket, LinkError> {
    let s = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port))
        .map_err(|source| LinkError::PortInUse { port, source })?;
    s.set_broadcast(true)?;
    s.set_nonblocking(true)?;
    Ok(s)
}

/// Choose the adapter as configured.
pub fn choose_adapter(wanted: Option<&str>) -> Option<Adapter> {
    let list = netif::list();
    netif::choose(&list, wanted)
        .filter(|i| i.up && !i.loopback)
        .map(Adapter::from)
}

/// Start the presence; returns once the sockets are bound.
pub fn start(mut cfg: LinkConfig) -> Result<LinkHandle, LinkError> {
    let sockets = [bind(PORT_ANNOUNCE)?, bind(PORT_BEAT)?, bind(PORT_STATUS)?];
    let rpc_sockets = [bind(PORT_PORTMAP)?, bind(PORT_MOUNTD)?, bind(PORT_NFS)?];
    let shared = Arc::new(Shared::default());
    if let Some(c) = cfg.catalog.take() {
        cfg.library = c.info();
        shared.set_catalog(c);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (ctl_tx, ctl_rx) = channel();
    let (ev_tx, ev_rx) = channel();
    let db = DbServer::start(
        shared.clone(),
        rekordbox_formats::dbserver::PORT_QUERY,
        cfg.db_port.unwrap_or(crate::server::DB_PORT),
        ev_tx.clone(),
    )?;
    let flag = stop.clone();
    let sh = shared.clone();
    let thread = std::thread::Builder::new()
        .name("rbx-link".into())
        .spawn(move || run(cfg, sockets, rpc_sockets, flag, sh, ctl_rx, ev_tx))?;
    Ok(LinkHandle {
        stop,
        control: ctl_tx,
        events: ev_rx,
        thread: Some(thread),
        shared,
        db: Some(db),
    })
}

fn send(sockets: &[UdpSocket; 3], adapter: &Adapter, out: Vec<Out>) {
    for o in out {
        let (ip, port, bytes) = match o {
            Out::Broadcast { port, bytes } => (adapter.broadcast, port, bytes),
            Out::Unicast { ip, port, bytes } => (ip, port, bytes),
        };
        // send from the socket of the target port so that answers come from 50002 like rekordbox's
        let sock = match port {
            PORT_ANNOUNCE => &sockets[0],
            PORT_BEAT => &sockets[1],
            _ => &sockets[2],
        };
        match sock.send_to(&bytes, SocketAddrV4::new(ip, port)) {
            Ok(_) => trace!("-> {ip}:{port} {}", hex(&bytes)),
            Err(e) => debug!("send to {ip}:{port} failed: {e}"),
        }
    }
}

/// Datagram as hex for the trace log (`RUST_LOG=rbx_link::net=trace`).
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn run(
    cfg: LinkConfig,
    sockets: [UdpSocket; 3],
    rpc_sockets: [UdpSocket; 3],
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    control: Receiver<Control>,
    events: Sender<LinkEvent>,
) {
    let t0 = Instant::now();
    let now = || t0.elapsed().as_secs_f64();
    let mut library = cfg.library;
    let mut master_bpm = None;
    let mut adapter: Option<Adapter> = None;
    let mut presence: Option<Presence> = None;
    let mut last_check = Instant::now() - ADAPTER_CHECK;
    let mut buf = [0u8; 0x5C0];
    let mut nfs_fs: Option<(Vec<char>, Fs)> = None;
    while !stop.load(Ordering::Relaxed) {
        for c in control.try_iter() {
            match c {
                Control::Library(l) => {
                    library = l;
                    if let Some(p) = presence.as_mut() {
                        p.set_library(l);
                    }
                }
                Control::MasterBpm(b) => {
                    master_bpm = b;
                    if let Some(p) = presence.as_mut() {
                        p.set_master_bpm(b);
                    }
                }
            }
        }
        // hot-plug: the unit's adapter appears, disappears or changes its address
        if last_check.elapsed() >= ADAPTER_CHECK {
            last_check = Instant::now();
            let found = choose_adapter(cfg.adapter.as_deref());
            if found != adapter {
                if let (Some(p), Some(a)) = (presence.as_mut(), adapter.as_ref()) {
                    send(&sockets, a, p.stop());
                }
                match &found {
                    Some(a) => {
                        info!("Pro DJ Link on {} ({}, {})", a.name, a.description, a.ip);
                        shared.set_subnet(a.ip, a.netmask);
                    }
                    None => {
                        warn!("no usable network adapter – waiting");
                        shared.subnet.store(0, Ordering::Relaxed);
                    }
                }
                shared.own.store(0, Ordering::Relaxed);
                presence = found.as_ref().map(|a| {
                    let mut p = Presence::new(Config {
                        mac: a.mac,
                        ip: a.ip,
                        hostname: cfg.hostname.clone(),
                        library,
                    });
                    p.set_master_bpm(master_bpm);
                    p.start(now());
                    p
                });
                adapter = found.clone();
                let _ = events.send(LinkEvent::Adapter(found));
            }
        }
        let mut busy = false;
        if let (Some(p), Some(a)) = (presence.as_mut(), adapter.as_ref()) {
            for (i, sock) in sockets.iter().enumerate() {
                let port = [PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS][i];
                loop {
                    match sock.recv_from(&mut buf) {
                        Ok((n, std::net::SocketAddr::V4(from))) => {
                            busy = true;
                            trace!("<- {from} :{port} {}", hex(&buf[..n]));
                            let out = p.receive(now(), *from.ip(), port, &buf[..n]);
                            send(&sockets, a, out);
                        }
                        Ok(_) => busy = true,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(e) => {
                            // e.g. ICMP "port unreachable" reported on the next receive (Windows)
                            debug!("receive on {port}: {e}");
                            break;
                        }
                    }
                }
            }
            // like rekordbox: no exports while the library is not ready yet
            let drives: &[char] = if shared.catalog().is_some() {
                &cfg.drives
            } else {
                &[]
            };
            // keep one Fs per drive set (its handle table must persist across datagrams)
            if nfs_fs.as_ref().map(|(d, _)| d.as_slice()) != Some(drives) {
                nfs_fs = Some((drives.to_vec(), Fs::new(drives)));
            }
            let fs = &nfs_fs.as_ref().unwrap().1;
            busy |= serve_rpc(&rpc_sockets, a, drives, fs, &events, &mut buf);
            let out = p.tick(now());
            send(&sockets, a, out);
            for e in p.take_events() {
                match &e {
                    Event::Claimed(n) => shared.own.store(*n, Ordering::Relaxed),
                    Event::Conflict { .. } => shared.own.store(0, Ordering::Relaxed),
                    _ => {}
                }
                if !matches!(e, Event::Status(_)) {
                    debug!("{e:?}");
                }
                let _ = events.send(LinkEvent::Presence(e));
            }
        } else {
            // no adapter: drain the sockets so nothing piles up
            for sock in sockets.iter().chain(&rpc_sockets) {
                while sock.recv_from(&mut buf).is_ok() {}
            }
        }
        if !busy {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if let (Some(p), Some(a)) = (presence.as_mut(), adapter.as_ref()) {
        send(&sockets, a, p.stop());
    }
}

/// Answer pending portmapper/mountd datagrams; true if there were any.
fn serve_rpc(
    sockets: &[UdpSocket; 3],
    adapter: &Adapter,
    drives: &[char],
    fs: &Fs,
    events: &Sender<LinkEvent>,
    buf: &mut [u8],
) -> bool {
    let exports = Exports {
        drives: drives.to_vec(),
        ip: adapter.ip,
        netmask: adapter.netmask,
    };
    let mut busy = false;
    for (port, sock) in [PORT_PORTMAP, PORT_MOUNTD, PORT_NFS]
        .into_iter()
        .zip(sockets)
    {
        loop {
            let (n, from) = match sock.recv_from(buf) {
                Ok((n, SocketAddr::V4(from))) => (n, from),
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    debug!("rpc receive: {e}");
                    break;
                }
            };
            busy = true;
            if !adapter.same_subnet(*from.ip()) {
                debug!("rpc from {from} outside the link subnet ignored");
                continue;
            }
            if port == PORT_NFS {
                let Ok(call) = rpc::parse_call(&buf[..n]) else {
                    continue;
                };
                let (answer, what) = crate::nfs::answer(fs, &call);
                if let Err(e) = sock.send_to(&answer, from) {
                    debug!("nfs answer to {from}: {e}");
                }
                trace!("nfs {from}: {what}");
                continue;
            }
            if let Some((answer, kind)) = rpc::answer(&exports, &buf[..n]) {
                if let Err(e) = sock.send_to(&answer, from) {
                    debug!("rpc answer to {from}: {e}");
                }
                trace!(
                    "rpc {from}: {kind:?} <- {} -> {}",
                    hex(&buf[..n]),
                    hex(&answer)
                );
                let _ = events.send(LinkEvent::Rpc {
                    from: *from.ip(),
                    kind,
                });
            }
        }
    }
    busy
}
