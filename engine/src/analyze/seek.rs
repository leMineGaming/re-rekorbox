//! Seek tables written into the analysis files (SPEC §5j, observed):
//! FLAC → `PVB2` with 400 entries `[u64 frame start sample, u64 byte offset from the first frame,
//! u32 block size]` at `i · ⌊samples / 400⌋` rounded down to a frame; MP3 → `PVBR` with an empty table
//! and the sample count in the extra field; other formats → no table.

use std::path::Path;

/// One FLAC frame: first sample and byte offset relative to the first frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlacFrame {
    pub sample: u64,
    pub offset: u64,
    pub block_size: u32,
}

/// CRC-8 (polynomial 0x07) as used by FLAC frame headers.
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

/// Parse a frame header at `b`; returns (frame or sample number, block size, header length).
fn frame_header(b: &[u8], fixed_block: u32, stream_rate_bits: Option<u32>) -> Option<(u64, u32, bool)> {
    if b.len() < 6 || b[0] != 0xFF || (b[1] & 0xFE) != 0xF8 {
        return None;
    }
    let variable = b[1] & 1 == 1;
    let bs_code = b[2] >> 4;
    let sr_code = b[2] & 0x0F;
    if sr_code == 0x0F || (b[3] >> 4) >= 0x0B || ((b[3] >> 1) & 7) == 3 || ((b[3] >> 1) & 7) == 7 || b[3] & 1 != 0 {
        return None;
    }
    let _ = stream_rate_bits;
    // UTF-8 coded frame/sample number
    let first = b[4];
    let (mut value, extra) = match first {
        0x00..=0x7F => (u64::from(first), 0),
        0xC0..=0xDF => (u64::from(first & 0x1F), 1),
        0xE0..=0xEF => (u64::from(first & 0x0F), 2),
        0xF0..=0xF7 => (u64::from(first & 0x07), 3),
        0xF8..=0xFB => (u64::from(first & 0x03), 4),
        0xFC..=0xFD => (u64::from(first & 0x01), 5),
        0xFE => (0, 6),
        _ => return None,
    };
    let mut p = 5;
    for _ in 0..extra {
        let c = *b.get(p)?;
        if c & 0xC0 != 0x80 {
            return None;
        }
        value = (value << 6) | u64::from(c & 0x3F);
        p += 1;
    }
    let block = match bs_code {
        0 => return None,
        1 => 192,
        2..=5 => 576 << (bs_code - 2),
        6 => {
            p += 1;
            u32::from(*b.get(p - 1)?) + 1
        }
        7 => {
            p += 2;
            (u32::from(*b.get(p - 2)?) << 8 | u32::from(*b.get(p - 1)?)) + 1
        }
        _ => 256 << (bs_code - 8),
    };
    match sr_code {
        12 => p += 1,
        13 | 14 => p += 2,
        _ => {}
    }
    let crc = *b.get(p)?;
    if crc8(&b[..p]) != crc {
        return None;
    }
    let sample = if variable { value } else { value * u64::from(fixed_block.max(block)) };
    let _ = sample;
    Some((value, block, variable))
}

/// All frames of a FLAC file (sample position, byte offset from the first frame, block size).
pub fn flac_frames(path: &Path) -> Option<Vec<FlacFrame>> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 8 || &b[..4] != b"fLaC" {
        return None;
    }
    // skip metadata blocks; STREAMINFO gives the (max) block size
    let mut p = 4;
    let mut max_block = 4096u32;
    loop {
        let hdr = *b.get(p)?;
        let len = (usize::from(*b.get(p + 1)?) << 16) | (usize::from(*b.get(p + 2)?) << 8) | usize::from(*b.get(p + 3)?);
        if hdr & 0x7F == 0 && len >= 4 {
            max_block = u32::from(u16::from_be_bytes([b[p + 6], b[p + 7]]));
        }
        p += 4 + len;
        if hdr & 0x80 != 0 {
            break;
        }
    }
    let first = p;
    let mut frames = Vec::new();
    let mut i = first;
    let mut expected_next = 0u64;
    while i + 16 < b.len() {
        if b[i] == 0xFF && (b[i + 1] & 0xFE) == 0xF8 {
            if let Some((num, block, variable)) = frame_header(&b[i..(i + 16).min(b.len())], max_block, None) {
                let sample = if variable { num } else { num * u64::from(max_block) };
                // accept the next frame in sequence (a CRC-valid header with the expected number);
                // frames can be very short (silence), so only skip the sync code itself
                if sample == expected_next || frames.is_empty() {
                    frames.push(FlacFrame { sample, offset: (i - first) as u64, block_size: block });
                    expected_next = sample + u64::from(block);
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    Some(frames)
}

/// `PVB2` payload for a FLAC file with `samples` total samples: 400 entries of 20 bytes.
pub fn flac_pvb2(frames: &[FlacFrame], samples: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(400 * 20);
    for i in 0..400u64 {
        let target = i * (samples / 400);
        let idx = frames.partition_point(|f| f.sample <= target).saturating_sub(1);
        let f = frames.get(idx).copied().unwrap_or(FlacFrame { sample: 0, offset: 0, block_size: 4096 });
        out.extend_from_slice(&f.sample.to_be_bytes());
        out.extend_from_slice(&f.offset.to_be_bytes());
        out.extend_from_slice(&f.block_size.to_be_bytes());
    }
    out
}
