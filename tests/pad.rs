//! Pad file tests on synthetic data (SPEC §5g).

use rekordbox_formats::pad::{parse, parse_modes, write, Pad};

#[test]
fn parses_lines_and_skips_comments() {
    let pads = parse("function=HotCue,value=A\r\n\r\n# comment\nfunction=Sampler,value=2\n");
    assert_eq!(pads.len(), 2);
    assert_eq!((pads[0].function(), pads[0].value()), ("HotCue", "A"));
    assert_eq!((pads[1].function(), pads[1].value()), ("Sampler", "2"));
}

#[test]
fn key_value_rules() {
    let p = Pad::parse_line("function=BeatLoop,value=1/64,flag,value=1/32,text=a=b");
    assert_eq!(p.value(), "1/32", "later duplicates overwrite");
    assert_eq!(p.get("flag"), Some(""), "part without '=' -> key with empty value");
    assert_eq!(p.get("text"), Some("a=b"), "value is everything after the first '='");
}

#[test]
fn user_variant_replaces_function_and_value() {
    let mut p = Pad::parse_line("function=BeatJump,value=-1,userfunction=UserBeatJump,uservalue=-1");
    assert!(p.apply_user_variant());
    assert_eq!((p.function(), p.value()), ("UserBeatJump", "-1"));
    let mut q = Pad::parse_line("function=BeatJump,value=-1,userfunction=UserBeatJump,uservalue=");
    assert!(!q.apply_user_variant(), "both user fields must be non-empty");
    assert_eq!(q.function(), "BeatJump");
}

#[test]
fn mode_list() {
    let modes = parse_modes(
        "function=,type=separator,mode=off,template=off,pad=off,text=[PRESET MODE],select=\n\
         function=HotCue,type=preset,mode=on,template=on,pad=on,text=HOT CUE,select=1\n",
    );
    assert_eq!(modes[0].kind, "separator");
    assert_eq!(modes[0].pad_file_name(), None);
    assert_eq!(modes[1].select, Some(1));
    assert!(modes[1].mode && modes[1].template && modes[1].pad);
    assert_eq!(modes[1].pad_file_name().as_deref(), Some("HotCue.pad.csv"));
}

#[test]
fn write_round_trip() {
    let text = "function=HotCue,value=A\nfunction=BeatJump,value=+1,userfunction=UserBeatJump,uservalue=+1\n";
    assert_eq!(write(&parse(text)), text);
}
