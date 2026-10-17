//! MIDI mapping CSV tests on synthetic data. Rules come from SPEC.md §5f.

use rekordbox_formats::midi::{control_type_code, hex_value, int_value, parse, write, Binding, Options};

const SAMPLE: &str = "\u{feff}@file,1,TEST-CTRL\r\n\
#name,function,type,input,deck1,deck2,deck3,deck4,output,deck1,deck2,deck3,deck4,option,comment\r\n\
,,,,,,,,,,,,,,\r\n\
# Browser,,,,,,,,,,,,,,\r\n\
Browse,Browse,Rotary,B640,,,,,,,,,,,Browse\r\n\
Load,Browse+Press,Button,,9646,9647,9648,9649,,,,,,,Load\r\n\
Tempo,TempoSlider,KnobSliderHiRes,B000,0,1,2,3,,,,,,Fast,Tempo\r\n\
Cue,PAD1,Pad,9000,7,9,11,13,9000,7,9,11,13,Fast;Dual,Hot cue\r\n\
Level,ChannelLevel,Indicator,,,,,,B002,0,1,2,3,RO;Priority=50,Level\r\n\
Jog,JogIndicator,JogIndicator,,,,,,,BB00,BB01,BB02,BB03,RO;Min=1;Max=72;Priority=100,Jog\r\n\
Stamp,JogTimestamp,JogTimestamp,E0,0,1,,,,,,,,RO,Timestamp\r\n\
#Disabled,Thing,Button,9000,,,,,,,,,,,off\r\n\
Short,Row,Button\r\n\
@Special,Row,Button,9000,,,,,,,,,,,\r\n";

#[test]
fn parses_structure_and_skips_like_the_loader() {
    let m = parse(SAMPLE);
    assert!(m.report.had_bom);
    assert!(m.is_valid());
    assert_eq!((m.version, m.device.as_str()), (1, "TEST-CTRL"));
    assert_eq!(m.entries.len(), 7);
    assert_eq!(m.report.comment_lines, 3, "header, section comment, disabled row");
    assert_eq!(m.report.empty_name, 1, "',,,,' row has an empty name");
    assert_eq!(m.report.short_rows, 1);
    assert_eq!(m.report.special_rows, 1, "names containing '@' are not handled here");
    assert_eq!(m.entries[0].name, "Browse");
    assert_eq!(m.entries[0].line, 5);
}

#[test]
fn version_other_than_one_disables_rows() {
    let m = parse("@file,2,X\nBrowse,Browse,Rotary,B640,,,,,,,,,,,c\n");
    assert!(!m.is_valid());
    assert!(m.entries.is_empty());
    assert_eq!(m.report.rows_before_version, 1);
    let m = parse("Browse,Browse,Rotary,B640,,,,,,,,,,,c\n");
    assert!(!m.is_valid(), "no @file line at all");
}

#[test]
fn file_line_is_detected_by_substring() {
    // a BOM (or anything) before "@file" still makes it the header line
    let m = parse("xx@file,1,DEV\nBrowse,Browse,Rotary,B640,,,,,,,,,,,c\n");
    assert_eq!((m.version, m.device.as_str(), m.entries.len()), (1, "DEV", 1));
    // quirk: a data row containing "@file" is taken as a header line
    let m = parse("@file,1,DEV\nA,B,Button,9000,,,,,,,,,,,see @file docs\nC,D,Button,9001,,,,,,,,,,,x\n");
    assert_eq!(m.entries.len(), 0, "the second line reset the version to 0 (field 1 = 'B')");
}

#[test]
fn number_parsing_rules() {
    assert_eq!(hex_value("B000"), 0xB000);
    assert_eq!(hex_value("b0 0x1"), 0xB001, "non-hex characters are skipped: b,0,0,1 (x skipped)");
    assert_eq!(hex_value(""), 0);
    assert_eq!(int_value(" 12abc"), 12);
    assert_eq!(int_value("-3"), -3);
    assert_eq!(int_value("x7"), 0);
    assert_eq!(int_value("99999999999"), i32::MAX as i64);
}

#[test]
fn options_parsing() {
    let o = Options::parse("RO;Min=1;Max=72;Priority=100;Unknown=5");
    assert!(o.read_only());
    assert!(!o.fast());
    assert_eq!((o.min(), o.max(), o.priority()), (Some(1), Some(72), 100));
    assert_eq!(o.value(), None, "absent Value is -1 in the loader");
    assert_eq!(o.get("Unknown"), None, "unknown keys are ignored");
    let o = Options::parse("Fast;Dual;Blink=600;Value=");
    assert!(o.fast() && o.dual());
    assert_eq!(o.blink(), 600);
    assert_eq!(o.value(), Some(1), "key with empty value counts as 1");
    assert_eq!(Options::parse("Fast;Blink=600").to_text(), "Fast;Blink=600");
}

#[test]
fn bindings_hex_and_channel_rules() {
    let m = parse(SAMPLE);
    let dev = m.device.as_str();
    let e = |name: &str| m.entries.iter().find(|e| e.name == name).unwrap();
    let b = |deck, message, partner_message| Binding { deck, message, partner_message };

    // no deck columns -> one global binding
    assert_eq!(e("Browse").input_bindings(dev), vec![b(0, 0xB640, 0xB640)]);
    // empty Input -> deck columns are full hex messages
    assert_eq!(
        e("Load").input_bindings(dev),
        vec![b(1, 0x9646, 0x9646), b(2, 0x9647, 0x9647), b(3, 0x9648, 0x9648), b(4, 0x9649, 0x9649)]
    );
    // Input set -> deck columns are channels: (k << 8) | Input
    assert_eq!(e("Tempo").input_bindings(dev)[3], b(4, 0xB300, 0xB300));
    // Dual pairs deck 1<->3 and 2<->4
    assert_eq!(
        e("Cue").input_bindings(dev),
        vec![b(1, 0x9700, 0x9B00), b(2, 0x9900, 0x9D00), b(3, 0x9B00, 0x9700), b(4, 0x9D00, 0x9900)]
    );
    assert_eq!(e("Cue").output_bindings()[0], b(1, 0x9700, 0x9B00));
    // output-only indicator with channels
    assert!(e("Level").input_bindings(dev).is_empty());
    assert_eq!(e("Level").output_bindings()[2], b(3, 0xB202, 0xB202));
    // output deck columns as full hex messages
    assert_eq!(e("Jog").output_bindings()[1], b(2, 0xBB01, 0xBB01));
    // JogTimestamp: k | Input without shifting
    assert_eq!(e("Stamp").input_bindings(dev), vec![b(1, 0x00E0, 0x00E0), b(2, 0x00E1, 0x00E1)]);
}

#[test]
fn control_type_codes() {
    assert_eq!(control_type_code("Button", "X"), 0);
    assert_eq!(control_type_code("Toggle", "X"), 0x1b);
    assert_eq!(control_type_code("KnobSliderHiRes", "DDJ-400"), 7);
    assert_eq!(control_type_code("KnobSliderHiRes", "PIONEER DDJ-SX2"), 8);
    assert_eq!(control_type_code("JogTimestamp", "X"), 0x19);
    assert_eq!(control_type_code("Pad", "DDJ-RZX"), 0x1d);
    assert_eq!(control_type_code("Pad", "DDJ-1000"), 0x1e);
    assert_eq!(control_type_code("Pad", "PLX-CRSS12"), 0x1e);
    assert_eq!(control_type_code("Pad", "DDJ-SZ"), 0x1f);
    assert_eq!(control_type_code("Pad", "Pioneer DJ DJM-S9"), 0x20);
    assert_eq!(control_type_code("Pad", "DDJ-FLX4"), 0x21, "default pad type");
    assert_eq!(control_type_code("NoSuchType", "X"), 0, "unknown -> Button");
    assert_eq!(control_type_code("", "X"), 0);
}

#[test]
fn write_parse_round_trip() {
    let m = parse(SAMPLE);
    let text = write(&m).unwrap();
    assert!(text.starts_with("@file,1,TEST-CTRL\r\n#name,function,type,"));
    let back = parse(&text);
    assert_eq!(back.entries.len(), m.entries.len());
    for (a, b) in m.entries.iter().zip(&back.entries) {
        let (mut a, mut b) = (a.clone(), b.clone());
        a.line = 0;
        b.line = 0;
        assert_eq!(a, b);
    }
    let mut bad = m.clone();
    bad.entries[0].comment = "a,b".into();
    assert!(write(&bad).is_err(), "the format cannot store commas");
}
