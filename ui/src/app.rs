//! The main window: track list, two decks with waveforms, mixer, effects, sampler, settings.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Ui, Vec2};
use rbx_engine::dsp::ColorFx;
use rbx_engine::fx::{BeatFxKind, FxAssign};
use rbx_engine::mixer::{Command, MixerState, DECKS};
use rbx_engine::output::Output;
use rbx_engine::session::{fill_sampler, start_controller, start_link, ControllerHandle, LinkHandle, Loader, Log};

use crate::settings::Settings;
use crate::tracks::{build_rows, filter, RowSource, TrackRow};
use crate::widgets::{self, DetailAction, ACCENT, CUE_COLOR, HOT_CUE_COLORS};

/// Start-up options (from the command line).
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub anlz: Option<PathBuf>,
    pub music: Option<PathBuf>,
    /// Read the library on start.
    pub library: bool,
    pub auto_analyze: bool,
    /// Folder for own analyses (`None` = default `%APPDATA%/rekordbox-rs/ANLZ`).
    pub analysis_root: Option<PathBuf>,
    pub midi: Option<PathBuf>,
    /// `Some(None)` = DJ Link on the first suitable adapter.
    pub djlink: Option<Option<String>>,
    pub samples: Option<PathBuf>,
    /// Sources to load into deck 1 and 2.
    pub load: Vec<String>,
    /// Append messages to this file.
    pub log_file: Option<PathBuf>,
    /// Settings as loaded (changes made in the window are saved to `settings_path`).
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
}

/// Per-deck UI state that the engine does not report back.
#[derive(Debug, Clone)]
struct DeckUi {
    cue_held: bool,
    hot_held: [bool; 8],
    jump_beats: f64,
    color_knob: f32,
    trim_db: f32,
}

impl Default for DeckUi {
    fn default() -> Self {
        DeckUi { cue_held: false, hot_held: [false; 8], jump_beats: 4.0, color_knob: 0.5, trim_db: 0.0 }
    }
}

pub struct App {
    out: Output,
    pub loader: Loader,
    log: Log,
    ctl: Option<ControllerHandle>,
    link: Option<LinkHandle>,
    sampler_names: Vec<String>,
    rows: Vec<TrackRow>,
    rows_gen: u64,
    pub search: String,
    filtered: Vec<usize>,
    filtered_for: String,
    selected: Option<usize>,
    decks: [DeckUi; DECKS],
    color_fx: Option<ColorFx>,
    color_sub: f32,
    fx_depth: f32,
    sampler_volume: f32,
    master_gain: f32,
    /// Seconds visible in the detail waveform.
    pub zoom_s: f64,
    show_settings: bool,
    midi_path: String,
    link_adapter: String,
    error: Option<String>,
    settings: Settings,
    saved: Settings,
    settings_path: Option<PathBuf>,
    devices: Vec<String>,
    music_path: String,
    /// Settings that only apply after a restart were changed.
    restart_needed: bool,
    /// Last line written to the operation log (audio statistics once a minute).
    last_stats_log: std::time::Instant,
}

impl App {
    pub fn new(out: Output, opts: Options) -> App {
        let log = opts.log_file.as_deref().map(Log::with_file).unwrap_or_default();
        log.say(format!("Audio: {} ({} Hz, {} Kanäle)", out.device_name, out.sample_rate, out.channels));
        let mut loader = Loader::new(out.sender(), out.sample_rate, log.clone());
        loader.auto_analyze = opts.auto_analyze;
        if let Some(r) = &opts.analysis_root {
            loader.analysis_root = Some(r.clone());
        }
        if opts.library {
            loader.start_library(opts.anlz.clone(), opts.music.clone());
        }
        let sampler_names = fill_sampler(&out.sender(), out.sample_rate, opts.samples.as_deref());
        let mut app = App {
            out,
            loader,
            log,
            ctl: None,
            link: None,
            sampler_names,
            rows: Vec::new(),
            rows_gen: u64::MAX,
            search: String::new(),
            filtered: Vec::new(),
            filtered_for: String::new(),
            selected: None,
            decks: Default::default(),
            color_fx: None,
            color_sub: 0.5,
            fx_depth: 0.5,
            sampler_volume: 0.8,
            master_gain: 1.0,
            zoom_s: opts.settings.zoom_s,
            show_settings: false,
            midi_path: opts.midi.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            link_adapter: opts.settings.djlink_adapter.clone(),
            error: None,
            settings: opts.settings.clone(),
            saved: opts.settings.clone(),
            settings_path: opts.settings_path.clone(),
            devices: Vec::new(),
            music_path: opts.music.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            restart_needed: false,
            last_stats_log: std::time::Instant::now(),
        };
        for (d, s) in opts.load.iter().take(DECKS).enumerate() {
            app.loader.load(d, s);
        }
        if let Some(m) = opts.midi {
            app.connect_controller(m);
        }
        if let Some(a) = opts.djlink {
            app.link_adapter = a.clone().unwrap_or_default();
            app.join_link();
        }
        app
    }

    fn send(&self, cmd: Command) {
        self.out.send(cmd);
    }

    /// Latest engine state.
    pub fn state(&self) -> MixerState {
        self.out.state()
    }

    pub fn rows(&self) -> &[TrackRow] {
        &self.rows
    }

    /// Load a source into a deck (also tells the controller which file is loaded).
    pub fn load(&mut self, deck: usize, path: &str) {
        self.loader.load(deck, path);
        if let Some(c) = &self.ctl {
            c.controller.lock().unwrap().set_loaded_path(deck, std::path::Path::new(path));
        }
    }

    fn connect_controller(&mut self, mapping: PathBuf) {
        match start_controller(&mapping, None, None, None, self.out.sender(), self.out.state_handle(), &self.loader) {
            Ok(h) => {
                self.ctl = Some(h);
                self.error = None;
                self.settings.midi = Some(mapping);
            }
            Err(e) => {
                self.log.error(format!("Controller: {e}"));
                self.error = Some(e);
            }
        }
    }

    fn join_link(&mut self) {
        let a = self.link_adapter.trim();
        match start_link(if a.is_empty() { None } else { Some(a) }, self.out.state_handle(), self.out.sender(), &self.log) {
            Ok(h) => {
                self.link = Some(h);
                self.error = None;
                self.settings.djlink = true;
                self.settings.djlink_adapter = self.link_adapter.trim().to_string();
            }
            Err(e) => {
                self.log.error(format!("DJ Link: {e}"));
                self.error = Some(e);
            }
        }
    }

    /// Start `rbx-selftest` (next to this program) in its own console window; the report opens in the
    /// browser when it is done.
    fn run_selftest(&mut self, extra: &[&str]) {
        let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("rbx-selftest.exe")));
        let Some(exe) = exe.filter(|p| p.is_file()) else {
            self.log.error("rbx-selftest.exe nicht gefunden (liegt es neben rbx-ui.exe?)");
            return;
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.args(extra).args(["--open", "--pause"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x10;
            cmd.creation_flags(CREATE_NEW_CONSOLE);
        }
        match cmd.spawn() {
            Ok(_) => self.log.say("Selbsttest gestartet (eigenes Fenster)"),
            Err(e) => self.log.error(format!("Selbsttest: {e}")),
        }
    }

    /// Once a minute: audio statistics and deck states into the log (for reports after testing).
    fn log_stats(&mut self, s: &MixerState) {
        if self.last_stats_log.elapsed() < Duration::from_secs(60) {
            return;
        }
        self.last_stats_log = std::time::Instant::now();
        let st = self.out.stats();
        let decks: Vec<String> = (0..DECKS)
            .map(|d| {
                let ds = s.decks[d];
                format!("D{} {} {}", d + 1, if ds.playing { "spielt" } else { "steht" }, ds.bpm.map_or("-".into(), |b| format!("{b:.2}")))
            })
            .collect();
        self.log.say(format!(
            "Betrieb: Last {:.1} % (max {:.1} %), Überlastungen {}, Aussetzer {}, Puffer {}; {}",
            st.load * 100.0,
            st.max_load * 100.0,
            st.overloads,
            st.late,
            st.frames,
            decks.join(", ")
        ));
    }

    /// Save the settings if they changed.
    fn persist(&mut self) {
        self.settings.zoom_s = self.zoom_s;
        if self.settings != self.saved {
            if let Some(p) = &self.settings_path {
                match self.settings.save(p) {
                    Ok(()) => self.saved = self.settings.clone(),
                    Err(e) => self.log.say(format!("Einstellungen nicht gespeichert: {e}")),
                }
            }
        }
    }

    /// Rebuild the track rows when the library changed, refilter when the search changed.
    fn refresh_rows(&mut self) {
        let generation = self.loader.generation();
        if generation != self.rows_gen {
            let files = self.loader.music_files();
            let lib = self.loader.library();
            self.rows = build_rows(lib.as_ref().and_then(|g| g.as_ref()), &files);
            drop(lib);
            self.rows_gen = generation;
            self.filtered_for = "\u{0}".into();
        }
        if self.filtered_for != self.search {
            self.filtered = filter(&self.rows, &self.search);
            self.filtered_for = self.search.clone();
        }
    }

    // ------------------------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut Ui, s: &MixerState) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("rekordbox-rs").strong().size(16.0).color(ACCENT));
            ui.separator();
            ui.label(format!("Audio: {}", self.out.device_name));
            let st = self.out.stats();
            let col = if st.overloads + st.late > 0 { Color32::from_rgb(255, 120, 80) } else { Color32::GRAY };
            ui.label(RichText::new(format!("Last {:.0} %", st.load * 100.0)).color(col)).on_hover_text(format!(
                "Audio-Rechenzeit je Puffer; max {:.0} %, Überlastungen {}, Aussetzer {}",
                st.max_load * 100.0,
                st.overloads,
                st.late
            ));
            if st.overloads + st.late > 0 {
                ui.label(RichText::new(format!("{} Aussetzer", st.overloads + st.late)).color(col));
            }
            ui.separator();
            ui.label("Master");
            widgets::meter(ui, s.peak, 90.0);
            ui.separator();
            if self.loader.library_loading() {
                ui.spinner();
                ui.label("Bibliothek wird gelesen ...");
            } else {
                let analysed = self.rows.iter().filter(|r| r.source != RowSource::Missing).count();
                ui.label(format!("{} Tracks, {} analysiert", self.rows.len(), analysed));
            }
            let b = self.loader.batch();
            if b.running {
                ui.add(egui::ProgressBar::new(b.done as f32 / b.total.max(1) as f32).desired_width(140.0).text(format!("Analyse {}/{}", b.done, b.total)));
            }
            if let Some(c) = &self.ctl {
                ui.separator();
                let dev = c.controller.lock().unwrap().mapping().device.clone();
                ui.label(RichText::new(format!("Controller: {dev}")).color(if c.input_name.is_some() { Color32::LIGHT_GREEN } else { Color32::GRAY }));
            }
            if let Some(l) = &self.link {
                ui.separator();
                let n = l.link.lock().unwrap().state().devices.len();
                ui.label(RichText::new(format!("DJ Link ({n} Geräte)")).color(Color32::LIGHT_GREEN));
            }
            if let Some(e) = self.log.last_error(Duration::from_secs(15)) {
                ui.separator();
                ui.label(RichText::new(e).color(Color32::from_rgb(255, 90, 90)).strong());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Tasten").on_hover_text(SHORTCUTS).clicked() {
                    self.log.say(SHORTCUTS.replace('\n', "; "));
                }
                if ui.button("Einstellungen").clicked() {
                    self.show_settings = !self.show_settings;
                }
                ui.add(egui::Slider::new(&mut self.zoom_s, 2.0..=32.0).logarithmic(true).text("Zoom s").show_value(false));
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        if self.devices.is_empty() {
            self.devices = rbx_engine::output::list_devices();
        }
        egui::Window::new("Einstellungen").open(&mut open).default_width(460.0).show(ctx, |ui| {
            ui.label(RichText::new("Audio").strong());
            let st = self.out.stats();
            ui.label(format!(
                "Aktiv: {} - {} Hz, {} Kanäle, Puffer {} Frames ({:.1} ms)",
                self.out.device_name,
                self.out.sample_rate,
                self.out.channels,
                st.frames,
                f64::from(st.frames) / f64::from(self.out.sample_rate.max(1)) * 1000.0
            ));
            ui.label(format!(
                "Last {:.0} % (max {:.0} %), Überlastungen {}, Aussetzer {}",
                st.load * 100.0,
                st.max_load * 100.0,
                st.overloads,
                st.late
            ));
            ui.horizontal(|ui| {
                ui.label("Gerät");
                let current = self.settings.device.clone().unwrap_or_else(|| "Systemstandard".into());
                egui::ComboBox::from_id_salt("device").width(300.0).selected_text(current).show_ui(ui, |ui| {
                    if ui.selectable_label(self.settings.device.is_none(), "Systemstandard").clicked() {
                        self.settings.device = None;
                        self.restart_needed = true;
                    }
                    for d in self.devices.clone() {
                        if ui.selectable_label(self.settings.device.as_deref() == Some(d.as_str()), &d).clicked() {
                            self.settings.device = Some(d);
                            self.restart_needed = true;
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("Puffer");
                let text = self.settings.buffer.map_or("Standard".to_string(), |b| format!("{b} Frames"));
                egui::ComboBox::from_id_salt("buffer").selected_text(text).show_ui(ui, |ui| {
                    for b in [None, Some(128), Some(256), Some(512), Some(1024), Some(2048)] {
                        let t = b.map_or("Standard".to_string(), |b| format!("{b} Frames"));
                        if ui.selectable_label(self.settings.buffer == b, t).clicked() {
                            self.settings.buffer = b;
                            self.restart_needed = true;
                        }
                    }
                });
                ui.label(RichText::new("kleiner = weniger Verzögerung, größer = weniger Aussetzer").small().weak());
            });
            if self.restart_needed {
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::YELLOW, "Wird nach einem Neustart übernommen.");
                    if ui.button("Jetzt neu starten").clicked() {
                        self.persist();
                        restart(ctx);
                    }
                });
            }
            ui.separator();
            ui.label(RichText::new("Bibliothek").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.music_path).desired_width(300.0).hint_text("Musik-Ordner (leer = Eigene Musik)"));
                if ui.button("Einlesen").clicked() {
                    let m = self.music_path.trim().trim_matches('"');
                    let music = (!m.is_empty()).then(|| PathBuf::from(m));
                    self.settings.music = music.clone();
                    self.loader.start_library(self.settings.anlz.clone(), music);
                }
            });
            if ui.checkbox(&mut self.loader.auto_analyze, "Tracks ohne Analyse beim Laden analysieren").changed() {
                self.settings.auto_analyze = self.loader.auto_analyze;
            }
            ui.separator();
            ui.label(RichText::new("Controller (MIDI-Mapping, z. B. XDJ-RX3.midi.csv)").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.midi_path).desired_width(300.0).hint_text("Pfad zur .midi.csv"));
                if ui.button("Verbinden").clicked() && !self.midi_path.trim().is_empty() {
                    let p = PathBuf::from(self.midi_path.trim().trim_matches('"'));
                    self.connect_controller(p);
                }
            });
            if let Some(c) = &self.ctl {
                ui.label(format!(
                    "MIDI in: {}   MIDI out: {}",
                    c.input_name.as_deref().unwrap_or("nicht gefunden"),
                    c.output_name.as_deref().unwrap_or("nicht gefunden")
                ));
            }
            ui.separator();
            ui.label(RichText::new("Pro DJ Link").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.link_adapter).desired_width(200.0).hint_text("Adapter (leer = automatisch)"));
                if self.link.is_none() && ui.button("Beitreten").clicked() {
                    self.join_link();
                }
            });
            if let Some(l) = &self.link {
                let st = l.link.lock().unwrap().state();
                ui.label(format!("{} – Nummer {:?}", l.adapter, st.number));
                for d in &st.devices {
                    ui.label(format!("#{} {} ({})", d.number, d.name, d.ip));
                }
                for d in 0..DECKS {
                    let mut f = l.follow.lock().unwrap()[d].is_some();
                    if ui.checkbox(&mut f, format!("Deck {} folgt dem Tempo des DJ-Link-Geräts", d + 1)).changed() {
                        l.follow.lock().unwrap()[d] = f.then_some(None);
                    }
                }
            }
            ui.separator();
            ui.label(RichText::new("Selbsttest").strong());
            ui.label(
                RichText::new(
                    "Prüft automatisch Audio-Qualität (offline), Soundkarte (nur Stille), Leistung, Dauerlauf, \
                     Bibliothek und Analyse, MIDI und DJ Link. Ergebnis als Bericht im Browser.",
                )
                .small(),
            );
            ui.horizontal(|ui| {
                if ui.button("Schnelltest (~30 s)").clicked() {
                    self.run_selftest(&["--quick"]);
                }
                if ui.button("Vollständig (~3 min)").clicked() {
                    self.run_selftest(&[]);
                }
                if ui.button("mit Controller-Eingaben (20 s)").on_hover_text("Während des Tests Regler und Tasten am Controller bewegen").clicked() {
                    self.run_selftest(&["--quick", "--midi-listen", "20"]);
                }
            });
            if let Some(dir) = rbx_engine::session::data_dir() {
                ui.label(RichText::new(format!("Logdateien und Berichte: {}", dir.display())).small().weak());
            }
            if let Some(e) = &self.error {
                ui.colored_label(Color32::LIGHT_RED, e);
            }
        });
        self.show_settings = open;
    }

    /// A button that sends `press` when pressed and `release` when let go (CUE, hot cues).
    fn hold_button(&self, ui: &mut Ui, button: egui::Button, held: &mut bool, press: Command, release: Command) -> egui::Response {
        let resp = ui.add(button);
        let down = resp.is_pointer_button_down_on();
        if down && !*held {
            *held = true;
            self.send(press);
        } else if !down && *held {
            *held = false;
            self.send(release);
        } else if !down && resp.clicked() {
            self.send(press);
            self.send(release);
        }
        resp
    }

    fn deck_panel(&mut self, ui: &mut Ui, d: usize, s: &MixerState) {
        let ds = s.decks[d];
        let info = self.loader.deck(d);
        let other = 1 - d;
        egui::Frame::group(ui.style()).fill(Color32::from_gray(24)).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            // title row
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("DECK {}", d + 1)).strong().color(ACCENT));
                let title = info.title.clone().unwrap_or_else(|| if ds.loaded { "?".into() } else { "– leer –".into() });
                ui.label(RichText::new(title).strong().size(15.0));
                if let Some(b) = &info.busy {
                    ui.spinner();
                    ui.label(RichText::new(b).italics());
                }
            });
            // numbers
            ui.horizontal(|ui| {
                let bpm = ds.bpm.map_or("--.--".into(), |b| format!("{b:6.2}"));
                ui.label(RichText::new(bpm).monospace().size(26.0).strong());
                ui.vertical(|ui| {
                    ui.label(RichText::new("BPM").small());
                    ui.label(RichText::new(format!("{:+.2} %", (ds.tempo - 1.0) * 100.0)).monospace());
                });
                ui.separator();
                let rem = (ds.duration_s - ds.position_s).max(0.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(fmt_time(ds.position_s)).monospace().size(18.0));
                    ui.label(RichText::new(format!("-{}", fmt_time(rem))).monospace().weak());
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("Key {}", info.key.as_deref().unwrap_or("–"))).monospace());
                    let beat = ds.beat.map_or("–".into(), |b| format!("{}", (b.floor() as i64).rem_euclid(4) + 1));
                    ui.label(RichText::new(format!("Beat {beat}/4")).monospace());
                });
                if let Some(a) = &info.analysis {
                    if a.variable_tempo {
                        ui.label(RichText::new("variables Tempo").small().color(Color32::YELLOW));
                    }
                }
                if let Some(w) = &info.wave {
                    ui.label(RichText::new(if w.from_anlz { "ANLZ" } else { "Audio" }).small().weak())
                        .on_hover_text("Wellenform aus der Analysedatei (ANLZ) bzw. aus dem Audio berechnet");
                }
            });
            if let Some(f) = widgets::overview(ui, info.wave.as_deref(), &ds, 44.0, &format!("Übersicht Deck {}", d + 1)) {
                self.send(Command::SeekFraction(d, f));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // transport
                ui.vertical(|ui| {
                    let big = Vec2::new(64.0, 40.0);
                    let cue_col = if ds.at_cue { CUE_COLOR } else { Color32::from_gray(60) };
                    let mut held = self.decks[d].cue_held;
                    self.hold_button(
                        ui,
                        egui::Button::new(RichText::new("CUE").strong()).min_size(big).fill(cue_col),
                        &mut held,
                        Command::CuePress(d),
                        Command::CueRelease(d),
                    );
                    self.decks[d].cue_held = held;
                    let play = if ds.playing { "PAUSE" } else { "PLAY" };
                    let col = if ds.playing { Color32::from_rgb(20, 120, 40) } else { Color32::from_gray(60) };
                    if ui.add(egui::Button::new(RichText::new(play).strong()).min_size(big).fill(col)).clicked() {
                        self.send(Command::TogglePlay(d));
                    }
                });
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui.add(egui::Button::new("SYNC").selected(false)).on_hover_text("Tempo und Beat an das andere Deck angleichen").clicked() {
                            self.send(Command::Sync(d));
                        }
                        let is_master = s.master == Some(d);
                        if ui.add(egui::Button::new("MASTER").selected(is_master)).clicked() {
                            self.send(Command::SetMaster(if is_master { None } else { Some(d) }));
                        }
                        if ui.add(egui::Button::new("Q").selected(ds.quantize)).on_hover_text("Quantize").clicked() {
                            self.send(Command::Quantize(d, !ds.quantize));
                        }
                        if ui.add(egui::Button::new("KEY LOCK").selected(ds.master_tempo)).clicked() {
                            self.send(Command::MasterTempo(d, !ds.master_tempo));
                        }
                        if ui.add(egui::Button::new("SLIP").selected(ds.slip)).clicked() {
                            self.send(Command::Slip(d, !ds.slip));
                        }
                        if ui.add(egui::Button::new("REV").selected(ds.reverse)).clicked() {
                            self.send(Command::Reverse(d, !ds.reverse));
                        }
                    });
                    // hot cues
                    ui.horizontal(|ui| {
                        for i in 0..8 {
                            let set = ds.hot_cues[i];
                            let fill = if set { HOT_CUE_COLORS[i].gamma_multiply(0.8) } else { Color32::from_gray(45) };
                            let text = RichText::new(((b'A' + i as u8) as char).to_string()).strong().color(if set { Color32::BLACK } else { Color32::LIGHT_GRAY });
                            let mut held = self.decks[d].hot_held[i];
                            let r = self.hold_button(
                                ui,
                                egui::Button::new(text).min_size(Vec2::new(30.0, 26.0)).fill(fill),
                                &mut held,
                                Command::HotCuePress(d, i),
                                Command::HotCueRelease(d, i),
                            );
                            self.decks[d].hot_held[i] = held;
                            if r.secondary_clicked() {
                                self.send(Command::HotCueDelete(d, i));
                            }
                            r.on_hover_text("Hot Cue: Klick setzt / springt, Rechtsklick löscht");
                        }
                    });
                    // loops
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("IN").clicked() {
                            self.send(Command::LoopIn(d));
                        }
                        if ui.button("OUT").clicked() {
                            self.send(Command::LoopOut(d));
                        }
                        if ui.add(egui::Button::new("RELOOP").selected(ds.looping)).clicked() {
                            self.send(Command::ReloopExit(d));
                        }
                        ui.separator();
                        for beats in [1.0, 2.0, 4.0, 8.0, 16.0] {
                            if ui.small_button(format!("{beats}")).on_hover_text(format!("Loop {beats} Beats")).clicked() {
                                self.send(Command::BeatLoop(d, beats));
                            }
                        }
                        if ui.small_button("½").clicked() {
                            self.send(Command::LoopScale(d, 0.5));
                        }
                        if ui.small_button("×2").clicked() {
                            self.send(Command::LoopScale(d, 2.0));
                        }
                    });
                    // beat jump, memory, key
                    ui.horizontal_wrapped(|ui| {
                        let j = self.decks[d].jump_beats;
                        if ui.button("<<").on_hover_text("Beat Jump zurück").clicked() {
                            self.send(Command::BeatJump(d, -j));
                        }
                        egui::ComboBox::from_id_salt(("jump", d)).width(48.0).selected_text(format!("{j}")).show_ui(ui, |ui| {
                            for b in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0] {
                                ui.selectable_value(&mut self.decks[d].jump_beats, b, format!("{b} Beats"));
                            }
                        });
                        if ui.button(">>").on_hover_text("Beat Jump vor").clicked() {
                            self.send(Command::BeatJump(d, j));
                        }
                        ui.separator();
                        if ui.button("MEM").on_hover_text("Memory Cue setzen").clicked() {
                            self.send(Command::MemoryStore(d));
                        }
                        if ui.small_button("<").clicked() {
                            self.send(Command::MemoryCall(d, false));
                        }
                        if ui.small_button(">").clicked() {
                            self.send(Command::MemoryCall(d, true));
                        }
                        ui.label(RichText::new(format!("{}", ds.memory_cues)).weak());
                        ui.separator();
                        if ui.small_button("b").on_hover_text("Tonart -1").clicked() {
                            self.send(Command::Key(d, ds.key - 1));
                        }
                        ui.label(RichText::new(format!("{:+}", ds.key)).monospace());
                        if ui.small_button("#").on_hover_text("Tonart +1").clicked() {
                            self.send(Command::Key(d, ds.key + 1));
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button(format!("Instant Double von {}", other + 1)).clicked() {
                            self.send(Command::InstantDouble(d));
                        }
                    });
                });
                // tempo fader: minus at the top like the hardware
                ui.vertical(|ui| {
                    ui.label(RichText::new("TEMPO").small());
                    let mut v = -ds.tempo_fader;
                    let r = ui.add(egui::Slider::new(&mut v, -1.0..=1.0).vertical().show_value(false));
                    if r.changed() {
                        self.send(Command::TempoFader(d, -v));
                    }
                    if r.double_clicked() {
                        self.send(Command::TempoFader(d, 0.0));
                    }
                    let range = ds.tempo_range;
                    let label = if range >= 1.0 { "WIDE".to_string() } else { format!("±{:.0}", range * 100.0) };
                    if ui.small_button(label).on_hover_text("Tempo-Bereich").clicked() {
                        let idx = rbx_engine::deck::TEMPO_RANGES.iter().position(|&r| (r - range).abs() < 1e-9).unwrap_or(0);
                        self.send(Command::TempoRange(d, (idx + 1) % rbx_engine::deck::TEMPO_RANGES.len()));
                    }
                });
            });
        });
    }

    fn mixer_panel(&mut self, ui: &mut Ui, s: &MixerState) {
        egui::Frame::group(ui.style()).fill(Color32::from_gray(20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                for d in 0..DECKS {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("CH {}", d + 1)).strong());
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                let mut t = self.decks[d].trim_db;
                                if widgets::knob(ui, &mut t, -12.0..=12.0, 0.0, "TRIM").changed() {
                                    self.decks[d].trim_db = t;
                                    self.send(Command::Trim(d, 10f32.powf(t / 20.0)));
                                }
                                for (band, name) in [(2usize, "HI"), (1, "MID"), (0, "LOW")] {
                                    let mut v = s.eq[d][band];
                                    if widgets::knob(ui, &mut v, 0.0..=1.0, 0.5, name).changed() {
                                        self.send(Command::Eq(d, band, v));
                                    }
                                }
                                let mut c = self.decks[d].color_knob;
                                if widgets::knob(ui, &mut c, 0.0..=1.0, 0.5, "COLOR").changed() {
                                    self.decks[d].color_knob = c;
                                    self.send(Command::ColorKnob(d, c));
                                }
                            });
                            // channel fader with level meter
                            ui.vertical(|ui| {
                                ui.add_space(30.0);
                                ui.horizontal_top(|ui| {
                                    widgets::meter_v(ui, s.channel_peak[d], 170.0);
                                    ui.spacing_mut().slider_width = 170.0;
                                    let mut v = s.decks[d].volume;
                                    if ui.add(egui::Slider::new(&mut v, 0.0..=1.0).vertical().show_value(false)).changed() {
                                        self.send(Command::Volume(d, v));
                                    }
                                });
                            });
                        });
                        if ui.add(egui::Button::new("HP CUE").selected(s.cue[d])).on_hover_text("Kanal in den Kopfhörer").clicked() {
                            self.send(Command::HeadphoneCue(d, !s.cue[d]));
                        }
                    });
                }
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(RichText::new("MASTER").strong());
                    let mut g = self.master_gain;
                    if widgets::knob(ui, &mut g, 0.0..=2.0, 1.0, "LEVEL").changed() {
                        self.master_gain = g;
                        self.send(Command::MasterGain(g));
                    }
                    if ui.add(egui::Button::new("HP MST").selected(s.master_cue)).clicked() {
                        self.send(Command::MasterCue(!s.master_cue));
                    }
                    let mut sv = self.sampler_volume;
                    if widgets::knob(ui, &mut sv, 0.0..=1.0, 0.8, "SMP VOL").changed() {
                        self.sampler_volume = sv;
                        self.send(Command::SamplerVolume(sv));
                    }
                    let mut curve = s.crossfader_curve;
                    if widgets::knob(ui, &mut curve, 0.0..=1.0, 0.0, "CURVE").changed() {
                        self.send(Command::CrossfaderCurve(curve));
                    }
                });
            });
            ui.label(RichText::new("CROSSFADER").small());
            let mut x = s.crossfader;
            ui.spacing_mut().slider_width = ui.available_width() - 8.0;
            if ui.add(egui::Slider::new(&mut x, -1.0..=1.0).show_value(false)).changed() {
                self.send(Command::Crossfader(x));
            }
        });
    }

    fn beat_fx_panel(&mut self, ui: &mut Ui, s: &MixerState) {
        let fx = s.beat_fx;
        egui::Frame::group(ui.style()).fill(Color32::from_gray(20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("BEAT FX").strong().color(ACCENT));
                egui::ComboBox::from_id_salt("beatfx").selected_text(format!("{:?}", fx.kind).to_uppercase()).show_ui(ui, |ui| {
                    for k in BeatFxKind::ALL {
                        if ui.selectable_label(fx.kind == k, format!("{k:?}").to_uppercase()).clicked() {
                            self.send(Command::BeatFxSelect(k));
                        }
                    }
                });
                if ui.small_button("<").clicked() {
                    self.send(Command::BeatFxBeat(-1));
                }
                ui.label(RichText::new(fmt_beats(fx.beats)).monospace());
                if ui.small_button(">").clicked() {
                    self.send(Command::BeatFxBeat(1));
                }
                ui.label(RichText::new(format!("{:.1} BPM", fx.bpm)).weak());
            });
            ui.horizontal(|ui| {
                let assigns = [(FxAssign::Channel(0), "CH1"), (FxAssign::Channel(1), "CH2"), (FxAssign::Master, "MST"), (FxAssign::Sampler, "SMP")];
                for (a, n) in assigns {
                    if ui.add(egui::Button::new(n).selected(fx.assign == a)).clicked() {
                        self.send(Command::BeatFxAssign(a));
                    }
                }
                let mut depth = self.fx_depth;
                if widgets::knob(ui, &mut depth, 0.0..=1.0, 0.5, "DEPTH").changed() {
                    self.fx_depth = depth;
                    self.send(Command::BeatFxDepth(depth));
                }
                let on_fill = if fx.on { Color32::from_rgb(200, 60, 20) } else { Color32::from_gray(60) };
                if ui.add(egui::Button::new(RichText::new("ON").strong()).fill(on_fill).min_size(Vec2::new(44.0, 30.0))).clicked() {
                    self.send(Command::BeatFxOn(!fx.on));
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("SOUND COLOR FX").strong().color(ACCENT))
                    .on_hover_text("Effekt wählen, dann den COLOR-Regler des Kanals im Mixer drehen");
                let mut sub = self.color_sub;
                if ui.add(egui::Slider::new(&mut sub, 0.0..=1.0).text("PARAM").show_value(false)).changed() {
                    self.color_sub = sub;
                    self.send(Command::ColorSub(sub));
                }
            });
            ui.horizontal_wrapped(|ui| {
                for fx in ColorFx::ALL {
                    let on = self.color_fx == Some(fx);
                    if ui.add(egui::Button::new(format!("{fx:?}").to_uppercase()).selected(on)).clicked() {
                        self.color_fx = if on { None } else { Some(fx) };
                        self.send(Command::ColorFxSelect(self.color_fx));
                    }
                }
            });
        });
    }

    fn sampler_panel(&mut self, ui: &mut Ui, s: &MixerState) {
        egui::Frame::group(ui.style()).fill(Color32::from_gray(20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("SAMPLER").strong().color(ACCENT));
            egui::Grid::new("sampler").spacing(Vec2::new(3.0, 3.0)).show(ui, |ui| {
                for slot in 0..16 {
                    let playing = s.sampler_playing & (1 << slot) != 0;
                    let name = self.sampler_names.get(slot).cloned().unwrap_or_default();
                    let short: String = name.chars().take(8).collect();
                    let fill = if playing { Color32::from_rgb(160, 90, 0) } else { Color32::from_gray(45) };
                    let r = ui.add(egui::Button::new(RichText::new(short).small()).fill(fill).min_size(Vec2::new(58.0, 22.0)));
                    if r.clicked() {
                        self.send(Command::SamplerStart(slot));
                    }
                    if r.secondary_clicked() {
                        self.send(Command::SamplerStop(slot));
                    }
                    r.on_hover_text(format!("{name} (Rechtsklick: Stopp)"));
                    if slot % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
        });
    }

    fn library_panel(&mut self, ui: &mut Ui) {
        self.refresh_rows();
        let missing: Vec<PathBuf> = self.rows.iter().filter(|r| r.source == RowSource::Missing).map(|r| r.path.clone()).collect();
        ui.horizontal(|ui| {
            ui.label("Suche");
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Suche: Interpret, Titel, Datei").desired_width(260.0));
            ui.label(format!("{} von {}", self.filtered.len(), self.rows.len()));
            ui.separator();
            let b = self.loader.batch();
            let label = format!("Fehlende analysieren ({})", missing.len());
            if ui.add_enabled(!b.running && !missing.is_empty(), egui::Button::new(label)).clicked() {
                self.loader.analyze_batch(missing.clone());
            }
            if let Some(i) = self.selected {
                if let Some(r) = self.rows.get(i) {
                    ui.separator();
                    if ui.button("> Deck 1").clicked() {
                        let p = r.path.to_string_lossy().into_owned();
                        self.load(0, &p);
                    } else if ui.button("> Deck 2").clicked() {
                        let p = r.path.to_string_lossy().into_owned();
                        self.load(1, &p);
                    }
                }
            }
            ui.label(RichText::new("Doppelklick lädt in das freie / angehaltene Deck; Dateien können ins Fenster gezogen werden").small().weak());
        });
        ui.separator();
        let row_h = 20.0;
        // column widths: deck, artist, title (rest), BPM, key, analysis
        let sp = ui.spacing().item_spacing.x;
        let fixed = 36.0 + 220.0 + 80.0 + 50.0 + 90.0;
        let title_w = (ui.available_width() - fixed - 6.0 * sp - 14.0).max(120.0);
        let widths = [36.0, 220.0, title_w, 80.0, 50.0, 90.0];
        ui.horizontal(|ui| {
            for (n, w) in ["Deck", "Interpret", "Titel", "BPM", "Key", "Analyse"].iter().zip(widths) {
                ui.add_sized(Vec2::new(w, 16.0), egui::Label::new(RichText::new(*n).small().strong()));
            }
        });
        let loaded: Vec<Option<PathBuf>> = (0..DECKS).map(|d| self.loader.deck(d).source.map(PathBuf::from)).collect();
        let mut load: Option<(usize, String)> = None;
        let s = self.state();
        let n = self.filtered.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, n, |ui, range| {
            for k in range {
                let i = self.filtered[k];
                let r = &self.rows[i];
                let loaded_in: Vec<usize> = (0..DECKS).filter(|&d| loaded[d].as_ref() == Some(&r.path)).collect();
                let resp = ui
                    .horizontal(|ui| {
                        ui.set_height(row_h);
                        let deck_txt = loaded_in.iter().map(|d| (d + 1).to_string()).collect::<Vec<_>>().join(",");
                        ui.add_sized(Vec2::new(widths[0], row_h), egui::Label::new(RichText::new(deck_txt).color(ACCENT).strong()));
                        let sel = self.selected == Some(i);
                        let a = ui.add_sized(Vec2::new(widths[1], row_h), egui::Button::selectable(sel, r.artist.as_str()).truncate());
                        let t = ui.add_sized(Vec2::new(widths[2], row_h), egui::Button::selectable(sel, r.title.as_str()).truncate());
                        let bpm = r.bpm.map_or("–".into(), |b| format!("{b:.2}{}", if r.variable_tempo { "*" } else { "" }));
                        ui.add_sized(Vec2::new(widths[3], row_h), egui::Label::new(RichText::new(bpm).monospace()));
                        ui.add_sized(Vec2::new(widths[4], row_h), egui::Label::new(r.key.as_deref().unwrap_or("–")));
                        let col = match r.source {
                            RowSource::Rekordbox => Color32::LIGHT_BLUE,
                            RowSource::Own => Color32::LIGHT_GREEN,
                            RowSource::Missing => Color32::GRAY,
                        };
                        ui.add_sized(Vec2::new(widths[5], row_h), egui::Label::new(RichText::new(r.source.label()).color(col)));
                        a.union(t)
                    })
                    .inner;
                if resp.clicked() {
                    self.selected = Some(i);
                }
                if resp.double_clicked() {
                    // a deck that is empty or paused, else deck 1
                    let d = (0..DECKS).find(|&d| !s.decks[d].loaded).or_else(|| (0..DECKS).find(|&d| !s.decks[d].playing)).unwrap_or(0);
                    load = Some((d, r.path.to_string_lossy().into_owned()));
                }
            }
        });
        if let Some((d, p)) = load {
            self.load(d, &p);
        }
    }

    fn log_panel(&self, ui: &mut Ui) {
        for line in self.log.recent(4) {
            ui.label(RichText::new(line).small().monospace().weak());
        }
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if dropped.is_empty() {
            return;
        }
        let mut files = Vec::new();
        for p in dropped {
            if p.is_dir() {
                files.extend(rbx_engine::session::audio_files(&p));
            } else {
                files.push(p);
            }
        }
        self.log.say(format!("{} Datei(en) hinzugefügt", files.len()));
        self.loader.add_files(&files);
    }

    /// Keyboard shortcuts (not while typing in a text field), see [`SHORTCUTS`].
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let events: Vec<(egui::Key, bool, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed, repeat: false, modifiers, .. } => Some((*key, *pressed, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        let s = self.state();
        use egui::Key;
        for (key, pressed, m) in events {
            let hot = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8];
            if let Some(i) = hot.iter().position(|&k| k == key) {
                let d = usize::from(m.shift);
                self.send(if pressed { Command::HotCuePress(d, i) } else { Command::HotCueRelease(d, i) });
                continue;
            }
            match (key, pressed) {
                (Key::Z, true) => self.send(Command::CuePress(0)),
                (Key::Z, false) => self.send(Command::CueRelease(0)),
                (Key::X, true) => self.send(Command::TogglePlay(0)),
                (Key::C, true) => self.send(Command::Sync(0)),
                (Key::B, true) => self.send(Command::CuePress(1)),
                (Key::B, false) => self.send(Command::CueRelease(1)),
                (Key::N, true) => self.send(Command::TogglePlay(1)),
                (Key::M, true) => self.send(Command::Sync(1)),
                (Key::ArrowLeft, true) => self.send(Command::Crossfader((s.crossfader - 0.1).max(-1.0))),
                (Key::ArrowRight, true) => self.send(Command::Crossfader((s.crossfader + 0.1).min(1.0))),
                (Key::ArrowUp | Key::ArrowDown, true) => {
                    let pos = self.selected.and_then(|i| self.filtered.iter().position(|&f| f == i));
                    let next = match (key, pos) {
                        (Key::ArrowUp, Some(p)) => p.saturating_sub(1),
                        (Key::ArrowDown, Some(p)) => (p + 1).min(self.filtered.len().saturating_sub(1)),
                        _ => 0,
                    };
                    self.selected = self.filtered.get(next).copied();
                }
                (Key::Enter, true) => {
                    if let Some(r) = self.selected.and_then(|i| self.rows.get(i)) {
                        let p = r.path.to_string_lossy().into_owned();
                        self.load(usize::from(m.shift), &p);
                    }
                }
                _ => {}
            }
        }
    }

    fn side_effects(&self) {
        if let Some(c) = &self.ctl {
            rbx_engine::session::side_effects(&c.controller, &self.loader);
        }
    }

    /// Draw one frame (also used by the tests).
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(16));
        self.handle_drops(&ctx);
        self.shortcuts(&ctx);
        self.side_effects();
        let s = self.state();
        self.log_stats(&s);
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui, &s));
        egui::Panel::bottom("log").show(ui, |ui| self.log_panel(ui));
        egui::Panel::bottom("library").resizable(true).default_size(280.0).min_size(120.0).show(ui, |ui| self.library_panel(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            for d in 0..DECKS {
                let info = self.loader.deck(d);
                let ds = s.decks[d];
                let grid = info.analysis.as_ref().and_then(|a| a.grid).or_else(|| ds.bpm.map(|_| rbx_engine::deck::Grid { bpm: 0.0, first_beat_s: 0.0 }).filter(|_| false));
                let map = info.analysis.as_ref().and_then(|a| a.beats.clone());
                let acts = widgets::detail(ui, info.wave.as_deref(), map.as_deref(), grid, &ds, self.zoom_s, 64.0, &format!("Wellenform Deck {}", d + 1));
                for a in acts {
                    match a {
                        DetailAction::Touch(t) => self.send(Command::JogTouch(d, t)),
                        DetailAction::Scrub(secs) => self.send(Command::SeekBy(d, secs)),
                        DetailAction::Nudge(n) => self.send(Command::Jog(d, n * 8)),
                    }
                }
                ui.add_space(2.0);
            }
            ui.add_space(4.0);
            // three fixed columns: deck 1 | mixer + FX | deck 2
            let area = ui.available_rect_before_wrap();
            let gap = 6.0;
            let mixer_w = 300.0f32.min(area.width() * 0.3);
            let deck_w = ((area.width() - mixer_w) / 2.0 - gap).max(200.0);
            let col = |x: f32, w: f32| egui::Rect::from_min_size(egui::pos2(x, area.top()), Vec2::new(w, area.height()));
            let rects = [
                col(area.left(), deck_w),
                col(area.left() + deck_w + gap, mixer_w),
                col(area.left() + deck_w + mixer_w + 2.0 * gap, deck_w),
            ];
            let top_down = egui::Layout::top_down(egui::Align::Min);
            ui.scope_builder(egui::UiBuilder::new().max_rect(rects[0]).layout(top_down), |ui| {
                self.deck_panel(ui, 0, &s);
                self.beat_fx_panel(ui, &s);
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(rects[1]).layout(top_down), |ui| {
                egui::ScrollArea::vertical().id_salt("mixer").show(ui, |ui| self.mixer_panel(ui, &s));
            });
            ui.scope_builder(egui::UiBuilder::new().max_rect(rects[2]).layout(top_down), |ui| {
                self.deck_panel(ui, 1, &s);
                self.sampler_panel(ui, &s);
            });
        });
        if self.show_settings {
            self.settings_window(&ctx);
        }
        self.persist();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

/// Keyboard shortcuts (also shown by the "Tasten" button).
pub const SHORTCUTS: &str = "Deck 1: Z = CUE (halten), X = PLAY/PAUSE, C = SYNC, 1-8 = Hot Cues A-H\n\
Deck 2: B = CUE (halten), N = PLAY/PAUSE, M = SYNC, Umschalt+1-8 = Hot Cues A-H\n\
Pfeil links/rechts = Crossfader, Pfeil hoch/runter = Track wählen, Enter = in Deck 1 laden (Umschalt: Deck 2)";

/// Start the program again with the same arguments and close this window.
fn restart(ctx: &egui::Context) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).spawn();
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

/// `m:ss.t`
pub fn fmt_time(s: f64) -> String {
    let s = s.max(0.0);
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u64, s - m * 60.0)
}

fn fmt_beats(b: f64) -> String {
    if b >= 1.0 {
        format!("{b}")
    } else {
        format!("1/{}", (1.0 / b).round())
    }
}
