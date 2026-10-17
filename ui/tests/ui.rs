//! The window driven headless (egui_kittest, AccessKit): silent engine, synthetic click tracks in a
//! temporary folder, own analyses written there too. Saves screenshots to `target/ui-shots/`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use eframe::egui;
use rbx_engine::output::Output;
use rbx_engine::pcm::{write_wav, Pcm};
use rbx_ui::app::{App, Options};
use rbx_ui::tracks::RowSource;

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rbx-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn app_with_tracks(dir: &Path) -> App {
    let files: Vec<PathBuf> = [("Test Artist - Click 128.wav", 128.0), ("Other - Click 124.wav", 124.0)]
        .iter()
        .map(|(n, bpm)| {
            let p = dir.join(n);
            write_wav(&p, &Pcm::click_track(44100, *bpm, 40.0)).unwrap();
            p
        })
        .collect();
    let opts = Options { library: false, auto_analyze: true, analysis_root: Some(dir.join("anlz")), ..Default::default() };
    let app = App::new(Output::silent(44100), opts);
    app.loader.add_files(&files);
    app
}

fn harness(app: App) -> Harness<'static, App> {
    Harness::builder()
        .with_size([1500.0, 950.0])
        .with_accessibility_check(false)
        .build_ui_state(|ui, app: &mut App| app.show(ui), app)
}

/// Step the UI (in real time, the engine and loader run on threads) until `done` or timeout.
fn wait(h: &mut Harness<'static, App>, secs: f64, done: impl Fn(&App) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < secs {
        h.step();
        if done(h.state()) {
            h.step();
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn shot(h: &mut Harness<'static, App>, name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    match h.render() {
        Ok(img) => img.save(dir.join(format!("{name}.png"))).unwrap(),
        Err(e) => eprintln!("no screenshot ({e})"),
    }
}

#[test]
fn load_analyse_play_and_cue_from_the_window() {
    let dir = temp("main");
    let mut h = harness(app_with_tracks(&dir));
    assert!(wait(&mut h, 5.0, |a| a.rows().len() == 2), "track list shows the files");
    assert!(h.state().rows().iter().all(|r| r.source == RowSource::Missing));

    // select a track and load it into deck 1 → it gets analysed automatically
    h.get_by_label("Click 128").click();
    h.run_steps(2);
    h.get_by_label("> Deck 1").click();
    h.step();
    let analysed = wait(&mut h, 30.0, |a| {
        let d = a.loader.deck(0);
        // the engine applies the analysis a moment after the loader knows it
        d.analysis.is_some() && d.busy.is_none() && a.state().decks[0].loaded && a.state().decks[0].bpm.is_some()
    });
    assert!(analysed, "deck 1 loaded and analysed");
    let s = h.state().state();
    assert!((s.decks[0].bpm.unwrap() - 128.0).abs() < 0.02, "bpm {:?}", s.decks[0].bpm);
    assert!(wait(&mut h, 5.0, |a| a.rows().iter().any(|r| r.source == RowSource::Own && r.title == "Click 128")), "list shows the own analysis");
    assert!(dir.join("anlz").is_dir(), "analysis written to the test folder");

    // play: position advances
    h.get_all_by_label("PLAY").next().unwrap().click();
    assert!(wait(&mut h, 5.0, |a| a.state().decks[0].position_s > 0.3), "playing");
    assert!(h.state().state().decks[0].playing);

    // hot cue A on deck 1 (first of the two "A" buttons)
    h.get_all_by_label("A").next().unwrap().click();
    assert!(wait(&mut h, 2.0, |a| a.state().decks[0].hot_cues[0]), "hot cue A set");

    // second track into deck 2 by its button, SYNC deck 2 to deck 1
    h.get_by_label("Click 124").click();
    h.run_steps(2);
    h.get_by_label("> Deck 2").click();
    assert!(wait(&mut h, 30.0, |a| a.loader.deck(1).analysis.is_some() && a.loader.deck(1).busy.is_none()), "deck 2 analysed");
    shot(&mut h, "before-sync");
    h.get_all_by_label("SYNC").nth(1).unwrap().click();
    let synced = wait(&mut h, 2.0, |a| a.state().decks[1].bpm.is_some_and(|b| (b - 128.0).abs() < 0.05));
    assert!(synced, "deck 2 synced to 128: {:?}", h.state().state().decks.map(|d| (d.bpm, d.tempo, d.loaded, d.playing)));
    shot(&mut h, "main");

    // pause
    h.get_all_by_label("PAUSE").next().unwrap().click();
    assert!(wait(&mut h, 2.0, |a| !a.state().decks[0].playing), "paused");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn search_filters_the_list() {
    let dir = temp("search");
    let mut h = harness(app_with_tracks(&dir));
    assert!(wait(&mut h, 5.0, |a| a.rows().len() == 2));
    h.state_mut().search = "other".into();
    h.step();
    h.step();
    assert!(h.query_by_label("Click 124").is_some());
    assert!(h.query_by_label("Click 128").is_none(), "filtered out");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn time_format() {
    assert_eq!(rbx_ui::app::fmt_time(0.0), "0:00.0");
    assert_eq!(rbx_ui::app::fmt_time(83.46), "1:23.5");
}

/// Screenshot with the real library of this machine (read only: no analysis, nothing written).
/// `cargo test -p rbx-ui --test ui -- --ignored real_library`
#[test]
#[ignore]
fn real_library_screenshot() {
    let dir = temp("real");
    let opts = Options { library: true, auto_analyze: false, analysis_root: Some(dir.join("anlz")), ..Default::default() };
    let mut h = harness(App::new(Output::silent(44100), opts));
    assert!(wait(&mut h, 60.0, |a| !a.loader.library_loading() && a.rows().len() > 10), "library read");
    let rekordbox: Vec<String> = h
        .state()
        .rows()
        .iter()
        .filter(|r| r.source == RowSource::Rekordbox && r.bpm.is_some())
        .take(2)
        .map(|r| r.path.to_string_lossy().into_owned())
        .collect();
    for (d, p) in rekordbox.iter().enumerate() {
        h.state_mut().load(d, p);
    }
    assert!(wait(&mut h, 30.0, |a| (0..2).all(|d| a.loader.deck(d).analysis.is_some() && a.state().decks[d].loaded)));
    h.state().loader.log.say("(screenshot)");
    for d in 0..2 {
        h.state().state();
        let _ = d;
    }
    shot(&mut h, "real-library");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn keyboard_plays_and_cues() {
    let dir = temp("keys");
    let mut h = harness(app_with_tracks(&dir));
    assert!(wait(&mut h, 5.0, |a| a.rows().len() == 2));
    let p = h.state().rows()[1].path.to_string_lossy().into_owned();
    h.state_mut().load(0, &p);
    assert!(wait(&mut h, 30.0, |a| a.state().decks[0].loaded && a.loader.deck(0).busy.is_none()));
    h.key_press(egui::Key::X);
    assert!(wait(&mut h, 3.0, |a| a.state().decks[0].playing), "X = play deck 1");
    h.key_press(egui::Key::Num1);
    assert!(wait(&mut h, 2.0, |a| a.state().decks[0].hot_cues[0]), "1 = hot cue A");
    h.key_press(egui::Key::X);
    assert!(wait(&mut h, 2.0, |a| !a.state().decks[0].playing), "X again = pause");
    h.key_press(egui::Key::ArrowRight);
    assert!(wait(&mut h, 2.0, |a| a.state().crossfader > 0.05), "arrow right moves the crossfader");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_round_trip() {
    use rbx_ui::settings::Settings;
    let s = Settings {
        device: Some("Lautsprecher (Realtek)".into()),
        buffer: Some(256),
        music: Some(PathBuf::from(r"D:\Musik")),
        midi: Some(PathBuf::from(r"C:\m\XDJ-RX3.midi.csv")),
        djlink: true,
        djlink_adapter: "Ethernet".into(),
        auto_analyze: false,
        zoom_s: 12.0,
        ..Default::default()
    };
    assert_eq!(Settings::parse(&s.to_text()), s);
    // unknown keys and bad values are ignored
    let p = Settings::parse("buffer=5\nzoom_s=abc\nfoo=bar\ndevice=\n");
    assert_eq!(p, Settings::default());
    let dir = temp("settings");
    let f = dir.join("sub").join("settings.txt");
    s.save(&f).unwrap();
    assert_eq!(Settings::load(&f), s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// MIDI message of a mapping function (deck 0 = global control).
fn rx3_message(name: &str, deck: u8, value: u8, t: f64) -> rekordbox_formats::midi_input::MidiMessage {
    let m = rekordbox_formats::midi::parse(rbx_engine::controller::XDJ_RX3_MAPPING);
    let e = m.entries.iter().find(|e| rbx_engine::controller::function_of(e) == name).unwrap();
    let b = e.input_bindings(&m.device).into_iter().find(|b| b.deck == deck).unwrap();
    rekordbox_formats::midi_input::MidiMessage::from_bytes(&[(b.message >> 8) as u8, b.message as u8, value], t).unwrap()
}

#[test]
fn browse_and_load_with_the_controller() {
    let dir = temp("browse");
    let mut h = harness(app_with_tracks(&dir));
    assert!(wait(&mut h, 5.0, |a| a.rows().len() == 2));
    let mapping = rekordbox_formats::midi::parse(rbx_engine::controller::XDJ_RX3_MAPPING);
    h.state_mut().connect_mapping(mapping, Some("rbx-test-no-such-port")).unwrap();
    let send = |h: &mut Harness<'static, App>, name: &str, deck: u8, value: u8, t: f64| {
        let msg = rx3_message(name, deck, value, t);
        let cmds = h.state().controller().unwrap().controller.lock().unwrap().handle(&msg);
        assert!(cmds.is_empty(), "browser controls do not touch the engine");
        h.run_steps(2);
    };
    // BROWSE two steps down: first row, then second row
    send(&mut h, "Browse", 0, 0x41, 1.0);
    assert_eq!(h.state().selected(), Some(0));
    send(&mut h, "Browse", 0, 0x41, 1.1);
    assert_eq!(h.state().selected(), Some(1));
    let second = h.state().rows()[1].path.clone();
    // LOAD on deck 1 loads the selected track
    send(&mut h, "Load", 1, 0x7F, 2.0);
    send(&mut h, "Load", 1, 0x00, 2.1);
    assert!(wait(&mut h, 30.0, |a| a.loader.deck(0).source.as_deref().map(PathBuf::from) == Some(second.clone())), "deck 1 has the selected track");
    // TAG TRACK, tag list view, BACK leaves it
    send(&mut h, "AddToTagList", 0, 0x7F, 3.0);
    assert!(h.state().tags.contains(&second));
    send(&mut h, "TrackFilter", 0, 0x7F, 3.5);
    assert!(h.state().tag_view);
    send(&mut h, "Back", 0, 0x7F, 4.0);
    assert!(!h.state().tag_view);
    shot(&mut h, "controller-browse");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}
