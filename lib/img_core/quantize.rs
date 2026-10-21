// quantize -- colour quantization: palette + index image (the 8-bit encoders: GIF, BMP 8 bit,
// PNG palette)                                                        [src/img/quantize.s]
//
//   quantize(sf, max_colors, dither, pal, idx) -> (colours, transparent index or -1)
//   quantize_ex(sf, max_colors, cfg, pal, idx) -> (colours, transparent index or -1)
//   quantize_alloc(sf, max_colors, cfg, pal) -> Some((colours, transparent index, idx))
//   opt_qcfg(opt) -> cfg from ImgOpt::dither / athresh                [imgcore.s opt_qcfg]
//
// Input: FMT_PARGB (un-premultiplied), FMT_ARGB (straight), FMT_XRGB, FMT_A8 (grey).
// max_colors is clamped to 2..256.
// cfg (quantize_ex) = dither level 0..8 in the low byte (0 = none, 8 = full Floyd-Steinberg;
// the diffused error is scaled by level / 8; > 8 counts as 8)
//   | threshold << IQ_THRESH_SHIFT: pixels with alpha < threshold (0..256) become
//     transparent, all others opaque with their straight colour
//   | IQF_WHITE: composite over white first (c + (255 - a) premultiplied), no transparency
//   | IQF_KEEPALPHA: exact palette only, alpha kept per entry (straight ARGB, every alpha-0
//     pixel counts as 0x00000000); more than max_colors colours -> 0 colours (no Wu fallback;
//     dither and threshold are ignored)
// quantize(sf, max, dither, ...) = quantize_ex with level 8 when dither != 0 (else 0) and
// threshold 128.
//
// Exact palette: when the image has at most max_colors distinct colours (the transparent
// pixels count as one colour) the palette holds exactly those, in order of first appearance.
// Otherwise: Wu's variance-minimizing quantizer (X. Wu, Graphics Gems II, 1991) on a
// 32 x 32 x 32 histogram of the opaque pixels (exact 64-bit moments) splits RGB space into
// max_colors boxes (max_colors - 1 when there are transparent pixels); a palette colour is the
// rounded mean of its box.  Pixels map to the nearest palette colour (squared RGB distance,
// lowest index on ties, searched for the centre of the pixel's 64 x 64 x 64 cell and cached per
// cell), with serpentine Floyd-Steinberg error diffusion (7/16, 3/16, 5/16, 1/16; the
// accumulated error e (in 16ths) is applied as (e * level + 64) >> 7) when the level is not 0.
// Transparent pixels get the last palette entry (0x00000000) and take no part in the diffusion.
//
//   pal  u32[256] straight 0xAARRGGBB: alpha 255, or 0x00000000 for the transparent entry
//        (IQF_KEEPALPHA: any alpha); entries past the count are set to 0.  Left untouched
//        when the result is 0 colours.
//   idx  w * h bytes, rows packed (stride w).  As in the assembly, a failed exact palette
//        (IQF_KEEPALPHA) leaves the indices written before the failing pixel.
// Result: (number of palette entries 1..max_colors, index of the transparent entry or -1);
// (0, -1) for an empty surface, an unknown format or an index buffer smaller than w * h.

use alloc::vec;
use alloc::vec::Vec;

use crate::meta::get_row;
use crate::{ImgOpt, Surface};
use gfx::{FMT_ARGB, FMT_PARGB};
use simd::{Simd, Sse2};

pub const IQ_THRESH_SHIFT: u32 = 8; // cfg: threshold << IQ_THRESH_SHIFT
pub const IQF_KEEPALPHA: u32 = 1 << 20; // exact palette with per-entry alpha only
pub const IQF_WHITE: u32 = 1 << 21; // composite over white first (no transparent entry)

const HASHMUL: u32 = 0x9E37_79B1;
const SIDE: usize = 33; // Wu histogram: 32 cells + a zero plane per axis
const SG: usize = SIDE; // cell strides
const SR: usize = SIDE * SIDE;
const NCELL: usize = SIDE * SIDE * SIDE;
const STRIDES: [usize; 3] = [SR, SG, 1];
const AXES: [[usize; 2]; 3] = [[1, 2], [0, 2], [0, 1]]; // the two other axes per direction

/// img_quantize: cfg = level 8 (dither != 0) or 0, threshold 128
pub fn quantize(sf: &Surface, max_colors: i32, dither: u32, pal: &mut [u32; 256], idx: &mut [u8]) -> (u32, i32) {
    let cfg = if dither != 0 { 8 } else { 0 } | 128 << IQ_THRESH_SHIFT;
    quantize_ex(sf, max_colors, cfg, pal, idx)
}

/// img_quantize_alloc: quantize_ex into a new index image (w * h bytes); None on failure
pub fn quantize_alloc(sf: &Surface, max_colors: i32, cfg: u32, pal: &mut [u32; 256]) -> Option<(u32, i32, Vec<u8>)> {
    if sf.w <= 0 || sf.h <= 0 {
        return None;
    }
    let mut idx = vec![0u8; sf.w as usize * sf.h as usize];
    let (n, t) = quantize_ex(sf, max_colors, cfg, pal, &mut idx);
    if n == 0 { None } else { Some((n, t, idx)) }
}

/// opt_qcfg: the quantize_ex cfg from the biased ImgOpt::dither (0 = level 7, n = level n - 1,
/// at most 8) and ImgOpt::athresh (0 = 128, n = threshold n - 1, at most 255)  [imgcore.s]
pub fn opt_qcfg(opt: &ImgOpt) -> u32 {
    let d = if opt.dither == 0 { 8 } else { opt.dither };
    let t = if opt.athresh == 0 { 129 } else { opt.athresh };
    (d - 1).min(8) | (t - 1).min(255) << IQ_THRESH_SHIFT
}

/// img_quantize_ex (see the module header)
pub fn quantize_ex(sf: &Surface, max_colors: i32, cfg: u32, pal_out: &mut [u32; 256], idx: &mut [u8]) -> (u32, i32) {
    if sf.w <= 0 || sf.h <= 0 || sf.fmt > FMT_ARGB {
        return (0, -1);
    }
    let (w, h) = (sf.w as usize, sf.h as usize);
    if idx.len() < w * h {
        return (0, -1);
    }
    let mut q = Qz {
        w,
        h,
        max: max_colors.clamp(2, 256) as u32,
        level: (cfg & 0xFF).min(8),
        thresh: ((cfg >> IQ_THRESH_SHIFT) & 0x1FF).min(256),
        flags: cfg & (IQF_KEEPALPHA | IQF_WHITE),
        hastr: false,
        k: 0,
        trans: -1,
        pal: [0; 256],
        work: vec![0u32; w * h],
    };
    q.convert(sf);
    let mut count = q.exact(idx);
    if count == 0 {
        if q.flags & IQF_KEEPALPHA != 0 {
            return (0, -1); // exact palette only
        }
        // more colours than allowed: Wu + mapping
        q.k = q.max - q.hastr as u32;
        q.wu();
        count = q.k;
        if q.hastr {
            q.trans = q.k as i32;
            q.pal[q.k as usize] = 0;
            count += 1;
        }
        q.map(idx);
    }
    for (i, p) in pal_out.iter_mut().enumerate() {
        *p = if (i as u32) < count { q.pal[i] } else { 0 };
    }
    (count, q.trans)
}

// the quantizer state (QZ_* of the assembly)
struct Qz {
    w: usize,
    h: usize,
    max: u32,
    level: u32,  // dither level 0..8
    thresh: u32, // alpha threshold 0..256
    flags: u32,  // IQF_*
    hastr: bool, // there are transparent pixels
    k: u32,      // opaque palette colours (quantized path)
    trans: i32,  // transparent index or -1
    pal: [u32; 256],
    work: Vec<u32>, // straight pixels, 0 = transparent
}

// moments of a histogram cell / box: weight, sums of r, g, b and of r^2 + g^2 + b^2
#[derive(Clone, Copy, Default)]
struct Mom {
    wt: i64,
    r: i64,
    g: i64,
    b: i64,
    m2: i64,
}

impl Mom {
    #[inline(always)]
    fn add(self, o: Mom) -> Mom {
        Mom { wt: self.wt + o.wt, r: self.r + o.r, g: self.g + o.g, b: self.b + o.b, m2: self.m2 + o.m2 }
    }
    #[inline(always)]
    fn sub(self, o: Mom) -> Mom {
        Mom { wt: self.wt - o.wt, r: self.r - o.r, g: self.g - o.g, b: self.b - o.b, m2: self.m2 - o.m2 }
    }
}

// a Wu box: (lo, hi] per axis R, G, B (lohi[axis * 2], lohi[axis * 2 + 1]), volume in cells
#[derive(Clone, Copy, Default)]
struct WBox {
    lohi: [i32; 6],
    vol: i32,
}

impl WBox {
    fn set_vol(&mut self) {
        let l = &self.lohi;
        self.vol = (l[1] - l[0]).wrapping_mul(l[3] - l[2]).wrapping_mul(l[5] - l[4]);
    }
}

// qz_corners: stride along dir, the face corner offsets (+, -, -, +) across the two other
// axes, lo and hi along dir
#[inline(always)]
fn corners(b: &WBox, dir: usize) -> (usize, [usize; 4], usize, usize) {
    let [u, v] = AXES[dir];
    let (su, sv) = (STRIDES[u], STRIDES[v]);
    let (lu, hu) = (b.lohi[u * 2] as u32 as usize * su, b.lohi[u * 2 + 1] as u32 as usize * su);
    let (lv, hv) = (b.lohi[v * 2] as u32 as usize * sv, b.lohi[v * 2 + 1] as u32 as usize * sv);
    (STRIDES[dir], [hu + hv, hu + lv, lu + hv, lu + lv], b.lohi[dir * 2] as u32 as usize, b.lohi[dir * 2 + 1] as u32 as usize)
}

// QZ_FACE: the moments summed over the face at p
#[inline(always)]
fn face(m: &[Mom], p: usize, c: &[usize; 4]) -> Mom {
    m[p + c[0]].sub(m[p + c[1]]).sub(m[p + c[2]]).add(m[p + c[3]])
}

// qz_boxmom: moments of a box
fn boxmom(m: &[Mom], b: &WBox) -> Mom {
    let (s, c, lo, hi) = corners(b, 0);
    face(m, hi * s, &c).sub(face(m, lo * s, &c))
}

// qz_boxvar: sum of squared deviations of the box's pixels from their mean (0 for a single
// cell or an empty box)
fn boxvar(m: &[Mom], b: &WBox) -> f64 {
    if b.vol <= 1 {
        return 0.0;
    }
    let w = boxmom(m, b);
    if w.wt == 0 {
        return 0.0;
    }
    let mut x = (w.r as f64) * (w.r as f64);
    x += (w.g as f64) * (w.g as f64);
    x += (w.b as f64) * (w.b as f64);
    x /= w.wt as f64;
    w.m2 as f64 - x
}

// qz_maximize: best cut along dir (-1 = none) and its score sum(|half moments|^2 / half
// weight) over both halves
fn maximize(m: &[Mom], b: &WBox, dir: usize) -> (i32, f64) {
    let (s, c, lo, hi) = corners(b, dir);
    let base = face(m, lo * s, &c);
    let whole = face(m, hi * s, &c).sub(base);
    let mut best = 0.0f64;
    let mut cut = -1i32;
    for pos in lo + 1..hi {
        let half = face(m, pos * s, &c).sub(base);
        if half.wt == 0 {
            continue;
        }
        let rest = whole.wt - half.wt;
        if rest == 0 {
            continue;
        }
        let mut x = (half.r as f64) * (half.r as f64);
        x += (half.g as f64) * (half.g as f64);
        x += (half.b as f64) * (half.b as f64);
        x /= half.wt as f64;
        let (ur, ug, ub) = ((whole.r - half.r) as f64, (whole.g - half.g) as f64, (whole.b - half.b) as f64);
        let mut y = ur * ur;
        y += ug * ug;
        y += ub * ub;
        y /= rest as f64;
        x += y;
        if x > best {
            best = x;
            cut = pos as i32;
        }
    }
    (cut, best)
}

// qz_cut: split boxes[i] into boxes[i] + boxes[j] along the best axis; false = no cut
fn cut(m: &[Mom], boxes: &mut [WBox], i: usize, j: usize) -> bool {
    let (cr, sr) = maximize(m, &boxes[i], 0);
    let (cg, sg) = maximize(m, &boxes[i], 1);
    let (cb, sb) = maximize(m, &boxes[i], 2);
    // comisd + jb: an unordered compare counts as "below"
    let ge = |a: f64, b: f64| !(a < b || a.is_nan() || b.is_nan());
    let (dir, at) = if ge(sr, sg) && ge(sr, sb) {
        if cr < 0 {
            return false;
        }
        (0, cr)
    } else if ge(sg, sr) && ge(sg, sb) {
        (1, cg)
    } else {
        (2, cb)
    };
    boxes[j] = boxes[i];
    boxes[i].lohi[dir * 2 + 1] = at;
    boxes[j].lohi[dir * 2] = at;
    boxes[i].set_vol();
    boxes[j].set_vol();
    true
}

impl Qz {
    // qz_convert: surface -> work pixels: straight 0xFFRRGGBB or 0 = transparent (alpha <
    // thresh); IQF_WHITE: composited over white, all opaque; IQF_KEEPALPHA: straight ARGB,
    // alpha 0 -> 0
    fn convert(&mut self, sf: &Surface) {
        let w = self.w;
        let white = self.flags & IQF_WHITE != 0;
        let keep = self.flags & IQF_KEEPALPHA != 0;
        let mut tr = false;
        let mut rowb = vec![0u8; w * 4];
        for y in 0..self.h {
            get_row(sf, y, &mut rowb, if white { FMT_PARGB } else { FMT_ARGB });
            for (p, s) in self.work[y * w..y * w + w].iter_mut().zip(rowb.chunks_exact(4)) {
                let c = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                let a = c >> 24;
                *p = if white {
                    c.wrapping_add((a ^ 255) * 0x010101) | 0xFF00_0000
                } else if keep {
                    if a == 0 { 0 } else { c }
                } else if a < self.thresh {
                    tr = true;
                    0
                } else {
                    c | 0xFF00_0000
                };
            }
        }
        self.hastr = tr;
    }

    // qz_exact: the exact palette (pal, trans) and indices when the image has at most max
    // colours; 0 (trans = -1) when there are more
    fn exact(&mut self, idx: &mut [u8]) -> u32 {
        let mut ht = [(1u32, 0u32); 1024]; // colour (1 = empty: no pixel is 1), index
        let (mut prev, mut pidx, mut n) = (1u32, 0u32, 0u32);
        for (o, &c) in idx.iter_mut().zip(&self.work) {
            if c != prev {
                prev = c;
                let mut hs = (c.wrapping_mul(HASHMUL) >> 22) as usize;
                loop {
                    let (hc, hi) = ht[hs];
                    if hc == c {
                        pidx = hi;
                        break;
                    }
                    if hc == 1 {
                        if n >= self.max {
                            self.trans = -1;
                            return 0;
                        }
                        ht[hs] = (c, n);
                        self.pal[n as usize] = c;
                        if c == 0 {
                            self.trans = n as i32;
                        }
                        pidx = n;
                        n += 1;
                        break;
                    }
                    hs = (hs + 1) & 1023;
                }
            }
            *o = pidx as u8;
        }
        n
    }

    // qz_wu: histogram, cumulative moments, k boxes -> pal[0..k-1]; k becomes the number of
    // boxes made (fewer when the colours run out)
    fn wu(&mut self) {
        let mut m = vec![Mom::default(); NCELL];
        for &c in &self.work {
            if c == 0 {
                continue;
            }
            let (r, g, b) = ((c >> 16) & 255, (c >> 8) & 255, c & 255);
            let cell = &mut m[((r >> 3) + 1) as usize * SR + ((g >> 3) + 1) as usize * SG + ((b >> 3) + 1) as usize];
            cell.wt += 1;
            cell.r += r as i64;
            cell.g += g as i64;
            cell.b += b as i64;
            cell.m2 += (r * r + g * g + b * b) as i64;
        }
        // cumulative moments: M[r][g][b] = sum over [1..r][1..g][1..b]
        for r in 1..SIDE {
            let mut area = [Mom::default(); SIDE];
            for g in 1..SIDE {
                let mut line = Mom::default();
                for (b, ar) in area.iter_mut().enumerate().skip(1) {
                    let i = r * SR + g * SG + b;
                    line = line.add(m[i]);
                    *ar = ar.add(line);
                    m[i] = m[i - SR].add(*ar);
                }
            }
        }
        // boxes: split the one with the largest variance until k boxes
        let mut boxes = [WBox::default(); 256];
        let mut vv = [0f64; 256];
        boxes[0] = WBox { lohi: [0, 32, 0, 32, 0, 32], vol: 32 * 32 * 32 };
        let mut i = 1usize; // next box to fill
        let mut next = 0usize; // box to split
        let k = self.k as usize;
        while i < k {
            if cut(&m, &mut boxes, next, i) {
                vv[next] = boxvar(&m, &boxes[next]);
                vv[i] = boxvar(&m, &boxes[i]);
            } else {
                vv[next] = 0.0;
                i -= 1;
            }
            next = 0;
            let mut best = vv[0];
            for (c, &v) in vv.iter().enumerate().take(i + 1).skip(1) {
                if v > best {
                    best = v;
                    next = c;
                }
            }
            i += 1;
            if !(best > 0.0) {
                break; // nothing left to split
            }
        }
        self.k = i as u32;
        // palette = rounded mean of every box
        for (p, b) in self.pal.iter_mut().zip(&boxes).take(i) {
            let w = boxmom(&m, b);
            let mut c = 0xFF00_0000u32;
            if w.wt != 0 {
                let (wt, half) = (w.wt as u64, w.wt as u64 >> 1);
                c |= (((w.r as u64 + half) / wt) as u32) << 16;
                c |= (((w.g as u64 + half) / wt) as u32) << 8;
                c |= ((w.b as u64 + half) / wt) as u32;
            }
            *p = c;
        }
    }

    // qz_map (with qz_prep / qz_search): the index of every pixel = nearest colour of (pixel +
    // diffused error); the error lanes in SSE2 registers as in the assembly
    fn map(&self, idx: &mut [u8]) {
        let (w, h) = (self.w, self.h);
        let k = self.k as usize;
        // qz_prep: palette as B, G, R, 255 i16 lanes; search tables padded with the last colour
        let mut p16 = [0u8; 256 * 8];
        let ngrp = (k + 3) / 4;
        let mut srg = [0u8; 1024]; // u16 r, g per colour
        let mut sb = [0u8; 1024]; // u16 b, 0 per colour
        for d in 0..256 {
            let c = self.pal[if d < k { d } else { k - 1 }];
            let (b, g, r) = (c as u8, (c >> 8) as u8, (c >> 16) as u8);
            p16[d * 8..d * 8 + 8].copy_from_slice(&[b, 0, g, 0, r, 0, 255, 0]);
            srg[d * 4..d * 4 + 4].copy_from_slice(&[r, 0, g, 0]);
            sb[d * 4] = b;
        }
        let (srg, sb) = (&srg[..ngrp * 16], &sb[..ngrp * 16]);
        let dither = self.level != 0;
        let trans = self.trans as u8;
        let x = Sse2;
        let lv = x.splat16(self.level as u16);
        let (c64, c7, c5, c3) = (x.splat16(64), x.splat16(7), x.splat16(5), x.splat16(3));
        let (sign, k256) = (x.splat16(0x8000), x.splat16(256));
        let mut cache = vec![0u16; 1 << 18]; // nearest colour per 64^3 cell (index + 1)
        let ew = (w + 2) * 8;
        let mut err = vec![0u8; 2 * ew]; // two rows of (w + 2) entries of 4 i16 lanes
        for y in 0..h {
            let (cur, nxt) = if y & 1 == 0 { (0, ew) } else { (ew, 0) };
            err[nxt..nxt + ew].fill(0);
            let (mut xi, dir) = if y & 1 == 0 { (0isize, 1isize) } else { (w as isize - 1, -1isize) };
            let row = &self.work[y * w..y * w + w];
            let orow = &mut idx[y * w..y * w + w];
            for _ in 0..w {
                let xu = xi as usize;
                xi += dir;
                let c = row[xu];
                if c == 0 {
                    orow[xu] = trans;
                    continue;
                }
                let mut px = x.widen_px(x.from_i32(c as i32));
                let at = (xu + 1) * 8;
                if dither {
                    // (e * level + 64) >> 7 (psraw = logical shift of the biased value)
                    let e = x.load_raw(&err[cur + at..]);
                    let t = x.add16(x.mullo16(e, lv), c64);
                    px = x.add16(px, x.sub16(x.srl16::<7>(x.xor(t, sign)), k256));
                }
                let q = x.to_i32(x.packus16(px, px)) as u32;
                let cell = ((q >> 6) & 0x3F000 | (q >> 4) & 0xFC0 | (q >> 2) & 0x3F) as usize;
                let mut ci = cache[cell];
                if ci == 0 {
                    let (r, g, b) = ((cell >> 12) * 4 + 2, ((cell >> 6) & 63) * 4 + 2, (cell & 63) * 4 + 2);
                    ci = search(srg, sb, r as u32, g as u32, b as u32) as u16 + 1;
                    cache[cell] = ci;
                }
                let i = (ci - 1) as usize & 255;
                orow[xu] = i as u8;
                if dither {
                    let e = x.sub16(x.widen_px(x.from_i32(q as i32)), x.load_raw(&p16[i * 8..]));
                    let ahead = (at as isize + dir * 8) as usize;
                    let behind = (at as isize - dir * 8) as usize;
                    let s = &mut err[cur + ahead..];
                    x.store_raw(s, x.add16(x.load_raw(s), x.mullo16(e, c7))); // ahead: 7/16
                    let s = &mut err[nxt + ahead..];
                    x.store_raw(s, x.add16(x.load_raw(s), e)); // below ahead: 1/16
                    let s = &mut err[nxt + at..];
                    x.store_raw(s, x.add16(x.load_raw(s), x.mullo16(e, c5))); // below: 5/16
                    let s = &mut err[nxt + behind..];
                    x.store_raw(s, x.add16(x.load_raw(s), x.mullo16(e, c3))); // below behind: 3/16
                }
            }
        }
    }
}

// qz_search: index of the nearest palette colour (squared RGB distance, lowest index on ties);
// SSE2, 4 colours per step: dist << 8 | index, signed dword minimum
#[inline(always)]
fn search(srg: &[u8], sb: &[u8], r: u32, g: u32, b: u32) -> u32 {
    let x = Sse2;
    let rg = x.splat32(r | g << 16);
    let bv = x.splat32(b);
    let four = x.splat32(4);
    let mut lanes = x.load_i32(&[0, 1, 2, 3]);
    let mut best = x.splat32(0x7FFF_FFFF);
    for (a, c) in srg.chunks_exact(16).zip(sb.chunks_exact(16)) {
        let d0 = x.sub16(x.load(a), rg);
        let d1 = x.sub16(x.load(c), bv);
        let dist = x.add32(x.madd16(d0, d0), x.madd16(d1, d1));
        let key = x.or(x.sll32::<8>(dist), lanes);
        lanes = x.add32(lanes, four);
        best = min32(x, best, key);
    }
    best = min32(x, best, x.shuf32::<0x4E>(best));
    best = min32(x, best, x.shuf32::<0xB1>(best));
    x.to_i32(best) as u32 & 0xFF
}

// QZ_MIN: signed dword minimum
#[inline(always)]
fn min32(x: Sse2, a: <Sse2 as Simd>::V, b: <Sse2 as Simd>::V) -> <Sse2 as Simd>::V {
    let t = x.cmpgts32(a, b);
    x.or(x.and(b, t), x.andnot(t, a))
}
