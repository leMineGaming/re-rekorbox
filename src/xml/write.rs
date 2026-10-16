//! `rekordbox.xml` writer (SPEC §5e "Dokumentaufbau", "TRACK-Attribute", "TEMPO", "POSITION_MARK",
//! "Ausgabeformat").
//!
//! The document is built as a small element tree and serialised like the original writer:
//! header line, blank line, root element with two-space indentation per level, `\r\n` line
//! endings, and attribute wrapping once the attributes on the current line exceed 60 bytes.

use super::{encode_location, escape_attr, Document, Node, PositionMark, Tempo, Track, TRACK_COLOURS};

/// Line ending used by the original writer (default of its output stream, §5e).
pub const NEWLINE: &str = "\r\n";
/// Attribute line wrap length passed by the original writer (§5e).
pub const LINE_WRAP: usize = 60;

/// A generic XML element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn new(name: &str) -> Self {
        Element { name: name.to_string(), attrs: Vec::new(), children: Vec::new() }
    }

    pub fn attr(mut self, k: &str, v: impl Into<String>) -> Self {
        self.attrs.push((k.to_string(), v.into()));
        self
    }

    pub fn attrs(mut self, list: Vec<(&str, String)>) -> Self {
        self.attrs.extend(list.into_iter().map(|(k, v)| (k.to_string(), v)));
        self
    }

    pub fn child(mut self, c: Element) -> Self {
        self.children.push(c);
        self
    }

    /// Serialise with the original algorithm: `indent` spaces, `<name`, attributes (each as
    /// ` name="value"`; a line break plus `indent + 1 + len(name)` spaces is inserted before an
    /// attribute when the attributes already written on the line exceed `wrap` bytes), then `/>`
    /// or `>` + children (each on a new line, indent + 2) + newline + indent + `</name>`.
    pub fn write(&self, out: &mut String, indent: usize, wrap: usize) {
        push_spaces(out, indent);
        out.push('<');
        out.push_str(&self.name);
        let align = indent + 1 + self.name.len();
        let mut line_len = 0usize;
        for (k, v) in &self.attrs {
            if line_len > wrap {
                out.push_str(NEWLINE);
                push_spaces(out, align);
                line_len = 0;
            }
            let start = out.len();
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_attr(v));
            out.push('"');
            line_len += out.len() - start;
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            out.push_str(NEWLINE);
            c.write(out, indent + 2, wrap);
        }
        out.push_str(NEWLINE);
        push_spaces(out, indent);
        out.push_str("</");
        out.push_str(&self.name);
        out.push('>');
    }
}

fn push_spaces(out: &mut String, n: usize) {
    out.extend(std::iter::repeat(' ').take(n));
}

/// `%.02f` of `value × 0.01` (AverageBpm, Bpm).
fn x100(v: u32) -> String {
    format!("{:.2}", v as f64 * 0.01)
}

/// `%.03f` of `ms × 0.001` (Inizio, Start, End).
fn seconds(ms: u32) -> String {
    format!("{:.3}", ms as f64 * 0.001)
}

/// Build the element tree of a document.
pub fn to_element(doc: &Document) -> Element {
    let product = Element::new("PRODUCT")
        .attr("Name", doc.product.name.clone())
        .attr("Version", doc.product.version.clone())
        .attr("Company", doc.product.company.clone());
    let mut collection = Element::new("COLLECTION").attr("Entries", doc.tracks.len().to_string());
    collection.children = doc.tracks.iter().map(track_element).collect();
    let mut root = Element::new("NODE")
        .attr("Type", "0")
        .attr("Name", "ROOT")
        .attr("Count", doc.playlists.len().to_string());
    root.children = doc.playlists.iter().map(node_element).collect();
    Element::new("DJ_PLAYLISTS")
        .attr("Version", doc.version.clone())
        .child(product)
        .child(collection)
        .child(Element::new("PLAYLISTS").child(root))
}

/// Serialise a document exactly like the original writer (header, blank line, CRLF).
pub fn to_string(doc: &Document) -> String {
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    s.push_str(NEWLINE);
    s.push_str(NEWLINE);
    to_element(doc).write(&mut s, 0, LINE_WRAP);
    s.push_str(NEWLINE);
    s
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

fn track_element(t: &Track) -> Element {
    let mut e = Element::new("TRACK").attrs(track_attributes(t));
    for tempo in &t.tempos {
        e.children.push(Element::new("TEMPO").attrs(tempo_attributes(tempo)));
    }
    for m in &t.marks {
        e.children.push(Element::new("POSITION_MARK").attrs(mark_attributes(m)));
    }
    e
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

fn node_element(n: &Node) -> Element {
    match n {
        Node::Folder { name, children } => {
            let mut e = Element::new("NODE")
                .attr("Name", name.clone())
                .attr("Type", "0")
                .attr("Count", children.len().to_string());
            e.children = children.iter().map(node_element).collect();
            e
        }
        Node::Playlist { name, key_type, keys } => {
            let mut e = Element::new("NODE")
                .attr("Name", name.clone())
                .attr("Type", "1")
                .attr("KeyType", key_type.to_string())
                .attr("Entries", keys.len().to_string());
            e.children = keys.iter().map(|k| Element::new("TRACK").attr("Key", k.clone())).collect();
            e
        }
    }
}
