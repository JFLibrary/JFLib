// enc -- BCn block encoders BC1..BC5, the surface driver (block rows in parallel on the
// thread pool) and the float "pixel set" helpers shared with bc7 / bc6h      [src/img/bc_enc.s]
//
//   encode_block(bcfmt, px, dst, cfg) -> 0 / ERR_UNSUPPORTED            bc_encode_block
//   encode_to(sf, bcfmt, cfg, dst, cancel) -> 0 / ERR_*                  bc_encode_to
//   encode(sf, bcfmt, cfg) -> Ok(blocks) / Err(ERR_*)                    bc_encode
//   encoded_size(w, h, bcfmt) -> bytes (0 = unknown format)              bc_encoded_size
//
// bcfmt = BCF_*; px = one 4x4 block as 16 straight BGRA pixels (64 bytes, row-major).  cfg =
// BCQ_*: level (fast / normal / slow, 0 = normal), BCQ_UNIFORM (BC1..BC3 colour error
// metric; default DirectXTex's perceptual channel scaling R 0.2125/0.7154, G 1, B
// 0.0721/0.7154), BC1 alpha threshold (bits 8..16 = threshold + 1, 0 = 128).
// Surfaces: any format get_row reads, converted to straight BGRA rows; blocks in raster order
// (ceil(w / 4) per block row), partial edge blocks padded by repeating the last column / row.
// Block rows run on the worker pool (plat::parallel_for): every block depends only on its
// pixels, so the output is identical for any thread count (SSE float arithmetic in a fixed
// order, no approximate reciprocals).  The cancel flag is polled once per block row.
//
// Encoders:
// * BC1 / BC2 / BC3 colour: pixels with alpha < threshold (BC1 only) are transparent
//   (3-colour mode, index 3); a single colour uses exact lookup tables; otherwise principal
//   axis -> endpoints -> 565 quantization -> nearest palette entry (exact decoder palette) ->
//   least-squares endpoint refinement (fast 1, normal 2, slow 4 iterations), for BC1 also the
//   3-colour mode (normal / slow), then a greedy +-1 search on the six endpoint fields (normal
//   1 round, slow 8).  4-colour blocks get c0 > c1 (c0 == c1: every index 0), 3-colour
//   blocks c0 <= c1.
// * BC2 alpha: round(a / 17).  BC3 alpha / BC4 / BC5: 8- and 6-value modes from min / max,
//   least squares, greedy +-1 search (normal 4 rounds, slow 16 after coarse grids and +-4
//   windows), exact decoder palettes.  BC4 encodes the luminance (7471 B + 38470 G + 19595
//   R) >> 16, BC5 R and G.  Signed BC4 / BC5: codes -127..127.
//
// Pixel set (Pxs, also used by bc7 / bc6h): up to 16 pixels of 4 float channels, SoA and AoS,
// lane mask and the block position of each lane; unused channels are zero.
//   load(px, aos)                       16 BGRA bytes -> f32[16][4] (R, G, B, A)  [bce_load]
//   Pxs::gather(aos, mask, perm)        pixels of the 16-bit mask; perm = rotation (0..3:
//                                       swap channel perm - 1 with 3) | zero mask << 4
//   pca(pxs) -> [E0, E1]                principal axis endpoints                  [bce_pca]
//   bcast(pal, k, scale, palb)          i32[k][4] palette -> f32 (x scale)       [bce_bcast]
//   assign(pxs, palb, k, idx) -> err    nearest entries (ties: lowest)          [bce_assign]
//   lsq(pxs, idx, wtab, out) -> bool    least-squares endpoints for weights wtab[idx] [bce_lsq]
// Like the assembly they work on SSE2 registers (simd::Sse2 directly: the assembly has no
// AVX2 path here), lane by lane in the assembly's operation order, so every float result is
// the same bit pattern; the BC4 byte search (bc4_eval) is the SSE2 byte search of the
// assembly.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_ARGB, Surface};
use img_core::meta::get_row;
use img_core::{ERR_CANCEL, ERR_DIMS, ERR_NOMEM, ERR_UNSUPPORTED};
use rt::fx::{cvt, maxss, minss};
use rt::sync::SpinLock;
use simd::{Simd, Sse2};

use crate::bc6h::{self, H6};
use crate::bc7::{self, B7};
use crate::dec::{T52, T53, T62, T63};
use crate::tables::{S3_5, S3_6, S4_5, S4_6};
use crate::{BCF_COUNT, BCQ_ATHR_SHIFT, BCQ_FAST, BCQ_LEVEL, BCQ_NORMAL, BCQ_SLOW, BCQ_UNIFORM};

const BCK_COLOR3: u32 = 1 << 30; // bc1_color: BC2 / BC3 colour block (4 colours, no alpha)

pub(crate) const INF: f32 = f32::INFINITY;
pub(crate) const TINY: f32 = 1e-20; // 0x1E3CE508
const EPS: f32 = 1e-5; // 0x3727C5AC

static Q565: [i32; 4] = [0x3DF8_F8F9, 0x3E7C_FCFD, 0x3DF8_F8F9, 0]; // 31/255, 63/255, 31/255, 0 (f32 bits)
static SC_PERC: [u32; 4] = [0x3E98_152F, 0x3F80_0000, 0x3DCE_6734, 0]; // 0.2125/0.7154, 1, 0.0721/0.7154
static ISC_PERC: [u32; 4] = [0x4057_762F, 0x3F80_0000, 0x411E_C1DD, 0]; // inverses
static SC_UNI: [u32; 4] = [0x3F80_0000, 0x3F80_0000, 0x3F80_0000, 0];
// BC1 least-squares weights per index: four colours (0, 1, 1/3, 2/3), three (0, 1, 1/2)
static C_W4: [u32; 4] = [0, 0x3F80_0000, 0x3EAA_AAAB, 0x3F2A_AAAB];
static C_W3: [u32; 4] = [0, 0x3F80_0000, 0x3F00_0000, 0];
// BC4 least-squares weights per index (-1: fixed code, not interpolated)
static C4_LW7: [u32; 8] = [0, 0x3F80_0000, 0x3E12_4925, 0x3E92_4925, 0x3EDB_6DB7, 0x3F12_4925, 0x3F36_DB6E, 0x3F5B_6DB7];
static C4_LW5: [u32; 8] = [0, 0x3F80_0000, 0x3E4C_CCCD, 0x3ECC_CCCD, 0x3F19_999A, 0x3F4C_CCCD, 0xBF80_0000, 0xBF80_0000];
const C_254_255: u32 = 0x3F7E_FEFF; // 254 / 255
// BC4 weight pairs (w0, w1) per code: 8-value mode / 6-value mode
use crate::dec::{BC4_W5, BC4_W7};
// greedy search neighbours (d0, d1)
static C4_NBD: [[i32; 2]; 8] = [[-1, 0], [1, 0], [0, -1], [0, 1], [-1, -1], [1, 1], [-1, 1], [1, -1]];
// BC1 endpoint fields (shift, max): c0 R G B, c1 R G B
static C1_FIELDS: [[u32; 2]; 6] = [[11, 31], [5, 63], [0, 31], [11, 31], [5, 63], [0, 31]];

/// a table of f32 bit patterns as floats (const)
pub(crate) const fn fbits<const N: usize>(b: [u32; N]) -> [f32; N] {
    let mut t = [0f32; N];
    let mut i = 0;
    while i < N {
        t[i] = f32::from_bits(b[i]);
        i += 1;
    }
    t
}

#[inline(always)]
pub(crate) fn f4(b: &[u32; 4]) -> [f32; 4] {
    [f32::from_bits(b[0]), f32::from_bits(b[1]), f32::from_bits(b[2]), f32::from_bits(b[3])]
}

// ------------------------------------------------------------------ pixel set

/// a pixel set (PXS): up to 16 pixels of 4 float channels
#[derive(Clone, Copy)]
pub(crate) struct Pxs {
    pub soa: [[f32; 16]; 4], // channel c, lane i
    pub aos: [[f32; 4]; 16], // lane i (R, G, B, A)
    pub mask: [i32; 16],     // lane mask (valid lanes all ones)
    pub pos: [u8; 16],       // block pixel of each lane
    pub n: u32,              // valid lanes (0..16)
    pub ng: u32,             // groups of four lanes
}

impl Pxs {
    pub const ZERO: Pxs = Pxs { soa: [[0.0; 16]; 4], aos: [[0.0; 4]; 16], mask: [0; 16], pos: [0; 16], n: 0, ng: 0 };

    /// the pixels of the 16-bit mask (block order); perm bits 0..1: rotation r (1..3 swaps
    /// channel r - 1 with 3), bits 4..7: channels set to zero; lanes up to the group end
    /// repeat lane 0 (mask 0)  [bce_gather]
    pub fn gather(&mut self, aos: &[[f32; 4]; 16], mask: u32, perm: u32) {
        let s = Sse2;
        let z = perm >> 4;
        let zm = [-((z & 1 == 0) as i32), -((z & 2 == 0) as i32), -((z & 4 == 0) as i32), -((z & 8 == 0) as i32)];
        let zm = s.load_i32(&zm);
        let r = perm & 3;
        let mut m = mask & 0xFFFF;
        let mut l = 0usize;
        while m != 0 && l < 16 {
            let b = m.trailing_zeros() as usize & 15;
            m &= m - 1;
            self.pos[l] = b as u8;
            // the lane after the rotation (swap channel r - 1 with 3) and the zeroing
            let v = s.load_f32(&aos[b]);
            let v = match r {
                0 => v,
                1 => s.shuf32::<0x27>(v),
                2 => s.shuf32::<0x6C>(v),
                _ => s.shuf32::<0xB4>(v),
            };
            s.store_f32(&mut self.aos[l], s.and(v, zm));
            self.mask[l] = -1;
            l += 1;
        }
        self.n = l as u32;
        let ng = (l + 3) >> 2;
        self.ng = ng as u32;
        // pad to the group end with lane 0
        let (p0, a0) = (self.pos[0], self.aos[0]);
        for _ in 0..3 {
            if l >= ng * 4 {
                break;
            }
            self.pos[l & 15] = p0;
            self.aos[l & 15] = a0;
            self.mask[l & 15] = 0;
            l += 1;
        }
        // SoA from AoS (4 x 4 transposes)
        for g in 0..ng.min(4) {
            let o = g * 4;
            let a = s.load_f32(&self.aos[o]);
            let b = s.load_f32(&self.aos[o + 1]);
            let c = s.load_f32(&self.aos[o + 2]);
            let d = s.load_f32(&self.aos[o + 3]);
            let t0 = s.shufps::<0x44>(a, b); // a0 a1 b0 b1
            let t2 = s.shufps::<0xEE>(a, b); // a2 a3 b2 b3
            let t1 = s.shufps::<0x44>(c, d);
            let t3 = s.shufps::<0xEE>(c, d);
            s.store_f32(&mut self.soa[0][o..o + 4], s.shufps::<0x88>(t0, t1));
            s.store_f32(&mut self.soa[1][o..o + 4], s.shufps::<0xDD>(t0, t1));
            s.store_f32(&mut self.soa[2][o..o + 4], s.shufps::<0x88>(t2, t3));
            s.store_f32(&mut self.soa[3][o..o + 4], s.shufps::<0xDD>(t2, t3));
        }
    }
}

// B G R A dwords -> R G B A floats
#[inline(always)]
fn f(x: V4) -> V4 {
    let s = Sse2;
    s.cvt_i32_f32(s.shuf32::<0xC6>(x))
}

/// 16 straight BGRA pixels -> f32[16][4] (R, G, B, A)  [bce_load]
#[inline]
pub(crate) fn load(px: &[u8; 64], aos: &mut [[f32; 4]; 16]) {
    let s = Sse2;
    let z = s.zero();
    for q in 0..4 {
        let v = s.load(&px[q * 16..q * 16 + 16]);
        let lo = s.unpacklo8(v, z);
        let hi = s.unpackhi8(v, z);
        s.store_f32(&mut aos[q * 4], f(s.unpacklo16(lo, z)));
        s.store_f32(&mut aos[q * 4 + 1], f(s.unpackhi16(lo, z)));
        s.store_f32(&mut aos[q * 4 + 2], f(s.unpacklo16(hi, z)));
        s.store_f32(&mut aos[q * 4 + 3], f(s.unpackhi16(hi, z)));
    }
}

/// endpoints along the principal axis: mean + tmin v, mean + tmax v; v from 8 power
/// iterations on the covariance (start: the column with the largest variance); one colour
/// or no pixels: both = the mean  [bce_pca]
pub(crate) fn pca(p: &Pxs) -> [[f32; 4]; 2] {
    let s = Sse2;
    let n = (p.n as usize).min(16);
    if n == 0 {
        return [[0.0; 4]; 2];
    }
    let mut mean = s.zero();
    for a in &p.aos[..n] {
        mean = s.addf(mean, s.load_f32(a));
    }
    mean = s.divf(mean, s.splatf(n as f32));
    let mut col = [s.zero(); 4];
    for a in &p.aos[..n] {
        let d = s.subf(s.load_f32(a), mean);
        col[0] = s.addf(col[0], s.mulf(s.shuf32::<0x00>(d), d));
        col[1] = s.addf(col[1], s.mulf(s.shuf32::<0x55>(d), d));
        col[2] = s.addf(col[2], s.mulf(s.shuf32::<0xAA>(d), d));
        col[3] = s.addf(col[3], s.mulf(s.shuf32::<0xFF>(d), d));
    }
    // the column with the largest diagonal element
    let diag = [lane::<0x00>(col[0]), lane::<0x55>(col[1]), lane::<0xAA>(col[2]), lane::<0xFF>(col[3])];
    let mut dmax = diag[0];
    let mut ci = 0;
    for (k, &d) in diag.iter().enumerate().skip(1) {
        if d > dmax {
            dmax = d;
            ci = k;
        }
    }
    let mut e = [[0f32; 4]; 2];
    if !(dmax > 0.0) {
        s.store_f32(&mut e[0], mean);
        s.store_f32(&mut e[1], mean);
        return e;
    }
    let mut v = col[ci];
    for _ in 0..8 {
        let w = matv(&col, v);
        let m = vmax(s.and(w, s.splat32(0x7FFF_FFFF)));
        if !(lane::<0>(m) > TINY) {
            break;
        }
        v = s.divf(w, m);
    }
    // unit length
    v = s.divf(v, s.sqrtf(vsum(s.mulf(v, v))));
    // projections
    let (mut tmin, mut tmax) = (INF, -INF);
    for a in &p.aos[..n] {
        let t = lane::<0>(vsum(s.mulf(s.subf(s.load_f32(a), mean), v)));
        tmin = minss(tmin, t);
        tmax = maxss(tmax, t);
    }
    s.store_f32(&mut e[0], s.addf(s.mulf(s.splatf(tmin), v), mean));
    s.store_f32(&mut e[1], s.addf(s.mulf(s.splatf(tmax), v), mean));
    e
}

pub(crate) type V4 = <Sse2 as Simd>::V;

/// lane k (shuf32 immediate k * 0x55) of a float register
#[inline(always)]
pub(crate) fn lane<const IMM: i32>(v: V4) -> f32 {
    let s = Sse2;
    f32::from_bits(s.to_i32(s.shuf32::<IMM>(v)) as u32)
}

/// x + shuf(x, 0x4E), then + shuf(., 0xB1): every lane the sum of the four
#[inline(always)]
pub(crate) fn vsum(x: V4) -> V4 {
    let s = Sse2;
    let x = s.addf(x, s.shuf32::<0x4E>(x));
    s.addf(x, s.shuf32::<0xB1>(x))
}

/// maxps(x, shuf(x, 0x4E)), then maxps(., shuf(., 0xB1))
#[inline(always)]
pub(crate) fn vmax(x: V4) -> V4 {
    let s = Sse2;
    let x = s.maxf(x, s.shuf32::<0x4E>(x));
    s.maxf(x, s.shuf32::<0xB1>(x))
}

/// C v: ((v0 C0 + v1 C1) + v2 C2) + v3 C3, lane-wise
#[inline(always)]
pub(crate) fn matv(col: &[V4; 4], v: V4) -> V4 {
    let s = Sse2;
    let w = s.addf(s.mulf(s.shuf32::<0x00>(v), col[0]), s.mulf(s.shuf32::<0x55>(v), col[1]));
    let w = s.addf(w, s.mulf(s.shuf32::<0xAA>(v), col[2]));
    s.addf(w, s.mulf(s.shuf32::<0xFF>(v), col[3]))
}

/// a palette with every channel broadcast: entry j, channel c, 4 lanes (PALB)
pub(crate) type PalB = [[[f32; 4]; 4]; 16];
pub(crate) const PALB0: PalB = [[[0.0; 4]; 4]; 16];

/// k entries i32[4] (R, G, B, A) -> floats (times scale), every channel broadcast
/// [bce_bcast]
#[inline]
pub(crate) fn bcast(pal: &[[i32; 4]], k: usize, scale: Option<&[f32; 4]>, palb: &mut PalB) {
    let s = Sse2;
    for j in 0..k.min(16) {
        let mut f = s.cvt_i32_f32(s.load_i32(&pal[j]));
        if let Some(sc) = scale {
            f = s.mulf(f, s.load_f32(sc));
        }
        bput(&mut palb[j], f);
    }
}

/// one palette entry (4 floats) with every channel broadcast
#[inline(always)]
pub(crate) fn bput(e: &mut [[f32; 4]; 4], f: V4) {
    let s = Sse2;
    s.store_f32(&mut e[0], s.shuf32::<0x00>(f));
    s.store_f32(&mut e[1], s.shuf32::<0x55>(f));
    s.store_f32(&mut e[2], s.shuf32::<0xAA>(f));
    s.store_f32(&mut e[3], s.shuf32::<0xFF>(f));
}

/// the nearest of k palette entries (4 channels, squared distance; ties: lowest index) for
/// every lane, indices to idx (ng * 4 lanes), the sum of the valid lanes' errors (SSE2 as
/// the assembly: four lanes per step)  [bce_assign]
#[inline]
pub(crate) fn assign(p: &Pxs, palb: &PalB, k: usize, idx: &mut [u8; 16]) -> f32 {
    let s = Sse2;
    let one = s.splat32(1);
    let mut acc = s.zero();
    for g in 0..(p.ng as usize).min(4) {
        let o = g * 4;
        let x0 = s.load_f32(&p.soa[0][o..o + 4]);
        let x1 = s.load_f32(&p.soa[1][o..o + 4]);
        let x2 = s.load_f32(&p.soa[2][o..o + 4]);
        let x3 = s.load_f32(&p.soa[3][o..o + 4]);
        let mut best = s.splatf(INF);
        let mut bi = s.zero();
        let mut cur = s.zero();
        for e in &palb[..k] {
            let d0 = s.subf(x0, s.load_f32(&e[0]));
            let mut d = s.mulf(d0, d0);
            let d1 = s.subf(x1, s.load_f32(&e[1]));
            d = s.addf(d, s.mulf(d1, d1));
            let d2 = s.subf(x2, s.load_f32(&e[2]));
            d = s.addf(d, s.mulf(d2, d2));
            let d3 = s.subf(x3, s.load_f32(&e[3]));
            d = s.addf(d, s.mulf(d3, d3)); // distance
            let better = s.cmpltf(d, best);
            best = s.minf(best, d);
            bi = s.or(s.and(better, cur), s.andnot(better, bi));
            cur = s.add32(cur, one);
        }
        // indices -> bytes
        let w = s.packs32(bi, bi);
        let b8 = s.packus16(w, w);
        idx[o..o + 4].copy_from_slice(&s.to_i32(b8).to_le_bytes());
        acc = s.addf(acc, s.and(best, s.load_i32(&p.mask[o..o + 4])));
    }
    let x = s.addf(s.shuf32::<0x4E>(acc), acc);
    let x = s.addf(x, s.shuf32::<0xB1>(x));
    f32::from_bits(s.to_i32(x) as u32)
}

/// least-squares endpoints A, B of min sum |(1 - w) A + w B - x|^2 with w = wtab[idx] over
/// the valid lanes -> false when singular  [bce_lsq]
pub(crate) fn lsq(p: &Pxs, idx: &[u8; 16], wtab: &[f32], out: &mut [[f32; 4]; 2]) -> bool {
    let s = Sse2;
    let (mut aa, mut ab, mut bb) = (0f32, 0f32, 0f32);
    let mut s7 = s.zero();
    let mut s8 = s.zero();
    for i in 0..(p.n as usize).min(16) {
        let w = wtab[(idx[i] & 15) as usize];
        let a = 1.0 - w;
        let x = s.load_f32(&p.aos[i]);
        s7 = s.addf(s7, s.mulf(x, s.splatf(a)));
        s8 = s.addf(s8, s.mulf(x, s.splatf(w)));
        aa += a * a;
        ab += a * w;
        bb += w * w;
    }
    let ab2 = ab * ab;
    let aabb = aa * bb;
    let det = aabb - ab2;
    if !(det > aabb * EPS) || !(det > 0.0) {
        return false;
    }
    let (vaa, vab, vbb, vdet) = (s.splatf(aa), s.splatf(ab), s.splatf(bb), s.splatf(det));
    s.store_f32(&mut out[0], s.divf(s.subf(s.mulf(vbb, s7), s.mulf(vab, s8)), vdet));
    s.store_f32(&mut out[1], s.divf(s.subf(s.mulf(vaa, s8), s.mulf(vab, s7)), vdet));
    true
}

// ------------------------------------------------------------------ scratch

/// BC1 colour scratch (B1_*)
#[derive(Clone, Copy)]
struct B1 {
    aos: [[f32; 4]; 16], // scaled pixels (R, G, B, 0)
    pxs: Pxs,
    pali: [[i32; 4]; 4], // integer palette (R, G, B, 0)
    palb: PalB,
    e: [[f32; 4]; 2],  // fit state
    pe: [[f32; 4]; 2], // principal axis
    idx: [u8; 16],     // current indices (lanes)
    bidx: [u8; 16],    // best indices (lanes)
    berr: f32,
    bc0: u32,
    bc1: u32,
    bm: u32, // best mode 4 / 3
    sc: [f32; 4],
    isc: [f32; 4],
    valid: u32, // opaque pixel mask
    trans: u32, // transparent pixel mask
    lvl: u32,
    kind: u32, // 0 = BC1, 1 = BC2 / BC3 colour
}

impl B1 {
    const ZERO: B1 = B1 {
        aos: [[0.0; 4]; 16],
        pxs: Pxs::ZERO,
        pali: [[0; 4]; 4],
        palb: PALB0,
        e: [[0.0; 4]; 2],
        pe: [[0.0; 4]; 2],
        idx: [0; 16],
        bidx: [0; 16],
        berr: 0.0,
        bc0: 0,
        bc1: 0,
        bm: 0,
        sc: [0.0; 4],
        isc: [0.0; 4],
        valid: 0,
        trans: 0,
        lvl: 0,
        kind: 0,
    };
}

/// BC4 channel scratch (C4_*)
#[derive(Clone, Copy)]
struct C4 {
    vals: [u8; 16], // target bytes
    pal: [u8; 8],
    idx: [u8; 16],
    bidx: [u8; 16], // best indices
    berr: u32,
    be0: i32,
    be1: i32,
    sgn: u32,
    lo: i32,
    hi: i32,
}

impl C4 {
    const ZERO: C4 = C4 { vals: [0; 16], pal: [0; 8], idx: [0; 16], bidx: [0; 16], berr: 0, be0: 0, be1: 0, sgn: 0, lo: 0, hi: 0 };
}

/// the scratch of one block encoder call (BCE_SCRATCH)
pub struct Scratch {
    b1: B1,
    c4: C4,
    pub(crate) b7: B7,
    pub(crate) h6: H6,
}

impl Scratch {
    pub fn new() -> Box<Scratch> {
        Box::new(Scratch { b1: B1::ZERO, c4: C4::ZERO, b7: B7::ZERO, h6: H6::ZERO })
    }
}

// ------------------------------------------------------------------ public API

static FMT_BYTES: [u8; 11] = [0, 8, 16, 16, 8, 16, 16, 16, 8, 16, 16];

/// bytes of ceil(w / 4) x ceil(h / 4) blocks (0 = unknown format)  [bc_encoded_size]
pub fn encoded_size(w: u32, h: u32, fmt: u32) -> u64 {
    if fmt.wrapping_sub(1) > BCF_COUNT - 2 {
        return 0;
    }
    ((w as u64 + 3) >> 2) * ((h as u64 + 3) >> 2) * FMT_BYTES[fmt as usize] as u64
}

// one block of a valid bcfmt (bce_fns)
fn enc(s: &mut Scratch, fmt: u32, px: &[u8; 64], dst: &mut [u8], cfg: u32) {
    match fmt {
        1 => bc1_color(s, px, dst, cfg & !BCK_COLOR3),
        2 => bc2_enc(s, px, dst, cfg),
        3 => bc3_enc(s, px, dst, cfg),
        4 => bc4_lum(s, px, dst, cfg & BCQ_LEVEL),
        5 => bc5_rg(s, px, dst, cfg & BCQ_LEVEL),
        6 => bc6h::enc(s, px, dst, cfg),
        7 => bc7::enc(s, px, dst, cfg),
        8 => bc4_lum(s, px, dst, cfg & BCQ_LEVEL | 256),
        9 => bc5_rg(s, px, dst, cfg & BCQ_LEVEL | 256),
        _ => bc6h::enc(s, px, dst, cfg | 1 << 31),
    }
}

/// encode one block (16 straight BGRA pixels) into 8 / 16 bytes at dst -> 0 /
/// ERR_UNSUPPORTED  [bc_encode_block]
pub fn encode_block(fmt: u32, px: &[u8; 64], dst: &mut [u8], cfg: u32) -> i32 {
    if fmt.wrapping_sub(1) > BCF_COUNT - 2 {
        return ERR_UNSUPPORTED;
    }
    let mut s = Scratch::new();
    enc(&mut s, fmt, px, dst, cfg);
    0
}

/// encode the whole surface into dst (encoded_size bytes, blocks in raster order) -> 0 /
/// ERR_DIMS / ERR_UNSUPPORTED / ERR_CANCEL  [bc_encode_to]
pub fn encode_to(sf: &Surface, fmt: u32, cfg: u32, dst: &mut [u8], cancel: Option<&AtomicU32>) -> i32 {
    if fmt.wrapping_sub(1) > BCF_COUNT - 2 {
        return ERR_UNSUPPORTED;
    }
    if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() {
        return ERR_DIMS;
    }
    let bs = FMT_BYTES[fmt as usize] as usize;
    let bw = ((sf.w as u32 + 3) >> 2) as usize;
    let bh = ((sf.h as u32 + 3) >> 2) as usize;
    let rows: Vec<SpinLock<&mut [u8]>> = dst.chunks_mut(bw * bs).take(bh).map(SpinLock::new).collect();
    let cancelled = |c: Option<&AtomicU32>| c.is_some_and(|f| f.load(Ordering::Relaxed) != 0);
    let item = |by: usize, job: &plat::JobCtl| {
        if cancelled(cancel) {
            job.cancel();
            return;
        }
        let mut g = rows[by].lock();
        enc_row(sf, fmt, cfg, by, bw, bs, &mut g);
    };
    let r = plat::parallel_for(bh, &item);
    if r.is_err() || cancelled(cancel) {
        return ERR_CANCEL;
    }
    0
}

// encode block row by  [bce_row]
fn enc_row(sf: &Surface, fmt: u32, cfg: u32, by: usize, bw: usize, bs: usize, out: &mut [u8]) {
    let w = sf.w as usize;
    let h = sf.h as usize;
    let mut rows = alloc::vec![0u8; 16 * w];
    for k in 0..4 {
        let y = (by * 4 + k).min(h - 1);
        get_row(sf, y, &mut rows[k * 4 * w..(k + 1) * 4 * w], FMT_ARGB);
    }
    let mut s = Scratch::new();
    let mut px = [0u8; 64];
    for bx in 0..bw {
        if bx * 4 + 4 <= w {
            for r in 0..4 {
                let o = (r * w + bx * 4) * 4;
                px[r * 16..r * 16 + 16].copy_from_slice(&rows[o..o + 16]);
            }
        } else {
            // partial edge block: repeat the last column
            for i in 0..16 {
                let x = (bx * 4 + (i & 3)).min(w - 1);
                let o = ((i >> 2) * w + x) * 4;
                px[i * 4..i * 4 + 4].copy_from_slice(&rows[o..o + 4]);
            }
        }
        enc(&mut s, fmt, &px, &mut out[bx * bs..bx * bs + bs], cfg);
    }
}

/// encode_to into a new buffer -> Ok(blocks) / Err(ERR_DIMS / ERR_UNSUPPORTED / ERR_NOMEM /
/// ERR_CANCEL)  [bc_encode]
pub fn encode(sf: &Surface, fmt: u32, cfg: u32) -> Result<Vec<u8>, i32> {
    if sf.w <= 0 || sf.h <= 0 {
        return Err(ERR_DIMS);
    }
    let n = encoded_size(sf.w as u32, sf.h as u32, fmt);
    if n == 0 {
        return Err(ERR_UNSUPPORTED);
    }
    let mut buf = Vec::new();
    if buf.try_reserve_exact(n as usize).is_err() {
        return Err(ERR_NOMEM);
    }
    buf.resize(n as usize, 0);
    let r = encode_to(sf, fmt, cfg, &mut buf, None);
    if r != 0 { Err(r) } else { Ok(buf) }
}

// ------------------------------------------------------------------ block encoders

// BC2: explicit alpha + four-colour block  [bc2_enc]
fn bc2_enc(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], cfg: u32) {
    let mut a = 0u64;
    for i in (0..16).rev() {
        let q = ((px[i * 4 + 3] as u32 + 8) * 3856) >> 16; // round(a / 17)
        a = a << 4 | q as u64;
    }
    dst[..8].copy_from_slice(&a.to_le_bytes());
    bc1_color(s, px, &mut dst[8..16], cfg | BCK_COLOR3);
}

// BC3: BC4-style alpha + four-colour block  [bc3_enc]
fn bc3_enc(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], cfg: u32) {
    let mut ch = [0u8; 16];
    for i in 0..16 {
        ch[i] = px[i * 4 + 3];
    }
    bc4_chan(&mut s.c4, &ch, &mut dst[..8], cfg & BCQ_LEVEL);
    bc1_color(s, px, &mut dst[8..16], cfg | BCK_COLOR3);
}

// BC4 of the luminance (arg = level | 256 signed)  [bc4_enc / bc4s_enc]
fn bc4_lum(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], arg: u32) {
    let mut ch = [0u8; 16];
    for i in 0..16 {
        let p = &px[i * 4..];
        ch[i] = ((p[0] as u32 * 7471 + p[1] as u32 * 38470 + p[2] as u32 * 19595) >> 16) as u8;
    }
    bc4_chan(&mut s.c4, &ch, &mut dst[..8], arg);
}

// BC5: red block, green block  [bc5_enc / bc5s_enc]
fn bc5_rg(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], arg: u32) {
    let mut ch = [0u8; 16];
    for i in 0..16 {
        ch[i] = px[i * 4 + 2];
    }
    bc4_chan(&mut s.c4, &ch, &mut dst[..8], arg);
    for i in 0..16 {
        ch[i] = px[i * 4 + 1];
    }
    bc4_chan(&mut s.c4, &ch, &mut dst[8..16], arg);
}

// ------------------------------------------------------------------ BC4 channel

// encode 16 bytes as one BC4 block (8 bytes at dst); arg = level (0 = normal) | 256 for
// signed codes  [bc4_chan]
fn bc4_chan(c: &mut C4, vals: &[u8; 16], dst: &mut [u8], arg: u32) {
    c.vals = *vals;
    c.berr = 0x7FFF_FFFF;
    c.be0 = 0;
    c.be1 = 0;
    c.sgn = arg >> 8 & 1;
    (c.lo, c.hi) = if c.sgn != 0 { (-127, 127) } else { (0, 255) };
    let mut lvl = arg & BCQ_LEVEL;
    if lvl == 0 {
        lvl = BCQ_NORMAL;
    }
    // min / max of all values and of the values other than 0 / 255
    let mn = *vals.iter().min().unwrap() as i32;
    let mx = *vals.iter().max().unwrap() as i32;
    let (mut mn6, mut mx6) = (255i32, 0i32);
    for &v in vals {
        if v != 0 && v != 255 {
            mn6 = mn6.min(v as i32);
            mx6 = mx6.max(v as i32);
        }
    }
    if mn6 > mx6 {
        mn6 = mn; // only 0 / 255 present
        mx6 = mn;
    }
    // candidate A: 8-value mode (max, min)
    let sg = c.sgn != 0;
    let (a0, a1) = (bc4_code(mx, sg), bc4_code(mn, sg));
    bc4_try(c, a0, a1);
    // candidate B: 6-value mode (min', max')
    let (b0, b1) = (bc4_code(mn6, sg), bc4_code(mx6, sg));
    bc4_try(c, b0, b1);
    if lvl != BCQ_FAST {
        // normal (and the start of slow): least squares from the best, greedy +-1 rounds
        bc4_lsq(c);
        bc4_greedy(c, 4);
        if lvl == BCQ_SLOW {
            // coarse grids around both starting points, a +-4 window around the best,
            // window + least squares while they gain, greedy rounds
            bc4_grid(c, a0, a1);
            bc4_grid(c, b0, b1);
            for _ in 0..4 {
                let e = c.berr;
                let (e0, e1) = (c.be0, c.be1);
                bc4_window(c, e0, e1);
                bc4_lsq(c);
                if c.berr >= e {
                    break;
                }
            }
            bc4_greedy(c, 16);
        }
    }
    dst[0] = c.be0 as u8;
    dst[1] = c.be1 as u8;
    let mut bits = 0u64;
    for i in (0..16).rev() {
        bits = bits << 3 | (c.bidx[i] & 7) as u64;
    }
    dst[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
}

// up to `rounds` rounds of bc4_try on the 8 neighbours (+-1) of the best codes, stopping
// when a round gains nothing  [bc4_greedy]
fn bc4_greedy(c: &mut C4, rounds: u32) {
    for _ in 0..rounds {
        let e = c.berr;
        let (b0, b1) = (c.be0, c.be1);
        for d in &C4_NBD {
            bc4_try(c, b0 + d[0], b1 + d[1]);
        }
        if c.berr >= e {
            break;
        }
    }
}

// bc4_try every code pair within +-4 of (e0, e1)  [bc4_window]
fn bc4_window(c: &mut C4, e0: i32, e1: i32) {
    for i in 0..9 {
        for j in 0..9 {
            bc4_try(c, e0 - 4 + i, e1 - 4 + j);
        }
    }
}

// bc4_try the code pairs (e0 + 8 i, e1 + 8 j), i, j = -5..5, then a +-4 window around the
// best pair of this grid  [bc4_grid]
fn bc4_grid(c: &mut C4, e0: i32, e1: i32) {
    let (mut gb, mut g0, mut g1) = (u32::MAX, e0, e1);
    for i in 0..11 {
        for j in 0..11 {
            let (x, y) = (e0 - 40 + 8 * i, e1 - 40 + 8 * j);
            let err = bc4_try(c, x, y);
            if err < gb {
                (gb, g0, g1) = (err, x, y);
            }
        }
    }
    bc4_window(c, g0, g1);
}

// code of the byte x: unsigned x, signed round(254 x / 255) - 127  [bc4_code]
#[inline]
fn bc4_code(x: i32, signed: bool) -> i32 {
    if !signed { x } else { ((x as u32 * 508 + 255) / 510) as i32 - 127 }
}

// evaluate the codes (clamped to the code range) and keep them when better  [bc4_try]
fn bc4_try(c: &mut C4, e0: i32, e1: i32) -> u32 {
    let e0 = e0.max(c.lo).min(c.hi);
    let e1 = e1.max(c.lo).min(c.hi);
    bc4_pal(e0, e1, c.sgn != 0, &mut c.pal);
    let err = bc4_eval(&c.vals, &c.pal, &mut c.idx);
    if err < c.berr {
        c.berr = err;
        c.be0 = e0;
        c.be1 = e1;
        c.bidx = c.idx;
    }
    err
}

// the 8 decoded bytes of a BC4 block with codes e0 / e1 (signed: -127..127), exactly as
// dec::bc4_vals  [bc4_pal]
#[inline]
fn bc4_pal(e0: i32, e1: i32, signed: bool, pal: &mut [u8; 8]) {
    let mut w = &BC4_W7;
    let mut d = 7;
    if !signed {
        if (e0 as u32) <= (e1 as u32) {
            w = &BC4_W5;
            d = 5;
        }
        for (k, o) in pal.iter_mut().enumerate() {
            let x = (w[2 * k] as i32 * e0 + w[2 * k + 1] as i32 * e1) as u32;
            *o = (if d == 7 { ((x + 3) * 9363) >> 16 } else { ((x + 2) * 13108) >> 16 }) as u8;
        }
    } else {
        if e0 <= e1 {
            w = &BC4_W5;
            d = 5;
        }
        let d127 = 127 * d;
        let magic: u64 = if d == 7 { 1_207_797 } else { 1_690_955 };
        for (k, o) in pal.iter_mut().enumerate() {
            let u = w[2 * k] as i32 * e0 + w[2 * k + 1] as i32 * e1 + d127; // 0 .. 254 D
            let x = (u * 510 + 2 * d127) as u32 as u64;
            *o = ((x * magic) >> 32) as u8;
        }
    }
    if d == 5 {
        pal[6] = 0;
        pal[7] = 255;
    }
}

// nearest of the 8 palette bytes for each of the 16 values (ties: lowest index), indices
// to idx, sum of squared differences (SSE2 byte search as the assembly)  [bc4_eval]
#[inline]
fn bc4_eval(vals: &[u8; 16], pal: &[u8; 8], idx: &mut [u8; 16]) -> u32 {
    let s = Sse2;
    let z = s.zero();
    let one = s.splat8(1);
    let x = s.load(vals);
    let mut best = s.splat8(255);
    let mut bi = z;
    let mut cur = z;
    for &p in pal {
        let v = s.splat8(p);
        let d = s.or(s.subus8(x, v), s.subus8(v, x)); // |x - v|
        let keep = s.cmpeq8(s.subus8(best, d), z); // best <= d: keep
        best = s.minu8(best, d);
        bi = s.or(s.and(bi, keep), s.andnot(keep, cur));
        cur = s.add8(cur, one);
    }
    s.store(idx, bi);
    let lo = s.unpacklo8(best, z);
    let hi = s.unpackhi8(best, z);
    let q = s.add32(s.madd16(lo, lo), s.madd16(hi, hi));
    let q = s.add32(q, s.shuf32::<0x4E>(q));
    let q = s.add32(q, s.shuf32::<0xB1>(q));
    s.to_i32(q) as u32
}

// least-squares codes for the best indices (in the code domain), tried with bc4_try
// [bc4_lsq]
fn bc4_lsq(c: &mut C4) {
    let lw = if c.be0 > c.be1 { &C4_LW7 } else { &C4_LW5 };
    let (mut aa, mut ab, mut bb, mut ax, mut bx) = (0f32, 0f32, 0f32, 0f32, 0f32);
    for i in 0..16 {
        let w = f32::from_bits(lw[(c.bidx[i] & 7) as usize]);
        if !(w >= 0.0) {
            continue; // fixed code: not interpolated
        }
        let mut t = c.vals[i] as f32;
        if c.sgn != 0 {
            t = t * f32::from_bits(C_254_255) - 127.0;
        }
        let a = 1.0 - w;
        aa += a * a;
        ab += a * w;
        bb += w * w;
        ax += a * t;
        bx += w * t;
    }
    let aabb = aa * bb;
    let det = aabb - ab * ab;
    if !(det > aabb * EPS) || !(det > 0.0) {
        return;
    }
    let mut a = (bb * ax - ab * bx) / det;
    let mut b = (aa * bx - ab * ax) / det;
    // clamp to +-512 before the conversion (NaN / huge -> bounded)
    a = maxss(a, -512.0);
    b = maxss(b, -512.0);
    a = minss(a, 512.0);
    b = minss(b, 512.0);
    bc4_try(c, cvt(a), cvt(b));
}

// ------------------------------------------------------------------ BC1 colour

// one BC1 colour block (8 bytes at dst); cfgk = cfg | BCK_COLOR3 for BC2 / BC3 (four colours,
// alpha ignored)  [bc1_color]
fn bc1_color(s: &mut Scratch, px: &[u8; 64], dst: &mut [u8], cfgk: u32) {
    let b = &mut s.b1;
    b.lvl = cfgk & BCQ_LEVEL;
    if b.lvl == 0 {
        b.lvl = BCQ_NORMAL;
    }
    b.kind = cfgk >> 30 & 1;
    let (sc, isc) = if cfgk & BCQ_UNIFORM != 0 { (&SC_UNI, &SC_UNI) } else { (&SC_PERC, &ISC_PERC) };
    b.sc = f4(sc);
    b.isc = f4(isc);
    // alpha threshold
    let mut thr = 0u32;
    if b.kind == 0 {
        let t = cfgk >> BCQ_ATHR_SHIFT & 0x1FF;
        thr = if t == 0 { 128 } else { t - 1 };
    }
    let mut valid = 0u32;
    for i in 0..16 {
        if px[i * 4 + 3] as u32 >= thr {
            valid |= 1 << i;
        }
    }
    b.valid = valid;
    b.trans = valid ^ 0xFFFF;
    if valid == 0 {
        // every pixel transparent
        dst[..4].copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
        dst[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        return;
    }
    // one colour?
    let rgb = |i: usize| u32::from_le_bytes([px[i * 4], px[i * 4 + 1], px[i * 4 + 2], 0]);
    let first = rgb(valid.trailing_zeros() as usize);
    let mut m = valid;
    let mut single = true;
    while m != 0 {
        let i = m.trailing_zeros() as usize;
        m &= m - 1;
        if rgb(i) != first {
            single = false;
            break;
        }
    }
    if single {
        bc1_single(b, first, dst);
        return;
    }
    // scaled float pixels, the opaque ones as a pixel set
    load(px, &mut b.aos);
    let sv = Sse2;
    let sc = sv.load_f32(&b.sc);
    for a in b.aos.iter_mut() {
        let v = sv.mulf(sv.load_f32(a), sc);
        sv.store_f32(a, v);
    }
    b.pxs.gather(&b.aos, valid, 0);
    b.berr = INF;
    b.bm = 4;
    b.pe = pca(&b.pxs);
    // iterations by level: fast 1, normal 2, slow 4
    let it = if b.lvl == BCQ_SLOW { 4 } else { b.lvl };
    if b.trans == 0 {
        bc1_fit(b, 4, it);
        if b.kind == 0 && b.lvl != BCQ_FAST {
            bc1_fit(b, 3, it);
        }
    } else {
        bc1_fit(b, 3, it);
    }
    // greedy endpoint search: normal 1 round, slow 8
    if b.lvl != BCQ_FAST {
        bc1_local(b, if b.lvl == BCQ_SLOW { 8 } else { 1 });
    }
    bc1_pack(b, dst);
}

// quantize / evaluate / least squares from the principal axis endpoints (mode 4 = four
// colours, 3 = three colours)  [bc1_fit]
fn bc1_fit(b: &mut B1, mode: u32, iters: u32) {
    b.e = b.pe;
    let w = f4(if mode == 4 { &C_W4 } else { &C_W3 });
    let mut i = 0;
    loop {
        let (c0, c1) = bc1_quant(b);
        bc1_eval(b, c0, c1, mode);
        if i >= iters {
            break;
        }
        let idx = b.idx;
        if !lsq(&b.pxs, &idx, &w, &mut b.e) {
            break;
        }
        i += 1;
    }
}

// the fit endpoints (scaled floats) as 565 colours  [bc1_quant]
fn bc1_quant(b: &B1) -> (u32, u32) {
    let s = Sse2;
    let isc = s.load_f32(&b.isc);
    let q565 = s.load_i32(&Q565);
    let (z, c255) = (s.zero(), s.splatf(255.0));
    let mut c = [0u32; 2];
    for k in 0..2 {
        let v = s.cvtr_f32_i32(s.mulf(s.minf(s.maxf(s.mulf(s.load_f32(&b.e[k]), isc), z), c255), q565));
        let mut q = [0i32; 4];
        s.store_i32(&mut q, v);
        c[k] = (q[0] as u32 & 0xFFFF) << 11 | (q[1] as u32 & 0xFFFF) << 5 | (q[2] as u32 & 0xFFFF);
    }
    (c[0], c[1])
}

// palette of the 565 colours (mode 4 / 3), nearest entries of the opaque pixels; keeps the
// best (error, colours, mode, indices)  [bc1_eval]
fn bc1_eval(b: &mut B1, c0: u32, c1: u32, mode: u32) {
    let (c0, c1) = (c0 & 0xFFFF, c1 & 0xFFFF);
    for (ch, &(sh, msk)) in [(11u32, 31u32), (5, 63), (0, 31)].iter().enumerate() {
        let a = (c0 >> sh & msk) as usize;
        let bb = (c1 >> sh & msk) as usize;
        let (t3, t2): (&[u8], &[u8]) = if msk == 31 { (&T53, &T52) } else { (&T63, &T62) };
        b.pali[0][ch] = t3[3 * a] as i32;
        b.pali[1][ch] = t3[3 * bb] as i32;
        if mode == 4 {
            b.pali[2][ch] = t3[2 * a + bb] as i32;
            b.pali[3][ch] = t3[a + 2 * bb] as i32;
        } else {
            b.pali[2][ch] = t2[a + bb] as i32;
        }
    }
    for p in b.pali.iter_mut() {
        p[3] = 0;
    }
    let k = mode as usize;
    let sc = b.sc;
    bcast(&b.pali, k, Some(&sc), &mut b.palb);
    let err = assign(&b.pxs, &b.palb, k, &mut b.idx);
    if !(err >= b.berr) {
        b.berr = err;
        b.bc0 = c0;
        b.bc1 = c1;
        b.bm = mode;
        b.bidx = b.idx;
    }
}

// greedy +-1 search on the six endpoint fields of the best colours  [bc1_local]
fn bc1_local(b: &mut B1, rounds: u32) {
    for _ in 0..rounds {
        let start = b.berr;
        for f in 0..12usize {
            let (mut c0, mut c1) = (b.bc0, b.bc1);
            let fi = f >> 1;
            let [sh, max] = C1_FIELDS[fi];
            let t = if fi < 3 { c0 } else { c1 };
            let mut v = t >> sh & max;
            if f & 1 == 0 {
                if v == 0 {
                    continue;
                }
                v -= 1;
            } else {
                if v >= max {
                    continue;
                }
                v += 1;
            }
            let t = (t & !(max << sh)) | v << sh;
            if fi < 3 {
                c0 = t;
            } else {
                c1 = t;
            }
            let bm = b.bm;
            bc1_eval(b, c0, c1, bm);
        }
        if b.berr >= start {
            break;
        }
    }
}

// write the best colours / indices (ordering rules of the modes)  [bc1_pack]
fn bc1_pack(b: &B1, dst: &mut [u8]) {
    let (mut c0, mut c1, m) = (b.bc0, b.bc1, b.bm);
    let mut idx = [3u8; 16];
    for l in 0..(b.pxs.n as usize).min(16) {
        idx[(b.pxs.pos[l] & 15) as usize] = b.bidx[l] & 3;
    }
    let valid = b.valid;
    if m == 4 {
        if c0 < c1 {
            core::mem::swap(&mut c0, &mut c1);
            for (i, x) in idx.iter_mut().enumerate() {
                if valid >> i & 1 != 0 {
                    *x ^= 1;
                }
            }
        } else if c0 == c1 {
            for (i, x) in idx.iter_mut().enumerate() {
                if valid >> i & 1 != 0 {
                    *x = 0;
                }
            }
        }
    } else if c0 > c1 {
        core::mem::swap(&mut c0, &mut c1);
        for (i, x) in idx.iter_mut().enumerate() {
            if valid >> i & 1 != 0 && *x < 2 {
                *x ^= 1;
            }
        }
    }
    dst[0..2].copy_from_slice(&(c0 as u16).to_le_bytes());
    dst[2..4].copy_from_slice(&(c1 as u16).to_le_bytes());
    let mut bits = 0u32;
    for i in (0..16).rev() {
        bits = bits << 2 | (idx[i] & 3) as u32;
    }
    dst[4..8].copy_from_slice(&bits.to_le_bytes());
}

// (a, b, |err|) per channel from the 5- / 6-bit tables -> c0, c1, weighted squared error
// [SGPAIR]
fn sgpair(b: &B1, t5: &[u8], t6: &[u8], bl: usize, g: usize, r: usize) -> (u32, u32, f32) {
    let er = t5[3 * r + 2] as i32 as f32 * b.sc[0];
    let mut err = er * er;
    let mut c0 = (t5[3 * r] as u32) << 11;
    let mut c1 = (t5[3 * r + 1] as u32) << 11;
    c0 |= (t6[3 * g] as u32) << 5;
    c1 |= (t6[3 * g + 1] as u32) << 5;
    let eg = t6[3 * g + 2] as i32 as f32 * b.sc[1];
    err += eg * eg;
    c0 |= t5[3 * bl] as u32;
    c1 |= t5[3 * bl + 1] as u32;
    let eb = t5[3 * bl + 2] as i32 as f32 * b.sc[2];
    err += eb * eb;
    (c0, c1, err)
}

// all opaque pixels have colour rgb (0x00RRGGBB): the best 4-colour (index 2) or 3-colour
// (index 2 = 1/2) pair from the tables  [bc1_single]
fn bc1_single(b: &B1, rgb: u32, dst: &mut [u8]) {
    let (bl, g, r) = ((rgb & 255) as usize, (rgb >> 8 & 255) as usize, (rgb >> 16 & 255) as usize);
    let (mut c0, mut c1, e4) = sgpair(b, &S4_5, &S4_6, bl, g, r);
    let mut mode = 4;
    if b.kind == 0 && (b.trans != 0 || b.lvl != BCQ_FAST) {
        let (t0, t1, e3) = sgpair(b, &S3_5, &S3_6, bl, g, r);
        if b.trans != 0 || !(e3 >= e4) {
            (c0, c1, mode) = (t0, t1, 3);
        }
    }
    // indices: valid pixels 2 (4-colour c0 < c1: 3, c0 == c1: 0), transparent 3
    let mut vi = 2u32;
    if mode == 4 {
        if c0 < c1 {
            core::mem::swap(&mut c0, &mut c1);
            vi = 3;
        } else if c0 == c1 {
            vi = 0;
        }
    } else if c0 > c1 {
        core::mem::swap(&mut c0, &mut c1);
    }
    let mut bits = 0u32;
    for i in (0..16).rev() {
        bits = bits << 2 | if b.valid >> i & 1 != 0 { vi } else { 3 };
    }
    dst[0..2].copy_from_slice(&(c0 as u16).to_le_bytes());
    dst[2..4].copy_from_slice(&(c1 as u16).to_le_bytes());
    dst[4..8].copy_from_slice(&bits.to_le_bytes());
}
