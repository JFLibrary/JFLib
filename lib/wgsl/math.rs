// math -- f64 functions for constant evaluation (core has no libm): exact floor / ceil /
// trunc / round (half to even) / sqrt (correctly rounded) / ldexp / frexp, and the
// transcendental functions with errors of a few ulp (enough: const-expressions of f32 / f16
// are rounded afterwards, AbstractFloat accuracy is "no stricter than WebAssembly / JS")
//
//   floor ceil trunc round sqrt ldexp frexp
//   exp exp2 log log2 pow sin cos tan asin acos atan atan2 sinh cosh tanh asinh acosh atanh
//
// Results outside the domain are NaN, overflow gives +-inf; the caller turns both into
// "cannot be represented" errors (spec 15.7.3).

use crate::lex::{f64_parts, round_bits, Fmt};

pub const PI: f64 = core::f64::consts::PI;
const LN2_HI: f64 = 6.931_471_803_691_238_164_90e-01;
const LN2_LO: f64 = 1.908_214_929_270_587_700_02e-10;
const LN2: f64 = core::f64::consts::LN_2;

pub fn trunc(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if e < 0 {
        return f64::from_bits(bits & (1 << 63));
    }
    let mask = (1u64 << (52 - e)) - 1;
    f64::from_bits(bits & !mask)
}

pub fn floor(x: f64) -> f64 {
    let t = trunc(x);
    if t > x { t - 1.0 } else { t }
}

pub fn ceil(x: f64) -> f64 {
    let t = trunc(x);
    if t < x { t + 1.0 } else { t }
}

/// to nearest, halves to even
pub fn round(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    let f = floor(x);
    let d = x - f;
    let r = if d > 0.5 {
        f + 1.0
    } else if d < 0.5 {
        f
    } else if trunc(f * 0.5) * 2.0 == f {
        f
    } else {
        f + 1.0
    };
    if r == 0.0 { if x < 0.0 { -0.0 } else { 0.0 } } else { r }
}

fn isqrt(n: u128) -> (u128, u128) {
    // bit by bit: r = floor(sqrt(n)), n - r^2
    let mut rem = n;
    let mut r: u128 = 0;
    let mut bit: u128 = 1 << 126;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= r + bit {
            rem -= r + bit;
            r = (r >> 1) + bit;
        } else {
            r >>= 1;
        }
        bit >>= 2;
    }
    (r, rem)
}

/// correctly rounded
pub fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    let (m, e, _) = f64_parts(x);
    let bl = 128 - m.leading_zeros() as i64;
    let mut s = 112 - bl;
    if (e - s) & 1 != 0 {
        s += 1;
    }
    let (r, rem) = isqrt(m << s);
    round_bits(r, (e - s) / 2, rem != 0, Fmt::F64).map(|v| v.0).unwrap_or(f64::INFINITY)
}

fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// x * 2^n, rounded once
pub fn ldexp(x: f64, n: i64) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let (m, e, neg) = f64_parts(x);
    match round_bits(m, e.saturating_add(n.clamp(-5000, 5000)), false, Fmt::F64) {
        Some((v, _)) => {
            if neg {
                -v
            } else {
                v
            }
        }
        None => {
            if neg {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }
        }
    }
}

/// (fraction in [0.5, 1), exponent) with x = fraction * 2^exponent; (0, 0) for 0
pub fn frexp(x: f64) -> (f64, i64) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let (m, e, neg) = f64_parts(x);
    let bl = 128 - m.leading_zeros() as i64;
    let exp = e + bl;
    let fr = (m as f64) * pow2(-(bl as i32));
    (if neg { -fr } else { fr }, exp)
}

/// e^r for |r| <= 0.36
fn exp_small(r: f64) -> f64 {
    let mut s = 1.0;
    let mut t = 1.0;
    for k in 1..=18 {
        t = t * r / k as f64;
        s += t;
        if t.abs() < 1e-18 * s.abs() {
            break;
        }
    }
    s
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.782_712_893_384 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let k = round(x / LN2);
    let r = (x - k * LN2_HI) - k * LN2_LO;
    ldexp(exp_small(r), k as i64)
}

pub fn exp2(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x >= 1024.0 {
        return f64::INFINITY;
    }
    if x < -1080.0 {
        return 0.0;
    }
    let k = round(x);
    let r = x - k;
    ldexp(exp_small(r * LN2), k as i64)
}

/// (k, log(m)) with x = m * 2^k, m in [sqrt(1/2), sqrt(2))
fn log_parts(x: f64) -> (i64, f64) {
    let (fr, mut k) = frexp(x);
    let mut m = fr * 2.0;
    k -= 1;
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        k += 1;
    }
    let f = m - 1.0;
    let s = f / (2.0 + f);
    let s2 = s * s;
    let mut t = 0.0;
    let mut p = 2.0 * s;
    let mut n = 1.0;
    for _ in 0..30 {
        let term = p / n;
        t += term;
        if term.abs() < 1e-19 * t.abs().max(1e-300) {
            break;
        }
        p *= s2;
        n += 2.0;
    }
    (k, t)
}

pub fn log(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (k, l) = log_parts(x);
    k as f64 * LN2_HI + (l + k as f64 * LN2_LO)
}

pub fn log2(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (k, l) = log_parts(x);
    k as f64 + l / LN2
}

pub fn pow(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if y == 0.0 {
        return if x.is_infinite() { f64::NAN } else { 1.0 };
    }
    if x == 1.0 {
        return if y.is_infinite() { f64::NAN } else { 1.0 };
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    // small integer exponents: repeated squaring (exact where the result is)
    if y == trunc(y) && y.abs() <= 64.0 {
        let mut n = y.abs() as u32;
        let mut b = x;
        let mut r = 1.0;
        while n > 0 {
            if n & 1 == 1 {
                r *= b;
            }
            b *= b;
            n >>= 1;
        }
        return if y < 0.0 { 1.0 / r } else { r };
    }
    // x^y = 2^(y log2 x) with log2 x split into k + l / ln 2
    let (k, l) = log_parts(x);
    let a = y * k as f64;
    let ai = round(a);
    let rest = (a - ai) * LN2 + y * l;
    if ai > 2000.0 {
        return f64::INFINITY;
    }
    if ai < -2200.0 {
        return 0.0;
    }
    ldexp(exp(rest), ai as i64)
}

// sin / cos: x = n * pi/2 + r with a three-part pi/2 (Cody-Waite), |r| <= pi/4
const PIO2_1: f64 = 1.570_796_326_734_125_614_17e+00;
const PIO2_2: f64 = 6.077_100_506_303_965_976_6e-11;
const PIO2_3: f64 = 2.022_266_248_711_166_455_8e-21;

fn reduce(x: f64) -> (i64, f64) {
    let n = round(x * (2.0 / PI));
    let r = ((x - n * PIO2_1) - n * PIO2_2) - n * PIO2_3;
    ((n as i64) & 3, r)
}

fn ksin(r: f64) -> f64 {
    let r2 = r * r;
    let mut t = r;
    let mut s = r;
    let mut k = 1.0;
    for _ in 0..12 {
        t *= -r2 / ((k + 1.0) * (k + 2.0));
        k += 2.0;
        s += t;
    }
    s
}

fn kcos(r: f64) -> f64 {
    let r2 = r * r;
    let mut t = 1.0;
    let mut s = 1.0;
    let mut k = 0.0;
    for _ in 0..12 {
        t *= -r2 / ((k + 1.0) * (k + 2.0));
        k += 2.0;
        s += t;
    }
    s
}

pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x.abs() < 1e-8 {
        return x;
    }
    let (n, r) = reduce(x);
    match n {
        0 => ksin(r),
        1 => kcos(r),
        2 => -ksin(r),
        _ => -kcos(r),
    }
}

pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (n, r) = reduce(x);
    match n {
        0 => kcos(r),
        1 => -ksin(r),
        2 => -kcos(r),
        _ => ksin(r),
    }
}

pub fn tan(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (n, r) = reduce(x);
    let (s, c) = (ksin(r), kcos(r));
    if n & 1 == 0 { s / c } else { -c / s }
}

pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.is_infinite() {
        return if x > 0.0 { PI / 2.0 } else { -PI / 2.0 };
    }
    let neg = x < 0.0;
    let mut a = x.abs();
    let mut inv = false;
    if a > 1.0 {
        a = 1.0 / a;
        inv = true;
    }
    // halve the angle twice: atan(a) = 2 atan(a / (1 + sqrt(1 + a^2)))
    let mut mul = 1.0;
    for _ in 0..3 {
        if a > 0.1 {
            a = a / (1.0 + sqrt(1.0 + a * a));
            mul *= 2.0;
        }
    }
    let a2 = a * a;
    let mut t = a;
    let mut s = a;
    let mut k = 1.0;
    for _ in 0..30 {
        t *= -a2;
        k += 2.0;
        let term = t / k;
        s += term;
        if term.abs() < 1e-20 {
            break;
        }
    }
    let mut r = s * mul;
    if inv {
        r = PI / 2.0 - r;
    }
    if neg { -r } else { r }
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 && y == 0.0 {
        return f64::NAN;
    }
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { atan(y / x) + PI } else { atan(y / x) - PI }
    } else if y > 0.0 {
        PI / 2.0
    } else {
        -PI / 2.0
    }
}

pub fn asin(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2(x, sqrt((1.0 - x) * (1.0 + x)))
}

pub fn acos(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    if x == 1.0 {
        return 0.0;
    }
    atan2(sqrt((1.0 - x) * (1.0 + x)), x)
}

pub fn sinh(x: f64) -> f64 {
    if x.abs() < 0.5 {
        let x2 = x * x;
        let mut t = x;
        let mut s = x;
        let mut k = 1.0;
        for _ in 0..12 {
            t *= x2 / ((k + 1.0) * (k + 2.0));
            k += 2.0;
            s += t;
        }
        return s;
    }
    let e = exp(x.abs());
    let r = 0.5 * e - 0.5 / e;
    if x < 0.0 { -r } else { r }
}

pub fn cosh(x: f64) -> f64 {
    let e = exp(x.abs());
    0.5 * e + 0.5 / e
}

pub fn tanh(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x.abs() > 22.0 {
        return if x > 0.0 { 1.0 } else { -1.0 };
    }
    if x.abs() < 0.5 {
        return sinh(x) / cosh(x);
    }
    let e = exp(2.0 * x);
    (e - 1.0) / (e + 1.0)
}

pub fn asinh(x: f64) -> f64 {
    let a = x.abs();
    let r = if a < 0.5 {
        // log(a + sqrt(1 + a^2)) without the cancellation: the series of asinh
        let a2 = a * a;
        let mut t = a;
        let mut s = a;
        let mut k = 1.0;
        for _ in 0..40 {
            t *= -a2 * k * k / ((k + 1.0) * (k + 2.0));
            k += 2.0;
            s += t;
            if t.abs() < 1e-20 {
                break;
            }
        }
        s
    } else if a > 1e8 {
        log(a) + LN2
    } else {
        log(a + sqrt(a * a + 1.0))
    };
    if x < 0.0 { -r } else { r }
}

pub fn acosh(x: f64) -> f64 {
    if x.is_nan() || x < 1.0 {
        return f64::NAN;
    }
    if x > 1e8 {
        return log(x) + LN2;
    }
    log(x + sqrt((x - 1.0) * (x + 1.0)))
}

pub fn atanh(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    if x.abs() == 1.0 {
        return if x > 0.0 { f64::INFINITY } else { f64::NEG_INFINITY };
    }
    if x.abs() < 0.5 {
        let x2 = x * x;
        let mut t = x;
        let mut s = x;
        let mut k = 1.0;
        for _ in 0..60 {
            t *= x2;
            k += 2.0;
            let term = t / k;
            s += term;
            if term.abs() < 1e-20 {
                break;
            }
        }
        return s;
    }
    0.5 * log((1.0 + x) / (1.0 - x))
}
