//! RTL8152/8153 Ethernet-over-USB framing (`xdj-rx3-link-export-protokoll.md` §4.2).
//!
//! The XDJ-RX3's network function is a Realtek RTL8152/8153 chip; the capture matches the Linux
//! driver `drivers/net/usb/r8152.c` (`struct tx_desc`, `struct rx_desc`) byte for byte. Each bulk
//! transfer on the endpoints carries the driver's descriptors, not just the raw frame:
//!
//! * **TX (host → unit, bulk OUT `0x02`):** one 8-byte `tx_desc`, then the frame (no FCS).
//!   `opts1 = frame_len | FS | LS` (first and last segment), `opts2` = checksum-offload flags.
//! * **RX (unit → host, bulk IN `0x81`):** several frames per transfer possible; each is a 24-byte
//!   `rx_desc` whose `opts1` low 14 bits hold the length **including** the 4-byte FCS, followed by
//!   the frame and its FCS. The next descriptor is 8-byte aligned.
//!
//! All descriptor fields are little-endian. When checksum offload is not implemented, compute the
//! IP/TCP/UDP checksums in software and send `opts2 = 0` (protokoll §4.2, "zu testen").

/// Bulk OUT endpoint: TX (host → unit).
pub const EP_BULK_OUT: u8 = 0x02;
/// Bulk IN endpoint: RX (unit → host).
pub const EP_BULK_IN: u8 = 0x81;
/// Interrupt IN endpoint: link status (not evaluated in the capture, protokoll §4.1).
pub const EP_INTR_IN: u8 = 0x83;

/// Size of a `tx_desc` in bytes.
pub const TX_DESC_LEN: usize = 8;
/// Size of an `rx_desc` in bytes.
pub const RX_DESC_LEN: usize = 24;
/// Length of the trailing Ethernet frame check sequence.
pub const FCS_LEN: usize = 4;

// tx_desc.opts1 (little-endian)
/// First segment of a frame.
pub const TX_FS: u32 = 1 << 31;
/// Last segment of a frame.
pub const TX_LS: u32 = 1 << 30;
/// Mask of the frame length in `opts1`.
pub const TX_LEN_MASK: u32 = 0x3_FFFF;

// tx_desc.opts2 checksum-offload flags (Linux `r8152.c`)
/// UDP checksum offload.
pub const UDP_CS: u32 = 1 << 31;
/// TCP checksum offload.
pub const TCP_CS: u32 = 1 << 30;
/// IPv4 checksum offload.
pub const IPV4_CS: u32 = 1 << 29;
/// IPv6 checksum offload.
pub const IPV6_CS: u32 = 1 << 28;

// rx_desc.opts1 (little-endian)
/// Mask of the length (including FCS) in `opts1`.
pub const RX_LEN_MASK: u32 = 0x7FFF;

// Vendor control request (statistics polling, protokoll §4.2)
/// `bmRequestType` of the GET_REGS vendor read (device-to-host, vendor, device).
pub const REQT_READ: u8 = 0xC0;
/// `bRequest` of the GET_REGS vendor read (Linux `RTL8152_REQ_GET_REGS`).
pub const REQ_GET_REGS: u8 = 0x05;
/// Tally-counter register read periodically (~150×/15 s in the capture).
pub const REG_TALLYCNT: u16 = 0xE890;
/// Second tally register address observed in the capture.
pub const REG_TALLYCNT2: u16 = 0xE908;

/// Round `n` up to a multiple of `align` (`align` must be a power of two).
fn align_up(n: usize, align: usize) -> usize {
    (n + align - 1) & !(align - 1)
}

/// Build a bulk-OUT transfer for one Ethernet frame: the 8-byte `tx_desc` plus the frame (no FCS).
///
/// `opts2` are the checksum-offload flags ([`UDP_CS`], [`TCP_CS`], [`IPV4_CS`], [`IPV6_CS`]); pass
/// `0` to compute the checksums in software.
pub fn tx_transfer(frame: &[u8], opts2: u32) -> Vec<u8> {
    let opts1 = (frame.len() as u32 & TX_LEN_MASK) | TX_FS | TX_LS;
    let mut out = Vec::with_capacity(TX_DESC_LEN + frame.len());
    out.extend_from_slice(&opts1.to_le_bytes());
    out.extend_from_slice(&opts2.to_le_bytes());
    out.extend_from_slice(frame);
    out
}

/// The frame length (including FCS) a received `rx_desc.opts1` announces.
pub fn rx_len(opts1: u32) -> usize {
    (opts1 & RX_LEN_MASK) as usize
}

/// Split a bulk-IN transfer into Ethernet frames (without FCS).
///
/// A transfer may hold several frames, each preceded by a 24-byte `rx_desc`; the next descriptor
/// begins 8-byte aligned. A descriptor whose length is 0 ends the transfer. Truncated transfers
/// yield only the complete frames found so far.
pub fn rx_frames(transfer: &[u8]) -> Vec<&[u8]> {
    let mut frames = Vec::new();
    let mut off = 0usize;
    while off + RX_DESC_LEN <= transfer.len() {
        let opts1 = u32::from_le_bytes(transfer[off..off + 4].try_into().expect("4 bytes"));
        let len = rx_len(opts1);
        if len == 0 {
            break;
        }
        let frame_len = len.saturating_sub(FCS_LEN);
        let start = off + RX_DESC_LEN;
        let end = start + frame_len;
        if end > transfer.len() {
            break;
        }
        frames.push(&transfer[start..end]);
        off = align_up(start + len, 8);
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 4.2: `opts1 = 0xC0000062` for a 98-byte frame, transfer length = 8 + frame_len.
    #[test]
    fn tx_descriptor_for_a_98_byte_frame() {
        let frame = vec![0xAB; 98];
        let t = tx_transfer(&frame, 0);
        assert_eq!(t.len(), TX_DESC_LEN + 98);
        let opts1 = u32::from_le_bytes(t[..4].try_into().unwrap());
        assert_eq!(opts1, 0xC000_0062);
        let opts2 = u32::from_le_bytes(t[4..8].try_into().unwrap());
        assert_eq!(opts2, 0, "no checksum offload");
        assert_eq!(&t[8..], &frame[..]);
    }

    #[test]
    fn tx_checksum_flags_match_the_observed_values() {
        // observed in the capture: 0xA0000000, 0x20000000, 0x60000000
        assert_eq!(UDP_CS | IPV4_CS, 0xA000_0000);
        assert_eq!(IPV4_CS, 0x2000_0000);
        assert_eq!(TCP_CS | IPV4_CS, 0x6000_0000);
        let t = tx_transfer(&[0u8; 1], UDP_CS | IPV4_CS);
        assert_eq!(u32::from_le_bytes(t[4..8].try_into().unwrap()), 0xA000_0000);
    }

    /// Section 4.2: a 368-byte transfer = 24-byte descriptor + 338 bytes (334 frame + 4 FCS),
    /// rounded up to 8 → one 334-byte frame.
    #[test]
    fn rx_single_frame_matches_the_capture() {
        let frame = vec![0x5A; 334];
        let mut t = Vec::new();
        t.extend_from_slice(&(334u32 + FCS_LEN as u32).to_le_bytes()); // opts1 = 338
        t.extend_from_slice(&[0u8; RX_DESC_LEN - 4]);
        t.extend_from_slice(&frame);
        t.extend_from_slice(&[1, 2, 3, 4]); // FCS
        assert_eq!(t.len(), RX_DESC_LEN + 338);
        t.resize(368, 0); // pad to 8-byte alignment
        assert_eq!(t.len(), 368);
        let frames = rx_frames(&t);
        assert_eq!(frames, vec![&frame[..]]);
    }

    #[test]
    fn rx_splits_several_frames_in_one_transfer() {
        // status packet (short) and an NFS request, as the capture mixes them
        let a = vec![0x11u8; 40];
        let b = vec![0x22u8; 90];
        let mut t = Vec::new();
        for f in [&a, &b] {
            t.extend_from_slice(&((f.len() + FCS_LEN) as u32).to_le_bytes());
            t.extend_from_slice(&[0u8; RX_DESC_LEN - 4]);
            t.extend_from_slice(f);
            t.extend_from_slice(&[0u8; FCS_LEN]);
            let end = align_up(t.len(), 8);
            t.resize(end, 0);
        }
        let frames = rx_frames(&t);
        assert_eq!(frames, vec![&a[..], &b[..]]);
    }

    #[test]
    fn rx_stops_at_a_zero_length_descriptor_and_tolerates_truncation() {
        let mut t = Vec::new();
        t.extend_from_slice(&[0u8; RX_DESC_LEN]); // end marker
        assert!(rx_frames(&t).is_empty());
        // truncated frame is dropped, not panicked on
        let mut short = Vec::new();
        short.extend_from_slice(&(500u32).to_le_bytes());
        short.extend_from_slice(&[0u8; 8]);
        assert!(rx_frames(&short).is_empty());
    }
}
