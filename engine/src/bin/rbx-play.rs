//! `rbx-play` – try the audio engine.
//!
//! ```text
//! rbx-play devices                                    list output devices
//! rbx-play play <file1> [file2] [--device NAME]       live playback, commands on stdin (see control.rs)
//! rbx-play render <out.wav> <seconds> <file1|tone|click:BPM> [file2|…] [--script "0:play 1; …"]
//! rbx-play tone [seconds]                             440 Hz test tone on the default device
//! rbx-play midi-coverage <mapping.csv>                which mapping functions the engine handles
//! ```
//! `play` options: `--device NAME` (audio output), `--midi MAPPING.csv` (controller; MIDI in/out are
//! found by the mapping's device name, override with `--midi-in NAME` / `--midi-out NAME`). With a
//! mapping loaded, `midi B0 22 41` on stdin injects a message as if the controller had sent it.
//! `--samples DIR` fills the sampler from a folder (else built-in sounds), `--browse DIR` sets the
//! browser root for BROWSE/LOAD (default: the user's Music folder). `--djlink [ADAPTER]` joins the
//! Pro DJ Link network on that adapter (name part or IP; default: first private / link-local one),
//! The rekordbox library (analysis folder + music folder, `--anlz DIR`, `--music DIR`, `--no-library`)
//! is read in the background; loaded files get beat grid, beat positions and cues from it; `lib words`
//! searches it, `lload <deck> <n>` loads a search result, the browser shows `[Library]`.
//! sends beat packets of the playing deck; `link` on stdin shows the network, `linksync 1 on [N]` lets
//! deck 1 follow tempo and beat of DJ Link device N (default: the first one sending beats).
//! Sources: an audio file path, `tone` (440 Hz sine) or `click:BPM` (click track) – the latter two
//! allow testing without music files.

use std::io::BufRead;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rbx_engine::control;
use rbx_engine::controller::Controller;
use rbx_engine::deck::Grid;
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::output::{list_devices, Output};
use rbx_engine::pcm::{decode_file, write_wav, Pcm};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let r = match a.as_slice() {
        ["devices"] => {
            for d in list_devices() {
                println!("{d}");
            }
            Ok(())
        }
        ["play", rest @ ..] => play(rest),
        ["render", out, secs, rest @ ..] => render(Path::new(out), secs, rest),
        ["midi-coverage", m] => coverage(Path::new(m)),
        ["tone", rest @ ..] => tone(rest.first().and_then(|s| s.parse().ok()).unwrap_or(2.0)),
        _ => Err("usage: rbx-play devices | play <file1> [file2] [--device NAME] | \
                  render <out.wav> <seconds> <src1> [src2] [--script S] | tone [seconds]"
            .into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Split positional sources from `--flag value` options.
fn split_opts<'a>(rest: &[&'a str]) -> (Vec<&'a str>, Vec<(&'a str, &'a str)>) {
    let (mut pos, mut opts) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < rest.len() {
        if let Some(flag) = rest[i].strip_prefix("--") {
            // a flag without value (end of line or followed by another flag) gets ""
            match rest.get(i + 1) {
                Some(v) if !v.starts_with("--") => {
                    opts.push((flag, *v));
                    i += 2;
                }
                _ => {
                    opts.push((flag, ""));
                    i += 1;
                }
            }
        } else {
            pos.push(rest[i]);
            i += 1;
        }
    }
    (pos, opts)
}

/// Load a source: file, `tone` or `click:BPM` (the click track also gets a grid).
fn source(spec: &str, sample_rate: u32) -> Result<(Arc<Pcm>, Option<Grid>), String> {
    if spec == "tone" {
        return Ok((Arc::new(Pcm::sine(sample_rate, 440.0, 60.0, 0.3)), None));
    }
    if let Some(bpm) = spec.strip_prefix("click:") {
        let bpm: f64 = bpm.parse().map_err(|_| format!("{spec}: invalid BPM"))?;
        return Ok((Arc::new(Pcm::click_track(sample_rate, bpm, 120.0)), Some(Grid { bpm, first_beat_s: 0.0 })));
    }
    let t = std::time::Instant::now();
    let pcm = decode_file(Path::new(spec)).map_err(|e| format!("{spec}: {e}"))?;
    eprintln!("decoded {spec}: {:.1} s at {} Hz in {:.2} s", pcm.duration_s(), pcm.sample_rate, t.elapsed().as_secs_f64());
    Ok((Arc::new(pcm), None))
}

fn render(out: &Path, secs: &str, rest: &[&str]) -> Result<(), String> {
    let seconds: f64 = secs.parse().map_err(|_| "invalid seconds")?;
    let (sources, opts) = split_opts(rest);
    if sources.is_empty() || sources.len() > 2 {
        return Err("give one or two sources".into());
    }
    let mut mixer = Mixer::new(44100);
    for (d, s) in sources.iter().enumerate() {
        let (pcm, grid) = source(s, 44100)?;
        mixer.apply(Command::Load { deck: d, pcm, grid });
    }
    let script = opts.iter().find(|(k, _)| *k == "script").map(|(_, v)| *v).unwrap_or("0:play 1");
    let pcm = rbx_engine::control::render_script(&mut mixer, script, seconds)?;
    write_wav(out, &pcm).map_err(|e| e.to_string())?;
    println!("wrote {} ({seconds} s, peak {:.2})", out.display(), mixer.state().peak);
    Ok(())
}

fn print_state(o: &Output) {
    let s = o.state();
    for (i, d) in s.decks.iter().enumerate() {
        println!(
            "deck {}: {} {:>7.2}/{:.2} s speed {:.3} tempo {:+.2} % bpm {} cue {:.2} {}",
            i + 1,
            if !d.loaded { "empty " } else if d.playing { "PLAY  " } else { "pause " },
            d.position_s,
            d.duration_s,
            d.speed,
            (d.tempo - 1.0) * 100.0,
            d.bpm.map_or("-".into(), |b| format!("{b:.2}")),
            d.cue_s,
            if d.looping { "LOOP" } else { "" }
        );
    }
    println!("crossfader {:+.2} peak {:.2}", s.crossfader, s.peak);
}

fn play(rest: &[&str]) -> Result<(), String> {
    let (sources, opts) = split_opts(rest);
    let device = opts.iter().find(|(k, _)| *k == "device").map(|(_, v)| *v);
    let out = Output::open(device)?;
    println!("output: {} ({} Hz, {} ch)", out.device_name, out.sample_rate, out.channels);
    let loader = Loader::new(out.sender(), out.sample_rate);
    if !opts.iter().any(|(k, _)| *k == "no-library") {
        let opt = |k: &str| opts.iter().find(|(o, _)| *o == k).map(|(_, v)| std::path::PathBuf::from(v));
        loader.start_library(opt("anlz"), opt("music"));
    }
    for (d, s) in sources.iter().take(2).enumerate() {
        loader.load(d, s);
    }
    fill_sampler(&out, opts.iter().find(|(k, _)| *k == "samples").map(|(_, v)| Path::new(*v)));
    let ctl = match opts.iter().find(|(k, _)| *k == "midi") {
        Some((_, m)) => Some(start_controller(m, &opts, &out, &loader)?),
        None => None,
    };
    let link = match opts.iter().find(|(k, _)| *k == "djlink") {
        Some((_, iface)) => Some(start_link(if iface.is_empty() || *iface == "auto" { None } else { Some(*iface) }, &out)?),
        None => None,
    };
    if let Some(c) = &ctl {
        let mut c = c.controller.lock().unwrap();
        for (d, s) in sources.iter().take(2).enumerate() {
            if Path::new(s).is_file() {
                c.set_loaded_path(d, Path::new(s));
            }
        }
    }
    println!("commands: play 1, cue 1, hot 1 1, loop 1 4, tempo 1 0.2, sync 2, xf 0, load 2 <file>, lib <words>, lload 1 <n>, midi 90 00 7F, state, quit");
    let mut results: Vec<std::path::PathBuf> = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        match line {
            "" => continue,
            "quit" | "q" | "exit" => break,
            "state" | "s" => {
                print_state(&out);
                for d in 0..2 {
                    if let Some(t) = loader.title(d) {
                        println!("deck {}: {t}", d + 1);
                    }
                }
            }
            l if l == "lib" || l.starts_with("lib ") => match loader.library() {
                Some(guard) => {
                    let lib = guard.as_ref().unwrap();
                    let found = lib.search(l.get(4..).unwrap_or(""));
                    results = found.iter().filter_map(|t| t.path.clone()).collect();
                    println!("{} of {} tracks", found.len(), lib.tracks.len());
                    for (i, t) in found.iter().filter(|t| t.path.is_some()).take(30).enumerate() {
                        println!(
                            "  {:>3}  {} - {}  {}{}",
                            i + 1,
                            t.artist.as_deref().unwrap_or("?"),
                            t.title,
                            t.bpm().map_or("-".into(), |b| format!("{b:.2} BPM")),
                            if t.variable_tempo() { " (variable)" } else { "" }
                        );
                    }
                }
                None => println!("library not ready (or disabled)"),
            },
            l if l.starts_with("lload ") => {
                let w: Vec<&str> = l.split_whitespace().collect();
                let d = w.get(1).and_then(|d| d.parse::<usize>().ok()).filter(|d| (1..=2).contains(d));
                let n = w.get(2).and_then(|n| n.parse::<usize>().ok()).filter(|&n| n >= 1 && n <= results.len());
                match (d, n) {
                    (Some(d), Some(n)) => {
                        let p = results[n - 1].clone();
                        loader.load(d - 1, &p.to_string_lossy());
                        if let Some(c) = &ctl {
                            c.controller.lock().unwrap().set_loaded_path(d - 1, &p);
                        }
                    }
                    _ => eprintln!("lload <deck> <number from the last lib search>"),
                }
            }
            "link" => match &link {
                Some(l) => print_link(&l.link.lock().unwrap()),
                None => eprintln!("start with --djlink [adapter]"),
            },
            l if l.starts_with("linksync ") => match &link {
                Some(h) => {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    let deck = w.get(1).and_then(|d| d.parse::<usize>().ok()).filter(|d| (1..=2).contains(d));
                    match (deck, w.get(2).copied()) {
                        (Some(d), Some("on")) => {
                            let dev = w.get(3).and_then(|n| n.parse::<u8>().ok());
                            h.follow.lock().unwrap()[d - 1] = Some(dev);
                            println!("deck {d} follows DJ Link device {}", dev.map_or("(first with beats)".into(), |n| n.to_string()));
                        }
                        (Some(d), Some("off")) => h.follow.lock().unwrap()[d - 1] = None,
                        _ => eprintln!("linksync <deck> on [device] | linksync <deck> off"),
                    }
                }
                None => eprintln!("start with --djlink [adapter]"),
            },
            "browse" => match &ctl {
                Some(c) => match c.controller.lock().unwrap().browser() {
                    Some(b) => println!("{}", b.describe()),
                    None => println!("no browser"),
                },
                None => eprintln!("start with --midi <mapping.csv>"),
            },
            l if l.starts_with("midi ") => match (&ctl, parse_hex(&l[5..])) {
                (Some(c), Ok(bytes)) => match rekordbox_formats::midi_input::MidiMessage::from_bytes(&bytes, 0.0) {
                    Some(msg) => {
                        let cmds = c.controller.lock().unwrap().handle(&msg);
                        println!("-> {cmds:?}");
                        for cmd in cmds {
                            out.send(cmd);
                        }
                        side_effects(&c.controller, &loader);
                    }
                    None => eprintln!("first byte must be a status byte"),
                },
                (None, _) => eprintln!("start with --midi <mapping.csv> to use the controller layer"),
                (_, Err(e)) => eprintln!("{e}"),
            },
            l if l.starts_with("load ") => {
                let mut p = l.splitn(3, ' ').skip(1);
                let d = p.next().and_then(|d| d.parse::<usize>().ok()).filter(|d| (1..=2).contains(d));
                match (d, p.next()) {
                    (Some(d), Some(f)) => {
                        let f = f.trim().trim_matches('"');
                        loader.load(d - 1, f);
                        if let Some(c) = &ctl {
                            c.controller.lock().unwrap().set_loaded_path(d - 1, Path::new(f));
                        }
                    }
                    _ => eprintln!("load <deck> <file>"),
                }
            }
            l => match control::parse(l) {
                Ok(cmd) => out.send(cmd),
                Err(e) => eprintln!("{e}"),
            },
        }
    }
    Ok(())
}

fn tone(seconds: f64) -> Result<(), String> {
    let out = Output::open(None)?;
    println!("440 Hz on {} for {seconds} s", out.device_name);
    out.send(Command::Load { deck: 0, pcm: Arc::new(Pcm::sine(out.sample_rate, 440.0, seconds + 1.0, 0.2)), grid: None });
    out.send(Command::Play(0));
    std::thread::sleep(Duration::from_secs_f64(seconds));
    Ok(())
}

/// DJ Link in the background: the link and which decks follow which device.
struct LinkHandle {
    link: Arc<std::sync::Mutex<rekordbox_formats::djlink_net::Link>>,
    /// Per deck: `Some(device)` = follow that device (`Some(None)` = first device with beats).
    follow: Arc<std::sync::Mutex<[Option<Option<u8>>; 2]>>,
}

/// Join DJ Link, send beat packets of the playing deck (the sync master, else the louder one) and
/// let decks follow external devices.
fn start_link(iface: Option<&str>, out: &Output) -> Result<LinkHandle, String> {
    use rekordbox_formats::djlink_net::{external_beat, BeatClock, Link, LinkConfig};
    use rekordbox_formats::djlink_participant::Config;
    use rekordbox_formats::netif;
    let list = netif::list();
    let i = netif::choose(&list, iface).ok_or(format!("no network adapter matching {iface:?} (see `cargo run --example netif`)"))?;
    println!("DJ Link: {} {} (broadcast {})", i.name, i.ip, i.broadcast());
    let cfg = LinkConfig::new(Config::new("rekordbox-rs", i.mac, i.ip), i.broadcast());
    let link = Arc::new(std::sync::Mutex::new(Link::start(cfg).map_err(|e| format!("{e} (is rekordbox running?)"))?));
    let follow: Arc<std::sync::Mutex<[Option<Option<u8>>; 2]>> = Arc::new(std::sync::Mutex::new([None, None]));
    let (l, state, fw, tx) = (link.clone(), out.state_handle(), follow.clone(), out.sender());
    std::thread::spawn(move || {
        let mut clock = BeatClock::default();
        let mut applied: [Option<std::time::Instant>; 2] = [None, None];
        loop {
            std::thread::sleep(Duration::from_millis(5));
            let s = *state.lock().unwrap();
            let playing: Vec<usize> = (0..2).filter(|&d| s.decks[d].playing && s.decks[d].beat.is_some()).collect();
            let deck = match s.master {
                Some(m) if playing.contains(&m) => Some(m),
                _ => playing.into_iter().max_by(|&a, &b| s.channel_peak[a].total_cmp(&s.channel_peak[b])),
            };
            let beat = deck.and_then(|d| s.decks[d].bpm.map(|bpm| (s.decks[d].beat, bpm)));
            let packet = match beat {
                Some((b, bpm)) => clock.update("rekordbox-rs", b, bpm),
                None => clock.update("rekordbox-rs", None, 120.0),
            };
            let Ok(link) = l.lock() else { return };
            if let Some(p) = packet {
                link.send_beat(&p);
            }
            // external sync: re-align on every new beat packet of the followed device
            let targets = *fw.lock().unwrap();
            let ls = link.state();
            for (d, t) in targets.iter().enumerate() {
                let Some(dev) = t else { continue };
                let Some(dev) = dev.or_else(|| ls.beats.keys().next().copied()) else { continue };
                let at = ls.beat_at.get(&dev).copied();
                if at.is_none() || at == applied[d] {
                    continue;
                }
                if let Some((bpm, phase)) = external_beat(&ls, dev) {
                    let _ = tx.send(Command::SyncExternal { deck: d, bpm, phase });
                    applied[d] = at;
                }
            }
        }
    });
    Ok(LinkHandle { link, follow })
}

fn print_link(l: &rekordbox_formats::djlink_net::Link) {
    let s = l.state();
    println!("DJ Link: number {:?}, phase {:?}, {} undecodable datagrams", s.number, s.phase, s.errors);
    for d in &s.devices {
        println!("  #{:<3} {:<20} kind {} ip {}{}", d.number, d.name, d.kind, d.ip, if d.second_deck { " (2nd deck)" } else { "" });
    }
    for (n, b) in &s.beats {
        println!("  beat  #{n}: {:.2} BPM, beat {}/4", f64::from(b.effective_bpm100()) / 100.0, b.beat_in_bar);
    }
    for (n, c) in &s.status {
        println!("  status #{n}: play state {} tempo {:?}", c.play_state, c.effective_bpm100().map(|v| f64::from(v) / 100.0));
    }
    for (n, m) in &s.mixers {
        println!("  mixer #{n}: decks {:?}", m.decks);
    }
    for e in s.events.iter().rev().take(5) {
        println!("  event: {e:?}");
    }
}

/// Fill the 16 sampler slots: audio files of `dir`, the rest with the built-in sounds.
fn fill_sampler(out: &Output, dir: Option<&Path>) {
    use rbx_engine::sampler::{builtin_sounds, load_folder, BUILTIN_NAMES};
    let files = dir.map(load_folder).unwrap_or_default();
    let builtin = builtin_sounds(out.sample_rate);
    let mut names = Vec::new();
    for (slot, b) in builtin.into_iter().enumerate() {
        match files.get(slot) {
            Some((p, pcm)) => {
                names.push(p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()));
                out.send(Command::SamplerLoad(slot, Arc::new(pcm.clone())));
            }
            None => {
                names.push(BUILTIN_NAMES[slot].to_string());
                out.send(Command::SamplerLoad(slot, Arc::new(b)));
            }
        }
    }
    println!("sampler: {}", names.join(", "));
}

/// Loads requested by the controller (decoded on a worker thread), the library for the browser once it
/// is ready, and browser display updates.
fn side_effects(c: &Arc<std::sync::Mutex<Controller>>, loader: &Loader) {
    if let Some(tracks) = loader.take_library_for_browser() {
        c.lock().unwrap().set_library(tracks);
    }
    let (loads, browser) = {
        let mut g = c.lock().unwrap();
        let changed = g.take_browser_changed();
        (g.take_loads(), if changed { g.browser().map(|b| b.describe()) } else { None })
    };
    if let Some(text) = browser {
        println!("{text}");
    }
    for (deck, path) in loads {
        loader.load(deck, &path.to_string_lossy());
    }
}

type Shared<T> = Arc<std::sync::Mutex<T>>;

/// Loads audio on worker threads and attaches the library analysis (grid, beats, cues).
#[derive(Clone)]
struct Loader {
    tx: std::sync::mpsc::Sender<Command>,
    sample_rate: u32,
    library: Arc<std::sync::RwLock<Option<rekordbox_formats::library::Library>>>,
    loaded: Shared<[Option<String>; 2]>,
    titles: Shared<[Option<String>; 2]>,
    browser_pending: Arc<std::sync::atomic::AtomicBool>,
}

impl Loader {
    fn new(tx: std::sync::mpsc::Sender<Command>, sample_rate: u32) -> Loader {
        Loader {
            tx,
            sample_rate,
            library: Default::default(),
            loaded: Default::default(),
            titles: Default::default(),
            browser_pending: Default::default(),
        }
    }

    /// Read the library in the background; analyses are then attached to already loaded decks.
    fn start_library(&self, anlz: Option<std::path::PathBuf>, music: Option<std::path::PathBuf>) {
        use rekordbox_formats::library::{default_anlz_root, default_music_root, Library};
        let (Some(anlz), Some(music)) = (anlz.or_else(default_anlz_root), music.or_else(default_music_root)) else {
            println!("library: no rekordbox analysis folder or music folder found");
            return;
        };
        let me = self.clone();
        std::thread::spawn(move || {
            let lib = Library::build(&anlz, &[music]);
            let r = lib.report;
            println!("library: {} tracks, {} with audio file, {} with beat grid", r.tracks, r.matched, r.with_grid);
            *me.library.write().unwrap() = Some(lib);
            me.browser_pending.store(true, std::sync::atomic::Ordering::Relaxed);
            let loaded = me.loaded.lock().unwrap().clone();
            for (deck, path) in loaded.iter().enumerate() {
                if let Some(p) = path {
                    me.apply(deck, p);
                }
            }
        });
    }

    fn library(&self) -> Option<std::sync::RwLockReadGuard<'_, Option<rekordbox_formats::library::Library>>> {
        let g = self.library.read().ok()?;
        g.is_some().then_some(g)
    }

    /// Playable library tracks for the browser, once after the library became ready.
    fn take_library_for_browser(&self) -> Option<Vec<std::path::PathBuf>> {
        if !self.browser_pending.swap(false, std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        let g = self.library.read().ok()?;
        Some(g.as_ref()?.playable().filter_map(|t| t.path.clone()).collect())
    }

    fn title(&self, deck: usize) -> Option<String> {
        self.titles.lock().unwrap()[deck].clone()
    }

    /// Send the analysis of `path` to `deck` if the library knows it.
    fn apply(&self, deck: usize, path: &str) {
        let g = self.library.read().unwrap();
        let Some(lib) = g.as_ref() else { return };
        let Some(t) = lib.by_path(Path::new(path)) else { return };
        let a = rbx_engine::analysis::TrackAnalysis::from_library(t);
        println!(
            "deck {}: {} – {}{}",
            deck + 1,
            a.title,
            a.grid.map_or("no beat grid".into(), |g| format!("{:.2} BPM", g.bpm)),
            if a.variable_tempo { " (variable tempo)" } else { "" }
        );
        self.titles.lock().unwrap()[deck] = Some(a.title.clone());
        let _ = self.tx.send(Command::ApplyAnalysis { deck, analysis: Arc::new(a) });
    }

    /// Decode on a worker thread, load, then attach the analysis.
    fn load(&self, deck: usize, spec: &str) {
        let me = self.clone();
        let spec = spec.to_string();
        std::thread::spawn(move || match source(&spec, me.sample_rate) {
            Ok((pcm, grid)) => {
                println!("deck {}: loaded {spec}", deck + 1);
                let _ = me.tx.send(Command::Load { deck, pcm, grid });
                me.loaded.lock().unwrap()[deck] = Some(spec.clone());
                me.titles.lock().unwrap()[deck] = None;
                me.apply(deck, &spec);
            }
            Err(e) => eprintln!("{e}"),
        });
    }
}

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace().map(|h| u8::from_str_radix(h, 16).map_err(|_| format!("{h}: not a hex byte"))).collect()
}

fn load_mapping(path: &Path) -> Result<rekordbox_formats::midi::MidiMapping, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let m = rekordbox_formats::midi::parse(&text);
    if !m.is_valid() {
        return Err(format!("{}: not a version 1 mapping file", path.display()));
    }
    Ok(m)
}

fn coverage(path: &Path) -> Result<(), String> {
    let c = Controller::new(load_mapping(path)?);
    let (yes, no) = c.coverage();
    let (ignored, active): (Vec<_>, Vec<_>) =
        yes.into_iter().partition(|f| rbx_engine::controller::ignore_reason(f).is_some());
    println!(
        "{}: {} functions handled, {} deliberately without action, {} not handled",
        c.mapping().device,
        active.len(),
        ignored.len(),
        no.len()
    );
    println!("handled:     {}", active.join(", "));
    for f in &ignored {
        println!("no action:   {f} ({})", rbx_engine::controller::ignore_reason(f).unwrap_or(""));
    }
    println!("not handled: {}", no.into_iter().collect::<Vec<_>>().join(", "));
    Ok(())
}

/// Controller running in the background: MIDI input thread and LED refresh thread.
struct ControllerHandle {
    controller: Arc<std::sync::Mutex<Controller>>,
    #[cfg(windows)]
    _input: Option<rekordbox_formats::midi_device::InputPort>,
}

fn start_controller(mapping: &str, opts: &[(&str, &str)], out: &Output, loader: &Loader) -> Result<ControllerHandle, String> {
    let m = load_mapping(Path::new(mapping))?;
    let device = m.device.clone();
    let controller = Arc::new(std::sync::Mutex::new(Controller::new(m)));
    let root = opts
        .iter()
        .find(|(k, _)| *k == "browse")
        .map(|(_, v)| std::path::PathBuf::from(v))
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| Path::new(&h).join("Music")))
        .filter(|p| p.is_dir());
    if let Some(r) = root {
        println!("browser: {}", r.display());
        controller.lock().unwrap().set_browser_root(r);
        controller.lock().unwrap().take_browser_changed();
    }
    let (yes, no) = controller.lock().unwrap().coverage();
    println!("controller {device:?}: {} functions handled, {} not (see midi-coverage)", yes.len(), no.len());
    #[cfg(windows)]
    {
        use rekordbox_formats::midi_device::{list_inputs, list_outputs, InputPort, OutputPort};
        use rekordbox_formats::midi_input::MidiMessage;
        let pick = |list: Vec<rekordbox_formats::midi_device::DeviceInfo>, key: &str| {
            let want = opts.iter().find(|(k, _)| *k == key).map_or(device.as_str(), |(_, v)| *v).to_lowercase();
            list.into_iter().find(|d| d.name.to_lowercase().contains(&want))
        };
        let mut input = None;
        match pick(list_inputs(), "midi-in") {
            Some(d) => {
                let (port, rx) = InputPort::open(d.index)?;
                println!("MIDI in: {}", d.name);
                let (c, tx, ld) = (controller.clone(), out.sender(), loader.clone());
                std::thread::spawn(move || {
                    for (packed, ms) in rx {
                        if let Some(msg) = MidiMessage::from_packed(packed, f64::from(ms) / 1000.0) {
                            let cmds = c.lock().unwrap().handle(&msg);
                            for cmd in cmds {
                                let _ = tx.send(cmd);
                            }
                            side_effects(&c, &ld);
                        }
                    }
                });
                input = Some(port);
            }
            None => println!("MIDI in: no device matching {device:?} found (use `midi ..` to simulate)"),
        }
        match pick(list_outputs(), "midi-out") {
            Some(d) => {
                let port = OutputPort::open(d.index)?;
                println!("MIDI out (LEDs): {}", d.name);
                let (c, state) = (controller.clone(), out.state_handle());
                std::thread::spawn(move || {
                    let mut last = std::collections::HashMap::new();
                    loop {
                        let s = *state.lock().unwrap();
                        for led in c.lock().unwrap().leds(&s) {
                            let key = (led.status, led.data1);
                            if last.get(&key) != Some(&led.value) {
                                if port.send(led.packed()).is_err() {
                                    return;
                                }
                                last.insert(key, led.value);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(40));
                    }
                });
            }
            None => println!("MIDI out: no device matching {device:?} found (no LED feedback)"),
        }
        return Ok(ControllerHandle { controller, _input: input });
    }
    #[cfg(not(windows))]
    {
        let _ = opts;
        Ok(ControllerHandle { controller })
    }
}
