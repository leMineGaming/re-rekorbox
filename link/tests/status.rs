//! Deck-status decode (type 0x0A) against the idle-capture fixture and a synthetic playing packet.

mod fixtures;

use fixtures::{bytes, XDJ_50002_0A};
use rbx_link::status::DeckStatus;

#[test]
fn idle_deck_status_from_capture() {
    let s = DeckStatus::parse(&bytes(XDJ_50002_0A)).unwrap();
    assert_eq!(s.device, 0x0C, "the fixture is deck 12");
    assert_eq!(s.slot, 0, "no media");
    assert!(!s.has_track());
    assert_eq!(s.track_id, None, "no track id without a slot");
    assert_eq!(s.bpm_x100, None, "0xFFFF means no track");
    assert_eq!(s.bpm(), None);
    assert_eq!(s.beat, None, "0xFFFFFFFF means no beat");
    assert_eq!(s.beat_in_bar, 0);
    assert_eq!(s.hw_class, 0x1F, "XDJ-RX3 hardware class");
    // pitch field is nominal → 0 %
    assert!(s.pitch_percent().abs() < 0.001, "{}", s.pitch_percent());
}

#[test]
fn playing_deck_status_decodes_bpm_pitch_beat() {
    // take the idle packet and set the fields a running track would carry
    let mut b = bytes(XDJ_50002_0A);
    b[0x29] = 4; // slot 4 = rekordbox
    b[0x2A] = 1; // rekordbox track
    b[0x2C..0x30].copy_from_slice(&0x1234u32.to_be_bytes()); // track id
    b[0x7B] = 0x6f; // some play-state code
    b[0x8C..0x90].copy_from_slice(&0x0011_0000u32.to_be_bytes()); // pitch +6.25 %
    b[0x90] = 0x7F;
    b[0x91] = 0xFF; // validity stays 0x7FFF
    b[0x92..0x94].copy_from_slice(&13000u16.to_be_bytes()); // 130.00 BPM
    b[0xA0..0xA4].copy_from_slice(&42u32.to_be_bytes()); // beat 42
    b[0xA6] = 2; // second beat in the bar

    let s = DeckStatus::parse(&b).unwrap();
    assert!(s.has_track());
    assert_eq!(s.track_id, Some(0x1234));
    assert_eq!(s.play_state, 0x6f);
    assert_eq!(s.bpm_x100, Some(13000));
    assert_eq!(s.bpm(), Some(130.0));
    assert_eq!(s.beat, Some(42));
    assert_eq!(s.beat_in_bar, 2);
    // pitch 0x106666 / 0x100000 − 1 = +6.25 %; effective BPM = 130 × 1.0625
    assert!(
        (s.pitch_percent() - 6.25).abs() < 0.01,
        "{}",
        s.pitch_percent()
    );
    assert!((s.effective_bpm().unwrap() - 138.125).abs() < 0.01);
}

#[test]
fn bad_input_is_an_error() {
    assert!(DeckStatus::parse(&[]).is_err());
    assert!(DeckStatus::parse(b"Qspt1WmJOL").is_err());
    let mut short = bytes(XDJ_50002_0A);
    short.truncate(0x80);
    assert!(DeckStatus::parse(&short).is_err());
}
