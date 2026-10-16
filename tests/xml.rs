//! rekordbox.xml tests on synthetic data. Expected output and rules come from SPEC.md §5e.

use rekordbox_formats::anlz::tags::Beat;
use rekordbox_formats::xml::read::{from_str_with_report, ReadReport};
use rekordbox_formats::xml::write::{LINE_WRAP, NEWLINE};
use rekordbox_formats::xml::{
    beat_grid_from_tempos, cue_kind_from_num, decode_location, encode_location, escape_attr, from_str,
    num_from_cue_kind, tempos_from_beat_grid, to_string, BeatGridExport, Document, Element, FileKind, MarkType, Node,
    PositionMark, Product, Tempo, TempoAnchor, Track,
};

fn sample_doc() -> Document {
    let track = Track {
        track_id: 7,
        name: "Song & \"Name\"".into(),
        artist: "A".into(),
        album: "Al".into(),
        genre: "House".into(),
        kind: FileKind::Flac,
        size: 12345,
        total_time: 300,
        track_number: 3,
        year: 2024,
        bpm_x100: 12850,
        date_added: "2026-01-05".into(),
        bit_rate: 1411,
        sample_rate: 44100,
        comments: "x <y>".into(),
        play_count: 2,
        rating: 4,
        location: "C:\\Music\\Ärger Mix.flac".into(),
        tonality: "Am".into(),
        colour: Some(5),
        tempos: vec![Tempo { inizio_ms: 25, bpm_x100: 12850, metro: "4/4".into(), battito: 1 }],
        marks: vec![
            PositionMark {
                name: "Drop".into(),
                mark_type: MarkType::Cue,
                start_ms: 64025,
                end_ms: None,
                num: 0,
                rgb: Some((40, 226, 20)),
            },
            PositionMark {
                name: String::new(),
                mark_type: MarkType::Loop,
                start_ms: 10000,
                end_ms: Some(17469),
                num: -1,
                rgb: None,
            },
        ],
        ..Track::default()
    };
    Document {
        version: "1.0.0".into(),
        product: Product { name: "rekordbox".into(), version: "7.2.19".into(), company: "AlphaTheta".into() },
        tracks: vec![track],
        playlists: vec![
            Node::Folder {
                name: "Folder".into(),
                children: vec![Node::Playlist { name: "Set 1".into(), key_type: 0, keys: vec!["7".into()] }],
            },
            Node::Playlist { name: "Empty".into(), key_type: 0, keys: vec![] },
        ],
    }
}

/// Expected output, computed by hand from the serialisation rules (CRLF, indent 2, wrap > 60 bytes,
/// continuation lines indented by indent + 1 + len(tag name) spaces before the attribute's own space).
fn expected() -> String {
    let lines = [
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "",
        r#"<DJ_PLAYLISTS Version="1.0.0">"#,
        r#"  <PRODUCT Name="rekordbox" Version="7.2.19" Company="AlphaTheta"/>"#,
        r#"  <COLLECTION Entries="1">"#,
        r#"    <TRACK TrackID="7" Name="Song &amp; &quot;Name&quot;" Artist="A" Composer="""#,
        r#"           Album="Al" Grouping="" Genre="House" Kind="FLAC File" Size="12345""#,
        r#"           TotalTime="300" DiscNumber="0" TrackNumber="3" Year="2024" AverageBpm="128.50""#,
        r#"           DateAdded="2026-01-05" BitRate="1411" SampleRate="44100" Comments="x &lt;y&gt;""#,
        r#"           PlayCount="2" Rating="204" Location="file://localhost/C:/Music/%c3%84rger%20Mix.flac""#,
        r#"           Remixer="" Tonality="Am" Label="" Mix="" Colour="0x00FF00">"#,
        r#"      <TEMPO Inizio="0.025" Bpm="128.50" Metro="4/4" Battito="1"/>"#,
        r#"      <POSITION_MARK Name="Drop" Type="0" Start="64.025" Num="0" Red="40" Green="226""#,
        r#"                     Blue="20"/>"#,
        r#"      <POSITION_MARK Name="" Type="4" Start="10.000" End="17.469" Num="-1"/>"#,
        r#"    </TRACK>"#,
        r#"  </COLLECTION>"#,
        r#"  <PLAYLISTS>"#,
        r#"    <NODE Type="0" Name="ROOT" Count="2">"#,
        r#"      <NODE Name="Folder" Type="0" Count="1">"#,
        r#"        <NODE Name="Set 1" Type="1" KeyType="0" Entries="1">"#,
        r#"          <TRACK Key="7"/>"#,
        r#"        </NODE>"#,
        r#"      </NODE>"#,
        r#"      <NODE Name="Empty" Type="1" KeyType="0" Entries="0"/>"#,
        r#"    </NODE>"#,
        r#"  </PLAYLISTS>"#,
        r#"</DJ_PLAYLISTS>"#,
        "",
    ];
    lines.join("\r\n")
}

#[test]
fn writer_matches_spec_layout() {
    assert_eq!(to_string(&sample_doc()), expected());
}

#[test]
fn attribute_wrapping_rule() {
    let e = Element::new("A").attr("k1", "x".repeat(60)).attr("k2", "y").attr("k3", "z");
    let mut s = String::new();
    e.write(&mut s, 0, LINE_WRAP);
    // ` k1="xxx…"` is 66 bytes > 60 -> break before k2, continuation indent 0 + 1 + len("A") = 2.
    assert_eq!(s, format!("<A k1=\"{}\"{NEWLINE}   k2=\"y\" k3=\"z\"/>", "x".repeat(60)));

    // exactly 60 bytes on the line does not wrap (the rule is "greater than")
    let e = Element::new("B").attr("k", "v".repeat(55)).attr("n", "1");
    let mut s = String::new();
    e.write(&mut s, 0, LINE_WRAP);
    assert!(!s.contains(NEWLINE), "60 bytes do not trigger a wrap: {s}");
}

#[test]
fn write_read_round_trip() {
    let doc = sample_doc();
    let back = from_str(&to_string(&doc)).unwrap();
    let mut expected = doc.clone();
    expected.tracks[0].location = "C:/Music/Ärger Mix.flac".into(); // rekordbox keeps paths with '/'
    assert_eq!(back, expected);
}

#[test]
fn track_attribute_mappings() {
    let mut t = Track { rating: 5, colour: Some(1), kind: FileKind::Aiff, ..Track::default() };
    let xml = to_string(&Document { tracks: vec![t.clone()], ..Document::default() });
    assert!(xml.contains(r#"Rating="255""#));
    assert!(xml.contains(r#"Colour="0xFF007F""#));
    assert!(xml.contains(r#"Kind="AIFF File""#));
    t.colour = None;
    t.rating = 0;
    let xml = to_string(&Document { tracks: vec![t], ..Document::default() });
    assert!(!xml.contains("Colour="), "no Colour attribute for colour index 0");
    assert!(xml.contains(r#"Rating="0""#));

    for (code, text) in [(1, "MP3 File"), (2, "AAC File"), (3, "MP4 File"), (4, "M4A File"), (5, "FLAC File"),
        (6, "ALAC File"), (11, "WAV File"), (12, "AIFF File"), (0, "Unknown Format")]
    {
        let k = FileKind::from_code(code);
        assert_eq!(k.as_str(), text);
        assert_eq!(FileKind::from_str_value(text), k);
    }
}

#[test]
fn location_encoding_follows_original() {
    assert_eq!(encode_location("C:\\x\\(1)!#$*+,-.;=?@_~.wav"), "file://localhost/C:/x/(1)!#$*+,-.;=?@_~.wav");
    assert_eq!(encode_location("ü&%"), "file://localhost/%c3%bc%26%25", "lower-case hex");
    assert_eq!(encode_location("/Users/dj/a b.mp3"), "file://localhost//Users/dj/a%20b.mp3", "path appended as is");

    assert_eq!(decode_location("file://localhost/C:/M/%C3%BC.mp3"), "C:/M/ü.mp3", "upper-case hex accepted");
    assert_eq!(decode_location("FILE://LOCALHOST/C:/a%20b"), "C:/a b", "prefix case-insensitive");
    assert_eq!(decode_location("file://localhost/C:\\dir\\f.mp3"), "C:/dir/f.mp3", "backslash -> slash");
    assert_eq!(decode_location("file://localhost/a%zzb%4"), "a%zzb%4", "malformed escapes stay");
    assert_eq!(decode_location("file://localhost/Users/dj/x.mp3"), "Users/dj/x.mp3", "no slash is added");
    assert_eq!(decode_location("D:/plain.wav"), "D:/plain.wav", "no prefix: unchanged");
    for p in ["C:/Music/Ärger Mix.flac", "/Users/dj/a b.mp3", "E:/x/#1 (live).wav"] {
        assert_eq!(decode_location(&encode_location(p)), p);
    }
}

#[test]
fn attribute_escaping() {
    assert_eq!(escape_attr(r#"a&b"c'd<e>f"#), "a&amp;b&quot;c&apos;d&lt;e&gt;f");
    assert_eq!(escape_attr("l1\nl2\r\n"), "l1l2", "newlines are removed in attributes");
    assert_eq!(escape_attr("x\u{1}y\tz"), "xyz", "control characters are removed");
    assert_eq!(escape_attr("Ä–€"), "Ä–€");
}

#[test]
fn hot_cue_palette_lookup() {
    use rekordbox_formats::xml::{nearest_hot_cue_colour, HOT_CUE_PALETTE};
    for (i, &c) in HOT_CUE_PALETTE.iter().enumerate() {
        assert_eq!(nearest_hot_cue_colour(c), i as u8 + 1, "exact palette colour {i}");
    }
    assert_eq!(nearest_hot_cue_colour((40, 226, 20)), 22);
    assert_eq!(nearest_hot_cue_colour((0, 0, 0)), 63);
    assert_eq!(nearest_hot_cue_colour((255, 255, 255)), 64);
    assert_eq!(nearest_hot_cue_colour((250, 250, 250)), 64);
    assert_eq!(sample_doc().tracks[0].marks[0].palette_index(), Some(22));
}

#[test]
fn cue_number_mapping() {
    let pairs = [(1, 0), (2, 1), (3, 2), (5, 3), (6, 4), (7, 5), (8, 6), (9, 7)];
    for (kind, num) in pairs {
        assert_eq!(num_from_cue_kind(kind), Some(num));
        assert_eq!(cue_kind_from_num(num, MarkType::Cue), Some(kind));
    }
    for kind in [-1, 0, 4] {
        assert_eq!(num_from_cue_kind(kind), Some(-1));
    }
    assert_eq!(num_from_cue_kind(10), None);
    assert_eq!(cue_kind_from_num(-1, MarkType::Cue), Some(0));
    assert_eq!(cue_kind_from_num(-1, MarkType::Loop), Some(-1));
    assert_eq!(cue_kind_from_num(8, MarkType::Cue), None);
}

const TOLERANT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="other app" Version="1" Company="X"/>
  <COLLECTION Entries="1">
    <TRACK TrackID="1" Name="T" TotalTime="100" AverageBpm="120,09" Rating="100" Colour="0x00ff00" Kind="mp3 file"
           Location="file://localhost/Music/t.mp3">
      <TEMPO Inizio="0,5" Bpm="120,00" Metro="4/4" Battito="1"/>
      <TEMPO Bpm="120"/>
      <POSITION_MARK Name="ok cue" Type="0" Start="1,25" Num="-1"/>
      <POSITION_MARK Name="ok loop" Type="4" Start="10" End="20" Num="3" Red="1" Green="2" Blue="3"/>
      <POSITION_MARK Name="fade" Type="1" Start="5" Num="-1"/>
      <POSITION_MARK Name="late" Type="0" Start="100.5" Num="-1"/>
      <POSITION_MARK Name="bad loop" Type="4" Start="20" End="10" Num="-1"/>
      <POSITION_MARK Name="loop past end" Type="4" Start="90" End="101" Num="-1"/>
      <POSITION_MARK Name="num 8" Type="0" Start="5" Num="8"/>
      <POSITION_MARK Name="no num" Type="0" Start="5"/>
    </TRACK>
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Name="P" Type="1" KeyType="0" Entries="2"><TRACK Key="1"/><TRACK Key="99"/></NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>"#;

#[test]
fn reader_tolerance_and_validation() {
    let (doc, report) = from_str_with_report(TOLERANT).unwrap();
    assert_eq!(report, ReadReport { skipped_marks: 6, skipped_tempos: 1 });
    let t = &doc.tracks[0];
    assert_eq!(t.bpm_x100, (120.09f64 * 100.0) as u32, "comma separator; truncated like the original");
    assert_eq!(t.rating, 1, "100 / 51 = 1");
    assert_eq!(t.colour, Some(5), "colour matched case-insensitively");
    assert_eq!(t.kind, FileKind::Mp3);
    assert_eq!(t.location, "Music/t.mp3");
    assert_eq!(t.tempos, vec![Tempo { inizio_ms: 500, bpm_x100: 12000, metro: "4/4".into(), battito: 1 }]);
    assert_eq!(t.marks.len(), 2);
    assert_eq!((t.marks[0].name.as_str(), t.marks[0].start_ms, t.marks[0].rgb), ("ok cue", 1250, None));
    assert_eq!(t.marks[1].end_ms, Some(20000));
    assert_eq!(t.marks[1].rgb, Some((1, 2, 3)));
    assert_eq!(
        doc.playlists,
        vec![Node::Playlist { name: "P".into(), key_type: 0, keys: vec!["1".into(), "99".into()] }]
    );
}

#[test]
fn reader_requires_root() {
    assert!(from_str("<?xml version=\"1.0\"?><OTHER/>").is_err());
    assert!(from_str("<DJ_PLAYLISTS Version=\"1.0.0\"/>").is_ok());
}

// ---------------------------------------------------------------------------------------------
// Beat grid export (§5e TEMPO, bridgeExportBeatGrid) and import expansion

fn grid(start_ms: f64, segments: &[(u16, usize)]) -> Vec<Beat> {
    let mut out = Vec::new();
    let mut t = start_ms;
    let mut beat = 1u16;
    for &(tempo, n) in segments {
        for _ in 0..n {
            out.push(Beat { beat, tempo_x100: tempo, time_ms: t.round() as u32 });
            t += 6_000_000.0 / tempo as f64;
            beat = beat % 4 + 1;
        }
    }
    out
}

#[test]
fn beat_grid_export_modes() {
    let constant = grid(50.0, &[(12800, 64)]);
    assert!(tempos_from_beat_grid(&constant, BeatGridExport::None).is_empty());
    assert_eq!(tempos_from_beat_grid(&constant, BeatGridExport::EveryBeat).len(), 64);
    let one = tempos_from_beat_grid(&constant, BeatGridExport::Changes);
    assert_eq!(one.len(), 1, "constant tempo: a single TEMPO");
    assert_eq!(one[0], Tempo { inizio_ms: 50, bpm_x100: 12800, metro: "4/4".into(), battito: 1 });

    let change = grid(50.0, &[(12800, 32), (13000, 32)]);
    let t = tempos_from_beat_grid(&change, BeatGridExport::Changes);
    assert_eq!(t.len(), 2, "tempo change starts a new TEMPO");
    assert_eq!(t[1].bpm_x100, 13000);
    assert_eq!(t[1].inizio_ms, change[32].time_ms);

    let mut shifted = grid(50.0, &[(12800, 32)]);
    for b in shifted.iter_mut().skip(16) {
        b.time_ms += 7; // beats leave the expected grid
    }
    let t = tempos_from_beat_grid(&shifted, BeatGridExport::Changes);
    assert_eq!(t.len(), 2, "off-grid beat starts a new TEMPO");
    assert_eq!(t[1].inizio_ms, shifted[16].time_ms);
}

fn anchor(inizio_s: f64, bpm: f64, metro: &str, battito: i64) -> TempoAnchor {
    TempoAnchor { inizio_s, bpm, metro: metro.into(), battito }
}

#[test]
fn import_expands_single_anchor_to_full_grid() {
    // 128 BPM -> 0.46875 s per beat (exact in binary). 64 beats fit into 29.6 s.
    let original = grid(0.0, &[(12800, 64)]);
    let anchors: Vec<TempoAnchor> =
        tempos_from_beat_grid(&original, BeatGridExport::Changes).iter().map(TempoAnchor::from_tempo).collect();
    assert_eq!(anchors.len(), 1);
    assert_eq!(beat_grid_from_tempos(&anchors, 29.6), original);
    // a beat exactly at TotalTime is still emitted (rule: stop only when t > TotalTime)
    assert_eq!(beat_grid_from_tempos(&anchors, 30.0).len(), 65);
}

#[test]
fn import_fills_between_anchors_and_respects_half_beat_rule() {
    let anchors = [anchor(0.0, 128.0, "4/4", 1), anchor(15.0, 130.0, "4/4", 1)];
    let g = beat_grid_from_tempos(&anchors, 20.0);
    // 32 beats at 128 BPM (0 … 14.53125 s), then the anchor at 15 s, then 130 BPM up to 20 s.
    assert_eq!(g[31], Beat { beat: 4, tempo_x100: 12800, time_ms: 14531 });
    assert_eq!(g[32], Beat { beat: 1, tempo_x100: 13000, time_ms: 15000 });
    assert_eq!(g[33].time_ms, 15462, "60/130 s later, rounded half up");
    let after = g.iter().filter(|b| b.tempo_x100 == 13000).count();
    assert_eq!(after, 11, "15 s + k·0.4615 s ≤ 20 s for k = 0…10");

    // anchor closer than half a beat to the previous generated beat: no extra beat in between
    let anchors = [anchor(0.0, 120.0, "4/4", 1), anchor(1.2, 120.0, "4/4", 3)];
    let g = beat_grid_from_tempos(&anchors, 1.3);
    let times: Vec<u32> = g.iter().map(|b| b.time_ms).collect();
    assert_eq!(times, vec![0, 500, 1200], "1.0 s beat skipped because 1.2 ≤ 1.0 + 0.25");
}

#[test]
fn import_uses_metro_numerator_and_ignores_invalid_anchors() {
    let g = beat_grid_from_tempos(&[anchor(0.0, 60.0, "3/4", 1)], 5.0);
    let beats: Vec<u16> = g.iter().map(|b| b.beat).collect();
    assert_eq!(beats, vec![1, 2, 3, 1, 2, 3]);
    assert!(beat_grid_from_tempos(&[anchor(0.0, 0.0, "4/4", 1)], 5.0).is_empty(), "Bpm 0");
    assert!(beat_grid_from_tempos(&[anchor(0.0, 120.0, "4", 1)], 5.0).is_empty(), "Metro without '/'");
    assert!(beat_grid_from_tempos(&[anchor(0.0, 120.0, "4/4", 0)], 5.0).is_empty(), "Battito 0");
    // Quirk of the original: an anchor after TotalTime is not emitted, but its tempo still drives the
    // final fill, which starts at the previous position (0 s): beats at 0.5, 1.0, … 5.0 s.
    let g = beat_grid_from_tempos(&[anchor(9.0, 120.0, "4/4", 1)], 5.0);
    assert_eq!(g.len(), 10);
    assert_eq!((g[0].beat, g[0].time_ms), (2, 500));
}
