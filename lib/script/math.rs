// math -- the transcendental functions of the script library in software: exp, log, log2,
// log10, pow, sin, cos and round, bit-identical on every target
//
//   exp(x) log(x) log2(x) log10(x)   ports of fdlibm (as in FreeBSD and musl), error below
//   pow(x, y) sin(x) cos(x)          1 ulp; exact where the result is exact: log2(8) = 3,
//                                    pow(2, 10) = 1024, pow(x, 0.5) = sqrt(x), exp(0) = 1
//   round(x)                         to the nearest integer, halves away from zero
//   scalbn(x, n)                     x * 2^n, rounded once
//
// Every float operation is one of num's correctly rounded integer routines (through the F
// wrapper below, so the ports read like the C originals), so the results depend on nothing
// but the input bits.  sin and cos reduce their argument exactly: integer Payne-Hanek with
// 1280 bits of 2/pi (computed with Python's integers from Machin's formula), so sin(1e300) is
// as good as sin(1).  NaN results are num::NAN; special values follow C99 Annex F (pow(x, 0)
// = 1 even for NaN, pow(-1, inf) = 1, log(0) = -inf, sin(inf) = NaN).

use core::ops::{Add, Div, Mul, Neg, Sub};

use crate::num::{self, NAN};

/// a float whose operators are num's
#[derive(Clone, Copy)]
struct F(f64);

impl Add for F {
    type Output = F;
    fn add(self, b: F) -> F {
        F(num::add(self.0, b.0))
    }
}

impl Sub for F {
    type Output = F;
    fn sub(self, b: F) -> F {
        F(num::sub(self.0, b.0))
    }
}

impl Mul for F {
    type Output = F;
    fn mul(self, b: F) -> F {
        F(num::mul(self.0, b.0))
    }
}

impl Div for F {
    type Output = F;
    fn div(self, b: F) -> F {
        F(num::div(self.0, b.0))
    }
}

impl Neg for F {
    type Output = F;
    fn neg(self) -> F {
        F(num::neg(self.0))
    }
}

impl F {
    fn hi(self) -> u32 {
        (self.0.to_bits() >> 32) as u32
    }
    fn lo(self) -> u32 {
        self.0.to_bits() as u32
    }
    fn with_hi(self, h: u32) -> F {
        F(f64::from_bits((h as u64) << 32 | self.lo() as u64))
    }
    fn with_lo(self, l: u32) -> F {
        F(f64::from_bits((self.hi() as u64) << 32 | l as u64))
    }
    fn lt(self, b: F) -> bool {
        num::lt(self.0, b.0)
    }
    fn le(self, b: F) -> bool {
        num::le(self.0, b.0)
    }
}

fn int(i: i32) -> F {
    F(num::from_i64(i as i64))
}

/// truncated towards zero (for values that fit)
fn to_i32(x: F) -> i32 {
    num::to_i64(x.0).unwrap_or(0) as i32
}

const ONE: F = F(1.0);
const TWO: F = F(2.0);
const HALF: F = F(0.5);

/// x * 2^n, rounded once (musl's scalbn)
pub fn scalbn(x: f64, n: i32) -> f64 {
    let p1023 = f64::from_bits(0x7FE0_0000_0000_0000);
    // 2^-1022 * 2^53: the last step lands below -53 so a subnormal result is rounded once
    let m969 = f64::from_bits(0x0360_0000_0000_0000);
    let (mut y, mut n) = (x, n);
    if n > 1023 {
        y = num::mul(y, p1023);
        n -= 1023;
        if n > 1023 {
            y = num::mul(y, p1023);
            n -= 1023;
            n = n.min(1023);
        }
    } else if n < -1022 {
        y = num::mul(y, m969);
        n += 1022 - 53;
        if n < -1022 {
            y = num::mul(y, m969);
            n += 1022 - 53;
            n = n.max(-1022);
        }
    }
    num::mul(y, f64::from_bits(((0x3FF + n) as u64) << 52))
}

/// to the nearest integer, halves away from zero (C's round); keeps -0 and the sign of
/// results that round to zero
pub fn round(x: f64) -> f64 {
    let t = num::trunc(x);
    if num::is_nan(x) || t.to_bits() == x.to_bits() {
        return t;
    }
    let d = num::abs(num::sub(x, t));
    if num::le(0.5, d) {
        let neg = x.to_bits() >> 63 != 0;
        num::add(t, if neg { -1.0 } else { 1.0 })
    } else {
        t
    }
}

// ---- exp

const LN2HI: F = F(6.93147180369123816490e-01);
const LN2LO: F = F(1.90821492927058770002e-10);
const INVLN2: F = F(1.44269504088896338700e+00);
const P1: F = F(1.66666666666666019037e-01);
const P2: F = F(-2.77777777770155933842e-03);
const P3: F = F(6.61375632143793436117e-05);
const P4: F = F(-1.65339022054652515390e-06);
const P5: F = F(4.13813679705723846039e-08);

/// e^x
pub fn exp(x: f64) -> f64 {
    let x = F(x);
    let hx = x.hi() & 0x7FFF_FFFF;
    let neg = x.hi() >> 31 != 0;
    if hx >= 0x4086_232B {
        // |x| >= 708.39..
        if num::is_nan(x.0) {
            return NAN;
        }
        if F(709.782712893383973096).lt(x) {
            return f64::INFINITY;
        }
        if x.lt(F(-745.13321910194110842)) {
            return 0.0;
        }
    }
    let (hi, lo, r, k);
    if hx > 0x3FD6_2E42 {
        // |x| > ln2 / 2
        k = if hx >= 0x3FF0_A2B2 {
            to_i32(INVLN2 * x + if neg { -HALF } else { HALF })
        } else if neg {
            -1
        } else {
            1
        };
        hi = x - int(k) * LN2HI;
        lo = int(k) * LN2LO;
        r = hi - lo;
    } else if hx > 0x3E30_0000 {
        // |x| > 2^-28
        k = 0;
        hi = x;
        lo = F(0.0);
        r = x;
    } else {
        return (ONE + x).0;
    }
    let xx = r * r;
    let c = r - xx * (P1 + xx * (P2 + xx * (P3 + xx * (P4 + xx * P5))));
    let y = ONE + (r * c / (TWO - c) - lo + hi);
    if k == 0 { y.0 } else { scalbn(y.0, k) }
}

// ---- log, log2, log10

const LG1: F = F(6.666666666666735130e-01);
const LG2: F = F(3.999999999940941908e-01);
const LG3: F = F(2.857142874366239149e-01);
const LG4: F = F(2.222219843214978396e-01);
const LG5: F = F(1.818357216161805012e-01);
const LG6: F = F(1.531383769920937332e-01);
const LG7: F = F(1.479819860511658591e-01);

/// the common part of the logarithms: x = 2^k * (1 + f) with 1 + f in [sqrt(2)/2, sqrt(2)];
/// Err for the special values (the result itself)
fn log_reduce(x: f64) -> Result<(i32, F), f64> {
    let mut u = x.to_bits();
    let mut hx = (u >> 32) as u32;
    let mut k: i32 = 0;
    if hx < 0x0010_0000 || hx >> 31 != 0 {
        if u << 1 == 0 {
            return Err(f64::NEG_INFINITY);
        }
        if hx >> 31 != 0 {
            return Err(NAN);
        }
        // subnormal: scale up by 2^54
        k -= 54;
        u = num::mul(x, 18014398509481984.0).to_bits();
        hx = (u >> 32) as u32;
    } else if hx >= 0x7FF0_0000 {
        return Err(if num::is_nan(x) { NAN } else { x });
    } else if hx == 0x3FF0_0000 && u << 32 == 0 {
        return Err(0.0);
    }
    hx = hx.wrapping_add(0x3FF0_0000 - 0x3FE6_A09E);
    k += (hx >> 20) as i32 - 0x3FF;
    hx = (hx & 0x000F_FFFF) + 0x3FE6_A09E;
    u = (hx as u64) << 32 | (u & 0xFFFF_FFFF);
    Ok((k, F(f64::from_bits(u)) - ONE))
}

/// s, hfsq and R of log(1 + f) = f - hfsq + s * (hfsq + R)
fn log_poly(f: F) -> (F, F, F) {
    let hfsq = HALF * f * f;
    let s = f / (TWO + f);
    let z = s * s;
    let w = z * z;
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    (s, hfsq, t2 + t1)
}

/// the natural logarithm
pub fn log(x: f64) -> f64 {
    let (k, f) = match log_reduce(x) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (s, hfsq, r) = log_poly(f);
    let dk = int(k);
    (s * (hfsq + r) + dk * LN2LO - hfsq + f + dk * LN2HI).0
}

/// hi + lo = log(1 + f) with hi's low word cleared (the extra precision of log2 / log10)
fn log_split(f: F) -> (F, F) {
    let (s, hfsq, r) = log_poly(f);
    let hi = (f - hfsq).with_lo(0);
    let lo = f - hi - hfsq + s * (hfsq + r);
    (hi, lo)
}

const IVLN2HI: F = F(1.44269504072144627571e+00);
const IVLN2LO: F = F(1.67517131648865118353e-10);

/// the logarithm to base 2 (exact for powers of two)
pub fn log2(x: f64) -> f64 {
    let (k, f) = match log_reduce(x) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (hi, lo) = log_split(f);
    let mut val_hi = hi * IVLN2HI;
    let mut val_lo = (lo + hi) * IVLN2LO + lo * IVLN2HI;
    let y = int(k);
    let w = y + val_hi;
    val_lo = val_lo + ((y - w) + val_hi);
    val_hi = w;
    (val_lo + val_hi).0
}

const IVLN10HI: F = F(4.34294481878168880939e-01);
const IVLN10LO: F = F(2.50829467116452752298e-11);
const LOG10_2HI: F = F(3.01029995663611771306e-01);
const LOG10_2LO: F = F(3.69423907715893078616e-13);

/// the logarithm to base 10
pub fn log10(x: f64) -> f64 {
    let (k, f) = match log_reduce(x) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (hi, lo) = log_split(f);
    let mut val_hi = hi * IVLN10HI;
    let dk = int(k);
    let y = dk * LOG10_2HI;
    let mut val_lo = dk * LOG10_2LO + (lo + hi) * IVLN10LO + lo * IVLN10HI;
    let w = y + val_hi;
    val_lo = val_lo + ((y - w) + val_hi);
    val_hi = w;
    (val_lo + val_hi).0
}

// ---- pow

const BP: [F; 2] = [F(1.0), F(1.5)];
const DP_H: [F; 2] = [F(0.0), F(5.84962487220764160156e-01)];
const DP_L: [F; 2] = [F(0.0), F(1.35003920212974897128e-08)];
const TWO53: f64 = 9007199254740992.0;
const HUGE: F = F(1.0e300);
const TINY: F = F(1.0e-300);
const L1: F = F(5.99999999999994648725e-01);
const L2: F = F(4.28571428578550184252e-01);
const L3: F = F(3.33333329818377432918e-01);
const L4: F = F(2.72728123808534006489e-01);
const L5: F = F(2.30660745775561754067e-01);
const L6: F = F(2.06975017800338417784e-01);
const LG2P: F = F(6.93147180559945286227e-01);
const LG2_H: F = F(6.93147182464599609375e-01);
const LG2_L: F = F(-1.90465429995776804525e-09);
const OVT: F = F(8.0085662595372944372e-17);
const CP: F = F(9.61796693925975554329e-01);
const CP_H: F = F(9.61796700954437255859e-01);
const CP_L: F = F(-7.02846165095275826516e-09);
const IVLN2: F = F(1.44269504088896338700e+00);
const IVLN2_H: F = F(1.44269502162933349609e+00);
const IVLN2_L: F = F(1.92596299112661746887e-08);

fn words(x: f64) -> (i32, u32) {
    ((x.to_bits() >> 32) as i32, x.to_bits() as u32)
}

/// x to the power y
pub fn pow(x: f64, y: f64) -> f64 {
    let (hx, lx) = words(x);
    let (hy, ly) = words(y);
    let mut ix = hx & 0x7FFF_FFFF;
    let iy = hy & 0x7FFF_FFFF;
    // x^0 = 1 and 1^y = 1, even for NaN
    if iy as u32 | ly == 0 || (hx == 0x3FF0_0000 && lx == 0) {
        return 1.0;
    }
    if ix > 0x7FF0_0000 || (ix == 0x7FF0_0000 && lx != 0) || iy > 0x7FF0_0000 || (iy == 0x7FF0_0000 && ly != 0) {
        return NAN;
    }
    // for x < 0: 0 when y is no integer, 1 for an odd one, 2 for an even one
    let mut yisint = 0;
    if hx < 0 {
        if iy >= 0x4340_0000 {
            yisint = 2;
        } else if iy >= 0x3FF0_0000 {
            let k = (iy >> 20) - 0x3FF;
            if k > 20 {
                let j = ly >> (52 - k);
                if j << (52 - k) == ly {
                    yisint = 2 - (j & 1) as i32;
                }
            } else if ly == 0 {
                let j = (iy as u32) >> (20 - k);
                if j << (20 - k) == iy as u32 {
                    yisint = 2 - (j & 1) as i32;
                }
            }
        }
    }
    if ly == 0 {
        if iy == 0x7FF0_0000 {
            // y = +-inf
            return if (ix - 0x3FF0_0000) as u32 | lx == 0 {
                1.0
            } else if ix >= 0x3FF0_0000 {
                if hy >= 0 { y } else { 0.0 }
            } else if hy >= 0 {
                0.0
            } else {
                num::neg(y)
            };
        }
        if iy == 0x3FF0_0000 {
            return if hy >= 0 { x } else { num::div(1.0, x) };
        }
        if hy == 0x4000_0000 {
            return num::mul(x, x);
        }
        if hy == 0x3FE0_0000 && hx >= 0 {
            return num::sqrt(x);
        }
    }
    let mut ax = num::abs(x);
    if lx == 0 && (ix == 0x7FF0_0000 || ix == 0 || ix == 0x3FF0_0000) {
        // x is +-0, +-inf, +-1
        let mut z = ax;
        if hy < 0 {
            z = num::div(1.0, z);
        }
        if hx < 0 {
            if (ix - 0x3FF0_0000) | yisint == 0 {
                z = NAN;
            } else if yisint == 1 {
                z = num::neg(z);
            }
        }
        return z;
    }
    let mut s = ONE;
    if hx < 0 {
        if yisint == 0 {
            return NAN;
        }
        if yisint == 1 {
            s = F(-1.0);
        }
    }
    let (t1, t2);
    if iy > 0x41E0_0000 {
        // |y| > 2^31
        if iy > 0x43F0_0000 {
            // |y| > 2^64: over- or underflows
            if ix <= 0x3FEF_FFFF {
                return if hy < 0 { (HUGE * HUGE).0 } else { (TINY * TINY).0 };
            }
            if ix >= 0x3FF0_0000 {
                return if hy > 0 { (HUGE * HUGE).0 } else { (TINY * TINY).0 };
            }
        }
        if ix < 0x3FEF_FFFF {
            return if hy < 0 { (s * HUGE * HUGE).0 } else { (s * TINY * TINY).0 };
        }
        if ix > 0x3FF0_0000 {
            return if hy > 0 { (s * HUGE * HUGE).0 } else { (s * TINY * TINY).0 };
        }
        // |1 - x| <= 2^-20: log(x) by x - x^2/2 + x^3/3 - x^4/4
        let t = F(ax) - ONE;
        let w = (t * t) * (HALF - t * (F(0.3333333333333333333333) - t * F(0.25)));
        let u = IVLN2_H * t;
        let v = t * IVLN2_L - w * IVLN2;
        t1 = (u + v).with_lo(0);
        t2 = v - (t1 - u);
    } else {
        let mut n: i32 = 0;
        if ix < 0x0010_0000 {
            // subnormal x
            ax = num::mul(ax, TWO53);
            n -= 53;
            ix = (ax.to_bits() >> 32) as i32;
        }
        n += (ix >> 20) - 0x3FF;
        let j = ix & 0x000F_FFFF;
        ix = j | 0x3FF0_0000;
        let k: usize;
        if j <= 0x3988E {
            // |x| < sqrt(3/2)
            k = 0;
        } else if j < 0xBB67A {
            // |x| < sqrt(3)
            k = 1;
        } else {
            k = 0;
            n += 1;
            ix -= 0x0010_0000;
        }
        let ax = F(ax).with_hi(ix as u32);
        // ss = s_h + s_l = (x - 1) / (x + 1) or (x - 1.5) / (x + 1.5)
        let u = ax - BP[k];
        let v = ONE / (ax + BP[k]);
        let ss = u * v;
        let s_h = ss.with_lo(0);
        let t_h = F(0.0).with_hi((((ix >> 1) as u32) | 0x2000_0000) + 0x0008_0000 + ((k as u32) << 18));
        let t_l = ax - (t_h - BP[k]);
        let s_l = v * ((u - s_h * t_h) - s_h * t_l);
        // log(ax)
        let s2 = ss * ss;
        let mut r = s2 * s2 * (L1 + s2 * (L2 + s2 * (L3 + s2 * (L4 + s2 * (L5 + s2 * L6)))));
        r = r + s_l * (s_h + ss);
        let s2 = s_h * s_h;
        let t_h = (F(3.0) + s2 + r).with_lo(0);
        let t_l = r - ((t_h - F(3.0)) - s2);
        let u = s_h * t_h;
        let v = s_l * t_h + t_l * ss;
        // 2 / (3 log2) * (ss + ..)
        let p_h = (u + v).with_lo(0);
        let p_l = v - (p_h - u);
        let z_h = CP_H * p_h;
        let z_l = CP_L * p_h + p_l * CP + DP_L[k];
        // log2(ax) = n + dp_h + z_h + z_l
        let t = int(n);
        t1 = (((z_h + z_l) + DP_H[k]) + t).with_lo(0);
        t2 = z_l - (((t1 - t) - DP_H[k]) - z_h);
    }
    // (y1 + y2) * (t1 + t2) with y split in two
    let y = F(y);
    let y1 = y.with_lo(0);
    let p_l = (y - y1) * t1 + y * t2;
    let mut p_h = y1 * t1;
    let z = p_l + p_h;
    let (j, i) = words(z.0);
    if j >= 0x4090_0000 {
        // z >= 1024
        if (j - 0x4090_0000) as u32 | i != 0 || (z - p_h).lt(p_l + OVT) {
            return (s * HUGE * HUGE).0;
        }
    } else if j & 0x7FFF_FFFF >= 0x4090_CC00 {
        // z <= -1075
        if (j as u32).wrapping_sub(0xC090_CC00) | i != 0 || p_l.le(z - p_h) {
            return (s * TINY * TINY).0;
        }
    }
    // 2^(p_h + p_l)
    let i = j & 0x7FFF_FFFF;
    let mut k = (i >> 20) - 0x3FF;
    let mut n: i32 = 0;
    if i > 0x3FE0_0000 {
        // |z| > 0.5: n = [z + 0.5]
        n = j.wrapping_add(0x0010_0000 >> (k + 1));
        k = ((n & 0x7FFF_FFFF) >> 20) - 0x3FF;
        let t = F(0.0).with_hi((n & !(0x000F_FFFF >> k)) as u32);
        n = ((n & 0x000F_FFFF) | 0x0010_0000) >> (20 - k);
        if j < 0 {
            n = -n;
        }
        p_h = p_h - t;
    }
    let t = (p_l + p_h).with_lo(0);
    let u = t * LG2_H;
    let v = (p_l - (t - p_h)) * LG2P + t * LG2_L;
    let z = u + v;
    let w = v - (z - u);
    let t = z * z;
    let t1 = z - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    let r = (z * t1) / (t1 - TWO) - (w + z * w);
    let z = ONE - (r - z);
    let j = (z.hi() as i32).wrapping_add(n << 20);
    let z = if j >> 20 <= 0 { F(scalbn(z.0, n)) } else { z.with_hi(j as u32) };
    (s * z).0
}

// ---- sin, cos

const S1: F = F(-1.66666666666666324348e-01);
const S2: F = F(8.33333333332248946124e-03);
const S3: F = F(-1.98412698298579493134e-04);
const S4: F = F(2.75573137070700676789e-06);
const S5: F = F(-2.50507602534068634195e-08);
const S6: F = F(1.58969099521155010221e-10);

/// sin(x + y) for |x + y| <= pi/4, y the tail of the reduced argument (fdlibm's __sin)
fn ksin(x: F, y: F, tail: bool) -> F {
    let z = x * x;
    let w = z * z;
    let r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
    let v = z * x;
    if tail { x - ((z * (HALF * y - v * r) - y) - v * S1) } else { x + v * (S1 + z * r) }
}

const C1: F = F(4.16666666666666019037e-02);
const C2: F = F(-1.38888888888741095749e-03);
const C3: F = F(2.48015872894767294178e-05);
const C4: F = F(-2.75573143513906633035e-07);
const C5: F = F(2.08757232129817482790e-09);
const C6: F = F(-1.13596475577881948265e-11);

/// cos(x + y) for |x + y| <= pi/4 (fdlibm's __cos)
fn kcos(x: F, y: F) -> F {
    let z = x * x;
    let w = z * z;
    let r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
    let hz = HALF * z;
    let w = ONE - hz;
    w + (((ONE - w) - hz) + (z * r - x * y))
}

/// the bits of 2/pi after the binary point, most significant first (1280 of them)
const TWO_OVER_PI: [u64; 20] = [
    0xA2F9836E4E441529, 0xFC2757D1F534DDC0, 0xDB6295993C439041, 0xFE5163ABDEBBC561,
    0xB7246E3A424DD2E0, 0x06492EEA09D1921C, 0xFE1DEB1CB129A73E, 0xE88235F52EBB4484,
    0xE99C7026B45F7E41, 0x3991D639835339F4, 0x9C845F8BBDF9283B, 0x1FF897FFDE05980F,
    0xEF2F118B5A0A6D1F, 0x6D367ECF27CB09B7, 0x4F463F669E5FEA2D, 0x7527BAC7EBE5F17B,
    0x3D0739F78A5292EA, 0x6BFB5FB11F8D5D08, 0x56033046FC7B6BAB, 0xF0CFBC209AF4361D,
];

/// pi/2 * 2^127, rounded
const PIO2_Q127: u128 = 0xC90F_DAA2_2168_C234_C4C6_628B_80DC_1CD1;

/// 256 bits of 2/pi from bit t on (bit 1 is the first after the point), as a little-endian
/// number
fn window(t: u32) -> [u64; 4] {
    let get = |i: u32| -> u64 {
        let pos = t - 1 + 64 * i;
        let (w, o) = ((pos / 64) as usize, pos % 64);
        let a = TWO_OVER_PI.get(w).copied().unwrap_or(0);
        let b = TWO_OVER_PI.get(w + 1).copied().unwrap_or(0);
        if o == 0 { a } else { (a << o) | (b >> (64 - o)) }
    };
    [get(3), get(2), get(1), get(0)]
}

/// bits lo .. lo + 128 of a little-endian number
fn bits128(p: &[u64; 6], lo: u32) -> u128 {
    let w = (lo / 64) as usize;
    let o = lo % 64;
    let g = |i: usize| -> u128 { p.get(i).copied().unwrap_or(0) as u128 };
    let a = g(w) | (g(w + 1) << 64);
    if o == 0 { a } else { (a >> o) | (g(w + 2) << (128 - o)) }
}

/// the full product of two u128
fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    let (a1, a0) = (a >> 64, a & u64::MAX as u128);
    let (b1, b0) = (b >> 64, b & u64::MAX as u128);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    let lo = (p00 & u64::MAX as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// x = n * pi/2 + y0 + y1 with |y0 + y1| <= pi/4, for finite |x| > pi/4: the exact product
/// of x's 53-bit mantissa with a 256-bit window of 2/pi (the bits above it only add multiples
/// of 4), then the fraction times pi/2 as a double-double
fn rem_pio2(x: f64) -> (i32, F, F) {
    let b = x.to_bits();
    let neg = b >> 63 != 0;
    let m = (b & ((1 << 52) - 1)) | 1 << 52;
    let e = ((b >> 52) & 0x7FF) as i32 - 1075;
    let t = (e - 1).max(1);
    let w = window(t as u32);
    let mut p = [0u64; 6];
    let mut carry: u128 = 0;
    for i in 0..4 {
        let v = m as u128 * w[i] as u128 + carry;
        p[i] = v as u64;
        carry = v >> 64;
    }
    p[4] = carry as u64;
    // |x| * 2/pi = p * 2^(e - t - 255): sh fraction bits
    let sh = (t + 255 - e) as u32;
    let mut q = (bits128(&p, sh) & 3) as i32;
    let f = bits128(&p, sh - 128);
    let (neg_r, mag) = if f >> 127 != 0 {
        q += 1;
        (true, f.wrapping_neg())
    } else {
        (false, f)
    };
    // r = mag * 2^-128 * pi/2
    let (hi, lo) = mul_wide(mag, PIO2_Q127);
    let (v, lz) = if hi != 0 {
        let lz = hi.leading_zeros();
        (if lz == 0 { hi } else { (hi << lz) | (lo >> (128 - lz)) }, lz as i32)
    } else if lo != 0 {
        let lz = lo.leading_zeros();
        (lo << lz, 128 + lz as i32)
    } else {
        (0, 0)
    };
    // the value is v * 2^(-127 - lz), v with bit 127 set
    let (y0, y1) = if v == 0 {
        (F(0.0), F(0.0))
    } else {
        let mut m53 = (v >> 75) as u64;
        let rest = v & ((1u128 << 75) - 1);
        let half = 1u128 << 74;
        let mut tail = rest as i128;
        if rest > half || (rest == half && m53 & 1 == 1) {
            m53 += 1;
            tail -= 1i128 << 75;
        }
        let y0 = scalbn(num::from_i64(m53 as i64), -52 - lz);
        let ta = tail.unsigned_abs();
        let drop = (128 - ta.leading_zeros()).saturating_sub(63);
        let y1 = scalbn(num::from_i64((ta >> drop) as i64), drop as i32 - 127 - lz);
        (F(y0), if tail < 0 { -F(y1) } else { F(y1) })
    };
    let (y0, y1) = if neg_r != neg { (-y0, -y1) } else { (y0, y1) };
    (if neg { -q } else { q }, y0, y1)
}

/// the sine
pub fn sin(x: f64) -> f64 {
    let ix = F(x).hi() & 0x7FFF_FFFF;
    if ix <= 0x3FE9_21FB {
        // |x| ~<= pi/4
        if ix < 0x3E50_0000 {
            // |x| < 2^-26
            return x;
        }
        return ksin(F(x), F(0.0), false).0;
    }
    if ix >= 0x7FF0_0000 {
        return NAN;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => ksin(y0, y1, true).0,
        1 => kcos(y0, y1).0,
        2 => (-ksin(y0, y1, true)).0,
        _ => (-kcos(y0, y1)).0,
    }
}

/// the cosine
pub fn cos(x: f64) -> f64 {
    let ix = F(x).hi() & 0x7FFF_FFFF;
    if ix <= 0x3FE9_21FB {
        if ix < 0x3E46_A09E {
            // |x| < 2^-27 * sqrt(2)
            return 1.0;
        }
        return kcos(F(x), F(0.0)).0;
    }
    if ix >= 0x7FF0_0000 {
        return NAN;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kcos(y0, y1).0,
        1 => (-ksin(y0, y1, true)).0,
        2 => (-kcos(y0, y1)).0,
        _ => ksin(y0, y1, true).0,
    }
}
