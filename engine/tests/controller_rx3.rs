//! The built-in XDJ-RX3 mapping driven by synthetic MIDI (no device needed): every control, browser
//! events for the host, LEDs, unknown messages, MIDI log recording and replay.

use rbx_engine::controller::{builtin_for_port, builtin_mappings, function_of, parse_midilog, BrowseEvent, Controller, XDJ_RX3_MAPPING};
use rbx_engine::mixer::MixerState;
use rekordbox_formats::midi::{parse, MidiMapping};
use rekordbox_formats::midi_input::MidiMessage;

fn rx3() -> MidiMapping {
    let m = parse(XDJ_RX3_MAPPING);
    assert!(m.is_valid());
    m
}

/// Messages that operate a control of this type once (press/release, a turn, a fader move).
fn operate(type_text: &str, message: u16, t: &mut f64) -> Vec<MidiMessage> {
    let (st, d1) = ((message >> 8) as u8, message as u8);
    if type_text.ends_with("HiRes") {
        // 14-bit: MSB on the message, LSB on message + 0x20 (the value is reported with the LSB)
        let mut v = Vec::new();
        for (msb, lsb) in [(0x20u8, 0x10u8), (0x60, 0x00)] {
            *t += 0.5;
            v.push(MidiMessage::from_bytes(&[st, d1, msb], *t).unwrap());
            v.push(MidiMessage::from_bytes(&[st, d1 + 0x20, lsb], *t).unwrap());
        }
        return v;
    }
    let values: &[u8] = match type_text {
        "Button" | "Pad" => &[0x7F, 0x00],
        "KnobSlider" | "Knob" | "Slider" => &[0x20, 0x60],
        _ => &[0x41, 0x3F],
    };
    values
        .iter()
        .map(|&v| {
            *t += 0.5; // far apart: no double-press / long-press effects
            MidiMessage::from_bytes(&[st, d1, v], *t).unwrap()
        })
        .collect()
}

#[test]
fn builtin_mapping_is_complete() {
    let m = rx3();
    assert_eq!(m.device, "XDJ-RX3");
    assert_eq!(builtin_mappings().len(), 1);
    assert!(builtin_for_port("2- XDJ-RX3").is_some());
    assert!(builtin_for_port("DDJ-400").is_none());
    let c = Controller::new(m);
    let (yes, no) = c.coverage();
    assert!(no.is_empty(), "{no:?}");
    assert!(yes.len() >= 210);
}

#[test]
fn every_control_reaches_the_controller() {
    // the automated version of "move every control once"
    let m = rx3();
    let mut c = Controller::new(m.clone());
    let mut t = 0.0;
    for e in &m.entries {
        for b in e.input_bindings(&m.device) {
            for msg in operate(&e.type_text, b.message, &mut t) {
                let _ = c.handle(&msg);
            }
        }
    }
    let missing: Vec<String> = c
        .testable_controls()
        .into_iter()
        .filter(|(_, f, d)| !c.seen().contains(&(f.clone(), *d)))
        .map(|(g, f, d)| format!("{g}/{f}@{d}"))
        .collect();
    assert!(missing.is_empty(), "controls without event: {missing:?}");
    assert!(c.unknown().is_empty(), "every message of the mapping is known");
    assert!(c.testable_controls().len() > 200);
}

fn input_of(m: &MidiMapping, name: &str, deck: u8) -> u16 {
    let e = m.entries.iter().find(|e| function_of(e) == name).unwrap_or_else(|| panic!("{name} not in mapping"));
    e.input_bindings(&m.device).into_iter().find(|b| b.deck == deck).unwrap_or_else(|| panic!("{name} deck {deck}")).message
}

#[test]
fn browser_controls_become_host_events() {
    let m = rx3();
    let mut c = Controller::new(m.clone());
    c.set_external_browser(true);
    let msg = |msg: u16, v: u8, t: f64| MidiMessage::from_bytes(&[(msg >> 8) as u8, msg as u8, v], t).unwrap();
    let browse = input_of(&m, "Browse", 0);
    c.handle(&msg(browse, 0x41, 1.0));
    c.handle(&msg(browse, 0x3F, 1.1));
    let load1 = input_of(&m, "Load", 1);
    c.handle(&msg(load1, 0x7F, 2.0));
    c.handle(&msg(load1, 0x00, 2.05));
    let load2 = input_of(&m, "Load", 2);
    c.handle(&msg(load2, 0x7F, 3.0));
    let back = input_of(&m, "Back", 0);
    c.handle(&msg(back, 0x7F, 4.0));
    let tag = input_of(&m, "AddToTagList", 0);
    c.handle(&msg(tag, 0x7F, 5.0));
    let next = input_of(&m, "TrackNext", 1);
    c.handle(&msg(next, 0x7F, 6.0));
    let ev = c.take_browse_events();
    assert_eq!(
        ev,
        [BrowseEvent::Scroll(1), BrowseEvent::Scroll(-1), BrowseEvent::Load(0), BrowseEvent::Load(1), BrowseEvent::Back, BrowseEvent::Tag, BrowseEvent::Step(0, true)]
    );
    assert!(c.take_loads().is_empty(), "the built-in folder browser is not used");
}

#[test]
fn leds_follow_the_engine_and_are_valid_midi() {
    let c = Controller::new(rx3());
    let all = c.led_outputs();
    assert!(all.len() > 50, "LED outputs: {}", all.len());
    assert!(all.iter().all(|(_, l)| (0x80..=0xEF).contains(&l.status) && l.data1 < 0x80 && l.value == 0x7F));
    let mut s = MixerState::default();
    s.decks[0].playing = true;
    s.decks[0].loaded = true;
    let leds = c.leds(&s);
    assert!(leds.iter().all(|l| (0x80..=0xEF).contains(&l.status) && l.data1 < 0x80 && l.value < 0x80));
    let play1 = leds.iter().find(|l| l.status == 0x90 && l.data1 == 0x00).expect("PLAY deck 1");
    assert_eq!(play1.value, 0x7F);
    let play2 = leds.iter().find(|l| l.status == 0x91 && l.data1 == 0x00).expect("PLAY deck 2");
    assert_eq!(play2.value, 0);
}

#[test]
fn unknown_messages_are_reported() {
    let mut c = Controller::new(rx3());
    c.handle(&MidiMessage::from_bytes(&[0x9F, 0x7E, 0x7F], 0.0).unwrap());
    c.handle(&MidiMessage::from_bytes(&[0x9F, 0x7E, 0x00], 0.1).unwrap());
    assert_eq!(c.unknown().get(&(0x9F, 0x7E)), Some(&2));
    assert!(c.monitor().back().unwrap().unknown);
    c.reset_test();
    assert!(c.unknown().is_empty() && c.monitor().is_empty());
}

#[test]
fn record_and_replay_a_session() {
    let m = rx3();
    let dir = std::env::temp_dir().join(format!("rbx-midilog-{}", std::process::id()));
    let path = dir.join("session.midilog");
    let mut c = Controller::new(m.clone());
    c.start_recording(&path).unwrap();
    let mut t = 0.0;
    let mut sent = Vec::new();
    for name in ["PlayPause", "Cue", "Sync", "ChannelFader", "EQHigh", "CrossFader"] {
        let e = m.entries.iter().find(|e| function_of(e) == name).unwrap();
        let b = e.input_bindings(&m.device)[0];
        sent.extend(operate(&e.type_text, b.message, &mut t));
    }
    let live: Vec<String> = sent.iter().flat_map(|msg| c.handle(msg)).map(|c| format!("{c:?}")).collect();
    c.stop_recording();
    let text = std::fs::read_to_string(&path).unwrap();
    let back = parse_midilog(&text);
    assert_eq!(back.len(), sent.len());
    assert!(back.iter().zip(&sent).all(|(a, b)| (a.status, a.data1, a.data2) == (b.status, b.data1, b.data2) && (a.time_s - b.time_s).abs() < 1e-3));
    let mut fresh = Controller::new(m);
    let replayed: Vec<String> = fresh.replay(&back).into_iter().map(|(_, c)| format!("{c:?}")).collect();
    assert_eq!(replayed, live, "a replay gives the same engine commands");
    assert!(!live.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
