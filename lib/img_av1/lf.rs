// lf -- AV1 deblocking loop filter of intra frames (specification 7.14)   [src/img/av1_lf.s]
//
//   loopfilter(h, f)     loop filter process on CurrFrame: all vertical edges of a plane, then
//                        all horizontal ones, on the worker pool
//
// Intra frames: every block is intra (isIntra = 1), so a transform edge is always filtered
// (applyFilter = isTxEdge).  Per plane: the vertical edges (pass 0) one MI row (or chroma row
// pair: 4 plane rows) per item, then the horizontal edges (pass 1) one strip of LF_STRIP MI
// columns per item, as in the assembly (a vertical edge only changes samples of its rows, a
// horizontal one only samples of its columns).  Pass 0 items own their 4 rows (chunks of the
// plane); a pass 1 item filters a copy of its column strip (all rows), which is copied back
// afterwards: the filter of a column only reads that column, so the result is the same.

use alloc::vec::Vec;

use rt::sync::SpinLock;

use crate::blk::{MIB_DLF, MIB_LTX, MIB_SEG};
use crate::dec::{Frame, Hdr};
use crate::tab::*;

const LF_STRIP: u32 = 16; // MI columns per item of the horizontal pass
const SEG_LVL_ALT_LF_Y_V: usize = 1;

/// the context of the filter items
struct Lf<'a> {
    h: &'a Hdr,
    mi: &'a [[u8; 16]],
    mis: u32,
    plane: u32,
    sx: u32,
    sy: u32,
    bd8: u32, // BitDepth - 8
}

/// a filtered sample buffer: buf[ ( y - oy ) * st + x - ox ] = plane[ y ][ x ]
struct Buf<'b> {
    d: &'b mut [u16],
    st: usize,
    ox: u32,
    oy: u32,
}

/// limit / blimit / thresh of lf_strength
#[derive(Clone, Copy, Default)]
struct Lim {
    limit: u32,
    blimit: u32,
    thresh: u32,
}

/// av1_loopfilter
pub(crate) fn loopfilter(h: &Hdr, f: &mut Frame) {
    let Frame { pl, ps, ph, mi, mis, .. } = f;
    for p in 0..h.np as usize {
        if p > 0 && h.lflvl[p + 1] == 0 {
            continue;
        }
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let st = ps[p] as usize;
        let lf = Lf { h, mi, mis: *mis, plane: p as u32, sx, sy, bd8: h.bd - 8 };
        // pass 0: the vertical edges of MI row index << subY (4 plane rows)
        let n0 = ((h.mirows + sy) >> sy) as usize;
        {
            let chunks: Vec<SpinLock<&mut [u16]>> = pl[p].chunks_mut(4 * st).map(SpinLock::new).collect();
            let item = |i: usize, _: &plat::JobCtl| {
                let mut c = chunks[i].lock();
                let mut b = Buf { d: &mut c, st, ox: 0, oy: 4 * i as u32 };
                let row = (i as u32) << sy;
                if row < h.mirows {
                    let mut col = 0;
                    while col < h.micols {
                        lf.edge(&mut b, 0, row, col);
                        col += 1 << sx;
                    }
                }
            };
            let _ = plat::parallel_for(n0, &item);
        }
        // pass 1: the horizontal edges of MI columns index * LF_STRIP .. + LF_STRIP - 1 (all rows,
        // top down), on a copy of the strip
        let n1 = ((h.micols + LF_STRIP - 1) / LF_STRIP) as usize;
        let rows = ph[p] as usize;
        let strips: Vec<SpinLock<Vec<u16>>> = (0..n1).map(|_| SpinLock::new(Vec::new())).collect();
        let span = |k: usize| {
            let c0 = k as u32 * LF_STRIP;
            let c1 = (c0 + LF_STRIP).min(h.micols);
            (((c0 * 4) >> sx) as usize, ((c1 * 4) >> sx) as usize)
        };
        {
            let plane: &[u16] = &pl[p];
            let item = |k: usize, _: &plat::JobCtl| {
                let (x0, x1) = span(k);
                let w = x1 - x0;
                let mut v: Vec<u16> = Vec::with_capacity(w * rows);
                for y in 0..rows {
                    v.extend_from_slice(&plane[y * st + x0..y * st + x1]);
                }
                let mut b = Buf { d: &mut v, st: w, ox: x0 as u32, oy: 0 };
                let c0 = k as u32 * LF_STRIP;
                let c1 = (c0 + LF_STRIP).min(h.micols);
                let mut row = 0;
                while row < h.mirows {
                    let mut col = c0;
                    while col < c1 {
                        lf.edge(&mut b, 1, row, col);
                        col += 1 << sx;
                    }
                    row += 1 << sy;
                }
                *strips[k].lock() = v;
            };
            let _ = plat::parallel_for(n1, &item);
        }
        // copy the strips back (bands of 64 rows)
        let sv: Vec<Vec<u16>> = strips.iter().map(|l| core::mem::take(&mut *l.lock())).collect();
        let bands: Vec<SpinLock<&mut [u16]>> = pl[p].chunks_mut(64 * st).map(SpinLock::new).collect();
        let item = |i: usize, _: &plat::JobCtl| {
            let mut c = bands[i].lock();
            let y0 = i * 64;
            let n = (c.len() / st).min(rows.saturating_sub(y0));
            for (k, s) in sv.iter().enumerate() {
                let (x0, x1) = span(k);
                let w = x1 - x0;
                for y in 0..n {
                    c[y * st + x0..y * st + x1].copy_from_slice(&s[(y0 + y) * w..(y0 + y) * w + w]);
                }
            }
        };
        let _ = plat::parallel_for(bands.len(), &item);
    }
}

#[inline(always)]
fn absd(a: i32, b: i32) -> u32 {
    a.wrapping_sub(b).unsigned_abs()
}

impl Lf<'_> {
    #[inline(always)]
    fn mib(&self, row: u32, col: u32) -> &[u8; 16] {
        &self.mi[row.wrapping_mul(self.mis).wrapping_add(col) as usize]
    }

    /// lf_edge: edge loop filter process for ( plane, pass, row, col )
    fn edge(&self, b: &mut Buf, pass: u32, row: u32, col: u32) {
        let h = self.h;
        let x = col << 2; // x (luma)
        let y = row << 2;
        if x >= h.fw || y >= h.fh {
            return;
        }
        if pass == 0 {
            if x == 0 {
                return;
            }
        } else if y == 0 {
            return;
        }
        let (sx, sy) = (self.sx, self.sy);
        let xp = x >> sx;
        let yp = y >> sy;
        let row = row | sy;
        let col = col | sx;
        // transform sizes on both sides
        let p = self.plane as usize;
        let tx = self.mib(row >> sy, col >> sx)[MIB_LTX + p] as usize;
        let (prow, pcol) = if pass == 0 { (row, col.wrapping_sub(1 << sx)) } else { (row.wrapping_sub(1 << sy), col) };
        let ptx = self.mib(prow >> sy, pcol >> sx)[MIB_LTX + p] as usize;
        // isTxEdge, baseSize
        let (sz, psz, pos) = if pass == 0 {
            (TX_WIDTH[tx] as u32, TX_WIDTH[ptx] as u32, xp)
        } else {
            (TX_HEIGHT[tx] as u32, TX_HEIGHT[ptx] as u32, yp)
        };
        if pos & sz.wrapping_sub(1) != 0 {
            return; // not a transform edge
        }
        let fsize = psz.min(sz).min(if p == 0 { 16 } else { 8 });
        // filter strength of this block, else of the previous one
        let mut l = Lim::default();
        if self.strength(row, col, pass, &mut l) == 0 && self.strength(prow, pcol, pass, &mut l) == 0 {
            return;
        }
        // samples: x = xP + dy * i, y = yP + dx * i
        let base = (yp - b.oy) as usize * b.st + (xp - b.ox) as usize;
        let (step, along) = if pass == 0 { (1, b.st) } else { (b.st, 1) };
        for k in 0..4 {
            self.sample(b.d, base + k * along, step, fsize, &l);
        }
    }

    /// lf_strength: lvl of the block at ( row, col ) (limit / blimit / thresh into l)
    fn strength(&self, row: u32, col: u32, pass: u32, l: &mut Lim) -> u32 {
        let h = self.h;
        let m = self.mib(row, col);
        let seg = (m[MIB_SEG] & 7) as usize;
        let i = if self.plane == 0 { pass } else { self.plane + 1 } as usize;
        let di = if h.dlfmulti != 0 { i } else { 0 };
        let mut lvl = (m[MIB_DLF + di] as i8 as i32 + h.lflvl[i] as i32).clamp(0, 63); // baseFilterLevel
        if h.segen != 0 && h.fen[seg][SEG_LVL_ALT_LF_Y_V + i] != 0 {
            lvl = (lvl + h.fdata[seg][SEG_LVL_ALT_LF_Y_V + i] as i32).clamp(0, 63);
        }
        if h.lfdeltaen != 0 {
            let ns = (lvl as u32) >> 5; // nShift
            lvl = (lvl + (h.lfrefd[0] as i32).wrapping_shl(ns)).clamp(0, 63); // INTRA_FRAME
        }
        let lvl = lvl as u32;
        let sharp = h.lfsharp;
        let sh = if sharp == 0 {
            0
        } else if sharp <= 4 {
            1
        } else {
            2
        };
        let mut lim = (lvl >> sh) as i32;
        if sharp != 0 {
            lim = lim.min(9 - sharp as i32);
        }
        let lim = lim.max(1) as u32;
        l.limit = lim;
        l.blimit = lim + (lvl + 2) * 2;
        l.thresh = lvl >> 4;
        lvl
    }

    /// lf_sample: sample filtering process at d[q0] across step
    #[inline]
    fn sample(&self, d: &mut [u16], q0: usize, step: usize, fsize: u32, l: &Lim) {
        // s[ 6 - k ] = p_k, s[ 7 + k ] = q_k (only as far as the filter can reach)
        let mut s = [0i32; 16];
        let nl = if fsize < 16 { 4 } else { 7 };
        for k in 0..nl {
            s[7 + k] = d[q0 + k * step] as i32;
            s[6 - k] = d[q0 - (k + 1) * step] as i32;
        }
        let bd8 = self.bd8;
        let thr = l.thresh << bd8;
        let lim = l.limit << bd8;
        let blim = l.blimit << bd8;
        let t1 = 1u32 << bd8; // thresholdBd
        let hev = absd(s[5], s[6]) > thr || absd(s[8], s[7]) > thr;
        let flen = if fsize == 4 {
            4
        } else if self.plane != 0 {
            6
        } else if fsize == 8 {
            8
        } else {
            16
        };
        // filterMask
        if absd(s[5], s[6]) > lim || absd(s[8], s[7]) > lim {
            return;
        }
        if absd(s[6], s[7]) * 2 + (absd(s[5], s[8]) >> 1) > blim {
            return;
        }
        if flen >= 6 && (absd(s[4], s[5]) > lim || absd(s[9], s[8]) > lim) {
            return;
        }
        if flen >= 8 && (absd(s[3], s[4]) > lim || absd(s[10], s[9]) > lim) {
            return;
        }
        // flatMask (filterSize >= 8), flatMask2 (filterSize >= 16)
        let mut flat = false;
        let mut flat2 = false;
        if fsize >= 8 {
            flat = absd(s[5], s[6]) <= t1
                && absd(s[8], s[7]) <= t1
                && absd(s[4], s[6]) <= t1
                && absd(s[9], s[7]) <= t1
                && (flen < 8 || (absd(s[3], s[6]) <= t1 && absd(s[10], s[7]) <= t1));
            if fsize >= 16 {
                flat2 = (4..7).all(|k| absd(s[6 - k], s[6]) <= t1 && absd(s[7 + k], s[7]) <= t1);
            }
        }
        if fsize == 4 || !flat {
            self.narrow(d, q0, step, hev, &s);
        } else {
            let log2 = if fsize == 8 || !flat2 { 3 } else { 4 };
            self.wide(d, q0, step, log2, &s);
        }
    }

    /// lf_narrow: narrow filter process
    #[inline]
    fn narrow(&self, d: &mut [u16], q0: usize, step: usize, hev: bool, s: &[i32; 16]) {
        let hi = (1i32 << (self.h.bd - 1)) - 1;
        let lo = -(1i32 << (self.h.bd - 1));
        let c4 = |v: i32| v.max(lo).min(hi);
        let off = 0x80i32 << self.bd8;
        let ps1 = s[5] - off;
        let ps0 = s[6] - off;
        let qs0 = s[7] - off;
        let qs1 = s[8] - off;
        let mut flt = if hev { c4(ps1 - qs1) } else { 0 };
        flt = c4(flt + (qs0 - ps0) * 3);
        let f1 = c4(flt + 4) >> 3;
        let f2 = c4(flt + 3) >> 3;
        d[q0 - step] = (c4(ps0 + f2) + off) as u16;
        d[q0] = (c4(qs0 - f1) + off) as u16;
        if !hev {
            let f = (f1 + 1) >> 1; // Round2( filter1, 1 )
            d[q0 + step] = (c4(qs1 - f) + off) as u16;
            d[q0 - 2 * step] = (c4(ps1 + f) + off) as u16;
        }
    }

    /// lf_wide: wide filter process
    #[inline]
    fn wide(&self, d: &mut [u16], q0: usize, step: usize, log2: u32, s: &[i32; 16]) {
        let n: i32 = if log2 == 4 {
            6
        } else if self.plane == 0 {
            3
        } else {
            2
        };
        let n2: u32 = if log2 == 3 && self.plane == 0 { 0 } else { 1 };
        let mut out = [0i32; 16];
        for i in -n..n {
            let mut t = 0i32;
            for j in -n..=n {
                let p = (i + j).max(-(n + 1)).min(n);
                let v = s[(p + 7) as usize];
                t += if j.unsigned_abs() <= n2 { v * 2 } else { v };
            }
            out[(i + 7) as usize] = ((t as u32 + ((1u32 << log2) >> 1)) >> log2) as i32;
        }
        for i in -n..n {
            let q = (q0 as isize + i as isize * step as isize) as usize;
            d[q] = out[(i + 7) as usize] as u16;
        }
    }
}
