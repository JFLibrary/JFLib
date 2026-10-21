// dec -- BC1..BC7 block decoders (DDS reader; verification helper of the encoders)
//                                                                       [src/img/bc_dec.s]
//
//   block_bytes(bcfmt) -> 8 / 16 (0 = unknown format)                       bc_block_bytes
//   decode_block(bcfmt, src, dst, stride) -> 0 / ERR_UNSUPPORTED            bc_decode_block
//   bc6h_decode_half(src, signed, dst)                                      bc6h_decode_half
//   decode_to(src, bcfmt, px, stride, w, h, cancel) -> 0 / ERR_*            bc_decode_to
//   decode(src, w, h, bcfmt) -> Ok(surface) / Err(ERR_*)                    bc_decode
//
// Block formats (bcfmt, BCF_*): BC1 .. BC7 = 1 .. 7 (BC4 / BC5 unsigned, BC6H UF16), BC4S 8,
// BC5S 9, BC6HS 10.  Every decoder produces 4 rows of 4 straight BGRA pixels; sRGB variants
// decode to the same bytes.
//
// Exactness (D3D11 functional spec):
// * BC6H and BC7 are integer-exact (unquantize, interpolation weights, finish_unquantize,
//   p-bits, anchors, rotations, index selection).  Reserved BC7 modes give (0, 0, 0, 0),
//   reserved BC6H modes black.  BC6H halves become bytes as round(clamp(h, 0, 1) * 255)
//   (ties up; negative -> 0).
// * BC1..BC5: the spec's float colours evaluated exactly (rationals) and rounded to nearest;
//   exact ties (BC1's three-colour 1/2 mix) round down (WIC's decoder).  BC1: colour0 >
//   colour1 -> four colours, else three + transparent black; BC2 / BC3 always four.
//   BC4 / BC5 signed: -128 counts as -127, the mode compares the raw bytes, [-1, 1] maps to
//   [0, 255] as round(255 * (v + 1) / 2).
// * BC4 -> grey (A 255); BC5 -> (R, G, 0, 255); BC6H alpha 255.
//
// decode_to decodes blocks in raster order (ceil(w / 4) per block row) into an existing
// 32-bit pixel area (rows `stride` bytes apart; partial edge blocks clipped) and polls the
// cancel flag once per block row.  The block functions write rows through `Out` (straight
// into the area for full blocks, through a 16-pixel buffer for edge blocks).

use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_ARGB, Surface};
use simd::{Simd, Sse2};
use img_core::meta::surf_alloc;
use img_core::{ERR_CANCEL, ERR_TRUNC, ERR_UNSUPPORTED, ImgDec};

use crate::tables::{ANC2, ANC3A, ANC3B, BC6_LAYOUT, BC6_MODEMAP, BC6_MODES, PART2, PART3};
use crate::BCF_COUNT;

// ------------------------------------------------------------------ shared small tables

/// interpolation weights of 2-, 3- and 4-bit indices (BC6H / BC7)
pub static W2: [u8; 4] = [0, 21, 43, 64];
pub static W3: [u8; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
pub static W4: [u8; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

static FMT_BYTES: [u8; 11] = [0, 8, 16, 16, 8, 16, 16, 16, 8, 16, 16];

const fn tab<const N: usize>(mul: usize, add: usize, div: usize) -> [u8; N] {
    let mut t = [0u8; N];
    let mut s = 0;
    while s < N {
        t[s] = ((mul * s + add) / div) as u8;
        s += 1;
    }
    t
}

/// BC1 colour tables: t53[s] = round(255 s / 93), t52[s] = round(255 s / 62) (ties down),
/// t63[s] = round(255 s / 189), t62[s] = round(255 s / 126) (ties down); s = 2a + b, 3a, a + b
pub static T53: [u8; 94] = tab::<94>(510, 93, 186);
pub static T52: [u8; 63] = tab::<63>(510, 61, 124);
pub static T63: [u8; 190] = tab::<190>(510, 189, 378);
pub static T62: [u8; 127] = tab::<127>(510, 125, 252);

/// BC4 weight pairs (w0, w1) per code: 8-value mode / 6-value mode (codes 6 / 7 fixed)
pub static BC4_W7: [u8; 16] = [7, 0, 0, 7, 6, 1, 5, 2, 4, 3, 3, 4, 2, 5, 1, 6];
pub static BC4_W5: [u8; 16] = [5, 0, 0, 5, 4, 1, 3, 2, 2, 3, 1, 4, 0, 0, 0, 0];

#[inline(always)]
fn u16_at(b: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([b[o], b[o + 1]]) as u32
}

#[inline(always)]
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

#[inline(always)]
fn u64_at(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

// ------------------------------------------------------------------ public API

/// bytes of one block of bcfmt (0 = unknown format)  [bc_block_bytes]
pub fn block_bytes(fmt: u32) -> usize {
    if fmt > BCF_COUNT - 1 { 0 } else { FMT_BYTES[fmt as usize] as usize }
}

/// decode one 4x4 block (8 / 16 bytes at src) into 4 rows of 4 straight BGRA pixels at dst,
/// rows stride bytes apart -> 0 / ERR_UNSUPPORTED  [bc_decode_block]
pub fn decode_block(fmt: u32, src: &[u8], dst: &mut [u8], stride: usize) -> i32 {
    if fmt.wrapping_sub(1) > BCF_COUNT - 2 {
        return ERR_UNSUPPORTED;
    }
    let mut px = [0u32; 16];
    block(fmt, src, &mut px);
    store(&px, dst, stride, 4, 4);
    0
}

/// where a decoded block goes: 4 rows of 4 BGRA pixels
pub(crate) trait Out {
    fn row(&mut self, y: usize, r: [u32; 4]);
}

/// a full block in a pixel area (row y at o + y * stride)
struct Full<'a> {
    d: &'a mut [u8],
    o: usize,
    stride: usize,
}

impl Out for Full<'_> {
    #[inline(always)]
    fn row(&mut self, y: usize, r: [u32; 4]) {
        let o = self.o + y * self.stride;
        let d: &mut [u8; 16] = (&mut self.d[o..o + 16]).try_into().unwrap();
        d[0..4].copy_from_slice(&r[0].to_le_bytes());
        d[4..8].copy_from_slice(&r[1].to_le_bytes());
        d[8..12].copy_from_slice(&r[2].to_le_bytes());
        d[12..16].copy_from_slice(&r[3].to_le_bytes());
    }
}

/// a block held apart (edge blocks, decode_block)
impl Out for [u32; 16] {
    #[inline(always)]
    fn row(&mut self, y: usize, r: [u32; 4]) {
        self[(y & 3) * 4..(y & 3) * 4 + 4].copy_from_slice(&r);
    }
}

// 16 pixels -> rows of `cols` pixels (the first `rows` rows) at dst
#[inline(always)]
fn store(px: &[u32; 16], dst: &mut [u8], stride: usize, rows: usize, cols: usize) {
    for y in 0..rows {
        let d = &mut dst[y * stride..y * stride + cols * 4];
        for (x, o) in d.chunks_exact_mut(4).enumerate() {
            o.copy_from_slice(&px[y * 4 + x].to_le_bytes());
        }
    }
}

// one block of a valid bcfmt (1..10) as 16 BGRA pixels (bc_fns)
#[inline(always)]
fn block<O: Out>(fmt: u32, src: &[u8], o: &mut O) {
    match fmt {
        1 => bc1_block(src, o, false),
        2 => bc2_block(src, o),
        3 => bc3_block(src, o),
        4 => bc4_block(src, o, false),
        5 => bc5_block(src, o, false),
        6 => bc6_block8(src, o, false),
        7 => bc7_block(src, o),
        8 => bc4_block(src, o, true),
        9 => bc5_block(src, o, true),
        _ => bc6_block8(src, o, true),
    }
}

/// decode ceil(w / 4) x ceil(h / 4) blocks (raster order) into the 32-bit pixels at px (row
/// y at y * stride, w x h pixels) -> 0 / ERR_TRUNC / ERR_CANCEL / ERR_UNSUPPORTED; the
/// cancel flag is polled once per block row  [bc_decode_to]
pub fn decode_to(src: &[u8], fmt: u32, px: &mut [u8], stride: usize, w: u32, h: u32, cancel: Option<&AtomicU32>) -> i32 {
    if fmt.wrapping_sub(1) > BCF_COUNT - 2 {
        return ERR_UNSUPPORTED;
    }
    let bs = FMT_BYTES[fmt as usize] as u64;
    let cols = (w as u64 + 3) >> 2;
    let rows = (h as u64 + 3) >> 2;
    if rows * cols * bs > src.len() as u64 {
        return ERR_TRUNC;
    }
    let a = Area { src, px, stride, w: w as usize, h: h as usize, cancel };
    match fmt {
        1 => a.run::<1, 8>(),
        2 => a.run::<2, 16>(),
        3 => a.run::<3, 16>(),
        4 => a.run::<4, 8>(),
        5 => a.run::<5, 16>(),
        6 => a.run::<6, 16>(),
        7 => a.run::<7, 16>(),
        8 => a.run::<8, 8>(),
        9 => a.run::<9, 16>(),
        _ => a.run::<10, 16>(),
    }
}

// the block loop of decode_to, one instance per format
struct Area<'a, 'b> {
    src: &'a [u8],
    px: &'a mut [u8],
    stride: usize,
    w: usize,
    h: usize,
    cancel: Option<&'b AtomicU32>,
}

impl Area<'_, '_> {
    fn run<const F: u32, const BS: usize>(self) -> i32 {
        let (w, h, stride) = (self.w, self.h, self.stride);
        let cols = (w + 3) >> 2;
        let rows = (h + 3) >> 2;
        let mut blocks = self.src.chunks_exact(BS);
        for by in 0..rows {
            if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
                return ERR_CANCEL;
            }
            let rl = h - by * 4; // rows left (>= 1)
            let row = by * 4 * stride;
            for bx in 0..cols {
                let cl = w - bx * 4; // columns left
                let b: &[u8; BS] = blocks.next().unwrap().try_into().unwrap();
                if rl >= 4 && cl >= 4 {
                    block(F, b, &mut Full { d: self.px, o: row + bx * 16, stride });
                } else {
                    let mut t = [0u32; 16];
                    block(F, b, &mut t);
                    store(&t, &mut self.px[row + bx * 16..], stride, rl.min(4), cl.min(4));
                }
            }
        }
        0
    }
}

/// decode a whole BCn image (blocks in raster order) into a new FMT_ARGB surface (w, h
/// 1..65535, at most IMG_MAXPX_DEF pixels: ERR_DIMS / ERR_LIMIT; ERR_TRUNC,
/// ERR_UNSUPPORTED)  [bc_decode]
pub fn decode(src: &[u8], w: u32, h: u32, fmt: u32) -> Result<Surface, i32> {
    if block_bytes(fmt) == 0 {
        return Err(ERR_UNSUPPORTED);
    }
    let mut sf = surf_alloc(w, h, FMT_ARGB, &ImgDec::default())?;
    let stride = sf.stride as usize;
    let r = decode_to(src, fmt, &mut sf.px, stride, w, h, None);
    if r != 0 { Err(r) } else { Ok(sf) }
}

// ------------------------------------------------------------------ BC1 / BC2 / BC3

// the 4 colours of a BC1 colour block as BGRA; four forces the four-colour mode (both
// palettes are looked up and the mode selects, without branches)  [bc1_pal]
#[inline(always)]
fn bc1_pal(src: &[u8], four: bool) -> [u32; 4] {
    let c0 = u16_at(src, 0);
    let c1 = u16_at(src, 2);
    let four = four || c0 > c1;
    let (b0, b1) = ((c0 & 31) as usize, (c1 & 31) as usize);
    let (g0, g1) = ((c0 >> 5 & 63) as usize, (c1 >> 5 & 63) as usize);
    let (r0, r1) = ((c0 >> 11 & 31) as usize, (c1 >> 11 & 31) as usize);
    let t53 = |i: usize| T53[i.min(93)] as u32;
    let t63 = |i: usize| T63[i.min(189)] as u32;
    let p0 = t53(3 * b0) | t63(3 * g0) << 8 | t53(3 * r0) << 16 | 0xFF00_0000;
    let p1 = t53(3 * b1) | t63(3 * g1) << 8 | t53(3 * r1) << 16 | 0xFF00_0000;
    let p2f = t53(2 * b0 + b1) | t63(2 * g0 + g1) << 8 | t53(2 * r0 + r1) << 16 | 0xFF00_0000;
    let p3f = t53(b0 + 2 * b1) | t63(g0 + 2 * g1) << 8 | t53(r0 + 2 * r1) << 16 | 0xFF00_0000;
    let p2t = T52[(b0 + b1).min(62)] as u32 | (T62[(g0 + g1).min(126)] as u32) << 8 | (T52[(r0 + r1).min(62)] as u32) << 16 | 0xFF00_0000;
    let m = (four as u32).wrapping_neg();
    [p0, p1, (p2f & m) | (p2t & !m), p3f & m]
}

// the 4 pixels of row y: 2-bit indices at bits 8 y .. 8 y + 7
#[inline(always)]
fn bc1_row(pal: &[u32; 4], idx: u32, y: usize) -> [u32; 4] {
    let i = idx >> (8 * y);
    [pal[(i & 3) as usize], pal[(i >> 2 & 3) as usize], pal[(i >> 4 & 3) as usize], pal[(i >> 6 & 3) as usize]]
}

// BC1 colour block -> 16 pixels  [bc1_block]
#[inline(always)]
fn bc1_block<O: Out>(src: &[u8], o: &mut O, four: bool) {
    let pal = bc1_pal(src, four);
    let idx = u32_at(src, 4);
    for y in 0..4 {
        o.row(y, bc1_row(&pal, idx, y));
    }
}

// BC2: explicit 4-bit alpha + four-colour BC1 block  [bc2_block]
fn bc2_block<O: Out>(src: &[u8], o: &mut O) {
    let pal = bc1_pal(&src[8..], true);
    let idx = u32_at(src, 12);
    let a = u64_at(src, 0);
    for y in 0..4 {
        let mut r = bc1_row(&pal, idx, y);
        for (x, p) in r.iter_mut().enumerate() {
            let v = (a >> (16 * y + 4 * x) & 15) as u32 * 17;
            *p = (*p & 0x00FF_FFFF) | v << 24;
        }
        o.row(y, r);
    }
}

// BC3: BC4-style alpha block + four-colour BC1 block  [bc3_block]
fn bc3_block<O: Out>(src: &[u8], o: &mut O) {
    let pal = bc1_pal(&src[8..], true);
    let idx = u32_at(src, 12);
    let vals = bc4_vals(src, false);
    let ai = u64_at(src, 0) >> 16;
    for y in 0..4 {
        let mut r = bc1_row(&pal, idx, y);
        for (x, p) in r.iter_mut().enumerate() {
            let v = vals[(ai >> (12 * y + 3 * x) & 7) as usize] as u32;
            *p = (*p & 0x00FF_FFFF) | v << 24;
        }
        o.row(y, r);
    }
}

// ------------------------------------------------------------------ BC4 / BC5

// the 8 values (bytes) of a BC4 channel block: codes 0 / 1 the endpoints, then the
// interpolations; signed: SNORM  [bc4_vals]
#[inline(always)]
pub(crate) fn bc4_vals(src: &[u8], signed: bool) -> [u8; 8] {
    let mut v = [0u8; 8];
    if !signed {
        let (r0, r1) = (src[0] as u32, src[1] as u32);
        let (w, mul, add) = if r0 > r1 { (&BC4_W7, 9363u32, 3u32) } else { (&BC4_W5, 13108, 2) };
        for (k, o) in v.iter_mut().enumerate() {
            let x = w[2 * k] as u32 * r0 + w[2 * k + 1] as u32 * r1;
            *o = ((x + add) * mul >> 16) as u8;
        }
        if add == 2 {
            v[6] = 0;
            v[7] = 255;
        }
        return v;
    }
    let (mut s0, mut s1) = (src[0] as i8 as i32, src[1] as i8 as i32);
    let (w, d) = if s0 > s1 { (&BC4_W7, 7i32) } else { (&BC4_W5, 5) };
    s0 = s0.max(-127);
    s1 = s1.max(-127);
    let div = (508 * d) as u32;
    let d127 = 127 * d;
    for (k, o) in v.iter_mut().enumerate() {
        let u = w[2 * k] as i32 * s0 + w[2 * k + 1] as i32 * s1 + d127;
        *o = (((u * 510 + 2 * d127) as u32) / div) as u8;
    }
    if d == 5 {
        v[6] = 0;
        v[7] = 255;
    }
    v
}

// BC4 -> grey pixels  [bc4_block]
fn bc4_block<O: Out>(src: &[u8], o: &mut O, signed: bool) {
    let vals = bc4_vals(src, signed);
    let idx = u64_at(src, 0) >> 16;
    for y in 0..4 {
        let mut r = [0u32; 4];
        for (x, p) in r.iter_mut().enumerate() {
            *p = vals[(idx >> (12 * y + 3 * x) & 7) as usize] as u32 * 0x010101 | 0xFF00_0000;
        }
        o.row(y, r);
    }
}

// BC5 -> (R, G, 0, 255)  [bc5_block]
fn bc5_block<O: Out>(src: &[u8], o: &mut O, signed: bool) {
    let vr = bc4_vals(src, signed);
    let vg = bc4_vals(&src[8..], signed);
    let ir = u64_at(src, 0) >> 16;
    let ig = u64_at(src, 8) >> 16;
    for y in 0..4 {
        let mut r = [0u32; 4];
        for (x, p) in r.iter_mut().enumerate() {
            let sh = 12 * y + 3 * x;
            let rr = vr[(ir >> sh & 7) as usize] as u32;
            let g = vg[(ig >> sh & 7) as usize] as u32;
            *p = rr << 16 | g << 8 | 0xFF00_0000;
        }
        o.row(y, r);
    }
}

// ------------------------------------------------------------------ BC6H

// BC6H -> bytes round(clamp(half, 0, 1) * 255)  [bc6_block8]
fn bc6_block8<O: Out>(src: &[u8], o: &mut O, signed: bool) {
    let mut h = [0u16; 64];
    bc6h_decode_half(src, signed, &mut h);
    for y in 0..4 {
        let mut r = [0u32; 4];
        for (x, p) in r.iter_mut().enumerate() {
            let i = y * 4 + x;
            let b = h2u8(h[i * 4 + 2] as u32);
            let g = h2u8(h[i * 4 + 1] as u32);
            let rr = h2u8(h[i * 4] as u32);
            *p = b | g << 8 | rr << 16 | 0xFF00_0000;
        }
        o.row(y, r);
    }
}

/// IEEE half (low 16 bits) -> round(clamp(h, 0, 1) * 255), ties up, negative / NaN -> 0,
/// +inf -> 255  [bc_h2u8]
#[inline]
pub(crate) fn h2u8(h: u32) -> u32 {
    let h = h & 0xFFFF;
    if h & 0x8000 != 0 {
        return 0;
    }
    if h >= 0x3C00 {
        return if h > 0x7C00 { 0 } else { 255 };
    }
    let mut e = h >> 10;
    let mut m = h & 0x3FF;
    if e == 0 {
        e = 1;
    } else {
        m |= 0x400;
    }
    let sh = 25 - e; // 11..24
    (m * 255 + (1 << (sh - 1))) >> sh
}

/// BC6H unquantize of an endpoint (spec "unquantize"): v (i32), endpoint bits, signed
/// (SF16)  [bc6_unq]
#[inline]
pub(crate) fn bc6_unq(v: i32, bits: u32, signed: bool) -> i32 {
    if !signed {
        if bits >= 15 || v == 0 {
            return v;
        }
        let m = (1u32 << bits).wrapping_sub(1);
        if v as u32 == m {
            return 0xFFFF;
        }
        return (((v as u32) << 16).wrapping_add(0x8000) >> bits) as i32;
    }
    if bits >= 16 {
        return v;
    }
    let s = v >> 31;
    let a = ((v ^ s).wrapping_sub(s)) as u32;
    if a == 0 {
        return 0;
    }
    let b = bits - 1;
    let m = (1u32 << b).wrapping_sub(1);
    let r = if a >= m { 0x7FFF } else { (a << 15).wrapping_add(0x4000) >> b };
    ((r as i32) ^ s).wrapping_sub(s)
}

#[inline(always)]
fn sext(v: i32, bits: u32) -> i32 {
    let s = 32 - bits;
    v.wrapping_shl(s).wrapping_shr(s)
}

/// decode one BC6H block (signed: SF16, else UF16) into 16 x (r, g, b, a) IEEE halves
/// (a = 1.0); reserved modes -> 0  [bc6h_decode_half]
pub fn bc6h_decode_half(src: &[u8], signed: bool, dst: &mut [u16; 64]) {
    let lo = u64_at(src, 0);
    let hi = u64_at(src, 8);
    let mi = BC6_MODEMAP[(lo & 31) as usize] as usize;
    if mi >= 14 {
        for p in dst.chunks_exact_mut(4) {
            p.copy_from_slice(&[0, 0, 0, 0x3C00]);
        }
        return;
    }
    let mut pos: u32 = if mi < 2 { 2 } else { 5 };
    let desc = &BC6_MODES[mi];
    let hbits = desc[5] as usize;
    let lay = desc[6] as usize;
    let mut fld = [0i32; 16];
    for k in 0..hbits {
        let c = BC6_LAYOUT[lay + k] as usize;
        let (f, bit) = (c >> 4, c & 15);
        let v = if pos < 64 { lo >> pos & 1 } else { hi >> (pos & 63) & 1 };
        fld[f] |= (v as i32) << bit;
        pos += 1;
    }
    // the indices follow at bit pos (>= 64)
    let mut ib_bits = hi >> (pos - 64);
    let nend = if mi < 10 { 4 } else { 2 };
    let epb = desc[1] as u32;
    let mut uq = [[0i32; 4]; 4]; // [k][c]
    for c in 0..3 {
        let mut a0 = fld[4 * c];
        if signed {
            a0 = sext(a0, epb);
        }
        uq[0][c] = bc6_unq(a0, epb, signed);
        for k in 1..nend {
            let mut v = fld[4 * c + k];
            if desc[0] != 0 {
                v = sext(v, desc[2 + c] as u32).wrapping_add(a0) & ((1i32 << epb) - 1);
            }
            if signed {
                v = sext(v, epb);
            }
            uq[k][c] = bc6_unq(v, epb, signed);
        }
    }
    // indices: 3 bits (2 regions, anchors 0 and ANC2[d]) or 4 bits (anchor 0)
    let mut anc = 1u32;
    let mut sub = 0u32;
    let mut ib = 4u32;
    if nend == 4 {
        let d = (fld[12] & 31) as usize;
        sub = PART2[d];
        anc |= 1 << ANC2[d];
        ib = 3;
    }
    let mut idx = [0u8; 16];
    for o in idx.iter_mut() {
        let n = ib - (anc & 1);
        anc >>= 1;
        *o = (ib_bits & ((1u64 << n) - 1)) as u8;
        ib_bits >>= n;
    }
    let wt: &[u8] = if ib == 4 { &W4 } else { &W3 };
    for (p, out) in dst.chunks_exact_mut(4).enumerate() {
        let s = (sub & 1) as usize;
        sub >>= 2;
        let w = wt[(idx[p] & 15) as usize] as i32;
        for c in 0..3 {
            let a = uq[2 * s][c];
            let b = uq[2 * s + 1][c];
            let v = (a.wrapping_mul(64 - w).wrapping_add(b.wrapping_mul(w)).wrapping_add(32)) >> 6;
            out[c] = finish(v, signed);
        }
        out[3] = 0x3C00;
    }
}

// finish_unquantize of an interpolated value -> half bits (-0 -> +0)
#[inline(always)]
pub(crate) fn finish(v: i32, signed: bool) -> u16 {
    if !signed {
        return ((v as u32).wrapping_mul(31) >> 6) as u16;
    }
    let s = v >> 31;
    let a = ((v ^ s).wrapping_sub(s)) as u32;
    let m = a.wrapping_mul(31) >> 5;
    if m == 0 { 0 } else { (m | (s as u32 & 0x8000)) as u16 }
}

// ------------------------------------------------------------------ BC7
//
// One specialised decoder per mode (const generics, like the B7ONE / B7DUAL macros of the
// assembly): every field is read at its constant bit offset from the 128-bit block, each
// endpoint is assembled as BGRA (p-bit appended, bits replicated per byte), the palettes are
// interpolated with SSE2 two entries per step, and the index bits are normalised by
// inserting a 0 above every anchor index, so pixel i's index sits at bits i * ib.

// bit replication of n-bit values in the bytes of x selected by msk to 8 bits  [B7REP]
#[inline(always)]
fn rep(x: u32, n: u32, msk: u32) -> u32 {
    if n >= 8 {
        return x;
    }
    let x = x << (8 - n);
    x | (x >> n) & (((0xFF >> n) * 0x0101_0101) & msk)
}

#[inline(always)]
fn fld(b: u128, off: u32, n: u32) -> u32 {
    (b >> off) as u32 & ((1u32 << n) - 1)
}

// endpoint e as expanded BGRA; o = offset of the first endpoint field, ne endpoints; R of
// all endpoints, then G, B, A, then the p-bits (pbm 1: per endpoint, 2: per subset)  [B7EPR]
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn b7_ep(b: u128, e: u32, o: u32, ne: u32, cb: u32, ab: u32, pbm: u32, nc: u32, na: u32) -> u32 {
    let mut v = fld(b, o + 2 * ne * cb + e * cb, cb) | fld(b, o + ne * cb + e * cb, cb) << 8 | fld(b, o + e * cb, cb) << 16;
    if ab != 0 {
        v |= fld(b, o + 3 * ne * cb + e * ab, ab) << 24;
    }
    if pbm != 0 {
        let p = if pbm == 1 { fld(b, o + ne * (3 * cb + ab) + e, 1) } else { fld(b, o + ne * (3 * cb + ab) + e / 2, 1) };
        v = v.wrapping_add(v) | p * 0x0101_0101;
    }
    if na == nc {
        rep(v, nc, 0xFFFF_FFFF)
    } else {
        let c = v;
        let mut r = rep(v & 0x00FF_FFFF, nc, 0x00FF_FFFF);
        if na != 0 {
            r |= rep(c >> 24, na, 0xFF) << 24;
        } else {
            r |= 0xFF00_0000;
        }
        r
    }
}

// weight pairs for b7_pal: w_k in lanes 0..3, w_k+1 in lanes 4..7 (i16 bytes)  [bc_wpairs]
const fn wpairs<const N: usize>(w: [u8; N]) -> [[u8; 16]; 8] {
    let mut t = [[0u8; 16]; 8];
    let mut k = 0;
    while k < N / 2 {
        let mut l = 0;
        while l < 8 {
            t[k][2 * l] = w[2 * k + l / 4];
            l += 1;
        }
        k += 1;
    }
    t
}
static WP2: [[u8; 16]; 8] = wpairs(W2);
static WP3: [[u8; 16]; 8] = wpairs(W3);
static WP4: [[u8; 16]; 8] = wpairs(W4);

// palette of the endpoint pair: 2^IB entries (e0 (64 - w) + e1 w + 32) >> 6 as BGRA bytes
// (SSE2, two entries per step)  [B7PAL]
#[inline(always)]
fn b7_pal<const IB: u32>(e0: u32, e1: u32, pal: &mut [u8; 64]) {
    let s = Sse2;
    let x = s.unpacklo8(s.splat64(e0 as u64 | (e1 as u64) << 32), s.zero()); // e0 lanes 0..3, e1 4..7
    let a = s.shuf32::<0x44>(x); // e0 | e0
    let d = s.sub16(s.shuf32::<0xEE>(x), a); // e1 - e0
    let base = s.add16(s.sll16::<6>(a), s.splat16(32)); // 64 e0 + 32
    let wp = match IB {
        2 => &WP2,
        3 => &WP3,
        _ => &WP4,
    };
    for k in 0..(1usize << (IB - 1)) {
        let t = s.sra16::<6>(s.add16(s.mullo16(s.load(&wp[k]), d), base));
        s.store_px(&mut pal[k * 8..k * 8 + 8], t);
    }
}

#[inline(always)]
fn ent(pal: &[u8; 64], v: usize) -> u32 {
    let v = (v & 15) * 4;
    u32::from_le_bytes([pal[v], pal[v + 1], pal[v + 2], pal[v + 3]])
}

// insert a 0 bit at position p (bits >= p move up)  [B7INSC / B7INSV]
#[inline(always)]
fn ins(x: u64, p: u32) -> u64 {
    let m = (1u64 << p) - 1;
    (x & m) | (x & !m) << 1
}

// a mode without secondary indices (0..3, 6, 7)  [B7ONE]
#[inline(always)]
fn b7_one<O: Out, const M: u32, const NS: u32, const PB: u32, const CB: u32, const AB: u32, const PBM: u32, const NPB: u32, const IB: u32, const NC: u32, const NA: u32>(
    b: u128,
    o: &mut O,
) {
    let part = if PB != 0 { fld(b, M + 1, PB) as usize } else { 0 };
    let ofs = M + 1 + PB;
    let mut pal = [[0u8; 64]; 4];
    // subset by subset (unrolled: constant field offsets)
    b7_sub::<0, NS, CB, AB, PBM, IB, NC, NA>(b, ofs, &mut pal[0]);
    if NS > 1 {
        b7_sub::<1, NS, CB, AB, PBM, IB, NC, NA>(b, ofs, &mut pal[1]);
    }
    if NS > 2 {
        b7_sub::<2, NS, CB, AB, PBM, IB, NC, NA>(b, ofs, &mut pal[2]);
    }
    // indices: 16 ib - ns bits; zero bits above the anchors (ascending positions)
    let mut x = (b >> (ofs + 2 * NS * (3 * CB + AB) + NPB)) as u64;
    x = ins(x, IB - 1);
    let mut sub = 0u32;
    if NS == 2 {
        x = ins(x, ANC2[part] as u32 * IB + IB - 1);
        sub = PART2[part];
    } else if NS == 3 {
        let (a1, a2) = (ANC3A[part] as u32, ANC3B[part] as u32);
        let (lo, hi) = if a1 < a2 { (a1, a2) } else { (a2, a1) };
        x = ins(x, lo * IB + IB - 1);
        x = ins(x, hi * IB + IB - 1);
        sub = PART3[part];
    }
    let mask = (1u64 << IB) - 1;
    macro_rules! px {
        ($i:expr) => {{
            let v = (x >> ($i * IB) & mask) as usize;
            if NS == 1 { ent(&pal[0], v) } else { ent(&pal[(sub >> (2 * $i) & 3) as usize], v) }
        }};
    }
    o.row(0, [px!(0), px!(1), px!(2), px!(3)]);
    o.row(1, [px!(4), px!(5), px!(6), px!(7)]);
    o.row(2, [px!(8), px!(9), px!(10), px!(11)]);
    o.row(3, [px!(12), px!(13), px!(14), px!(15)]);
}

// the palette of subset S (endpoints 2 S and 2 S + 1)
#[inline(always)]
fn b7_sub<const S: u32, const NS: u32, const CB: u32, const AB: u32, const PBM: u32, const IB: u32, const NC: u32, const NA: u32>(
    b: u128,
    o: u32,
    pal: &mut [u8; 64],
) {
    let e0 = b7_ep(b, 2 * S, o, 2 * NS, CB, AB, PBM, NC, NA);
    let e1 = b7_ep(b, 2 * S + 1, o, 2 * NS, CB, AB, PBM, NC, NA);
    b7_pal::<IB>(e0, e1, pal);
}

// modes 4 / 5: one subset, rotation, index selection, colour and alpha indices  [B7DUAL]
#[inline(always)]
fn b7_dual<O: Out, const M: u32, const RB: u32, const ISB: u32, const CB: u32, const AB: u32, const NC: u32, const NA: u32, const IB: u32, const IB2: u32>(
    b: u128,
    o: &mut O,
) {
    let rot = fld(b, M + 1, RB) & 3;
    let sel = if ISB != 0 { fld(b, M + 1 + RB, 1) } else { 0 };
    let ofs = M + 1 + RB + ISB;
    let e0 = b7_ep(b, 0, ofs, 2, CB, AB, 0, NC, NA);
    let e1 = b7_ep(b, 1, ofs, 2, CB, AB, 0, NC, NA);
    let o1 = ofs + 2 * (3 * CB + AB);
    // primary (ib bits) and secondary (ib2 bits) index sets, anchor bit inserted
    let x1 = ins((b >> o1) as u64 & ((1u64 << (16 * IB - 1)) - 1), IB - 1);
    let x2 = ins((b >> (o1 + 16 * IB - 1)) as u64, IB2 - 1);
    let mut cpal = [0u8; 64];
    let mut apal = [0u8; 64];
    // colour from the primary indices, alpha from the secondary (index selection: the other
    // way round)
    let (xc, xa) = if sel == 0 {
        b7_pal::<IB>(e0, e1, &mut cpal);
        b7_pal::<IB2>(e0, e1, &mut apal);
        (x1, x2)
    } else {
        b7_pal::<IB2>(e0, e1, &mut cpal);
        b7_pal::<IB>(e0, e1, &mut apal);
        (x2, x1)
    };
    let (cb, ab) = if sel == 0 { (IB, IB2) } else { (IB2, IB) };
    // the colour palette keeps RGB, the alpha palette A; rotation swaps A with R / G / B
    // [B7ROT]
    let cl = (3 - rot) * 8;
    let mut cp = [0u32; 8];
    let mut ap = [0u32; 8];
    for k in 0..8 {
        let (mut c, mut a) = (ent(&cpal, k) & 0x00FF_FFFF, ent(&apal, k) & 0xFF00_0000);
        if rot != 0 {
            let r11 = (c >> cl) << 24;
            c = (c & !(0xFF << cl)) | r11;
            a = (a >> 24) << cl;
        }
        cp[k] = c;
        ap[k] = a;
    }
    let (cm, am) = ((1u64 << cb) - 1, (1u64 << ab) - 1);
    macro_rules! px {
        ($i:expr) => {{
            let ci = (xc >> ($i * cb) & cm) as usize;
            let ai = (xa >> ($i * ab) & am) as usize;
            cp[ci & 7] | ap[ai & 7]
        }};
    }
    o.row(0, [px!(0), px!(1), px!(2), px!(3)]);
    o.row(1, [px!(4), px!(5), px!(6), px!(7)]);
    o.row(2, [px!(8), px!(9), px!(10), px!(11)]);
    o.row(3, [px!(12), px!(13), px!(14), px!(15)]);
}

// BC7 block -> 16 pixels  [bc7_block]
fn bc7_block<O: Out>(src: &[u8], o: &mut O) {
    let b = u64_at(src, 0) as u128 | (u64_at(src, 8) as u128) << 64;
    if b as u8 == 0 {
        for y in 0..4 {
            o.row(y, [0; 4]);
        }
        return;
    }
    match (b as u8).trailing_zeros() {
        //            m  ns pb cb ab pbm npb ib nc na
        0 => b7_one::<O, 0, 3, 4, 4, 0, 1, 6, 3, 5, 0>(b, o),
        1 => b7_one::<O, 1, 2, 6, 6, 0, 2, 2, 3, 7, 0>(b, o),
        2 => b7_one::<O, 2, 3, 6, 5, 0, 0, 0, 2, 5, 0>(b, o),
        3 => b7_one::<O, 3, 2, 6, 7, 0, 1, 4, 2, 8, 0>(b, o),
        //             m  rb isb cb ab nc na ib ib2
        4 => b7_dual::<O, 4, 2, 1, 5, 6, 5, 6, 2, 3>(b, o),
        5 => b7_dual::<O, 5, 2, 0, 7, 8, 7, 8, 2, 2>(b, o),
        6 => b7_one::<O, 6, 1, 0, 7, 7, 1, 2, 4, 8, 8>(b, o),
        _ => b7_one::<O, 7, 2, 6, 5, 5, 1, 4, 2, 6, 6>(b, o),
    }
}
