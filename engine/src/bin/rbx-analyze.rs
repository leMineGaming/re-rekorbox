//! `rbx-analyze` – analyse audio files and write ANLZ files (own analysis, SPEC §5j).
//!
//! ```text
//! rbx-analyze <file|folder>... [--out DIR] [--force] [--missing] [--compare] [--threads N] [--limit N]
//! ```
//! Results go to `%APPDATA%\rekordbox-rs\ANLZ` (or `--out`), one folder per file, with
//! `ANLZ0000.DAT/.EXT/.2EX` and `rbx-analysis.txt` (BPM, key, confidence). Existing results are
//! skipped unless `--force`. `--missing` analyses only files that have no rekordbox analysis;
//! `--compare` prints rekordbox's BPM next to ours where it exists.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rbx_engine::analyze::{analyze_file, default_output_root, folder_for, write_anlz};
use rekordbox_formats::library::{default_anlz_root, default_music_root, is_audio, Library};

fn collect(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        if let Ok(it) = std::fs::read_dir(p) {
            let mut v: Vec<PathBuf> = it.flatten().map(|e| e.path()).collect();
            v.sort();
            for e in v {
                collect(&e, out);
            }
        }
    } else if is_audio(p) {
        out.push(std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut inputs = Vec::new();
    let (mut out, mut force, mut missing, mut compare, mut threads, mut limit) = (None, false, false, false, 0usize, usize::MAX);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                out = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            "--force" => force = true,
            "--missing" => missing = true,
            "--compare" => compare = true,
            "--threads" => {
                threads = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
                i += 1;
            }
            "--limit" => {
                limit = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
                i += 1;
            }
            a => inputs.push(PathBuf::from(a)),
        }
        i += 1;
    }
    if inputs.is_empty() {
        eprintln!("usage: rbx-analyze <file|folder>... [--out DIR] [--force] [--missing] [--compare] [--threads N] [--limit N]");
        return ExitCode::FAILURE;
    }
    let Some(root) = out.or_else(default_output_root) else {
        eprintln!("no output folder");
        return ExitCode::FAILURE;
    };
    let mut files = Vec::new();
    for p in &inputs {
        collect(p, &mut files);
    }
    let rb = if missing || compare {
        default_anlz_root().map(|a| Library::build(&a, &default_music_root().into_iter().chain(inputs.iter().filter(|p| p.is_dir()).cloned()).collect::<Vec<_>>()))
    } else {
        None
    };
    let todo: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| force || !root.join(folder_for(f)).join("ANLZ0000.DAT").exists())
        .filter(|f| !missing || rb.as_ref().is_none_or(|l| l.by_path(f).is_none()))
        .take(limit)
        .collect();
    let n = todo.len();
    println!("{n} files to analyse → {}", root.display());
    let threads = if threads == 0 { std::thread::available_parallelism().map_or(2, |v| v.get()).min(8) } else { threads };
    let queue = Arc::new(Mutex::new(todo.into_iter().enumerate().collect::<Vec<_>>()));
    let done = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let rb = Arc::new(rb);
    let t0 = std::time::Instant::now();
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            let (queue, done, failed, root, rb) = (queue.clone(), done.clone(), failed.clone(), root.clone(), rb.clone());
            std::thread::spawn(move || loop {
                let Some((_, f)) = queue.lock().unwrap().pop() else { break };
                let t = std::time::Instant::now();
                let name = f.file_name().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                match analyze_file(&f).map_err(|e| e.to_string()).and_then(|r| write_anlz(&r, &root).map(|_| r).map_err(|e| e.to_string())) {
                    Ok(r) => {
                        let k = done.fetch_add(1, Ordering::Relaxed) + 1;
                        let theirs = if compare {
                            (*rb).as_ref().and_then(|l| l.by_path(&f)).and_then(|t| t.bpm()).map_or(String::new(), |b| format!(" (rekordbox {b:.2})"))
                        } else {
                            String::new()
                        };
                        println!(
                            "[{k}/{n}] {name}: {}{theirs} {} ({:.1} s)",
                            r.grid.as_ref().map_or("no grid".into(), |g| format!("{:.2} BPM", g.bpm)),
                            r.key.map_or(String::new(), |(k, _)| format!("{} / {}", k.name(), k.camelot())),
                            t.elapsed().as_secs_f64()
                        );
                    }
                    Err(e) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        eprintln!("{name}: {e}");
                    }
                }
            })
        })
        .collect();
    for w in workers {
        let _ = w.join();
    }
    println!(
        "done: {} analysed, {} failed in {:.1} s",
        done.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
        t0.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}
