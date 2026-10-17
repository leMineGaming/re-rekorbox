//! `rbx-ui` – the rekordbox-rs player with a window.
//!
//! ```text
//! rbx-ui [file1] [file2] [--device NAME] [--buffer FRAMES] [--silent] [--midi MAPPING.csv]
//!        [--djlink [ADAPTER]] [--anlz DIR] [--music DIR] [--no-library] [--no-analyze] [--samples DIR]
//! ```
//! Settings changed in the window are stored in `%APPDATA%\rekordbox-rs\settings.txt`; command-line
//! options override them for this run. Messages go to `%APPDATA%\rekordbox-rs\logs\rbx-ui.log`, crashes
//! to `logs\crash-rbx-ui.txt`. `--silent` runs the engine without a sound device (nothing is audible).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use rbx_engine::output::Output;
use rbx_engine::session::{data_dir, install_crash_handler};
use rbx_ui::app::{App, Options};
use rbx_ui::settings::Settings;

fn main() -> eframe::Result {
    install_crash_handler("rbx-ui");
    let settings_path = Settings::default_path();
    let settings = settings_path.as_deref().map(Settings::load).unwrap_or_default();
    let mut opts = Options {
        library: true,
        auto_analyze: settings.auto_analyze,
        music: settings.music.clone(),
        anlz: settings.anlz.clone(),
        midi: settings.midi.clone(),
        djlink: settings.djlink.then(|| Some(settings.djlink_adapter.clone()).filter(|a| !a.is_empty())),
        log_file: data_dir().map(|d| d.join("logs").join("rbx-ui.log")),
        settings: settings.clone(),
        settings_path,
        ..Default::default()
    };
    let (mut device, mut buffer, mut silent) = (settings.device.clone(), settings.buffer, false);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        // value of a `--flag value` option (not another flag)
        let value = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
        let takes_value = matches!(args[i].as_str(), "--device" | "--buffer" | "--midi" | "--djlink" | "--anlz" | "--music" | "--samples");
        match args[i].as_str() {
            "--device" => device = value.clone(),
            "--buffer" => buffer = value.as_deref().and_then(|v| v.parse().ok()),
            "--silent" => silent = true,
            "--midi" => opts.midi = value.clone().map(PathBuf::from),
            "--djlink" => opts.djlink = Some(value.clone()),
            "--anlz" => opts.anlz = value.clone().map(PathBuf::from),
            "--music" => opts.music = value.clone().map(PathBuf::from),
            "--samples" => opts.samples = value.clone().map(PathBuf::from),
            "--no-library" => opts.library = false,
            "--no-analyze" => opts.auto_analyze = false,
            a => opts.load.push(a.to_string()),
        }
        i += if takes_value && value.is_some() { 2 } else { 1 };
    }
    let out = if silent {
        Output::silent(44100)
    } else {
        Output::open_with(device.as_deref(), buffer)
            .or_else(|e| {
                eprintln!("audio: {e} – trying the default device");
                Output::open(None)
            })
            .unwrap_or_else(|e| {
                eprintln!("audio: {e} – running silent");
                Output::silent(44100)
            })
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("rekordbox-rs")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true)
            .with_icon(egui::IconData { rgba: include_bytes!("../assets/icon-64.rgba").to_vec(), width: 64, height: 64 }),
        ..Default::default()
    };
    eframe::run_native(
        "rekordbox-rs",
        native,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(App::new(out, opts)))
        }),
    )
}
