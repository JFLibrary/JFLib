// num -- IEEE 754 binary64 arithmetic in software, so float results of scripts are
// bit-identical on every target
//
//   add sub mul div sqrt     correctly rounded (to nearest, ties to even)
//   rem                      fmod: exact, with the sign of the dividend
//   from_i64(i)              correctly rounded; to_i64(x) truncates (None for NaN, infinity and
//                            values outside i64)
//   cmp / eq / lt / le       comparisons: NaN is unordered, -0 == +0
//   cmp_int(i, x)            an i64 against a float, exactly (i is not rounded first)
//   neg abs trunc floor ceil
//   NAN                      the one NaN every operation returns (0x7FF8_0000_0000_0000)
//   fmt(x, out)              shortest text that reads back as the same bits: 1.0, 0.1, 1e21,
//                            -0.0, inf, -inf, nan
//
// Only integer arithmetic: the floating-point unit is never used, so modes such as flush to
// zero and the NaN bits of different processors cannot change a result.  (For finite values
// x86-64, ARM64 and wasm hardware would agree with these functions; the NaN bits do not.)

use alloc::format;
use alloc::string::String;
use core::cmp::Ordering;

/// the NaN every operation of this module returns
pub const NAN: f64 = f64::from_bits(0x7FF8_0000_0000_0000);

const SIGN: u64 = 1 << 63;
const EXP: u64 = 0x7FF << 52;
const FRAC: u64 = (1 << 52) - 1;

/// what a float is; Finite(e, m): m has bit 63 set and the value is m * 2^(e - 63)
#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Nan,
    Inf,
    Zero,
    Finite(i32, u64),
}

fn unpack(x: u64) -> (bool, Class) {
    let sign = x & SIGN != 0;
    let be = ((x >> 52) & 0x7FF) as i32;
    let frac = x & FRAC;
    if be == 0x7FF {
        return (sign, if frac != 0 { Class::Nan } else { Class::Inf });
    }
    if be == 0 {
        if frac == 0 {
            return (sign, Class::Zero);
        }
        // subnormal: frac * 2^-1074
        let lz = frac.leading_zeros();
        return (sign, Class::Finite(-1011 - lz as i32, frac << lz));
    }
    (sign, Class::Finite(be - 1023, ((1 << 52) | frac) << 11))
}

fn inf(sign: bool) -> f64 {
    f64::from_bits(if sign { SIGN | EXP } else { EXP })
}

fn zero(sign: bool) -> f64 {
    f64::from_bits(if sign { SIGN } else { 0 })
}

/// round (m + sticky) * 2^(e - 63) to a float; m has bit 63 set
fn pack(sign: bool, e: i32, m: u64, sticky: bool) -> f64 {
    let s = if sign { SIGN } else { 0 };
    let be = e + 1023;
    if be >= 0x7FF {
        return inf(sign);
    }
    let (mut m, mut sticky) = (m, sticky);
    let sub = be <= 0;
    if sub {
        let sh = (1 - be) as u32;
        if sh >= 64 {
            sticky |= m != 0;
            m = 0;
        } else {
            sticky |= m << (64 - sh) != 0;
            m >>= sh;
        }
    }
    let rb = m & 0x7FF;
    let mut sig = m >> 11;
    if rb > 0x400 || (rb == 0x400 && (sticky || sig & 1 != 0)) {
        sig += 1;
    }
    let bits = if sub { sig } else { ((be as u64 - 1) << 52) + sig };
    f64::from_bits(s | bits.min(EXP))
}

/// x >> d, with the bits shifted out ORed into bit 0
fn shr_jam(x: u128, d: u32) -> u128 {
    if d == 0 {
        x
    } else if d >= 128 {
        (x != 0) as u128
    } else {
        (x >> d) | (x << (128 - d) != 0) as u128
    }
}

/// whether x is a NaN
pub fn is_nan(x: f64) -> bool {
    x.to_bits() & !SIGN > EXP
}

/// a + b
pub fn add(a: f64, b: f64) -> f64 {
    let (sa, ca) = unpack(a.to_bits());
    let (sb, cb) = unpack(b.to_bits());
    match (ca, cb) {
        (Class::Nan, _) | (_, Class::Nan) => NAN,
        (Class::Inf, Class::Inf) => {
            if sa == sb { inf(sa) } else { NAN }
        }
        (Class::Inf, _) => inf(sa),
        (_, Class::Inf) => inf(sb),
        (Class::Zero, Class::Zero) => zero(sa && sb),
        (Class::Zero, _) => b,
        (_, Class::Zero) => a,
        (Class::Finite(ea, ma), Class::Finite(eb, mb)) => {
            let ((sa, ea, ma), (sb, eb, mb)) = if (ea, ma) >= (eb, mb) { ((sa, ea, ma), (sb, eb, mb)) } else { ((sb, eb, mb), (sa, ea, ma)) };
            let x = (ma as u128) << 62;
            let y = shr_jam((mb as u128) << 62, (ea - eb) as u32);
            let s = if sa == sb { x + y } else { x - y };
            if s == 0 {
                return zero(false);
            }
            let lz = s.leading_zeros();
            let s = s << lz;
            pack(sa, ea + 2 - lz as i32, (s >> 64) as u64, s as u64 != 0)
        }
    }
}

/// a - b
pub fn sub(a: f64, b: f64) -> f64 {
    if is_nan(b) {
        return NAN;
    }
    add(a, f64::from_bits(b.to_bits() ^ SIGN))
}

/// a * b
pub fn mul(a: f64, b: f64) -> f64 {
    let (sa, ca) = unpack(a.to_bits());
    let (sb, cb) = unpack(b.to_bits());
    let sign = sa != sb;
    match (ca, cb) {
        (Class::Nan, _) | (_, Class::Nan) => NAN,
        (Class::Inf, Class::Zero) | (Class::Zero, Class::Inf) => NAN,
        (Class::Inf, _) | (_, Class::Inf) => inf(sign),
        (Class::Zero, _) | (_, Class::Zero) => zero(sign),
        (Class::Finite(ea, ma), Class::Finite(eb, mb)) => {
            let p = ma as u128 * mb as u128;
            let (e, p) = if p >> 127 != 0 { (ea + eb + 1, p) } else { (ea + eb, p << 1) };
            pack(sign, e, (p >> 64) as u64, p as u64 != 0)
        }
    }
}

/// a / b
pub fn div(a: f64, b: f64) -> f64 {
    let (sa, ca) = unpack(a.to_bits());
    let (sb, cb) = unpack(b.to_bits());
    let sign = sa != sb;
    match (ca, cb) {
        (Class::Nan, _) | (_, Class::Nan) => NAN,
        (Class::Inf, Class::Inf) | (Class::Zero, Class::Zero) => NAN,
        (Class::Inf, _) | (_, Class::Zero) => inf(sign),
        (_, Class::Inf) | (Class::Zero, _) => zero(sign),
        (Class::Finite(ea, ma), Class::Finite(eb, mb)) => {
            let n = (ma as u128) << 64;
            let q = n / mb as u128;
            let r = n % mb as u128;
            if q >> 64 != 0 {
                pack(sign, ea - eb, (q >> 1) as u64, r != 0 || q & 1 != 0)
            } else {
                pack(sign, ea - eb - 1, q as u64, r != 0)
            }
        }
    }
}

/// the remainder of a / b truncated (C's fmod): exact, with the sign of a
pub fn rem(a: f64, b: f64) -> f64 {
    let (sa, ca) = unpack(a.to_bits());
    let (_, cb) = unpack(b.to_bits());
    match (ca, cb) {
        (Class::Nan, _) | (_, Class::Nan) | (Class::Inf, _) | (_, Class::Zero) => NAN,
        (Class::Zero, _) | (_, Class::Inf) => a,
        (Class::Finite(ea, ma), Class::Finite(eb, mb)) => {
            if (ea, ma) < (eb, mb) {
                return a;
            }
            // (ma * 2^(ea - eb)) mod mb, in units of 2^(eb - 63)
            let mut r = ma % mb;
            let mut d = (ea - eb) as u32;
            while d > 0 && r != 0 {
                let s = d.min(64);
                r = (((r as u128) << s) % mb as u128) as u64;
                d -= s;
            }
            if r == 0 {
                return zero(sa);
            }
            let lz = r.leading_zeros();
            pack(sa, eb - lz as i32, r << lz, false)
        }
    }
}

/// floor(sqrt(n))
fn isqrt(n: u128) -> u64 {
    let mut x = n;
    let mut r: u128 = 0;
    let mut bit: u128 = 1 << 126;
    while bit > x {
        bit >>= 2;
    }
    while bit != 0 {
        if x >= r + bit {
            x -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    r as u64
}

/// the square root (NaN for negative numbers, sqrt(-0) = -0)
pub fn sqrt(a: f64) -> f64 {
    let (s, c) = unpack(a.to_bits());
    match c {
        Class::Nan => NAN,
        Class::Zero => a,
        Class::Inf => {
            if s { NAN } else { a }
        }
        Class::Finite(e, m) => {
            if s {
                return NAN;
            }
            // n * 2^k with k even and n of 127 or 128 bits
            let (n, k) = if e & 1 != 0 { ((m as u128) << 64, e - 127) } else { ((m as u128) << 63, e - 126) };
            let r = isqrt(n);
            pack(false, k / 2 + 63, r, r as u128 * r as u128 != n)
        }
    }
}

/// i as a float, correctly rounded
pub fn from_i64(i: i64) -> f64 {
    if i == 0 {
        return 0.0;
    }
    let u = i.unsigned_abs();
    let lz = u.leading_zeros();
    pack(i < 0, 63 - lz as i32, u << lz, false)
}

/// x truncated towards zero; None for NaN, infinity and values outside i64
pub fn to_i64(x: f64) -> Option<i64> {
    let (s, c) = unpack(x.to_bits());
    match c {
        Class::Nan | Class::Inf => None,
        Class::Zero => Some(0),
        Class::Finite(e, m) => {
            if e < 0 {
                return Some(0);
            }
            if e >= 63 {
                return if s && e == 63 && m == 1 << 63 { Some(i64::MIN) } else { None };
            }
            let v = (m >> (63 - e)) as i64;
            Some(if s { -v } else { v })
        }
    }
}

/// -x (NaN stays the one NaN)
pub fn neg(x: f64) -> f64 {
    if is_nan(x) { NAN } else { f64::from_bits(x.to_bits() ^ SIGN) }
}

/// |x|
pub fn abs(x: f64) -> f64 {
    if is_nan(x) { NAN } else { f64::from_bits(x.to_bits() & !SIGN) }
}

/// x without its fraction (towards zero)
pub fn trunc(x: f64) -> f64 {
    let b = x.to_bits();
    let be = ((b >> 52) & 0x7FF) as i32;
    if be == 0x7FF {
        return if is_nan(x) { NAN } else { x };
    }
    if be < 1023 {
        return f64::from_bits(b & SIGN);
    }
    if be >= 1075 {
        return x;
    }
    f64::from_bits(b & !(FRAC >> (be - 1023)))
}

/// the largest integer <= x
pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if x.to_bits() & SIGN != 0 && !is_nan(x) && t.to_bits() != x.to_bits() { sub(t, 1.0) } else { t }
}

/// the smallest integer >= x
pub fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if x.to_bits() & SIGN == 0 && !is_nan(x) && t.to_bits() != x.to_bits() { add(t, 1.0) } else { t }
}

/// an integer key with the order of the floats (not for NaN; -0 and +0 both give 0)
fn key(x: u64) -> i64 {
    let mag = (x & !SIGN) as i64;
    if x & SIGN != 0 { -mag } else { mag }
}

/// the order of a and b; None when one of them is NaN
pub fn cmp(a: f64, b: f64) -> Option<Ordering> {
    if is_nan(a) || is_nan(b) {
        return None;
    }
    Some(key(a.to_bits()).cmp(&key(b.to_bits())))
}

/// a == b (false for NaN, true for -0 == +0)
pub fn eq(a: f64, b: f64) -> bool {
    cmp(a, b) == Some(Ordering::Equal)
}

/// a < b
pub fn lt(a: f64, b: f64) -> bool {
    cmp(a, b) == Some(Ordering::Less)
}

/// a <= b
pub fn le(a: f64, b: f64) -> bool {
    matches!(cmp(a, b), Some(Ordering::Less | Ordering::Equal))
}

/// the order of the integer i and the float x, exactly; None when x is NaN
pub fn cmp_int(i: i64, x: f64) -> Option<Ordering> {
    if is_nan(x) {
        return None;
    }
    let Some(t) = to_i64(x) else {
        // infinite, or beyond the range of i64
        return Some(if x.to_bits() & SIGN != 0 { Ordering::Greater } else { Ordering::Less });
    };
    match i.cmp(&t) {
        Ordering::Equal => cmp(from_i64(t), x),
        o => Some(o),
    }
}

/// the shortest text that reads back as the same bits: 1.0, 0.1, 0.30000000000000004, 1e21,
/// 1.5e-7, -0.0, inf, -inf, nan
pub fn fmt(x: f64, out: &mut String) {
    if is_nan(x) {
        out.push_str("nan");
    } else {
        // core's shortest round-trip formatting works on the bits with integer arithmetic
        out.push_str(&format!("{:?}", x));
    }
}
