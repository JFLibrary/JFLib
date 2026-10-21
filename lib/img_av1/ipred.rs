// ipred -- AV1 intra prediction (specification 7.11.2, 7.11.4, 7.11.5)  [src/img/av1_ipred.s]
//
//   T::predict()    predict_intra( ppl, px, py, phl, pha, phar, phbl, pmode, plw, plh ):
//                   recursive (filter intra), directional (edge filter / upsampling), smooth,
//                   DC, Paeth
//   T::palpred()    predict_palette( ppl, px, py, pbx, pby, ptx )
//   T::cfl()        predict_chroma_from_luma( ppl, px, py, ptx )
//
// The edge arrays AboveRow / LeftCol are i32 in Ts::edge (index + 32, as the assembly's TS_EDGE:
// EA = AboveRow[ 0 ], EL = LeftCol[ 0 ]).  Predictions go straight into the u16 plane.

use crate::blk::{T, D67_PRED, V_PRED};
use crate::tab::*;

const EA: usize = 32; // AboveRow[ 0 ]
const EL: usize = 320 + 32; // LeftCol[ 0 ]
const SMOOTH_PRED: u32 = 9;
const SMOOTH_V_PRED: u32 = 10;
const SMOOTH_H_PRED: u32 = 11;

static SMW: [&[u8]; 5] = [&SM_WEIGHTS_TX_4X4, &SM_WEIGHTS_TX_8X8, &SM_WEIGHTS_TX_16X16, &SM_WEIGHTS_TX_32X32, &SM_WEIGHTS_TX_64X64];

/// the locals of av1_predict (PR_*)
struct Pr {
    w: u32,
    h: u32,
    maxx: i32,
    maxy: i32,
    stride: usize,
    plane: u32,
    x: u32,
    y: u32,
    dst: usize, // index of CurrFrame[ plane ][ y ][ x ]
    maxv: i32,  // ( 1 << BitDepth ) - 1
}

#[inline(always)]
fn ix(base: usize, i: i32) -> usize {
    (base as isize + i as isize) as usize
}

impl T<'_> {
    /// av1_predict
    pub fn predict(&mut self) {
        let h = self.h;
        let p = self.s.ppl as usize;
        let (mut mx, mut my) = (h.micols << 2, h.mirows << 2);
        if p != 0 {
            mx >>= h.ssx;
            my >>= h.ssy;
        }
        let stride = self.f.ps[p] as usize;
        let x = self.s.px;
        let y = self.s.py;
        let pr = Pr {
            w: 1 << self.s.plw,
            h: 1 << self.s.plh,
            maxx: mx as i32 - 1,
            maxy: my as i32 - 1,
            stride,
            plane: p as u32,
            x,
            y,
            dst: y as usize * stride + x as usize,
            maxv: (1i32 << h.bd) - 1,
        };
        let wh = (pr.w + pr.h) as usize;
        let pl = &self.f.pl[p];
        let s = &mut *self.s;
        let e = &mut s.edge;
        let d = pr.dst;
        // AboveRow[ 0 .. w + h - 1 ]
        if s.pha == 0 {
            let mut v = (1i32 << (h.bd - 1)) - 1;
            if s.phl != 0 {
                v = pl[d - 1] as i32; // CurrFrame[ y ][ x - 1 ]
            }
            e[EA..EA + wh].fill(v);
        } else {
            let n = if s.phar != 0 { pr.w * 2 } else { pr.w };
            let lim = (x as i32 + n as i32 - 1).min(pr.maxx) - x as i32; // aboveLimit - x
            let row = d - stride;
            for i in 0..wh {
                e[EA + i] = pl[row + (i as i32).min(lim) as usize] as i32;
            }
        }
        // LeftCol[ 0 .. w + h - 1 ]
        if s.phl == 0 {
            let mut v = (1i32 << (h.bd - 1)) + 1;
            if s.pha != 0 {
                v = pl[d - stride] as i32; // CurrFrame[ y - 1 ][ x ]
            }
            e[EL..EL + wh].fill(v);
        } else {
            let n = if s.phbl != 0 { pr.h * 2 } else { pr.h };
            let lim = (y as i32 + n as i32 - 1).min(pr.maxy) - y as i32; // leftLimit - y
            for i in 0..wh {
                e[EL + i] = pl[d + (i as i32).min(lim) as usize * stride - 1] as i32;
            }
        }
        let corner = if s.pha != 0 {
            (if s.phl != 0 { pl[d - stride - 1] } else { pl[d - stride] }) as i32
        } else if s.phl != 0 {
            pl[d - 1] as i32
        } else {
            1i32 << (h.bd - 1)
        };
        e[EA - 1] = corner;
        e[EL - 1] = corner;
        // dispatch
        let mode = s.pmode;
        if p == 0 && s.ufi != 0 {
            self.recursive(&pr);
        } else if mode.wrapping_sub(V_PRED) <= D67_PRED - V_PRED {
            self.directional(&pr);
        } else if mode < SMOOTH_PRED {
            self.dc(&pr);
        } else if mode <= SMOOTH_H_PRED {
            self.smooth(&pr, mode);
        } else {
            self.paeth(&pr);
        }
    }

    /// pr_paeth: basic (Paeth) prediction
    fn paeth(&mut self, pr: &Pr) {
        let e = &self.s.edge;
        let dst = &mut self.f.pl[pr.plane as usize];
        let tl = e[EA - 1];
        let mut row = pr.dst;
        for i in 0..pr.h as usize {
            let l = e[EL + i];
            for j in 0..pr.w as usize {
                let a = e[EA + j];
                let base = a.wrapping_add(l).wrapping_sub(tl);
                let pl = base.wrapping_sub(l).unsigned_abs();
                let pt = base.wrapping_sub(a).unsigned_abs();
                let ptl = base.wrapping_sub(tl).unsigned_abs();
                let v = if pl <= pt && pl <= ptl {
                    l
                } else if pt <= ptl {
                    a
                } else {
                    tl
                };
                dst[row + j] = v as u16;
            }
            row += pr.stride;
        }
    }

    /// pr_dc: DC prediction
    fn dc(&mut self, pr: &Pr) {
        let s = &*self.s;
        let e = &s.edge;
        let (w, hh) = (pr.w, pr.h);
        let mut ls = 0u32;
        for i in 0..hh as usize {
            ls = ls.wrapping_add(e[EL + i] as u32);
        }
        let mut as_ = 0u32;
        for i in 0..w as usize {
            as_ = as_.wrapping_add(e[EA + i] as u32);
        }
        let v = if s.phl != 0 {
            if s.pha != 0 {
                let n = w + hh;
                (ls.wrapping_add(as_).wrapping_add(n >> 1)) / n
            } else {
                (ls.wrapping_add(hh >> 1) >> s.plh).min(pr.maxv as u32)
            }
        } else if s.pha != 0 {
            (as_.wrapping_add(w >> 1) >> s.plw).min(pr.maxv as u32)
        } else {
            1u32 << (self.h.bd - 1)
        };
        let dst = &mut self.f.pl[pr.plane as usize];
        let mut row = pr.dst;
        for _ in 0..hh {
            dst[row..row + w as usize].fill(v as u16);
            row += pr.stride;
        }
    }

    /// pr_smooth: SMOOTH / SMOOTH_V / SMOOTH_H prediction
    fn smooth(&mut self, pr: &Pr, mode: u32) {
        let s = &*self.s;
        let e = &s.edge;
        let wx = SMW[s.plw as usize - 2];
        let wy = SMW[s.plh as usize - 2];
        let bl = e[EL + pr.h as usize - 1]; // LeftCol[ h - 1 ]
        let tr = e[EA + pr.w as usize - 1]; // AboveRow[ w - 1 ]
        let dst = &mut self.f.pl[pr.plane as usize];
        let mut row = pr.dst;
        for i in 0..pr.h as usize {
            let wyi = wy[i] as i32;
            let l = e[EL + i];
            for j in 0..pr.w as usize {
                let v = if mode == SMOOTH_PRED {
                    let wxj = wx[j] as i32;
                    let t = e[EA + j]
                        .wrapping_mul(wyi)
                        .wrapping_add((256 - wyi).wrapping_mul(bl))
                        .wrapping_add(wxj.wrapping_mul(l))
                        .wrapping_add((256 - wxj).wrapping_mul(tr))
                        .wrapping_add(256);
                    (t as u32) >> 9
                } else if mode == SMOOTH_V_PRED {
                    let t = e[EA + j].wrapping_mul(wyi).wrapping_add((256 - wyi).wrapping_mul(bl)).wrapping_add(128);
                    (t as u32) >> 8
                } else {
                    let wxj = wx[j] as i32;
                    let t = l.wrapping_mul(wxj).wrapping_add((256 - wxj).wrapping_mul(tr)).wrapping_add(128);
                    (t as u32) >> 8
                };
                dst[row + j] = v as u16;
            }
            row += pr.stride;
        }
    }

    /// pr_recursive: recursive intra prediction (filter intra, luma)
    fn recursive(&mut self, pr: &Pr) {
        let s = &*self.s;
        let e = &s.edge;
        let taps = &INTRA_FILTER_TAPS[s.fim as usize * 56..];
        let dst = &mut self.f.pl[pr.plane as usize];
        let st = pr.stride;
        let b0 = pr.dst;
        let at = |r: i32, c: i32| ix(b0, 0).wrapping_add((r as isize * st as isize + c as isize) as usize);
        for i2 in 0..(pr.h >> 1) as i32 {
            for j4 in 0..(pr.w >> 2) as i32 {
                let mut pv = [0i32; 7];
                for (i, pp) in pv.iter_mut().enumerate() {
                    let i = i as i32;
                    *pp = if i < 5 {
                        if i2 == 0 {
                            e[ix(EA, j4 * 4 + i - 1)]
                        } else if j4 == 0 && i == 0 {
                            e[ix(EL, i2 * 2 - 1)]
                        } else {
                            dst[at(i2 * 2 - 1, j4 * 4 + i - 1)] as i32
                        }
                    } else if j4 == 0 {
                        e[ix(EL, i2 * 2 + i - 5)]
                    } else {
                        dst[at(i2 * 2 + i - 5, j4 * 4 - 1)] as i32
                    };
                }
                // 4x2 outputs
                for k in 0..8 {
                    let mut sum = 0i32;
                    for t in 0..7 {
                        sum = sum.wrapping_add((taps[k * 7 + t] as i32).wrapping_mul(pv[t]));
                    }
                    // Clip1( Round2Signed( pr, 4 ) )
                    let v = if sum >= 0 {
                        sum.wrapping_add(8) >> 4
                    } else {
                        -(sum.wrapping_neg().wrapping_add(8) >> 4)
                    };
                    let v = v.max(0).min(pr.maxv);
                    dst[at(i2 * 2 + (k >> 2) as i32, j4 * 4 + (k & 3) as i32)] = v as u16;
                }
            }
        }
    }

    /// pr_filtertype: get_filter_type( plane )
    fn filtertype(&self, p: u32) -> u32 {
        let h = self.h;
        let s = &*self.s;
        let (r, c) = (s.mirow, s.micol);
        let mut res = 0;
        let au = if p != 0 { s.auc } else { s.au };
        if au != 0 {
            let mut rr = r - 1;
            let mut cc = c;
            if p != 0 {
                if h.ssx != 0 && c & 1 == 0 {
                    cc += 1;
                }
                if h.ssy != 0 && r & 1 != 0 {
                    rr = rr.wrapping_sub(1);
                }
            }
            res |= self.is_smooth(rr, cc, p);
        }
        let al = if p != 0 { s.alc } else { s.al };
        if al != 0 {
            let mut rr = r;
            let mut cc = c - 1;
            if p != 0 {
                if h.ssx != 0 && c & 1 != 0 {
                    cc = cc.wrapping_sub(1);
                }
                if h.ssy != 0 && r & 1 == 0 {
                    rr += 1;
                }
            }
            res |= self.is_smooth(rr, cc, p);
        }
        res
    }

    // ft_smooth: is_smooth of the MIB at (r, c)
    fn is_smooth(&self, r: u32, c: u32, p: u32) -> u32 {
        let m = &self.f.mi[self.mib(r, c)];
        let mode = if p != 0 { m[crate::blk::MIB_UVMODE] } else { m[crate::blk::MIB_YMODE] } as u32;
        (mode.wrapping_sub(SMOOTH_PRED) <= SMOOTH_H_PRED - SMOOTH_PRED) as u32
    }

    /// pr_edgefilter: intra edge filter process of buf (EA / EL), sz, strength
    fn edgefilter(&mut self, buf: usize, sz: u32, strength: u32) {
        if strength == 0 {
            return;
        }
        let sz = sz.min(129) as usize;
        let k = &INTRA_EDGE_KERNEL[(((strength - 1) & 3) * 5) as usize..];
        let e = &mut self.s.edge;
        let mut edge = [0i32; 136];
        for i in 0..sz {
            edge[i] = e[buf + i - 1]; // edge[ i ] = buf[ i - 1 ]
        }
        for i in 1..sz {
            let mut s = 0i32;
            for j in 0..5 {
                let q = (i as i32 + j as i32 - 2).max(0).min(sz as i32 - 1) as usize;
                s = s.wrapping_add((k[j] as i32).wrapping_mul(edge[q]));
            }
            e[buf + i - 1] = s.wrapping_add(8) >> 4;
        }
    }

    /// pr_upsample: intra edge upsample process of buf, numPx
    fn upsample(&mut self, buf: usize, num: u32, maxv: i32) {
        let n = num.min(32) as usize;
        let e = &mut self.s.edge;
        let mut dup = [0i32; 40];
        dup[0] = e[buf - 1];
        for i in -1..n as isize {
            dup[(i + 2) as usize] = e[(buf as isize + i) as usize];
        }
        dup[n + 2] = e[buf + n - 1];
        e[buf - 2] = dup[0];
        for i in 0..n {
            let s = (dup[i + 1].wrapping_add(dup[i + 2]))
                .wrapping_mul(9)
                .wrapping_sub(dup[i])
                .wrapping_sub(dup[i + 3]);
            let s = (s.wrapping_add(8) >> 4).max(0).min(maxv);
            e[buf + 2 * i - 1] = s;
            e[buf + 2 * i] = dup[i + 2];
        }
    }

    /// pr_directional: directional intra prediction process
    fn directional(&mut self, pr: &Pr) {
        let h = self.h;
        let s = &*self.s;
        let ad = if pr.plane != 0 { s.aduv } else { s.ady };
        let pa = MODE_TO_ANGLE[s.pmode as usize] as i32 + ad * 3; // pAngle
        let (w, hh) = (pr.w, pr.h);
        let mut ft = 0;
        if h.eief != 0 && pa != 90 && pa != 180 {
            if pa > 90 && pa < 180 && w + hh >= 24 {
                let e = &mut self.s.edge;
                let v = (e[EL].wrapping_add(e[EA]).wrapping_mul(5).wrapping_add(e[EA - 1].wrapping_mul(6)).wrapping_add(8)) >> 4;
                e[EA - 1] = v;
                e[EL - 1] = v;
            }
            ft = self.filtertype(pr.plane);
            if self.s.pha != 0 {
                let st = strength(w + hh, ft, pa - 90);
                let mut n = (pr.maxx - pr.x as i32 + 1).min(w as i32);
                if pa < 90 {
                    n += hh as i32;
                }
                self.edgefilter(EA, (n + 1) as u32, st);
            }
            if self.s.phl != 0 {
                let st = strength(w + hh, ft, pa - 180);
                let mut n = (pr.maxy - pr.y as i32 + 1).min(hh as i32);
                if pa > 180 {
                    n += w as i32;
                }
                self.edgefilter(EL, (n + 1) as u32, st);
            }
        }
        let mut upa = 0;
        let mut upl = 0;
        if h.eief != 0 {
            upa = upsel(w + hh, ft, pa - 90);
            if upa != 0 {
                let n = if pa < 90 { w + hh } else { w };
                self.upsample(EA, n, pr.maxv);
            }
            upl = upsel(w + hh, ft, pa - 180);
            if upl != 0 {
                let n = if pa > 180 { hh + w } else { hh };
                self.upsample(EL, n, pr.maxv);
            }
        }
        let mut dx = 0i32;
        let mut dy = 0i32;
        if pa < 90 {
            dx = DR_INTRA_DERIVATIVE[pa as usize] as i32;
        } else if pa > 90 && pa < 180 {
            // (the assembly also loads it for 90, past the table; unused there)
            dx = DR_INTRA_DERIVATIVE[(180 - pa) as usize] as i32;
        }
        if pa > 90 && pa < 180 {
            dy = DR_INTRA_DERIVATIVE[(pa - 90) as usize] as i32;
        } else if pa > 180 {
            dy = DR_INTRA_DERIVATIVE[(270 - pa) as usize] as i32;
        }
        let e = &self.s.edge;
        let dst = &mut self.f.pl[pr.plane as usize];
        let st = pr.stride;
        let interp = |a: i32, b: i32, sh: i32| (a << 5).wrapping_add(b.wrapping_sub(a).wrapping_mul(sh)).wrapping_add(16) >> 5;
        if pa < 90 {
            // zone 1 (above row)
            let maxbx = ((w + hh - 1) << upa) as i32;
            let mut row = pr.dst;
            for i in 0..hh as i32 {
                let idx = (i + 1) * dx;
                let sh = (idx.wrapping_shl(upa) >> 1) & 31;
                let b0 = idx >> (6 - upa);
                for j in 0..w as usize {
                    let b = b0 + ((j as i32) << upa);
                    let v = if b < maxbx {
                        interp(e[ix(EA, b)], e[ix(EA, b + 1)], sh)
                    } else {
                        e[ix(EA, maxbx)]
                    };
                    dst[row + j] = v as u16;
                }
                row += st;
            }
        } else if pa == 90 {
            let mut row = pr.dst;
            for _ in 0..hh {
                for j in 0..w as usize {
                    dst[row + j] = e[EA + j] as u16;
                }
                row += st;
            }
        } else if pa < 180 {
            // zone 2
            let mut row = pr.dst;
            for i in 0..hh as i32 {
                for j in 0..w as i32 {
                    let idx = (j << 6) - (i + 1) * dx;
                    let b = idx >> (6 - upa);
                    let v = if b >= -(1 << upa) {
                        let sh = (idx.wrapping_shl(upa) >> 1) & 31;
                        interp(e[ix(EA, b)], e[ix(EA, b + 1)], sh)
                    } else {
                        let idx = (i << 6) - (j + 1) * dy;
                        let b = idx >> (6 - upl);
                        let sh = (idx.wrapping_shl(upl) >> 1) & 31;
                        interp(e[ix(EL, b)], e[ix(EL, b + 1)], sh)
                    };
                    dst[row + j as usize] = v as u16;
                }
                row += st;
            }
        } else if pa == 180 {
            let mut row = pr.dst;
            for i in 0..hh as usize {
                dst[row..row + w as usize].fill(e[EL + i] as u16);
                row += st;
            }
        } else {
            // zone 3 (left column)
            for j in 0..w as i32 {
                let idx = (j + 1) * dy;
                let sh = (idx.wrapping_shl(upl) >> 1) & 31;
                let b0 = idx >> (6 - upl);
                let mut row = pr.dst;
                for i in 0..hh as i32 {
                    let b = b0 + (i << upl);
                    let l = e[ix(EL, b)];
                    let v = l.wrapping_mul(32).wrapping_add(e[ix(EL, b + 1)].wrapping_sub(l).wrapping_mul(sh)).wrapping_add(16) >> 5;
                    dst[row + j as usize] = v as u16;
                    row += st;
                }
            }
        }
    }

    /// av1_palpred: palette prediction of one transform block
    pub fn palpred(&mut self) {
        let s = &*self.s;
        let p = s.ppl as usize;
        let pal = match p {
            0 => &s.paly,
            1 => &s.palu,
            _ => &s.palv,
        };
        let map = if p == 0 { &s.cmapy } else { &s.cmapuv };
        let tx = s.ptx as usize;
        let (w, hh) = (TX_WIDTH[tx] as usize, TX_HEIGHT[tx] as usize);
        let st = self.f.ps[p] as usize;
        let dst = &mut self.f.pl[p];
        let mut row = s.py as usize * st + s.px as usize;
        let mut mo = (((s.pby << 2) & 63) << 6) as usize + ((s.pbx << 2) & 63) as usize;
        for _ in 0..hh {
            for j in 0..w {
                dst[row + j] = pal[(map[mo + j] & 7) as usize];
            }
            row += st;
            mo += 64;
        }
    }

    /// av1_cfl: predict_chroma_from_luma of one transform block
    pub fn cfl(&mut self) {
        let h = self.h;
        let s = &mut *self.s;
        let p = s.ppl as usize;
        let tx = s.ptx as usize;
        let (w, hh) = (TX_WIDTH[tx] as u32, TX_HEIGHT[tx] as u32);
        let (sx, sy) = (h.ssx, h.ssy);
        let luma = &self.f.pl[0];
        let ls = self.f.ps[0] as usize;
        let mut avg = 0u32;
        for i in 0..hh {
            let ly = ((s.py + i) << sy).min(s.maxlh.wrapping_sub(1 << sy)) as usize;
            for j in 0..w {
                let lx = ((s.px + j) << sx).min(s.maxlw.wrapping_sub(1 << sx)) as usize;
                let q = ly * ls + lx;
                let mut t = luma[q] as u32;
                if sx != 0 {
                    t += luma[q + 1] as u32;
                }
                if sy != 0 {
                    t += luma[q + ls] as u32;
                    if sx != 0 {
                        t += luma[q + ls + 1] as u32;
                    }
                }
                let v = t << (3 - sx - sy);
                s.cfll[(i * 32 + j) as usize] = v as i32;
                avg = avg.wrapping_add(v);
            }
        }
        let sh = TX_WIDTH_LOG2[tx] as u32 + TX_HEIGHT_LOG2[tx] as u32;
        let avg = (avg.wrapping_add((1u32 << sh) >> 1) >> sh) as i32; // lumaAvg
        let alpha = if p == 1 { s.cflu } else { s.cflv };
        let mx = (1i32 << h.bd) - 1;
        let st = self.f.ps[p] as usize;
        let dst = &mut self.f.pl[p];
        let mut row = s.py as usize * st + s.px as usize;
        for i in 0..hh as usize {
            for j in 0..w as usize {
                let d = alpha.wrapping_mul(s.cfll[i * 32 + j].wrapping_sub(avg));
                // Round2Signed( ., 6 )
                let r = if d >= 0 { d.wrapping_add(32) >> 6 } else { -(d.wrapping_neg().wrapping_add(32) >> 6) };
                let v = (r.wrapping_add(dst[row + j] as i32)).max(0).min(mx);
                dst[row + j] = v as u16;
            }
            row += st;
        }
    }
}

/// pr_strength: intra edge filter strength
fn strength(wh: u32, ft: u32, delta: i32) -> u32 {
    let d = delta.unsigned_abs();
    if ft == 0 {
        if wh <= 8 {
            (d >= 56) as u32
        } else if wh <= 16 {
            (d >= 40) as u32
        } else if wh <= 24 {
            if d < 8 {
                0
            } else if d < 16 {
                1
            } else if d < 32 {
                2
            } else {
                3
            }
        } else if wh <= 32 {
            if d < 4 {
                1
            } else if d < 32 {
                2
            } else {
                3
            }
        } else {
            3
        }
    } else if wh <= 8 {
        if d < 40 {
            0
        } else if d < 64 {
            1
        } else {
            2
        }
    } else if wh <= 16 {
        if d < 20 {
            0
        } else if d < 48 {
            1
        } else {
            2
        }
    } else if wh <= 24 {
        if d < 4 { 0 } else { 3 }
    } else {
        3
    }
}

/// pr_upsel: useUpsample
fn upsel(wh: u32, ft: u32, delta: i32) -> u32 {
    let d = delta.unsigned_abs();
    if d == 0 || d >= 40 {
        return 0;
    }
    let lim = if ft != 0 { 8 } else { 16 };
    (wh <= lim) as u32
}
