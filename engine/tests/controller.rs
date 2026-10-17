//! Controller binding tests: synthetic mapping in the format of the rekordbox mapping files (own rows,
//! modelled on the structure of an all-in-one unit), synthetic MIDI messages, real mixer.

use std::sync::Arc;

use rbx_engine::controller::{target_of, Controller, Target};
use rbx_engine::deck::Grid;
use rbx_engine::mixer::{Command, Mixer};
use rbx_engine::pcm::Pcm;
use rekordbox_formats::midi::parse;
use rekordbox_formats::midi_input::MidiMessage;

const MAP: &str = "@file,1,XDJ-RX3\n\
PlayPause,PlayPause,Button,9000,0,1,2,3,9000,0,1,2,3,Fast,play\n\
Cue,Cue,Button,9001,0,1,2,3,9001,0,1,2,3,Fast,cue\n\
LoopIn,LoopIn,Button,9006,0,1,2,3,9006,0,1,2,3,Fast,\n\
LoopOut,LoopOut,Button,9007,0,1,2,3,9007,0,1,2,3,Fast,\n\
ReloopExit,ReloopExit,Button,9008,0,1,2,3,9008,0,1,2,3,Fast,\n\
Quantize,Quantize,Button,9009,0,1,2,3,9009,0,1,2,3,,\n\
TempoRange,TempoRange,Button,9010,0,1,2,3,,,,,,,\n\
Vinyl,Vinyl,Button,9012,0,1,2,3,9012,0,1,2,3,,\n\
Sync,Sync,Button,901F,0,1,2,3,901F,0,1,2,3,Blink=600,\n\
JogScratch,JogScratch,JogRotate,B022,0,1,2,3,,,,,,RO,\n\
JogPitchBend,JogPitchBend,JogRotate,B022,0,1,2,3,,,,,,RO,same message\n\
WheelSearch,WheelSearch,JogRotate,B026,0,1,2,3,,,,,,RO,\n\
JogTouch,JogTouch,JogTouch,9020,0,1,2,3,,,,,,RO,\n\
TempoSlider,TempoSlider,KnobSliderHiRes,B000,0,1,2,3,,,,,,Fast,\n\
NeedleSearch,NeedleSearch,Value,B01C,0,1,2,3,,,,,,Max=16383,\n\
ChannelFader,ChannelFader,KnobSlider,,B411,B412,B413,B414,,,,,,,\n\
Gain,Gain,KnobSlider,,B401,B406,B40C,B450,,,,,,Fast,\n\
CrossFader,CrossFader,KnobSlider,B40B,,,,,,,,,,Fast,\n\
EQHigh,EQHigh,KnobSlider,,B402,B407,B40E,B451,,,,,,Fast,\n\
EQLow,EQLow,KnobSlider,,B404,B409,B410,B452,,,,,,Fast,\n\
CFXParameterCH2,CFXParameterCH2,KnobSlider,B40A,,,,,,,,,,Fast,\n\
CFXSubParameter,CFXSubParameter,KnobSlider,B46C,,,,,,,,,,Fast,\n\
CFX6On,CFX1On,Button,B457,,,,,B457,,,,,RO;Fast,CFX FILTER\n\
CFX1On,CFX2On,Button,B469,,,,,B469,,,,,RO;Fast,CFX SPACE\n\
FX1-1On,FX1-1On,Button,B472,,,,,B472,,,,,,beat fx on\n\
FX1-1Select,FX1-1On+Shift,Button,B434,,,,,B434,,,,,,\n\
FX1-1,FX1-1,KnobSlider,B45B,,,,,,,,,,,\n\
FX1BeatUp,FX1BeatUp,Button,B44D,,,,,,,,,,,\n\
FX1TapBeat,FX1Tap,Button,B44E,,,,,B44E,,,,,,\n\
FX1AutoBeat,FX1Tap+Shift,Button,B47E,,,,,B47E,,,,,,\n\
FX1AdjustTime,FX1ReleaseFx,Rotary,B42D,,,,,,,,,,,\n\
FX1-1Select.ECHO,ECHO,Button,B437,,,,,,,,,,,\n\
FX1Assign.CH1,,Button,B427,,,,,,,,,,,\n\
PAD1_PadFx1,PAD1.PadFx1,Pad,9020,5,6,7,8,9020,5,6,7,8,Fast,\n\
Reverse,Reverse,Button,9021,0,1,2,3,9021,0,1,2,3,Fast,\n\
Slip,Slip,Button,902C,0,1,2,3,902C,0,1,2,3,,\n\
MasterTempo,MasterTempo,Button,9011,0,1,2,3,9011,0,1,2,3,,\n\
VinylAdjustBrake,VinylAdjustBrake,KnobSlider,B01E,0,1,2,3,,,,,,,\n\
Memory,Memory,Button,900A,0,1,2,3,,,,,,,\n\
CallNext,CallNext,Button,900B,0,1,2,3,,,,,,,\n\
Delete,Delete,Button,900D,0,1,2,3,,,,,,,\n\
TimeMode,TimeMode,Button,900E,0,1,2,3,,,,,,,\n\
AutoCue,TimeMode+LongPress,Button,900F,0,1,2,3,,,,,,,\n\
4BeatLoopLongPress,LoopIn+LongPress,Button,9045,0,1,2,3,9045,0,1,2,3,Fast,\n\
Master,Master,Button,901E,0,1,2,3,901E,0,1,2,3,,\n\
InstDoubles,Sync+LongPress,Button,9047,0,1,2,3,,,,,,,\n\
SearchFwd,SearchFwd,Button,9002,0,1,2,3,,,,,,,\n\
Keyboard,HotCue+Shift,Button,9026,0,1,2,3,9026,0,1,2,3,Fast,\n\
KeyShift,BeatLoop+Shift,Button,9027,0,1,2,3,9027,0,1,2,3,Fast,\n\
PAD1_KeyShift,,Pad,9050,5,6,7,8,9050,5,6,7,8,Fast,\n\
PAD3_Keyboard,,Pad,9042,5,6,7,8,9042,5,6,7,8,Fast,\n\
JogTouch,JogTouch+Shift,JogTouch,9048,0,1,2,3,,,,,,RO,\n\
SlipStateForFlashing,SlipStateForFlashing,Indicator,,,,,,,9B24,9B25,9B26,9B27,RO,\n\
LoadedIndicator,LoadedIndicator,Indicator,,,,,,,9B00,9B01,9B02,9B03,RO,\n\
ChannelLevel,ChannelLevel,Indicator,,,,,,B002,0,1,2,3,RO,\n\
PAD9_PadFx2,,Pad,9068,5,6,7,8,9068,5,6,7,8,Fast,\n\
PAD1_HotCue,PAD1.HotCue.Call,Pad,9000,5,6,7,8,9000,5,6,7,8,Fast,\n\
PAD2_HotCue,PAD2.HotCue.Call,Pad,9001,5,6,7,8,9001,5,6,7,8,Fast,\n\
PAD1_HotCue+Shift,PAD1.HotCue.Delete,Pad,9008,5,6,7,8,9008,5,6,7,8,Fast,\n\
PAD5_BeatLoop,,Pad,9010,5,6,7,8,9010,5,6,7,8,Fast,\n\
PAD7_BeatLoop,,Pad,9012,5,6,7,8,9012,5,6,7,8,Fast,\n\
PAD1_BeatJump,PAD1.BeatJump,Pad,9030,5,6,7,8,9030,5,6,7,8,Fast,\n\
PAD4_BeatJump,PAD4.BeatJump,Pad,9033,5,6,7,8,9033,5,6,7,8,Fast,\n";

const SR: u32 = 44100;

struct Rig {
    ctl: Controller,
    mixer: Mixer,
}

impl Rig {
    fn new() -> Rig {
        let mut mixer = Mixer::new(SR);
        for (d, bpm) in [(0, 120.0), (1, 126.0)] {
            let pcm = Arc::new(Pcm::click_track(SR, bpm, 60.0));
            mixer.apply(Command::Load { deck: d, pcm, grid: Some(Grid { bpm, first_beat_s: 0.0 }) });
        }
        Rig { ctl: Controller::new(parse(MAP)), mixer }
    }

    /// Send MIDI bytes; returns the produced commands (also applied to the mixer).
    fn midi(&mut self, bytes: &[u8]) -> Vec<String> {
        let cmds = self.ctl.handle(&MidiMessage::from_bytes(bytes, 0.0).unwrap());
        let names = cmds.iter().map(|c| format!("{c:?}")).collect();
        for c in cmds {
            self.mixer.apply(c);
        }
        names
    }

    fn run(&mut self, s: f64) {
        self.mixer.render_offline(s);
    }

    fn pos(&self, d: usize) -> f64 {
        self.mixer.decks[d].position_s()
    }
}

#[test]
fn function_classification() {
    assert_eq!(target_of("PlayPause"), Some(Target::PlayPause));
    assert_eq!(target_of("PAD3.HotCue.Call"), Some(Target::HotCueCall(2)));
    assert_eq!(target_of("PAD8.HotCue.Delete"), Some(Target::HotCueDelete(7)));
    assert_eq!(target_of("PAD5_BeatLoop"), Some(Target::BeatLoopPad(0)), "first beat-loop pad is named PAD5");
    assert_eq!(target_of("PAD12_BeatLoop"), Some(Target::BeatLoopPad(7)));
    assert_eq!(target_of("PAD2.BeatJump"), Some(Target::BeatJumpPad(1)));
    assert_eq!(target_of("PAD9.HotCue.Call"), None, "only 8 hot cues");
    assert_eq!(target_of("JogPitchBend"), Some(Target::NoOp), "same message as JogScratch; the deck decides by touch");
    assert_eq!(target_of("EQHigh"), Some(Target::Eq(2)));
    assert_eq!(target_of("CFXParameterCH3"), Some(Target::ColorKnob(2)));
}

#[test]
fn coverage_lists_handled_and_missing() {
    let r = Rig::new();
    let (yes, no) = r.ctl.coverage();
    assert!(yes.contains("PlayPause") && yes.contains("PAD5_BeatLoop"));
    assert!(yes.contains("EQHigh") && yes.contains("CFX1On") && yes.contains("CFXParameterCH2"));
    assert!(yes.contains("JogPitchBend"), "deliberately mapped to no action");
}

#[test]
fn transport_and_cue() {
    let mut r = Rig::new();
    assert_eq!(r.midi(&[0x90, 0x00, 0x7F]), ["TogglePlay(0)"]);
    assert!(r.midi(&[0x90, 0x00, 0x00]).is_empty(), "release does nothing");
    r.run(1.0);
    assert!((r.pos(0) - 1.0).abs() < 0.01);
    assert_eq!(r.midi(&[0x90, 0x01, 0x7F]), ["CuePress(0)"], "playing: back to cue and stop");
    assert!(!r.mixer.decks[0].is_playing());
    assert_eq!(r.midi(&[0x91, 0x00, 0x7F]), ["TogglePlay(1)"], "channel 1 = deck 2");
    assert!(r.midi(&[0x92, 0x00, 0x7F]).is_empty(), "deck 3 does not exist in the engine");
}

#[test]
fn pads_hot_cues_loops_and_jumps() {
    let mut r = Rig::new();
    r.mixer.apply(Command::SeekSeconds(0, 4.0));
    assert_eq!(r.midi(&[0x95, 0x00, 0x7F]), ["HotCuePress(0, 0)"], "pads of deck 1 on channel 5");
    r.midi(&[0x95, 0x00, 0x00]);
    assert_eq!(r.mixer.decks[0].hot_cue(0), Some(4.0));
    assert!(r.midi(&[0x95, 0x00, 0x40]).is_empty(), "pad pressure alone does nothing");
    r.midi(&[0x95, 0x08, 0x7F]);
    assert_eq!(r.mixer.decks[0].hot_cue(0), None, "shift + pad deletes");
    // beat loop pad 3 (named PAD7) = 1 beat at 120 BPM
    r.midi(&[0x90, 0x00, 0x7F]);
    assert_eq!(r.midi(&[0x95, 0x12, 0x7F]), ["BeatLoop(0, 1.0)"]);
    r.run(2.0);
    assert!((4.0..4.5).contains(&r.pos(0)), "{}", r.pos(0));
    r.midi(&[0x90, 0x08, 0x7F]); // reloop/exit
    // beat jump: pad 1 = 1 beat back, pad 4 = 2 beats forward
    let p = r.pos(0);
    assert_eq!(r.midi(&[0x95, 0x30, 0x7F]), ["BeatJump(0, -1.0)"]);
    assert!((r.pos(0) - (p - 0.5)).abs() < 1e-6);
    assert_eq!(r.midi(&[0x95, 0x33, 0x7F]), ["BeatJump(0, 2.0)"]);
}

#[test]
fn faders_and_knobs() {
    let mut r = Rig::new();
    // 14-bit tempo: MSB alone does nothing, LSB completes the value
    assert!(r.midi(&[0xB0, 0x00, 0x7F]).is_empty());
    r.midi(&[0xB0, 0x20, 0x7F]);
    assert!((r.mixer.decks[0].tempo() - 1.16).abs() < 1e-3, "full up = +16 % (default range)");
    r.midi(&[0x90, 0x10, 0x7F]); // tempo range: 16 -> 100 %
    assert!((r.mixer.decks[0].tempo_range() - 1.0).abs() < 1e-9);
    r.midi(&[0xB4, 0x11, 0x00]); // channel fader deck 1 down
    assert_eq!(r.mixer.decks[0].volume, 0.0);
    r.midi(&[0xB4, 0x01, 64]); // gain centre = 0 dB
    assert!((r.mixer.decks[0].trim - 1.0).abs() < 0.01);
    r.midi(&[0xB4, 0x0B, 0x7F]);
    assert!((r.mixer.state().crossfader - 1.0).abs() < 1e-6);
    r.midi(&[0xB1, 0x1C, 64]); // needle: middle of deck 2
    assert!((r.pos(1) - 30.0).abs() < 0.1, "{}", r.pos(1));
    assert_eq!(r.midi(&[0xB4, 0x09, 0x00]), ["Eq(1, 0, 0.0)"], "EQ low of deck 2 (full hex message per deck)");
    assert_eq!(r.mixer.state().eq[1], [0.0, 0.5, 0.5]);
}

#[test]
fn jog_scratch_bend_search_and_vinyl_mode() {
    let mut r = Rig::new();
    r.mixer.apply(Command::SeekSeconds(0, 10.0));
    assert_eq!(r.midi(&[0x90, 0x20, 0x7F]), ["JogTouch(0, true)"]);
    // JogScratch and JogPitchBend share B022; only one command comes out
    assert_eq!(r.midi(&[0xB0, 0x22, 0x50]), ["Jog(0, 16)"]);
    r.run(0.5);
    assert!(r.pos(0) > 10.0, "scratched forward");
    r.midi(&[0x90, 0x20, 0x00]);
    // vinyl mode off: touching the jog is ignored
    r.midi(&[0x90, 0x12, 0x7F]);
    assert!(r.midi(&[0x90, 0x20, 0x7F]).is_empty());
    // search wheel
    let p = r.pos(0);
    assert_eq!(r.midi(&[0xB0, 0x26, 0x4A]), ["SeekBy(0, 0.5)"]);
    assert!((r.pos(0) - (p + 0.5)).abs() < 1e-6);
}

#[test]
fn sync_quantize_and_leds() {
    let mut r = Rig::new();
    let led = |r: &Rig, status: u8, d1: u8| {
        r.ctl.leds(&r.mixer.state()).into_iter().find(|l| l.status == status && l.data1 == d1).map(|l| l.value)
    };
    assert_eq!(led(&r, 0x90, 0x00), Some(0), "play LED off");
    assert_eq!(led(&r, 0x90, 0x01), Some(0x7F), "cue LED: paused on the cue point");
    r.midi(&[0x90, 0x00, 0x7F]);
    r.run(0.1);
    assert_eq!(led(&r, 0x90, 0x00), Some(0x7F));
    assert_eq!(led(&r, 0x91, 0x1F), Some(0), "deck 2 not synced");
    r.midi(&[0x91, 0x1F, 0x7F]);
    assert!((r.mixer.decks[1].bpm().unwrap() - 120.0).abs() < 1e-9);
    assert_eq!(led(&r, 0x91, 0x1F), Some(0x7F), "sync LED on when tempos match");
    r.midi(&[0x90, 0x09, 0x7F]);
    assert!(r.mixer.decks[0].quantize);
    assert_eq!(led(&r, 0x90, 0x09), Some(0x7F));
    assert_eq!(led(&r, 0x90, 0x12), Some(0x7F), "vinyl mode on by default");
    r.mixer.apply(Command::HotCuePress(0, 1));
    assert_eq!(led(&r, 0x95, 0x01), Some(0x7F), "pad LED of a set hot cue");
    assert_eq!(led(&r, 0x95, 0x00), Some(0));
    let l = r.ctl.leds(&r.mixer.state()).into_iter().find(|l| l.status == 0x95 && l.data1 == 0x01).unwrap();
    assert_eq!(l.packed(), 0x007F_0195);
}

#[test]
fn sound_color_fx() {
    use rbx_engine::dsp::ColorFx;
    let mut r = Rig::new();
    assert_eq!(r.midi(&[0xB4, 0x57, 0x7F]), ["ColorFxSelect(Some(Filter))"], "function CFX1On = FILTER");
    assert_eq!(r.midi(&[0xB4, 0x69, 0x7F]), ["ColorFxSelect(Some(Space))"], "switching type");
    assert_eq!(r.midi(&[0xB4, 0x69, 0x7F]), ["ColorFxSelect(None)"], "same button again = off");
    r.midi(&[0xB4, 0x57, 0x7F]);
    assert_eq!(r.mixer.state().color_fx, Some(ColorFx::Filter));
    assert_eq!(r.midi(&[0xB4, 0x0A, 0x7F]), ["ColorKnob(1, 1.0)"], "CH2 knob -> deck 2");
    assert_eq!(r.midi(&[0xB4, 0x6C, 0x7F]), ["ColorSub(1.0)"]);
    let leds = r.ctl.leds(&r.mixer.state());
    let led = |d1: u8| leds.iter().find(|l| l.status == 0xB4 && l.data1 == d1).map(|l| l.value);
    assert_eq!((led(0x57), led(0x69)), (Some(0x7F), Some(0)), "CFX button LEDs");
}

#[test]
fn beat_fx_section_and_pad_fx() {
    use rbx_engine::fx::{BeatFxKind, FxAssign};
    let mut r = Rig::new();
    assert_eq!(r.midi(&[0xB4, 0x37, 0x7F]), ["BeatFxSelect(Echo)"]);
    assert_eq!(r.midi(&[0xB4, 0x34, 0x7F]), ["BeatFxSelect(PingPong)"], "shift + on cycles the type");
    assert_eq!(r.midi(&[0xB4, 0x72, 0x7F]), ["BeatFxOn(true)"]);
    assert_eq!(r.midi(&[0xB4, 0x5B, 0x7F]), ["BeatFxDepth(1.0)"]);
    assert_eq!(r.midi(&[0xB4, 0x4D, 0x7F]), ["BeatFxBeat(1)"]);
    assert_eq!(r.midi(&[0xB4, 0x27, 0x7F]), ["BeatFxAssign(Channel(0))"], "row without function uses its name");
    assert_eq!(r.midi(&[0xB4, 0x2D, 0x02]), ["BeatFxAdjust(2.0)"], "adjust time rotary");
    // tap at 0.5 s intervals = 120 BPM
    for t in [1.0, 1.5] {
        r.ctl.handle(&MidiMessage::from_bytes(&[0xB4, 0x4E, 0x7F], t).unwrap()).into_iter().for_each(|c| r.mixer.apply(c));
    }
    let s = r.mixer.state().beat_fx;
    assert_eq!((s.on, s.kind, s.assign, s.bpm, s.auto_bpm), (true, BeatFxKind::PingPong, FxAssign::Channel(0), 120.0, false));
    let leds = r.ctl.leds(&r.mixer.state());
    let led = |d1: u8| leds.iter().find(|l| l.status == 0xB4 && l.data1 == d1).map(|l| l.value);
    assert_eq!((led(0x72), led(0x4E), led(0x7E)), (Some(0x7F), Some(0x7F), Some(0)), "on, tap, auto LEDs");
    assert_eq!(r.midi(&[0xB4, 0x7E, 0x7F]), ["BeatFxBpm(None)"], "shift + tap = auto beat");
    // pad FX: bank 1 slot 1 on deck 1, bank 2 slot 9 (named PAD9_PadFx2) on deck 2
    assert_eq!(r.midi(&[0x95, 0x20, 0x7F]), ["PadFx(0, Some((0, 0)))"]);
    assert_eq!(r.ctl.leds(&r.mixer.state()).iter().find(|l| l.status == 0x95 && l.data1 == 0x20).map(|l| l.value), Some(0x7F));
    assert_eq!(r.midi(&[0x95, 0x20, 0x00]), ["PadFx(0, None)"]);
    assert_eq!(r.midi(&[0x96, 0x68, 0x7F]), ["PadFx(1, Some((1, 8)))"]);
}

#[test]
fn deck_modes_memory_and_indicators() {
    use rbx_engine::controller::PadMode;
    let mut r = Rig::new();
    assert_eq!(r.midi(&[0x90, 0x21, 0x7F]), ["Reverse(0, true)"]);
    assert_eq!(r.midi(&[0x90, 0x21, 0x7F]), ["Reverse(0, false)"], "toggle");
    assert_eq!(r.midi(&[0x90, 0x2C, 0x7F]), ["Slip(0, true)"]);
    assert_eq!(r.midi(&[0x90, 0x11, 0x7F]), ["MasterTempo(0, true)"]);
    assert_eq!(r.midi(&[0xB0, 0x1E, 0x7F]), ["Brake(0, 3.0)"]);
    r.midi(&[0xB0, 0x1E, 0x00]);
    assert_eq!(r.midi(&[0x90, 0x0A, 0x7F]), ["MemoryStore(0)"]);
    assert_eq!(r.midi(&[0x90, 0x0B, 0x7F]), ["MemoryCall(0, true)"]);
    assert_eq!(r.midi(&[0x90, 0x0D, 0x7F]), ["MemoryDelete(0)"]);
    assert!(r.midi(&[0x90, 0x0E, 0x7F]).is_empty(), "time mode is display state only");
    assert!(r.ctl.time_remaining(0));
    assert_eq!(r.midi(&[0x90, 0x0F, 0x7F]), ["AutoCue(0, true)"]);
    assert_eq!(r.midi(&[0x90, 0x45, 0x7F]), ["BeatLoop(0, 4.0)"]);
    assert_eq!(r.midi(&[0x91, 0x1E, 0x7F]), ["SetMaster(Some(1))"]);
    assert_eq!(r.midi(&[0x91, 0x1E, 0x7F]), ["SetMaster(None)"], "toggle off");
    assert_eq!(r.midi(&[0x91, 0x47, 0x7F]), ["InstantDouble(1)"]);
    assert_eq!(r.midi(&[0x90, 0x02, 0x7F]), ["Search(0, 4.0)"]);
    assert_eq!(r.midi(&[0x90, 0x02, 0x00]), ["Search(0, 0.0)"]);
    assert!(r.midi(&[0x90, 0x48, 0x7F]).is_empty(), "JogTouch+Shift: deliberately no action");
    // pad modes, key shift and keyboard pads
    assert!(r.midi(&[0x90, 0x27, 0x7F]).is_empty());
    assert_eq!(r.ctl.pad_mode(0), PadMode::KeyShift);
    assert_eq!(r.midi(&[0x95, 0x50, 0x7F]), ["Key(0, 1)"]);
    assert_eq!(r.midi(&[0x95, 0x50, 0x7F]), ["Key(0, 0)"], "same pad again resets");
    r.midi(&[0x90, 0x26, 0x7F]);
    assert_eq!(r.ctl.pad_mode(0), PadMode::Keyboard);
    assert_eq!(r.midi(&[0x95, 0x42, 0x7F]), ["KeyboardPress(0, 2)"]);
    assert_eq!(r.midi(&[0x95, 0x42, 0x00]), ["KeyboardRelease(0)"]);
    // LEDs and indicators
    r.mixer.apply(Command::Play(0));
    r.run(0.2);
    let leds = r.ctl.leds(&r.mixer.state());
    let led = |st: u8, d1: u8| leds.iter().find(|l| l.status == st && l.data1 == d1).map(|l| l.value);
    assert_eq!(led(0x90, 0x2C), Some(0x7F), "slip LED");
    assert_eq!(led(0x90, 0x11), Some(0x7F), "master tempo LED");
    assert_eq!(led(0x90, 0x26), Some(0x7F), "keyboard mode LED");
    assert_eq!(led(0x90, 0x27), Some(0), "key shift mode LED off");
    assert_eq!(led(0x95, 0x42), Some(0x7F), "keyboard pad of the current key");
    assert_eq!(led(0x9B, 0x00), Some(0x7F), "loaded indicator deck 1");
    assert!(led(0xB0, 0x02).is_some(), "channel level meter is driven");
    let (yes, no) = r.ctl.coverage();
    assert!(yes.contains("JogTouch+Shift") && yes.contains("Sync+LongPress"));
    assert!(no.is_empty(), "every input row of the test mapping is handled: {no:?}");
}
