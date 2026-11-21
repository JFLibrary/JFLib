// b64 -- Base64 (RFC 4648): the standard and the URL-safe alphabet, with or without padding

use alloc::string::String;
use alloc::vec::Vec;

// Keys go through here (PEM, OpenSSH key files, PHC strings), so neither direction branches on
// or indexes by a character or a value: the alphabet is arithmetic on masks (ct_mask), and
// decoding looks at every character before it answers.  Lengths and padding are public.

/// all ones when lo <= x <= hi, else 0 (x, lo, hi small)
#[inline(always)]
fn within(x: u32, lo: u32, hi: u32) -> u32 {
    let (x, lo, hi) = (x as i32, lo as i32, hi as i32);
    crate::ct_mask((((lo - 1 - x) & (x - hi - 1)) >> 31) as u32 as u64) as u32
}

/// the character of a six-bit value
#[inline(always)]
fn char_of(v: u32, url: bool) -> u8 {
    let (c62, c63) = if url { (b'-', b'_') } else { (b'+', b'/') };
    let c = (within(v, 0, 25) & (v + b'A' as u32))
        | (within(v, 26, 51) & (v + b'a' as u32 - 26))
        | (within(v, 52, 61) & (v + b'0' as u32).wrapping_sub(52))
        | (within(v, 62, 62) & c62 as u32)
        | (within(v, 63, 63) & c63 as u32);
    c as u8
}

/// the six-bit value of a character and all ones, or 0 and 0 for a character outside the alphabet
#[inline(always)]
fn value_of(c: u8, url: bool) -> (u32, u32) {
    let c = c as u32;
    let (c62, c63) = if url { (b'-', b'_') } else { (b'+', b'/') };
    let (up, low, dig) = (within(c, 65, 90), within(c, 97, 122), within(c, 48, 57));
    let (m62, m63) = (within(c, c62 as u32, c62 as u32), within(c, c63 as u32, c63 as u32));
    let v = (up & c.wrapping_sub(65)) | (low & c.wrapping_sub(71)) | (dig & (c + 4)) | (m62 & 62) | (m63 & 63);
    (v, up | low | dig | m62 | m63)
}

fn enc(data: &[u8], url: bool, pad: bool) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        let k = c.len() + 1;
        for i in 0..4 {
            if i < k {
                s.push(char_of(n >> (18 - 6 * i) & 63, url) as char);
            } else if pad {
                s.push('=');
            }
        }
    }
    s
}

fn dec(s: &str, url: bool) -> Option<Vec<u8>> {
    let body = s.trim_end_matches('=');
    let pad = s.len() - body.len();
    // padding is optional, but when present it completes the last group of four
    if pad > 2 || (pad > 0 && s.len() % 4 != 0) {
        return None;
    }
    let b = body.as_bytes();
    if b.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() * 3 / 4);
    // all ones while every character is in the alphabet and no partial group has bits left over
    let mut good = u32::MAX;
    for c in b.chunks(4) {
        let mut n = 0u32;
        for (i, &x) in c.iter().enumerate() {
            let (v, ok) = value_of(x, url);
            n |= v << (18 - 6 * i);
            good &= ok;
        }
        let bytes = c.len() - 1;
        for i in 0..bytes {
            out.push((n >> (16 - 8 * i)) as u8);
        }
        // the unused low bits of a partial group must be zero (one encoding per value)
        if bytes < 3 {
            let rest = n & ((1 << (24 - 8 * bytes)) - 1);
            good &= crate::ct_mask((((rest | rest.wrapping_neg()) >> 31) as u64).wrapping_sub(1)) as u32;
        }
    }
    if good == 0 {
        crate::zeroize(&mut out);
        return None;
    }
    Some(out)
}

/// standard alphabet, padded
pub fn encode(data: &[u8]) -> String {
    enc(data, false, true)
}

/// standard alphabet, no padding (PHC strings)
pub fn encode_nopad(data: &[u8]) -> String {
    enc(data, false, false)
}

/// URL-safe alphabet, no padding (tokens, ids)
pub fn encode_url(data: &[u8]) -> String {
    enc(data, true, false)
}

/// standard alphabet, padding optional
pub fn decode(s: &str) -> Option<Vec<u8>> {
    dec(s, false)
}

/// URL-safe alphabet, padding optional
pub fn decode_url(s: &str) -> Option<Vec<u8>> {
    dec(s, true)
}
