// gradient -- paint.net style gradients (7 shapes x 3 repeat modes)       [src/gfx/gradient.s]
//
//   grad_span(dst, x, y, g)         dst.len() / 4 straight ARGB pixels of row y, columns x ..
//                                   (pixel centres x + i + 0.5, y + 0.5); with GF_ALPHA_MODE the
//                                   existing pixels keep their colour and their alpha is
//                                   multiplied by the ramp A.a -> B.a: a' = round(a * ramp / 255)
//   grad_alpha_span(dst, x, y, g)   dst.len() bytes: the ramp A.a -> B.a
//   grad_t_span(dst, x, y, g)       positions T (0 .. GT_ONE)
//
// With d = end - start, L2 = d.d, v = p - start, dot = vx dx + vy dy, cross = vy dx - vx dy,
// r = sqrt(vx vx + vy vy):  linear dot / L2, linear reflected |dot| / L2, diamond (|dot| +
// |cross|) / L2, radial r / L, conical |atan2(cross, dot)| / pi, spirals frac(r / L +-
// atan2 / 2pi).  atan2 is the polynomial of the assembly (Abramowitz-Stegun 4.4.49).  Repeat:
// none clamps to [0, 1], wrap frac(t), reflect the triangle wave of period 2.  T = floor(t *
// 16384 + 0.5); every channel c = (A (16384 - T) + B T + bias) >> 14, bias 8192 or the 4 x 4
// Bayer threshold (GF_DITHER).  Degenerate L2 (0 or not finite): T = 16384.  f64 operations in
// the order of the assembly (no FMA), lane by lane in SIMD registers, so the results are
// bit-identical.

use rt::fx::sqrtd;
use simd::{Kernel, Simd};

pub const GT_LINEAR: u8 = 0;
pub const GT_LINEAR_REFLECTED: u8 = 1;
pub const GT_DIAMOND: u8 = 2;
pub const GT_RADIAL: u8 = 3;
pub const GT_CONICAL: u8 = 4;
pub const GT_SPIRAL_CW: u8 = 5;
pub const GT_SPIRAL_CCW: u8 = 6;
pub const GT_COUNT: u8 = 7;
pub const GR_NONE: u8 = 0;
pub const GR_WRAP: u8 = 1;
pub const GR_REFLECT: u8 = 2;
pub const GF_ALPHA_MODE: u16 = 1;
pub const GF_DITHER: u16 = 2;
pub const GT_ONE: i32 = 16384;

/// a gradient (the GRAD record): type GT_*, repeat GR_*, flags GF_*, colours A (t = 0) and
/// B (t = 1), start and end point
#[derive(Clone, Copy, Default)]
pub struct Grad {
    pub typ: u8,
    pub repeat: u8,
    pub flags: u16,
    pub col_a: u32,
    pub col_b: u32,
    pub sx: f64,
    pub sy: f64,
    pub ex: f64,
    pub ey: f64,
}

const SIGN: u64 = 0x8000_0000_0000_0000;
const DBL_MIN: f64 = f64::from_bits(0x0010_0000_0000_0000);
const HPI: f64 = f64::from_bits(0x3FF9_21FB_5444_2D18);
const PI: f64 = f64::from_bits(0x4009_21FB_5444_2D18);
const IPI: f64 = f64::from_bits(0x3FD4_5F30_6DC9_C883);
const I2PI: f64 = f64::from_bits(0x3FC4_5F30_6DC9_C883);
const AT1: f64 = f64::from_bits(0x3FEF_FEE6_FB4C_3C19);
const AT3: f64 = f64::from_bits(0xBFD5_23A0_8398_A655);
const AT5: f64 = f64::from_bits(0x3FC7_0EDC_3BD5_9924);
const AT7: f64 = f64::from_bits(0xBFB5_CB46_BACF_7447);
const AT9: f64 = f64::from_bits(0x3F95_55CB_E46F_80C1);
const LO: f64 = -1073741824.0; // -2^30
const HI: f64 = 1073741824.0;
static BAYER: [u32; 16] = [512, 8704, 2560, 10752, 12800, 4608, 14848, 6656, 3584, 11776, 1536, 9728, 15872, 7680, 13824, 5632];

// ---- the kernel (gr_span): T of 64-pixel chunks in f64 lanes (SSE2 2, AVX2 4; IEEE
// operations lane by lane, so the result does not depend on the lane count), then the output
// stage per pixel (colour: pmaddwd of the (A, B) word pairs with (16384 - T, T))

const CHUNK: usize = 64;
static I0123: [i32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

enum Out<'a> {
    Color(&'a mut [u8]),    // straight ARGB pixels
    AlphaMul(&'a mut [u8]), // GF_ALPHA_MODE: the existing alphas x the ramp
    A8(&'a mut [u8]),       // the ramp
    T(&'a mut [i32]),       // the positions
}

struct GradK<'a> {
    out: Out<'a>,
    x: i32,
    y: i32,
    g: &'a Grad,
}

impl Kernel for GradK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        span(s, self);
    }
}

// GR_FRAC: frac of v clamped to +-2^30
#[inline(always)]
fn vfrac<S: Simd>(s: S, v: S::V) -> S::V {
    let v = s.mind(s.maxd(v, s.splatd(LO)), s.splatd(HI));
    let t = s.cvt_i32_f64(s.cvtt_f64_i32(v));
    let fl = s.subd(t, s.and(s.cmpltd(v, t), s.splatd(1.0)));
    s.subd(v, fl)
}

// GR_ATAN2: the polynomial atan2(y, x) lane by lane
#[inline(always)]
fn vatan2<S: Simd>(s: S, y: S::V, x: S::V) -> S::V {
    let abs = s.splat64(!SIGN);
    let ax = s.and(x, abs);
    let ay = s.and(y, abs);
    let mx = s.maxd(s.maxd(ax, ay), s.splatd(DBL_MIN));
    let z = s.divd(s.mind(ax, ay), mx);
    let lt = s.cmpltd(ax, ay); // |x| < |y|
    let q = s.muld(z, z);
    let mut p = s.splatd(AT9);
    for c in [AT7, AT5, AT3, AT1] {
        p = s.addd(s.muld(p, q), s.splatd(c));
    }
    let p = s.muld(p, z);
    let r = s.or(s.and(s.subd(s.splatd(HPI), p), lt), s.andnot(lt, p));
    let xn = s.cmpltd(x, s.zero()); // x < 0: pi - r
    let r = s.or(s.and(s.subd(s.splatd(PI), r), xn), s.andnot(xn, r));
    let yn = s.cmpltd(y, s.zero()); // y < 0: -r
    s.xor(r, s.and(yn, s.splat64(SIGN)))
}

#[inline(always)]
fn span<S: Simd>(s: S, k: GradK) {
    let GradK { mut out, x, y, g } = k;
    let n = match &out {
        Out::Color(d) | Out::AlphaMul(d) => d.len() / 4,
        Out::A8(d) => d.len(),
        Out::T(d) => d.len(),
    };
    let typ = if g.typ >= GT_COUNT { GT_LINEAR } else { g.typ };
    let dx = g.ex - g.sx;
    let dy = g.ey - g.sy;
    let l2 = dx * dx + dy * dy;
    let degenerate = !(l2 > 0.0) || (l2 - l2).is_nan();
    let vy = (y as f64 + 0.5) - g.sy;
    let (vdx, vdy, vsx) = (s.splatd(dx), s.splatd(dy), s.splatd(g.sx));
    let (vydy, vydx, vyvy) = (s.splatd(vy * dy), s.splatd(vy * dx), s.splatd(vy * vy));
    let (il2, il) = (s.splatd(1.0 / l2), s.splatd(1.0 / sqrtd(l2)));
    let (half, one, absm) = (s.splatd(0.5), s.splatd(1.0), s.splat64(!SIGN));
    let dl = S::BYTES / 8; // f64 lanes
    let (aa, ba) = (g.col_a >> 24, g.col_b >> 24);
    let mut tb = [0i32; CHUNK + 4];
    let mut done = 0;
    while done < n {
        let m = (n - done).min(CHUNK);
        let x0 = x.wrapping_add(done as i32);
        if degenerate {
            tb[..m].fill(GT_ONE);
        } else {
            let mut i = 0;
            while i < m {
                let xi = s.add32(s.splat32(x0.wrapping_add(i as i32) as u32), s.load_i32(&I0123));
                let vx = s.subd(s.addd(s.cvt_i32_f64(xi), half), vsx);
                let t = match typ {
                    GT_RADIAL => s.muld(s.sqrtd(s.addd(s.muld(vx, vx), vyvy)), il),
                    GT_CONICAL | GT_SPIRAL_CW | GT_SPIRAL_CCW => {
                        let dot = s.addd(s.muld(vx, vdx), vydy);
                        let cross = s.subd(vydx, s.muld(vx, vdy));
                        let rl = s.muld(s.sqrtd(s.addd(s.muld(vx, vx), vyvy)), il);
                        let a = vatan2(s, cross, dot);
                        if typ == GT_CONICAL {
                            s.muld(s.and(a, absm), s.splatd(IPI))
                        } else {
                            let mut sa = s.muld(a, s.splatd(I2PI));
                            if typ == GT_SPIRAL_CCW {
                                sa = s.xor(sa, s.splat64(SIGN));
                            }
                            vfrac(s, s.addd(sa, rl))
                        }
                    }
                    _ => {
                        let mut dot = s.addd(s.muld(vx, vdx), vydy);
                        if typ != GT_LINEAR {
                            dot = s.and(dot, absm);
                        }
                        if typ == GT_DIAMOND {
                            dot = s.addd(dot, s.and(s.subd(vydx, s.muld(vx, vdy)), absm));
                        }
                        s.muld(dot, il2)
                    }
                };
                let t = match g.repeat {
                    GR_WRAP => vfrac(s, t),
                    GR_REFLECT => {
                        let f = vfrac(s, s.muld(t, half));
                        let f = s.and(s.subd(s.addd(f, f), one), absm);
                        s.subd(one, f)
                    }
                    _ => s.mind(s.maxd(t, s.zero()), one),
                };
                let tv = s.cvtt_f64_i32(s.addd(s.muld(t, s.splatd(16384.0)), half));
                s.store_i32_half(&mut tb[i..], tv);
                i += dl;
            }
        }
        // output stage
        match &mut out {
            Out::Color(d) => {
                let x2 = simd::Sse2; // one pixel per register, as in the assembly
                let z = x2.zero();
                let ab = x2.unpacklo16(x2.unpacklo8(x2.from_i32(g.col_a as i32), z), x2.unpacklo8(x2.from_i32(g.col_b as i32), z));
                for j in 0..m {
                    let t = tb[j];
                    let w = x2.splat32((t as u32) << 16 | (GT_ONE - t) as u32);
                    let b = x2.splat32(bias(g, x0.wrapping_add(j as i32), y));
                    let v = x2.sra32::<14>(x2.add32(x2.madd16(ab, w), b));
                    let px = x2.to_i32(x2.packus16(x2.packs32(v, v), z));
                    let o = (done + j) * 4;
                    d[o..o + 4].copy_from_slice(&px.to_le_bytes());
                }
            }
            Out::AlphaMul(d) => {
                for j in 0..m {
                    let t = tb[j] as u32;
                    let ramp = ((16384 - t) * aa + t * ba + bias(g, x0.wrapping_add(j as i32), y)) >> 14;
                    let o = (done + j) * 4 + 3;
                    d[o] = (((d[o] as u32 * ramp + 128) * 257) >> 16) as u8;
                }
            }
            Out::A8(d) => {
                for j in 0..m {
                    let t = tb[j] as u32;
                    d[done + j] = (((16384 - t) * aa + t * ba + bias(g, x0.wrapping_add(j as i32), y)) >> 14) as u8;
                }
            }
            Out::T(d) => d[done..done + m].copy_from_slice(&tb[..m]),
        }
        done += m;
    }
}

fn bias(g: &Grad, x: i32, y: i32) -> u32 {
    if g.flags & GF_DITHER != 0 { BAYER[((y & 3) * 4 + (x & 3)) as usize] } else { 8192 }
}

/// straight ARGB pixels of gradient g (GF_ALPHA_MODE: multiply the existing alphas)  [grad_span]
pub fn grad_span(dst: &mut [u8], x: i32, y: i32, g: &Grad) {
    let out = if g.flags & GF_ALPHA_MODE != 0 { Out::AlphaMul(dst) } else { Out::Color(dst) };
    simd::dispatch(GradK { out, x, y, g });
}

/// the alpha ramp A.a -> B.a (transparency mode mask)  [grad_alpha_span]
pub fn grad_alpha_span(dst: &mut [u8], x: i32, y: i32, g: &Grad) {
    simd::dispatch(GradK { out: Out::A8(dst), x, y, g });
}

/// the positions T (0 .. GT_ONE)  [grad_t_span]
pub fn grad_t_span(dst: &mut [i32], x: i32, y: i32, g: &Grad) {
    simd::dispatch(GradK { out: Out::T(dst), x, y, g });
}
