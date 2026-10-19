//! TCP side of the database server: the port query on 12523 (`00 00 00 0f "RemoteDBServer\0"` →
//! u16 port, `ff ff` while no library is loaded, as rekordbox) and the database port ([`DB_PORT`],
//! fixed so that a firewall rule can name it; rekordbox picks a free one) with one thread per
//! connection running a [`Session`]. Only devices in the link adapter's subnet are served.

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use rekordbox_formats::capture;
use rekordbox_formats::dbserver::{self as wire, GREETING, PORT_QUERY_REQUEST};
use tracing::{debug, info};

use crate::catalog::Catalog;
use crate::dbserver::{encode, Reply, Session};
use crate::error::LinkError;
use crate::net::LinkEvent;

/// Default TCP port of the database server.
pub const DB_PORT: u16 = 12524;

/// How long a blocking read waits before checking the stop flag.
const POLL: Duration = Duration::from_millis(500);

/// State shared between the network thread and the server threads.
#[derive(Debug, Default)]
pub struct Shared {
    pub catalog: RwLock<Option<Arc<Catalog>>>,
    /// Our device number (0 = not claimed yet).
    pub own: AtomicU8,
    /// Link adapter address and netmask (`ip << 32 | mask`, 0 = no adapter).
    pub subnet: AtomicU64,
    pub stop: AtomicBool,
}

impl Shared {
    pub fn set_subnet(&self, ip: Ipv4Addr, mask: Ipv4Addr) {
        let v = (u64::from(u32::from(ip)) << 32) | u64::from(u32::from(mask));
        self.subnet.store(v, Ordering::Relaxed);
    }

    fn allowed(&self, peer: SocketAddr) -> bool {
        let SocketAddr::V4(p) = peer else {
            return false;
        };
        let v = self.subnet.load(Ordering::Relaxed);
        let (ip, mask) = ((v >> 32) as u32, v as u32);
        v != 0 && u32::from(*p.ip()) & mask == ip & mask
    }

    pub fn catalog(&self) -> Option<Arc<Catalog>> {
        match self.catalog.read() {
            Ok(c) => c.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn set_catalog(&self, c: Arc<Catalog>) {
        match self.catalog.write() {
            Ok(mut g) => *g = Some(c),
            Err(poisoned) => *poisoned.into_inner() = Some(c),
        }
    }
}

/// The running listeners.
pub struct DbServer {
    pub db_port: u16,
    pub query_port: u16,
    threads: Vec<JoinHandle<()>>,
    shared: Arc<Shared>,
}

impl DbServer {
    /// Bind the port query (`query_port`, normally 12523) and the database port (`db_port`, normally
    /// [`DB_PORT`]); 0 = any free port (tests).
    pub fn start(
        shared: Arc<Shared>,
        query_port: u16,
        db_port: u16,
        events: Sender<LinkEvent>,
    ) -> Result<DbServer, LinkError> {
        let query =
            TcpListener::bind(("0.0.0.0", query_port)).map_err(|source| LinkError::PortInUse {
                port: query_port,
                source,
            })?;
        let db =
            TcpListener::bind(("0.0.0.0", db_port)).map_err(|source| LinkError::PortInUse {
                port: db_port,
                source,
            })?;
        let db_port = db.local_addr()?.port();
        let query_port = query.local_addr()?.port();
        query.set_nonblocking(true)?;
        db.set_nonblocking(true)?;
        info!("database server on TCP {db_port}, port query on {query_port}");
        let mut threads = Vec::new();
        {
            let shared = shared.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("rbx-link-query".into())
                    .spawn(move || {
                        accept_loop(&query, &shared, |s| port_query(s, &shared, db_port))
                    })?,
            );
        }
        {
            let shared = shared.clone();
            threads.push(
                std::thread::Builder::new()
                    .name("rbx-link-db".into())
                    .spawn(move || {
                        accept_loop(&db, &shared, |s| {
                            let (shared, events) = (shared.clone(), events.clone());
                            let spawned = std::thread::Builder::new()
                                .name("rbx-link-session".into())
                                .spawn(move || {
                                    let peer = s.peer_addr().ok();
                                    if let Err(e) = session(s, &shared, &events) {
                                        debug!("database connection {peer:?} ended: {e}");
                                    }
                                });
                            if let Err(e) = spawned {
                                debug!("no session thread: {e}");
                            }
                        })
                    })?,
            );
        }
        Ok(DbServer {
            db_port,
            query_port,
            threads,
            shared,
        })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for DbServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(l: &TcpListener, shared: &Shared, mut on: impl FnMut(TcpStream)) {
    while !shared.stop.load(Ordering::Relaxed) {
        match l.accept() {
            Ok((s, peer)) => {
                if !shared.allowed(peer) {
                    debug!("TCP from {peer} outside the link subnet refused");
                    continue;
                }
                if s.set_nonblocking(false).is_ok() {
                    on(s);
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => {
                debug!("accept: {e}");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn port_query(mut s: TcpStream, shared: &Shared, db_port: u16) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut req = [0u8; 19];
    if s.read_exact(&mut req).is_err() || req.as_slice() != PORT_QUERY_REQUEST {
        debug!("unknown port query from {:?}", s.peer_addr().ok());
        return;
    }
    let ready = shared.catalog().is_some() && shared.own.load(Ordering::Relaxed) != 0;
    let answer = if ready { db_port } else { 0xFFFF };
    debug!("port query from {:?} -> {answer:#06x}", s.peer_addr().ok());
    let _ = s.write_all(&answer.to_be_bytes());
}

fn timed_out(e: &std::io::Error) -> bool {
    matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// Read exactly `buf.len()` bytes, checking the stop flag between timeouts.
fn read_full(s: &mut TcpStream, buf: &mut [u8], shared: &Shared) -> std::io::Result<()> {
    let mut got = 0;
    while got < buf.len() {
        if shared.stop.load(Ordering::Relaxed) {
            return Err(ErrorKind::Interrupted.into());
        }
        match s.read(&mut buf[got..]) {
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn session(mut s: TcpStream, shared: &Shared, events: &Sender<LinkEvent>) -> std::io::Result<()> {
    let peer = s.peer_addr()?;
    s.set_read_timeout(Some(POLL))?;
    s.set_nodelay(true)?;
    let mut greet = [0u8; 5];
    read_full(&mut s, &mut greet, shared)?;
    if greet != GREETING {
        debug!("unexpected greeting from {peer}: {greet:02x?}");
        return Ok(());
    }
    s.write_all(&greet)?;
    info!("database connection from {peer}");
    let mut session = Session::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let (msgs, used, err) = wire::decode_stream(&buf);
        buf.drain(..used);
        for m in msgs {
            debug!("db <- {peer}{}", capture::describe(&m));
            let _ = events.send(LinkEvent::Db {
                from: peer,
                kind: m.kind,
            });
            let Some(catalog) = shared.catalog() else {
                return Ok(());
            };
            match session.handle(&catalog, shared.own.load(Ordering::Relaxed), &m) {
                Reply::Close => return Ok(()),
                Reply::Send(out) => {
                    let mut bytes = Vec::new();
                    for r in &out {
                        bytes.extend(encode(r));
                    }
                    s.write_all(&bytes)?;
                }
            }
        }
        if let Some(e) = err {
            debug!("protocol error from {peer}: {e}");
            return Ok(());
        }
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        match s.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if timed_out(&e) => {}
            Err(e) => return Err(e),
        }
    }
}
