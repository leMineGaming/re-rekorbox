//! DeviceSQL string codec and .edb container tests on synthetic data (SPEC §5h).

use rekordbox_formats::devicesql::{DsqlString, EdbFile, TEXT_END, TEXT_START};

#[test]
fn short_ascii_strings() {
    let s = DsqlString::ShortAscii("Pink".into());
    let b = s.encode().unwrap();
    assert_eq!(b, [0x0b, b'P', b'i', b'n', b'k'], "kind = ((4+1)<<1)|1");
    assert_eq!(DsqlString::decode(&b).unwrap(), (s, 5));
    assert_eq!(DsqlString::ShortAscii("Red".into()).encode().unwrap()[0], 0x09);
    assert_eq!(DsqlString::ShortAscii(String::new()).encode().unwrap(), [0x03]);
    assert!(DsqlString::ShortAscii("x".repeat(127)).encode().is_err());
}

#[test]
fn long_utf16_strings_with_markers() {
    let s = DsqlString::marked_utf16("DEFAULT");
    let b = s.encode().unwrap();
    assert_eq!(&b[..4], &[0x90, 0x16, 0x00, 0x00], "0x90, total 22 bytes, pad");
    assert_eq!(&b[4..8], &[0xfa, 0xff, b'D', 0x00]);
    assert_eq!(&b[b.len() - 2..], &[0xfb, 0xff]);
    let (back, n) = DsqlString::decode(&b).unwrap();
    assert_eq!(n, 22);
    assert_eq!(back.text(), "DEFAULT");
    match back {
        DsqlString::LongUtf16(u) => assert_eq!((u[0], *u.last().unwrap()), (TEXT_START, TEXT_END)),
        _ => panic!(),
    }
    assert_eq!(DsqlString::marked_utf16("Ärger €").text(), "Ärger €");
}

#[test]
fn decode_errors() {
    assert!(DsqlString::decode(&[]).is_err());
    assert!(DsqlString::decode(&[0x0b, b'P']).is_err(), "truncated");
    assert!(DsqlString::decode(&[0x42]).is_err(), "unknown kind");
    assert!(DsqlString::decode(&[0x90, 0x05, 0x00, 0x00, 0x00]).is_err(), "odd UTF-16 length");
}

fn synthetic_edb(rows: &[Vec<u8>]) -> Vec<u8> {
    // header (0x38 bytes) + offset table + 0 terminator, rows after a small gap
    let table_len = (rows.len() + 1) * 2;
    let first = 0x38 + table_len + 2;
    let mut offsets = Vec::new();
    let mut pos = first;
    for r in rows {
        offsets.push(pos);
        pos += r.len();
    }
    let total = pos;
    let mut b = vec![0u8; first];
    b[12..16].copy_from_slice(&(total as u32).to_le_bytes());
    for (k, o) in offsets.iter().enumerate() {
        let rel = (o - 0x14) as u16;
        b[0x38 + k * 2..0x38 + k * 2 + 2].copy_from_slice(&rel.to_le_bytes());
    }
    for r in rows {
        b.extend_from_slice(r);
    }
    b
}

#[test]
fn edb_container() {
    let mut colour = vec![0x80, 0x03, 0x20, 0x00, 0, 0, 0, 0, 0x00, 0x01, 0x00, 0x00];
    colour.extend(DsqlString::ShortAscii("Pink".into()).encode().unwrap());
    let mut column = vec![0xa0, 0x04, 0x40, 0x00, 0x19, 0x00, 0xa1, 0x00];
    column.extend(DsqlString::marked_utf16("DEFAULT").encode().unwrap());
    let bytes = synthetic_edb(&[colour.clone(), column.clone()]);
    let f = EdbFile::parse(&bytes).unwrap();
    assert_eq!(f.rows.len(), 2);
    assert_eq!(f.header[3] as usize, bytes.len());
    assert_eq!((f.rows[0].kind(), f.rows[0].index_shift()), (0x0380, 0x20));
    assert_eq!(f.rows[0].string_at(12).unwrap().text(), "Pink");
    assert_eq!(f.rows[1].string_at(8).unwrap().text(), "DEFAULT");
    assert_eq!(f.rows[1].offset + f.rows[1].data.len(), bytes.len(), "last row runs to the end of the file");

    let mut bad = bytes.clone();
    bad[12] ^= 1;
    assert!(EdbFile::parse(&bad).is_err(), "file size field must match");
}
