// blendmode -- paint.net layer blend modes on straight-alpha BGRA rows (FMT_ARGB)  [src/gfx/blendmode.s]
//
//   bm_row(dst, lhs, rhs, mode_op)              layer rhs over layer lhs
//   bm_cov_row(dst, lhs, rhs, cov, mode_op)     same, rhs alpha x A8 coverage (None: full)
//   bm_solid_row(dst, lhs, argb, cov, mode_op)  a solid colour through the coverage
//   *_inplace: dst = lhs;   bm_px: one pixel
//
// mode_op = mode | opacity << 8 (BM_*, unknown modes are BM_NORMAL).  lhs = bottom row,
// rhs = top row.  The arithmetic is paint.net's UserBlendOps, all integers 0..255:
//
//   INT_SCALE(a, b) = round(a*b / 255)
//   rhsA = INT_SCALE(INT_SCALE(rhs.A, opacity), cov);  y = INT_SCALE(lhs.A, 255 - rhsA)
//   x = INT_SCALE(lhs.A, rhsA);  z = rhsA - x;  totalA = y + rhsA (0 -> pixel 0)
//   c = floor((lhs.c*y + rhs.c*z + F(lhs.c, rhs.c)*x) / totalA),  A = totalA
//
// BM_OVERWRITE (tools): dst = round((lhs*(255 - m) + rhs*m) / 255) on all four channels,
// m = INT_SCALE(cov, opacity).  Pixels with coverage 0 keep lhs.  bm_px is the scalar
// reference; the row kernels are the assembly's SSE2 / AVX2 code (simd::Kernel), which
// computes exactly this per pixel: its fast paths are exact shortcuts and its f32 divisions
// are exact floor divisions in this range (numerators <= 65025, divisors 1..255).

use rt::fx::div255;
use simd::{Kernel, Simd};

pub const BM_NORMAL: u32 = 0;
pub const BM_MULTIPLY: u32 = 1;
pub const BM_ADDITIVE: u32 = 2;
pub const BM_COLORBURN: u32 = 3;
pub const BM_COLORDODGE: u32 = 4;
pub const BM_REFLECT: u32 = 5;
pub const BM_GLOW: u32 = 6;
pub const BM_OVERLAY: u32 = 7;
pub const BM_DIFFERENCE: u32 = 8;
pub const BM_NEGATION: u32 = 9;
pub const BM_LIGHTEN: u32 = 10;
pub const BM_DARKEN: u32 = 11;
pub const BM_SCREEN: u32 = 12;
pub const BM_XOR: u32 = 13;
pub const BM_OVERWRITE: u32 = 14;
pub const BM_COUNT: u32 = 15;
pub const BM_OPAC_SHIFT: u32 = 8;

#[inline]
fn iscale(a: u32, b: u32) -> u32 {
    div255(a * b)
}

#[inline]
fn ch(p: u32, k: u32) -> u32 {
    (p >> (k * 8)) & 255
}

// F of one channel (l = lhs, r = rhs)
#[inline]
fn f(mode: u32, l: u32, r: u32) -> u32 {
    match mode {
        BM_MULTIPLY => iscale(l, r),
        BM_ADDITIVE => (l + r).min(255),
        BM_COLORBURN => {
            if r == 0 {
                0
            } else {
                (255 - ((255 - l) * 255 / r).min(32767) as i32).max(0) as u32
            }
        }
        BM_COLORDODGE => {
            if r == 255 { 255 } else { (l * 255 / (255 - r)).min(255) }
        }
        BM_REFLECT => {
            if r == 255 { 255 } else { (l * l / (255 - r)).min(255) }
        }
        BM_GLOW => {
            if l == 255 { 255 } else { (r * r / (255 - l)).min(255) }
        }
        BM_OVERLAY => {
            if l < 128 { iscale(2 * l, r) } else { 255 - iscale(2 * (255 - l), 255 - r) }
        }
        BM_DIFFERENCE => l.abs_diff(r),
        BM_NEGATION => 255 - (255 - l as i32 - r as i32).unsigned_abs(),
        BM_LIGHTEN => l.max(r),
        BM_DARKEN => l.min(r),
        BM_SCREEN => r + l - iscale(r, l),
        BM_XOR => l ^ r,
        _ => r, // BM_NORMAL
    }
}

/// one pixel: rhs over lhs with coverage cov (0..255) and mode_op = mode | opacity << 8
pub fn bm_px(l: u32, r: u32, cov: u32, mode_op: u32) -> u32 {
    if cov == 0 {
        return l;
    }
    let mut mode = mode_op & 255;
    let op = (mode_op >> 8) & 255;
    if mode >= BM_COUNT {
        mode = BM_NORMAL;
    }
    if mode == BM_OVERWRITE {
        let m = iscale(cov, op);
        let mut out = 0;
        for k in 0..4 {
            out |= div255(ch(l, k) * (255 - m) + ch(r, k) * m) << (k * 8);
        }
        return out;
    }
    let ra = iscale(iscale(r >> 24, op), cov);
    let la = l >> 24;
    let y = iscale(255 - ra, la);
    let x = iscale(la, ra);
    let z = ra - x;
    let total = y + ra;
    let div = total.max(1);
    let mut out = total << 24;
    for k in 0..3 {
        let (lc, rc) = (ch(l, k), ch(r, k));
        let num = f(mode, lc, rc) * x + lc * y + rc * z;
        out |= (num / div).min(255) << (k * 8);
    }
    out
}

// ---- SIMD kernels (simd::Kernel: SSE2 2 pixels / AVX2 4 pixels per step, pixel lanes):
// the assembly's BM_GEN / F_* / BM_DIV macros operation by operation, and its fast paths per
// step (exact shortcuts of the formula, so the grouping of pixels changes no result).
// The loops follow the assembly's BM_FUNC / BM_FUNC_AVX (perf-blend): the alpha tests on
// the raw registers (AVX2: one vptest for ZF / CF; SSE2: the pixel pair as a u64), the
// coverage bytes as one word, one step body per mode (Normal: two, with and without a
// coverage row; in place or not is a per-step pointer choice).  Rows run in blocks of BPX
// pixels over chunk iterators (no index checks in the step loop); a solid colour / a missing
// coverage row is a block-long row; the last n mod PX pixels are one more block on copies.
// The packs (packssdw / packuswb_px) and ptest come from simd methods that keep them single
// instructions on AVX2.

// BM_DIV: floor(a / b) per u16 lane (b >= 1, a <= 65535), quotients saturated to 32767
#[inline(always)]
fn div<S: Simd>(s: S, a: S::V, b: S::V) -> S::V {
    let z = s.zero();
    let qlo = s.cvtt_f32_i32(s.divf(s.cvt_i32_f32(s.unpacklo16(a, z)), s.cvt_i32_f32(s.unpacklo16(b, z))));
    let qhi = s.cvtt_f32_i32(s.divf(s.cvt_i32_f32(s.unpackhi16(a, z)), s.cvt_i32_f32(s.unpackhi16(b, z))));
    s.packssdw(qlo, qhi)
}

// the lane constants of the kernels
#[derive(Clone, Copy)]
struct C<V> {
    z: V,
    w1: V,
    w128: V,
    w255: V,
    w257: V,
    rgbw: V,  // B G R lanes
    aw: V,    // A lane
    aw255: V, // A lane = 255
    op: V,    // opacity
    am: V,    // the alpha bytes of raw pixels
}

impl<V: Copy> C<V> {
    #[inline(always)]
    fn new<S: Simd<V = V>>(s: S, op: u32) -> C<V> {
        C {
            z: s.zero(),
            w1: s.splat16(1),
            w128: s.splat16(128),
            w255: s.splat16(255),
            w257: s.splat16(257),
            rgbw: s.splat64(0x0000_FFFF_FFFF_FFFF),
            aw: s.splat64(0xFFFF_0000_0000_0000),
            aw255: s.splat64(0x00FF_0000_0000_0000),
            op: s.splat16(op as u16),
            am: s.splat32(0xFF00_0000),
        }
    }
}

// ISCALE: (v + 128) * 257 >> 16 = round(v / 255)
#[inline(always)]
fn vscale<S: Simd>(s: S, c: &C<S::V>, v: S::V) -> S::V {
    s.mulhiu16(s.add16(v, c.w128), c.w257)
}

// F_*: the mode function of lhs lanes l and rhs lanes r
#[inline(always)]
fn fv<S: Simd, const M: u32>(s: S, c: &C<S::V>, l: S::V, r: S::V) -> S::V {
    match M {
        BM_MULTIPLY => vscale(s, c, s.mullo16(l, r)),
        BM_ADDITIVE => s.mins16(s.add16(l, r), c.w255),
        BM_COLORBURN => {
            let t = s.mullo16(s.sub16(c.w255, l), c.w255); // (255 - lhs) * 255
            let q = div(s, t, s.maxs16(r, c.w1));
            let v = s.maxs16(s.sub16(c.w255, q), c.z); // max(0, 255 - q)
            s.andnot(s.cmpeq16(r, c.z), v) // rhs == 0 -> 0
        }
        BM_COLORDODGE | BM_REFLECT | BM_GLOW => {
            let (t, d) = match M {
                BM_COLORDODGE => (s.mullo16(l, c.w255), r), // lhs * 255 / (255 - rhs)
                BM_REFLECT => (s.mullo16(l, l), r),         // lhs * lhs / (255 - rhs)
                _ => (s.mullo16(r, r), l),                  // rhs * rhs / (255 - lhs)
            };
            let q = s.mins16(div(s, t, s.maxs16(s.sub16(c.w255, d), c.w1)), c.w255);
            s.or(q, s.and(s.cmpeq16(d, c.w255), c.w255)) // divisor 0 -> 255
        }
        BM_OVERLAY => {
            let a = vscale(s, c, s.mullo16(s.add16(l, l), r)); // INT_SCALE(2 lhs, rhs)
            let il = s.sub16(c.w255, l);
            let b = vscale(s, c, s.mullo16(s.add16(il, il), s.sub16(c.w255, r)));
            let b = s.sub16(c.w255, b);
            let m = s.cmpgts16(c.w128, l); // lhs < 128
            s.or(s.and(a, m), s.andnot(m, b))
        }
        BM_DIFFERENCE => s.or(s.subus16(l, r), s.subus16(r, l)),
        BM_NEGATION => {
            let t = s.sub16(s.sub16(c.w255, l), r); // 255 - lhs - rhs (signed)
            let a = s.maxs16(t, s.sub16(c.z, t));
            s.sub16(c.w255, a)
        }
        BM_LIGHTEN => s.maxs16(l, r),
        BM_DARKEN => s.mins16(l, r),
        BM_SCREEN => s.sub16(s.add16(l, r), vscale(s, c, s.mullo16(l, r))),
        BM_XOR => s.xor(l, r),
        _ => r,
    }
}

// BM_GEN: the general formula (coverage lanes cv; coverage-0 pixels keep lhs)
#[inline(always)]
fn general<S: Simd, const M: u32>(s: S, c: &C<S::V>, l: S::V, r: S::V, cv: S::V) -> S::V {
    let ra = vscale(s, c, s.mullo16(s.shuf16::<0xFF>(r), c.op)); // rhs.A x opacity
    let ra = vscale(s, c, s.mullo16(ra, cv)); // x coverage
    let la = s.shuf16::<0xFF>(l);
    let y = vscale(s, c, s.mullo16(s.sub16(c.w255, ra), la));
    let x = vscale(s, c, s.mullo16(la, ra));
    let z = s.sub16(ra, x);
    let tot = s.add16(y, ra);
    let f = fv::<S, M>(s, c, l, r);
    let num = s.add16(s.add16(s.mullo16(f, x), s.mullo16(l, y)), s.mullo16(r, z));
    let q = div(s, num, s.maxs16(tot, c.w1));
    let v = s.or(s.and(q, c.rgbw), s.and(tot, c.aw));
    let keep = s.cmpeq16(cv, c.z);
    s.or(s.and(l, keep), s.andnot(keep, v))
}

// raw pixels with alpha 0 set to 0 (psrld 24, pcmpeqd 0, pandn)
#[inline(always)]
fn zero_clear<S: Simd>(s: S, c: &C<S::V>, v: S::V) -> S::V {
    s.andnot(s.cmpeq32(s.srl32::<24>(v), c.z), v)
}

// the alpha bytes of a pixel pair as a u64
const AM2: u64 = 0xFF00_0000_FF00_0000;

// SSE2: the pixel pair of b as a u64 (the alpha tests run on it, as in the assembly); AVX2: 0
// (unused, the tests are ptest on the register)
#[inline(always)]
fn pair<S: Simd>(b: &[u8]) -> u64 {
    if S::PX == 2 { u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) } else { 0 }
}

// (every alpha 0, every alpha 255) of the PX raw pixels v (SSE2: their u64 w; AVX2: one
// vptest, ZF / CF as in the assembly)
#[inline(always)]
fn alphas<S: Simd>(s: S, c: &C<S::V>, v: S::V, w: u64) -> (bool, bool) {
    if S::PX == 2 {
        let a = w & AM2;
        (a == 0, a == AM2)
    } else {
        let t = s.ptest_raw(v, c.am);
        (t == 1, t == 2)
    }
}

// the PX coverage bytes of b as one word (byte i = pixel i)
#[inline(always)]
fn cov_word<S: Simd>(b: &[u8]) -> u32 {
    if S::PX == 2 {
        u16::from_le_bytes(*b.first_chunk::<2>().unwrap()) as u32
    } else {
        u32::from_le_bytes(*b.first_chunk::<4>().unwrap())
    }
}

// a zero byte among the PX coverage bytes of cw
#[inline(always)]
fn zero_byte<S: Simd>(cw: u32) -> bool {
    let lo: u32 = if S::PX == 2 { 0x0101 } else { 0x0101_0101 };
    cw.wrapping_sub(lo) & !cw & (lo << 7) != 0
}

// one step of PX pixels: raw lhs register lr (SSE2: its u64 lw), the rhs bytes rb, the
// coverage word cw -> out (4 * PX bytes); full = the coverage word of "full coverage and
// opacity 255" (0 when the opacity is lower: cw = 0 has returned before)  [BM_FUNC /
// BM_FUNC_AVX]
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn step<S: Simd, const M: u32>(s: S, c: &C<S::V>, full: u32, lr: S::V, lw: u64, rb: &[u8], cw: u32, out: &mut [u8]) {
    if cw == 0 {
        s.store_raw(out, lr); // no coverage: lhs unchanged
        return;
    }
    let rr = s.load_raw(rb);
    if M == BM_OVERWRITE {
        // dst = round((lhs * (255 - m) + rhs * m) / 255), m = INT_SCALE(cov, opacity)
        let m = vscale(s, c, s.mullo16(s.bcast_px(&cw.to_le_bytes()), c.op));
        let v = s.add16(s.mullo16(s.widen_px(lr), s.sub16(c.w255, m)), s.mullo16(s.widen_px(rr), m));
        s.store_raw(out, s.packuswb_px(vscale(s, c, v)));
        return;
    }
    let (rz, rf) = alphas(s, c, rr, pair::<S>(rb));
    if rz {
        if !zero_byte::<S>(cw) {
            s.store_raw(out, zero_clear(s, c, lr)); // transparent rhs: lhs, lhs.A = 0 pixels -> 0
            return;
        }
    } else if cw == full {
        // Normal needs no lhs opacity: a ZF-only test
        let (lz, lf) = if M == BM_NORMAL && S::PX != 2 { (s.ptestz_raw(lr, c.am), false) } else { alphas(s, c, lr, lw) };
        if lz {
            s.store_raw(out, zero_clear(s, c, rr)); // transparent lhs: rhs, rhs.A = 0 pixels -> 0
            return;
        }
        if rf {
            if M == BM_NORMAL {
                s.store_raw(out, rr); // opaque rhs: rhs
                return;
            }
            if lf {
                // both opaque: y = 0, x = 255, z = 0, totalA = 255 -> F, alpha 255
                let f = fv::<S, M>(s, c, s.widen_px(lr), s.widen_px(rr));
                s.store_raw(out, s.packuswb_px(s.or(s.and(f, c.rgbw), c.aw255)));
                return;
            }
        }
    }
    let v = general::<S, M>(s, c, s.widen_px(lr), s.widen_px(rr), s.bcast_px(&cw.to_le_bytes()));
    s.store_raw(out, s.packuswb_px(v));
}

// the top layer of a row kernel: a row or one colour
#[derive(Clone, Copy)]
enum Top<'a> {
    Row(&'a [u8]),
    Solid(u32),
}

struct BmK<'a> {
    out: &'a mut [u8],
    lhs: Option<&'a [u8]>,
    top: Top<'a>,
    cov: Option<&'a [u8]>,
    n: usize,
    mode_op: u32,
}

// pixels per block of the row loop (the full coverage row is this long)
const BPX: usize = 256;
// pixels per block with a solid colour (its row is filled per call: kept short)
const SPX: usize = 64;

// the coverage row of "no coverage row"
static FULL_COV: [u8; BPX] = [255; BPX];

// one block: out (a multiple of 4 * PX bytes) from lhs (None: out itself), rhs and the
// coverage, all of the same pixel count; one step per PX pixels (chunk iterators: no index
// checks in the loop)
#[inline(always)]
fn block<S: Simd, const M: u32, const COV: bool>(s: S, c: &C<S::V>, full: u32, out: &mut [u8], lhs: Option<&[u8]>, rhs: &[u8], cov: &[u8]) {
    let px = S::PX;
    let nb = 4 * px;
    let ip = lhs.is_none();
    let lhs = match lhs {
        Some(l) => l,
        None => rhs, // not read
    };
    let it = out.chunks_exact_mut(nb).zip(lhs.chunks_exact(nb)).zip(rhs.chunks_exact(nb)).zip(cov.chunks_exact(px));
    for (((ob, lb), rb), cb) in it {
        let cw = if COV { cov_word::<S>(cb) } else if S::PX == 2 { 0xFFFF } else { 0xFFFF_FFFF };
        let src: &[u8] = if ip { &*ob } else { lb };
        let (lr, lw) = (s.load_raw(src), pair::<S>(src));
        step::<S, M>(s, c, full, lr, lw, rb, cw, ob);
    }
}

// SPX pixels of the colour argb
#[inline(always)]
fn solid_row(argb: u32) -> [u8; 4 * SPX] {
    let mut r = [0u8; 4 * SPX];
    for p in r.chunks_exact_mut(4) {
        p.copy_from_slice(&argb.to_le_bytes());
    }
    r
}

// one mode: n pixels into out from lhs (None: out itself), the top and the coverage (None:
// 255), in blocks of BPX pixels (SPX with a solid colour); a solid colour and a missing
// coverage row are block-long rows (no per-call zeroing of large stack buffers: short rows
// are common, PaintCraft composes 64-pixel tile rows); the last n mod PX pixels are one more
// block of one step on copies (padding: zero pixels without coverage), so the step code
// exists once
#[inline(always)]
fn mode<S: Simd, const M: u32, const COV: bool>(s: S, k: BmK) {
    let BmK { out, lhs, top, cov, n, mode_op } = k;
    let op = (mode_op >> 8) & 255;
    let c = C::new(s, op);
    let px = S::PX;
    let nb = 4 * px;
    let full: u32 = if op != 255 { 0 } else if px == 2 { 0xFFFF } else { 0xFFFF_FFFF };
    let rbuf: [u8; 4 * SPX];
    let (srow, bpx): (&[u8], usize) = match top {
        Top::Row(_) => (&[], BPX),
        Top::Solid(a) => {
            rbuf = solid_row(a);
            (&rbuf, SPX)
        }
    };
    let full_px = n / px * px;
    let m = n - full_px;
    let (head, rest) = out.split_at_mut(4 * full_px);
    let rest = &mut rest[..4 * m];
    let (mut lb, mut ob, mut rb, mut cb) = ([0u8; 16], [0u8; 16], [0u8; 16], [0u8; 4]);
    if m > 0 {
        match lhs {
            Some(l) => lb[..4 * m].copy_from_slice(&l[4 * full_px..4 * n]),
            None => ob[..4 * m].copy_from_slice(rest),
        }
        match top {
            Top::Row(t) => rb[..4 * m].copy_from_slice(&t[4 * full_px..4 * n]),
            Top::Solid(_) => rb[..4 * m].copy_from_slice(&srow[..4 * m]),
        }
        match cov {
            Some(cv) => cb[..m].copy_from_slice(&cv[full_px..n]),
            None => cb[..m].fill(255),
        }
    }
    let mut x = 0;
    let mut tail = m > 0;
    loop {
        let (o_b, l_b, r_b, c_b): (&mut [u8], Option<&[u8]>, &[u8], &[u8]);
        if x < full_px {
            let b = (full_px - x).min(bpx);
            let (x4, e4) = (4 * x, 4 * (x + b));
            o_b = &mut head[x4..e4];
            l_b = match lhs {
                Some(l) => Some(&l[x4..e4]),
                None => None,
            };
            r_b = match top {
                Top::Row(t) => &t[x4..e4],
                Top::Solid(_) => &srow[..4 * b],
            };
            c_b = match cov {
                Some(cv) => &cv[x..x + b],
                None => &FULL_COV[..b],
            };
            x += b;
        } else if tail {
            tail = false;
            o_b = &mut ob[..nb];
            l_b = if lhs.is_some() { Some(&lb[..nb]) } else { None };
            r_b = &rb[..nb];
            c_b = &cb[..px];
        } else {
            break;
        }
        block::<S, M, COV>(s, &c, full, o_b, l_b, r_b, c_b);
    }
    rest.copy_from_slice(&ob[..4 * m]);
}

// a mode with a coverage row or without one (COV = false: the coverage word is a constant:
// Normal without a coverage row -- most layers in compositing, bm_row in effects -- runs
// 15-35 % faster than with a full-coverage row read every step; used for Normal only, every
// mode twice would cost 45 KB of code)
#[inline(always)]
fn mode2<S: Simd, const M: u32>(s: S, k: BmK) {
    if k.cov.is_some() {
        mode::<S, M, true>(s, k)
    } else {
        mode::<S, M, false>(s, k)
    }
}

// every mode of one register type (direct calls: a function pointer would compile mode
// outside the AVX2 context)
#[inline(always)]
fn modes<S: Simd>(s: S, k: BmK) {
    match k.mode_op & 255 {
        BM_MULTIPLY => mode::<S, BM_MULTIPLY, true>(s, k),
        BM_ADDITIVE => mode::<S, BM_ADDITIVE, true>(s, k),
        BM_COLORBURN => mode::<S, BM_COLORBURN, true>(s, k),
        BM_COLORDODGE => mode::<S, BM_COLORDODGE, true>(s, k),
        BM_REFLECT => mode::<S, BM_REFLECT, true>(s, k),
        BM_GLOW => mode::<S, BM_GLOW, true>(s, k),
        BM_OVERLAY => mode::<S, BM_OVERLAY, true>(s, k),
        BM_DIFFERENCE => mode::<S, BM_DIFFERENCE, true>(s, k),
        BM_NEGATION => mode::<S, BM_NEGATION, true>(s, k),
        BM_LIGHTEN => mode::<S, BM_LIGHTEN, true>(s, k),
        BM_DARKEN => mode::<S, BM_DARKEN, true>(s, k),
        BM_SCREEN => mode::<S, BM_SCREEN, true>(s, k),
        BM_XOR => mode::<S, BM_XOR, true>(s, k),
        BM_OVERWRITE => mode::<S, BM_OVERWRITE, true>(s, k),
        _ => mode2::<S, BM_NORMAL>(s, k),
    }
}

// the SSE2 instance as a function of its own (perf-blend): inlined into the dispatcher, it
// gave every call, also the AVX2 ones, its frame and the saves of xmm6-15 (Win64); short
// rows (64-pixel tile rows) paid that per call
#[inline(never)]
fn modes_sse2(k: BmK) {
    modes(simd::Sse2, k)
}

impl Kernel for BmK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        if S::PX == 2 {
            modes_sse2(self)
        } else {
            modes(s, self)
        }
    }
}

fn run(out: &mut [u8], lhs: Option<&[u8]>, top: Top, cov: Option<&[u8]>, n: usize, mode_op: u32) {
    simd::dispatch(BmK { out, lhs, top, cov, n, mode_op });
}

fn npx(a: &[u8], b: &[u8]) -> usize {
    a.len().min(b.len()) / 4
}

/// k_bm_row: dst = rhs over lhs (pixels = shortest row)
pub fn bm_row(dst: &mut [u8], lhs: &[u8], rhs: &[u8], mode_op: u32) {
    let n = npx(dst, lhs).min(rhs.len() / 4);
    run(dst, Some(lhs), Top::Row(rhs), None, n, mode_op);
}

pub fn bm_row_inplace(lhs: &mut [u8], rhs: &[u8], mode_op: u32) {
    let n = npx(lhs, rhs);
    run(lhs, None, Top::Row(rhs), None, n, mode_op);
}

/// k_bm_cov_row: rhs alpha scaled by the coverage row (None: full coverage); n pixels
pub fn bm_cov_row(dst: &mut [u8], lhs: &[u8], rhs: &[u8], cov: Option<&[u8]>, n: usize, mode_op: u32) {
    run(dst, Some(lhs), Top::Row(rhs), cov, n, mode_op);
}

pub fn bm_cov_row_inplace(lhs: &mut [u8], rhs: &[u8], cov: Option<&[u8]>, n: usize, mode_op: u32) {
    run(lhs, None, Top::Row(rhs), cov, n, mode_op);
}

/// k_bm_solid_row: the straight colour argb through the coverage row (None: full); n pixels
pub fn bm_solid_row(dst: &mut [u8], lhs: &[u8], argb: u32, cov: Option<&[u8]>, n: usize, mode_op: u32) {
    run(dst, Some(lhs), Top::Solid(argb), cov, n, mode_op);
}

pub fn bm_solid_row_inplace(lhs: &mut [u8], argb: u32, cov: Option<&[u8]>, n: usize, mode_op: u32) {
    run(lhs, None, Top::Solid(argb), cov, n, mode_op);
}
