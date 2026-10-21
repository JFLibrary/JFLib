// lut -- shared helpers of the effect kernels: per-channel tables, intensity, sRGB, Levels
// and spline tables, the x87 maths                                          [src/fx/fx_lut.s]
//
//   lut_row(dst, src, lut)        per channel table lookup; lut = 4 x 256 bytes B, G, R, A
//   lut_ident(lut)                four identity tables
//   luma(argb) -> u32             paint.net intensity (7471 B + 38470 G + 19595 R) >> 16
//   luma_row(dst8, src)           the intensity of every pixel as a byte
//   pow / exp2 / log2 / cos / sin x87 maths (rt::x87; fx_pow, fx_exp2, fx_log2, fx_cos, fx_sin)
//   srgb_dec(c) / srgb_enc(l)     sRGB 0..1 <-> linear 0..1 (IEC 61966-2-1, f64)
//   lin_table(out)                f32[256]: srgb_dec(i / 255)
//   srgb_table(out, n)            u8[n]: round(255 srgb_enc(i / (n - 1))), n >= 2
//   levels_lut(lut, lvl)          paint.net UnaryPixelOps.Level as a 1024-byte lut
//   spline_lut(lut, pts)          u8[256] from control points {x, y}: natural cubic spline
//                                 (the values of paint.net's Curves), ClampToByte
//
// Levels, channel i (B, G, R) and input j: v = f32(j) - in_lo; v < 0 -> out_lo; j >= in_hi
// -> out_hi; else trunc(clamp(out_lo + (out_hi - out_lo) pow(f64(f32(v / f32(in_hi -
// in_lo))), f64(gamma)), 0, 255)).  A channel with in_hi <= in_lo is the identity; gamma is
// clamped to 0.1 .. 10 (NaN -> 1).

pub use rt::x87::{cos, exp2, log2, pow, sin};
use rt::fx::{maxsd, minsd};

pub const FX_SPLINE_MAX: usize = 64;

/// dst[i] = {lut[B], lut[256 + G], lut[512 + R], lut[768 + A]}  [fx_lut_row]
#[inline]
pub fn lut_row(dst: &mut [u8], src: &[u8], lut: &[u8; 1024]) {
    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let p = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        let v = lut[(p & 255) as usize] as u32
            | (lut[256 + (p >> 8 & 255) as usize] as u32) << 8
            | (lut[512 + (p >> 16 & 255) as usize] as u32) << 16
            | (lut[768 + (p >> 24) as usize] as u32) << 24;
        d.copy_from_slice(&v.to_le_bytes());
    }
}

/// four identity tables  [fx_lut_ident]
pub fn lut_ident(lut: &mut [u8]) {
    for (i, v) in lut.iter_mut().take(1024).enumerate() {
        *v = i as u8;
    }
}

/// (7471 B + 38470 G + 19595 R) >> 16  [fx_luma]
#[inline(always)]
pub fn luma(c: u32) -> u32 {
    ((c & 255) * 7471 + (c >> 8 & 255) * 38470 + (c >> 16 & 255) * 19595) >> 16
}

/// the intensity of every pixel  [fx_luma_row]
pub fn luma_row(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.iter_mut().zip(src.chunks_exact(4)) {
        *d = ((s[0] as u32 * 7471 + s[1] as u32 * 38470 + s[2] as u32 * 19595) >> 16) as u8;
    }
}

/// c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055)^2.4  [fx_srgb_dec]
pub fn srgb_dec(c: f64) -> f64 {
    if c > 0.04045 { pow((c + 0.055) / 1.055, 2.4) } else { c / 12.92 }
}

/// l <= 0.0031308 ? 12.92 l : 1.055 l^(1/2.4) - 0.055, clamped to 0..1  [fx_srgb_enc]
pub fn srgb_enc(l: f64) -> f64 {
    let l = minsd(maxsd(l, 0.0), 1.0);
    if l > 0.0031308 { minsd(pow(l, 0.41666666666666669) * 1.055 - 0.055, 1.0) } else { l * 12.92 }
}

/// f32[256] linear values of the sRGB bytes  [fx_lin_table]
pub fn lin_table(out: &mut [f32]) {
    for (i, v) in out.iter_mut().take(256).enumerate() {
        *v = srgb_dec(i as f64 / 255.0) as f32;
    }
}

/// u8[n]: trunc(255 srgb_enc(i / (n - 1)) + 0.5)  [fx_srgb_table]
pub fn srgb_table(out: &mut [u8], n: usize) {
    if n < 2 {
        return;
    }
    let d = (n - 1) as i32 as f64;
    for (i, v) in out.iter_mut().take(n).enumerate() {
        *v = rt::fx::cvttd(srgb_enc(i as i32 as f64 / d) * 255.0 + 0.5) as u8;
    }
}

/// the LVL record of levels_lut (channel order B, G, R)
#[derive(Clone, Copy, Default)]
pub struct Lvl {
    pub inlo: [u8; 4],
    pub inhi: [u8; 4],
    pub outlo: [u8; 4],
    pub outhi: [u8; 4],
    pub gamma: [f32; 3],
}

/// paint.net UnaryPixelOps.Level as a 1024-byte lut (alpha identity)  [fx_levels_lut]
pub fn levels_lut(lut: &mut [u8; 1024], lvl: &Lvl) {
    lut_ident(lut);
    for c in 0..3 {
        let (lo, hi) = (lvl.inlo[c] as i32, lvl.inhi[c] as i32);
        if hi <= lo {
            continue; // invalid: identity
        }
        let range = (hi - lo) as f32;
        let olo = lvl.outlo[c] as i32;
        let olo_d = olo as f64;
        let span = (lvl.outhi[c] as i32 - olo) as f64;
        let g = lvl.gamma[c] as f64;
        let g = if g.is_nan() { 1.0 } else { g };
        let g = minsd(maxsd(g, 0.1), 10.0);
        let t = &mut lut[c * 256..c * 256 + 256];
        for j in 0..256i32 {
            t[j as usize] = if j < lo {
                lvl.outlo[c]
            } else if j >= hi {
                lvl.outhi[c]
            } else {
                let v = ((j - lo) as f32 / range) as f64;
                let r = pow(v, g) * span + olo_d;
                rt::fx::cvttd(minsd(maxsd(r, 0.0), 255.0)) as u8
            };
        }
    }
}

/// u8[256] from up to FX_SPLINE_MAX control points {x, y} (pts = 2 n bytes, any order, a
/// repeated x: the last wins); n = 0: identity, n = 1: constant  [fx_spline_lut]
pub fn spline_lut(lut: &mut [u8], pts: &[u8], n: usize) {
    if n == 0 {
        for (i, v) in lut.iter_mut().take(256).enumerate() {
            *v = i as u8;
        }
        return;
    }
    let n = n.min(FX_SPLINE_MAX);
    let mut xa = [0f64; FX_SPLINE_MAX];
    let mut ya = [0f64; FX_SPLINE_MAX];
    let mut cp = [0f64; FX_SPLINE_MAX];
    let mut rp = [0f64; FX_SPLINE_MAX];
    let mut m = [0f64; FX_SPLINE_MAX];
    let mut cnt = 0usize;
    // insertion into the sorted key list (a repeated x replaces its y)
    for i in 0..n {
        let x = pts[2 * i] as f64;
        let y = pts[2 * i + 1] as f64;
        let mut p = 0;
        let mut repl = false;
        while p < cnt {
            if xa[p] == x {
                repl = true;
                break;
            }
            if xa[p] > x {
                break;
            }
            p += 1;
        }
        if repl {
            ya[p] = y;
            continue;
        }
        let mut c = cnt;
        while c > p {
            xa[c] = xa[c - 1];
            ya[c] = ya[c - 1];
            c -= 1;
        }
        xa[p] = x;
        ya[p] = y;
        cnt += 1;
    }
    if cnt == 1 {
        let v = rt::fx::cvttd(ya[0]) as u8;
        for t in lut.iter_mut().take(256) {
            *t = v;
        }
        return;
    }
    // Natural cubic spline: the second derivatives m[i] at the knots solve the tridiagonal
    // system  mu_i m[i-1] + 2 m[i] + (1 - mu_i) m[i+1] = 6 (s_i - s_(i-1)) / (x[i+1] - x[i-1])
    // for the inner knots, with mu_i = (x[i] - x[i-1]) / (x[i+1] - x[i-1]), s_i the slope of
    // interval i and m = 0 at both ends.  Thomas algorithm: forward elimination into the
    // reduced super-diagonal cp and right-hand side rp, then back substitution.
    for i in 1..cnt - 1 {
        let span = xa[i + 1] - xa[i - 1];
        let mu = (xa[i] - xa[i - 1]) / span;
        let pivot = 2.0 - mu * cp[i - 1];
        cp[i] = (1.0 - mu) / pivot;
        let bend = (ya[i + 1] - ya[i]) / (xa[i + 1] - xa[i]) - (ya[i] - ya[i - 1]) / (xa[i] - xa[i - 1]);
        rp[i] = (bend * 6.0 / span - mu * rp[i - 1]) / pivot;
    }
    for i in (0..cnt - 1).rev() {
        m[i] = rp[i] - cp[i] * m[i + 1];
    }
    // evaluation on interval seg (x[seg] <= x < x[seg + 1], clamped to the first / last one):
    // the linear interpolation plus the cubic correction of the end curvatures
    let mut seg = 0usize;
    for x in 0..256usize {
        let xf = x as f64;
        while seg + 2 < cnt && xa[seg + 1] <= xf {
            seg += 1;
        }
        let (x0, x1) = (xa[seg], xa[seg + 1]);
        let h = x1 - x0;
        let w0 = (x1 - xf) / h;
        let w1 = (xf - x0) / h;
        let mut v = w0 * ya[seg] + w1 * ya[seg + 1];
        let corr = (w0 * w0 * w0 - w0) * m[seg] + (w1 * w1 * w1 - w1) * m[seg + 1];
        v += corr * (h * h) / 6.0;
        // ClampToByte: > 255 -> 255, < 0 -> 0, else truncate (NaN -> 0)
        lut[x] = if v > 255.0 {
            255
        } else if v > 0.0 {
            rt::fx::cvttd(v) as u8
        } else {
            0
        };
    }
}
