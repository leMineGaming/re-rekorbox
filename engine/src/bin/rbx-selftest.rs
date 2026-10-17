//! `rbx-selftest` – everything that would otherwise have to be tried by hand, as automatic checks with
//! a report. Nothing is audible: audio devices only get silence, test audio is rendered offline.
//!
//! ```text
//! rbx-selftest [--quick] [--soak SECONDS] [--tracks N] [--midi-listen SECONDS] [--no-device]
//!              [--no-network] [--out DIR] [--open] [--pause]
//! ```
//! Sections: audio quality (clicks, pitch, sync, echo, levels, routing), sound devices (callback timing
//! with silence), performance, long run with random operation (offline and in real time), library
//! and analysis accuracy against rekordbox on this machine, MIDI ports and mapping coverage, network
//! adapters and Pro DJ Link (devices seen on the network). The report goes to
//! `%APPDATA%\rekordbox-rs\selftest\` (Markdown) and the console; exit code 1 when a check failed.
//! `--midi-listen 20` additionally records 20 s of controller input (move the controls meanwhile).
//! `--open` opens the HTML report afterwards, `--pause` waits for Enter before closing (used when the
//! UI starts the test in its own console window).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rbx_engine::deck::Grid;
use rbx_engine::dsp::ColorFx;
use rbx_engine::fx::BeatFxKind;
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::output::Output;
use rbx_engine::pcm::Pcm;
use rbx_engine::session::{data_dir, install_crash_handler, timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
    Info,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::Warn => "WARNUNG",
            Status::Fail => "FEHLER",
            Status::Info => "INFO",
        }
    }
}

struct Item {
    status: Status,
    name: String,
    result: String,
}

struct Section {
    title: &'static str,
    items: Vec<Item>,
}

impl Section {
    fn new(title: &'static str) -> Section {
        println!("\n== {title} ==");
        Section { title, items: Vec::new() }
    }

    fn add(&mut self, status: Status, name: impl Into<String>, result: impl Into<String>) {
        let (name, result) = (name.into(), result.into());
        println!("[{:<7}] {name}: {result}", status.label());
        self.items.push(Item { status, name, result });
    }
}

struct Opts {
    quick: bool,
    soak_s: f64,
    tracks: usize,
    midi_listen_s: f64,
    device: bool,
    network: bool,
    out: Option<PathBuf>,
    open: bool,
    pause: bool,
}

fn main() -> ExitCode {
    install_crash_handler("rbx-selftest");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let quick = args.iter().any(|a| a == "--quick");
    let val = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let o = Opts {
        quick,
        soak_s: val("--soak").and_then(|v| v.parse().ok()).unwrap_or(if quick { 10.0 } else { 60.0 }),
        tracks: val("--tracks").and_then(|v| v.parse().ok()).unwrap_or(if quick { 5 } else { 20 }),
        midi_listen_s: val("--midi-listen").and_then(|v| v.parse().ok()).unwrap_or(0.0),
        device: !args.iter().any(|a| a == "--no-device"),
        network: !args.iter().any(|a| a == "--no-network"),
        out: val("--out").map(PathBuf::from),
        open: args.iter().any(|a| a == "--open"),
        pause: args.iter().any(|a| a == "--pause"),
    };
    println!("rekordbox-rs Selbsttest {} – {}", env!("CARGO_PKG_VERSION"), timestamp());
    let t0 = Instant::now();
    let mut sections = vec![system(), quality()];
    if o.device {
        sections.push(devices(&o));
    }
    sections.push(performance());
    sections.push(soak(&o));
    sections.push(library(&o));
    sections.push(midi(&o));
    if o.network {
        sections.push(network(&o));
    }
    let report = render_report(&sections, t0.elapsed());
    let dir = o.out.clone().or_else(|| data_dir().map(|d| d.join("selftest"))).unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    let name = format!("selftest-{}.md", timestamp().replace([' ', ':'], "-"));
    let path = dir.join(&name);
    let _ = std::fs::write(&path, &report);
    let _ = std::fs::write(dir.join("latest.md"), &report);
    let html = dir.join("latest.html");
    let _ = std::fs::write(&html, render_html(&sections, t0.elapsed()));
    let count = |s: Status| sections.iter().flat_map(|x| &x.items).filter(|i| i.status == s).count();
    println!(
        "\nErgebnis: {} OK, {} Warnungen, {} Fehler ({:.0} s)\nBericht: {}",
        count(Status::Ok),
        count(Status::Warn),
        count(Status::Fail),
        t0.elapsed().as_secs_f64(),
        path.display()
    );
    println!("HTML: {}", html.display());
    if o.open {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &html.to_string_lossy()]).spawn();
    }
    if o.pause {
        println!("\nEnter drücken zum Schließen …");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }
    if count(Status::Fail) > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn render_report(sections: &[Section], took: Duration) -> String {
    let mut r = format!("# rekordbox-rs Selbsttest\n\nVersion {}, {} UTC, Dauer {:.0} s\n\n", env!("CARGO_PKG_VERSION"), timestamp(), took.as_secs_f64());
    let count = |s: Status| sections.iter().flat_map(|x| &x.items).filter(|i| i.status == s).count();
    r.push_str(&format!("**{} OK, {} Warnungen, {} Fehler**\n\n", count(Status::Ok), count(Status::Warn), count(Status::Fail)));
    for s in sections {
        r.push_str(&format!("## {}\n\n| Status | Prüfung | Ergebnis |\n|---|---|---|\n", s.title));
        for i in &s.items {
            r.push_str(&format!("| {} | {} | {} |\n", i.status.label(), i.name.replace('|', "/"), i.result.replace('|', "/").replace('\n', " ")));
        }
        r.push('\n');
    }
    r
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn render_html(sections: &[Section], took: Duration) -> String {
    let count = |s: Status| sections.iter().flat_map(|x| &x.items).filter(|i| i.status == s).count();
    let mut r = String::from(
        "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>rekordbox-rs Selbsttest</title><style>\
body{font-family:Segoe UI,sans-serif;background:#16181c;color:#ddd;margin:24px}h1{color:#39a0ff}\
table{border-collapse:collapse;width:100%;margin-bottom:24px}td,th{border-bottom:1px solid #333;padding:4px 8px;text-align:left;vertical-align:top}\
.OK{color:#5c5}.WARNUNG{color:#fb3}.FEHLER{color:#f55;font-weight:bold}.INFO{color:#999}</style></head><body>",
    );
    r.push_str(&format!(
        "<h1>rekordbox-rs Selbsttest</h1><p>Version {}, {} UTC, Dauer {:.0} s</p><p><b>{} OK, {} Warnungen, {} Fehler</b></p>",
        env!("CARGO_PKG_VERSION"),
        timestamp(),
        took.as_secs_f64(),
        count(Status::Ok),
        count(Status::Warn),
        count(Status::Fail)
    ));
    for s in sections {
        r.push_str(&format!("<h2>{}</h2><table><tr><th>Status</th><th>Prüfung</th><th>Ergebnis</th></tr>", esc(s.title)));
        for i in &s.items {
            let l = i.status.label();
            r.push_str(&format!("<tr><td class=\"{l}\">{l}</td><td>{}</td><td>{}</td></tr>", esc(&i.name), esc(&i.result)));
        }
        r.push_str("</table>");
    }
    r.push_str("</body></html>");
    r
}

// ---------------------------------------------------------------------------------------------

fn system() -> Section {
    let mut s = Section::new("System");
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    s.add(Status::Info, "Prozessorkerne", cores.to_string());
    s.add(Status::Info, "Programm", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default());
    match data_dir() {
        Some(d) => {
            let probe = d.join("selftest").join(".write-test");
            let ok = std::fs::create_dir_all(probe.parent().unwrap()).and_then(|_| std::fs::write(&probe, b"x")).is_ok();
            let _ = std::fs::remove_file(&probe);
            s.add(if ok { Status::Ok } else { Status::Fail }, "Datenordner beschreibbar", d.display().to_string());
        }
        None => s.add(Status::Fail, "Datenordner", "APPDATA nicht gesetzt"),
    }
    let crash = data_dir().map(|d| d.join("logs"));
    let crashes: Vec<String> = crash
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("crash-")).collect())
        .unwrap_or_default();
    if crashes.is_empty() {
        s.add(Status::Ok, "Absturzprotokolle", "keine");
    } else {
        s.add(Status::Warn, "Absturzprotokolle vorhanden", format!("{} (in %APPDATA%\\rekordbox-rs\\logs – bitte schicken)", crashes.join(", ")));
    }
    s
}

fn quality() -> Section {
    let mut s = Section::new("Audio-Qualität (offline gerendert, Hörtest-Ersatz)");
    for c in rbx_engine::quality::run_all() {
        let value = if c.unit.is_empty() { format!("{:.4}", c.value) } else { format!("{:.3} {}", c.value, c.unit) };
        s.add(if c.passed { Status::Ok } else { Status::Fail }, format!("{}: {}", c.group, c.name), format!("{value} – {}", c.detail));
    }
    s
}

/// Settings of the UI (device, buffer, MIDI mapping), read without the UI crate.
fn ui_setting(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(data_dir()?.join("settings.txt")).ok()?;
    text.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::trim).filter(|v| !v.is_empty()).map(String::from))
}

fn devices(o: &Opts) -> Section {
    let mut s = Section::new("Audiogeräte (nur Stille, nichts hörbar)");
    let list = rbx_engine::output::list_devices();
    s.add(if list.is_empty() { Status::Fail } else { Status::Ok }, "Ausgabegeräte", list.join(", "));
    let configured = ui_setting("device");
    let buffer: Option<u32> = ui_setting("buffer").and_then(|b| b.parse().ok());
    let mut targets: Vec<(Option<String>, Option<u32>, String)> = vec![(None, None, "Standardgerät".into())];
    if configured.is_some() || buffer.is_some() {
        targets.push((configured.clone(), buffer, format!("Eingestellt: {} / Puffer {}", configured.as_deref().unwrap_or("Standard"), buffer.map_or("Standard".into(), |b| b.to_string()))));
    }
    if !o.quick {
        targets.push((configured.clone(), Some(256), "Puffer 256 Frames".into()));
    }
    let secs = if o.quick { 2.0 } else { 5.0 };
    for (dev, buf, label) in targets {
        match Output::open_with(dev.as_deref(), buf) {
            Ok(out) => {
                // nothing is loaded: the mixer renders silence
                std::thread::sleep(Duration::from_secs_f64(secs));
                let st = out.stats();
                let ms = f64::from(st.frames) / f64::from(out.sample_rate.max(1)) * 1000.0;
                let status = if st.callbacks == 0 {
                    Status::Fail
                } else if st.late > 0 || st.overloads > 0 {
                    Status::Warn
                } else {
                    Status::Ok
                };
                s.add(
                    status,
                    label,
                    format!(
                        "{} – {} Hz, {} Kanäle{}, Puffer {} Frames ({ms:.1} ms), {} Callbacks in {secs} s, Last {:.1} % (max {:.1} %), Aussetzer {}, Überlastungen {}",
                        out.device_name,
                        out.sample_rate,
                        out.channels,
                        if out.channels >= 4 { " (Kopfhörer-Cue auf 3/4)" } else { "" },
                        st.frames,
                        st.callbacks,
                        st.load * 100.0,
                        st.max_load * 100.0,
                        st.late,
                        st.overloads
                    ),
                );
            }
            Err(e) => s.add(Status::Fail, label, e),
        }
    }
    s
}

fn busy_mixer() -> Mixer {
    let mut m = Mixer::new(44100);
    let track = |bpm: f64| {
        // a rich test signal: clicks plus chords
        let mut p = Pcm::click_track(44100, bpm, 90.0);
        for (i, f) in p.frames.iter_mut().enumerate() {
            let t = i as f64 / 44100.0;
            let v = ((t * 220.0 * std::f64::consts::TAU).sin() + (t * 277.2 * std::f64::consts::TAU).sin() + (t * 329.6 * std::f64::consts::TAU).sin()) as f32 * 0.1;
            f[0] += v;
            f[1] += v;
        }
        Arc::new(p)
    };
    m.apply(Command::Load { deck: 0, pcm: track(128.0), grid: Some(Grid { bpm: 128.0, first_beat_s: 0.0 }) });
    m.apply(Command::Load { deck: 1, pcm: track(124.0), grid: Some(Grid { bpm: 124.0, first_beat_s: 0.0 }) });
    m
}

fn performance() -> Section {
    let mut s = Section::new("Leistung");
    // worst case: both decks with key lock, EQ, colour FX and beat FX
    let mut m = busy_mixer();
    for d in 0..2 {
        m.apply(Command::Play(d));
        m.apply(Command::TempoFader(d, 0.4));
        m.apply(Command::MasterTempo(d, true));
        m.apply(Command::Eq(d, 0, 0.2));
        m.apply(Command::ColorKnob(d, 0.3));
    }
    m.apply(Command::ColorFxSelect(Some(ColorFx::Filter)));
    m.apply(Command::BeatFxSelect(BeatFxKind::Reverb));
    m.apply(Command::BeatFxOn(true));
    let secs = 30.0;
    let t = Instant::now();
    let mut buf = vec![[0.0f32; 2]; 512];
    let mut cue = vec![[0.0f32; 2]; 512];
    for _ in 0..(secs * 44100.0 / 512.0) as usize {
        m.render_with_cue(&mut buf, &mut cue);
    }
    let factor = secs / t.elapsed().as_secs_f64();
    let status = if factor < 2.0 {
        Status::Fail
    } else if factor < 10.0 {
        Status::Warn
    } else {
        Status::Ok
    };
    s.add(status, "Mixer, Volllast (2 Decks, Key Lock, EQ, Color FX, Reverb)", format!("{factor:.0}× schneller als Echtzeit ({:.1} % eines Kerns)", 100.0 / factor));
    // analysis speed on a synthetic 3-minute track
    let pcm = Pcm::click_track(44100, 126.0, 180.0);
    let t = Instant::now();
    let r = rbx_engine::analyze::analyze_pcm(Path::new("selftest.wav"), &pcm);
    let took = t.elapsed().as_secs_f64();
    s.add(
        if took < 5.0 { Status::Ok } else { Status::Warn },
        "Analyse eines 3-Minuten-Tracks (ohne Dekodieren)",
        format!("{took:.2} s, {}", r.grid.map_or("kein Grid".into(), |g| format!("{:.2} BPM", g.bpm))),
    );
    s
}

/// Pseudo-random operation of everything (deterministic).
struct Monkey(u64);

impl Monkey {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn f(&mut self) -> f64 {
        (self.next() % 10_000) as f64 / 10_000.0
    }

    fn command(&mut self) -> Command {
        let d = (self.next() % 2) as usize;
        match self.next() % 30 {
            0 | 1 => Command::TogglePlay(d),
            2 => Command::CuePress(d),
            3 => Command::CueRelease(d),
            4 => Command::HotCuePress(d, (self.next() % 8) as usize),
            5 => Command::HotCueRelease(d, (self.next() % 8) as usize),
            6 => Command::BeatLoop(d, [0.25, 1.0, 4.0, 16.0][(self.next() % 4) as usize]),
            7 => Command::ReloopExit(d),
            8 => Command::BeatJump(d, [-4.0, -1.0, 1.0, 8.0][(self.next() % 4) as usize]),
            9 => Command::TempoFader(d, self.f() * 2.0 - 1.0),
            10 => Command::Eq(d, (self.next() % 3) as usize, self.f() as f32),
            11 => Command::Crossfader(self.f() as f32 * 2.0 - 1.0),
            12 => Command::Sync(d),
            13 => Command::BeatFxOn(self.next() % 2 == 0),
            14 => Command::BeatFxSelect(BeatFxKind::ALL[(self.next() % 14) as usize]),
            15 => Command::ColorFxSelect(if self.next() % 7 == 0 { None } else { Some(ColorFx::ALL[(self.next() % 6) as usize]) }),
            16 => Command::ColorKnob(d, self.f() as f32),
            17 => Command::Reverse(d, self.next() % 2 == 0),
            18 => Command::Slip(d, self.next() % 2 == 0),
            19 => Command::MasterTempo(d, self.next() % 2 == 0),
            20 => Command::Key(d, (self.next() % 13) as i32 - 6),
            21 => Command::SeekFraction(d, self.f()),
            22 => Command::JogTouch(d, self.next() % 2 == 0),
            23 => Command::Jog(d, (self.next() % 200) as i32 - 100),
            24 => Command::Volume(d, self.f() as f32),
            25 => Command::LoopScale(d, if self.next() % 2 == 0 { 0.5 } else { 2.0 }),
            26 => Command::PadFx(d, if self.next() % 2 == 0 { Some(((self.next() % 2) as usize, (self.next() % 16) as usize)) } else { None }),
            27 => Command::SamplerStart((self.next() % 16) as usize),
            28 => Command::BeatFxBeat(if self.next() % 2 == 0 { 1 } else { -1 }),
            _ => Command::Brake(d, self.f() * 2.0),
        }
    }
}

fn soak(o: &Opts) -> Section {
    let mut s = Section::new("Dauerlauf mit zufälliger Bedienung");
    // offline: much more audio than real time allows
    let minutes = if o.quick { 5.0 } else { 30.0 };
    let result = std::panic::catch_unwind(|| {
        let mut m = busy_mixer();
        for (slot, p) in rbx_engine::sampler::builtin_sounds(44100).into_iter().enumerate() {
            m.apply(Command::SamplerLoad(slot, Arc::new(p)));
        }
        let mut monkey = Monkey(0x9E37_79B9_7F4A_7C15);
        let mut buf = vec![[0.0f32; 2]; 441];
        let mut cue = vec![[0.0f32; 2]; 441];
        let (mut bad, mut peak, mut commands) = (0usize, 0.0f32, 0usize);
        for block in 0..(minutes * 60.0 * 100.0) as usize {
            if block % 5 == 0 {
                m.apply(monkey.command());
                commands += 1;
            }
            m.render_with_cue(&mut buf, &mut cue);
            for f in buf.iter().chain(&cue) {
                if !f[0].is_finite() || !f[1].is_finite() {
                    bad += 1;
                } else {
                    peak = peak.max(f[0].abs()).max(f[1].abs());
                }
            }
        }
        (bad, peak, commands)
    });
    match result {
        Ok((bad, peak, commands)) => {
            let status = if bad > 0 || peak > 1.0 { Status::Fail } else { Status::Ok };
            s.add(status, format!("{minutes} Minuten Audio offline, Befehl alle 50 ms"), format!("{commands} Befehle, ungültige Samples {bad}, Spitzenpegel {peak:.3} (≤ 1)"));
        }
        Err(e) => {
            let msg = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            s.add(Status::Fail, "Offline-Dauerlauf", format!("Absturz: {msg}"));
        }
    }
    // real time: the audio thread keeps running while commands come from another thread
    let out = Output::silent(44100);
    let tx = out.sender();
    let m = busy_mixer();
    for d in 0..2 {
        tx.send(Command::Load { deck: d, pcm: m.decks[d].pcm_arc().unwrap(), grid: m.decks[d].grid }).ok();
    }
    let mut monkey = Monkey(0x1234_5678_9ABC_DEF1);
    let t = Instant::now();
    let mut last = 0;
    let mut stalled = false;
    while t.elapsed().as_secs_f64() < o.soak_s {
        tx.send(monkey.command()).ok();
        std::thread::sleep(Duration::from_millis(20));
        if t.elapsed().as_secs() > last {
            last = t.elapsed().as_secs();
            let before = out.stats().callbacks;
            std::thread::sleep(Duration::from_millis(50));
            if out.stats().callbacks == before {
                stalled = true;
                break;
            }
        }
    }
    let st = out.stats();
    let state = out.state();
    let sane = state.decks.iter().all(|d| d.position_s.is_finite() && d.position_s >= 0.0) && state.peak.is_finite();
    let status = if stalled || !sane {
        Status::Fail
    } else if st.overloads > 0 {
        Status::Warn
    } else {
        Status::Ok
    };
    s.add(
        status,
        format!("{:.0} s Echtzeit (Audio-Thread + Bedien-Thread)", o.soak_s),
        format!(
            "{}; Last max {:.1} %, Überlastungen {}",
            if stalled { "Audio-Thread steht (abgestürzt?)" } else if sane { "läuft, Zustand gültig" } else { "ungültiger Zustand" },
            st.max_load * 100.0,
            st.overloads
        ),
    );
    s
}

fn library(o: &Opts) -> Section {
    use rekordbox_formats::library::{default_anlz_root, default_music_root, Library, Source};
    let mut s = Section::new("Bibliothek und Analyse-Genauigkeit");
    let mut roots: Vec<PathBuf> = default_anlz_root().into_iter().collect();
    roots.extend(rbx_engine::analyze::default_output_root().filter(|p| p.is_dir()));
    let music: Vec<PathBuf> = ui_setting("music").map(PathBuf::from).or_else(default_music_root).into_iter().collect();
    if roots.is_empty() || music.is_empty() {
        s.add(Status::Info, "Bibliothek", "kein Analyse- oder Musikordner gefunden");
        return s;
    }
    let t = Instant::now();
    let lib = Library::build_multi(&roots, &music);
    let r = lib.report;
    s.add(Status::Info, "Gelesen", format!("{} Analysen, {} mit Audiodatei, {} mit Beatgrid ({:.1} s)", r.tracks, r.matched, r.with_grid, t.elapsed().as_secs_f64()));
    let files = rbx_engine::session::audio_files(&music[0]);
    let missing = files.iter().filter(|f| lib.by_path(f).is_none()).count();
    s.add(Status::Info, "Audiodateien ohne Analyse", format!("{missing} von {} (in rbx-ui: „Fehlende analysieren“)", files.len()));
    // own analysis against rekordbox on tracks of this machine (in memory, nothing written)
    let candidates: Vec<_> = lib.playable().filter(|t| t.source == Source::Rekordbox && t.bpm().is_some() && !t.variable_tempo()).collect();
    if candidates.is_empty() {
        s.add(Status::Info, "Analyse-Vergleich", "keine rekordbox-Analysen mit konstantem Tempo");
        return s;
    }
    let step = (candidates.len() / o.tracks.max(1)).max(1);
    let (mut n, mut bpm_ok, mut phase_ok, mut decode_fail, mut secs) = (0, 0, 0, 0, 0.0);
    for t in candidates.iter().step_by(step).take(o.tracks) {
        let Some(p) = &t.path else { continue };
        let started = Instant::now();
        let Ok(pcm) = rbx_engine::pcm::decode_file(p) else {
            decode_fail += 1;
            continue;
        };
        let res = rbx_engine::analyze::analyze_pcm(p, &pcm);
        secs += started.elapsed().as_secs_f64();
        n += 1;
        let (Some(g), Some(theirs), Some(first)) = (res.grid, t.bpm(), t.first_downbeat_s()) else { continue };
        if (g.bpm - theirs).abs() <= 0.02 {
            bpm_ok += 1;
            let beat = 60.0 / theirs;
            let mut d = (g.first_downbeat_s - first).rem_euclid(beat);
            if d > beat / 2.0 {
                d -= beat;
            }
            if d.abs() <= 0.010 {
                phase_ok += 1;
            }
        }
    }
    if decode_fail > 0 {
        s.add(Status::Warn, "Dekodieren", format!("{decode_fail} Dateien ließen sich nicht dekodieren"));
    }
    if n > 0 {
        let share = bpm_ok as f64 / n as f64;
        s.add(
            if share >= 0.85 { Status::Ok } else { Status::Warn },
            format!("Eigene Analyse vs. rekordbox ({n} Tracks)"),
            format!("BPM gleich: {bpm_ok}/{n}, Beat-Phase ±10 ms: {phase_ok}/{bpm_ok}; {:.1} s je Track inkl. Dekodieren", secs / n as f64),
        );
    }
    s
}

fn midi(o: &Opts) -> Section {
    let mut s = Section::new("MIDI / Controller");
    #[cfg(windows)]
    {
        use rekordbox_formats::midi_device::{list_inputs, list_outputs, InputPort, OutputPort};
        let ins = list_inputs();
        let outs = list_outputs();
        s.add(Status::Info, "MIDI-Eingänge", if ins.is_empty() { "keine".into() } else { ins.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(", ") });
        s.add(Status::Info, "MIDI-Ausgänge", if outs.is_empty() { "keine".into() } else { outs.iter().map(|d| d.name.clone()).collect::<Vec<_>>().join(", ") });
        for d in &ins {
            match InputPort::open(d.index) {
                Ok(_) => s.add(Status::Ok, format!("Eingang öffnen: {}", d.name), "frei"),
                Err(e) => s.add(Status::Warn, format!("Eingang öffnen: {}", d.name), format!("{e} (rekordbox oder ein anderes Programm hat ihn offen?)")),
            }
        }
        for d in &outs {
            match OutputPort::open(d.index) {
                Ok(_) => s.add(Status::Ok, format!("Ausgang öffnen: {}", d.name), "frei"),
                Err(e) => s.add(Status::Warn, format!("Ausgang öffnen: {}", d.name), e),
            }
        }
        // mappings: the one from the settings, plus rekordbox's folder next to an installation
        let mut maps: Vec<PathBuf> = ui_setting("midi").map(PathBuf::from).into_iter().collect();
        if let Ok(it) = std::fs::read_dir(r"C:\Program Files\rekordbox") {
            for e in it.flatten() {
                if let Ok(m) = std::fs::read_dir(e.path().join("MidiMappings")) {
                    maps.extend(m.flatten().map(|f| f.path()).filter(|p| p.to_string_lossy().ends_with(".midi.csv")));
                }
            }
        }
        // shown: the configured mapping, the XDJ-RX3 one, and any whose device is connected
        let configured = ui_setting("midi").map(PathBuf::from);
        let mut matched = false;
        for m in &maps {
            let Ok(mapping) = rbx_engine::session::load_mapping(m) else { continue };
            let dev = mapping.device.to_lowercase();
            let connected = ins.iter().any(|d| d.name.to_lowercase().contains(&dev));
            let wanted = configured.as_ref() == Some(m) || (configured.is_none() && dev == "xdj-rx3");
            if !connected && !wanted {
                continue;
            }
            matched |= connected;
            let c = rbx_engine::controller::Controller::new(mapping);
            let (yes, no) = c.coverage();
            s.add(
                if no.is_empty() { Status::Ok } else { Status::Warn },
                format!("Mapping {}", m.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                format!("{}: {} Funktionen umgesetzt, {} nicht{}", c.mapping().device, yes.len(), no.len(), if connected { " – Gerät angeschlossen" } else { "" }),
            );
        }
        if !matched {
            s.add(Status::Info, "Controller", "kein Gerät mit passendem Mapping angeschlossen");
        }
        if o.midi_listen_s > 0.0 {
            for d in &ins {
                let Ok((port, rx)) = InputPort::open(d.index) else { continue };
                println!("Bewege jetzt {} Sekunden lang Regler und Tasten an „{}“ …", o.midi_listen_s, d.name);
                let t = Instant::now();
                let mut seen = std::collections::BTreeSet::new();
                let mut count = 0usize;
                while t.elapsed().as_secs_f64() < o.midi_listen_s {
                    if let Ok((packed, _)) = rx.recv_timeout(Duration::from_millis(100)) {
                        let b = packed.to_le_bytes();
                        if b[0] & 0xF0 != 0xF0 {
                            count += 1;
                            seen.insert((b[0], b[1]));
                        }
                    }
                }
                drop(port);
                s.add(if count > 0 { Status::Ok } else { Status::Warn }, format!("Eingaben von {}", d.name), format!("{count} Nachrichten, {} verschiedene Bedienelemente", seen.len()));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = o;
        s.add(Status::Info, "MIDI", "nur unter Windows geprüft");
    }
    s
}

fn network(o: &Opts) -> Section {
    use rekordbox_formats::djlink_net::{Link, LinkConfig};
    use rekordbox_formats::djlink_participant::Config;
    use rekordbox_formats::netif;
    let mut s = Section::new("Netzwerk / Pro DJ Link");
    let list = netif::list();
    for i in &list {
        s.add(Status::Info, format!("Adapter {}", i.name), format!("{} – {} (Broadcast {})", i.description, i.ip, i.broadcast()));
    }
    let wanted = ui_setting("djlink_adapter");
    let Some(i) = netif::choose(&list, wanted.as_deref()) else {
        s.add(Status::Warn, "DJ Link", "kein passender Netzwerkadapter");
        return s;
    };
    let cfg = LinkConfig::new(Config::new("rbx-selftest", i.mac, i.ip), i.broadcast());
    match Link::start(cfg) {
        Ok(mut link) => {
            let secs = if o.quick { 5.0 } else { 12.0 };
            std::thread::sleep(Duration::from_secs_f64(secs));
            let st = link.state();
            let devices: Vec<String> = st.devices.iter().map(|d| format!("#{} {} ({})", d.number, d.name, d.ip)).collect();
            s.add(Status::Ok, format!("DJ Link auf {} ({})", i.name, i.ip), format!("{secs} s beigetreten, eigene Nummer {:?}", st.number));
            s.add(
                Status::Info,
                "Geräte im Netz",
                if devices.is_empty() { "keine (kein CDJ/XDJ/Mixer angeschlossen oder anderes Netz)".into() } else { devices.join(", ") },
            );
            if !st.beats.is_empty() {
                s.add(Status::Ok, "Beat-Pakete empfangen", st.beats.iter().map(|(n, b)| format!("#{n}: {:.2} BPM", f64::from(b.effective_bpm100()) / 100.0)).collect::<Vec<_>>().join(", "));
            }
            s.add(if st.errors > 0 { Status::Warn } else { Status::Ok }, "Nicht dekodierbare Pakete", st.errors.to_string());
            for e in st.events.iter().rev().take(5) {
                s.add(Status::Info, "Ereignis", format!("{e:?}"));
            }
            link.stop();
        }
        Err(e) => s.add(Status::Warn, "DJ Link beitreten", format!("{e} (läuft rekordbox? Es belegt dieselben Ports)")),
    }
    s
}
