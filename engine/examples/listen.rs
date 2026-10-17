//! Listen to every MIDI input for N seconds and print what arrives (no output is sent).
fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(120);
    #[cfg(windows)]
    {
        let ports: Vec<_> = rekordbox_formats::midi_device::list_inputs()
            .into_iter()
            .filter_map(|d| rekordbox_formats::midi_device::InputPort::open(d.index).ok().map(|(p, rx)| (d.name, p, rx)))
            .collect();
        println!("listening {secs} s on {:?}", ports.iter().map(|p| &p.0).collect::<Vec<_>>());
        let t = std::time::Instant::now();
        let mut counts = std::collections::BTreeMap::new();
        let mut n = 0usize;
        while t.elapsed().as_secs() < secs {
            for (name, _, rx) in &ports {
                while let Ok((m, ms)) = rx.try_recv() {
                    n += 1;
                    let b = m.to_le_bytes();
                    *counts.entry((b[0], b[1])).or_insert(0usize) += 1;
                    if n <= 40 {
                        println!("{ms:>8} {name}: {:02X} {:02X} {:02X}", b[0], b[1], b[2]);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        println!("{n} messages, {} different (status, data1)", counts.len());
        for ((s, d), c) in counts.iter().take(60) {
            println!("  {s:02X} {d:02X} x{c}");
        }
    }
}
