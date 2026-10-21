// Portable replacement for the native x87 entry points. Decimal scaling emulates the
// x87 64-bit significand after each multiply/divide. Transcendentals use explicit scalar
// binary64 series; native x87 remains unchanged and is the comparison reference.

use super::fx;

const PI: f64 = core::f64::consts::PI;
const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;
const LN2: f64 = core::f64::consts::LN_2;
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
    1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
];

#[derive(Clone, Copy)]
struct Extended {
    m: u64,
    e: i32,
}

fn round_shift(v: u128, shift: u32) -> u128 {
    if shift == 0 {
        return v;
    }
    if shift > 128 {
        return 0;
    }
    if shift == 128 {
        return (v > 1u128 << 127) as u128;
    }
    let q = v >> shift;
    let remainder = v & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    q + (remainder > half || (remainder == half && q & 1 != 0)) as u128
}

impl Extended {
    fn integer(v: u64) -> Self {
        if v == 0 {
            return Self { m: 0, e: 0 };
        }
        let sh = v.leading_zeros();
        Self {
            m: v << sh,
            e: -(sh as i32),
        }
    }
    fn from_f64(v: f64) -> Self {
        let bits = v.to_bits();
        let exp = ((bits >> 52) & 2047) as i32;
        let m = bits & ((1u64 << 52) - 1);
        if exp == 0 {
            let mut n = Self::integer(m);
            n.e -= 1074;
            n
        } else {
            Self {
                m: ((1u64 << 52) | m) << 11,
                e: exp - 1023 - 63,
            }
        }
    }
    fn mul(self, b: Self) -> Self {
        if self.m == 0 || b.m == 0 {
            return Self::integer(0);
        }
        let product = self.m as u128 * b.m as u128;
        let sh = 64 - product.leading_zeros();
        let mut m = round_shift(product, sh);
        let mut e = self.e + b.e + sh as i32;
        if m > u64::MAX as u128 {
            m >>= 1;
            e += 1;
        }
        Self { m: m as u64, e }
    }
    fn div(self, b: Self) -> Self {
        if self.m == 0 {
            return self;
        }
        let sh = if self.m >= b.m { 63 } else { 64 };
        let numerator = (self.m as u128) << sh;
        let mut m = numerator / b.m as u128;
        let remainder = numerator % b.m as u128;
        if remainder * 2 > b.m as u128 || (remainder * 2 == b.m as u128 && m & 1 != 0) {
            m += 1;
        }
        let mut e = self.e - b.e - sh;
        if m > u64::MAX as u128 {
            m >>= 1;
            e += 1;
        }
        Self { m: m as u64, e }
    }
    fn double(self) -> f64 {
        if self.m == 0 {
            return 0.0;
        }
        let mut exponent = self.e + 63;
        if exponent > 1023 {
            return f64::INFINITY;
        }
        if exponent < -1022 {
            let sh = -(self.e + 1074);
            return f64::from_bits(if sh > 128 {
                0
            } else {
                round_shift(self.m as u128, sh.max(0) as u32) as u64
            });
        }
        let mut m = round_shift(self.m as u128, 11) as u64;
        if m == 1u64 << 53 {
            m >>= 1;
            exponent += 1;
        }
        if exponent > 1023 {
            return f64::INFINITY;
        }
        f64::from_bits(((exponent + 1023) as u64) << 52 | (m & ((1u64 << 52) - 1)))
    }
    fn rounded_integer(self, negative: bool) -> i64 {
        let m = if self.e >= 0 {
            if self.e >= 64 {
                return i64::MIN;
            }
            (self.m as u128) << self.e
        } else {
            round_shift(self.m as u128, self.e.unsigned_abs())
        };
        if negative {
            if m > 1u128 << 63 {
                i64::MIN
            } else {
                (m as i64).wrapping_neg()
            }
        } else if m > i64::MAX as u128 {
            i64::MIN
        } else {
            m as i64
        }
    }
}

fn power10(mut n: u32) -> Extended {
    let mut p = Extended::integer(1);
    while n > 22 {
        p = p.mul(Extended::from_f64(POW10[22]));
        n -= 22;
    }
    p.mul(Extended::from_f64(POW10[n as usize]))
}

/// Decimal scaling with the same 64-bit significand intermediate rounding as x87.
pub fn scale(m: u64, e: i32) -> f64 {
    let signed = m as i64;
    if signed == 0 {
        return 0.0;
    }
    if e > 5000 {
        return if signed < 0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if e < -5000 {
        return if signed < 0 { -0.0 } else { 0.0 };
    }
    let a = Extended::integer(signed.unsigned_abs());
    let p = power10(e.unsigned_abs());
    let out = if e < 0 { a.div(p) } else { a.mul(p) }.double();
    if signed < 0 { -out } else { out }
}
/// Nearest-even integer after decimal scaling, including native indefinite overflow.
pub fn scale_round(v: f64, k: i32) -> i64 {
    if !v.is_finite() || k > 5000 {
        return i64::MIN;
    }
    if k < -5000 || v == 0.0 {
        return 0;
    }
    let a = Extended::from_f64(v.abs());
    let p = power10(k.unsigned_abs());
    if k < 0 { a.div(p) } else { a.mul(p) }.rounded_integer(v.is_sign_negative())
}
/// Nearest integer decimal logarithm; nonpositive inputs produce the SSE indefinite value.
pub fn log10_round(v: f64) -> i32 {
    fx::cvtd(log2(v) * core::f64::consts::LOG10_2)
}

/// Binary logarithm using a normalized mantissa and a convergent atanh series.
pub fn log2(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x < 0.0 {
        return f64::from_bits(0xfff8000000000000);
    }
    if x == f64::INFINITY {
        return x;
    }
    let ext = Extended::from_f64(x);
    let mut m = ext.m as f64 / 9223372036854775808.0;
    let mut e = ext.e + 63;
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let z = (m - 1.0) / (m + 1.0);
    let zz = z * z;
    let mut term = z;
    let mut sum = z;
    let mut i = 3;
    while i <= 39 {
        term *= zz;
        sum += term / i as f64;
        i += 2;
    }
    e as f64 + (2.0 * sum) / LN2
}
/// Platform-independent base-two exponent; finite underflow rounds into subnormals.
pub fn exp2(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x >= 1024.0 {
        return f64::INFINITY;
    }
    if x <= -1075.0 {
        return 0.0;
    }
    let n = fx::cvtd(x);
    let f = (x - n as f64) * LN2;
    let mut sum = 1.0;
    let mut term = 1.0;
    let mut i = 1;
    while i <= 20 {
        term *= f / i as f64;
        sum += term;
        i += 1;
    }
    if n > 1023 {
        (sum * 2.0) * f64::from_bits(2046u64 << 52)
    } else if n < -1022 {
        (sum * f64::from_bits(((n + 1077) as u64) << 52)) * f64::from_bits(969u64 << 52)
    } else {
        sum * f64::from_bits(((n + 1023) as u64) << 52)
    }
}
/// Existing rotation/zoom base-two logarithm API.
pub fn rz_log2(x: f64) -> f64 {
    log2(x)
}
/// Existing rotation/zoom base-two exponent API.
pub fn rz_exp2(x: f64) -> f64 {
    exp2(x)
}
/// Effect power contract: y=0 yields 1, nonpositive bases yield 0, NaNs propagate canonically.
pub fn pow(x: f64, y: f64) -> f64 {
    if y.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return 1.0;
    }
    if x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if y == 1.0 {
        return x;
    }
    exp2(y * log2(x))
}

fn atan_reduced(mut x: f64) -> f64 {
    let inverse = x > 1.0;
    if inverse {
        x = 1.0 / x;
    }
    let transformed = x > 0.41421356237309503;
    if transformed {
        x = (x - 1.0) / (x + 1.0);
    }
    let xx = -x * x;
    let mut term = x;
    let mut sum = x;
    let mut i = 3;
    while i <= 61 {
        term *= xx;
        sum += term / i as f64;
        i += 2;
    }
    if transformed {
        sum += core::f64::consts::FRAC_PI_4;
    }
    if inverse { HALF_PI - sum } else { sum }
}

fn remainder(x: f64, divisor: f64) -> f64 {
    if x.abs() < divisor {
        return x;
    }
    let a = Extended::from_f64(x.abs());
    let b = Extended::from_f64(divisor);
    let mut m = a.m as u128;
    let mut e = a.e;
    while e > b.e {
        if m >= b.m as u128 {
            m -= b.m as u128;
        }
        m <<= 1;
        e -= 1;
    }
    if m >= b.m as u128 {
        m -= b.m as u128;
    }
    let mut result = Extended::integer(m as u64);
    result.e += b.e;
    let v = result.double();
    if x.is_sign_negative() { -v } else { v }
}
/// Quadrant-aware arctangent, including signed zeros and infinite operands.
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return x + y;
    }
    let ax = x.abs();
    let ay = y.abs();
    let mut a = if ay == 0.0 {
        0.0
    } else if ax == 0.0 {
        HALF_PI
    } else if ax.is_infinite() && ay.is_infinite() {
        core::f64::consts::FRAC_PI_4
    } else {
        atan_reduced(ay / ax)
    };
    if x.is_sign_negative() {
        a = PI - a;
    }
    if y.is_sign_negative() { -a } else { a }
}

/// Cosine and sine with shared quadrant reduction; return order matches the x87 API.
pub fn sincos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    if x == 0.0 {
        return (1.0, x);
    }
    let reduced = remainder(x, 2.0 * PI);
    let quadrant = fx::cvtd(reduced / HALF_PI);
    // Split pi/2 minimizes cancellation for the normal UI/effect angle range.
    let r =
        (reduced - quadrant as f64 * 1.5707963267948966) - quadrant as f64 * 6.123233995736766e-17;
    let rr = -r * r;
    let mut s = r;
    let mut st = r;
    let mut c = 1.0;
    let mut ct = 1.0;
    let mut i = 1;
    while i <= 10 {
        st *= rr / ((2 * i) * (2 * i + 1)) as f64;
        ct *= rr / ((2 * i - 1) * (2 * i)) as f64;
        s += st;
        c += ct;
        i += 1;
    }
    match quadrant.rem_euclid(4) {
        0 => (c, s),
        1 => (-s, c),
        2 => (-c, -s),
        _ => (s, -c),
    }
}
/// Scalar cosine using the common angle reduction.
pub fn cos(x: f64) -> f64 {
    sincos(x).0
}
/// Scalar sine using the common angle reduction.
pub fn sin(x: f64) -> f64 {
    sincos(x).1
}
