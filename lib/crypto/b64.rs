// b64 -- Base64 (RFC 4648): the standard and the URL-safe alphabet, with or without padding

use alloc::string::String;
use alloc::vec::Vec;

const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn enc(data: &[u8], alpha: &[u8; 64], pad: bool) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        let k = c.len() + 1;
        for i in 0..4 {
            if i < k {
                s.push(alpha[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else if pad {
                s.push('=');
            }
        }
    }
    s
}

fn dec(s: &str, url: bool) -> Option<Vec<u8>> {
    let b = s.trim_end_matches('=').as_bytes();
    if b.len() % 4 == 1 {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' if !url => 62,
            b'/' if !url => 63,
            b'-' if url => 62,
            b'_' if url => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    for c in b.chunks(4) {
        let mut n = 0u32;
        for (i, &x) in c.iter().enumerate() {
            n |= val(x)? << (18 - 6 * i);
        }
        let bytes = c.len() - 1;
        for i in 0..bytes {
            out.push((n >> (16 - 8 * i)) as u8);
        }
        // the unused low bits of a partial group must be zero (one encoding per value)
        let used = 8 * bytes;
        if n & ((1 << (24 - used)) - 1) != 0 && bytes < 3 {
            return None;
        }
    }
    Some(out)
}

/// standard alphabet, padded
pub fn encode(data: &[u8]) -> String {
    enc(data, STD, true)
}

/// standard alphabet, no padding (PHC strings)
pub fn encode_nopad(data: &[u8]) -> String {
    enc(data, STD, false)
}

/// URL-safe alphabet, no padding (tokens, ids)
pub fn encode_url(data: &[u8]) -> String {
    enc(data, URL, false)
}

/// standard alphabet, padding optional
pub fn decode(s: &str) -> Option<Vec<u8>> {
    dec(s, false)
}

/// URL-safe alphabet, padding optional
pub fn decode_url(s: &str) -> Option<Vec<u8>> {
    dec(s, true)
}
