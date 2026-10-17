//! MIDI input processing tests on a synthetic mapping. Rules come from SPEC.md §5f-2.

use rekordbox_formats::midi::parse;
use rekordbox_formats::midi_input::{rotary_accelerate, rotary_delta, Action, Event, InputProcessor, MidiMessage};

// Device name XDJ-RX3 so that `Pad` resolves to the DdjRxPad class; the rows are invented test data.
const MAP: &str = "@file,1,XDJ-RX3\n\
Play,PlayPause,Button,900B,0,1,2,3,,,,,,,play\n\
Shift,Shift,Button,903F,,,,,,,,,,Min=5;Max=99,custom range\n\
Jog,JogScratch,JogRotate,B022,0,1,2,3,,,,,,RO,jog\n\
Bend,JogPitchBend,JogRotate,B022,0,1,2,3,,,,,,RO,second row on same message\n\
Search,JogSearch,Difference,B029,0,1,2,3,,,,,,RO,search\n\
Touch,JogTouch,JogTouch,9020,0,1,2,3,,,,,,RO,touch\n\
Browse,Browse,Rotary,B04F,,,,,,,,,,,browse\n\
Plain,Plain,Rotary,B050,,,,,,,,,,Value=0,no acceleration\n\
Knob,KnobLike,RotaryAsKnob,B051,,,,,,,,,,Value=100,knob\n\
Trim,Trim,KnobSlider,B004,0,1,2,3,,,,,,,trim\n\
Needle,NeedleSearch,Value,B01C,0,1,2,3,,,,,,Max=16383,needle\n\
Small,Small,Value,B01D,,,,,,,,,,,default max\n\
Raw,Raw,RawValue,B01E,,,,,,,,,,,raw\n\
Tempo,TempoSlider,KnobSliderHiRes,B000,0,1,2,3,,,,,,,tempo\n\
Cue,PAD1.HotCue.Call,Pad,9000,5,6,7,8,9000,5,6,7,8,Fast,pad\n\
Lamp,Lamp,Indicator,,,,,,B002,0,1,2,3,RO,output only\n\
Toggle,Toggle,Toggle,9050,,,,,,,,,,,no input handler\n";

fn msg(bytes: &[u8]) -> MidiMessage {
    MidiMessage::from_bytes(bytes, 1.5).unwrap()
}

fn proc() -> InputProcessor {
    InputProcessor::new(parse(MAP))
}

fn first(p: &mut InputProcessor, bytes: &[u8]) -> Action {
    p.process(&msg(bytes))[0].action
}

fn names(p: &InputProcessor, evs: &[Event]) -> Vec<String> {
    evs.iter().map(|e| p.entry(e).function.clone()).collect()
}

#[test]
fn message_construction() {
    let m = MidiMessage::from_packed(0x0041_22B0, 0.25).unwrap();
    assert_eq!((m.key(), m.value(), m.time_ms()), (0xB022, 0x41, 250.0));
    let pc = MidiMessage::from_packed(0x0000_05C0, 0.0).unwrap();
    assert_eq!((pc.key(), pc.data2, pc.value()), (0xC005, None, 0), "2-byte message: value 0");
    assert!(MidiMessage::from_bytes(&[0x22], 0.0).is_none(), "no status byte");
}

#[test]
fn button_state_and_value() {
    let mut p = proc();
    let ev = p.process(&msg(&[0x91, 0x0B, 0x7F]));
    assert_eq!((ev.len(), ev[0].deck), (1, 2), "channel 1 = deck 2");
    assert_eq!(ev[0].action, Action::Switch { on: true, value: 0x7F });
    assert_eq!(first(&mut p, &[0x91, 0x0B, 0]), Action::Switch { on: false, value: 0 });
    // 63 is not a press value, but the state is still raw != 0
    assert_eq!(first(&mut p, &[0x90, 0x0B, 63]), Action::Switch { on: true, value: 0 });
    assert_eq!(first(&mut p, &[0x90, 0x0B, 64]), Action::Switch { on: true, value: 0x7F });
    assert_eq!(first(&mut p, &[0x90, 0x0B, 65]), Action::Switch { on: true, value: 0 });
    // Min/Max options replace 0 / 0x7F
    assert_eq!(first(&mut p, &[0x90, 0x3F, 0x7F]), Action::Switch { on: true, value: 99 });
    assert_eq!(first(&mut p, &[0x90, 0x3F, 0]), Action::Switch { on: false, value: 5 });
}

#[test]
fn rx_pad_press_release_and_velocity() {
    let mut p = proc();
    let ev = p.process(&msg(&[0x97, 0x00, 0x7F]));
    assert_eq!((ev[0].deck, ev[0].action), (3, Action::PadPress { pressed: true }), "channel 7 = deck 3");
    assert_eq!(first(&mut p, &[0x97, 0x00, 0]), Action::PadPress { pressed: false });
    assert_eq!(first(&mut p, &[0x97, 0x00, 0x30]), Action::PadValue { value: 0x30 });
    assert!(p.process(&msg(&[0x90, 0x00, 0x7F])).is_empty(), "channel 0 is not a pad channel");
}

#[test]
fn jog_messages() {
    let mut p = proc();
    let ev = p.process(&msg(&[0xB0, 0x22, 0x3E]));
    assert_eq!(names(&p, &ev), ["JogScratch", "JogPitchBend"], "both rows on B022 report, in file order");
    assert_eq!(ev[0].action, Action::JogDelta { delta: -2, time_ms: 1500.0 });
    assert_eq!(first(&mut p, &[0xB3, 0x29, 0x45]), Action::SearchDelta { delta: 5, time_ms: 1500.0 });
    assert_eq!(first(&mut p, &[0x90, 0x20, 0x7F]), Action::Touch { touched: true, time_ms: 1500.0 });
    assert_eq!(first(&mut p, &[0x90, 0x20, 0x40]), Action::Touch { touched: false, time_ms: 1500.0 });
}

#[test]
fn rotary_rules() {
    assert_eq!(rotary_delta(1), 1);
    assert_eq!(rotary_delta(30), 30);
    assert_eq!(rotary_delta(0x7F), -1);
    assert_eq!(rotary_delta(98), -30);
    assert_eq!(rotary_delta(65), 1);
    assert_eq!(rotary_delta(63), -1);
    assert_eq!(rotary_delta(0), 0);
    assert_eq!(rotary_delta(32), 32, "outside all ranges: unchanged");
    // acceleration with the default factor 2.22
    assert_eq!(rotary_accelerate(1, 2.22), 1);
    assert_eq!(rotary_accelerate(3, 2.22), 4, "2.22^2 = 4.93 -> 4");
    assert_eq!(rotary_accelerate(10, 2.22), 30, "clamped");
    assert_eq!(rotary_accelerate(-1, 2.22), -1);
    assert_eq!(rotary_accelerate(-3, 2.22), -4);
    assert_eq!(rotary_accelerate(-30, 2.22), -29, "clamped");
    assert_eq!(rotary_accelerate(0, 2.22), 0);

    let mut p = proc();
    assert_eq!(first(&mut p, &[0xB0, 0x4F, 3]), Action::Relative { delta: 4 }, "default acceleration");
    assert_eq!(first(&mut p, &[0xB0, 0x50, 3]), Action::Relative { delta: 3 }, "Value=0: off");
}

#[test]
fn rotary_as_knob_accumulates() {
    let mut p = proc();
    // Value=100 -> range 0..100, start round(50.0) = 50
    assert_eq!(first(&mut p, &[0xB0, 0x51, 10]), Action::Absolute { value: (0.6 * 16383.0) as u16 });
    assert_eq!(first(&mut p, &[0xB0, 0x51, 0x7F - 9]), Action::Absolute { value: (0.5 * 16383.0) as u16 }, "-10");
    for _ in 0..10 {
        p.process(&msg(&[0xB0, 0x51, 63]));
    }
    assert_eq!(first(&mut p, &[0xB0, 0x51, 0]), Action::Absolute { value: 16383 }, "clamped at max");
}

#[test]
fn scaled_values() {
    let mut p = proc();
    // KnobSlider default range 0..16383, center 8192
    assert_eq!(first(&mut p, &[0xB0, 0x04, 0]), Action::Absolute { value: 0 });
    assert_eq!(first(&mut p, &[0xB0, 0x04, 64]), Action::Absolute { value: 8192 });
    assert_eq!(first(&mut p, &[0xB0, 0x04, 32]), Action::Absolute { value: 4096 });
    assert_eq!(first(&mut p, &[0xB0, 0x04, 127]), Action::Absolute { value: 16383 });
    // Value with Max=16383 behaves the same; without Max the range is 0..127 (center 64)
    assert_eq!(first(&mut p, &[0xB1, 0x1C, 127]), Action::Absolute { value: 16383 });
    assert_eq!(first(&mut p, &[0xB0, 0x1D, 127]), Action::Absolute { value: 127 });
    assert_eq!(first(&mut p, &[0xB0, 0x1D, 100]), Action::Absolute { value: 100 });
    assert_eq!(first(&mut p, &[0xB0, 0x1D, 10]), Action::Absolute { value: 10 });
    assert_eq!(first(&mut p, &[0xB0, 0x1E, 77]), Action::Absolute { value: 77 }, "RawValue");
}

#[test]
fn fourteen_bit_reports_on_lsb_only() {
    let mut p = proc();
    assert!(p.process(&msg(&[0xB2, 0x00, 0x40])).is_empty(), "MSB alone reports nothing");
    let ev = p.process(&msg(&[0xB2, 0x20, 0x05]));
    assert_eq!((ev[0].deck, ev[0].action), (3, Action::Absolute { value: (0x40 << 7) | 5 }));
    // a second LSB is ORed into the stored value (no replacement)
    assert_eq!(first(&mut p, &[0xB2, 0x20, 0x02]), Action::Absolute { value: (0x40 << 7) | 7 });
    // a new MSB resets the low bits
    p.process(&msg(&[0xB2, 0x00, 0x7F]));
    assert_eq!(first(&mut p, &[0xB2, 0x20, 0x7F]), Action::Absolute { value: 0x3FFF });
}

#[test]
fn routing_overview() {
    let p = proc();
    let keys = p.listened_keys();
    assert!(keys.contains(&0xB020) && keys.contains(&0xB320), "LSB companions are registered");
    assert!(!keys.contains(&0xB002), "output-only rows are not inputs");
    let unhandled: Vec<_> = p.unhandled().iter().map(|&(i, c)| (p.mapping().entries[i].name.as_str(), c)).collect();
    assert_eq!(unhandled, [("Toggle", 0x1B)]);
    assert!(InputProcessor::new(parse("@file,2,X\nA,B,Button,9000,,,,,,,,,,,c\n")).listened_keys().is_empty());
}
