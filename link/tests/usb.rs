//! USB layer (RTL8152 framing, MIDI SysEx) against `xdj-rx3-link-export-protokoll.md` §4 and §11.5.

use rbx_link::usb::{midi, rtl8152};

#[test]
fn tx_descriptor_opts1_is_frame_len_with_fs_ls() {
    // §4.2: "u32 LE opts1 = frame_len | (1<<31) | (1<<30)  // z. B. 0xC0000062 für 98 Byte"
    let t = rtl8152::tx_transfer(&vec![0u8; 98], 0);
    assert_eq!(t.len(), 8 + 98);
    assert_eq!(u32::from_le_bytes(t[..4].try_into().unwrap()), 0xC000_0062);
    assert_eq!(u32::from_le_bytes(t[4..8].try_into().unwrap()), 0);
}

#[test]
fn rx_transfer_lengths_include_the_fcs() {
    // §4.2: "Transfer 368 B = 24 + 338 (= 334 Frame + 4 FCS = 362) → auf 8 aufgerundet = 368"
    let frame = vec![0xEE; 334];
    let mut t = Vec::new();
    t.extend_from_slice(&(338u32).to_le_bytes());
    t.extend_from_slice(&[0u8; 20]);
    t.extend_from_slice(&frame);
    t.extend_from_slice(&[0u8; 4]);
    t.resize(368, 0);
    assert_eq!(rtl8152::rx_frames(&t), vec![&frame[..]]);
}

#[test]
fn sysex_keepalive_is_packetized_as_captured() {
    // §11.5: `04 F0 00 40 | 04 05 00 00 | 04 03 0D 00 | 07 50 01 F7`
    assert_eq!(
        midi::SYSEX,
        [0xF0, 0x00, 0x40, 0x05, 0x00, 0x00, 0x03, 0x0D, 0x00, 0x50, 0x01, 0xF7]
    );
    assert_eq!(
        midi::usb_midi_packets(&midi::SYSEX),
        vec![
            [0x04, 0xF0, 0x00, 0x40],
            [0x04, 0x05, 0x00, 0x00],
            [0x04, 0x03, 0x0D, 0x00],
            [0x07, 0x50, 0x01, 0xF7],
        ]
    );
}
