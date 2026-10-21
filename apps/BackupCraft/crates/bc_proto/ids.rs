// ids -- identifiers and small text helpers shared by every program
//
//   uuid(random16) -> "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"   a version 4 UUID from 16 random
//                     bytes (the caller supplies them: plat::fill_random)
//   is_uuid(s)        exact UUID text (lower-case hex)
//   key(s) -> u64     a stable 64-bit key of a text (FNV-1a; grid rows, cache keys)
//   percent_encode / percent_decode   query values and path segments
//   hex / unhex

use alloc::string::String;
use alloc::vec::Vec;

const HEX: &[u8; 16] = b"0123456789abcdef";

/// a version 4 UUID from 16 random bytes
pub fn uuid(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let mut s = String::with_capacity(36);
    for (i, x) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 15) as usize] as char);
    }
    s
}

/// whether s is UUID text (8-4-4-4-12 lower-case hex digits)
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| {
            if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { c.is_ascii_digit() || (b'a'..=b'f').contains(&c) }
        })
}

/// a stable 64-bit key of a text (FNV-1a)
pub fn key(s: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &c in s.as_bytes() {
        h ^= c as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// lower-case hex of bytes
pub fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 15) as usize] as char);
    }
    s
}

/// bytes of hex text (None on odd length or a non-hex digit)
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let d = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    b.chunks(2).map(|p| Some(d(p[0])? << 4 | d(p[1])?)).collect()
}

/// percent-encode everything but unreserved characters (and '/' when keep_slash)
pub fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') || (keep_slash && c == b'/') {
            out.push(c as char);
        } else {
            out.push('%');
            out.push(HEX[(c >> 4) as usize].to_ascii_uppercase() as char);
            out.push(HEX[(c & 15) as usize].to_ascii_uppercase() as char);
        }
    }
    out
}

/// decode %XX escapes and '+' (as a space); invalid escapes stay as they are
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match unhex(core::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz")) {
                    Some(v) => {
                        out.push(v[0]);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}
