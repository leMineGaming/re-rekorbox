//! ANLZ tests on synthetic data. Expected bytes and rules come from SPEC.md §5a–§5d.

use rekordbox_formats::anlz::phrase::{phrase_label, Mood, Phrase, SongStructure};
use rekordbox_formats::anlz::tags::{
    decode_path, encode_path, Beat, BeatGrid, CueList, CueListType, CuePoint, ExtBeatGrid, Vbr2,
};
use rekordbox_formats::anlz::threex::{self, Value};
use rekordbox_formats::anlz::waveform::{ColorColumn, MonoColumn, WavePreview, Waveform};
use rekordbox_formats::anlz::{AnlzError, AnlzFile, FourCC, RawTag, Tag};

fn be32(b: &[u8], off: usize) -> u32 {
    u32::from_be_bytes(b[off..off + 4].try_into().unwrap())
}

fn be16(b: &[u8], off: usize) -> u16 {
    u16::from_be_bytes(b[off..off + 2].try_into().unwrap())
}

fn sample_beats(n: usize, tempo_x100: u16) -> Vec<Beat> {
    let step = 6_000_000.0 / tempo_x100 as f64;
    (0..n)
        .map(|i| Beat { beat: (i % 4) as u16 + 1, tempo_x100, time_ms: (3.0 + i as f64 * step).round() as u32 })
        .collect()
}

/// A synthetic .DAT-like file: PPTH, PVBR(raw), PQTZ, PWAV, PWV2, PCOB, PCOB.
fn sample_dat() -> AnlzFile {
    let mut f = AnlzFile::new();
    f.push(&Tag::Path("?/Test Artist - Test Track.flac".into()));
    f.tags.push(RawTag::new(FourCC::PVBR, vec![0; 4], vec![0; 1604]));
    f.push(&Tag::BeatGrid(BeatGrid::new(sample_beats(16, 12800))));
    f.push(&Tag::WavePreview(WavePreview::new(FourCC::PWAV, (0..400).map(|i| (i % 256) as u8).collect())));
    f.push(&Tag::WavePreview(WavePreview::new(FourCC::PWV2, (0..100).map(|i| (i % 16) as u8).collect())));
    f.push(&Tag::CueList(CueList::new(CueListType::HotCue)));
    f.push(&Tag::CueList(CueList::new(CueListType::Memory)));
    f
}

// ---------------------------------------------------------------------------------------------
// Container (§5a, §5c)

#[test]
fn pmai_header_bytes_match_spec() {
    let bytes = sample_dat().to_bytes();
    assert_eq!(&bytes[0..4], b"PMAI");
    assert_eq!(be32(&bytes, 4), 28, "header_len");
    assert_eq!(be32(&bytes, 8) as usize, bytes.len(), "total_len = file size");
    // the 16 constant bytes observed in all files: 00000001 00010000 00010000 00000000
    assert_eq!(&bytes[12..28], &[0, 0, 0, 1, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[28..32], b"PPTH", "first tag follows the 28-byte header");
}

#[test]
fn container_round_trip_is_byte_exact() {
    let bytes = sample_dat().to_bytes();
    let parsed = AnlzFile::parse(&bytes).unwrap();
    assert_eq!(parsed.tags.len(), 7);
    assert_eq!(parsed.to_bytes(), bytes);
    parsed.check_reader_order().unwrap();
    let order: Vec<String> = parsed.tags.iter().map(|t| t.fourcc.to_string()).collect();
    assert_eq!(order, ["PPTH", "PVBR", "PQTZ", "PWAV", "PWV2", "PCOB", "PCOB"]);
    for t in &parsed.tags {
        assert!(t.typed_round_trip_ok().unwrap(), "typed round trip of {}", t.fourcc);
    }
}

#[test]
fn tag_lengths_follow_header_rules() {
    let bytes = sample_dat().to_bytes();
    let f = AnlzFile::parse(&bytes).unwrap();
    let pqtz = f.find(FourCC::PQTZ).unwrap();
    assert_eq!(pqtz.header_len(), 24);
    assert_eq!(pqtz.total_len(), 24 + 16 * 8);
    assert_eq!(f.find(FourCC::PWAV).unwrap().header_len(), 20);
    assert_eq!(f.find(FourCC::PCOB).unwrap().total_len(), 24);
}

#[test]
fn unknown_tags_are_preserved() {
    let mut f = sample_dat();
    f.tags.insert(2, RawTag::new(FourCC(*b"PXYZ"), vec![1, 2, 3, 4], vec![9; 10]));
    let bytes = f.to_bytes();
    let back = AnlzFile::parse(&bytes).unwrap();
    assert_eq!(back.tags[2].fourcc, FourCC(*b"PXYZ"));
    assert!(matches!(back.tags[2].decode().unwrap(), Tag::Raw(_)));
    assert_eq!(back.to_bytes(), bytes);
}

#[test]
fn trailing_bytes_shorter_than_a_tag_header_are_ignored_and_kept() {
    let mut bytes = sample_dat().to_bytes();
    bytes.extend_from_slice(&[0xAA; 5]);
    let len = bytes.len() as u32;
    bytes[8..12].copy_from_slice(&len.to_be_bytes());
    let f = AnlzFile::parse(&bytes).unwrap();
    assert_eq!(f.trailing, vec![0xAA; 5]);
    assert_eq!(f.to_bytes(), bytes);
}

#[test]
fn container_errors() {
    let good = sample_dat().to_bytes();

    let mut bad_magic = good.clone();
    bad_magic[0] = b'X';
    assert!(matches!(AnlzFile::parse(&bad_magic), Err(AnlzError::BadMagic { .. })));

    let mut bad_total = good.clone();
    bad_total[11] ^= 1;
    assert!(matches!(AnlzFile::parse(&bad_total), Err(AnlzError::BadLength(_))));

    // first tag: total_len = 0 -> error
    let mut zero_tl = good.clone();
    zero_tl[28 + 8..28 + 12].copy_from_slice(&0u32.to_be_bytes());
    assert!(matches!(AnlzFile::parse(&zero_tl), Err(AnlzError::BadLength(_))));

    // first tag runs past the end of the file
    let mut overrun = good.clone();
    overrun[28 + 8..28 + 12].copy_from_slice(&(good.len() as u32).to_be_bytes());
    assert!(matches!(AnlzFile::parse(&overrun), Err(AnlzError::BadLength(_))));

    assert!(matches!(AnlzFile::parse(&good[..20]), Err(AnlzError::TooShort { .. })));
}

// ---------------------------------------------------------------------------------------------
// PPTH, PQTZ (§5b)

#[test]
fn path_is_utf16be_nul_terminated() {
    let raw = encode_path("?/Musik/Ärger – Ω.mp3");
    let h = raw.full_header();
    assert_eq!(be16(&h, 0x0E) as usize, raw.body.len(), "u16@0E = byte length");
    assert_eq!(&raw.body[raw.body.len() - 2..], &[0, 0], "NUL terminated");
    assert_eq!(&raw.body[0..4], &[0x00, b'?', 0x00, b'/'], "UTF-16BE");
    assert_eq!(decode_path(&raw).unwrap(), "?/Musik/Ärger – Ω.mp3");
}

#[test]
fn beat_grid_layout() {
    let grid = BeatGrid::new(sample_beats(4, 14898));
    let raw = grid.to_raw();
    let h = raw.full_header();
    assert_eq!(be16(&h, 0x10), 8, "entry size default 8");
    assert_eq!(be32(&h, 0x14), 4, "count");
    assert_eq!(&raw.body[0..8], &[0, 1, 0x3a, 0x32, 0, 0, 0, 3], "beat 1, 148.98 BPM, 3 ms");
    let back = BeatGrid::from_raw(&raw).unwrap();
    assert_eq!(back, grid);
    assert!((back.beats[0].bpm() - 148.98).abs() < 1e-9);
    // spacing of 403 ms at 148.98 BPM (60000 / 148.98 = 402.7)
    assert_eq!(back.beats[1].time_ms - back.beats[0].time_ms, 403);
}

#[test]
fn beat_grid_too_short_body_is_an_error() {
    let mut raw = BeatGrid::new(sample_beats(4, 12000)).to_raw();
    raw.body.truncate(20);
    assert!(matches!(BeatGrid::from_raw(&raw), Err(AnlzError::TooShort { .. })));
}

#[test]
fn vbr_table_and_extra_length_rules() {
    use rekordbox_formats::anlz::tags::Vbr;
    let mut table = [0u32; 400];
    for (i, v) in table.iter_mut().enumerate() {
        *v = i as u32 * 1000;
    }
    let v = Vbr::new(&table, 0xDEAD_BEEF);
    let raw = v.to_raw();
    assert_eq!(raw.header_len(), 16);
    assert_eq!(raw.total_len(), 16 + 0x644);
    let back = Vbr::from_raw(&raw).unwrap();
    assert_eq!(back.table()[399], 399_000);
    assert_eq!(back.extra(), 0xDEAD_BEEF);
    let only_table = Vbr { data: v.data[..0x640].to_vec(), ..v.clone() };
    assert_eq!((only_table.table()[1], only_table.extra()), (1000, 0));
    let only_extra = Vbr { data: vec![0, 0, 0, 7], ..v.clone() };
    assert_eq!((only_extra.table()[1], only_extra.extra()), (0, 7));
    let odd = Vbr { data: vec![1; 100], ..v };
    assert_eq!((odd.table()[0], odd.extra()), (0, 0), "other lengths: zeros");
}

// ---------------------------------------------------------------------------------------------
// Waveforms (§5b, §5d)

#[test]
fn preview_requires_exact_length_otherwise_zeroes() {
    let ok = WavePreview::new(FourCC::PWAV, vec![0xE5; 400]);
    assert_eq!(ok.normalized(), vec![0xE5; 400]);
    let col = ok.columns()[0];
    assert_eq!((col.value, col.height), (7, 5));

    let short = WavePreview::new(FourCC::PWAV, vec![0xFF; 399]);
    assert_eq!(short.normalized(), vec![0; 400]);
    let pwv2 = WavePreview::new(FourCC::PWV2, vec![7; 100]);
    assert_eq!(pwv2.normalized().len(), 100);

    let raw = ok.to_raw();
    let h = raw.full_header();
    assert_eq!(raw.header_len(), 20);
    assert_eq!(be16(&h, 0x0E), 400);
    assert_eq!(be16(&h, 0x10), 1);
}

#[test]
fn mono_byte_layout() {
    for value in 0..8u8 {
        for height in 0..32u8 {
            let c = MonoColumn { value, height };
            let b = c.to_byte();
            assert_eq!(b, value * 32 + height, "dst = value·0x20 + height");
            assert_eq!(MonoColumn::from_byte(b), c);
        }
    }
}

#[test]
fn pwv5_bit_layout() {
    let c = ColorColumn { c0: 5, c1: 5, c2: 3, height: 0 };
    assert_eq!(c.to_u16(), 0xB580, "observed sample value");
    let c = ColorColumn { c0: 0, c1: 5, c2: 5, height: 7 };
    assert_eq!(c.to_u16(), 0x169C, "observed sample value");
    for v in [0u16, 0xFFFC, 0x8004, 0x1234 & !3] {
        let c = ColorColumn::from_u16(v);
        assert_eq!(c.to_u16(), v);
        assert_eq!(c.to_u16() & 3, 0, "bits 1-0 always zero");
    }
}

#[test]
fn sized_waveforms_round_trip_and_headers() {
    let cols: Vec<ColorColumn> = (0..300).map(|i| ColorColumn { c0: i as u8 % 8, c1: 3, c2: 7, height: i as u8 % 32 }).collect();
    let pwv5 = Waveform::color_detail(&cols);
    let raw = pwv5.to_raw();
    let h = raw.full_header();
    assert_eq!(raw.header_len(), 0x18);
    assert_eq!(be16(&h, 0x0E), 2);
    assert_eq!(be32(&h, 0x10), 300);
    assert_eq!(&h[0x14..0x18], &[0x00, 0x96, 0x03, 0x05], "u16@14 = 150, u8@16 = 3, u8@17 = 5");
    let back = Waveform::from_raw(&raw).unwrap();
    assert_eq!(back.color_columns().unwrap(), cols);
    assert_eq!(back.entries_per_second(), Some(150));

    let mono: Vec<MonoColumn> = (0..50).map(|i| MonoColumn { value: 6, height: i as u8 % 32 }).collect();
    let pwv3 = Waveform::mono_detail(&mono);
    let raw3 = pwv3.to_raw();
    assert_eq!(be32(&raw3.full_header(), 0x10), 50, "PWV3 count is a byte count");
    assert_eq!(Waveform::from_raw(&raw3).unwrap().mono_columns().unwrap(), mono);

    let pwv6 = Waveform { kind: FourCC::PWV6, entry_size: 3, extra: None, data: vec![1; 3600] };
    let raw6 = pwv6.to_raw();
    assert_eq!(raw6.header_len(), 0x14);
    assert_eq!(be32(&raw6.full_header(), 0x10), 1200);
    assert_eq!(Waveform::from_raw(&raw6).unwrap(), pwv6);
}

#[test]
fn sized_waveform_header_checks() {
    let mut raw = Waveform::color_detail(&[ColorColumn { c0: 1, c1: 1, c2: 1, height: 1 }]).to_raw();
    raw.header[0] = 1; // u16@0C != 0
    assert!(matches!(Waveform::from_raw(&raw), Err(AnlzError::BadLength(_))));
    let mut raw = Waveform::color_detail(&[]).to_raw();
    raw.header.push(0); // header length 0x19
    assert!(matches!(Waveform::from_raw(&raw), Err(AnlzError::BadLength(_))));
}

// ---------------------------------------------------------------------------------------------
// Cue lists (§5b, §5d)

#[test]
fn cue_list_with_entries() {
    let mut list = CueList::new(CueListType::HotCue);
    for slot in 1..=3 {
        list.cues.push(CuePoint { slot, unknown_20: [slot as u32 * 1000, 0, 0, 0, 0, 0], ..CuePoint::default() });
    }
    let raw = list.to_raw();
    let h = raw.full_header();
    assert_eq!(be16(&h, 0x0E), 1, "type 1 = hot cues");
    assert_eq!(be16(&h, 0x12), 3, "count");
    assert_eq!(be32(&h, 0x14), 0xFFFF_FFFF);
    assert_eq!(raw.total_len(), 0x18 + 3 * 0x38, "total_len = 0x18 + 0x38 · count");
    assert_eq!(&raw.body[0..4], b"PCPT");
    assert_eq!(be32(&raw.body, 4), 0x1C);
    assert_eq!(be32(&raw.body, 8), 0x38);
    assert_eq!(be16(&raw.body, 0x0E), 1, "slot");
    assert_eq!(CueList::from_raw(&raw).unwrap(), list);
    assert_eq!(CueListType::HotCue.default_capacity(), 3);
    assert_eq!(CueListType::Memory.default_capacity(), 10);
}

#[test]
fn cue_list_rejects_unknown_type() {
    let mut raw = CueList::new(CueListType::Memory).to_raw();
    raw.header[3] = 2; // u16@0E = 2
    assert!(matches!(CueList::from_raw(&raw), Err(AnlzError::UnexpectedType { value: 2, .. })));
}

// ---------------------------------------------------------------------------------------------
// PQT2, PVB2 (§5c)

fn sample_pqt2(with_tail: bool) -> ExtBeatGrid {
    ExtBeatGrid {
        unknown_0e: 0,
        version: 1,
        unknown_11: 0,
        entry_size: 2,
        unknown_14: 0,
        unknown_16: 0,
        first_beat: Beat { beat: 1, tempo_x100: 14898, time_ms: 3 },
        last_beat: Beat { beat: 4, tempo_x100: 14898, time_ms: 126_867 },
        unknown_2c: 0x0179_B744,
        unknown_30: with_tail.then_some((0, 0)),
        data: (0..632u32).map(|i| i as u8).collect(),
    }
}

#[test]
fn pqt2_layout_both_header_lengths() {
    for (tail, hl) in [(true, 0x38), (false, 0x30)] {
        let q = sample_pqt2(tail);
        let raw = q.to_raw();
        assert_eq!(raw.header_len(), hl);
        let h = raw.full_header();
        assert_eq!(h[0x10], 1);
        assert_eq!(be16(&h, 0x12), 2, "entry size");
        assert_eq!(&h[0x18..0x20], &[0, 1, 0x3a, 0x32, 0, 0, 0, 3], "embedded first beat");
        assert_eq!(be32(&h, 0x28), 316, "count");
        assert_eq!(ExtBeatGrid::from_raw(&raw).unwrap(), q);
    }
    let mut raw = sample_pqt2(true).to_raw();
    raw.header.truncate(raw.header.len() - 4); // header_len 0x34 is invalid
    assert!(ExtBeatGrid::from_raw(&raw).is_err());
}

#[test]
fn pvb2_layout() {
    let v = Vbr2 { unknown_0e: 0, sample_count: 7_056_000, entry_size: 20, data: vec![0x11; 8000] };
    let raw = v.to_raw();
    let h = raw.full_header();
    assert_eq!(raw.header_len(), 0x20);
    assert_eq!(u64::from_be_bytes(h[0x10..0x18].try_into().unwrap()), 7_056_000);
    assert_eq!(be32(&h, 0x18), 400);
    assert_eq!(be32(&h, 0x1C), 20);
    assert_eq!(Vbr2::from_raw(&raw).unwrap(), v);
}

// ---------------------------------------------------------------------------------------------
// PSSI (§5c, §5d)

fn phrase(index: u16, beat: u16, kind: u16) -> Phrase {
    Phrase { index, beat, kind, ..Phrase::default() }
}

#[test]
fn pssi_layout_and_validation() {
    let mut p2 = phrase(2, 33, 2);
    p2.fill = 1;
    p2.fill_beat = 62;
    let s = SongStructure::new(Mood::High, 200, vec![phrase(1, 1, 1), p2, phrase(3, 65, 5)]);
    let raw = s.to_raw();
    let h = raw.full_header();
    assert_eq!(raw.header_len(), 0x20);
    assert_eq!(be16(&h, 0x0E), 0x18);
    assert_eq!(be16(&h, 0x10), 3, "count");
    assert_eq!(be16(&h, 0x12), 1, "mood");
    assert_eq!(be16(&h, 0x1A), 200, "end beat");
    assert_eq!(raw.total_len(), 0x20 + 24 * 3);
    assert_eq!(&raw.body[24..30], &[0, 2, 0, 33, 0, 2], "index, beat, kind as BE u16");
    assert_eq!(SongStructure::from_raw(&raw).unwrap(), s);

    let mut bad = raw.clone();
    bad.body.extend_from_slice(&[0; 2]); // total_len != 0x20 + 24 n
    assert!(matches!(SongStructure::from_raw(&bad), Err(AnlzError::BadLength(_))));
    let mut bad = raw.clone();
    bad.header[3] = 0x10; // u16@0E != 0x18
    assert!(SongStructure::from_raw(&bad).is_err());
}

#[test]
fn phrase_labels_mood_high() {
    let l = |kind, k1, k2, k3| phrase_label(Mood::High, kind, k1, k2, k3);
    assert_eq!(l(1, true, false, false), Some("INTRO 1"));
    assert_eq!(l(1, false, false, false), Some("INTRO 2"));
    assert_eq!(l(2, false, false, false), Some("UP 1"));
    assert_eq!(l(2, false, false, true), Some("UP 2"));
    assert_eq!(l(2, false, true, true), Some("UP 3"), "k2 wins over k3");
    assert_eq!(l(3, false, false, false), Some("DOWN"));
    assert_eq!(l(4, false, false, false), Some("UP 2"));
    assert_eq!(l(5, true, false, false), Some("CHORUS 1"));
    assert_eq!(l(5, false, false, false), Some("CHORUS 2"));
    assert_eq!(l(6, true, false, false), Some("OUTRO 1"));
    assert_eq!(l(6, false, false, false), Some("OUTRO 2"));
    assert_eq!(l(7, false, false, false), None);
    assert_eq!(l(0, false, false, false), None);
}

#[test]
fn phrase_labels_mood_mid_and_low() {
    let mid: Vec<_> = (1..=10).map(|k| phrase_label(Mood::Mid, k, false, false, false).unwrap()).collect();
    assert_eq!(mid, ["INTRO", "VERSE1", "VERSE2", "VERSE3", "VERSE4", "VERSE5", "VERSE6", "BRIDGE", "CHORUS", "OUTRO"]);
    assert_eq!(phrase_label(Mood::Mid, 11, false, false, false), None);

    let low: Vec<_> = (1..=10).map(|k| phrase_label(Mood::Low, k, false, false, false).unwrap()).collect();
    assert_eq!(
        low,
        ["INTRO", "VERSE 1", "VERSE 1", "VERSE 1", "VERSE 2", "VERSE 2", "VERSE 2", "BRIDGE", "CHORUS", "OUTRO"]
    );
    assert_eq!(phrase_label(Mood::Other(4), 1, false, false, false), None);
}

#[test]
fn fill_in_steps() {
    let mut p1 = phrase(1, 1, 1);
    p1.fill = 1;
    p1.fill_beat = 30; // next phrase starts at 33 -> 3-beat fill
    let mut p2 = phrase(2, 33, 2);
    p2.fill = 1;
    p2.fill_beat = 60; // 5-beat fill -> ignored
    let s = SongStructure::new(Mood::High, 100, vec![p1, p2, phrase(3, 65, 5)]);
    assert_eq!(s.phrase_at(1), Some(0));
    assert_eq!(s.phrase_at(64), Some(1));
    assert_eq!(s.phrase_at(99), Some(2));
    assert_eq!(s.phrase_at(100), None);
    assert_eq!(s.fill_step(29), None);
    assert_eq!(s.fill_step(30), Some(1));
    assert_eq!(s.fill_step(31), Some(2));
    assert_eq!(s.fill_step(32), Some(3));
    assert_eq!(s.fill_step(33), None);
    assert_eq!(s.fill_step(61), None, "fill longer than 4 beats");
}

// ---------------------------------------------------------------------------------------------
// .3EX MessagePack (§5a)

#[test]
fn threex_decodes_embedding_document() {
    let doc = Value::Map(vec![(
        Value::Str("embedding".into()),
        Value::Map(vec![
            (Value::Str("d2".into()), Value::Str("138568199".into())),
            (Value::Str("d4".into()), Value::F64(127.0036875)),
            (Value::Str("d6".into()), Value::Array((0..64).map(|i| Value::F64(i as f64 / 64.0)).collect())),
            (Value::Str("d9".into()), Value::UInt(56)),
            (Value::Str("d11".into()), Value::F64(148.98)),
        ]),
    )]);
    let mut bytes = Vec::new();
    threex::encode(&doc, &mut bytes);
    assert_eq!(&bytes[0..11], &[0x81, 0xa9, b'e', b'm', b'b', b'e', b'd', b'd', b'i', b'n', b'g']);
    let back = threex::decode(&bytes).unwrap();
    assert_eq!(back, doc);
    let emb = back.get("embedding").unwrap();
    assert_eq!(emb.get("d11").and_then(Value::as_f64), Some(148.98));
    match emb.get("d6") {
        Some(Value::Array(a)) => assert_eq!(a.len(), 64),
        other => panic!("d6: {other:?}"),
    }
    bytes.push(0);
    assert!(threex::decode(&bytes).is_err(), "trailing bytes are rejected");
}
