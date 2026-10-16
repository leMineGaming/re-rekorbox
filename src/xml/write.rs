//! `rekordbox.xml` writer (SPEC §5e "Dokumentaufbau", "TRACK-Attribute", "TEMPO", "POSITION_MARK").
//!
//! Attribute order and number formats follow the original writer. Indentation (two spaces per
//! level, one element per line) is a design choice – the original layout was not determined.

use super::{encode_location, escape_attr, Document, Node, PositionMark, Tempo, Track, TRACK_COLOURS};
use std::fmt::Write as _;

struct Out {
    s: String,
}

impl Out {
    fn open(&mut self, depth: usize, name: &str, attrs: &[(&str, String)], empty: bool) {
        for _ in 0..depth {
            self.s.push_str("  ");
        }
        self.s.push('<');
        self.s.push_str(name);
        for (k, v) in attrs {
            let _ = write!(self.s, " {k}=\"{}\"", escape_attr(v));
        }
        self.s.push_str(if empty { "/>\n" } else { ">\n" });
    }

    fn close(&mut self, depth: usize, name: &str) {
        for _ in 0..depth {
            self.s.push_str("  ");
        }
        let _ = writeln!(self.s, "</{name}>");
    }
}

/// `%.02f` of `value × 0.01` (AverageBpm, Bpm).
fn x100(v: u32) -> String {
    format!("{:.2}", v as f64 * 0.01)
}

/// `%.03f` of `ms × 0.001` (Inizio, Start, End).
fn seconds(ms: u32) -> String {
    format!("{:.3}", ms as f64 * 0.001)
}

/// Serialise a document.
pub fn to_string(doc: &Document) -> String {
    let mut o = Out { s: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\n") };
    o.open(0, "DJ_PLAYLISTS", &[("Version", doc.version.clone())], false);
    o.open(
        1,
        "PRODUCT",
        &[
            ("Name", doc.product.name.clone()),
            ("Version", doc.product.version.clone()),
            ("Company", doc.product.company.clone()),
        ],
        true,
    );
    o.open(1, "COLLECTION", &[("Entries", doc.tracks.len().to_string())], doc.tracks.is_empty());
    if !doc.tracks.is_empty() {
        for t in &doc.tracks {
            write_track(&mut o, t);
        }
        o.close(1, "COLLECTION");
    }
    o.open(1, "PLAYLISTS", &[], false);
    let root = [("Type", "0".to_string()), ("Name", "ROOT".to_string()), ("Count", doc.playlists.len().to_string())];
    o.open(2, "NODE", &root, doc.playlists.is_empty());
    if !doc.playlists.is_empty() {
        for n in &doc.playlists {
            write_node(&mut o, 3, n);
        }
        o.close(2, "NODE");
    }
    o.close(1, "PLAYLISTS");
    o.close(0, "DJ_PLAYLISTS");
    o.s
}

/// Attributes of a `TRACK` element in writer order (§5e).
pub fn track_attributes(t: &Track) -> Vec<(&'static str, String)> {
    let mut a = vec![
        ("TrackID", t.track_id.to_string()),
        ("Name", t.name.clone()),
        ("Artist", t.artist.clone()),
        ("Composer", t.composer.clone()),
        ("Album", t.album.clone()),
        ("Grouping", t.grouping.clone()),
        ("Genre", t.genre.clone()),
        ("Kind", t.kind.as_str().to_string()),
        ("Size", t.size.to_string()),
        ("TotalTime", t.total_time.to_string()),
        ("DiscNumber", t.disc_number.to_string()),
        ("TrackNumber", t.track_number.to_string()),
        ("Year", t.year.to_string()),
        ("AverageBpm", x100(t.bpm_x100)),
        ("DateAdded", t.date_added.clone()),
        ("BitRate", t.bit_rate.to_string()),
        ("SampleRate", t.sample_rate.to_string()),
        ("Comments", t.comments.clone()),
        ("PlayCount", t.play_count.to_string()),
        ("Rating", (t.rating.min(5) as u32 * 51).to_string()),
        ("Location", encode_location(&t.location)),
        ("Remixer", t.remixer.clone()),
        ("Tonality", t.tonality.clone()),
        ("Label", t.label.clone()),
        ("Mix", t.mix.clone()),
    ];
    if let Some(c) = t.colour.filter(|c| (1..=8).contains(c)) {
        a.push(("Colour", TRACK_COLOURS[c as usize - 1].to_string()));
    }
    a
}

fn write_track(o: &mut Out, t: &Track) {
    let empty = t.tempos.is_empty() && t.marks.is_empty();
    o.open(2, "TRACK", &track_attributes(t), empty);
    if empty {
        return;
    }
    for tempo in &t.tempos {
        o.open(3, "TEMPO", &tempo_attributes(tempo), true);
    }
    for m in &t.marks {
        o.open(3, "POSITION_MARK", &mark_attributes(m), true);
    }
    o.close(2, "TRACK");
}

/// `TEMPO` attributes: `Inizio`, `Bpm`, `Metro`, `Battito`.
pub fn tempo_attributes(t: &Tempo) -> Vec<(&'static str, String)> {
    vec![
        ("Inizio", seconds(t.inizio_ms)),
        ("Bpm", x100(t.bpm_x100)),
        ("Metro", t.metro.clone()),
        ("Battito", t.battito.to_string()),
    ]
}

/// `POSITION_MARK` attributes: `Name`, `Type`, `Start`, [`End`], `Num`, [`Red`, `Green`, `Blue`].
pub fn mark_attributes(m: &PositionMark) -> Vec<(&'static str, String)> {
    let mut a = vec![("Name", m.name.clone()), ("Type", m.mark_type.code().to_string()), ("Start", seconds(m.start_ms))];
    if let Some(end) = m.end_ms.filter(|&e| e != 0) {
        a.push(("End", seconds(end)));
    }
    a.push(("Num", m.num.to_string()));
    if (0..8).contains(&m.num) {
        if let Some((r, g, b)) = m.rgb {
            a.push(("Red", r.to_string()));
            a.push(("Green", g.to_string()));
            a.push(("Blue", b.to_string()));
        }
    }
    a
}

fn write_node(o: &mut Out, depth: usize, n: &Node) {
    match n {
        Node::Folder { name, children } => {
            let attrs = [("Name", name.clone()), ("Type", "0".into()), ("Count", children.len().to_string())];
            o.open(depth, "NODE", &attrs, children.is_empty());
            if !children.is_empty() {
                for c in children {
                    write_node(o, depth + 1, c);
                }
                o.close(depth, "NODE");
            }
        }
        Node::Playlist { name, key_type, keys } => {
            let attrs = [
                ("Name", name.clone()),
                ("Type", "1".into()),
                ("KeyType", key_type.to_string()),
                ("Entries", keys.len().to_string()),
            ];
            o.open(depth, "NODE", &attrs, keys.is_empty());
            if !keys.is_empty() {
                for k in keys {
                    o.open(depth + 1, "TRACK", &[("Key", k.clone())], true);
                }
                o.close(depth, "NODE");
            }
        }
    }
}
