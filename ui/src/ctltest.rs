//! Controller test window: MIDI monitor, which controls have been moved (per group), messages the
//! mapping does not know, LED test, recording to a MIDI log and a report file.

use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};
use rbx_engine::session::{data_dir, timestamp, ControllerHandle, Log};

/// Folder for MIDI logs and controller test reports.
pub fn midi_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("midi"))
}

fn stamp() -> String {
    timestamp().replace([' ', ':'], "-")
}

/// Write a Markdown report of the current test state; returns its path.
pub fn write_report(h: &ControllerHandle) -> std::io::Result<PathBuf> {
    let c = h.controller.lock().unwrap();
    let all = c.testable_controls();
    let seen = c.seen();
    let mut r = format!(
        "# Controller-Test {}\n\n{} UTC – MIDI in: {}, MIDI out: {}\n\n",
        c.mapping().device,
        timestamp(),
        h.input_name.as_deref().unwrap_or("-"),
        h.output_name.as_deref().unwrap_or("-")
    );
    let done = all.iter().filter(|(_, f, d)| seen.contains(&(f.clone(), *d))).count();
    r.push_str(&format!("**{done} von {} Bedienelementen haben gesendet.**\n\n", all.len()));
    let mut groups: Vec<&str> = all.iter().map(|(g, _, _)| *g).collect();
    groups.dedup();
    for g in groups {
        let items: Vec<_> = all.iter().filter(|(x, _, _)| *x == g).collect();
        let ok = items.iter().filter(|(_, f, d)| seen.contains(&(f.clone(), *d))).count();
        r.push_str(&format!("## {g}: {ok}/{}\n\nNoch nicht gesendet: ", items.len()));
        let missing: Vec<String> = items.iter().filter(|(_, f, d)| !seen.contains(&(f.clone(), *d))).map(|(_, f, d)| format!("{f}@{d}")).collect();
        r.push_str(if missing.is_empty() { "–" } else { "" });
        r.push_str(&missing.join(", "));
        r.push_str("\n\n");
    }
    r.push_str("## Nachrichten ohne Mapping-Eintrag\n\n");
    if c.unknown().is_empty() {
        r.push_str("keine\n\n");
    }
    for ((st, d1), n) in c.unknown() {
        r.push_str(&format!("- `{st:02X} {d1:02X}` × {n}\n"));
    }
    r.push_str("\n## Letzte Nachrichten\n\n```\n");
    for e in c.monitor() {
        let hex: Vec<String> = e.bytes.iter().map(|b| format!("{b:02X}")).collect();
        r.push_str(&format!("{:9.3}  {:<9} {:<40} {}\n", e.time_s, hex.join(" "), e.functions.join(" "), e.commands.join("; ")));
    }
    r.push_str("```\n");
    let dir = midi_dir().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("controller-test-{}.md", stamp()));
    std::fs::write(&path, r)?;
    Ok(path)
}

/// The window. `open` is cleared when the user closes it.
pub fn window(ctx: &egui::Context, h: &ControllerHandle, log: &Log, open: &mut bool) {
    egui::Window::new("Controller-Test").open(open).default_size([720.0, 560.0]).show(ctx, |ui| {
        let mut want_report = false;
        let mut c = h.controller.lock().unwrap();
        ui.label(format!(
            "{} – MIDI in: {}, MIDI out (LEDs): {}",
            c.mapping().device,
            h.input_name.as_deref().unwrap_or("nicht gefunden"),
            h.output_name.as_deref().unwrap_or("nicht gefunden")
        ));
        ui.label(
            RichText::new(
                "Alle Tasten, Regler, Fader, Pads und Jogs nacheinander bedienen (Deck 1 und 2, auch mit SHIFT). \
                 Grün = hat gesendet. Unbekannte Nachrichten zeigen Bedienelemente, die das Mapping nicht kennt.",
            )
            .small(),
        );
        ui.horizontal(|ui| {
            if ui.button("Zurücksetzen").clicked() {
                c.reset_test();
            }
            let led_running = h.led_test.load(std::sync::atomic::Ordering::Relaxed);
            if ui.add_enabled(!led_running && h.output_name.is_some(), egui::Button::new(if led_running { "LED-Test läuft ..." } else { "LED-Test" })).on_hover_text("Alle LEDs nacheinander einschalten (Lauflicht, ca. 10 s)").clicked() {
                h.led_test.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            if c.is_recording() {
                if ui.button("Aufnahme stoppen").clicked() {
                    c.stop_recording();
                    log.say("MIDI-Aufnahme gestoppt");
                }
            } else if ui.button("Aufnahme starten").on_hover_text("Alle eingehenden MIDI-Nachrichten in eine Datei schreiben (zum Nachspielen in Tests)").clicked() {
                if let Some(dir) = midi_dir() {
                    let p = dir.join(format!("aufnahme-{}.midilog", stamp()));
                    match c.start_recording(&p) {
                        Ok(()) => log.say(format!("MIDI-Aufnahme: {}", p.display())),
                        Err(e) => log.error(format!("MIDI-Aufnahme: {e}")),
                    }
                }
            }
            if ui.button("Bericht speichern").clicked() {
                want_report = true;
            }
        });
        ui.separator();
        let all = c.testable_controls();
        let seen = c.seen().clone();
        let done = all.iter().filter(|(_, f, d)| seen.contains(&(f.clone(), *d))).count();
        ui.add(egui::ProgressBar::new(done as f32 / all.len().max(1) as f32).text(format!("{done} von {} Bedienelementen", all.len())));
        let mut groups: Vec<&str> = all.iter().map(|(g, _, _)| *g).collect();
        groups.dedup();
        egui::ScrollArea::vertical().id_salt("controls").max_height(220.0).show(ui, |ui| {
            for g in groups {
                let items: Vec<_> = all.iter().filter(|(x, _, _)| *x == g).collect();
                let ok = items.iter().filter(|(_, f, d)| seen.contains(&(f.clone(), *d))).count();
                egui::CollapsingHeader::new(format!("{g}: {ok}/{}", items.len())).id_salt(g).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (_, f, d) in items {
                            let hit = seen.contains(&(f.clone(), *d));
                            let text = if *d == 0 { f.clone() } else { format!("{f} ({d})") };
                            ui.label(RichText::new(text).small().color(if hit { Color32::LIGHT_GREEN } else { Color32::GRAY }));
                        }
                    });
                });
            }
        });
        if !c.unknown().is_empty() {
            ui.separator();
            let list: Vec<String> = c.unknown().iter().map(|((s, d), n)| format!("{s:02X} {d:02X} ×{n}")).collect();
            ui.colored_label(Color32::YELLOW, format!("Ohne Mapping: {}", list.join(", ")));
        }
        ui.separator();
        ui.label(RichText::new("MIDI-Monitor (neueste oben)").strong());
        egui::ScrollArea::vertical().id_salt("monitor").max_height(200.0).show(ui, |ui| {
            for e in c.monitor().iter().rev().take(40) {
                let hex: Vec<String> = e.bytes.iter().map(|b| format!("{b:02X}")).collect();
                let text = format!("{:9.3}  {:<9} {:<34} {}", e.time_s, hex.join(" "), e.functions.join(" "), e.commands.join("; "));
                let col = if e.unknown { Color32::YELLOW } else { Color32::LIGHT_GRAY };
                ui.label(RichText::new(text).monospace().small().color(col));
            }
        });
        drop(c);
        if want_report {
            match write_report(h) {
                Ok(p) => log.say(format!("Controller-Bericht: {}", p.display())),
                Err(e) => log.error(format!("Controller-Bericht: {e}")),
            }
        }
    });
}
