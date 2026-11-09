// Portable replacement for the native x87 entry points. Decimal scaling emulates the x87
// 64-bit significand after each multiply/divide. The transcendentals carry double-doubles
// (below) and stay within about half an ulp; native x87 is the comparison reference.

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
    fx::cvtd(log2_series(v) * core::f64::consts::LOG10_2)
}

/// Binary logarithm by a convergent atanh series: log10_round's, the same bits as x87 there.
fn log2_series(x: f64) -> f64 {
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
/// Binary logarithm within about half an ulp (a table and a series in double-doubles).
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
    log2_dd(x).0
}
/// Platform-independent base-two exponent within about half an ulp (double-doubles, rounded
/// once, subnormal results too); finite underflow rounds into subnormals, 1024 on is inf.
pub fn exp2(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    exp2_dd(x, 0.0)
}
// log10_round keeps the series of log2_series: its integers must match the x87 unit's
// fldlg2 / fyl2x / fistp bit for bit (portabletest decimal_checks, math_diff.py), and the
// more exact log2 could change the rare roundings near half integers away from x87's.
// rz_log2 / rz_exp2 are log2 / exp2 here; their x87 originals (fyl2x alone, f2xm1 + fscale)
// are not, so ARM64 is nearer the true value there than x86-64.
/// Existing rotation/zoom base-two logarithm API.
pub fn rz_log2(x: f64) -> f64 {
    log2(x)
}
/// Existing rotation/zoom base-two exponent API.
pub fn rz_exp2(x: f64) -> f64 {
    exp2(x)
}

// exp2, log2, pow, sin, cos and atan2 carry their intermediate results as double-doubles, so
// they stay within about half an ulp of the true value; sin and cos reduce any finite angle
// exactly enough (Cody-Waite with pi/2 in four parts below 2^19, Payne-Hanek with 1216 bits of
// 2/pi above). Only IEEE double operations and integer arithmetic: the same bits everywhere.
//
// Double-double arithmetic: a value is hi + lo with |lo| at most about half an ulp of hi.
// Products split their factors into 26 + 27 bits (Veltkamp), so no step needs an FMA.

/// a + b = s + e exactly
#[inline(always)]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// a + b = s + e exactly when |a| >= |b|
#[inline(always)]
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    (s, b - (s - a))
}

/// a = hi + lo, hi with 26 significant bits, lo with 27 (|a| < 2^996)
#[inline(always)]
fn split(a: f64) -> (f64, f64) {
    let t = 134217729.0 * a;
    let hi = t - (t - a);
    (hi, a - hi)
}

/// a * b = p + e exactly (barring underflow)
#[inline(always)]
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    let p = a * b;
    let (ah, al) = split(a);
    let (bh, bl) = split(b);
    (p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

/// (ah + al)(bh + bl) without the product of the low parts
#[inline(always)]
fn dd_mul(ah: f64, al: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (p, e) = two_prod(ah, bh);
    fast_two_sum(p, e + (ah * bl + al * bh))
}

/// (ah + al)^2 without al^2
#[inline(always)]
fn dd_sqr(ah: f64, al: f64) -> (f64, f64) {
    let p = ah * ah;
    let (h, l) = split(ah);
    let e = ((h * h - p) + 2.0 * h * l) + l * l;
    fast_two_sum(p, e + 2.0 * ah * al)
}

/// (ah + al) + (bh + bl)
#[inline(always)]
fn dd_add(ah: f64, al: f64, bh: f64, bl: f64) -> (f64, f64) {
    let (s, e) = two_sum(ah, bh);
    fast_two_sum(s, e + (al + bl))
}

/// 1.5 * 2^52: (v + ROUND) - ROUND rounds |v| < 2^51 to an integer (ties to even)
const ROUND: f64 = 6755399441055744.0;

/// 2^k as a double, -1022 <= k <= 1023
#[inline(always)]
fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

// pow's constants (hi + lo of the true values; each lo rounds the rest)
const LN2_H: f64 = 0.6931471805599453;
const LN2_L: f64 = 2.3190468138462996e-17;
const C_H: f64 = 2.8853900817779268; // 2 / ln 2
const C_L: f64 = 4.0710547481862066e-17;
/// 2 / ((2k + 1) ln 2) for k = 1 ..= 5: log2's series in z^2 after its first term
const LOG_P: [f64; 5] = [0.9617966939259756, 0.5770780163555853, 0.4121985831111324, 0.3205988979753252, 0.2623081892525388];
/// 1 / k! for k = 2 ..= 8: exp(g) = 1 + g + g^2 (1/2 + g/6 + ...)
const EXP_P: [f64; 7] = [
    0.5, 0.16666666666666666, 0.041666666666666664, 0.008333333333333333, 0.001388888888888889,
    0.0001984126984126984, 2.48015873015873e-05,
];
/// 2^(j/32) for j = 0 ..= 31 as hi + lo
const EXP2_T: [(f64, f64); 32] = [
    (1.0, 0.0),
    (1.0218971486541166, 5.109225028973444e-17),
    (1.0442737824274138, 8.551889705537965e-17),
    (1.0671404006768237, -7.899853966841582e-17),
    (1.0905077326652577, -3.046782079812471e-17),
    (1.1143867425958924, 1.0410278456845571e-16),
    (1.1387886347566916, 8.912812676025408e-17),
    (1.1637248587775775, 3.8292048369240935e-17),
    (1.189207115002721, 3.982015231465646e-17),
    (1.215247359980469, -7.712630692681488e-17),
    (1.241857812073484, 4.658027591836937e-17),
    (1.2690509571917332, 2.667932131342186e-18),
    (1.2968395546510096, 2.5382502794888315e-17),
    (1.3252366431597413, -2.8587312100388614e-17),
    (1.3542555469368927, 7.70094837980299e-17),
    (1.383909881963832, -6.770511658794786e-17),
    (1.4142135623730951, -9.667293313452913e-17),
    (1.4451808069770467, -3.0237581349939873e-17),
    (1.4768261459394993, -3.483994556892796e-17),
    (1.5091644275934228, -1.016455327754295e-16),
    (1.5422108254079407, 7.949834809697621e-17),
    (1.5759808451078865, -1.0136916471278304e-17),
    (1.6104903319492543, 2.4707192569797888e-17),
    (1.645755478153965, -1.0125679913674773e-16),
    (1.681792830507429, 8.199010020581497e-17),
    (1.718619298122478, -1.851380418263111e-17),
    (1.7562521603732995, 2.960140695448873e-17),
    (1.7947090750031072, 1.8227458427912087e-17),
    (1.8340080864093424, 3.283107224245627e-17),
    (1.8741676341103, -6.122763413004143e-17),
    (1.9152065613971474, -1.0619946056195963e-16),
    (1.9571441241754002, 8.960767791036668e-17),
];

/// log2(1 + j/64) for j = -19 ..= 27 as hi + lo
const LOG2_T: [(f64, f64); 47] = [
    (-0.5081469036703253, 4.469046911206334e-17),
    (-0.4764380439429871, -1.7126428340435497e-17),
    (-0.4454111483223626, -1.2269989151629687e-17),
    (-0.4150374992788438, -5.224490061390109e-18),
    (-0.38529015588479176, -2.2208024293925304e-17),
    (-0.3561438102252753, -7.434039928285364e-19),
    (-0.3275746580285044, -2.6214744450027748e-17),
    (-0.2995602818589078, -2.2038346320583612e-17),
    (-0.2720795454368008, -2.476475356878588e-17),
    (-0.24511249783653147, 1.2082105431458586e-17),
    (-0.2186402864753404, -7.522378350087652e-19),
    (-0.19264507794239588, -1.1104012146962652e-17),
    (-0.16710998583525832, -1.358271707229876e-18),
    (-0.14201900487242788, 4.898294009682521e-18),
    (-0.11735695063815874, -5.45905252946375e-18),
    (-0.09310940439148147, -5.596192057804377e-18),
    (-0.06926266243711372, -6.924843096294881e-19),
    (-0.04580368961312479, -2.5450972276520026e-18),
    (-0.02272007650008353, -7.363105580211854e-19),
    (0.0, 0.0),
    (0.02236781302845451, -1.593366605276194e-18),
    (0.044394119358453436, 1.3338680039226223e-18),
    (0.06608919045777244, -4.130247852756734e-18),
    (0.0874628412503394, 6.765321226991275e-18),
    (0.10852445677816905, 5.4046572138033075e-18),
    (0.12928301694496647, -1.147571414337692e-17),
    (0.14974711950468206, 3.3957331682262494e-18),
    (0.16992500144231237, -1.0448980122780218e-17),
    (0.18982455888001723, -2.362617117852667e-19),
    (0.20945336562894978, -1.747801539116594e-18),
    (0.22881869049588088, -5.967894054218645e-18),
    (0.2479275134435855, 3.8662183541602335e-18),
    (0.2667865406949014, -1.148454798555715e-17),
    (0.28540221886224837, -2.726283638197372e-17),
    (0.30378074817710293, -8.333787019748188e-18),
    (0.32192809488736235, -3.717019964142682e-19),
    (0.33985000288462475, -2.0897960245560436e-17),
    (0.3575520046180837, 1.8984820907705057e-17),
    (0.37503943134692475, 1.099000777384843e-17),
    (0.3923174227787603, -1.6328502208352762e-17),
    (0.4093909361377018, -2.1361956385051908e-17),
    (0.42626475470209796, -1.9932012137193316e-17),
    (0.4429434958487283, 2.7429379563921325e-17),
    (0.45943161863729726, -3.8053583859449705e-19),
    (0.47573343096639775, 2.6712179058256416e-18),
    (0.4918530963296747, -1.0820682119194486e-17),
    (0.5077946401986962, 2.2368792763711565e-17),
];

/// log2 x as hi + lo for finite x > 0, relative error below 2^-66: x = 2^e m with m in
/// (sqrt 1/2, sqrt 2], m = c (1 + z) / (1 - z) with c = 1 + j/64 the nearest, |z| < 0.0056,
/// log2 m = log2 c + (2 / ln 2) atanh z, the series' first term as a double-double
fn log2_dd(x: f64) -> (f64, f64) {
    let mut bits = x.to_bits();
    let mut e = (bits >> 52) as i32 - 1023;
    if bits >> 52 == 0 {
        bits = (x * pow2(64)).to_bits(); // subnormal: normalize first
        e = (bits >> 52) as i32 - 1023 - 64;
    }
    let mut m = f64::from_bits(bits & 0x000F_FFFF_FFFF_FFFF | 0x3FF0_0000_0000_0000);
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let jf = ((m - 1.0) * 64.0 + ROUND) - ROUND;
    let c = 1.0 + jf * (1.0 / 64.0);
    let f = m - c; // exact
    let (dh, dl) = two_sum(m, c);
    let zh = f / dh;
    let (p, pe) = two_prod(zh, dh);
    let zl = (((f - p) - pe) - zh * dl) / dh;
    let u = zh * zh;
    let mut q = LOG_P[4];
    for &k in LOG_P[..4].iter().rev() {
        q = k + u * q;
    }
    let (sh, sl) = dd_mul(C_H, C_L, zh, zl);
    let (th, tl) = LOG2_T[(jf as i32 + 19) as usize];
    let (h, l) = two_sum(th, sh);
    let l = l + (tl + (sl + zh * u * q));
    let (h, l2) = two_sum(e as f64, h);
    fast_two_sum(h, l2 + l)
}

/// 2^(wh + wl) for |wl| <= ulp(wh), rounded once at the end (subnormal results too):
/// 2^w = 2^n 2^(j/32) exp(g) with g = (w - n - j/32) ln 2, |g| <= ln 2 / 64
fn exp2_dd(wh: f64, wl: f64) -> f64 {
    if wh >= 1024.0 {
        return f64::INFINITY;
    }
    if wh <= -1080.0 {
        return 0.0;
    }
    let kf = (wh * 32.0 + ROUND) - ROUND;
    let k = kf as i32;
    let (fh, fl) = two_sum(wh - kf * (1.0 / 32.0), wl); // the subtraction is exact
    let (gh, gl) = dd_mul(fh, fl, LN2_H, LN2_L);
    let mut q = EXP_P[6];
    for &c in EXP_P[..6].iter().rev() {
        q = c + gh * q;
    }
    let (eh, el) = fast_two_sum(1.0, gh);
    let el = el + (gl + gh * gh * q);
    let (th, tl) = EXP2_T[(k & 31) as usize];
    let (rh, rl) = two_prod(th, eh);
    let rl = rl + (th * el + tl * eh); // rh + rl in [0.98, 2)
    let n = k >> 5;
    if n < -1022 {
        // subnormal: C + r with C = 2^(-1022 - n) rounds r to the multiples of 2^(-1074 - n)
        let c = pow2(-1022 - n);
        let (s, e) = fast_two_sum(c, rh);
        return ((s + (e + rl)) - c) * pow2(n + 64) * pow2(-64); // both products exact
    }
    let r = rh + rl;
    if n > 1023 { r * 2.0 * pow2(n - 1) } else { r * pow2(n) }
}

/// Effect power contract: y=0 yields 1, nonpositive bases yield 0, NaNs propagate canonically.
/// Else 2^(y log2 x) with log2 x and its product with y as double-doubles, rounded once:
/// within 0.501 ulp in the tests (|y log2 x| up to 1074); x = +inf gives inf (y > 0) or 0.
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
    if x == f64::INFINITY {
        return if y > 0.0 { x } else { 0.0 };
    }
    let (lh, ll) = log2_dd(x);
    let w = y * lh;
    if w.is_nan() {
        return f64::NAN; // y infinite, x == 1
    }
    if !(w.abs() < 1100.0) {
        return if w > 0.0 { f64::INFINITY } else { 0.0 };
    }
    let (ph, pl) = two_prod(y, lh);
    let (wh, wl) = fast_two_sum(ph, pl + y * ll);
    exp2_dd(wh, wl)
}

// atan2: atan(n / d) for 0 < n <= d from a table, atan t = atan c + atan((t - c) / (1 + t c))
// with c = j/32 the nearest, |(t - c) / (1 + t c)| <= 1/64; t and the quotient as double-doubles
const PI_H: f64 = PI;
const PI_L: f64 = 1.2246467991473532e-16;
/// atan(j/32) for j = 0 ..= 32 as hi + lo
const ATAN_T: [(f64, f64); 33] = [
    (0.0, 0.0),
    (0.031239833430268277, -1.188442711587748e-18),
    (0.06241880999595735, -1.5490756308295046e-18),
    (0.09347678115858947, -6.2844725995420954e-18),
    (0.12435499454676144, -3.1253241424539383e-18),
    (0.15499674192394097, 9.585415594114324e-18),
    (0.18534794999569476, 4.180692268843079e-18),
    (0.21535769969773805, 4.738160130078733e-19),
    (0.24497866312686414, 1.0698755618734451e-17),
    (0.2741674511196588, 8.261353575163773e-18),
    (0.3028848683749714, -1.1010827903001369e-17),
    (0.3310960767041321, -7.952610375793799e-18),
    (0.35877067027057225, -2.4623815582638635e-17),
    (0.38588266939807375, 2.378822732491941e-17),
    (0.4124104415973873, -1.587652227770689e-17),
    (0.43833655985795783, -2.494277030626541e-17),
    (0.4636476090008061, 2.2698777452961687e-17),
    (0.48833395105640554, -1.1373236189329585e-17),
    (0.5123894603107377, -2.5462781472855804e-17),
    (0.5358112379604637, -4.0637956834825575e-18),
    (0.5585993153435624, -5.4556305485916264e-18),
    (0.5807563535676704, -1.441464378193067e-17),
    (0.6022873461349642, 2.950430737228402e-17),
    (0.6231993299340659, 2.672403885140095e-17),
    (0.6435011087932844, 1.5834785051444286e-17),
    (0.6632029927060933, -3.076054864429649e-17),
    (0.6823165548747481, 6.943223671560008e-18),
    (0.7008544078844502, -1.987626234335816e-17),
    (0.7188299996216245, -2.1478388444456983e-17),
    (0.7362574289814281, 3.473937648299457e-17),
    (0.7531512809621944, -2.4256934659182068e-17),
    (0.7695264804056583, -3.704991905602721e-17),
    (0.7853981633974483, 3.061616997868383e-17),
];
/// (-1)^k / (2k + 1) for k = 1 ..= 5: atan s = s + s^3 (-1/3 + s^2/5 - ...)
const ATAN_P: [f64; 5] = [-0.3333333333333333, 0.2, -0.14285714285714285, 0.1111111111111111, -0.09090909090909091];

/// atan(n / d) as hi + lo for 0 < n <= d (d may be infinite), relative error below 2^-62
fn atan_dd(mut n: f64, mut d: f64) -> (f64, f64) {
    let th = n / d;
    if th < 8.673617379884035e-19 {
        return (th, 0.0); // t < 2^-60: atan t = t (1 - t^2/3 + ...) rounds like t
    }
    if d > pow2(400) {
        (n, d) = (n * pow2(-600), d * pow2(-600)); // exact: n >= 2^-60 d, two_prod fits
    } else if d < pow2(-400) {
        (n, d) = (n * pow2(600), d * pow2(600));
    }
    let (p, pe) = two_prod(th, d);
    let tl = ((n - p) - pe) / d; // t = th + tl
    let jf = (th * 32.0 + ROUND) - ROUND;
    let c = jf * (1.0 / 32.0);
    let (nh, nl) = two_sum(th - c, tl); // th - c is exact (Sterbenz, or c = 0)
    let (q, qe) = two_prod(th, c);
    let (bh, bl) = fast_two_sum(1.0, q);
    let bl = bl + (qe + tl * c); // 1 + t c = bh + bl
    let sh = nh / bh;
    let (p, pe) = two_prod(sh, bh);
    let sl = ((((nh - p) - pe) + nl) - sh * bl) / bh; // s = (t - c) / (1 + t c) = sh + sl
    let u = sh * sh;
    let mut k = ATAN_P[4];
    for &a in ATAN_P[..4].iter().rev() {
        k = a + u * k;
    }
    let (ah, al) = ATAN_T[jf as usize];
    let (h, l) = two_sum(ah, sh);
    (h, l + (al + (sl + sh * u * k)))
}

/// Quadrant-aware arctangent, including signed zeros and infinite operands: c + atan(n / d)
/// or c - atan(n / d) with c = 0, pi/2 or pi as double-doubles, rounded once (within about
/// half an ulp).
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return x + y;
    }
    let (mut ax, mut ay) = (x.abs(), y.abs());
    if ax.is_infinite() && ay.is_infinite() {
        (ax, ay) = (1.0, 1.0);
    }
    let swap = ay > ax;
    // x >= 0: atan(ay/ax) or pi/2 - atan(ax/ay); x < 0: pi - atan(ay/ax) or pi/2 + atan(ax/ay)
    let (ch, cl, minus) = match (x.is_sign_negative(), swap) {
        (false, false) => (0.0, 0.0, 0),
        (false, true) => (PIO2_H, PIO2_L, 1u64 << 63),
        (true, false) => (PI_H, PI_L, 1 << 63),
        (true, true) => (PIO2_H, PIO2_L, 0),
    };
    let (ah, al) = if ay == 0.0 {
        (0.0, 0.0)
    } else if swap {
        atan_dd(ax, ay)
    } else {
        atan_dd(ay, ax)
    };
    let (h, l) = two_sum(ch, f64::from_bits(ah.to_bits() ^ minus));
    let a = h + (l + (cl + f64::from_bits(al.to_bits() ^ minus)));
    f64::from_bits(a.to_bits() ^ y.to_bits() & 1 << 63)
}

// sin and cos: |x| = q pi/2 + r with |r| <= pi/4, r as a double-double, then Taylor series
// in r whose leading terms are double-doubles too
const PIO2_H: f64 = HALF_PI; // pi / 2
const PIO2_L: f64 = 6.123233995736766e-17;
const INV_PIO2: f64 = 0.6366197723675814; // 2 / pi
const CW_P1: f64 = 1.5707963267341256; // pi / 2 = P1 + P2 + P3 + P4
const CW_P2: f64 = 6.077100506303966e-11;
const CW_P3: f64 = 2.0222662487959506e-21;
const CW_P4: f64 = 1.0085854035872483e-37;
const S3_H: f64 = -0.16666666666666666; // -1/3!
const S3_L: f64 = -9.25185853854297e-18;
/// (-1)^k / (2k + 1)! for k = 2 ..= 10
const SIN_P: [f64; 9] = [
    0.008333333333333333, -0.0001984126984126984, 2.7557319223985893e-06, -2.505210838544172e-08,
    1.6059043836821613e-10, -7.647163731819816e-13, 2.8114572543455206e-15, -8.22063524662433e-18,
    1.9572941063391263e-20,
];
const C4_H: f64 = 0.041666666666666664; // 1/4!
const C4_L: f64 = 2.3129646346357427e-18;
/// (-1)^k / (2k)! for k = 3 ..= 10
const COS_P: [f64; 8] = [
    -0.001388888888888889, 2.48015873015873e-05, -2.755731922398589e-07, 2.08767569878681e-09,
    -1.1470745597729725e-11, 4.779477332387385e-14, -1.5619206968586225e-16, 4.110317623312165e-19,
];
/// the bits of 2/pi after the binary point (b_1 = the top bit of word 1), word 0 the zeros
/// in front: enough for the largest double plus 190 bits
const TWO_OVER_PI: [u64; 20] = [
    0x0000000000000000, 0xA2F9836E4E441529, 0xFC2757D1F534DDC0, 0xDB6295993C439041,
    0xFE5163ABDEBBC561, 0xB7246E3A424DD2E0, 0x06492EEA09D1921C, 0xFE1DEB1CB129A73E,
    0xE88235F52EBB4484, 0xE99C7026B45F7E41, 0x3991D639835339F4, 0x9C845F8BBDF9283B,
    0x1FF897FFDE05980F, 0xEF2F118B5A0A6D1F, 0x6D367ECF27CB09B7, 0x4F463F669E5FEA2D,
    0x7527BAC7EBE5F17B, 0x3D0739F78A5292EA, 0x6BFB5FB11F8D5D08, 0x56033046FC7B6BAB,
];

/// Cody-Waite: pi/4 < ax < 2^19 as q pi/2 + (rh + rl) with |r| <= pi/4 (q modulo 4).
/// pi/2 = P1 + P2 + P3 + P4 to 2^-178: q P1 and q P2 are exact (31 and 32 bits), ax - q P1
/// too (Sterbenz), q P3 by two_prod; the only rounding falls on the low part, below 2^-150
/// even where the high parts cancel.
fn reduce_medium(ax: f64) -> (u32, f64, f64) {
    let kf = (ax * INV_PIO2 + ROUND) - ROUND;
    let t = ax - kf * CW_P1;
    let (s, e) = two_sum(t, -(kf * CW_P2));
    let (ph, pl) = two_prod(kf, CW_P3);
    let (u, v) = two_sum(s, -ph);
    let (rh, rl) = fast_two_sum(u, ((v + e) - pl) - kf * CW_P4);
    (kf as u32 & 3, rh, rl)
}

/// Payne-Hanek: finite ax >= pi/4 as q pi/2 + (rh + rl) with |r| <= pi/4 (q modulo 4).
/// ax = m 2^e; 2^e m 2/pi modulo 4 needs the bits b_i of 2/pi from i = e - 1 on (the earlier
/// ones give multiples of 4): 192 of them times m, integer arithmetic, exact to 2^-137.
fn reduce(ax: f64) -> (u32, f64, f64) {
    let bits = ax.to_bits();
    let m = bits & 0x000F_FFFF_FFFF_FFFF | 1 << 52;
    let p = ((bits >> 52) as i32 - 1075 + 62) as usize; // b_(e-1) in TWO_OVER_PI, >= 9
    let (i, o) = (p / 64, p % 64);
    let word = |k: usize| {
        if o == 0 { TWO_OVER_PI[i + k] } else { TWO_OVER_PI[i + k] << o | TWO_OVER_PI[i + k + 1] >> (64 - o) }
    };
    // m * (w0 w1 w2) = p3 p2 p1 p0, the binary point 190 bits up
    let a = m as u128 * word(2) as u128;
    let p0 = a as u64;
    let a = m as u128 * word(1) as u128 + (a >> 64);
    let p1 = a as u64;
    let a = m as u128 * word(0) as u128 + (a >> 64);
    let (p2, p3) = (a as u64, (a >> 64) as u64);
    // the fraction in [0, 1) as hi (64 bits) and lo (128 bits); from 1/2 on (neg all ones)
    // q + 1 and the magnitude of f - 1
    let hi = p2 << 2 | p1 >> 62;
    let lo = ((p1 << 2 | p0 >> 62) as u128) << 64 | (p0 << 2) as u128;
    let neg = (hi >> 63).wrapping_neg();
    let q = (p3 << 2 | p2 >> 62).wrapping_sub(neg) as u32 & 3;
    let lo = (lo ^ ((neg as u128) << 64 | neg as u128)).wrapping_add((neg & 1) as u128);
    let hi = (hi ^ neg).wrapping_add((lo == 0) as u64 & neg);
    if hi == 0 && lo == 0 {
        return (q, 0.0, 0.0);
    }
    // the top 128 bits from the first one on, then 53 + 53 of them as f = fh + fl
    let (top, lz) = if hi != 0 {
        let lz = hi.leading_zeros();
        ((hi as u128) << (64 + lz) | lo >> (64 - lz), lz as i32)
    } else {
        let lz = lo.leading_zeros();
        (lo << lz, 64 + lz as i32)
    };
    let fh = (top >> 75) as i64 as f64 * pow2(-53 - lz);
    let fl = ((top >> 22) as i64 & ((1 << 53) - 1)) as f64 * pow2(-106 - lz);
    let (rh, rl) = dd_mul(fh, fl, PIO2_H, PIO2_L);
    let sign = neg & 1 << 63;
    (q, f64::from_bits(rh.to_bits() ^ sign), f64::from_bits(rl.to_bits() ^ sign))
}

/// |x| = q pi/2 + (rh + rl) and r^2 = ah + al; None for infinities and NaN
#[inline]
fn quadrant(x: f64) -> Option<(u32, f64, f64, f64, f64)> {
    let ax = x.abs();
    if !ax.is_finite() {
        return None;
    }
    let (q, rh, rl) = if ax <= core::f64::consts::FRAC_PI_4 {
        (0, ax, 0.0)
    } else if ax < 524288.0 {
        reduce_medium(ax)
    } else {
        reduce(ax)
    };
    let (ah, al) = dd_sqr(rh, rl);
    Some((q, rh, rl, ah, al))
}

/// sin r for |r| <= pi/4, r^2 = ah + al: r + r^3 (-1/6 + r^2/120 - ...)
fn sin_kernel(rh: f64, rl: f64, ah: f64, al: f64) -> f64 {
    let mut t = SIN_P[8];
    for &c in SIN_P[..8].iter().rev() {
        t = c + ah * t;
    }
    let (sh, sl) = two_sum(S3_H, ah * t);
    let (bh, bl) = dd_mul(ah, al, rh, rl);
    let (uh, ul) = dd_mul(bh, bl, sh, sl + S3_L);
    let (vh, vl) = two_sum(rh, uh);
    vh + (vl + (rl + ul))
}

/// cos r for |r| <= pi/4, r^2 = ah + al: 1 - r^2/2 + r^4 (1/24 - r^2/720 + ...)
fn cos_kernel(ah: f64, al: f64) -> f64 {
    let mut t = COS_P[7];
    for &c in COS_P[..7].iter().rev() {
        t = c + ah * t;
    }
    let (ch, cl) = two_sum(C4_H, ah * t);
    let (bh, bl) = dd_sqr(ah, al);
    let (vh, vl) = dd_mul(bh, bl, ch, cl + C4_L);
    let (wh, wl) = dd_add(-0.5 * ah, -0.5 * al, vh, vl);
    let (oh, ol) = fast_two_sum(1.0, wh);
    oh + (ol + wl)
}

/// the sign bits of cos and sin in quadrant q (sin's still without the sign of x)
#[inline(always)]
fn signs(q: u32) -> (u64, u64) {
    ((((q + 1) & 2) as u64) << 62, ((q & 2) as u64) << 62)
}

/// Cosine and sine with shared quadrant reduction; return order matches the x87 API.
pub fn sincos(x: f64) -> (f64, f64) {
    let Some((q, rh, rl, ah, al)) = quadrant(x) else {
        return (f64::NAN, f64::NAN);
    };
    let (s, c) = (sin_kernel(rh, rl, ah, al), cos_kernel(ah, al));
    let (c, s) = if q & 1 == 0 { (c, s) } else { (s, c) };
    let (cs, ss) = signs(q);
    (f64::from_bits(c.to_bits() ^ cs), f64::from_bits(s.to_bits() ^ ss ^ x.to_bits() & 1 << 63))
}
/// Scalar cosine using the common angle reduction (the same bits as sincos).
pub fn cos(x: f64) -> f64 {
    let Some((q, rh, rl, ah, al)) = quadrant(x) else {
        return f64::NAN;
    };
    let c = if q & 1 == 0 { cos_kernel(ah, al) } else { sin_kernel(rh, rl, ah, al) };
    f64::from_bits(c.to_bits() ^ signs(q).0)
}
/// Scalar sine using the common angle reduction (the same bits as sincos).
pub fn sin(x: f64) -> f64 {
    let Some((q, rh, rl, ah, al)) = quadrant(x) else {
        return f64::NAN;
    };
    let s = if q & 1 == 0 { sin_kernel(rh, rl, ah, al) } else { cos_kernel(ah, al) };
    f64::from_bits(s.to_bits() ^ signs(q).1 ^ x.to_bits() & 1 << 63)
}
