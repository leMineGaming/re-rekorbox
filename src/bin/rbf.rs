//! `rbf` – inspect and validate rekordbox files with the clean-room implementation.
//!
//! ```text
//! rbf anlz  <ANLZ file>          show tags of a .DAT/.EXT/.2EX (or decode a .3EX)
//! rbf check <directory>          parse every ANLZ file below <directory>, verify byte-exact round trips
//! rbf xml   <rekordbox.xml>      summarise a rekordbox.xml
//! rbf xml-rewrite <in> <out>     read and write a rekordbox.xml with this implementation
//! rbf midi <file|dir>            validate MIDI mapping CSV files
//! rbf midi-route <mapping>       list the input messages a mapping listens to, per control type
//! rbf midi-decode <mapping> <hex bytes>...   process messages offline, e.g. `B0 22 41 97 00 7F`
//! rbf midi-devices               list MIDI input/output devices (Windows)
//! rbf midi-monitor <mapping> [device] [seconds]   decode live MIDI input with a mapping (Windows)
//! rbf djlink-listen [seconds] [--raw]   passively decode Pro DJ Link traffic on UDP 50000-50002
//! ```

use rekordbox_formats::anlz::waveform::WavePreview;
use rekordbox_formats::anlz::{threex, AnlzFile, Tag};
use rekordbox_formats::xml;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["anlz", f] => show_anlz(Path::new(f)),
        ["check", d] => check(Path::new(d)),
        ["xml", f] => show_xml(Path::new(f)),
        ["xml-rewrite", i, o] => rewrite_xml(Path::new(i), Path::new(o)),
        ["midi", p] => check_midi(Path::new(p)),
        ["pad", d] => check_pads(Path::new(d)),
        ["edb", f] => show_edb(Path::new(f)),
        ["midi-route", m] => midi_route(Path::new(m)),
        ["midi-decode", m, bytes @ ..] => midi_decode(Path::new(m), bytes),
        #[cfg(windows)]
        ["midi-devices"] => midi_devices(),
        #[cfg(windows)]
        ["midi-monitor", m, rest @ ..] => midi_monitor(Path::new(m), rest),
        ["djlink-listen", rest @ ..] => djlink_listen(rest),
        _ => Err(
            "usage: rbf anlz <file> | rbf check <dir> | rbf xml <file> | rbf xml-rewrite <in> <out> | rbf midi <file|dir> | rbf midi-route <mapping>              | rbf midi-decode <mapping> <hex>... | rbf midi-devices | rbf midi-monitor <mapping> [device] [seconds] | rbf djlink-listen [seconds] [--raw]"
                .into(),
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

type R = Result<(), String>;

fn is_3ex(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("3EX")).unwrap_or(false)
}

fn show_anlz(path: &Path) -> R {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    if is_3ex(path) {
        let v = threex::decode(&data).map_err(|e| e.to_string())?;
        println!("{v:#?}");
        return Ok(());
    }
    let f = AnlzFile::parse(&data).map_err(|e| e.to_string())?;
    println!("PMAI fields={:?} tail={} tags={} size={}", f.header.fields, f.header.tail, f.tags.len(), data.len());
    for raw in &f.tags {
        print!("  {} header={} total={}", raw.fourcc, raw.header_len(), raw.total_len());
        match raw.decode() {
            Ok(Tag::Path(p)) => println!("  path={p:?}"),
            Ok(Tag::BeatGrid(g)) => {
                let first = g.beats.first().map(|b| format!("{:.2} BPM @ {} ms", b.bpm(), b.time_ms));
                println!("  beats={} first={}", g.beats.len(), first.unwrap_or_default());
            }
            Ok(Tag::WavePreview(w)) => {
                println!("  bytes={} (expected {})", w.data.len(), WavePreview::expected_len(w.kind))
            }
            Ok(Tag::Waveform(w)) => {
                println!("  entries={} entry_size={} per_second={:?}", w.count(), w.entry_size, w.entries_per_second())
            }
            Ok(Tag::CueList(c)) => println!("  type={:?} cues={}", c.list_type, c.cues.len()),
            Ok(Tag::ExtBeatGrid(q)) => println!(
                "  version={} entries={} first={:?} last={:?}",
                q.version,
                q.count(),
                q.first_beat,
                q.last_beat
            ),
            Ok(Tag::Vbr2(v)) => println!("  samples={} entries={}", v.sample_count, v.count()),
            Ok(Tag::SongStructure(s)) => {
                println!("  mood={:?} end_beat={} phrases={}", s.mood, s.end_beat, s.phrases.len());
                for p in &s.phrases {
                    let fill = if p.fill != 0 { format!(" fill@{}", p.fill_beat) } else { String::new() };
                    println!("      #{:<3} beat {:>4}  {:<9}{}", p.index, p.beat, p.label(s.mood).unwrap_or("?"), fill);
                }
            }
            Ok(Tag::Vbr(v)) => println!("  table[0..4]={:?} extra={}", &v.table()[..4], v.extra()),
            Ok(Tag::Raw(_)) => println!("  (raw)"),
            Err(e) => println!("  DECODE ERROR: {e}"),
        }
    }
    Ok(())
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect(&p, out)?;
        } else if p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with("ANLZ")).unwrap_or(false) {
            out.push(p);
        }
    }
    Ok(())
}

/// Parse all ANLZ files, check container and typed round trips, and count labels/errors.
fn check(dir: &Path) -> R {
    let mut files = Vec::new();
    collect(dir, &mut files).map_err(|e| e.to_string())?;
    let mut stats: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for p in &files {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_uppercase();
        *stats.entry(format!("files .{ext}")).or_default() += 1;
        let data = match std::fs::read(p) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        if is_3ex(p) {
            match threex::decode(&data) {
                Ok(v) if v.get("embedding").is_some() => *stats.entry("3EX decoded".into()).or_default() += 1,
                Ok(_) => failures.push(format!("{}: no embedding key", p.display())),
                Err(e) => failures.push(format!("{}: {e}", p.display())),
            }
            continue;
        }
        let f = match AnlzFile::parse(&data) {
            Ok(f) => f,
            Err(e) => {
                failures.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        if f.to_bytes() == data {
            *stats.entry("container round trip ok".into()).or_default() += 1;
        } else {
            failures.push(format!("{}: container round trip differs", p.display()));
        }
        if f.check_reader_order().is_err() {
            failures.push(format!("{}: first tag is not PPTH", p.display()));
        }
        for raw in &f.tags {
            match raw.decode() {
                Ok(tag) => {
                    let kind = if matches!(tag, Tag::Raw(_)) { "raw" } else { "typed" };
                    *stats.entry(format!("tag {} {kind}", raw.fourcc)).or_default() += 1;
                    if tag.to_raw() != *raw {
                        failures.push(format!("{}: typed round trip of {} differs", p.display(), raw.fourcc));
                    }
                    if let Tag::SongStructure(s) = &tag {
                        for ph in &s.phrases {
                            match ph.label(s.mood) {
                                Some(_) => *stats.entry("phrase labelled".into()).or_default() += 1,
                                None => *stats.entry("phrase WITHOUT label".into()).or_default() += 1,
                            }
                        }
                    }
                }
                Err(e) => failures.push(format!("{}: {} decode error: {e}", p.display(), raw.fourcc)),
            }
        }
    }
    for (k, v) in &stats {
        println!("{k:<32} {v}");
    }
    println!("failures                         {}", failures.len());
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

fn show_xml(path: &Path) -> R {
    let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (doc, report) = xml::read::from_str_with_report(&s).map_err(|e| e.to_string())?;
    println!("version {} product {:?}", doc.version, doc.product);
    println!("tracks {}  playlists (top level) {}", doc.tracks.len(), doc.playlists.len());
    let tempos: usize = doc.tracks.iter().map(|t| t.tempos.len()).sum();
    let marks: usize = doc.tracks.iter().map(|t| t.marks.len()).sum();
    println!(
        "tempos {tempos}  marks {marks}  skipped marks {}  skipped tempos {}",
        report.skipped_marks, report.skipped_tempos
    );
    Ok(())
}

/// Parse MIDI mapping CSV files, compute bindings, verify write/parse round trips.
fn check_midi(path: &Path) -> R {
    use rekordbox_formats::midi;
    let files: Vec<PathBuf> = if path.is_dir() {
        let mut v: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().ends_with(".midi.csv"))
            .collect();
        v.sort();
        v
    } else {
        vec![path.to_path_buf()]
    };
    let mut stats: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    for p in &files {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let m = midi::parse(&text);
        *stats.entry("files".into()).or_default() += 1;
        if !m.is_valid() {
            failures.push(format!("{}: not valid (version {})", p.display(), m.version));
        }
        *stats.entry("files with BOM".into()).or_default() += m.report.had_bom as usize;
        *stats.entry("entries".into()).or_default() += m.entries.len();
        *stats.entry("skipped comment lines".into()).or_default() += m.report.comment_lines;
        *stats.entry("skipped short rows".into()).or_default() += m.report.short_rows;
        *stats.entry("skipped empty name".into()).or_default() += m.report.empty_name;
        *stats.entry("skipped special (@) rows".into()).or_default() += m.report.special_rows;
        for e in &m.entries {
            let code = e.control_code(&m.device);
            if code == 0 && e.type_text != "Button" {
                *stats.entry(format!("type {:?} -> Button (unknown text)", e.type_text)).or_default() += 1;
            }
            *stats.entry("input bindings".into()).or_default() += e.input_bindings(&m.device).len();
            *stats.entry("output bindings".into()).or_default() += e.output_bindings().len();
        }
        match midi::write(&m) {
            Ok(t) => {
                let back = midi::parse(&t);
                let same = back.entries.len() == m.entries.len()
                    && back.entries.iter().zip(&m.entries).all(|(a, b)| {
                        let (mut a, mut b) = (a.clone(), b.clone());
                        a.line = 0;
                        b.line = 0;
                        a == b
                    });
                if same {
                    *stats.entry("write/parse round trip ok".into()).or_default() += 1;
                } else {
                    failures.push(format!("{}: round trip differs", p.display()));
                }
            }
            Err(e) => failures.push(format!("{}: {e}", p.display())),
        }
    }
    for (k, v) in &stats {
        println!("{k:<40} {v}");
    }
    println!("failures                                 {}", failures.len());
    for f in failures.iter().take(20) {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

/// Parse a `pad` directory: version, mode list, every pad file, and write round trips.
fn check_pads(dir: &Path) -> R {
    use rekordbox_formats::pad::{self, Pad};
    let version = std::fs::read_to_string(dir.join("Version")).unwrap_or_default();
    println!("version {}", version.trim());
    let modes = pad::parse_modes(&std::fs::read_to_string(dir.join("Mode.pad.csv")).map_err(|e| e.to_string())?);
    println!("modes {} ({} with a pad file)", modes.len(), modes.iter().filter(|m| m.pad_file_name().is_some()).count());
    let mut failures = Vec::new();
    let (mut files, mut pads, mut user) = (0, 0, 0);
    for e in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let p = e.map_err(|e| e.to_string())?.path();
        if !p.to_string_lossy().ends_with(".pad.csv") {
            continue;
        }
        let text = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
        let list = pad::parse(&text);
        files += 1;
        pads += list.len();
        user += list.iter().filter(|p| Pad::clone(p).apply_user_variant()).count();
        let normalized: String = text.lines().filter(|l| !l.is_empty()).map(|l| format!("{l}\n")).collect();
        if pad::write(&list) != normalized {
            failures.push(format!("{}: round trip differs", p.display()));
        }
    }
    let missing: Vec<String> =
        modes.iter().filter_map(|m| m.pad_file_name()).filter(|f| !dir.join(f).exists()).collect();
    println!("pad files {files}, pads {pads}, pads with user variant {user}");
    println!("modes without pad file: {missing:?}");
    println!("failures {}", failures.len());
    for f in &failures {
        println!("  {f}");
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} failures", failures.len()))
    }
}

/// Show the rows of a DeviceSQL `.edb` file grouped by table kind, with any strings found.
fn show_edb(path: &Path) -> R {
    use rekordbox_formats::devicesql::{DsqlString, EdbFile};
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let f = EdbFile::parse(&data).map_err(|e| e.to_string())?;
    println!("header {:x?}", f.header);
    let mut kinds: BTreeMap<u16, usize> = BTreeMap::new();
    let mut strings = 0;
    for (i, r) in f.rows.iter().enumerate() {
        *kinds.entry(r.kind()).or_default() += 1;
        let found: Vec<String> = (4..r.data.len())
            .filter(|&o| matches!(r.data[o], 0x90) || (r.data[o] & 1 == 1 && r.data[o] > 3))
            .filter_map(|o| DsqlString::decode(&r.data[o..]).ok().map(|(s, n)| (o, s, n)))
            .filter(|(o, s, n)| {
                // keep strings that end exactly at the row end or are followed only by padding
                let end = o + n;
                let text_ok = s.text().chars().all(|c| !c.is_control());
                text_ok && !s.text().is_empty() && r.data[end..].iter().all(|&b| b == 0)
            })
            .map(|(o, s, _)| format!("@{o}:{:?}", s.text()))
            .take(1)
            .collect();
        strings += found.len();
        println!(
            "row {i:2} kind {:#06x} shift {:#06x} len {:4} {}",
            r.kind(),
            r.index_shift(),
            r.data.len(),
            found.join(" ")
        );
    }
    println!("rows {} kinds {:x?} rows with a trailing string {strings}", f.rows.len(), kinds);
    Ok(())
}

fn rewrite_xml(input: &Path, output: &Path) -> R {
    let s = std::fs::read_to_string(input).map_err(|e| e.to_string())?;
    let doc = xml::from_str(&s).map_err(|e| e.to_string())?;
    std::fs::write(output, xml::to_string(&doc)).map_err(|e| e.to_string())
}

fn load_processor(path: &Path) -> Result<rekordbox_formats::midi_input::InputProcessor, String> {
    use rekordbox_formats::{midi, midi_input::InputProcessor};
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let m = midi::parse(&text);
    if !m.is_valid() {
        return Err(format!("{}: not a version 1 mapping file", path.display()));
    }
    Ok(InputProcessor::new(m))
}

fn midi_route(path: &Path) -> R {
    let p = load_processor(path)?;
    let m = p.mapping();
    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    for e in &m.entries {
        if !e.input_bindings(&m.device).is_empty() {
            *by_type.entry(format!("{} (0x{:02X})", e.type_text, e.control_code(&m.device))).or_default() += 1;
        }
    }
    println!("device {:?}: {} rows, {} input messages", m.device, m.entries.len(), p.listened_keys().len());
    for (t, n) in &by_type {
        println!("  {t:<28} {n:>4} input rows");
    }
    for &(i, code) in p.unhandled() {
        let e = &m.entries[i];
        println!("  no input handler: line {} {} ({} 0x{code:02X})", e.line, e.name, e.type_text);
    }
    Ok(())
}

fn print_events(p: &mut rekordbox_formats::midi_input::InputProcessor, msg: &rekordbox_formats::midi_input::MidiMessage) {
    let bytes = match msg.data2 {
        Some(d2) => format!("{:02X} {:02X} {:02X}", msg.status, msg.data1, d2),
        None => format!("{:02X} {:02X}   ", msg.status, msg.data1),
    };
    let evs = p.process(msg);
    if evs.is_empty() {
        let note = if p.listens(msg.key()) { "(stored, reported with the next message)" } else { "(not mapped)" };
        println!("{:>10.3}s  {bytes}  {note}", msg.time_s);
    }
    for ev in evs {
        let e = p.entry(&ev);
        let deck = if ev.deck == 0 { "  -".to_string() } else { format!("D{}", ev.deck) };
        println!("{:>10.3}s  {bytes}  {deck}  {:<24} {:?}", msg.time_s, e.function, ev.action);
    }
}

fn midi_decode(path: &Path, hex: &[&str]) -> R {
    use rekordbox_formats::midi_input::{message_len, MidiMessage};
    let mut p = load_processor(path)?;
    let bytes: Vec<u8> = hex
        .iter()
        .map(|h| u8::from_str_radix(h.trim_start_matches("0x"), 16).map_err(|e| format!("{h}: {e}")))
        .collect::<Result<_, _>>()?;
    let mut i = 0;
    while i < bytes.len() {
        let len = message_len(bytes[i]).min(bytes.len() - i);
        let msg = MidiMessage::from_bytes(&bytes[i..i + len], 0.0).ok_or(format!("byte {i}: not a status byte"))?;
        print_events(&mut p, &msg);
        i += len;
    }
    Ok(())
}

#[cfg(windows)]
fn midi_devices() -> R {
    use rekordbox_formats::midi_device::{list_inputs, list_outputs};
    println!("inputs:");
    for d in list_inputs() {
        println!("  {:>2}  {}", d.index, d.name);
    }
    println!("outputs:");
    for d in list_outputs() {
        println!("  {:>2}  {}", d.index, d.name);
    }
    Ok(())
}

#[cfg(windows)]
fn midi_monitor(path: &Path, rest: &[&str]) -> R {
    use rekordbox_formats::midi_device::{list_inputs, InputPort};
    use rekordbox_formats::midi_input::MidiMessage;
    use std::time::{Duration, Instant};
    let mut p = load_processor(path)?;
    let inputs = list_inputs();
    // default device: the one whose name contains the mapping's device name
    let wanted = rest.first().copied().unwrap_or(p.mapping().device.as_str()).to_string();
    let dev = inputs
        .iter()
        .find(|d| wanted.parse::<u32>().ok() == Some(d.index))
        .or_else(|| inputs.iter().find(|d| d.name.to_lowercase().contains(&wanted.to_lowercase())))
        .ok_or(format!("no MIDI input matching {wanted:?} (see `rbf midi-devices`)"))?;
    let seconds: Option<u64> = rest.get(1).and_then(|s| s.parse().ok());
    let (_port, rx) = InputPort::open(dev.index)?;
    println!("listening on {} ({}) with mapping {:?}; Ctrl+C to stop", dev.index, dev.name, p.mapping().device);
    let start = Instant::now();
    loop {
        if let Some(s) = seconds {
            if start.elapsed() >= Duration::from_secs(s) {
                return Ok(());
            }
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok((packed, ms)) => {
                if let Some(msg) = MidiMessage::from_packed(packed, f64::from(ms) / 1000.0) {
                    print_events(&mut p, &msg);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// Listen on the three DJ Link ports without sending anything; print decoded packets and the device
/// table built from keep-alives. `--raw` also prints every datagram in hex (for captures).
fn djlink_listen(rest: &[&str]) -> R {
    use rekordbox_formats::djlink::{self, Announce, DeviceTable, Packet};
    use std::net::UdpSocket;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let raw = rest.contains(&"--raw");
    let seconds: Option<u64> = rest.iter().find_map(|s| s.parse().ok());
    let (tx, rx) = mpsc::channel::<(u16, std::net::SocketAddr, Vec<u8>)>();
    let mut bound = Vec::new();
    for port in [djlink::PORT_ANNOUNCE, djlink::PORT_BEAT, djlink::PORT_STATUS] {
        match UdpSocket::bind(("0.0.0.0", port)) {
            Ok(sock) => {
                let tx = tx.clone();
                bound.push(port);
                std::thread::spawn(move || {
                    let mut buf = [0u8; 0x5C0];
                    while let Ok((n, from)) = sock.recv_from(&mut buf) {
                        if tx.send((port, from, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                });
            }
            Err(e) => eprintln!("port {port}: {e} (is rekordbox running?)"),
        }
    }
    if bound.is_empty() {
        return Err("no DJ Link port could be opened".into());
    }
    println!("listening on UDP {bound:?} (receive only); Ctrl+C to stop");
    let mut table = DeviceTable::new(None);
    let start = Instant::now();
    loop {
        if seconds.is_some_and(|s| start.elapsed() >= Duration::from_secs(s)) {
            break;
        }
        let Ok((port, from, data)) = rx.recv_timeout(Duration::from_millis(200)) else { continue };
        let t = start.elapsed().as_secs_f64();
        if raw {
            println!("{t:>9.3}s {port} {from} RAW {}", hex(&data));
        }
        match djlink::decode(port, &data) {
            Ok(Packet::Announce(h, Announce::KeepAlive(k))) => {
                let changes = table.apply(&k);
                println!(
                    "{t:>9.3}s {port} keep-alive {:?} #{} kind {} ip {} mac {} -> {changes:?}",
                    h.name, k.device, k.kind, k.ip, hex(&k.mac)
                );
            }
            Ok(Packet::Announce(h, Announce::Disconnect { device, ip })) => {
                table.remove(device);
                println!("{t:>9.3}s {port} disconnect {:?} #{device} ip {ip}", h.name);
            }
            Ok(Packet::Announce(h, a)) => println!("{t:>9.3}s {port} {:?}: {a:?}", h.name),
            Ok(Packet::Beat(b)) => println!(
                "{t:>9.3}s {port} beat #{} {:?} beat {}/4 tempo {:.2} BPM (pitch {:+.2} %) next beat {} ms",
                b.device, b.name, b.beat_in_bar, f64::from(b.effective_bpm100()) / 100.0,
                djlink::pitch_percent(b.pitch), b.next_beat_ms
            ),
            Ok(Packet::CdjStatus(s)) => println!(
                "{t:>9.3}s {port} status #{} {:?} play-state {} flags {:02X} tempo {} master-handoff {:?} load {:?}/{}",
                s.device, s.name, s.play_state, s.flags,
                s.effective_bpm100().map_or("-".into(), |v| format!("{:.2}", f64::from(v) / 100.0)),
                s.master_handoff, s.load_info, s.track_ref
            ),
            Ok(Packet::MixerStatus(m)) => println!("{t:>9.3}s {port} mixer #{} {:?} decks {:?}", m.device, m.name, m.decks),
            Ok(Packet::Other(h)) => println!(
                "{t:>9.3}s {port} type 0x{:02X} #{} {:?} ({} bytes, not decoded)", h.kind, h.device, h.name, data.len()
            ),
            Err(e) => println!("{t:>9.3}s {port} {from}: {e} ({} bytes)", data.len()),
        }
    }
    println!("devices:");
    for d in table.devices() {
        println!("  #{:<3} {:<20} kind {} ip {}{}", d.number, d.name, d.kind, d.ip, if d.second_deck { " (second deck)" } else { "" });
    }
    Ok(())
}
