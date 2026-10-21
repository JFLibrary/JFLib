// num -- numbers <-> text                                                  [src/core/ini.s]
//
//   f64_parse(s) -> (value, used)   leading blanks, sign, digits, '.', exponent, inf /
//                                   infinity / nan; used = 0 when there is no number
//   f64_format(v) -> String         the shortest text that reads back to the same double,
//                                   '.' separator, "1e-07" style below 1e-5 and from 1e17,
//                                   "inf" / "-inf" / "nan"
//   parse_i64(s) -> Option<i64>     decimal (+/-), 0x hex, true/false/yes/no/on/off (the whole
//                                   string; overflow -> None)
//
// The scaling steps run on the x87 unit like in ASMLIB (rt::x87), so both directions are
// bit-identical to the assembly -- the text of a saved setting included.

use alloc::string::String;
use alloc::vec::Vec;

const EXP_MAX: i32 = 5000; // decimal exponents are clamped to +-EXP_MAX

const POW10_INT: [u64; 19] = [
    1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000, 1000000000, 10000000000, 100000000000,
    1000000000000, 10000000000000, 100000000000000, 1000000000000000, 10000000000000000, 100000000000000000,
    1000000000000000000,
];

#[inline]
fn at(s: &[u8], i: usize) -> u8 {
    if i < s.len() { s[i] } else { 0 }
}

/// s starts with the lower-case word, ASCII case-insensitive (stops at the end of s)  [ieq_z]
fn ieq_z(s: &[u8], i: usize, word: &[u8]) -> bool {
    word.iter().enumerate().all(|(k, &w)| {
        let c = at(s, i + k);
        c != 0 && c | 32 == w
    })
}

/// the number at the start of s: (value, bytes used; 0 = none)  [f64_parse]
pub fn f64_parse(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while matches!(at(s, i), b' ' | b'\t') {
        i += 1;
    }
    let mut neg = false;
    match at(s, i) {
        b'-' => {
            neg = true;
            i += 1;
        }
        b'+' => i += 1,
        _ => {}
    }
    let signed = |v: f64| if neg { f64::from_bits(v.to_bits() ^ 1 << 63) } else { v };
    match at(s, i) | 32 {
        b'i' => {
            if at(s, i + 1) | 32 != b'n' || at(s, i + 2) | 32 != b'f' {
                return (0.0, 0);
            }
            i += 3;
            if ieq_z(s, i, b"inity") {
                i += 5;
            }
            return (signed(f64::INFINITY), i);
        }
        b'n' => {
            if at(s, i + 1) | 32 != b'a' || at(s, i + 2) | 32 != b'n' {
                return (0.0, 0);
            }
            return (signed(f64::from_bits(0x7FF8_0000_0000_0000)), i + 3);
        }
        _ => {}
    }
    let mut m: u64 = 0; // at most 18 digits
    let mut kept = 0;
    let mut e10: i32 = 0;
    let mut seen = 0;
    loop {
        let d = at(s, i).wrapping_sub(b'0');
        if d > 9 {
            break;
        }
        seen += 1;
        i += 1;
        if m == 0 && d == 0 {
            continue; // leading zero
        }
        if kept < 18 {
            m = m * 10 + d as u64;
            kept += 1;
        } else if e10 < 100000 {
            e10 += 1;
        }
    }
    if at(s, i) == b'.' {
        i += 1;
        loop {
            let d = at(s, i).wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            seen += 1;
            i += 1;
            if m == 0 && d == 0 {
                if e10 > -100000 {
                    e10 -= 1; // 0.000...: only the exponent moves
                }
                continue;
            }
            if kept < 18 {
                m = m * 10 + d as u64;
                kept += 1;
                e10 -= 1;
            }
        }
    }
    if seen == 0 {
        return (0.0, 0);
    }
    if at(s, i) | 32 == b'e' {
        let mut j = i + 1;
        let mut eneg = false;
        match at(s, j) {
            b'-' => {
                eneg = true;
                j += 1;
            }
            b'+' => j += 1,
            _ => {}
        }
        if at(s, j).wrapping_sub(b'0') <= 9 {
            let mut ed: i32 = 0;
            loop {
                let d = at(s, j).wrapping_sub(b'0');
                if d > 9 {
                    break;
                }
                if ed < 100000 {
                    ed = ed * 10 + d as i32;
                }
                j += 1;
            }
            i = j;
            e10 += if eneg { -ed } else { ed };
        } // "e" without digits is not ours
    }
    let v = if m == 0 { 0.0 } else { rt::x87::scale(m, e10.clamp(-EXP_MAX, EXP_MAX)) };
    (signed(v), i)
}

/// the p-digit integer n (10^(p-1) <= n < 10^p) as n * 10^(e10-p+1), no trailing zeros  [f64_render]
fn render(mut n: u64, p: usize, e10: i32) -> Vec<u8> {
    let mut dig = [0u8; 17];
    for c in (0..p).rev() {
        dig[c] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let mut sig = p;
    while sig > 1 && dig[sig - 1] == b'0' {
        sig -= 1;
    }
    let dig = &dig[..sig];
    let mut out = Vec::with_capacity(32);
    if (-5..17).contains(&e10) {
        if e10 >= 0 {
            let int = e10 as usize + 1;
            for c in 0..int {
                out.push(if c < sig { dig[c] } else { b'0' });
            }
            if sig > int {
                out.push(b'.');
                out.extend_from_slice(&dig[int..]);
            }
        } else {
            out.extend_from_slice(b"0.");
            for _ in 0..(-e10 - 1) {
                out.push(b'0');
            }
            out.extend_from_slice(dig);
        }
    } else {
        out.push(dig[0]);
        if sig > 1 {
            out.push(b'.');
            out.extend_from_slice(&dig[1..]);
        }
        out.push(b'e');
        if e10 < 0 {
            out.push(b'-');
        }
        let a = e10.unsigned_abs();
        if a < 10 {
            out.push(b'0'); // at least two exponent digits: 1e-07
        }
        if a >= 100 {
            out.push(b'0' + (a / 100) as u8);
            out.push(b'0' + (a % 100 / 10) as u8);
        } else if a >= 10 {
            out.push(b'0' + (a / 10) as u8);
        }
        out.push(b'0' + (a % 10) as u8);
    }
    out
}

/// the shortest decimal text that reads back to v  [f64_format]
pub fn f64_format(v: f64) -> String {
    let bits = v.to_bits();
    let mut neg = bits >> 63 != 0;
    let abs = bits & !(1 << 63);
    let text: Vec<u8> = if abs >> 52 == 0x7FF {
        if abs << 12 != 0 {
            neg = false;
            b"nan".to_vec()
        } else {
            b"inf".to_vec()
        }
    } else if abs == 0 {
        b"0".to_vec()
    } else {
        let av = f64::from_bits(abs);
        let mut e10 = rt::x87::log10_round(av); // decimal exponent, +-1 (fixed below)
        let mut p = 1usize; // significant digits
        loop {
            let mut budget = 4;
            let n = loop {
                let k = p as i32 - 1 - e10;
                let n = rt::x87::scale_round(av, k) as u64;
                if n >= POW10_INT[p] {
                    e10 += 1; // the exponent was too small
                } else if n < POW10_INT[p - 1] {
                    e10 -= 1; // too large
                } else {
                    break n;
                }
                budget -= 1;
                if budget == 0 {
                    break n;
                }
            };
            let t = render(n, p, e10);
            if f64_parse(&t).0.to_bits() == abs || p >= 17 {
                break t;
            }
            p += 1;
        }
    };
    let mut s = String::with_capacity(text.len() + 1);
    if neg {
        s.push('-');
    }
    s.push_str(core::str::from_utf8(&text).unwrap_or("0"));
    s
}

/// an integer setting: the whole string must match  [parse_i64]
pub fn parse_i64(s: &[u8]) -> Option<i64> {
    const WORDS: [(&[u8], i64); 6] =
        [(b"true", 1), (b"false", 0), (b"yes", 1), (b"no", 0), (b"on", 1), (b"off", 0)];
    for (w, v) in WORDS {
        if ieq_z(s, 0, w) && at(s, w.len()) == 0 {
            return Some(v);
        }
    }
    let mut i = 0;
    let mut neg = false;
    match at(s, 0) {
        b'-' => {
            neg = true;
            i = 1;
        }
        b'+' => i = 1,
        _ => {}
    }
    let mut v: u64 = 0;
    let mut digits = 0;
    if at(s, i) == b'0' && at(s, i + 1) | 32 == b'x' {
        i += 2;
        loop {
            let c = at(s, i);
            let d = match c {
                b'0'..=b'9' => c - b'0',
                _ if (b'a'..=b'f').contains(&(c | 32)) => (c | 32) - 87,
                _ => break,
            };
            if digits >= 16 {
                return None; // more than 64 bits
            }
            v = v << 4 | d as u64;
            digits += 1;
            i += 1;
        }
    } else {
        loop {
            let d = at(s, i).wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            if v > 922337203685477580 {
                return None;
            }
            v = v * 10 + d as u64;
            if v > i64::MAX as u64 + neg as u64 {
                return None; // |min| is one more than max
            }
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 || at(s, i) != 0 {
        return None;
    }
    let v = v as i64;
    Some(if neg { v.wrapping_neg() } else { v })
}
