// obj -- the bytes of PDF objects: numbers, names, strings, dictionaries (ISO 32000-1, 7.3)
//
// Numbers have at most seven decimals and about eight significant digits, never an exponent
// and never "-0": the same value always gives the same bytes.

use alloc::vec::Vec;
use core::fmt::Write;

/// core::fmt into a byte buffer
pub(crate) struct Fmt<'a>(pub &'a mut Vec<u8>);

impl Write for Fmt<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// a real or integer number: up to 7 decimals, fewer from 10 upwards (8 significant digits),
/// trailing zeros dropped; not finite -> 0
pub(crate) fn num(o: &mut Vec<u8>, v: f64) {
    if !v.is_finite() {
        o.push(b'0');
        return;
    }
    let a = v.abs();
    let mut digits = 1usize;
    let mut p = 10.0;
    while a >= p && digits < 24 {
        digits += 1;
        p *= 10.0;
    }
    let prec = 8usize.saturating_sub(digits).min(7);
    let start = o.len();
    let _ = write!(Fmt(o), "{v:.prec$}");
    if prec > 0 {
        while o.last() == Some(&b'0') {
            o.pop();
        }
        if o.last() == Some(&b'.') {
            o.pop();
        }
    }
    // "-0" (a small negative value rounded away) is plain 0
    if &o[start..] == b"-0" {
        o.truncate(start);
        o.push(b'0');
    }
}

pub(crate) fn int(o: &mut Vec<u8>, v: i64) {
    let _ = write!(Fmt(o), "{v}");
}

/// `n 0 R`
pub(crate) fn r(o: &mut Vec<u8>, n: u32) {
    let _ = write!(Fmt(o), "{n} 0 R");
}

/// a name: `/` and the bytes, those outside ! .. ~ and the delimiters and # as #xx
pub(crate) fn name(o: &mut Vec<u8>, s: &[u8]) {
    o.push(b'/');
    for &c in s {
        if (0x21..=0x7E).contains(&c) && !b"#()<>[]{}/%".contains(&c) {
            o.push(c);
        } else {
            o.push(b'#');
            hex_byte(o, c);
        }
    }
}

fn hex_byte(o: &mut Vec<u8>, c: u8) {
    const H: &[u8; 16] = b"0123456789ABCDEF";
    o.push(H[(c >> 4) as usize]);
    o.push(H[(c & 15) as usize]);
}

/// `<hex>`
pub(crate) fn hex(o: &mut Vec<u8>, s: &[u8]) {
    o.push(b'<');
    for &c in s {
        hex_byte(o, c);
    }
    o.push(b'>');
}

/// a literal string `( )` of bytes: \ ( ) escaped, control bytes and bytes above 126 as octal
pub(crate) fn literal(o: &mut Vec<u8>, s: &[u8]) {
    o.push(b'(');
    for &c in s {
        match c {
            b'\\' | b'(' | b')' => {
                o.push(b'\\');
                o.push(c);
            }
            0x20..=0x7E => o.push(c),
            _ => {
                let _ = write!(Fmt(o), "\\{c:03o}");
            }
        }
    }
    o.push(b')');
}

/// a text string (7.9.2.2): printable ASCII as a literal, anything else as UTF-16BE with a
/// byte order mark
pub(crate) fn text(o: &mut Vec<u8>, s: &str) {
    if s.bytes().all(|c| (0x20..=0x7E).contains(&c)) {
        literal(o, s.as_bytes());
    } else {
        let mut b = Vec::with_capacity(2 + s.len() * 2);
        b.extend_from_slice(&[0xFE, 0xFF]);
        for u in s.encode_utf16() {
            b.extend_from_slice(&u.to_be_bytes());
        }
        hex(o, &b);
    }
}

/// UTF-16BE of s (ToUnicode targets), at most `max` code units
pub(crate) fn utf16be(s: &str, max: usize) -> Vec<u8> {
    let mut b = Vec::new();
    let mut n = 0;
    for c in s.chars() {
        let mut buf = [0u16; 2];
        let units = c.encode_utf16(&mut buf);
        if n + units.len() > max {
            break;
        }
        n += units.len();
        for u in units.iter() {
            b.extend_from_slice(&u.to_be_bytes());
        }
    }
    b
}

/// a URI for a link action: ASCII, everything outside the printable range percent-encoded
/// (an IRI's UTF-8)
pub(crate) fn uri_bytes(s: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(s.len());
    for &c in s.as_bytes() {
        if (0x21..=0x7E).contains(&c) {
            b.push(c);
        } else {
            b.push(b'%');
            hex_byte(&mut b, c);
        }
    }
    b
}

/// an array of numbers `[a b c]`
pub(crate) fn nums(o: &mut Vec<u8>, v: &[f64]) {
    o.push(b'[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            o.push(b' ');
        }
        num(o, *x);
    }
    o.push(b']');
}

/// one byte string of an FNV-1a style 64-bit hash (for the file ID and subset tags)
pub(crate) fn hash64(seed: u64, data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
    for &c in data {
        h ^= c as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    // a final mix so close inputs differ in every byte
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h
}
