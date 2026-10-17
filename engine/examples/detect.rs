fn main() {
    #[cfg(windows)]
    for d in rekordbox_formats::midi_device::list_inputs() {
        println!("input {:?} bytes {:02X?} -> builtin {:?}", d.name, d.name.as_bytes(), rbx_engine::controller::builtin_for_port(&d.name).map(|m| m.device));
        match rekordbox_formats::midi_device::InputPort::open(d.index) {
            Ok((_p, rx)) => {
                println!("  opened; listening 3 s");
                let t = std::time::Instant::now();
                let mut n = 0;
                while t.elapsed().as_secs() < 3 {
                    if let Ok((m, _)) = rx.recv_timeout(std::time::Duration::from_millis(100)) { n += 1; if n < 10 { println!("  msg {m:08X}"); } }
                }
                println!("  {n} messages");
            }
            Err(e) => println!("  open failed: {e}"),
        }
    }
    println!("detect: {:?}", rbx_engine::session::detect_controller().map(|(p, m)| (p, m.device)));
}
