// resample -- separable image resampling (resize) of 32-bit surfaces       [src/img/resample.s]
//
//   resize(src, dst, filter, flags) -> 0 / ERR_*            resample src into dst (dst's size
//                                                           is the target size)  [rs_resize]
//   Resize::prepare(src, dst, filter, flags) -> Result<Resize, i32>
//        validate and build the weight tables (dst is only measured)              [rs_prepare]
//   ctx.rows(band, y0, y1) -> 0 / ERR_*                     dst rows y0 .. y1-1 (clamped) into
//        a Band of dst; several threads may run it at once on disjoint bands  [rs_resize_rows]
//   axis_weights(s, d, filter) -> Result<AxisWeights, i32>  the table of one axis
//                                                                          [rs_axis_weights]
//   to_work(w, src, n, fmt, flags)      n 8-bit pixels -> working row             [rs_to_work]
//   from_work(dst, w, n, fmt, flags)    n working pixels -> 8-bit row           [rs_from_work]
//
// Formats: src and dst may be FMT_ARGB (straight) or FMT_PARGB (premultiplied), in any
// combination (ERR_UNSUPPORTED otherwise); sizes 1 .. 65535 (ERR_DIMS).  Filters and flags:
// RS_* below.
//
// Pipeline (filters other than RS_NEAREST):
//   1. Working values: every source row becomes 4 x i16 per pixel, premultiplied, a full
//      channel = RS_PONE = 16320 (= 255 * 64):
//        FMT_PARGB   P = v << 6 (all four channels)
//        FMT_ARGB    P_a = a << 6,  P_c = ((c * a + 2) * 16448) >> 16
//        RS_GAMMA    straight s (FMT_PARGB: s = min(255, (v * 255 + a / 2) / a), a = 0 -> 0),
//                    lin = LIN16[s] = round(16320 * srgb_to_linear(s / 255)),
//                    P_c = (lin * a * 257 + 32768) >> 16,  P_a = a << 6
//   2. Horizontal pass: H = (sum_k w_k * P[x0 + k] + 8192) >> 14, saturated to i16
//      (arithmetic shift = floor); results are NOT clamped between the passes.
//   3. Vertical pass: V = (sum_j w_j * H[y0 + j] + 8192) >> 14, saturated to i16.
//   4. Output: A = max(P_a, 0) (may exceed 16320: ringing at alpha edges), colours clamped
//      to 0 .. A, a = (min(A, 16320) + 32) >> 6; the colour ratio P_c / A is kept:
//        FMT_ARGB    c = rint_even(f32(P_c * 255) / f32(max(A, 1)))
//        FMT_PARGB   c = (P_c + 32) >> 6, or the straight c above when A > 16320 (a = 255)
//        RS_GAMMA    A = 0 -> pixel 0; else idx = (P_c * 8190 + A) / (2 A),
//                    s = SRGB8[idx] = round(255 * linear_to_srgb(idx / 4095)); straight: c = s,
//                    premultiplied: c = ((s * a + 128) * 257) >> 16
// RS_NEAREST copies the source pixel whose centre is nearest: sx = ((2x + 1) S) / (2 D), no
// premultiplication.  When both axes are identities (same size and an interpolating filter:
// everything except RS_BICUBIC_SMOOTH) the rows are copied.  Format changes on the copy /
// nearest paths use gfx::straight premul / unpremul.
//
// Weight tables (per axis, per output pixel i, all f64 with IEEE double semantics):
//   scale = S / D; fs = max(scale, 1) for widened filters, else 1; R = support * fs
//   (support: triangle 1, cubics 2, Lanczos 3).  center = ((2i + 1) S) / (2 D);
//   j = floor(center - R) .. ceil(center + R): w_j = K(((j + 0.5) - center) / fs).
//   RS_FANT: lo = (i S) / D, hi = ((i + 1) S) / D, j = floor(lo) .. ceil(hi) - 1:
//   w_j = min(hi, j + 1) - max(lo, j).
//   total = sum w_j (in j order); every w_j is added to pixel clamp(j, 0, S - 1) (clamp to
//   edge); q = floor(w / total * 16384 + 0.5); 16384 - sum(q) is added to the first largest
//   q; zero taps at both ends are dropped.  Kernels (x = |x|):
//     triangle  max(0, 1 - x)
//     cubic a   x < 1: ((a + 2) x - (a + 3)) x x + 1;  x < 2: ((a x - 5a) x + 8a) x - 4a
//               (Catmull-Rom a = -0.5; RS_ADAPTIVE growing: a = -0.75)
//     Mitchell  x < 1: (7/6 x - 2) x x + 8/9;  x < 2: ((-7/18 x + 2) x - 10/3) x + 16/9
//     Lanczos-3 x = 0: 1; x < 3: sinpi(x) sinpi(x / 3) / (pi x (pi x / 3)) with x / 3 computed
//               as x * (1/3); sinpi: n = floor(t + 0.5), z = pi (t - n), Taylor polynomial
//               of degree 13 (Horner in z^2), negated for odd n
//   RS_ADAPTIVE: per axis Lanczos-3 when S > D, else cubic a = -0.75.
// Record of a table (AxisWeights::tab, the bytes of the assembly): +0 i32 first source pixel
// x0, +4 i32 taps used, +8 8 zero bytes, +16 n/2 pairs of 16 bytes {w_2k, w_2k+1} x 4 (i16)
// -- the pmaddwd operand.  n = the largest tap count rounded up to even; x0 is moved left so
// that x0 + n <= S when possible.
//
// Threads: no global state.  rows() allocates its scratch per call (about (S_w + n) * 24 +
// K * (D_w + 4) * 8 bytes; K = taps of the vertical filter, at most RS_RING_BUDGET bytes of
// ring rows; more taps are summed in groups through i32 accumulators).  All sums are i32
// (wrapping), so groups, band splits and register widths never change a result.  The
// horizontal kernels keep the SSE2 layout of the assembly (pair rows); the vertical pass,
// to_work and from_work work on SSE2 or AVX2 registers (simd::Kernel), pixel by pixel equal.

use alloc::vec;
use alloc::vec::Vec;

use gfx::straight::{premul_inplace, premul_row, unpremul_inplace, unpremul_row};
use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use rt::fx::{cvt, cvttd64, maxsd, minsd};
use simd::{Kernel, Simd, Sse2};

use crate::ops::Band;
use crate::{ERR_DIMS, ERR_UNSUPPORTED};

pub const RS_NEAREST: u32 = 0; // point sampling (no premultiplication, pixels copied)
pub const RS_BILINEAR: u32 = 1; // triangle, widened by the scale factor when downscaling
pub const RS_BILINEAR_LQ: u32 = 2; // triangle, never widened (plain 2 x 2)
pub const RS_BICUBIC: u32 = 3; // Catmull-Rom (Keys a = -0.5), widened when downscaling
pub const RS_BICUBIC_SMOOTH: u32 = 4; // Mitchell-Netravali B = C = 1/3, widened
pub const RS_LANCZOS3: u32 = 5; // sinc(x) sinc(x/3), |x| < 3, widened when downscaling
pub const RS_FANT: u32 = 6; // exact area average
pub const RS_ADAPTIVE: u32 = 7; // per axis: Lanczos-3 shrinking, Keys a = -0.75 growing
pub const RS_FILTER_COUNT: u32 = 8;
pub const RS_MULTISAMPLE: u32 = 8; // xform only: 4 x 4 supersampled bilinear
pub const RS_GAMMA: u32 = 1; // flag: filter in linear light
pub const RS_WONE: i32 = 16384; // sum of every output's weights
pub const RS_PONE: i32 = 16320; // working value of a full channel (255 * 64)

const RS_RING_BUDGET: u64 = 64 << 20; // bytes of ring rows per rows() call
const RS_MAXDIM: u32 = 65535;

const KID_TRI: usize = 0;
const KID_CUB5: usize = 1;
const KID_MIT: usize = 2;
const KID_LAN: usize = 3;
const KID_CUB75: usize = 4;
const KID_FANT: usize = 5;

const VP_ADD: u32 = 1; // vertical pass: start from the accumulators
const VP_FINAL: u32 = 2; // round, shift, saturate into the output row

const SIGN: u64 = 0x8000_0000_0000_0000;
const PI: f64 = f64::from_bits(0x4009_21FB_5444_2D18);
const THIRD: f64 = f64::from_bits(0x3FD5_5555_5555_5555);
const S3: f64 = f64::from_bits(0xBFC5_5555_5555_5555);
const S5: f64 = f64::from_bits(0x3F81_1111_1111_1111);
const S7: f64 = f64::from_bits(0xBF2A_01A0_1A01_A01A);
const S9: f64 = f64::from_bits(0x3EC7_1DE3_A556_C734);
const S11: f64 = f64::from_bits(0xBE5A_E645_67F5_44E4);
const S13: f64 = f64::from_bits(0x3DE6_1246_13A8_6D09);
const M1: f64 = f64::from_bits(0x3FF2_AAAA_AAAA_AAAB); // 7/6
const M0: f64 = f64::from_bits(0x3FEC_71C7_1C71_C71C); // 8/9
const N3: f64 = f64::from_bits(0xBFD8_E38E_38E3_8E39); // -7/18
const N1: f64 = f64::from_bits(0x400A_AAAA_AAAA_AAAB); // 10/3
const N0: f64 = f64::from_bits(0x3FFC_71C7_1C71_C71C); // 16/9
// cubic coefficients {a + 2, a + 3, a, 5a, 8a, 4a}
const CUB5: [f64; 6] = [1.5, 2.5, -0.5, -2.5, -4.0, -2.0];
const CUB75: [f64; 6] = [1.25, 2.25, -0.75, -3.75, -6.0, -3.0];
// per kernel id: support
const SUP: [f64; 6] = [1.0, 2.0, 2.0, 3.0, 2.0, 1.0];
// per filter: kernel id, widened
const FKID: [usize; 8] = [KID_TRI, KID_TRI, KID_TRI, KID_CUB5, KID_MIT, KID_LAN, KID_FANT, KID_LAN];
const FWIDE: [bool; 8] = [false, true, false, true, true, true, false, true];

// ------------------------------------------------------------------ weight tables

/// the weight table of one axis (S source -> D destination pixels)
pub struct AxisWeights {
    pub tab: Vec<u8>, // D records of rb bytes (format: module header)
    pub n: u32,       // taps per record (even)
    pub rb: u32,      // bytes per record = n * 8 + 16
    pub nmax: u32,    // largest real tap count
    pub id: bool,     // S == D with one tap per output (an identity)
}

// rs_floor: floor(x) for |x| < 2^62 (truncate, then step down)
#[inline]
fn floor64(x: f64) -> i64 {
    let t = cvttd64(x);
    if t as f64 > x { t.wrapping_sub(1) } else { t }
}

// rs_ceil: -floor(-x)
#[inline]
fn ceil64(x: f64) -> i64 {
    let v = -x;
    let t = cvttd64(v);
    let t = if t as f64 > v { t.wrapping_sub(1) } else { t };
    t.wrapping_neg()
}

// SINPI: sin(pi t)
fn sinpi(t: f64) -> f64 {
    let t5 = t + 0.5;
    let mut n = cvttd64(t5);
    let mut nd = n as f64;
    if nd > t5 {
        n = n.wrapping_sub(1);
        nd = nd - 1.0;
    }
    let z = (t - nd) * PI;
    let z2 = z * z;
    let p = (((((S13 * z2 + S11) * z2 + S9) * z2 + S7) * z2 + S5) * z2 + S3) * z2 + 1.0;
    let r = z * p;
    if n & 1 != 0 { -r } else { r }
}

// rs_kern: K(x) of a kernel id
fn kern(kid: usize, x: f64) -> f64 {
    let ax = f64::from_bits(x.to_bits() & !SIGN);
    match kid {
        KID_TRI => maxsd(1.0 - ax, 0.0),
        KID_MIT => {
            if !(ax >= 1.0) {
                ((M1 * ax - 2.0) * ax) * ax + M0
            } else if !(ax >= 2.0) {
                ((N3 * ax + 2.0) * ax - N1) * ax + N0
            } else {
                0.0
            }
        }
        KID_LAN => {
            if ax >= 3.0 {
                0.0
            } else if x == 0.0 {
                1.0
            } else {
                let num = sinpi(x * THIRD) * sinpi(x);
                let px = x * PI;
                num / (px * (px * THIRD))
            }
        }
        _ => {
            let c = if kid == KID_CUB75 { &CUB75 } else { &CUB5 };
            if !(ax >= 1.0) {
                ((c[0] * ax - c[1]) * ax) * ax + 1.0
            } else if !(ax >= 2.0) {
                ((c[2] * ax - c[3]) * ax + c[4]) * ax - c[5]
            } else {
                0.0
            }
        }
    }
}

/// the weight table of one axis: ERR_DIMS (sizes outside 1 .. 65535), ERR_UNSUPPORTED
/// (RS_NEAREST or an unknown filter)  [rs_axis_weights]
pub fn axis_weights(s: i32, d: i32, filter: u32) -> Result<AxisWeights, i32> {
    let (su, du) = (s as u32, d as u32);
    if su.wrapping_sub(1) > RS_MAXDIM - 1 || du.wrapping_sub(1) > RS_MAXDIM - 1 {
        return Err(ERR_DIMS);
    }
    if filter > RS_ADAPTIVE || filter == RS_NEAREST {
        return Err(ERR_UNSUPPORTED);
    }
    let mut kid = FKID[filter as usize];
    if filter == RS_ADAPTIVE && su <= du {
        kid = KID_CUB75; // growing: Keys a = -0.75
    }
    let fant = kid == KID_FANT;
    let (sf, df) = (su as f64, du as f64);
    let scale = sf / df;
    let fs = if FWIDE[filter as usize] { maxsd(1.0, scale) } else { 1.0 };
    let r = SUP[kid] * fs;
    // bound B on the taps of one output
    let b = if fant { cvttd64(scale).wrapping_add(3) } else { cvttd64(r + r).wrapping_add(4) } as i32;
    let bu = b as usize;
    let (dn, sl) = (du as usize, su as i64 - 1);
    let mut raw = vec![0f64; bu];
    let mut folded = vec![0f64; bu];
    let mut q = vec![0i32; bu];
    let mut qtab = vec![0i16; dn * bu];
    let mut starts = vec![(0i32, 0u32); dn];
    let mut nmax = 0u32;

    // ---- pass 1: every output pixel
    for i in 0..dn {
        let (jlo, mut jhi);
        let mut total = 0.0f64;
        if !fant {
            let center = (((2 * i as i64 + 1) * su as i64) as f64) / ((2 * du as i64) as f64);
            jlo = floor64(center - r);
            jhi = ceil64(center + r);
            jhi = jhi.min(jlo.wrapping_add(b as i64 - 1));
            let mut k = 0;
            let mut j = jlo;
            while j <= jhi {
                let w = kern(kid, ((j as f64 + 0.5) - center) / fs);
                raw[k] = w;
                total += w;
                k += 1;
                j += 1;
            }
        } else {
            let lo = ((i as i64 * su as i64) as f64) / df;
            let hi = (((i as i64 + 1) * su as i64) as f64) / df;
            jlo = floor64(lo);
            jhi = ceil64(hi).wrapping_sub(1);
            jhi = jhi.min(jlo.wrapping_add(b as i64 - 1));
            let mut k = 0;
            let mut j = jlo;
            while j <= jhi {
                let jf = j as f64;
                let w = minsd(jf + 1.0, hi) - maxsd(jf, lo);
                raw[k] = w;
                total += w;
                k += 1;
                j += 1;
            }
        }
        let cnt = (jhi - jlo + 1) as u32 as usize;
        // ---- fold onto 0 .. S-1
        let base = jlo.max(0) as i32;
        let top = jhi.min(sl);
        let nf = (top - base as i64 + 1) as u32 as usize;
        folded[..nf].fill(0.0);
        for (k, &w) in raw[..cnt].iter().enumerate() {
            let j = (jlo + k as i64).max(0).min(sl);
            let idx = (j - base as i64) as usize;
            folded[idx] += w;
        }
        // ---- quantize
        if total == 0.0 {
            total = 1.0; // all weights 0: avoid 0 / 0
        }
        let mut sum = 0i32;
        for k in 0..nf {
            let v = floor64(folded[k] / total * 16384.0 + 0.5).clamp(-32767, 32767) as i32;
            q[k] = v;
            sum = sum.wrapping_add(v);
        }
        let mut am = 0;
        for k in 1..nf {
            if q[k] > q[am] {
                am = k;
            }
        }
        q[am] = q[am].wrapping_add(RS_WONE.wrapping_sub(sum));
        // trim zero taps at both ends
        let mut first = 0;
        while first < nf && q[first] == 0 {
            first += 1;
        }
        let mut last = nf as i64 - 1;
        while last > first as i64 && q[last as usize] == 0 {
            last -= 1;
        }
        if first >= nf {
            first = 0; // (cannot happen: the sum is 16384)
            last = 0;
        }
        let count = (last - first as i64 + 1) as u32;
        starts[i] = (base + first as i32, count);
        nmax = nmax.max(count);
        for t in 0..count as usize {
            qtab[i * bu + t] = q[first + t] as i16;
        }
    }

    // ---- pass 2: the table with n (even) taps per record
    let n = ((nmax + 1) & !1).max(2);
    let rb = n * 8 + 16;
    let mut tab = vec![0u8; rb as usize * dn];
    for (i, rec) in tab.chunks_exact_mut(rb as usize).enumerate() {
        let (start, count) = starts[i];
        let mut x0 = start;
        if (start as u32).wrapping_add(n) > su {
            x0 = (su.wrapping_sub(n)) as i32;
            if x0 < 0 {
                x0 = 0;
            }
        }
        rec[0..4].copy_from_slice(&x0.to_le_bytes());
        let off = start - x0;
        rec[4..8].copy_from_slice(&(off + count as i32).to_le_bytes());
        for t in 0..count as usize {
            let w = qtab[i * bu + t].to_le_bytes();
            let tap = off as usize + t;
            let pos = (tap >> 1) * 16 + (tap & 1) * 2 + 16;
            for k in 0..4 {
                rec[pos + k * 4..pos + k * 4 + 2].copy_from_slice(&w);
            }
        }
    }
    Ok(AxisWeights { tab, n, rb, nmax, id: su == du && nmax == 1 })
}

// ------------------------------------------------------------------ row kernels

#[inline]
fn px_of(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

// one pixel to working values (no RS_GAMMA)
#[inline(always)]
fn to_work_px(w: &mut [i16], p: u32, fmt: u8) {
    let a = p >> 24;
    for c in 0..3 {
        let v = (p >> (c * 8)) & 255;
        w[c] = if fmt == FMT_ARGB { (((v * a + 2) * 16448) >> 16) as i16 } else { (v << 6) as i16 };
    }
    w[3] = (a << 6) as i16;
}

// working values of PX pixels in pixel lanes (no RS_GAMMA): FMT_ARGB premultiplies
#[inline(always)]
fn work_lanes<S: Simd>(s: S, v: S::V, argb: bool) -> S::V {
    if !argb {
        return s.sll16::<6>(v);
    }
    let t = s.mulhiu16(s.add16(s.mullo16(v, s.shuf16::<0xFF>(v)), s.splat16(2)), s.splat16(16448));
    let am = s.splat64(0xFFFF_0000_0000_0000);
    s.or(s.and(s.sll16::<6>(v), am), s.andnot(am, t))
}

struct ToWorkK<'a> {
    w: &'a mut [i16],
    src: &'a [u8],
    n: usize,
    fmt: u8,
}

impl Kernel for ToWorkK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let ToWorkK { w, src, n, fmt } = self;
        let argb = fmt == FMT_ARGB;
        let mut i = 0;
        while i + S::PX <= n {
            let v = work_lanes(s, s.load_px(&src[i * 4..]), argb);
            s.store_i16(&mut w[i * 4..], v);
            i += S::PX;
        }
        while i < n {
            to_work_px(&mut w[i * 4..i * 4 + 4], px_of(&src[i * 4..]), fmt);
            i += 1;
        }
    }
}

// RS_GAMMA: linear light (straight s from LIN16)
fn to_work_gamma(w: &mut [i16], src: &[u8], n: usize, fmt: u8) {
    for (o, p) in w[..n * 4].chunks_exact_mut(4).zip(src.chunks_exact(4)) {
        let p = px_of(p);
        let a = p >> 24;
        o[3] = (a << 6) as i16;
        for c in 0..3 {
            let v = (p >> (c * 8)) & 255;
            let s = if fmt != FMT_PARGB {
                v
            } else if a == 0 {
                0
            } else {
                ((v * 255 + a / 2) / a).min(255)
            };
            o[c] = ((LIN16[s as usize] as u32 * a * 257 + 32768) >> 16) as i16;
        }
    }
}

/// n pixels of an 8-bit row (FMT_ARGB / FMT_PARGB) to working values (4 x i16 per pixel,
/// premultiplied, full = 16320); RS_GAMMA in flags: linear light.  Writes w[..n * 4].
/// [rs_to_work]
pub fn to_work(w: &mut [i16], src: &[u8], n: usize, fmt: u8, flags: u32) {
    if flags & RS_GAMMA != 0 {
        to_work_gamma(w, src, n, fmt);
    } else {
        simd::dispatch(ToWorkK { w, src, n, fmt });
    }
}

// one working pixel to 8 bits (no RS_GAMMA)
#[inline(always)]
fn from_work_px(p: &[i16], fmt: u8) -> u32 {
    let al = p[3];
    let aa = al.max(0) as i32;
    let a8 = ((aa.min(RS_PONE) + 32) >> 6) as u32;
    let straight = fmt == FMT_ARGB || aa > RS_PONE;
    let mut out = a8 << 24;
    for c in 0..3 {
        let v = p[c].min(al).max(0) as i32;
        let cv = if straight { cvt((v * 255) as f32 / aa.max(1) as f32).clamp(0, 255) } else { (v + 32) >> 6 };
        out |= (cv as u32) << (c * 8);
    }
    out
}

struct FromWorkK<'a> {
    dst: &'a mut [u8],
    w: &'a [i16],
    n: usize,
    fmt: u8,
}

impl Kernel for FromWorkK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let FromWorkK { dst, w, n, fmt } = self;
        let z = s.zero();
        let (pone, c32, one, f255) = (s.splat16(RS_PONE as u16), s.splat16(32), s.splat16(1), s.splatf(255.0));
        let cm = s.splat64(0x0000_FFFF_FFFF_FFFF); // colour lanes
        let argb = fmt == FMT_ARGB;
        let mut i = 0;
        while i + S::PX <= n {
            let v = s.load_i16(&w[i * 4..]);
            let al = s.shuf16::<0xFF>(v); // raw alpha of each pixel
            let c = s.maxs16(s.mins16(v, al), z); // 0 <= colour <= alpha
            let aa = s.maxs16(al, z);
            let a8 = s.srl16::<6>(s.add16(s.mins16(aa, pone), c32));
            let pm = s.srl16::<6>(s.add16(c, c32));
            let mut res = s.or(s.and(pm, cm), s.andnot(cm, a8)); // FMT_PARGB without overshoot
            let over = s.cmpgts16(aa, pone);
            if argb || s.movemask8(over) != 0 {
                let den = s.maxs16(aa, one);
                let lo = s.divf(s.mulf(s.cvt_i32_f32(s.unpacklo16(c, z)), f255), s.cvt_i32_f32(s.unpacklo16(den, z)));
                let hi = s.divf(s.mulf(s.cvt_i32_f32(s.unpackhi16(c, z)), f255), s.cvt_i32_f32(s.unpackhi16(den, z)));
                let st = s.packs32(s.cvtr_f32_i32(lo), s.cvtr_f32_i32(hi)); // straight colours
                if argb {
                    res = s.or(s.and(st, cm), s.andnot(cm, a8));
                } else {
                    let m = s.and(over, cm); // overshooting pixels: straight colour, alpha 255
                    res = s.or(s.and(st, m), s.andnot(m, res));
                }
            }
            s.store_px(&mut dst[i * 4..], res);
            i += S::PX;
        }
        while i < n {
            let p = from_work_px(&w[i * 4..i * 4 + 4], fmt);
            dst[i * 4..i * 4 + 4].copy_from_slice(&p.to_le_bytes());
            i += 1;
        }
    }
}

fn from_work_gamma(dst: &mut [u8], w: &[i16], n: usize, fmt: u8) {
    for (o, p) in dst[..n * 4].chunks_exact_mut(4).zip(w.chunks_exact(4)) {
        let al = p[3] as i32;
        let mut out = 0u32;
        if al > 0 {
            let a8 = ((al.min(RS_PONE) + 32) >> 6) as u32;
            out = a8 << 24;
            for c in 0..3 {
                let v = (p[c] as i32).max(0).min(al) as u32;
                let idx = ((v * 8190 + al as u32) / (2 * al as u32)).min(4095);
                let mut sv = SRGB8[idx as usize] as u32;
                if fmt == FMT_PARGB {
                    sv = ((sv * a8 + 128) * 257) >> 16;
                }
                out |= sv << (c * 8);
            }
        }
        o.copy_from_slice(&out.to_le_bytes());
    }
}

/// n working pixels (4 x i16, any values) to an 8-bit row of FMT_ARGB / FMT_PARGB (module
/// header).  Reads w[..n * 4], writes dst[..n * 4].  [rs_from_work]
pub fn from_work(dst: &mut [u8], w: &[i16], n: usize, fmt: u8, flags: u32) {
    if flags & RS_GAMMA != 0 {
        from_work_gamma(dst, w, n, fmt);
    } else {
        simd::dispatch(FromWorkK { dst, w, n, fmt });
    }
}

// pshufd / pshuflw / pshufhw 0xD8: the two pixels of a register interleaved channel by
// channel (c0 c0' c1 c1' c2 c2' c3 c3') -- the pmaddwd operand
#[inline(always)]
fn pair<S: Simd>(s: S, v: S::V) -> S::V {
    s.shuf16::<0xD8>(s.shuf32::<0xD8>(v))
}

// the pair row: q[i] = channels of P(i) and P(i + 1) interleaved for i < qn, P(j) = 0 for
// j >= n; from 8-bit pixels (no RS_GAMMA) or from a working row (RS_GAMMA: w[n..] is 0)
struct PairsK<'a> {
    q: &'a mut [i16],
    src: &'a [u8],
    work: Option<&'a [i16]>,
    n: usize,
    qn: usize,
    fmt: u8,
}

impl Kernel for PairsK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, _: S) {
        let x = Sse2; // pairs of neighbours: the 128-bit layout of the assembly
        let PairsK { q, src, work, n, qn, fmt } = self;
        if let Some(w) = work {
            for i in 0..qn {
                x.store_i16(&mut q[i * 8..], pair(x, x.load_i16(&w[i * 4..])));
            }
            return;
        }
        let argb = fmt == FMT_ARGB;
        let mut cur = conv2(x, src, 0, n, argb);
        let mut i = 0;
        while i < qn {
            let next = conv2(x, src, i + 2, n, argb);
            x.store_i16(&mut q[i * 8..], pair(x, cur));
            if i + 1 >= qn {
                break;
            }
            x.store_i16(&mut q[(i + 1) * 8..], pair(x, x.shufps::<0x4E>(cur, next)));
            cur = next;
            i += 2;
        }
    }
}

// RS_CONV2: working values of pixels j, j + 1 (0 beyond n)
#[inline(always)]
fn conv2(x: Sse2, src: &[u8], j: usize, n: usize, argb: bool) -> <Sse2 as Simd>::V {
    let v = if j + 2 <= n {
        x.load_px(&src[j * 4..])
    } else if j < n {
        x.widen_px(x.load_lane32(&src[j * 4..]))
    } else {
        return x.zero();
    };
    work_lanes(x, v, argb)
}

// one output of the horizontal pass: (sum of the np pmaddwd pairs + 8192) >> 14 as i32 lanes
#[inline(always)]
fn hsum(x: Sse2, q: &[i16], rec: &[u8], np: usize) -> <Sse2 as Simd>::V {
    let x0 = i32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]) as usize;
    // pair p = q entry x0 + 2p (taps 2p, 2p + 1); q has 8 spare entries for the last chunk
    let qs = &q[x0 * 8..x0 * 8 + np * 16];
    let mut acc = x.zero();
    for (qc, wc) in qs.chunks_exact(16).zip(rec[16..16 + np * 16].chunks_exact(16)) {
        acc = x.add32(acc, x.madd16(x.load_i16(qc), x.load(wc)));
    }
    x.sra32::<14>(x.add32(acc, x.splat32(8192)))
}

// horizontal pass: n output pixels (4 x i16 each) from the pair row q and n table records of
// rb bytes with np weight pairs; out has room for one more pixel
struct HPassK<'a> {
    out: &'a mut [i16],
    q: &'a [i16],
    tab: &'a [u8],
    n: usize,
    rb: usize,
    np: usize,
}

impl Kernel for HPassK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, _: S) {
        let x = Sse2;
        let HPassK { out, q, tab, n, rb, np } = self;
        let mut i = 0;
        while i < n {
            let v0 = hsum(x, q, &tab[i * rb..i * rb + 16 + np * 16], np);
            let v1 = if i + 1 < n { hsum(x, q, &tab[(i + 1) * rb..(i + 1) * rb + 16 + np * 16], np) } else { v0 };
            x.store_i16(&mut out[i * 4..], x.packs32(v0, v1));
            i += 2;
        }
    }
}

// vertical pass over the pairs of rows (bases[2k], bases[2k + 1]) of the ring with the
// weight pairs wts (u32 = w_2k | w_2k+1 << 16): n pixels rounded up to whole registers.
// VP_ADD: start from acc; VP_FINAL: round, shift, saturate into out, else store the sums
struct VPassK<'a> {
    out: &'a mut [i16],
    ring: &'a [i16],
    bases: &'a [usize],
    wts: &'a [u32],
    wbuf: &'a mut [i16],
    acc: &'a mut [i32],
    n: usize,
    mode: u32,
}

impl Kernel for VPassK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let VPassK { out, ring, bases, wts, wbuf, acc, n, mode } = self;
        let lanes = S::BYTES / 2; // i16 per register
        let dl = S::BYTES / 4; // i32 per register
        let len = n.div_ceil(S::PX) * S::PX * 4; // i16 of the whole registers of a row
        let np = wts.len();
        for (k, &w) in wts.iter().enumerate() {
            s.store_i16(&mut wbuf[k * lanes..], s.splat32(w));
        }
        let wbuf = &wbuf[..np * lanes];
        let rows: Vec<&[i16]> = bases.iter().map(|&b| &ring[b..b + len]).collect();
        let r = s.splat32(8192);
        let mut o = 0; // i16 offset of the first pixel of the register
        while o < len {
            let (mut lo, mut hi) = if mode & VP_ADD != 0 {
                (s.load_i32(&acc[o..]), s.load_i32(&acc[o + dl..]))
            } else {
                (s.zero(), s.zero())
            };
            for (pr, wv) in rows.chunks_exact(2).zip(wbuf.chunks_exact(lanes)) {
                let a = s.load_i16(&pr[0][o..o + lanes]);
                let b = s.load_i16(&pr[1][o..o + lanes]);
                let wv = s.load_i16(wv);
                lo = s.add32(lo, s.madd16(s.unpacklo16(a, b), wv));
                hi = s.add32(hi, s.madd16(s.unpackhi16(a, b), wv));
            }
            if mode & VP_FINAL != 0 {
                let v = s.packs32(s.sra32::<14>(s.add32(lo, r)), s.sra32::<14>(s.add32(hi, r)));
                s.store_i16(&mut out[o..], v);
            } else {
                s.store_i32(&mut acc[o..], lo);
                s.store_i32(&mut acc[o + dl..], hi);
            }
            o += lanes;
        }
    }
}

// ------------------------------------------------------------------ contexts

const RM_COPY: u8 = 0;
const RM_NEAREST: u8 = 1;
const RM_FILTER: u8 = 2;

/// a prepared resize of src into a destination of the measured size; read-only while
/// rows() runs (Sync)  [rs_prepare context]
pub struct Resize<'a> {
    src: &'a Surface,
    sw: usize,
    sh: usize,
    sfmt: u8,
    dw: usize,
    dh: i32,
    dfmt: u8,
    flags: u32,
    mode: u8,
    xidx: Vec<u32>, // nearest: source column per dst column
    hid: bool,      // horizontal axis is an identity
    nh: usize,
    nv: usize,
    hrb: usize,
    vrb: usize,
    htab: Vec<u8>,
    vtab: Vec<u8>,
    qn: usize,  // entries of the pair row = max(S_w, NH)
    k: usize,   // ring rows
    grp: usize, // vertical taps per group = min(K, NV)
}

#[inline]
fn fmt_ok(f: u8) -> bool {
    f == FMT_PARGB || f == FMT_ARGB
}

impl<'a> Resize<'a> {
    /// validate and build the weight tables: ERR_UNSUPPORTED (filter, formats), ERR_DIMS
    /// (sizes outside 1 .. 65535)  [rs_prepare]
    pub fn prepare(src: &'a Surface, dst: &Surface, filter: u32, flags: u32) -> Result<Resize<'a>, i32> {
        if filter > RS_ADAPTIVE || !fmt_ok(src.fmt) || !fmt_ok(dst.fmt) {
            return Err(ERR_UNSUPPORTED);
        }
        for v in [src.w, src.h, dst.w, dst.h] {
            if (v as u32).wrapping_sub(1) > RS_MAXDIM - 1 {
                return Err(ERR_DIMS);
            }
        }
        let mut c = Resize {
            src,
            sw: src.w as usize,
            sh: src.h as usize,
            sfmt: src.fmt,
            dw: dst.w as usize,
            dh: dst.h,
            dfmt: dst.fmt,
            flags,
            mode: RM_COPY,
            xidx: Vec::new(),
            hid: false,
            nh: 0,
            nv: 0,
            hrb: 0,
            vrb: 0,
            htab: Vec::new(),
            vtab: Vec::new(),
            qn: 0,
            k: 0,
            grp: 0,
        };
        if filter == RS_NEAREST {
            if c.sw != c.dw || c.sh != dst.h as usize {
                c.mode = RM_NEAREST;
                let (s, d2) = (c.sw as u64, 2 * c.dw as u64);
                c.xidx = (0..c.dw as u64).map(|x| ((2 * x + 1) * s / d2) as u32).collect();
            }
            return Ok(c);
        }
        let h = axis_weights(src.w, dst.w, filter)?;
        let v = axis_weights(src.h, dst.h, filter)?;
        if h.id && v.id {
            return Ok(c); // both axes identities: copy
        }
        c.mode = RM_FILTER;
        c.hid = h.id;
        (c.nh, c.hrb, c.htab) = (h.n as usize, h.rb as usize, h.tab);
        (c.nv, c.vrb, c.vtab) = (v.n as usize, v.rb as usize, v.tab);
        c.qn = c.sw.max(c.nh);
        let rs = ((c.dw as u64 * 8 + 16 + 63) & !63).max(64); // ring row stride of the assembly
        c.k = c.nv;
        if c.nv as u64 * rs > RS_RING_BUDGET {
            c.k = ((RS_RING_BUDGET / rs) as usize & !1).max(2);
        }
        c.grp = c.nv.min(c.k);
        Ok(c)
    }

    // source row r
    #[inline]
    fn src_row(&self, r: usize) -> &[u8] {
        let o = r * self.src.stride as usize;
        &self.src.px[o..o + self.sw * 4]
    }

    // a row of DW pixels from the source format to the destination format (plain copy when
    // they are equal)  [rs_rowconv]
    fn rowconv(&self, dst: &mut [u8], src: &[u8]) {
        if self.sfmt == self.dfmt {
            dst.copy_from_slice(src);
        } else if self.sfmt == FMT_ARGB {
            premul_row(dst, src);
        } else {
            unpremul_row(dst, src);
        }
    }

    /// dst rows y0 .. y1-1 (clamped to 0 .. dst h) into band, which must hold them  [rs_resize_rows]
    pub fn rows(&self, band: &mut Band, y0: i32, y1: i32) -> i32 {
        let y0 = y0.max(0);
        let y1 = y1.min(self.dh);
        if y0 >= y1 {
            return 0;
        }
        let n4 = self.dw * 4;
        match self.mode {
            RM_COPY => {
                for y in y0..y1 {
                    let d = &mut band.row(y)[..n4];
                    self.rowconv(d, self.src_row(y as usize));
                }
            }
            RM_NEAREST => {
                let (sh, d2) = (self.sh as u64, 2 * self.dh as u64);
                for y in y0..y1 {
                    let sy = ((2 * y as u64 + 1) * sh / d2) as usize;
                    let srow = &self.src.px[sy * self.src.stride as usize..];
                    let d = &mut band.row(y)[..n4];
                    for (o, &sx) in d.chunks_exact_mut(4).zip(&self.xidx) {
                        let s = sx as usize * 4;
                        o.copy_from_slice(&srow[s..s + 4]);
                    }
                    if self.sfmt != self.dfmt {
                        if self.sfmt == FMT_ARGB { premul_inplace(d) } else { unpremul_inplace(d) }
                    }
                }
            }
            _ => self.filter_rows(band, y0, y1),
        }
        0
    }

    // source row r through the horizontal pass into out (a ring row)  [rs_hrow]
    fn hrow(&self, out: &mut [i16], r: usize, w: &mut [i16], q: &mut [i16]) {
        let src = self.src_row(r);
        let gamma = self.flags & RS_GAMMA != 0;
        if self.hid {
            to_work(out, src, self.sw, self.sfmt, self.flags);
            return;
        }
        if gamma {
            to_work(w, src, self.sw, self.sfmt, self.flags);
            simd::dispatch(PairsK { q, src, work: Some(w), n: self.sw, qn: self.qn, fmt: self.sfmt });
        } else {
            simd::dispatch(PairsK { q, src, work: None, n: self.sw, qn: self.qn, fmt: self.sfmt });
        }
        simd::dispatch(HPassK { out, q, tab: &self.htab, n: self.dw, rb: self.hrb, np: self.nh / 2 });
    }

    fn filter_rows(&self, band: &mut Band, y0: i32, y1: i32) {
        let (k, grp, nv, dw) = (self.k, self.grp, self.nv, self.dw);
        let rsi = (dw + 8) * 4; // ring row: DW pixels + room for whole AVX2 registers
        let mut w = if self.flags & RS_GAMMA != 0 { vec![0i16; (self.qn + 2) * 4] } else { Vec::new() };
        let mut q = vec![0i16; self.qn * 8 + 16];
        let mut ring = vec![0i16; k * rsi];
        let mut tags = vec![-1i64; k];
        let mut out = vec![0i16; rsi];
        let mut acc = if k < nv { vec![0i32; rsi] } else { Vec::new() };
        let mut bases = vec![0usize; grp];
        let mut wts = vec![0u32; grp / 2];
        let mut wbuf = vec![0i16; grp / 2 * 16];
        let last = self.sh as i64 - 1;
        for y in y0..y1 {
            let vrec = &self.vtab[y as usize * self.vrb..(y as usize + 1) * self.vrb];
            let vy0 = i32::from_le_bytes([vrec[0], vrec[1], vrec[2], vrec[3]]) as i64;
            let mut g0 = 0;
            while g0 < nv {
                let cnt = (nv - g0).min(grp);
                for j in 0..cnt {
                    let row = (vy0 + (g0 + j) as i64).min(last);
                    let slot = (row as u64 % k as u64) as usize;
                    if tags[slot] != row {
                        tags[slot] = row;
                        self.hrow(&mut ring[slot * rsi..(slot + 1) * rsi], row as usize, &mut w, &mut q);
                    }
                    bases[j] = slot * rsi;
                }
                for (p, wt) in wts[..cnt / 2].iter_mut().enumerate() {
                    let o = 16 + (g0 / 2 + p) * 16;
                    *wt = u32::from_le_bytes([vrec[o], vrec[o + 1], vrec[o + 2], vrec[o + 3]]);
                }
                let mut mode = if g0 > 0 { VP_ADD } else { 0 };
                if g0 + cnt >= nv {
                    mode |= VP_FINAL;
                }
                simd::dispatch(VPassK {
                    out: &mut out,
                    ring: &ring,
                    bases: &bases[..cnt],
                    wts: &wts[..cnt / 2],
                    wbuf: &mut wbuf,
                    acc: &mut acc,
                    n: dw,
                    mode,
                });
                g0 += cnt;
            }
            from_work(&mut band.row(y)[..dw * 4], &out, dw, self.dfmt, self.flags);
        }
    }
}

/// resample src into dst (its size is the target size): prepare + every row  [rs_resize]
pub fn resize(src: &Surface, dst: &mut Surface, filter: u32, flags: u32) -> i32 {
    let c = match Resize::prepare(src, dst, filter, flags) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let h = dst.h;
    c.rows(&mut Band::whole(dst), 0, h)
}

// ------------------------------------------------------------------ tables

// round(16320 * srgb_to_linear(v / 255)), v = 0..255
static LIN16: [u16; 256] = [
    0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 66, 72, 78,
    85, 91, 99, 106, 114, 122, 131, 140, 149, 159, 169, 179, 190, 201, 212, 224,
    236, 248, 261, 274, 288, 302, 316, 331, 346, 362, 378, 394, 411, 428, 446, 464,
    482, 501, 521, 540, 560, 581, 602, 623, 645, 668, 691, 714, 737, 762, 786, 811,
    837, 863, 889, 916, 943, 971, 1000, 1028, 1058, 1087, 1118, 1148, 1179, 1211, 1243, 1276,
    1309, 1343, 1377, 1412, 1447, 1483, 1519, 1555, 1593, 1630, 1669, 1707, 1747, 1786, 1827, 1868,
    1909, 1951, 1993, 2036, 2080, 2124, 2168, 2214, 2259, 2305, 2352, 2399, 2447, 2496, 2545, 2594,
    2644, 2695, 2746, 2798, 2850, 2903, 2957, 3011, 3065, 3120, 3176, 3232, 3289, 3347, 3405, 3464,
    3523, 3583, 3643, 3704, 3766, 3828, 3891, 3954, 4018, 4083, 4148, 4214, 4280, 4347, 4415, 4483,
    4552, 4621, 4691, 4762, 4833, 4905, 4977, 5051, 5124, 5199, 5274, 5349, 5426, 5503, 5580, 5658,
    5737, 5816, 5897, 5977, 6059, 6141, 6223, 6307, 6390, 6475, 6560, 6646, 6733, 6820, 6908, 6996,
    7085, 7175, 7266, 7357, 7449, 7541, 7634, 7728, 7823, 7918, 8013, 8110, 8207, 8305, 8403, 8503,
    8603, 8703, 8804, 8906, 9009, 9112, 9216, 9321, 9426, 9532, 9639, 9746, 9854, 9963, 10073, 10183,
    10294, 10406, 10518, 10631, 10745, 10859, 10974, 11090, 11207, 11324, 11442, 11561, 11680, 11800, 11921, 12043,
    12165, 12288, 12412, 12536, 12661, 12787, 12914, 13041, 13169, 13298, 13428, 13558, 13689, 13821, 13953, 14087,
    14221, 14355, 14491, 14627, 14764, 14902, 15040, 15179, 15319, 15460, 15601, 15744, 15887, 16030, 16175, 16320,
];
// round(255 * linear_to_srgb(i / 4095)), i = 0..4095
static SRGB8: [u8; 4096] = [
    0, 1, 2, 2, 3, 4, 5, 6, 6, 7, 8, 9, 10, 10, 11, 12, 13, 13, 14, 15, 15, 16, 16, 17, 18, 18, 19, 19, 20, 20, 21, 21,
    22, 22, 23, 23, 23, 24, 24, 25, 25, 25, 26, 26, 27, 27, 27, 28, 28, 29, 29, 29, 30, 30, 30, 31, 31, 31, 32, 32, 32, 33, 33, 33,
    34, 34, 34, 34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 37, 38, 38, 38, 38, 39, 39, 39, 40, 40, 40, 40, 41, 41, 41, 41, 42, 42, 42,
    42, 43, 43, 43, 43, 43, 44, 44, 44, 44, 45, 45, 45, 45, 46, 46, 46, 46, 46, 47, 47, 47, 47, 48, 48, 48, 48, 48, 49, 49, 49, 49,
    49, 50, 50, 50, 50, 50, 51, 51, 51, 51, 51, 52, 52, 52, 52, 52, 53, 53, 53, 53, 53, 54, 54, 54, 54, 54, 55, 55, 55, 55, 55, 55,
    56, 56, 56, 56, 56, 57, 57, 57, 57, 57, 57, 58, 58, 58, 58, 58, 58, 59, 59, 59, 59, 59, 59, 60, 60, 60, 60, 60, 60, 61, 61, 61,
    61, 61, 61, 62, 62, 62, 62, 62, 62, 63, 63, 63, 63, 63, 63, 64, 64, 64, 64, 64, 64, 64, 65, 65, 65, 65, 65, 65, 66, 66, 66, 66,
    66, 66, 66, 67, 67, 67, 67, 67, 67, 67, 68, 68, 68, 68, 68, 68, 68, 69, 69, 69, 69, 69, 69, 69, 70, 70, 70, 70, 70, 70, 70, 71,
    71, 71, 71, 71, 71, 71, 72, 72, 72, 72, 72, 72, 72, 72, 73, 73, 73, 73, 73, 73, 73, 74, 74, 74, 74, 74, 74, 74, 74, 75, 75, 75,
    75, 75, 75, 75, 75, 76, 76, 76, 76, 76, 76, 76, 77, 77, 77, 77, 77, 77, 77, 77, 78, 78, 78, 78, 78, 78, 78, 78, 78, 79, 79, 79,
    79, 79, 79, 79, 79, 80, 80, 80, 80, 80, 80, 80, 80, 81, 81, 81, 81, 81, 81, 81, 81, 81, 82, 82, 82, 82, 82, 82, 82, 82, 83, 83,
    83, 83, 83, 83, 83, 83, 83, 84, 84, 84, 84, 84, 84, 84, 84, 84, 85, 85, 85, 85, 85, 85, 85, 85, 85, 86, 86, 86, 86, 86, 86, 86,
    86, 86, 87, 87, 87, 87, 87, 87, 87, 87, 87, 88, 88, 88, 88, 88, 88, 88, 88, 88, 88, 89, 89, 89, 89, 89, 89, 89, 89, 89, 90, 90,
    90, 90, 90, 90, 90, 90, 90, 90, 91, 91, 91, 91, 91, 91, 91, 91, 91, 91, 92, 92, 92, 92, 92, 92, 92, 92, 92, 92, 93, 93, 93, 93,
    93, 93, 93, 93, 93, 93, 94, 94, 94, 94, 94, 94, 94, 94, 94, 94, 95, 95, 95, 95, 95, 95, 95, 95, 95, 95, 96, 96, 96, 96, 96, 96,
    96, 96, 96, 96, 96, 97, 97, 97, 97, 97, 97, 97, 97, 97, 97, 98, 98, 98, 98, 98, 98, 98, 98, 98, 98, 98, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 100, 101, 101, 101, 101, 101, 101, 101, 101, 101, 101, 101, 102, 102, 102, 102, 102,
    102, 102, 102, 102, 102, 102, 103, 103, 103, 103, 103, 103, 103, 103, 103, 103, 103, 103, 104, 104, 104, 104, 104, 104, 104, 104, 104, 104, 104, 105, 105, 105,
    105, 105, 105, 105, 105, 105, 105, 105, 105, 106, 106, 106, 106, 106, 106, 106, 106, 106, 106, 106, 106, 107, 107, 107, 107, 107, 107, 107, 107, 107, 107, 107,
    107, 108, 108, 108, 108, 108, 108, 108, 108, 108, 108, 108, 108, 109, 109, 109, 109, 109, 109, 109, 109, 109, 109, 109, 109, 110, 110, 110, 110, 110, 110, 110,
    110, 110, 110, 110, 110, 111, 111, 111, 111, 111, 111, 111, 111, 111, 111, 111, 111, 111, 112, 112, 112, 112, 112, 112, 112, 112, 112, 112, 112, 112, 113, 113,
    113, 113, 113, 113, 113, 113, 113, 113, 113, 113, 113, 114, 114, 114, 114, 114, 114, 114, 114, 114, 114, 114, 114, 114, 115, 115, 115, 115, 115, 115, 115, 115,
    115, 115, 115, 115, 115, 116, 116, 116, 116, 116, 116, 116, 116, 116, 116, 116, 116, 116, 117, 117, 117, 117, 117, 117, 117, 117, 117, 117, 117, 117, 117, 117,
    118, 118, 118, 118, 118, 118, 118, 118, 118, 118, 118, 118, 118, 119, 119, 119, 119, 119, 119, 119, 119, 119, 119, 119, 119, 119, 119, 120, 120, 120, 120, 120,
    120, 120, 120, 120, 120, 120, 120, 120, 120, 121, 121, 121, 121, 121, 121, 121, 121, 121, 121, 121, 121, 121, 122, 122, 122, 122, 122, 122, 122, 122, 122, 122,
    122, 122, 122, 122, 122, 123, 123, 123, 123, 123, 123, 123, 123, 123, 123, 123, 123, 123, 123, 124, 124, 124, 124, 124, 124, 124, 124, 124, 124, 124, 124, 124,
    124, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 125, 126, 126, 126, 126, 126, 126, 126, 126, 126, 126, 126, 126, 126, 126, 127, 127,
    127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 128, 129, 129, 129, 129,
    129, 129, 129, 129, 129, 129, 129, 129, 129, 129, 129, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 130, 131, 131, 131, 131, 131, 131,
    131, 131, 131, 131, 131, 131, 131, 131, 131, 131, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 132, 133, 133, 133, 133, 133, 133, 133,
    133, 133, 133, 133, 133, 133, 133, 133, 133, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 134, 135, 135, 135, 135, 135, 135, 135,
    135, 135, 135, 135, 135, 135, 135, 135, 135, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 136, 137, 137, 137, 137, 137, 137, 137,
    137, 137, 137, 137, 137, 137, 137, 137, 137, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 138, 139, 139, 139, 139, 139, 139, 139,
    139, 139, 139, 139, 139, 139, 139, 139, 139, 139, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 140, 141, 141, 141, 141, 141,
    141, 141, 141, 141, 141, 141, 141, 141, 141, 141, 141, 141, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 142, 143, 143, 143,
    143, 143, 143, 143, 143, 143, 143, 143, 143, 143, 143, 143, 143, 143, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 144, 145,
    145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 145, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146, 146,
    146, 146, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 147, 148, 148, 148, 148, 148, 148, 148, 148, 148, 148, 148, 148,
    148, 148, 148, 148, 148, 148, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 149, 150, 150, 150, 150, 150, 150, 150, 150,
    150, 150, 150, 150, 150, 150, 150, 150, 150, 150, 150, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 151, 152, 152, 152,
    152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 152, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153, 153,
    153, 153, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 154, 155, 155, 155, 155, 155, 155, 155, 155, 155, 155, 155,
    155, 155, 155, 155, 155, 155, 155, 155, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 156, 157, 157, 157, 157,
    157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 157, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158, 158,
    158, 158, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 159, 160, 160, 160, 160, 160, 160, 160, 160, 160, 160,
    160, 160, 160, 160, 160, 160, 160, 160, 160, 160, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 161, 162, 162,
    162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 162, 163, 163, 163, 163, 163, 163, 163, 163, 163, 163, 163, 163, 163, 163,
    163, 163, 163, 163, 163, 163, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 164, 165, 165, 165, 165, 165,
    165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 165, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166, 166,
    166, 166, 166, 166, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 167, 168, 168, 168, 168, 168, 168, 168,
    168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 168, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169, 169,
    169, 169, 169, 169, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 170, 171, 171, 171, 171, 171, 171, 171,
    171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172, 172,
    172, 172, 172, 172, 172, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 173, 174, 174, 174, 174, 174,
    174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 174, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175, 175,
    175, 175, 175, 175, 175, 175, 175, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 176, 177, 177,
    177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 177, 178, 178, 178, 178, 178, 178, 178, 178, 178, 178, 178, 178,
    178, 178, 178, 178, 178, 178, 178, 178, 178, 178, 178, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179, 179,
    179, 179, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 180, 181, 181, 181, 181, 181, 181, 181,
    181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 181, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182, 182,
    182, 182, 182, 182, 182, 182, 182, 182, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 183, 184,
    184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 184, 185, 185, 185, 185, 185, 185, 185, 185, 185,
    185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 185, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186, 186,
    186, 186, 186, 186, 186, 186, 186, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187, 187,
    188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 188, 189, 189, 189, 189, 189, 189, 189, 189,
    189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 189, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 190,
    190, 190, 190, 190, 190, 190, 190, 190, 190, 190, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191, 191,
    191, 191, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 192, 193, 193, 193, 193,
    193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 193, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194,
    194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 194, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195, 195,
    195, 195, 195, 195, 195, 195, 195, 195, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196, 196,
    196, 196, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 197, 198, 198, 198, 198,
    198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 198, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199,
    199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 199, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200,
    200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 200, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201, 201,
    201, 201, 201, 201, 201, 201, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202, 202,
    202, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 203, 204, 204, 204, 204,
    204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 204, 205, 205, 205, 205, 205, 205, 205, 205, 205,
    205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 205, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206,
    206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 206, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 207,
    207, 207, 207, 207, 207, 207, 207, 207, 207, 207, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208, 208,
    208, 208, 208, 208, 208, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209, 209,
    209, 209, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 210, 211, 211,
    211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 211, 212, 212, 212, 212, 212, 212,
    212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 212, 213, 213, 213, 213, 213, 213, 213, 213, 213,
    213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 213, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214,
    214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 214, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215,
    215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 215, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216,
    216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 216, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217, 217,
    217, 217, 217, 217, 217, 217, 217, 217, 217, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218, 218,
    218, 218, 218, 218, 218, 218, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219, 219,
    219, 219, 219, 219, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220, 220,
    220, 220, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221, 221,
    221, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 222, 223,
    223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 223, 224, 224,
    224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 224, 225, 225, 225, 225,
    225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 225, 226, 226, 226, 226, 226,
    226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 226, 227, 227, 227, 227, 227, 227,
    227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 227, 228, 228, 228, 228, 228, 228,
    228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 228, 229, 229, 229, 229, 229, 229, 229,
    229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 229, 230, 230, 230, 230, 230, 230, 230,
    230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 230, 231, 231, 231, 231, 231, 231, 231,
    231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 231, 232, 232, 232, 232, 232, 232, 232,
    232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 232, 233, 233, 233, 233, 233, 233, 233,
    233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 233, 234, 234, 234, 234, 234, 234,
    234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 234, 235, 235, 235, 235, 235, 235,
    235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 235, 236, 236, 236, 236, 236,
    236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 236, 237, 237, 237, 237,
    237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 237, 238, 238, 238,
    238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 238, 239, 239,
    239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239, 239,
    240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240, 240,
    240, 240, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241, 241,
    241, 241, 241, 241, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242, 242,
    242, 242, 242, 242, 242, 242, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243, 243,
    243, 243, 243, 243, 243, 243, 243, 243, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 244,
    244, 244, 244, 244, 244, 244, 244, 244, 244, 244, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245,
    245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 245, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246,
    246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 246, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247,
    247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 247, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248,
    248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 248, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249,
    249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 249, 250, 250, 250, 250, 250, 250, 250,
    250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 250, 251, 251, 251,
    251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251, 251,
    251, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252, 252,
    252, 252, 252, 252, 252, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253, 253,
    253, 253, 253, 253, 253, 253, 253, 253, 253, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254,
    254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 254, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255,
];
