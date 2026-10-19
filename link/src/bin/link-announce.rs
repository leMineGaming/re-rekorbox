//! `link-announce` – join the Pro DJ Link network like rekordbox in LINK EXPORT mode and report what
//! the players do (Phase 1 live test; rekordbox must be closed).
//!
//! ```text
//! link-announce [--seconds N] [--adapter NAME|IP] [--tracks N] [--playlists N] [--drives CD]
//!               [--music DIR] [--no-library] [--db-port N]
//!               [--db-delay S] [--log FILE]
//! ```
//! Offers the library (rekordbox's analyses and our own, matched against `--music DIR` or the Music
//! folder) to the unit for browsing; `--no-library` announces without it. Loading tracks (NFS) is
//! Phase 5.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rbx_link::catalog::Catalog;
use rbx_link::devices::Event;
use rbx_link::net::{self, LinkConfig, LinkEvent};
use rbx_link::packets::LibraryInfo;
use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};

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
    let catalog = if args.iter().any(|a| a == "--no-library") {
        None
    } else {
        let t = Instant::now();
        let c = load_catalog(val("--music").map(PathBuf::from));
        out.line(&format!(
            "library: {} playable tracks ({:.1} s)",
            c.len(),
            t.elapsed().as_secs_f64()
        ));
        Some(Arc::new(c))
    };
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
        catalog: None,
        db_port: val("--db-port").and_then(|s| s.parse().ok()),
    };
    // like rekordbox while its database is still opening: the port query answers `ff ff` for a while
    let db_delay: f64 = val("--db-delay")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let mut cfg = cfg;
    if let Some(c) = &catalog {
        cfg.library = c.info();
        // diagnosis: report playlists although there are none yet
        if let Some(n) = val("--playlists").and_then(|s| s.parse().ok()) {
            cfg.library.playlists = n;
        }
        if db_delay <= 0.0 {
            cfg.catalog = Some(c.clone());
        }
    }
    let mut delayed = catalog.filter(|_| db_delay > 0.0);
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
    // per deck: (play state, track id, bpm×100, beat) – to print only on change
    type DeckSnapshot = (u8, Option<u32>, Option<u16>, Option<u32>);
    let mut play: BTreeMap<u8, DeckSnapshot> = BTreeMap::new();
    let mut rpcs: BTreeMap<String, usize> = BTreeMap::new();
    let mut db: BTreeMap<u16, usize> = BTreeMap::new();
    while t.elapsed().as_secs_f64() < seconds {
        if t.elapsed().as_secs_f64() >= db_delay {
            if let Some(c) = delayed.take() {
                link.set_catalog(c);
                out.line(&format!("{:8.2} database ready", t.elapsed().as_secs_f64()));
            }
        }
        for e in link.events() {
            let at = t.elapsed().as_secs_f64();
            let line = match e {
                LinkEvent::Adapter(Some(a)) => {
                    format!("{at:8.2} adapter {} ({}, {})", a.name, a.description, a.ip)
                }
                LinkEvent::Adapter(None) => format!("{at:8.2} no usable adapter – waiting"),
                LinkEvent::Presence(Event::Status(s)) => {
                    // the unit sends status ~3×/s per deck; print only when something changed
                    let now = (s.play_state, s.track_id, s.bpm_x100, s.beat);
                    if play.insert(s.device, now) == Some(now) {
                        continue;
                    }
                    format!(
                        "{at:8.2} deck {} play {:#04x} track {:?} bpm {:?} pitch {:+.2}% beat {:?} bar {}",
                        s.device,
                        s.play_state,
                        s.track_id,
                        s.bpm(),
                        s.pitch_percent(),
                        s.beat,
                        s.beat_in_bar
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
                LinkEvent::Db { from, kind } => {
                    let n = db.entry(kind).or_default();
                    *n += 1;
                    format!(
                        "{at:8.2} db {from}: {kind:#06x} {}",
                        rekordbox_formats::dbserver::kind::name(kind)
                    )
                }
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
    for (kind, n) in &db {
        out.line(&format!(
            "  db {kind:#06x} {} × {n}",
            rekordbox_formats::dbserver::kind::name(*kind)
        ));
    }
    if requests.keys().any(|(_, k)| *k == 0x05) {
        out.line("  media query 0x05 came – the unit asked for our library (answered with 0x06)");
    } else {
        out.line("  (no media query 0x05 – the unit did not ask for our library)");
    }
    ExitCode::SUCCESS
}

/// rekordbox's analyses plus our own (`%APPDATA%\rekordbox-rs\ANLZ`), matched against the music folder.
fn load_catalog(music: Option<PathBuf>) -> Catalog {
    let mut roots: Vec<PathBuf> = default_anlz_root().into_iter().collect();
    if let Some(own) = std::env::var_os("APPDATA")
        .map(|a| PathBuf::from(a).join("rekordbox-rs").join("ANLZ"))
        .filter(|p| p.is_dir())
    {
        roots.push(own);
    }
    let music: Vec<PathBuf> = music.or_else(default_music_root).into_iter().collect();
    Catalog::from_library(&Library::build_multi(&roots, &music))
}
