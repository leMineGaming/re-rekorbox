fn main() {
    let t = std::time::Instant::now();
    for c in rbx_engine::quality::run_all() {
        println!("{} {:<10} {:<42} {:>10.4} {} (limit {}) {}", if c.passed { "OK  " } else { "FAIL" }, c.group, c.name, c.value, c.unit, c.limit, c.detail);
    }
    println!("{:.1} s", t.elapsed().as_secs_f64());
}
