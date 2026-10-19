//! The USB-MIDI SysEx keepalive of LINK EXPORT (`xdj-rx3-link-export-protokoll.md` §11.5).
//!
//! Right before the link handshake rekordbox starts sending one SysEx on the audio/MIDI device's
//! MIDI streaming bulk-OUT endpoint (`0x04`), **64× identical, every 200 ms**, and keeps doing so
//! until the end of the capture. It looks like a keepalive/status command; the unit sends no MIDI
//! answer (endpoint `0x85` is silent). The manufacturer id is `00 40 05`. The purpose is unclear, so
//! it should be tried with and without (protokoll §11.7.2).

use std::time::Duration;

/// The SysEx as captured, verbatim (`F0 00 40 05 00 00 03 0D 00 50 01 F7`).
pub const SYSEX: [u8; 12] = [
    0xF0, 0x00, 0x40, 0x05, 0x00, 0x00, 0x03, 0x0D, 0x00, 0x50, 0x01, 0xF7,
];
/// The 3-byte manufacturer id of the SysEx.
pub const MANUFACTURER_ID: [u8; 3] = [0x00, 0x40, 0x05];
/// The send interval observed in the capture.
pub const INTERVAL: Duration = Duration::from_millis(200);
/// MIDI streaming bulk-OUT endpoint the SysEx is written to.
pub const EP_BULK_OUT: u8 = 0x04;

// USB-MIDI "code index number" (low nibble of the first byte of a 32-bit packet).
/// SysEx start/continue, 3 data bytes.
pub const CIN_SYSEX_CONTINUE: u8 = 0x4;
/// SysEx end, 1 data byte.
pub const CIN_SYSEX_END_1: u8 = 0x5;
/// SysEx end, 2 data bytes.
pub const CIN_SYSEX_END_2: u8 = 0x6;
/// SysEx end, 3 data bytes.
pub const CIN_SYSEX_END_3: u8 = 0x7;

/// Pack a SysEx message into USB-MIDI 32-bit packets (4 bytes each, `[CIN, d0, d1, d2]`).
///
/// Every packet carries up to 3 data bytes; all but the last use [`CIN_SYSEX_CONTINUE`], the last
/// uses the end code that matches its remaining byte count (the trailing `F7` is carried as a data
/// byte). The 12-byte keepalive therefore becomes exactly the four packets of the capture:
/// `04 F0 00 40 | 04 05 00 00 | 04 03 0D 00 | 07 50 01 F7`.
pub fn usb_midi_packets(sysex: &[u8]) -> Vec<[u8; 4]> {
    let mut out = Vec::with_capacity(sysex.len().div_ceil(3));
    let mut i = 0usize;
    while i < sysex.len() {
        let take = (sysex.len() - i).min(3);
        let last = i + take == sysex.len();
        let cin = if !last {
            CIN_SYSEX_CONTINUE
        } else {
            match take {
                1 => CIN_SYSEX_END_1,
                2 => CIN_SYSEX_END_2,
                _ => CIN_SYSEX_END_3,
            }
        };
        let mut pkt = [0u8; 4];
        pkt[0] = cin;
        pkt[1..1 + take].copy_from_slice(&sysex[i..i + take]);
        out.push(pkt);
        i += take;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §11.5: the keepalive becomes exactly the four observed USB-MIDI packets.
    #[test]
    fn keepalive_packets_match_the_capture() {
        assert_eq!(
            usb_midi_packets(&SYSEX),
            vec![
                [0x04, 0xF0, 0x00, 0x40],
                [0x04, 0x05, 0x00, 0x00],
                [0x04, 0x03, 0x0D, 0x00],
                [0x07, 0x50, 0x01, 0xF7],
            ]
        );
    }

    #[test]
    fn arbitrary_sysex_lengths_are_chunked_and_terminated() {
        // ends with a 2-byte packet (code 0x6)
        assert_eq!(
            usb_midi_packets(&[0xF0, 0x01, 0x02, 0x03, 0xF7]),
            vec![[0x04, 0xF0, 0x01, 0x02], [0x06, 0x03, 0xF7, 0x00]]
        );
        // ends with a 1-byte packet (code 0x5)
        assert_eq!(
            usb_midi_packets(&[0xF0, 0x01, 0x02, 0xF7]),
            vec![[0x04, 0xF0, 0x01, 0x02], [0x05, 0xF7, 0x00, 0x00]]
        );
        // exact multiple of 3 ends with a 3-byte packet (code 0x7)
        assert_eq!(
            usb_midi_packets(&[0xF0, 0x01, 0x02, 0x03, 0x04, 0xF7]),
            vec![[0x04, 0xF0, 0x01, 0x02], [0x07, 0x03, 0x04, 0xF7]]
        );
    }
}
