//! `link-announce` – join the Pro DJ Link network like rekordbox in LINK EXPORT mode and report what
//! the players do (Phase 1 live test; rekordbox must be closed).
//!
//! ```text
//! link-announce [--seconds N] [--adapter NAME|IP] [--tracks N] [--playlists N] [--drives CD]
//!               [--log FILE]
//! ```
//! There is no library server yet: the unit should list the computer under LINK and ask for its
//! media (0x05), but browsing does not work before Phase 4.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rbx_link::devices::Event;
use rbx_link::net::{self, LinkConfig, LinkEvent};
use rbx_link::packets::LibraryInfo;

/// Prints and, with `--log`, writes the same line to the file.
struct Out(Option<File>);

impl Out {
    fn line(&mut self, s: &str) {
        println!("{s}");
        if let Some(f) = self.0.as_mut() {
            let _ = writeln!(f, "{s}");
        }
    }
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .try_init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let val = |k: &str| {
        args.iter()
            .rposition(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let num = |k: &str, d: u16| val(k).and_then(|s| s.parse().ok()).unwrap_or(d);
    let seconds: f64 = val("--seconds")
        .and_then(|s| s.parse().ok())
        .unwrap_or(60.0);
    let mut out = Out(None);
    if let Some(path) = val("--log") {
        if let Some(dir) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match File::create(&path) {
            // BOM so that Notepad shows the umlauts
            Ok(mut f) => {
                let _ = f.write_all("\u{FEFF}".as_bytes());
                out.0 = Some(f);
            }
            Err(e) => eprintln!("log {path}: {e}"),
        }
    }
    let cfg = LinkConfig {
        adapter: val("--adapter"),
        hostname: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "rekordbox-rs".into()),
        library: LibraryInfo {
            tracks: num("--tracks", 0),
            playlists: num("--playlists", 0),
        },
        drives: val("--drives")
            .unwrap_or_else(|| "C".into())
            .chars()
            .filter(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .collect(),
    };
    let link = match net::start(cfg) {
        Ok(l) => l,
        Err(e) => {
            out.line(&format!("error: {e}"));
            return ExitCode::FAILURE;
        }
    };
    out.line(&format!(
        "announcing for {seconds} s – on the unit: choose LINK / the source selection"
    ));
    let t = Instant::now();
    let mut requests: BTreeMap<(u8, u8), usize> = BTreeMap::new();
    let mut play: BTreeMap<u8, u32> = BTreeMap::new();
    let mut rpcs: BTreeMap<String, usize> = BTreeMap::new();
    while t.elapsed().as_secs_f64() < seconds {
        for e in link.events() {
            let at = t.elapsed().as_secs_f64();
            let line = match e {
                LinkEvent::Adapter(Some(a)) => {
                    format!("{at:8.2} adapter {} ({}, {})", a.name, a.description, a.ip)
                }
                LinkEvent::Adapter(None) => format!("{at:8.2} no usable adapter – waiting"),
                LinkEvent::Presence(Event::Status(s)) => {
                    // only changes, the unit sends status every 200 ms per deck
                    if play.insert(s.device, s.play_state) == Some(s.play_state) {
                        continue;
                    }
                    format!(
                        "{at:8.2} deck {} play state {:#x}, track {}, tempo {:?}",
                        s.device, s.play_state, s.track_ref, s.tempo
                    )
                }
                LinkEvent::Presence(Event::Request { from, kind }) => {
                    let n = requests.entry((from, kind)).or_default();
                    *n += 1;
                    format!("{at:8.2} request {kind:#04x} from #{from} ({n}.)")
                }
                LinkEvent::Presence(Event::DeviceAdded(d)) => format!(
                    "{at:8.2} device #{} {} (kind {}, {}){}",
                    d.number,
                    d.name,
                    d.kind,
                    d.ip,
                    if d.second_deck { " second deck" } else { "" }
                ),
                LinkEvent::Presence(e) => format!("{at:8.2} {e:?}"),
                LinkEvent::Rpc { from, kind } => {
                    let what = format!("{kind:?}");
                    let n = rpcs.entry(what.clone()).or_default();
                    *n += 1;
                    // the unit polls the export list every 200 ms – show only the first few
                    if *n > 3 {
                        continue;
                    }
                    format!("{at:8.2} rpc from {from}: {what} ({n}.)")
                }
            };
            out.line(&line);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    link.stop();
    out.line("\nrequests answered or seen:");
    for ((from, kind), n) in &requests {
        out.line(&format!("  #{from} {kind:#04x} × {n}"));
    }
    for (what, n) in &rpcs {
        out.line(&format!("  rpc {what} × {n}"));
    }
    if requests.keys().any(|(_, k)| *k == 0x05) {
        out.line("  media query 0x05 came – the unit asked for our library (answered with 0x06)");
    } else {
        out.line("  (no media query 0x05 – the unit did not ask for our library)");
    }
    ExitCode::SUCCESS
}
