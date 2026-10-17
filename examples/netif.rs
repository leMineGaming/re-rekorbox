//! List IPv4 adapters as used for DJ Link: `cargo run --example netif`.
fn main() {
    let list = rekordbox_formats::netif::list();
    for i in &list {
        println!("{:<30} {:>15}/{:<2} bcast {:<15} mac {} up={} lo={}", i.name, i.ip, i.prefix_len, i.broadcast(),
            i.mac.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"), i.up, i.loopback);
    }
    println!("chosen: {:?}", rekordbox_formats::netif::choose(&list, None).map(|i| (&i.name, i.ip)));
}
