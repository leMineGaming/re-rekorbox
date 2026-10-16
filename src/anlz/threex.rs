//! `.3EX` files: a MessagePack document `{"embedding": {"d2": …, …, "d12": …}}` (SPEC §5a).
//!
//! The key semantics are unknown (SPEC U13), so the document is exposed as a generic [`Value`].

use super::{AnlzError, Result};

/// A MessagePack value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    UInt(u64),
    F32(f32),
    F64(f64),
    Str(String),
    Bin(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Ext(i8, Vec<u8>),
}

impl Value {
    /// Look up a string key in a map.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::F64(v) => Some(v),
            Value::F32(v) => Some(v as f64),
            Value::Int(v) => Some(v as f64),
            Value::UInt(v) => Some(v as f64),
            _ => None,
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.b.len() {
            return Err(AnlzError::TooShort { what: "MessagePack", needed: self.pos + n, available: self.b.len() });
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn uint(&mut self, n: usize) -> Result<u64> {
        Ok(self.take(n)?.iter().fold(0u64, |a, &x| (a << 8) | x as u64))
    }

    fn string(&mut self, n: usize) -> Result<Value> {
        let s = self.take(n)?;
        String::from_utf8(s.to_vec())
            .map(Value::Str)
            .map_err(|_| AnlzError::BadLength("MessagePack: invalid UTF-8 string".into()))
    }

    fn array(&mut self, n: usize) -> Result<Value> {
        (0..n).map(|_| self.value()).collect::<Result<Vec<_>>>().map(Value::Array)
    }

    fn map(&mut self, n: usize) -> Result<Value> {
        let mut m = Vec::with_capacity(n);
        for _ in 0..n {
            let k = self.value()?;
            let v = self.value()?;
            m.push((k, v));
        }
        Ok(Value::Map(m))
    }

    fn value(&mut self) -> Result<Value> {
        let t = self.take(1)?[0];
        Ok(match t {
            0x00..=0x7f => Value::UInt(t as u64),
            0x80..=0x8f => self.map((t & 0x0f) as usize)?,
            0x90..=0x9f => self.array((t & 0x0f) as usize)?,
            0xa0..=0xbf => self.string((t & 0x1f) as usize)?,
            0xc0 => Value::Nil,
            0xc2 => Value::Bool(false),
            0xc3 => Value::Bool(true),
            0xc4 | 0xc5 | 0xc6 => {
                let n = self.uint(1 << (t - 0xc4))? as usize;
                Value::Bin(self.take(n)?.to_vec())
            }
            0xc7 | 0xc8 | 0xc9 => {
                let n = self.uint(1 << (t - 0xc7))? as usize;
                let ty = self.take(1)?[0] as i8;
                Value::Ext(ty, self.take(n)?.to_vec())
            }
            0xca => Value::F32(f32::from_bits(self.uint(4)? as u32)),
            0xcb => Value::F64(f64::from_bits(self.uint(8)?)),
            0xcc..=0xcf => Value::UInt(self.uint(1 << (t - 0xcc))?),
            0xd0 => Value::Int(self.take(1)?[0] as i8 as i64),
            0xd1 => Value::Int(self.uint(2)? as u16 as i16 as i64),
            0xd2 => Value::Int(self.uint(4)? as u32 as i32 as i64),
            0xd3 => Value::Int(self.uint(8)? as i64),
            0xd4..=0xd8 => {
                let ty = self.take(1)?[0] as i8;
                Value::Ext(ty, self.take(1 << (t - 0xd4))?.to_vec())
            }
            0xd9 | 0xda | 0xdb => {
                let n = self.uint(1 << (t - 0xd9))? as usize;
                self.string(n)?
            }
            0xdc | 0xdd => {
                let n = self.uint(if t == 0xdc { 2 } else { 4 })? as usize;
                self.array(n)?
            }
            0xde | 0xdf => {
                let n = self.uint(if t == 0xde { 2 } else { 4 })? as usize;
                self.map(n)?
            }
            0xe0..=0xff => Value::Int(t as i8 as i64),
            0xc1 => return Err(AnlzError::UnexpectedType { what: "MessagePack type byte", value: 0xc1 }),
        })
    }
}

/// Decode a complete `.3EX` file. Trailing bytes are an error (all observed files decode exactly).
pub fn decode(data: &[u8]) -> Result<Value> {
    let mut r = Reader { b: data, pos: 0 };
    let v = r.value()?;
    if r.pos != data.len() {
        return Err(AnlzError::BadLength(format!("MessagePack: {} trailing bytes", data.len() - r.pos)));
    }
    Ok(v)
}

/// Encode a value (compact MessagePack forms). Floats keep their stored width.
pub fn encode(v: &Value, out: &mut Vec<u8>) {
    fn len_prefix(out: &mut Vec<u8>, n: usize, fix: u8, fix_max: usize, codes: [u8; 3]) {
        if n <= fix_max {
            out.push(fix | n as u8);
        } else if codes[0] != 0 && n <= 0xff {
            out.push(codes[0]);
            out.push(n as u8);
        } else if n <= 0xffff {
            out.push(codes[1]);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            out.push(codes[2]);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
    }
    match v {
        Value::Nil => out.push(0xc0),
        Value::Bool(b) => out.push(if *b { 0xc3 } else { 0xc2 }),
        Value::UInt(u) if *u <= 0x7f => out.push(*u as u8),
        Value::UInt(u) => {
            out.push(0xcf);
            out.extend_from_slice(&u.to_be_bytes());
        }
        Value::Int(i) if (-32..0).contains(i) => out.push(*i as i8 as u8),
        Value::Int(i) if (0..=0x7f).contains(i) => out.push(*i as u8),
        Value::Int(i) => {
            out.push(0xd3);
            out.extend_from_slice(&i.to_be_bytes());
        }
        Value::F32(f) => {
            out.push(0xca);
            out.extend_from_slice(&f.to_bits().to_be_bytes());
        }
        Value::F64(f) => {
            out.push(0xcb);
            out.extend_from_slice(&f.to_bits().to_be_bytes());
        }
        Value::Str(s) => {
            len_prefix(out, s.len(), 0xa0, 31, [0xd9, 0xda, 0xdb]);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Bin(b) => {
            if b.len() <= 0xff {
                out.extend_from_slice(&[0xc4, b.len() as u8]);
            } else {
                out.push(0xc6);
                out.extend_from_slice(&(b.len() as u32).to_be_bytes());
            }
            out.extend_from_slice(b);
        }
        Value::Array(a) => {
            len_prefix(out, a.len(), 0x90, 15, [0, 0xdc, 0xdd]);
            for x in a {
                encode(x, out);
            }
        }
        Value::Map(m) => {
            len_prefix(out, m.len(), 0x80, 15, [0, 0xde, 0xdf]);
            for (k, x) in m {
                encode(k, out);
                encode(x, out);
            }
        }
        Value::Ext(t, d) => {
            out.push(0xc9);
            out.extend_from_slice(&(d.len() as u32).to_be_bytes());
            out.push(*t as u8);
            out.extend_from_slice(d);
        }
    }
}
