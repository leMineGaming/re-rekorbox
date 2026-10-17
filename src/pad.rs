//! Performance pad definitions (`pad/*.pad.csv`, `pad/Version`, SPEC §5g).
//!
//! Every non-empty line that does not start with `#` describes one pad as comma-separated
//! `key=value` pairs, e.g. `function=HotCue,value=A`.

/// Number of pad slots rekordbox allocates per pad mode (§5g).
pub const PADS_PER_MODE: usize = 16;

/// One pad: ordered key/value properties (later duplicates overwrite earlier ones).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pad {
    pub props: Vec<(String, String)>,
}

impl Pad {
    /// Parse one line (§5g): split at `,`; key = text before the first `=`, value = text after it.
    pub fn parse_line(line: &str) -> Pad {
        let mut pad = Pad::default();
        for part in line.split(',') {
            let (k, v) = match part.split_once('=') {
                Some((k, v)) => (k, v),
                None => (part, ""),
            };
            pad.set(k, v);
        }
        pad
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.props.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    pub fn set(&mut self, key: &str, value: &str) {
        match self.props.iter_mut().find(|(k, _)| k == key) {
            Some(p) => p.1 = value.to_string(),
            None => self.props.push((key.to_string(), value.to_string())),
        }
    }

    pub fn function(&self) -> &str {
        self.get("function").unwrap_or("")
    }

    pub fn value(&self) -> &str {
        self.get("value").unwrap_or("")
    }

    /// Apply the user variant: if `userfunction` and `uservalue` are both non-empty they replace
    /// `function` and `value` (§5g). Returns whether the pad changed.
    pub fn apply_user_variant(&mut self) -> bool {
        let uf = self.get("userfunction").unwrap_or("").to_string();
        let uv = self.get("uservalue").unwrap_or("").to_string();
        if uf.is_empty() || uv.is_empty() {
            return false;
        }
        self.set("function", &uf);
        self.set("value", &uv);
        true
    }

    /// Line form (`key=value` joined by `,`).
    pub fn to_line(&self) -> String {
        self.props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
    }
}

/// Parse a `.pad.csv` file into pads in file order.
pub fn parse(text: &str) -> Vec<Pad> {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(Pad::parse_line)
        .collect()
}

/// Write pads (one line each, `\n` line endings like the shipped files).
pub fn write(pads: &[Pad]) -> String {
    pads.iter().map(|p| p.to_line() + "\n").collect()
}

/// One entry of `Mode.pad.csv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadMode {
    pub function: String,
    pub kind: String,
    pub mode: bool,
    pub template: bool,
    pub pad: bool,
    pub text: String,
    pub select: Option<u32>,
}

impl PadMode {
    pub fn from_pad(p: &Pad) -> PadMode {
        let on = |k: &str| p.get(k) == Some("on");
        PadMode {
            function: p.function().to_string(),
            kind: p.get("type").unwrap_or("").to_string(),
            mode: on("mode"),
            template: on("template"),
            pad: on("pad"),
            text: p.get("text").unwrap_or("").to_string(),
            select: p.get("select").and_then(|s| s.trim().parse().ok()),
        }
    }

    /// File holding the pads of this mode, e.g. `HotCue.pad.csv` (empty for separators).
    pub fn pad_file_name(&self) -> Option<String> {
        (!self.function.is_empty()).then(|| format!("{}.pad.csv", self.function))
    }
}

/// Parse `Mode.pad.csv`.
pub fn parse_modes(text: &str) -> Vec<PadMode> {
    parse(text).iter().map(PadMode::from_pad).collect()
}
