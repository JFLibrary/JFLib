// mip -- display mip levels: 2 x 2 box filter in premultiplied space        [src/img/mip.s]
//
//   mip_down(src, dst, x0, y0, x1, y1) -> 0 / ERR_*
//        update the part of dst (the next level of src) covered by the source rectangle
//        x0, y0 .. x1, y1 (exclusive; clamped to src)                            [mip_down]
//   mip_down_rows(src, band, x0, x1, dy0, dy1) -> 0 / ERR_*
//        the same for dst rows dy0 .. dy1-1 (of a Band of dst; workers split the dst rows) and
//        the source columns x0 .. x1-1                                       [mip_down_rows]
//   MipChain::alloc(w, h, levels) -> Option<MipChain>
//        levels 1 .. n below a w x h base (levels 0 = until 1 x 1; at most MIP_MAX)
//                                                                         [mip_chain_alloc]
//   chain.update(src, rect) -> 0 / ERR_*   propagate the dirty rectangle rect ([x0, y0, x1,
//        y1]; None = everything) of the base image src through every level [mip_chain_update]
//   chain.level(k) -> Option<&Surface>     level k (1 .. n)                [mip_chain_level]
//
// src: FMT_ARGB (straight) or FMT_PARGB; dst: FMT_PARGB of (w + 1) / 2 x (h + 1) / 2
// (ERR_UNSUPPORTED / ERR_DIMS otherwise).  dst(i, j) averages src columns 2i, min(2i + 1,
// w - 1) and rows 2j, min(2j + 1, h - 1) (odd edges repeat the last pixel):
//   FMT_PARGB   v = (p00 + p01 + p10 + p11 + 2) >> 2 per channel
//   FMT_ARGB    a = (sum a + 2) >> 2;  t = (sum c * a + 2) >> 2,  c = min(a, ((t + 128) * 257) >> 16)
// Thread safe (no global state); mip_down_rows calls on disjoint bands may run in parallel.
// The level surfaces are ordinary FMT_PARGB surfaces owned by the chain (drop = mip_chain_free).

use alloc::vec::Vec;

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use simd::{Kernel, Simd};

use crate::ops::Band;
use crate::{ERR_DIMS, ERR_UNSUPPORTED};

pub const MIP_MAX: u32 = 16; // levels below the base (65535 -> 1 needs 16)

#[inline]
fn px_of(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]])
}

// one dst pixel of a premultiplied source
#[inline(always)]
fn box_pm(p: [u32; 4]) -> u32 {
    let mut out = 0;
    for k in 0..4 {
        let c = |v: u32| (v >> (k * 8)) & 255;
        let s = c(p[0]) + c(p[1]) + c(p[2]) + c(p[3]);
        out |= ((s + 2) >> 2) << (k * 8);
    }
    out
}

// one dst pixel of a straight source (premultiplied at full precision)
#[inline(always)]
fn box_st(p: [u32; 4]) -> u32 {
    let sa = (p[0] >> 24) + (p[1] >> 24) + (p[2] >> 24) + (p[3] >> 24);
    let a = (sa + 2) >> 2;
    let mut out = a << 24;
    for k in 0..3 {
        let ca = |v: u32| ((v >> (k * 8)) & 255) * (v >> 24);
        let sca = ca(p[0]) + ca(p[1]) + ca(p[2]) + ca(p[3]);
        let t = (sca + 2) >> 2;
        let c = (((t + 128) * 257) >> 16).min(a);
        out |= c << (k * 8);
    }
    out
}

// one dst pixel per 128-bit lane of a straight source: rows a = [p00 | p01], b = [p10 | p11]
// (pixel lanes) -> i32 lanes (c0, c1, c2, -) = ((sum c a + 2) >> 2 + 128) * 257 >> 16 and
// (-, -, -, a) = (sum a + 2) >> 2
#[inline(always)]
fn box_st_lanes<S: Simd>(s: S, a: S::V, b: S::V) -> (S::V, S::V) {
    let z = s.zero();
    let s4 = s.add16(a, b);
    let sa = s.add16(s4, s.shuf32::<0x4E>(s4)); // low half: sums of the 4 pixels
    let ca = s.mullo16(a, s.shuf16::<0xFF>(a)); // c * a
    let cb = s.mullo16(b, s.shuf16::<0xFF>(b));
    let sca = s.add32(s.add32(s.unpacklo16(ca, z), s.unpackhi16(ca, z)), s.add32(s.unpacklo16(cb, z), s.unpackhi16(cb, z)));
    let t = s.add32(s.srl32::<2>(s.add32(sca, s.splat32(2))), s.splat32(128));
    let c = s.srl32::<16>(s.add32(t, s.sll32::<8>(t)));
    let al = s.srl32::<2>(s.add32(s.unpacklo16(sa, z), s.splat32(2)));
    (c, al)
}

// dst pixels i0 .. i1 of one row from source rows r0, r1 (w source pixels each)
struct RowK<'a> {
    d: &'a mut [u8],
    r0: &'a [u8],
    r1: &'a [u8],
    w: usize,
    i0: usize,
    i1: usize,
    straight: bool,
}

impl Kernel for RowK<'_> {
    type Out = ();
    #[inline(always)]
    fn run<S: Simd>(self, s: S) {
        let RowK { d, r0, r1, w, i0, i1, straight } = self;
        let mut i = i0;
        if straight {
            // BYTES source bytes per row = PX dst pixels: unpacklo8 holds the even dst pixels
            // (one per 128-bit lane), unpackhi8 the odd ones; packs32 puts them back in order
            let (z, am) = (s.zero(), s.splat64(0xFFFF_0000_0000_0000));
            while i + S::PX <= i1 && 2 * (i + S::PX) <= w {
                let a = s.load(&r0[i * 8..]);
                let b = s.load(&r1[i * 8..]);
                let (ce, ae) = box_st_lanes(s, s.unpacklo8(a, z), s.unpacklo8(b, z));
                let (co, ao) = box_st_lanes(s, s.unpackhi8(a, z), s.unpackhi8(b, z));
                let v = s.or(s.andnot(am, s.packs32(ce, co)), s.and(am, s.packs32(ae, ao)));
                s.store_px(&mut d[i * 4..], s.mins16(v, s.shuf16::<0xFF>(v))); // c <= a
                i += S::PX;
            }
        } else {
            // 2 * PX source pixels per register: [p0 p1 | p2 p3] -> [p0 + p1 | p2 + p3]
            let (z, two) = (s.zero(), s.splat16(2));
            while i + S::PX <= i1 && 2 * (i + S::PX) <= w {
                let a = s.load(&r0[i * 8..]);
                let b = s.load(&r1[i * 8..]);
                let (al, ah) = (s.unpacklo8(a, z), s.unpackhi8(a, z));
                let (bl, bh) = (s.unpacklo8(b, z), s.unpackhi8(b, z));
                let ta = s.add16(s.shufps::<0x44>(al, ah), s.shufps::<0xEE>(al, ah));
                let tb = s.add16(s.shufps::<0x44>(bl, bh), s.shufps::<0xEE>(bl, bh));
                s.store_px(&mut d[i * 4..], s.srl16::<2>(s.add16(s.add16(ta, tb), two)));
                i += S::PX;
            }
        }
        let last = w - 1;
        while i < i1 {
            let (c0, c1) = (2 * i, (2 * i + 1).min(last));
            let p = [px_of(r0, c0), px_of(r0, c1), px_of(r1, c0), px_of(r1, c1)];
            let v = if straight { box_st(p) } else { box_pm(p) };
            d[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            i += 1;
        }
    }
}

/// dst rows dy0 .. dy1-1 (clamped to the dst and to the band) from the source columns
/// x0 .. x1-1: ERR_UNSUPPORTED (formats), ERR_DIMS (dst not the half size)  [mip_down_rows]
pub fn mip_down_rows(src: &Surface, dst: &mut Band, x0: i32, x1: i32, dy0: i32, dy1: i32) -> i32 {
    if dst.fmt != FMT_PARGB || (src.fmt != FMT_PARGB && src.fmt != FMT_ARGB) {
        return ERR_UNSUPPORTED;
    }
    let (w, h) = (src.w, src.h);
    if w <= 0 || h <= 0 || (w + 1) >> 1 != dst.w || (h + 1) >> 1 != dst.h {
        return ERR_DIMS;
    }
    let i0 = (x0.max(0) >> 1) as usize;
    let i1 = (x1.min(w) + 1) >> 1;
    if i1 <= i0 as i32 {
        return 0;
    }
    let j0 = dy0.max(0).max(dst.y0);
    let j1 = dy1.min(dst.h).min(dst.y1);
    let (ss, wu) = (src.stride as usize, w as usize);
    let straight = src.fmt == FMT_ARGB;
    for j in j0..j1 {
        let a = 2 * j as usize;
        let b = (a + 1).min(h as usize - 1);
        let r0 = &src.px[a * ss..a * ss + wu * 4];
        let r1 = &src.px[b * ss..b * ss + wu * 4];
        simd::dispatch(RowK { d: dst.row(j), r0, r1, w: wu, i0, i1: i1 as usize, straight });
    }
    0
}

/// the part of dst covered by the source rectangle x0, y0 .. x1, y1  [mip_down]
pub fn mip_down(src: &Surface, dst: &mut Surface, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
    let y0 = y0.max(0);
    let y1 = y1.min(src.h);
    mip_down_rows(src, &mut Band::whole(dst), x0, x1, y0 >> 1, (y1 + 1) >> 1)
}

/// a mip chain: levels 1 .. n (FMT_PARGB) below a w x h base  [MC_*]
pub struct MipChain {
    pub n: u32,
    pub w: i32,
    pub h: i32,
    levels: Vec<Surface>,
}

impl MipChain {
    /// levels 1 .. MIP_MAX as given, else (0, negative, too many) as many as reach 1 x 1;
    /// None when w or h is outside 1 .. 65535  [mip_chain_alloc]
    pub fn alloc(w: i32, h: i32, levels: i32) -> Option<MipChain> {
        if (w as u32).wrapping_sub(1) > 65534 || (h as u32).wrapping_sub(1) > 65534 {
            return None;
        }
        let mut n = levels as u32;
        if levels <= 0 || n > MIP_MAX {
            n = 0;
            let (mut a, mut b) = (w, h);
            while a != 1 || b != 1 {
                a = (a + 1) >> 1;
                b = (b + 1) >> 1;
                n += 1;
            }
        }
        let (mut lw, mut lh) = (w, h);
        let mut lv = Vec::with_capacity(n as usize);
        for _ in 0..n {
            lw = (lw + 1) >> 1;
            lh = (lh + 1) >> 1;
            lv.push(Surface::new(lw, lh, FMT_PARGB));
        }
        Some(MipChain { n, w, h, levels: lv })
    }

    /// level k (1 .. n)  [mip_chain_level]
    pub fn level(&self, k: u32) -> Option<&Surface> {
        if k == 0 || k > self.n || k > MIP_MAX {
            return None;
        }
        self.levels.get(k as usize - 1)
    }

    /// propagate the dirty rectangle of the base image src (None = all of it) through every
    /// level: the next level's rectangle is (x0 >> 1, y0 >> 1, (x1 + 1) >> 1, (y1 + 1) >> 1)
    /// [mip_chain_update]
    pub fn update(&mut self, src: &Surface, rect: Option<[i32; 4]>) -> i32 {
        if src.w != self.w || src.h != self.h {
            return ERR_DIMS;
        }
        let [mut x0, mut y0, mut x1, mut y1] = rect.unwrap_or([0, 0, src.w, src.h]);
        let n = self.n.min(MIP_MAX) as usize;
        for l in 0..n {
            if x0 >= x1 || y0 >= y1 {
                break;
            }
            let (done, rest) = self.levels.split_at_mut(l);
            let prev = if l == 0 { src } else { &done[l - 1] };
            let r = mip_down(prev, &mut rest[0], x0, y0, x1, y1);
            if r != 0 {
                return r;
            }
            x0 >>= 1;
            y0 >>= 1;
            x1 = (x1 + 1) >> 1;
            y1 = (y1 + 1) >> 1;
        }
        0
    }
}
