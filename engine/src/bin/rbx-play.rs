//! `rbx-play` – try the audio engine.
//!
//! ```text
//! rbx-play devices                                    list output devices
//! rbx-play play <file1> [file2] [--device NAME]       live playback, commands on stdin (see control.rs)
//! rbx-play render <out.wav> <seconds> <file1|tone|click:BPM> [file2|…] [--script "0:play 1; …"]
//! rbx-play tone [seconds]                             440 Hz test tone on the default device
//! ```
//! Sources: an audio file path, `tone` (440 Hz sine) or `click:BPM` (click track) – the latter two
//! allow testing without music files.

use std::io::BufRead;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rbx_engine::control;
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
            opts.push((flag, rest.get(i + 1).copied().unwrap_or("")));
            i += 2;
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
    for (d, s) in sources.iter().take(2).enumerate() {
        let (pcm, grid) = source(s, out.sample_rate)?;
        out.send(Command::Load { deck: d, pcm, grid });
    }
    println!("commands: play 1, cue 1, hot 1 1, loop 1 4, tempo 1 0.2, sync 2, xf 0, load 2 <file>, state, quit");
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        match line {
            "" => continue,
            "quit" | "q" | "exit" => break,
            "state" | "s" => print_state(&out),
            l if l.starts_with("load ") => {
                let mut p = l.splitn(3, ' ').skip(1);
                let d = p.next().and_then(|d| d.parse::<usize>().ok()).filter(|d| (1..=2).contains(d));
                match (d, p.next()) {
                    (Some(d), Some(f)) => match source(f.trim().trim_matches('"'), out.sample_rate) {
                        Ok((pcm, grid)) => out.send(Command::Load { deck: d - 1, pcm, grid }),
                        Err(e) => eprintln!("{e}"),
                    },
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
