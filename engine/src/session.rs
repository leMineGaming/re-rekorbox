//! What a running player needs around the [`crate::output::Output`]: loading files on worker
//! threads, the library (rekordbox analyses plus own ones), automatic analysis of new files, deck
//! waveforms, sampler contents, the DJ Link participant and the MIDI controller. Shared by `rbx-play`
//! (text commands) and `rbx-ui` (window).
//!
//! Messages go to a [`Log`] (also printed to stdout) so a UI can show them.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::Duration;

use rekordbox_formats::anlz::tags::Tag;
use rekordbox_formats::anlz::waveform::ColorColumn;
use rekordbox_formats::anlz::{AnlzFile, FourCC};
use rekordbox_formats::library::{default_anlz_root, default_music_root, is_audio, read_track, Library};

use crate::analysis::TrackAnalysis;
use crate::controller::Controller;
use crate::deck::Grid;
use crate::mixer::{Command, MixerState, DECKS};
use crate::pcm::{decode_file, Pcm};

pub type Shared<T> = Arc<Mutex<T>>;

/// Recent messages (newest last), also printed to stdout.
#[derive(Clone, Default)]
pub struct Log(Shared<VecDeque<String>>);

impl Log {
    const KEEP: usize = 200;

    pub fn say(&self, msg: impl Into<String>) {
        let msg = msg.into();
        println!("{msg}");
        let mut q = self.0.lock().unwrap();
        if q.len() >= Self::KEEP {
            q.pop_front();
        }
        q.push_back(msg);
    }

    /// The last `n` messages, oldest first.
    pub fn recent(&self, n: usize) -> Vec<String> {
        let q = self.0.lock().unwrap();
        q.iter().skip(q.len().saturating_sub(n)).cloned().collect()
    }
}

/// Load a source: file, `tone` (440 Hz) or `click:BPM` (click track with grid).
pub fn source(spec: &str, sample_rate: u32) -> Result<(Arc<Pcm>, Option<Grid>), String> {
    if spec == "tone" {
        return Ok((Arc::new(Pcm::sine(sample_rate, 440.0, 60.0, 0.3)), None));
    }
    if let Some(bpm) = spec.strip_prefix("click:") {
        let bpm: f64 = bpm.parse().map_err(|_| format!("{spec}: invalid BPM"))?;
        return Ok((Arc::new(Pcm::click_track(sample_rate, bpm, 120.0)), Some(Grid { bpm, first_beat_s: 0.0 })));
    }
    let pcm = decode_file(Path::new(spec)).map_err(|e| format!("{spec}: {e}"))?;
    Ok((Arc::new(pcm), None))
}

/// Waveform of a loaded track for display: colour detail columns at 150 per second (`PWV5` layout).
#[derive(Debug, Clone, PartialEq)]
pub struct DeckWave {
    pub columns: Vec<ColorColumn>,
    /// Columns per second.
    pub rate: f64,
    /// True when it came from the analysis files, false when computed from the audio.
    pub from_anlz: bool,
}

impl DeckWave {
    /// Colour detail from the `.EXT` file of an analysis folder.
    pub fn from_anlz_dir(dir: &Path) -> Option<DeckWave> {
        let bytes = std::fs::read(dir.join("ANLZ0000.EXT")).ok()?;
        let file = AnlzFile::parse(&bytes).ok()?;
        let raw = file.tags.iter().find(|t| t.fourcc == FourCC::PWV5)?;
        let Ok(Tag::Waveform(w)) = Tag::from_raw(raw) else { return None };
        let rate = f64::from(w.entries_per_second().filter(|&r| r > 0).unwrap_or(150));
        Some(DeckWave { columns: w.color_columns()?, rate, from_anlz: true })
    }

    /// Computed from decoded audio with the own analyzer's waveform code.
    pub fn from_pcm(pcm: &Pcm) -> DeckWave {
        let rec = crate::analyze::wave::ms_records(pcm);
        let w = crate::analyze::wave::waveforms(&rec, pcm.len(), pcm.sample_rate);
        DeckWave { columns: w.color, rate: 150.0, from_anlz: false }
    }

    /// Column at a time in seconds.
    pub fn at(&self, t: f64) -> Option<&ColorColumn> {
        if t < 0.0 {
            return None;
        }
        self.columns.get((t * self.rate) as usize)
    }
}

/// What the session knows about one deck (besides the engine's [`crate::deck::DeckState`]).
#[derive(Debug, Clone, Default)]
pub struct DeckInfo {
    /// Source that was loaded (file path, `tone`, `click:BPM`).
    pub source: Option<String>,
    pub title: Option<String>,
    pub key: Option<String>,
    pub analysis: Option<Arc<TrackAnalysis>>,
    pub wave: Option<Arc<DeckWave>>,
    /// Work in progress ("decoding ...", "analysing ...").
    pub busy: Option<String>,
}

/// Progress of a batch analysis.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Batch {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub running: bool,
}

/// All audio files below `root` (sorted, recursive).
pub fn audio_files(root: &Path) -> Vec<PathBuf> {
    fn walk(p: &Path, out: &mut Vec<PathBuf>) {
        let Ok(it) = std::fs::read_dir(p) else { return };
        let mut v: Vec<PathBuf> = it.flatten().map(|e| e.path()).collect();
        v.sort();
        for e in v {
            if e.is_dir() {
                walk(&e, out);
            } else if is_audio(&e) {
                out.push(e);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

/// Loads audio on worker threads and attaches the library analysis (grid, beats, cues).
#[derive(Clone)]
pub struct Loader {
    tx: Sender<Command>,
    pub sample_rate: u32,
    library: Arc<RwLock<Option<Library>>>,
    decks: Shared<[DeckInfo; DECKS]>,
    browser_pending: Arc<AtomicBool>,
    library_busy: Arc<AtomicBool>,
    /// Incremented whenever the library or the file list changes.
    generation: Arc<AtomicU64>,
    /// All audio files of the music folder (also those without analysis).
    music_files: Arc<RwLock<Vec<PathBuf>>>,
    batch: Shared<Batch>,
    /// Analyse loaded files that have no analysis.
    pub auto_analyze: bool,
    /// Where own analyses are written and read (default `%APPDATA%/rekordbox-rs/ANLZ`).
    pub analysis_root: Option<PathBuf>,
    pub log: Log,
}

impl Loader {
    pub fn new(tx: Sender<Command>, sample_rate: u32, log: Log) -> Loader {
        Loader {
            tx,
            sample_rate,
            library: Default::default(),
            decks: Default::default(),
            browser_pending: Default::default(),
            library_busy: Default::default(),
            generation: Default::default(),
            music_files: Default::default(),
            batch: Default::default(),
            auto_analyze: true,
            analysis_root: crate::analyze::default_output_root(),
            log,
        }
    }

    /// Read the library in the background; analyses are then attached to already loaded decks.
    pub fn start_library(&self, anlz: Option<PathBuf>, music: Option<PathBuf>) {
        let anlz = anlz.or_else(default_anlz_root);
        let music = music.or_else(default_music_root);
        let mut roots: Vec<PathBuf> = anlz.into_iter().collect();
        roots.extend(self.analysis_root.clone().filter(|p| p.is_dir()));
        if roots.is_empty() {
            self.log.say("library: no analysis folder found");
            return;
        }
        let me = self.clone();
        me.library_busy.store(true, Ordering::Relaxed);
        std::thread::spawn(move || {
            let files = music.as_deref().map(audio_files).unwrap_or_default();
            let lib = Library::build_multi(&roots, &music.into_iter().collect::<Vec<_>>());
            let r = lib.report;
            me.log.say(format!(
                "library: {} tracks, {} with audio file, {} with beat grid; {} audio files in the music folder",
                r.tracks,
                r.matched,
                r.with_grid,
                files.len()
            ));
            *me.music_files.write().unwrap() = files;
            *me.library.write().unwrap() = Some(lib);
            me.library_busy.store(false, Ordering::Relaxed);
            me.generation.fetch_add(1, Ordering::Relaxed);
            me.browser_pending.store(true, Ordering::Relaxed);
            let loaded: Vec<Option<String>> = me.decks.lock().unwrap().iter().map(|d| d.source.clone()).collect();
            for (deck, path) in loaded.iter().enumerate() {
                if let Some(p) = path {
                    me.apply(deck, p);
                }
            }
        });
    }

    /// Changes whenever the library or the file list changes (for views that cache rows).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// All audio files of the music folder.
    pub fn music_files(&self) -> Vec<PathBuf> {
        self.music_files.read().unwrap().clone()
    }

    /// Add files (e.g. dropped onto the window) to the file list.
    pub fn add_files(&self, files: &[PathBuf]) {
        let mut m = self.music_files.write().unwrap();
        for f in files {
            if is_audio(f) && !m.contains(f) {
                m.push(f.clone());
            }
        }
        drop(m);
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn batch(&self) -> Batch {
        self.batch.lock().unwrap().clone()
    }

    fn store_analysis(&self, dir: &Path) {
        if let Some(track) = read_track(dir) {
            let mut g = self.library.write().unwrap();
            match g.as_mut() {
                Some(lib) => lib.insert(track),
                None => {
                    let mut lib = Library::default();
                    lib.insert(track);
                    *g = Some(lib);
                }
            }
        }
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Analyse files in the background (own analyzer, a few worker threads) and add the results to
    /// the library. Ignored while another batch runs.
    pub fn analyze_batch(&self, files: Vec<PathBuf>) {
        let Some(root) = self.analysis_root.clone() else { return };
        {
            let mut b = self.batch.lock().unwrap();
            if b.running || files.is_empty() {
                return;
            }
            *b = Batch { total: files.len(), done: 0, failed: 0, running: true };
        }
        self.log.say(format!("analysing {} files ...", files.len()));
        let queue = Arc::new(Mutex::new(files));
        // leave cores for audio and UI
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(2)).clamp(1, 4);
        let left = Arc::new(std::sync::atomic::AtomicUsize::new(threads));
        for _ in 0..threads {
            let (me, queue, root, left) = (self.clone(), queue.clone(), root.clone(), left.clone());
            std::thread::spawn(move || {
                loop {
                    let Some(f) = queue.lock().unwrap().pop() else { break };
                    let r = crate::analyze::analyze_file(&f).map_err(|e| e.to_string());
                    let ok = match r.and_then(|r| crate::analyze::write_anlz(&r, &root).map_err(|e| e.to_string())) {
                        Ok(dir) => {
                            me.store_analysis(&dir);
                            true
                        }
                        Err(e) => {
                            me.log.say(format!("{}: {e}", f.display()));
                            false
                        }
                    };
                    let mut b = me.batch.lock().unwrap();
                    b.done += 1;
                    b.failed += usize::from(!ok);
                }
                if left.fetch_sub(1, Ordering::Relaxed) == 1 {
                    let b = {
                        let mut b = me.batch.lock().unwrap();
                        b.running = false;
                        b.clone()
                    };
                    me.log.say(format!("analysis done: {} files, {} failed", b.done, b.failed));
                }
            });
        }
    }

    /// True while the library is being read.
    pub fn library_loading(&self) -> bool {
        self.library_busy.load(Ordering::Relaxed)
    }

    pub fn library(&self) -> Option<RwLockReadGuard<'_, Option<Library>>> {
        let g = self.library.read().ok()?;
        g.is_some().then_some(g)
    }

    /// Playable library tracks for the controller's browser, once after the library became ready.
    pub fn take_library_for_browser(&self) -> Option<Vec<PathBuf>> {
        if !self.browser_pending.swap(false, Ordering::Relaxed) {
            return None;
        }
        let g = self.library.read().ok()?;
        Some(g.as_ref()?.playable().filter_map(|t| t.path.clone()).collect())
    }

    pub fn title(&self, deck: usize) -> Option<String> {
        self.decks.lock().unwrap()[deck].title.clone()
    }

    /// Snapshot of what is known about a deck.
    pub fn deck(&self, deck: usize) -> DeckInfo {
        self.decks.lock().unwrap()[deck].clone()
    }

    /// Send the analysis of `path` to `deck`; returns false when the library has none.
    pub fn apply(&self, deck: usize, path: &str) -> bool {
        let g = self.library.read().unwrap();
        let Some(lib) = g.as_ref() else { return false };
        let Some(t) = lib.by_path(Path::new(path)) else { return false };
        let a = Arc::new(TrackAnalysis::from_library(t));
        self.log.say(format!(
            "deck {}: {} – {}{}",
            deck + 1,
            a.title,
            a.grid.map_or("no beat grid".into(), |g| format!("{:.2} BPM", g.bpm)),
            if a.variable_tempo { " (variable tempo)" } else { "" }
        ));
        let wave = DeckWave::from_anlz_dir(&t.anlz_dir).map(Arc::new);
        {
            let mut d = self.decks.lock().unwrap();
            let info = &mut d[deck];
            info.title = Some(a.title.clone());
            info.key = t.key.clone();
            info.analysis = Some(a.clone());
            if wave.is_some() {
                info.wave = wave;
            }
        }
        let _ = self.tx.send(Command::ApplyAnalysis { deck, analysis: a });
        true
    }

    fn set_busy(&self, deck: usize, busy: Option<String>) {
        self.decks.lock().unwrap()[deck].busy = busy;
    }

    /// Analyse a loaded file that has no analysis yet (own analyzer, SPEC §5j), store it in the
    /// own analysis folder, add it to the library and apply it to the deck if it is still loaded.
    pub fn analyze_missing(&self, deck: usize, path: &str, pcm: &Pcm) {
        let p = Path::new(path);
        if !p.is_file() {
            return;
        }
        let Some(root) = self.analysis_root.clone() else { return };
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.log.say(format!("deck {}: no analysis – analysing {name} ...", deck + 1));
        self.set_busy(deck, Some("analysing ...".into()));
        let t = std::time::Instant::now();
        let r = crate::analyze::analyze_pcm(p, pcm);
        let dir = match crate::analyze::write_anlz(&r, &root) {
            Ok(d) => d,
            Err(e) => {
                self.log.say(format!("analysis not saved: {e}"));
                self.set_busy(deck, None);
                return;
            }
        };
        self.log.say(format!(
            "deck {}: analysed in {:.1} s – {}{}",
            deck + 1,
            t.elapsed().as_secs_f64(),
            r.grid.as_ref().map_or("no beat grid".into(), |g| format!("{:.2} BPM", g.bpm)),
            r.key.map_or(String::new(), |(k, _)| format!(", key {} ({})", k.name(), k.camelot()))
        ));
        self.store_analysis(&dir);
        self.set_busy(deck, None);
        if self.decks.lock().unwrap()[deck].source.as_deref() == Some(path) {
            self.apply(deck, path);
        }
    }

    /// Decode on a worker thread, load, then attach the analysis (or analyse).
    pub fn load(&self, deck: usize, spec: &str) {
        let me = self.clone();
        let spec = spec.to_string();
        let name = Path::new(&spec).file_name().map_or(spec.clone(), |n| n.to_string_lossy().into_owned());
        self.set_busy(deck, Some(format!("loading {name} ...")));
        std::thread::spawn(move || match source(&spec, me.sample_rate) {
            Ok((pcm, grid)) => {
                me.log.say(format!("deck {}: loaded {spec} ({:.1} s)", deck + 1, pcm.duration_s()));
                let _ = me.tx.send(Command::Load { deck, pcm: pcm.clone(), grid });
                let wave = Arc::new(DeckWave::from_pcm(&pcm));
                *me.decks.lock().unwrap().get_mut(deck).unwrap() =
                    DeckInfo { source: Some(spec.clone()), title: Some(name), wave: Some(wave), ..Default::default() };
                if !me.apply(deck, &spec) && me.auto_analyze {
                    me.analyze_missing(deck, &spec, &pcm);
                }
            }
            Err(e) => {
                me.log.say(e);
                me.set_busy(deck, None);
            }
        });
    }
}

/// Fill the 16 sampler slots: audio files of `dir`, the rest with the built-in sounds. Returns the
/// slot names.
pub fn fill_sampler(tx: &Sender<Command>, sample_rate: u32, dir: Option<&Path>) -> Vec<String> {
    use crate::sampler::{builtin_sounds, load_folder, BUILTIN_NAMES};
    let files = dir.map(load_folder).unwrap_or_default();
    let builtin = builtin_sounds(sample_rate);
    let mut names = Vec::new();
    for (slot, b) in builtin.into_iter().enumerate() {
        match files.get(slot) {
            Some((p, pcm)) => {
                names.push(p.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned()));
                let _ = tx.send(Command::SamplerLoad(slot, Arc::new(pcm.clone())));
            }
            None => {
                names.push(BUILTIN_NAMES[slot].to_string());
                let _ = tx.send(Command::SamplerLoad(slot, Arc::new(b)));
            }
        }
    }
    names
}

/// DJ Link in the background: the link and which decks follow which device.
pub struct LinkHandle {
    pub link: Shared<rekordbox_formats::djlink_net::Link>,
    /// Per deck: `Some(device)` = follow that device (`Some(None)` = first device with beats).
    pub follow: Shared<[Option<Option<u8>>; DECKS]>,
    /// Adapter name and address.
    pub adapter: String,
}

/// Join DJ Link, send beat packets of the playing deck (the sync master, else the louder one) and
/// let decks follow external devices.
pub fn start_link(iface: Option<&str>, state: Shared<MixerState>, tx: Sender<Command>, log: &Log) -> Result<LinkHandle, String> {
    use rekordbox_formats::djlink_net::{external_beat, BeatClock, Link, LinkConfig};
    use rekordbox_formats::djlink_participant::Config;
    use rekordbox_formats::netif;
    let list = netif::list();
    let i = netif::choose(&list, iface).ok_or(format!("no network adapter matching {iface:?} (see `cargo run --example netif`)"))?;
    let adapter = format!("{} {}", i.name, i.ip);
    log.say(format!("DJ Link: {adapter} (broadcast {})", i.broadcast()));
    let cfg = LinkConfig::new(Config::new("rekordbox-rs", i.mac, i.ip), i.broadcast());
    let link = Arc::new(Mutex::new(Link::start(cfg).map_err(|e| format!("{e} (is rekordbox running?)"))?));
    let follow: Shared<[Option<Option<u8>>; DECKS]> = Arc::new(Mutex::new([None, None]));
    let (l, fw) = (link.clone(), follow.clone());
    std::thread::spawn(move || {
        let mut clock = BeatClock::default();
        let mut applied: [Option<std::time::Instant>; DECKS] = [None, None];
        loop {
            std::thread::sleep(Duration::from_millis(5));
            let s = *state.lock().unwrap();
            let playing: Vec<usize> = (0..DECKS).filter(|&d| s.decks[d].playing && s.decks[d].beat.is_some()).collect();
            let deck = match s.master {
                Some(m) if playing.contains(&m) => Some(m),
                _ => playing.into_iter().max_by(|&a, &b| s.channel_peak[a].total_cmp(&s.channel_peak[b])),
            };
            let beat = deck.and_then(|d| s.decks[d].bpm.map(|bpm| (s.decks[d].beat, bpm)));
            let packet = match beat {
                Some((b, bpm)) => clock.update("rekordbox-rs", b, bpm),
                None => clock.update("rekordbox-rs", None, 120.0),
            };
            let Ok(link) = l.lock() else { return };
            if let Some(p) = packet {
                link.send_beat(&p);
            }
            // external sync: re-align on every new beat packet of the followed device
            let targets = *fw.lock().unwrap();
            let ls = link.state();
            for (d, t) in targets.iter().enumerate() {
                let Some(dev) = t else { continue };
                let Some(dev) = dev.or_else(|| ls.beats.keys().next().copied()) else { continue };
                let at = ls.beat_at.get(&dev).copied();
                if at.is_none() || at == applied[d] {
                    continue;
                }
                if let Some((bpm, phase)) = external_beat(&ls, dev) {
                    let _ = tx.send(Command::SyncExternal { deck: d, bpm, phase });
                    applied[d] = at;
                }
            }
        }
    });
    Ok(LinkHandle { link, follow, adapter })
}

pub fn load_mapping(path: &Path) -> Result<rekordbox_formats::midi::MidiMapping, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let m = rekordbox_formats::midi::parse(&text);
    if !m.is_valid() {
        return Err(format!("{}: not a version 1 mapping file", path.display()));
    }
    Ok(m)
}

/// Loads requested by the controller (decoded on a worker thread), the library for the browser once it
/// is ready, and browser display updates.
pub fn side_effects(c: &Shared<Controller>, loader: &Loader) {
    if let Some(tracks) = loader.take_library_for_browser() {
        c.lock().unwrap().set_library(tracks);
    }
    let (loads, browser) = {
        let mut g = c.lock().unwrap();
        let changed = g.take_browser_changed();
        (g.take_loads(), if changed { g.browser().map(|b| b.describe()) } else { None })
    };
    if let Some(text) = browser {
        loader.log.say(text);
    }
    for (deck, path) in loads {
        loader.load(deck, &path.to_string_lossy());
    }
}

/// Controller running in the background: MIDI input thread and LED refresh thread.
pub struct ControllerHandle {
    pub controller: Shared<Controller>,
    /// MIDI input / output device names, if found.
    pub input_name: Option<String>,
    pub output_name: Option<String>,
    #[cfg(windows)]
    _input: Option<rekordbox_formats::midi_device::InputPort>,
}

/// Open a controller mapping and the matching MIDI ports. `midi_in` / `midi_out` override the device
/// name from the mapping; `browse` is the browser root (default: the Music folder).
pub fn start_controller(
    mapping: &Path,
    midi_in: Option<&str>,
    midi_out: Option<&str>,
    browse: Option<PathBuf>,
    tx: Sender<Command>,
    state: Shared<MixerState>,
    loader: &Loader,
) -> Result<ControllerHandle, String> {
    let m = load_mapping(mapping)?;
    let device = m.device.clone();
    let controller = Arc::new(Mutex::new(Controller::new(m)));
    let log = &loader.log;
    let root = browse.or_else(|| std::env::var_os("USERPROFILE").map(|h| Path::new(&h).join("Music"))).filter(|p| p.is_dir());
    if let Some(r) = root {
        log.say(format!("browser: {}", r.display()));
        controller.lock().unwrap().set_browser_root(r);
        controller.lock().unwrap().take_browser_changed();
    }
    let (yes, no) = controller.lock().unwrap().coverage();
    log.say(format!("controller {device:?}: {} functions handled, {} not (see midi-coverage)", yes.len(), no.len()));
    #[cfg(windows)]
    {
        use rekordbox_formats::midi_device::{list_inputs, list_outputs, InputPort, OutputPort};
        use rekordbox_formats::midi_input::MidiMessage;
        let pick = |list: Vec<rekordbox_formats::midi_device::DeviceInfo>, over: Option<&str>| {
            let want = over.unwrap_or(device.as_str()).to_lowercase();
            list.into_iter().find(|d| d.name.to_lowercase().contains(&want))
        };
        let (mut input, mut input_name, mut output_name) = (None, None, None);
        match pick(list_inputs(), midi_in) {
            Some(d) => {
                let (port, rx) = InputPort::open(d.index)?;
                log.say(format!("MIDI in: {}", d.name));
                input_name = Some(d.name.clone());
                let (c, tx, ld) = (controller.clone(), tx.clone(), loader.clone());
                std::thread::spawn(move || {
                    for (packed, ms) in rx {
                        if let Some(msg) = MidiMessage::from_packed(packed, f64::from(ms) / 1000.0) {
                            let cmds = c.lock().unwrap().handle(&msg);
                            for cmd in cmds {
                                let _ = tx.send(cmd);
                            }
                            side_effects(&c, &ld);
                        }
                    }
                });
                input = Some(port);
            }
            None => log.say(format!("MIDI in: no device matching {device:?} found")),
        }
        match pick(list_outputs(), midi_out) {
            Some(d) => {
                let port = OutputPort::open(d.index)?;
                log.say(format!("MIDI out (LEDs): {}", d.name));
                output_name = Some(d.name.clone());
                let c = controller.clone();
                std::thread::spawn(move || {
                    let mut last = std::collections::HashMap::new();
                    loop {
                        let s = *state.lock().unwrap();
                        for led in c.lock().unwrap().leds(&s) {
                            let key = (led.status, led.data1);
                            if last.get(&key) != Some(&led.value) {
                                if port.send(led.packed()).is_err() {
                                    return;
                                }
                                last.insert(key, led.value);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(40));
                    }
                });
            }
            None => log.say(format!("MIDI out: no device matching {device:?} found (no LED feedback)")),
        }
        Ok(ControllerHandle { controller, input_name, output_name, _input: input })
    }
    #[cfg(not(windows))]
    {
        let _ = (midi_in, midi_out, tx, state);
        Ok(ControllerHandle { controller, input_name: None, output_name: None })
    }
}
