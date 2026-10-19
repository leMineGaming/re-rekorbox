//! `link-sniff` – decode Pro DJ Link traffic, live or from a capture.
//!
//! ```text
//! link-sniff --pcap capture.pcapng [--db]   decode UDP 50000–50002 (and with --db the dbserver streams)
//! link-sniff [--seconds N]                  listen live on 50000–50002 (passive; rekordbox must be closed)
//! ```
//! Copies of the same datagram within 3 ms (pktmon records several layers) are shown once.

use std::collections::HashMap;
use std::net::{Ipv4Addr, UdpSocket};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rbx_link::packets::{describe, PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS};
use rekordbox_formats::{capture, dbserver};

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let val = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let result = match val("--pcap") {
        Some(f) => from_capture(&f, args.iter().any(|a| a == "--db")),
        None => live(
            val("--seconds")
                .and_then(|s| s.parse().ok())
                .unwrap_or(30.0),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn from_capture(path: &str, db: bool) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let packets = capture::read(&data)?;
    let mut last: HashMap<(Ipv4Addr, u16, Vec<u8>), f64> = HashMap::new();
    for p in packets
        .iter()
        .filter(|p| !p.tcp && (PORT_ANNOUNCE..=PORT_STATUS).contains(&p.dst.1))
    {
        let key = (p.src.0, p.dst.1, p.payload.clone());
        if last.get(&key).is_some_and(|t| p.time - t < 0.003) {
            continue;
        }
        last.insert(key, p.time);
        println!(
            "{:9.3} {}:{} -> {}:{}  {}",
            p.time,
            p.src.0,
            p.src.1,
            p.dst.0,
            p.dst.1,
            describe(p.dst.1, &p.payload)
        );
    }
    if db {
        for ((src, dst), s) in capture::tcp_streams(&packets) {
            let data = s
                .data
                .strip_prefix(&dbserver::GREETING[..])
                .unwrap_or(&s.data);
            let (msgs, skips) = dbserver::decode_stream_resync(data);
            if msgs.is_empty() {
                continue;
            }
            println!(
                "\n== dbserver {}:{} -> {}:{} ({} messages, {skips} gaps)",
                src.0,
                src.1,
                dst.0,
                dst.1,
                msgs.len()
            );
            for m in &msgs {
                println!("{}", capture::describe(m));
            }
        }
    }
    Ok(())
}

fn live(seconds: f64) -> Result<(), String> {
    let mut socks = Vec::new();
    for port in [PORT_ANNOUNCE, PORT_BEAT, PORT_STATUS] {
        let s = UdpSocket::bind(("0.0.0.0", port))
            .map_err(|e| format!("UDP {port}: {e} – is rekordbox running?"))?;
        s.set_nonblocking(true).map_err(|e| e.to_string())?;
        socks.push((port, s));
    }
    println!("listening {seconds} s on UDP 50000–50002 (nothing is sent)");
    let t = Instant::now();
    let mut buf = [0u8; 0x5C0];
    while t.elapsed().as_secs_f64() < seconds {
        let mut busy = false;
        for (port, s) in &socks {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                busy = true;
                println!(
                    "{:9.3} {from} -> :{port}  {}",
                    t.elapsed().as_secs_f64(),
                    describe(*port, &buf[..n])
                );
            }
        }
        if !busy {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    Ok(())
}
