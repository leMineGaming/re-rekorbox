//! Text commands for the CLI and for scripted renders.
//!
//! ```text
//! play 1 | pause 1 | toggle 1          transport (decks 1 and 2)
//! cue 1 | cueup 1                      CUE pressed / released
//! hot 1 3 | hotup 1 3 | hotdel 1 3     hot cue 3 (1-based) pressed / released / deleted
//! in 1 | out 1 | reloop 1              manual loop
//! loop 1 4 | loopx 1 0.5               beat loop of 4 beats, scale active loop
//! tempo 1 0.25 | range 1 16            tempo fader −1..1, range 6/10/16/100 %
//! bpm 1 128 [0.12]                     set grid: BPM and first beat in seconds
//! touch 1 | release 1 | jog 1 -40      jog wheel
//! seek 1 0.5 | at 1 63.2               needle (fraction) / absolute seconds
//! skip 1 -2.5 | jump 1 4               relative seconds / beat jump (negative = back)
//! vol 1 0.8 | trim 1 1.2 | quant 1 on  channel settings
//! sync 2                               match deck 2 to deck 1
//! xf -1 | gain 0.9                     crossfader −1..1, master gain
//! eq 1 low 0.2 | eq 2 high 0.5         EQ knob 0..1 (0.5 flat, 0 kill)
//! cfx filter|space|echo|sweep|noise|crush|off   Sound Color FX type
//! color 1 0.2 | sub 0.7                Color FX knob of a channel (0.5 off), sub-parameter
//! fx on|off | fx echo | fxdepth 0.6    Beat FX on/off, type (delay, echo, pingpong, spiral, helix,
//!                                      reverb, flanger, phaser, filter, trans, roll, sliproll, pitch,
//!                                      vinylbrake), depth
//! fxbeat up|down | fxbpm 128|auto      beat fraction, tempo (tapped value or automatic)
//! fxassign 1|2|master|sampler          where the Beat FX is inserted
//! padfx 1 1 5 | padfx 1 off            Pad FX of deck 1: bank 1, slot 5 (held) / release
//! reverse 1 on | slip 1 on | mt 1 on   reverse, slip mode, master tempo (key lock)
//! key 1 -2 | brake 1 1.5 | autocue 1 on   key shift (semitones), vinyl brake time, auto cue
//! mem 1 | call 1 next|prev | memdel 1  memory cues
//! search 1 4 | search 1 0              search button held at a speed / released
//! kbd 1 5 | kbdup 1                    keyboard pad (semitones) pressed / released
//! double 2 | master 1|auto             instant double, sync master
//! ```
//! A script is `time:command; time:command; …` with times in seconds, e.g.
//! `0:play 1; 4:bpm 2 124; 4:sync 2; 4:play 2; 8:xf 1`.

use crate::deck::Grid;
use crate::dsp::ColorFx;
use crate::mixer::Command;

fn deck(s: Option<&str>) -> Result<usize, String> {
    match s.and_then(|s| s.parse::<usize>().ok()) {
        Some(d @ 1..=2) => Ok(d - 1),
        _ => Err("deck must be 1 or 2".into()),
    }
}

fn num<T: std::str::FromStr>(s: Option<&str>, what: &str) -> Result<T, String> {
    s.and_then(|s| s.parse().ok()).ok_or(format!("missing or invalid {what}"))
}

fn slot(s: Option<&str>) -> Result<usize, String> {
    match num::<usize>(s, "hot cue number")? {
        n @ 1..=8 => Ok(n - 1),
        _ => Err("hot cue number must be 1..8".into()),
    }
}

fn on_off(s: Option<&str>) -> Result<bool, String> {
    match s {
        Some("on" | "1" | "true") => Ok(true),
        Some("off" | "0" | "false") => Ok(false),
        _ => Err("expected on or off".into()),
    }
}

/// Parse one command line (without `load`, which needs file decoding).
pub fn parse(line: &str) -> Result<Command, String> {
    let mut it = line.split_whitespace();
    let word = it.next().ok_or("empty command")?.to_lowercase();
    let a = it.next();
    let b = it.next();
    let c = it.next();
    use Command::*;
    Ok(match word.as_str() {
        "play" => Play(deck(a)?),
        "pause" => Pause(deck(a)?),
        "toggle" => TogglePlay(deck(a)?),
        "cue" => CuePress(deck(a)?),
        "cueup" => CueRelease(deck(a)?),
        "hot" => HotCuePress(deck(a)?, slot(b)?),
        "hotup" => HotCueRelease(deck(a)?, slot(b)?),
        "hotdel" => HotCueDelete(deck(a)?, slot(b)?),
        "in" => LoopIn(deck(a)?),
        "out" => LoopOut(deck(a)?),
        "reloop" => ReloopExit(deck(a)?),
        "loop" => BeatLoop(deck(a)?, num(b, "beats")?),
        "loopx" => LoopScale(deck(a)?, num(b, "factor")?),
        "tempo" => TempoFader(deck(a)?, num(b, "tempo fader value")?),
        "range" => {
            let idx = match num::<u32>(b, "range")? {
                6 => 0,
                10 => 1,
                16 => 2,
                100 => 3,
                _ => return Err("range must be 6, 10, 16 or 100".into()),
            };
            TempoRange(deck(a)?, idx)
        }
        "bpm" => {
            let bpm: f64 = num(b, "BPM")?;
            if bpm <= 0.0 {
                return Err("BPM must be positive".into());
            }
            let first = c.map_or(Ok(0.0), |s| s.parse().map_err(|_| "invalid first beat".to_string()))?;
            SetGrid { deck: deck(a)?, grid: Some(Grid { bpm, first_beat_s: first }) }
        }
        "touch" => JogTouch(deck(a)?, true),
        "release" => JogTouch(deck(a)?, false),
        "jog" => Jog(deck(a)?, num(b, "jog delta")?),
        "seek" => SeekFraction(deck(a)?, num(b, "fraction")?),
        "at" => SeekSeconds(deck(a)?, num(b, "seconds")?),
        "skip" => SeekBy(deck(a)?, num(b, "seconds")?),
        "jump" => BeatJump(deck(a)?, num(b, "beats")?),
        "vol" => Volume(deck(a)?, num(b, "volume")?),
        "trim" => Trim(deck(a)?, num(b, "trim")?),
        "quant" => Quantize(deck(a)?, matches!(b, Some("on" | "1" | "true"))),
        "sync" => Sync(deck(a)?),
        "eq" => {
            let band = match b {
                Some("low") => 0,
                Some("mid") => 1,
                Some("high" | "hi") => 2,
                _ => return Err("band must be low, mid or high".into()),
            };
            Eq(deck(a)?, band, num(c, "EQ value")?)
        }
        "cfx" => ColorFxSelect(match a {
            Some("off") => None,
            Some("filter") => Some(ColorFx::Filter),
            Some("space") => Some(ColorFx::Space),
            Some("echo" | "dubecho") => Some(ColorFx::DubEcho),
            Some("sweep") => Some(ColorFx::Sweep),
            Some("noise") => Some(ColorFx::Noise),
            Some("crush") => Some(ColorFx::Crush),
            _ => return Err("cfx filter|space|echo|sweep|noise|crush|off".into()),
        }),
        "color" => ColorKnob(deck(a)?, num(b, "color knob value")?),
        "sub" => ColorSub(num(a, "sub-parameter value")?),
        "fx" => match a {
            Some("on") => BeatFxOn(true),
            Some("off") => BeatFxOn(false),
            Some(name) => BeatFxSelect(
                crate::fx::BeatFxKind::from_name(&name.to_uppercase()).ok_or(format!("unknown beat fx {name:?}"))?,
            ),
            None => return Err("fx on|off|<type>".into()),
        },
        "fxdepth" => BeatFxDepth(num(a, "depth")?),
        "fxbeat" => BeatFxBeat(match a {
            Some("up") => 1,
            Some("down") => -1,
            _ => return Err("fxbeat up|down".into()),
        }),
        "fxbpm" => BeatFxBpm(match a {
            Some("auto") => None,
            other => Some(num(other, "BPM")?),
        }),
        "fxassign" => BeatFxAssign(match a {
            Some("1") => crate::fx::FxAssign::Channel(0),
            Some("2") => crate::fx::FxAssign::Channel(1),
            Some("master") => crate::fx::FxAssign::Master,
            Some("sampler") => crate::fx::FxAssign::Sampler,
            _ => return Err("fxassign 1|2|master|sampler".into()),
        }),
        "padfx" => PadFx(deck(a)?, match b {
            Some("off") => None,
            Some(bank) => {
                let bank: usize = bank.parse().map_err(|_| "invalid bank")?;
                let slot: usize = num(c, "slot")?;
                if !(1..=2).contains(&bank) || !(1..=16).contains(&slot) {
                    return Err("bank 1..2, slot 1..16".into());
                }
                Some((bank - 1, slot - 1))
            }
            None => return Err("padfx <deck> <bank> <slot> | padfx <deck> off".into()),
        }),
        "xf" => Crossfader(num(a, "crossfader value")?),
        "reverse" => Reverse(deck(a)?, on_off(b)?),
        "slip" => Slip(deck(a)?, on_off(b)?),
        "mt" => MasterTempo(deck(a)?, on_off(b)?),
        "autocue" => AutoCue(deck(a)?, on_off(b)?),
        "key" => Key(deck(a)?, num(b, "semitones")?),
        "brake" => Brake(deck(a)?, num(b, "seconds")?),
        "mem" => MemoryStore(deck(a)?),
        "call" => MemoryCall(deck(a)?, match b {
            Some("next") => true,
            Some("prev") => false,
            _ => return Err("call <deck> next|prev".into()),
        }),
        "memdel" => MemoryDelete(deck(a)?),
        "search" => Search(deck(a)?, num(b, "speed")?),
        "kbd" => KeyboardPress(deck(a)?, num(b, "semitones")?),
        "kbdup" => KeyboardRelease(deck(a)?),
        "double" => InstantDouble(deck(a)?),
        "master" => match a {
            Some("auto") => SetMaster(None),
            Some("1" | "2") => SetMaster(Some(deck(a)?)),
            _ => MasterGain(num(a, "master gain")?),
        },
        "gain" => MasterGain(num(a, "master gain")?),
        other => return Err(format!("unknown command {other:?}")),
    })
}

/// Parse a script into (time, command) pairs sorted by time.
pub fn parse_script(script: &str) -> Result<Vec<(f64, Command)>, String> {
    let mut out = Vec::new();
    for part in script.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (t, cmd) = part.split_once(':').ok_or(format!("{part:?}: expected time:command"))?;
        let t: f64 = t.trim().parse().map_err(|_| format!("{part:?}: invalid time"))?;
        out.push((t, parse(cmd).map_err(|e| format!("{part:?}: {e}"))?));
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out)
}

/// Run a script against a mixer for `seconds` and return the rendered audio. Commands take effect
/// at the first block boundary at or after their time (blocks are at most 10 ms).
pub fn render_script(mixer: &mut crate::mixer::Mixer, script: &str, seconds: f64) -> Result<crate::pcm::Pcm, String> {
    let sr = mixer.sample_rate;
    let total = (seconds * f64::from(sr)) as usize;
    let block = (sr as usize / 100).max(1);
    let mut events = parse_script(script)?.into_iter().peekable();
    let mut out = vec![[0.0f32; 2]; total];
    let mut done = 0;
    while done < total {
        let t = done as f64 / f64::from(sr);
        while events.peek().is_some_and(|(at, _)| *at <= t + 1e-9) {
            mixer.apply(events.next().unwrap().1);
        }
        let mut end = (done + block).min(total);
        if let Some((at, _)) = events.peek() {
            let at_frame = (at * f64::from(sr)).ceil() as usize;
            if at_frame > done && at_frame < end {
                end = at_frame;
            }
        }
        mixer.render(&mut out[done..end]);
        done = end;
    }
    Ok(crate::pcm::Pcm::new(sr, out))
}
