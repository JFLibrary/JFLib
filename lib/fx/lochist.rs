// lochist -- sliding local histograms for median / percentile / oil / surface style effects
//                                                                         [src/fx/fx_lochist.s]
//
//   lh_run(a, dst, r, shape, alpha0, f)    for every ROI pixel (x, y): the histograms (Hist) of
//                                    the window around it (LHS_DISC = paint.net disc of radius
//                                    r, LHS_SQUARE = (2r+1)^2), then dst(x, y) = f(hist)
//   lh_run_oil(a, dst, r, shape, levels, f)  the same with pixels bucketed by intensity (Oil)
//   lh_percentile(hist, ch, target)  smallest v with count(<= v) >= max(target, 1) (255 when
//                                    fewer pixels)
//   lh_rank(hist, ch, v)             pixels with value < v
//   lh_edges(r, shape) -> (E, area)  half widths of the window rows
//
// Window: pixels outside the image are not counted.  alpha0 (LHF_ALPHA0) keeps pixels with
// alpha 0 out of the colour histograms (ncol counts the rest; the alpha histogram sees every
// pixel, nall).  Oil: bucket = FastScale(I, levels), alpha-0 pixels count as transparent black.
// Traversal: the histogram of the first ROI column slides down one row per ROI row, is
// copied, then slides right along the row; every result depends on the window content only.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::conv::{disc_edges, roi};
use crate::*;

pub const LHS_DISC: u32 = 0;
pub const LHS_SQUARE: u32 = 1;

/// the per-channel histograms of a window (HIST): fine counts B, G, R, A, coarse counts (bins
/// v >> 4), pixel counts and the centre pixel
#[derive(Clone)]
pub struct Hist {
    pub fine: [[u32; 256]; 4],
    pub coarse: [[u32; 16]; 4],
    pub nall: u32,
    pub ncol: u32,
    pub src: u32,
}

/// the intensity buckets of a window (OIL): pixels per bucket, per bucket sum B*A, G*A, R*A, A
#[derive(Clone)]
pub struct Oil {
    pub cnt: [u32; 256],
    pub sum: [[u32; 4]; 256],
    pub n: u32,
    pub src: u32,
}

/// half widths of the window rows E[0..r] and the area  [lh_edges]
pub fn lh_edges(r: i32, shape: u32) -> (Vec<i32>, i32) {
    if shape == LHS_SQUARE {
        let e = alloc::vec![r; r.max(0) as usize + 1];
        return (e, (2 * r + 1) * (2 * r + 1));
    }
    disc_edges(r)
}

/// smallest v with count(<= v) >= max(target, 1) in channel ch, 255 when fewer  [lh_percentile]
pub fn lh_percentile(h: &Hist, ch: usize, target: i32) -> u32 {
    let ch = ch & 3;
    let t = target.max(1);
    let mut cum = 0i32;
    for b in 0..16usize {
        let c = (h.coarse[ch][b] as i32).wrapping_add(cum);
        if c >= t {
            let mut v = b * 16;
            loop {
                cum = cum.wrapping_add(h.fine[ch][v] as i32);
                if cum >= t {
                    return v as u32;
                }
                v += 1;
                if v >= b * 16 + 15 {
                    return v as u32;
                }
            }
        }
        cum = c;
    }
    255
}

/// pixels with channel value < v (v clamped to 0..256)  [lh_rank]
pub fn lh_rank(h: &Hist, ch: usize, v: i32) -> u32 {
    let ch = ch & 3;
    if v <= 0 {
        return 0;
    }
    let v = v.min(256) as usize;
    let mut s = 0u32;
    for b in 0..v >> 4 {
        s = s.wrapping_add(h.coarse[ch][b]);
    }
    for i in (v >> 4) * 16..v {
        s = s.wrapping_add(h.fine[ch][i]);
    }
    s
}

/// how a window record counts pixels
trait Rec: Clone {
    fn upd(&mut self, p: u32, add: bool, mask: u32);
    fn set_src(&mut self, p: u32);
}

impl Rec for Hist {
    #[inline(always)]
    fn upd(&mut self, p: u32, add: bool, mask: u32) {
        let a = (p >> 24) as usize;
        let d = |v: &mut u32| *v = if add { v.wrapping_add(1) } else { v.wrapping_sub(1) };
        d(&mut self.fine[3][a]);
        d(&mut self.coarse[3][a >> 4]);
        d(&mut self.nall);
        if (p | mask) & 0xFF00_0000 != 0 {
            for c in 0..3 {
                let v = (p >> (8 * c) & 255) as usize;
                d(&mut self.fine[c][v]);
                d(&mut self.coarse[c][v >> 4]);
            }
            d(&mut self.ncol);
        }
    }
    #[inline(always)]
    fn set_src(&mut self, p: u32) {
        self.src = p;
    }
}

impl Rec for Oil {
    #[inline(always)]
    fn upd(&mut self, p: u32, add: bool, levels: u32) {
        let p = if p & 0xFF00_0000 == 0 { 0 } else { p };
        let i = crate::lut::luma(p);
        let t = i.wrapping_mul(levels).wrapping_add(128);
        let b = ((t >> 8).wrapping_add(t) >> 8 & 255) as usize;
        let a = p >> 24;
        let v = [(p & 255) * a & 0xFFFF, (p >> 8 & 255) * a & 0xFFFF, (p >> 16 & 255) * a & 0xFFFF, a];
        let s = &mut self.sum[b];
        if add {
            self.cnt[b] = self.cnt[b].wrapping_add(1);
            self.n = self.n.wrapping_add(1);
            for c in 0..4 {
                s[c] = s[c].wrapping_add(v[c]);
            }
        } else {
            self.cnt[b] = self.cnt[b].wrapping_sub(1);
            self.n = self.n.wrapping_sub(1);
            for c in 0..4 {
                s[c] = s[c].wrapping_sub(v[c]);
            }
        }
    }
    #[inline(always)]
    fn set_src(&mut self, p: u32) {
        self.src = p;
    }
}

/// the traversal (LH_BODY) over the ROI clamped to the image
fn body<R: Rec>(a: &FxArgs, dst: &mut Dst, r: i32, shape: u32, mask: u32, h0: &mut R, hw: &mut R, mut f: impl FnMut(&R, i32, i32) -> u32) -> i32 {
    let Some(rc) = roi(a) else {
        return 0;
    };
    let r = r.clamp(0, 1000);
    let (w, h) = (a.src.w, a.src.h);
    let stride = a.src.stride as usize;
    let sp = &a.src.px;
    let at = |x: i32, y: i32| -> u32 { px(&sp[y as usize * stride..], x as usize) };
    let (e, _) = lh_edges(r, shape);
    let [x0, y0, x1, y1] = rc;
    let mut offs: Vec<(usize, usize)> = Vec::with_capacity(2 * r as usize + 1);
    let rd = |o: usize| -> u32 {
        let q: [u8; 4] = sp[o..o + 4].try_into().unwrap();
        u32::from_le_bytes(q)
    };
    // the initial window at (x0, y0)
    for v in -r..=r {
        let y = y0 + v;
        if y < 0 || y >= h {
            continue;
        }
        let ev = e[v.unsigned_abs() as usize];
        let end = (x0 + ev + 1).min(w);
        for x in (x0 - ev).max(0)..end {
            h0.upd(at(x, y), true, mask);
        }
    }
    for y in y0..y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        if y != y0 {
            // slide the column histogram down: row y-1 -> y
            for u in -r..=r {
                let col = x0 + u;
                if col < 0 || col >= w {
                    continue;
                }
                let ev = e[u.unsigned_abs() as usize];
                let leave = y - 1 - ev;
                if leave >= 0 {
                    h0.upd(at(col, leave), false, mask);
                }
                let enter = y + ev;
                if enter < h {
                    h0.upd(at(col, enter), true, mask);
                }
            }
        }
        hw.clone_from(h0);
        let vlo = (-r).max(-y);
        let vhi = (h - 1 - y).min(r);
        // interior columns (x - 1 - r >= 0, x + r < w): every window row loses its pixel at
        // x - 1 - e and gains the one at x + e; (leave, enter) byte offsets less x * 4.  The
        // counts do not depend on the order of the updates; an equal pair changes nothing.
        offs.clear();
        for v in vlo..=vhi {
            let ev = e[v.unsigned_abs() as usize] as usize;
            let ro = (y + v) as usize * stride;
            offs.push((ro.wrapping_sub((ev + 1) * 4), ro + ev * 4));
        }
        let drow = dst.row(y);
        let srow = y as usize * stride;
        for x in x0..x1 {
            if x != x0 && x - 1 - r >= 0 && x + r < w {
                let b = x as usize * 4;
                for &(ol, oe) in &offs {
                    let (pl, pe) = (rd(ol.wrapping_add(b)), rd(oe + b));
                    if pl != pe {
                        hw.upd(pl, false, mask);
                        hw.upd(pe, true, mask);
                    }
                }
            } else if x != x0 {
                // slide right: column x-1 -> x
                for v in vlo..=vhi {
                    let ev = e[v.unsigned_abs() as usize];
                    let ro = (y + v) as usize * stride;
                    let leave = x - 1 - ev;
                    if leave >= 0 {
                        hw.upd(px(&sp[ro..], leave as usize), false, mask);
                    }
                    let enter = x + ev;
                    if enter < w {
                        hw.upd(px(&sp[ro..], enter as usize), true, mask);
                    }
                }
            }
            hw.set_src(px(&sp[srow..], x as usize));
            let o = f(hw, x, y);
            put(drow, (x - a.x0) as usize, o);
        }
    }
    0
}

/// histograms of every ROI pixel's window -> dst = f(hist, x, y)  [lh_run, LHK_HIST]
pub fn lh_run(a: &FxArgs, dst: &mut Dst, r: i32, shape: u32, alpha0: bool, f: impl FnMut(&Hist, i32, i32) -> u32) -> i32 {
    let z = || Box::new(Hist { fine: [[0; 256]; 4], coarse: [[0; 16]; 4], nall: 0, ncol: 0, src: 0 });
    let (mut h0, mut hw) = (z(), z());
    let mask = if alpha0 { 0 } else { 0xFF00_0000 };
    body(a, dst, r, shape, mask, &mut *h0, &mut *hw, f)
}

/// intensity buckets of every ROI pixel's window -> dst = f(oil, x, y)  [lh_run, LHK_OIL]
pub fn lh_run_oil(a: &FxArgs, dst: &mut Dst, r: i32, shape: u32, levels: i32, f: impl FnMut(&Oil, i32, i32) -> u32) -> i32 {
    let z = || Box::new(Oil { cnt: [0; 256], sum: [[0; 4]; 256], n: 0, src: 0 });
    let (mut h0, mut hw) = (z(), z());
    body(a, dst, r, shape, levels.clamp(1, 255) as u32, &mut *h0, &mut *hw, f)
}
