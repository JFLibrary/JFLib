// math -- the float functions formulas need, without std and without libm
//
// Accurate to a few units in the last place over the ranges drawings use (angles of a few
// turns, lengths up to kilometres): range reduction, then series on a small interval.  The
// engine compares results with the values Visio stored, so plain truncated series that lose
// digits are not good enough; these keep about 15 significant digits.

use core::f64::consts::{FRAC_PI_2, FRAC_PI_4, LN_2, PI};

/// largest integer not above x
pub fn floor(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

/// smallest integer not below x
pub fn ceil(x: f64) -> f64 {
    -floor(-x)
}

/// x without its fraction
pub fn trunc(x: f64) -> f64 {
    if !x.is_finite() || x.abs() >= 4_503_599_627_370_496.0 {
        return x;
    }
    x as i64 as f64
}

/// x rounded half away from zero
pub fn round(x: f64) -> f64 {
    if x < 0.0 { -floor(-x + 0.5) } else { floor(x + 0.5) }
}

/// the square root (NaN below 0)
pub fn sqrt(x: f64) -> f64 {
    if x < 0.0 || x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    // scale into [1, 4) so the first guess is close, then Newton
    let e = ((x.to_bits() >> 52) & 0x7ff) as i64 - 1023;
    let (m, half) = if e.rem_euclid(2) == 0 { (x * pow2(-e), e / 2) } else { (x * pow2(-(e - 1)), (e - 1) / 2) };
    let mut r = 0.5 * (1.0 + m);
    for _ in 0..6 {
        r = 0.5 * (r + m / r);
    }
    // one correction step on the residual
    r = r + (m - r * r) / (2.0 * r);
    r * pow2(half)
}

/// 2^e for integer e (exact while the result is a normal number)
pub fn pow2(e: i64) -> f64 {
    if e > 1023 {
        return f64::INFINITY;
    }
    if e < -1022 {
        return pow2(e + 1022) * pow2(-1022);
    }
    f64::from_bits(((e + 1023) as u64) << 52)
}

// pi/2 in three parts for the reduction (Cody-Waite)
const PIO2_1: f64 = 1.570_796_326_734_125_6;
const PIO2_2: f64 = 6.077_100_506_303_966e-11;
const PIO2_3: f64 = 2.022_266_248_795_950_7e-21;

// x = k * pi/2 + r with |r| <= pi/4 (about)
fn reduce(x: f64) -> (i64, f64) {
    let k = round(x / FRAC_PI_2);
    let r = ((x - k * PIO2_1) - k * PIO2_2) - k * PIO2_3;
    (k as i64, r)
}

fn sin_series(r: f64) -> f64 {
    let r2 = r * r;
    let (mut term, mut sum) = (r, r);
    let mut n = 1.0;
    for _ in 0..12 {
        term *= -r2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

fn cos_series(r: f64) -> f64 {
    let r2 = r * r;
    let (mut term, mut sum) = (1.0, 1.0);
    let mut n = 0.0;
    for _ in 0..12 {
        term *= -r2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

/// sine and cosine
pub fn sincos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let (k, r) = reduce(x);
    let (s, c) = (sin_series(r), cos_series(r));
    match k.rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

/// sine
pub fn sin(x: f64) -> f64 {
    sincos(x).0
}

/// cosine
pub fn cos(x: f64) -> f64 {
    sincos(x).1
}

/// tangent
pub fn tan(x: f64) -> f64 {
    let (s, c) = sincos(x);
    s / c
}

/// arc tangent
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x < 0.0 {
        return -atan(-x);
    }
    if x > 1.0 {
        return FRAC_PI_2 - atan(1.0 / x);
    }
    // atan(x) = pi/4 + atan((x - 1) / (x + 1)) brings x near 1 down to small values
    let (base, y) = if x > 0.4142135623730950 { (FRAC_PI_4, (x - 1.0) / (x + 1.0)) } else { (0.0, x) };
    // halve once more: atan(y) = 2 atan(y / (1 + sqrt(1 + y^2)))
    let z = y / (1.0 + sqrt(1.0 + y * y));
    let z2 = z * z;
    let (mut term, mut sum) = (z, z);
    let mut n = 1.0;
    for _ in 0..20 {
        term *= -z2;
        n += 2.0;
        sum += term / n;
    }
    base + 2.0 * sum
}

/// the angle of (x, y): atan2(y, x) in -pi ..= pi
pub fn atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 {
        return if y > 0.0 {
            FRAC_PI_2
        } else if y < 0.0 {
            -FRAC_PI_2
        } else if x.is_sign_negative() {
            if y.is_sign_negative() { -PI } else { PI }
        } else {
            y
        };
    }
    let a = atan(y / x);
    if x > 0.0 {
        a
    } else if y >= 0.0 {
        a + PI
    } else {
        a - PI
    }
}

/// arc sine (NaN outside -1..1)
pub fn asin(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2(x, sqrt((1.0 - x) * (1.0 + x)))
}

/// arc cosine (NaN outside -1..1)
pub fn acos(x: f64) -> f64 {
    if !(-1.0..=1.0).contains(&x) {
        return f64::NAN;
    }
    atan2(sqrt((1.0 - x) * (1.0 + x)), x)
}

/// e^x
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.8 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let k = round(x / LN_2);
    // ln 2 in two parts
    let r = (x - k * 6.931_471_803_691_238e-1) - k * 1.908_214_929_270_587_7e-10;
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..20 {
        term *= r / n as f64;
        sum += term;
    }
    sum * pow2(k as i64)
}

/// the natural logarithm (NaN below 0, -inf at 0)
pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    // x = m 2^e with m in [sqrt(1/2), sqrt(2))
    let mut e = ((x.to_bits() >> 52) & 0x7ff) as i64 - 1023;
    let mut m = x * pow2(-e);
    if e == -1023 {
        // subnormal
        return ln(x * pow2(60)) - 60.0 * LN_2;
    }
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let (mut term, mut sum) = (s, s);
    let mut n = 1.0;
    for _ in 0..25 {
        term *= s2;
        n += 2.0;
        sum += term / n;
    }
    2.0 * sum + e as f64 * LN_2
}

/// the logarithm to base 10
pub fn log10(x: f64) -> f64 {
    ln(x) / core::f64::consts::LN_10
}

/// x to the power y (integer y by repeated squaring, exactly as Visio's ^2 needs)
pub fn pow(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return 1.0;
    }
    if y == trunc(y) && y.abs() <= 1024.0 {
        let mut n = y.abs() as u32;
        let (mut base, mut acc) = (x, 1.0);
        while n > 0 {
            if n & 1 == 1 {
                acc *= base;
            }
            base *= base;
            n >>= 1;
        }
        return if y < 0.0 { 1.0 / acc } else { acc };
    }
    if x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else { f64::INFINITY };
    }
    exp(y * ln(x))
}

/// hyperbolic sine, cosine, tangent
pub fn sinh(x: f64) -> f64 {
    if x.abs() < 1e-5 {
        return x + x * x * x / 6.0;
    }
    let e = exp(x);
    0.5 * (e - 1.0 / e)
}

/// hyperbolic cosine
pub fn cosh(x: f64) -> f64 {
    let e = exp(x);
    0.5 * (e + 1.0 / e)
}

/// hyperbolic tangent
pub fn tanh(x: f64) -> f64 {
    if x > 20.0 {
        return 1.0;
    }
    if x < -20.0 {
        return -1.0;
    }
    sinh(x) / cosh(x)
}

/// x modulo y with the sign of y (Visio's MODULUS); NaN for y = 0
pub fn modulus(x: f64, y: f64) -> f64 {
    if y == 0.0 {
        return f64::NAN;
    }
    let r = x - y * floor(x / y);
    // floating noise can leave r == y
    if (y > 0.0 && r >= y) || (y < 0.0 && r <= y) { 0.0 } else { r }
}
