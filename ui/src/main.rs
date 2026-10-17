//! `rbx-ui` – the rekordbox-rs player with a window.
//!
//! ```text
//! rbx-ui [file1] [file2] [--device NAME] [--silent] [--midi MAPPING.csv] [--djlink [ADAPTER]]
//!        [--anlz DIR] [--music DIR] [--no-library] [--no-analyze] [--samples DIR]
//! ```
//! `--silent` runs the engine without a sound device (nothing is audible).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use rbx_engine::output::Output;
use rbx_ui::app::{App, Options};

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = Options { library: true, auto_analyze: true, ..Default::default() };
    let (mut device, mut silent) = (None, false);
    let mut i = 0;
    while i < args.len() {
        // value of a `--flag value` option (not another flag)
        let value = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
        let used = value.is_some();
        match args[i].as_str() {
            "--device" => device = value,
            "--silent" => silent = true,
            "--midi" => opts.midi = value.map(PathBuf::from),
            "--djlink" => opts.djlink = Some(value),
            "--anlz" => opts.anlz = value.map(PathBuf::from),
            "--music" => opts.music = value.map(PathBuf::from),
            "--samples" => opts.samples = value.map(PathBuf::from),
            "--no-library" => opts.library = false,
            "--no-analyze" => opts.auto_analyze = false,
            a => {
                opts.load.push(a.to_string());
                i += 1;
                continue;
            }
        }
        let takes_value = matches!(args[i].as_str(), "--device" | "--midi" | "--djlink" | "--anlz" | "--music" | "--samples");
        i += if takes_value && used { 2 } else { 1 };
    }
    let out = if silent {
        Output::silent(44100)
    } else {
        match Output::open(device.as_deref()) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("audio: {e} – running silent");
                Output::silent(44100)
            }
        }
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("rekordbox-rs")
            .with_inner_size([1500.0, 950.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true),
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
