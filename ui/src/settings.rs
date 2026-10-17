//! Persistent settings: `%APPDATA%/rekordbox-rs/settings.txt`, one `key=value` per line.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Audio output device (name part), `None` = system default.
    pub device: Option<String>,
    /// Buffer size in frames, `None` = device default.
    pub buffer: Option<u32>,
    pub music: Option<PathBuf>,
    pub anlz: Option<PathBuf>,
    pub midi: Option<PathBuf>,
    /// Join DJ Link on start.
    pub djlink: bool,
    pub djlink_adapter: String,
    pub auto_analyze: bool,
    pub zoom_s: f64,
    /// Connect a known controller (built-in mapping) automatically when it is plugged in.
    pub auto_controller: bool,
    /// Offer the library to DJ players (Link Export) on start.
    pub link_export: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            device: None,
            buffer: None,
            music: None,
            anlz: None,
            midi: None,
            djlink: false,
            djlink_adapter: String::new(),
            auto_analyze: true,
            zoom_s: 8.0,
            auto_controller: true,
            link_export: false,
        }
    }
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        rbx_engine::session::data_dir().map(|d| d.join("settings.txt"))
    }

    /// Parse the file contents; unknown keys and bad values are ignored.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            let opt = |v: &str| (!v.is_empty()).then(|| v.to_string());
            match k.trim() {
                "device" => s.device = opt(v),
                "buffer" => s.buffer = v.parse().ok().filter(|&n: &u32| (32..=8192).contains(&n)),
                "music" => s.music = opt(v).map(PathBuf::from),
                "anlz" => s.anlz = opt(v).map(PathBuf::from),
                "midi" => s.midi = opt(v).map(PathBuf::from),
                "djlink" => s.djlink = v == "1" || v == "true",
                "djlink_adapter" => s.djlink_adapter = v.to_string(),
                "auto_analyze" => s.auto_analyze = v != "0" && v != "false",
                "zoom_s" => s.zoom_s = v.parse().ok().filter(|z: &f64| (1.0..=64.0).contains(z)).unwrap_or(8.0),
                "auto_controller" => s.auto_controller = v != "0" && v != "false",
                "link_export" => s.link_export = v == "1" || v == "true",
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let p = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        format!(
            "device={}\nbuffer={}\nmusic={}\nanlz={}\nmidi={}\ndjlink={}\ndjlink_adapter={}\nauto_analyze={}\nzoom_s={}\nauto_controller={}\nlink_export={}\n",
            self.device.as_deref().unwrap_or(""),
            self.buffer.map_or(String::new(), |b| b.to_string()),
            p(&self.music),
            p(&self.anlz),
            p(&self.midi),
            u8::from(self.djlink),
            self.djlink_adapter,
            u8::from(self.auto_analyze),
            self.zoom_s,
            u8::from(self.auto_controller),
            u8::from(self.link_export)
        )
    }

    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, self.to_text())
    }
}
