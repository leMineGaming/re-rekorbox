//! Portmapper and mountd against capture 1: the XDJ-RX3's requests, rekordbox's answers byte for byte.

mod fixtures;

use std::net::Ipv4Addr;

use fixtures::bytes;
use rbx_link::rpc::{self, Exports, Kind};

const GETPORT_MOUNT: &str = "aac2b2c40000000000000002000186a000000002000000030000000100000014ec9001da000000000000000000000000000000000000000000000000000186a5000000010000001100000000";
const GETPORT_MOUNT_REPLY: &str = "aac2b2c4000000010000000000000000000000000000000000008d9b";
const GETPORT_NFS: &str = "aac2b2c70000000000000002000186a00000000200000003000000010000001488b1cbef000000000000000000000000000000000000000000000000000186a3000000020000001100000000";
const GETPORT_NFS_REPLY: &str = "aac2b2c7000000010000000000000000000000000000000000000801";
const EXPORT: &str = "aac2b2c50000000000000002000186a50000000100000005000000010000001400147b7d000000000000000000000000000000000000000000000000";
const EXPORT_REPLY: &str = "aac2b2c5000000010000000000000000000000000000000000000001000000062f0043002f000000000000010000001b3136392e3235342e3232362e3231352f3235352e3235352e302e30000000000000000001000000062f0044002f000000000000010000001b3136392e3235342e3232362e3231352f3235352e3235352e302e30000000000000000000";
/// rekordbox before its library was ready
const EXPORT_EMPTY_REPLY: &str = "aac2b2b3000000010000000000000000000000000000000000000000";
const EXPORT_EARLY: &str = "aac2b2b30000000000000002000186a500000001000000050000000100000014906d577d000000000000000000000000000000000000000000000000";
const MNT_C: &str = "aac2b2c80000000000000002000186a50000000100000001000000010000001498dd496f000000000000000000000000000000000000000000000000000000062f0043002f000011";
const MNT_C_REPLY: &str = "aac2b2c80000000100000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
const MNT_D: &str = "aac2b2cb0000000000000002000186a500000001000000010000000100000014efb697a1000000000000000000000000000000000000000000000000000000062f0044002f000011";
const MNT_D_REPLY: &str = "aac2b2cb0000000100000000000000000000000000000000000000000000000100000001000000010000000000000000000000000000000000000000";

fn exports() -> Exports {
    Exports {
        drives: vec!['C', 'D'],
        ip: Ipv4Addr::new(169, 254, 226, 215),
        netmask: Ipv4Addr::new(255, 255, 0, 0),
    }
}

fn answer(e: &Exports, req: &str) -> (Vec<u8>, Kind) {
    rpc::answer(e, &bytes(req)).expect("an answer")
}

#[test]
fn portmapper_on_50111_like_rekordbox() {
    let (b, k) = answer(&exports(), GETPORT_MOUNT);
    assert_eq!(b, bytes(GETPORT_MOUNT_REPLY));
    assert_eq!(
        k,
        Kind::GetPort {
            prog: 100_005,
            vers: 1,
            port: 36251
        }
    );
    assert_eq!(answer(&exports(), GETPORT_NFS).0, bytes(GETPORT_NFS_REPLY));
}

#[test]
fn export_list_like_rekordbox() {
    let (b, k) = answer(&exports(), EXPORT);
    assert_eq!(b, bytes(EXPORT_REPLY));
    assert_eq!(k, Kind::Export);
    let none = Exports {
        drives: vec![],
        ..exports()
    };
    assert_eq!(answer(&none, EXPORT_EARLY).0, bytes(EXPORT_EMPTY_REPLY));
}

#[test]
fn mount_returns_rekordbox_s_handles() {
    assert_eq!(
        answer(&exports(), MNT_C),
        (bytes(MNT_C_REPLY), Kind::Mount { ok: true })
    );
    assert_eq!(answer(&exports(), MNT_D).0, bytes(MNT_D_REPLY));
    // a drive that is not exported
    let only_c = Exports {
        drives: vec!['C'],
        ..exports()
    };
    let (b, k) = answer(&only_c, MNT_D);
    assert_eq!(&b[24..28], &[0, 0, 0, 2]);
    assert_eq!(k, Kind::Mount { ok: false });
}

#[test]
fn garbage_gets_no_answer() {
    assert!(rpc::answer(&exports(), &[]).is_none());
    assert!(rpc::answer(&exports(), &[0; 40]).is_none());
    let mut cut = bytes(MNT_C);
    cut.truncate(30);
    assert!(rpc::answer(&exports(), &cut).is_none());
}
