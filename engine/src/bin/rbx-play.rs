//! `rbx-play` – try the audio engine.
//!
//! ```text
//! rbx-play devices                                    list output devices
//! rbx-play play <file1> [file2] [--device NAME]       live playback, commands on stdin (see control.rs)
//! rbx-play render <out.wav> <seconds> <file1|tone|click:BPM> [file2|…] [--script "0:play 1; …"]
//! rbx-play tone [seconds]                             440 Hz test tone on the default device
//! rbx-play midi-coverage <mapping.csv>                which mapping functions the engine handles
//! ```
//! `play` options: `--device NAME` (audio output), `--midi MAPPING.csv | auto` (controller; `auto` uses the
//! built-in mapping of a connected XDJ-RX3; MIDI in/out are
//! found by the mapping's device name, override with `--midi-in NAME` / `--midi-out NAME`). With a
//! mapping loaded, `midi B0 22 41` on stdin injects a message as if the controller had sent it.
//! `--samples DIR` fills the sampler from a folder (else built-in sounds), `--browse DIR` sets the
//! browser root for BROWSE/LOAD (default: the user's Music folder). `--djlink [ADAPTER]` joins the
//! Pro DJ Link network on that adapter (name part or IP; default: first private / link-local one),
//! The rekordbox library (analysis folder + music folder, `--anlz DIR`, `--music DIR`, `--no-library`)
//! is read in the background; loaded files get beat grid, beat positions and cues from it; `lib words`
//! searches it, `lload <deck> <n>` loads a search result, the browser shows `[Library]`.
//! Files without an analysis are analysed automatically on load (own analyzer, saved below
//! `%APPDATA%/rekordbox-rs/ANLZ`; `--no-analyze` turns this off).
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
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::output::{list_devices, Output};
use rbx_engine::pcm::{write_wav, Pcm};
use rbx_engine::session::{fill_sampler, load_mapping, side_effects, source, start_controller, start_link, Loader, Log};

fn main() -> ExitCode {
    rbx_engine::session::install_crash_handler("rbx-play");
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
    let mut loader = Loader::new(out.sender(), out.sample_rate, Log::default());
    loader.auto_analyze = !opts.iter().any(|(k, _)| *k == "no-analyze");
    if !opts.iter().any(|(k, _)| *k == "no-library") {
        let opt = |k: &str| opts.iter().find(|(o, _)| *o == k).map(|(_, v)| std::path::PathBuf::from(v));
        loader.start_library(opt("anlz"), opt("music"));
    }
    for (d, s) in sources.iter().take(2).enumerate() {
        loader.load(d, s);
    }
    let names = fill_sampler(&out.sender(), out.sample_rate, opts.iter().find(|(k, _)| *k == "samples").map(|(_, v)| Path::new(*v)));
    println!("sampler: {}", names.join(", "));
    let opt = |k: &str| opts.iter().find(|(o, _)| *o == k).map(|(_, v)| *v);
    let ctl = match opt("midi") {
        Some("auto") | Some("") => {
            let (port, mapping) = rbx_engine::session::detect_controller().ok_or("--midi auto: no controller with a built-in mapping connected")?;
            println!("controller: {port} (built-in mapping)");
            Some(rbx_engine::session::start_controller_with(
                mapping,
                Some(&port),
                opt("midi-out"),
                opt("browse").map(std::path::PathBuf::from),
                out.sender(),
                out.state_handle(),
                &loader,
            )?)
        }
        Some(m) => Some(start_controller(
            Path::new(m),
            opt("midi-in"),
            opt("midi-out"),
            opt("browse").map(std::path::PathBuf::from),
            out.sender(),
            out.state_handle(),
            &loader,
        )?),
        None => None,
    };
    let link = match opt("djlink") {
        Some(iface) => Some(start_link(
            if iface.is_empty() || iface == "auto" { None } else { Some(iface) },
            out.state_handle(),
            out.sender(),
            &loader.log,
        )?),
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

fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    s.split_whitespace().map(|h| u8::from_str_radix(h, 16).map_err(|_| format!("{h}: not a hex byte"))).collect()
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
