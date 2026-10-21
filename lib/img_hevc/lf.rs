// lf -- HEVC intra decoder: deblocking filter and SAO                [src/img/hevc_lf.s]
//
//   deblock()            8.7.2 for an all-intra picture (bS 2 on every TU / PU / CU edge of the
//                        8x8 grid): vertical edges of the whole picture, then horizontal edges;
//                        in place (edges 8 samples apart never share samples)  [hvl_deblock]
//   sao() -> 0           8.7.3 band / edge offsets per CTB on a copy of each plane  [hvl_sao]
//
// Edge flags (medge), QpY (mqp), filter-off flags (mbyp) and the per-CTB slice data (cslice,
// cdb, cbeta, csao) come from ctu.rs.  The assembly keeps the state of one edge segment in
// the HD_* fields of the record; here it is the Seg value passed along.

use alloc::vec::Vec;

use crate::dec::{Hevc, MB_BYPASS, MB_PCMOFF};
use crate::rec::QPC;

/// beta' for Q 0 .. 51
static BETA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 22, 24, //
    26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, //
    58, 60, 62, 64,
];
/// tC' for Q 0 .. 53
static TC: [u8; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, //
    0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3, //
    3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, //
    14, 16, 18, 20, 22, 24,
];
/// (dx, dy) of neighbour a per EO class
static EO: [[i8; 2]; 4] = [[-1, 0], [0, -1], [-1, -1], [1, -1]];
/// edgeIdx 0..4 -> SaoOffsetVal index (0 = none)
static EOMAP: [u8; 8] = [1, 2, 0, 3, 4, 0, 0, 0];

/// one edge segment (HD_*)
struct Seg {
    x: u32, // q position (luma)
    y: u32,
    px: u32, // p position
    py: u32,
    dir: u32, // 0 vertical edges, 1 horizontal
    nop: bool, // p side not modified
    noq: bool,
}

/// an edge segment's sample access: q0 index, steps across / along the edge
#[derive(Clone, Copy)]
struct Line {
    q0: usize,
    a: usize, // across (in samples)
}

impl Line {
    #[inline(always)]
    fn p(&self, pl: &[u16], i: usize) -> i32 {
        pl[self.q0 - self.a * (i + 1)] as i32
    }
    #[inline(always)]
    fn q(&self, pl: &[u16], i: usize) -> i32 {
        pl[self.q0 + self.a * i] as i32
    }
    #[inline(always)]
    fn setp(&self, pl: &mut [u16], i: usize, v: i32) {
        pl[self.q0 - self.a * (i + 1)] = v as u16;
    }
    #[inline(always)]
    fn setq(&self, pl: &mut [u16], i: usize, v: i32) {
        pl[self.q0 + self.a * i] = v as u16;
    }
}

/// Clip3(lo, hi, v) as the CLAMP macro (lo first)
#[inline(always)]
fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

#[inline(always)]
fn iabs(v: i32) -> i32 {
    if v < 0 { v.wrapping_neg() } else { v }
}

/// strongf: strong luma filter: p0..p2, q0..q2 clipped to +-2 tc
fn strongf(pl: &mut [u16], ln: Line, tc: i32, nop: bool, noq: bool) {
    let (p0, p1, p2, p3) = (ln.p(pl, 0), ln.p(pl, 1), ln.p(pl, 2), ln.p(pl, 3));
    let (q0, q1, q2, q3) = (ln.q(pl, 0), ln.q(pl, 1), ln.q(pl, 2), ln.q(pl, 3));
    let t2 = tc + tc;
    let clip = |v: i32, o: i32| clamp(v, o - t2, o + t2);
    if !nop {
        ln.setp(pl, 0, clip(((p1 + p0 + q0) * 2 + p2 + q1 + 4) >> 3, p0));
        ln.setp(pl, 1, clip((p2 + p1 + 2 + p0 + q0) >> 2, p1));
        ln.setp(pl, 2, clip((p3 * 2 + p2 * 3 + p1 + p0 + q0 + 4) >> 3, p2));
    }
    if !noq {
        ln.setq(pl, 0, clip(((p0 + q0 + q1) * 2 + p1 + q2 + 4) >> 3, q0));
        ln.setq(pl, 1, clip((p0 + q0 + 2 + q1 + q2) >> 2, q1));
        ln.setq(pl, 2, clip((q3 * 2 + q2 * 3 + p0 + q0 + q1 + 4) >> 3, q2));
    }
}

/// weakf: normal luma filter: p0 / q0 by delta, p1 / q1 by dEp / dEq
#[allow(clippy::too_many_arguments)]
fn weakf(pl: &mut [u16], ln: Line, tc: i32, max: i32, nop: bool, noq: bool, dep: bool, deq: bool) {
    let (p0, p1, p2) = (ln.p(pl, 0), ln.p(pl, 1), ln.p(pl, 2));
    let (q0, q1, q2) = (ln.q(pl, 0), ln.q(pl, 1), ln.q(pl, 2));
    // delta = (9 (q0 - p0) - 3 (q1 - p1) + 8) >> 4
    let d = ((q0 - p0) * 9 - (q1 - p1) * 3 + 8) >> 4;
    if iabs(d) >= tc * 10 {
        return;
    }
    let d = clamp(d, -tc, tc);
    if !nop {
        ln.setp(pl, 0, clamp(p0 + d, 0, max));
    }
    if !noq {
        ln.setq(pl, 0, clamp(q0 - d, 0, max));
    }
    let t2 = tc >> 1;
    if dep && !nop {
        // Clip3(-(tc >> 1), tc >> 1, (((p2 + p0 + 1) >> 1) - p1 + delta) >> 1)
        let v = clamp((((p2 + p0 + 1) >> 1) - p1 + d) >> 1, -t2, t2);
        ln.setp(pl, 1, clamp(v + p1, 0, max));
    }
    if deq && !noq {
        let v = clamp((((q2 + q0 + 1) >> 1) - q1 - d) >> 1, -t2, t2);
        ln.setq(pl, 1, clamp(v + q1, 0, max));
    }
}

impl Hevc<'_> {
    /// ctbat: CTB raster address of luma (x, y)
    #[inline]
    fn ctbat(&self, x: u32, y: u32) -> usize {
        let l = self.s.log2ctb;
        ((y >> l).wrapping_mul(self.s.pwctb).wrapping_add(x >> l)) as usize
    }

    /// tileof: tile id of CTB rs
    #[inline]
    fn tileof(&self, rs: usize) -> u16 {
        self.tid[self.rs2ts[rs] as usize]
    }

    /// edgeok: the edge between p and q is filtered: deblocking enabled in q's slice, slice
    /// / tile boundary allowed
    fn edgeok(&self, g: &Seg) -> bool {
        let q = self.ctbat(g.x, g.y);
        let p = self.ctbat(g.px, g.py);
        let f = self.cdb[q];
        if f & 1 != 0 {
            return false; // slice_deblocking_filter_disabled_flag
        }
        if q == p {
            return true;
        }
        if self.cslice[q] != self.cslice[p] && f & 2 == 0 {
            return false; // q's slice does not filter across slices
        }
        self.p.lftiles != 0 || self.tileof(q) == self.tileof(p)
    }

    /// edgeat: (g.x, g.y) starts an edge segment of direction g.dir on the grid mask (luma
    /// grid: 7 luma, 8 SubWidthC - 1 / 8 SubHeightC - 1 chroma); sets g.px / g.py
    fn edgeat(&self, g: &mut Seg, mask: u32) -> bool {
        g.px = g.x;
        g.py = g.y;
        if g.dir == 0 {
            if g.x == 0 || g.x & mask != 0 {
                return false;
            }
            g.px -= 1;
            self.medge[self.mapidx(g.x, g.y)] & 1 != 0
        } else {
            if g.y == 0 || g.y & mask != 0 {
                return false;
            }
            g.py -= 1;
            self.medge[self.mapidx(g.x, g.y)] >> 1 & 1 != 0
        }
    }

    /// edgeqp: (QpQ + QpP + 1) >> 1; g.nop / g.noq from the filter-off flags
    fn edgeqp(&self, g: &mut Seg) -> i32 {
        let q = self.mapidx(g.x, g.y);
        let p = self.mapidx(g.px, g.py);
        g.noq = self.mbyp[q] & (MB_BYPASS | MB_PCMOFF) != 0;
        g.nop = self.mbyp[p] & (MB_BYPASS | MB_PCMOFF) != 0;
        (self.mqp[q] as i32 + self.mqp[p] as i32 + 1) >> 1
    }

    // ================================================================== deblocking
    /// hvl_deblock (see the file header)
    pub(crate) fn deblock(&mut self) {
        let mut pl = [core::mem::take(&mut self.pl[0]), core::mem::take(&mut self.pl[1]), core::mem::take(&mut self.pl[2])];
        for dir in 0..2u32 {
            for c in 0..self.u.ncomp as usize {
                let mut y = 0u32;
                while y < self.s.h {
                    let mut x = 0u32;
                    while x < self.s.w {
                        let mut g = Seg { x, y, px: x, py: y, dir, nop: false, noq: false };
                        if c == 0 {
                            self.lumaseg(&mut pl[0], &mut g);
                        } else {
                            self.chromaseg(&mut pl[c], c, &mut g);
                        }
                        x += 4;
                    }
                    y += 4;
                }
            }
        }
        self.pl = pl;
    }

    /// lumaseg: luma edge segment (4 lines) at (g.x, g.y), direction g.dir
    fn lumaseg(&self, pl: &mut [u16], g: &mut Seg) {
        if !self.edgeat(g, 7) || !self.edgeok(g) {
            return;
        }
        let qpl = self.edgeqp(g);
        let cb = self.cbeta[self.ctbat(g.x, g.y)];
        let (bo, to) = (cb[0] as i32, cb[1] as i32); // beta / tc offsets of q's slice
        let bdsh = self.s.bdy.wrapping_sub(8) & 31;
        let max = (1i32 << self.s.bdy) - 1;
        let beta = (BETA[clamp(qpl + bo, 0, 51) as usize] as i32) << bdsh;
        let tc = (TC[clamp(qpl + to + 2, 0, 53) as usize] as i32) << bdsh;
        // pointers and steps
        let pw = self.pw[0] as usize;
        let q0 = g.y as usize * pw + g.x as usize;
        let (a, along) = if g.dir == 0 { (1, pw) } else { (pw, 1) };
        let l0 = Line { q0, a };
        let l3 = Line { q0: q0 + along * 3, a };
        // decisions (lines 0 and 3)
        let dpq = |ln: Line| {
            let dp = iabs(ln.p(pl, 2) - ln.p(pl, 1) - ln.p(pl, 1) + ln.p(pl, 0));
            let dq = iabs(ln.q(pl, 0) - ln.q(pl, 1) - ln.q(pl, 1) + ln.q(pl, 2));
            (dp, dq)
        };
        let (dp0, dq0) = dpq(l0);
        let (dp3, dq3) = dpq(l3);
        if dp0 + dq0 + dp3 + dq3 >= beta {
            return; // d >= beta
        }
        // dSam: 2 dpq < (beta >> 2), |p3 - p0| + |q0 - q3| < (beta >> 3), |p0 - q0| < (5 tc + 1) >> 1
        let dsam = |ln: Line, v: i32| {
            v < beta >> 2
                && iabs(ln.p(pl, 3) - ln.p(pl, 0)) + iabs(ln.q(pl, 3) - ln.q(pl, 0)) < beta >> 3
                && iabs(ln.q(pl, 0) - ln.p(pl, 0)) < (tc * 5 + 1) >> 1
        };
        let strong = dsam(l0, (dp0 + dq0) * 2) && dsam(l3, (dp3 + dq3) * 2);
        let side = (beta + (beta >> 1)) >> 3;
        let dep = dp0 + dp3 < side;
        let deq = dq0 + dq3 < side;
        for k in 0..4 {
            let ln = Line { q0: q0 + along * k, a };
            if strong {
                strongf(pl, ln, tc, g.nop, g.noq);
            } else {
                weakf(pl, ln, tc, max, g.nop, g.noq, dep, deq);
            }
        }
    }

    /// chromaseg: chroma edge segment at luma (g.x, g.y) of component c (bS 2)
    fn chromaseg(&self, pl: &mut [u16], c: usize, g: &mut Seg) {
        let mask = (if g.dir == 0 { self.s.subw } else { self.s.subh } << 3).wrapping_sub(1);
        if !self.edgeat(g, mask) || !self.edgeok(g) {
            return;
        }
        let qp = self.edgeqp(g);
        let mut q = qp.wrapping_add(if c == 1 { self.p.cboff } else { self.p.croff }); // qPi
        if self.s.cat == 1 {
            if q >= 30 {
                q = if q > 43 { q - 6 } else { QPC[(q - 30) as usize] as i32 };
            }
        } else if q > 51 {
            q = 51;
        }
        let to = self.cbeta[self.ctbat(g.x, g.y)][1] as i32;
        let max = (1i32 << self.s.bdc) - 1;
        let tc = (TC[clamp(q + to + 2, 0, 53) as usize] as i32) << (self.s.bdc.wrapping_sub(8) & 31);
        // q0 of the first sample, steps
        let pw = self.pw[c] as usize;
        let q0 = (g.y / self.s.subh) as usize * pw + (g.x / self.s.subw) as usize;
        let (a, along, cnt) = if g.dir == 0 { (1, pw, 4 / self.s.subh) } else { (pw, 1, 4 / self.s.subw) };
        for k in 0..cnt as usize {
            let ln = Line { q0: q0 + along * k, a };
            let (p0, p1, q0v, q1) = (ln.p(pl, 0), ln.p(pl, 1), ln.q(pl, 0), ln.q(pl, 1));
            // delta = Clip3(-tc, tc, ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3))
            let d = clamp((((q0v - p0) << 2) + p1 - q1 + 4) >> 3, -tc, tc);
            if !g.nop {
                ln.setp(pl, 0, clamp(p0 + d, 0, max));
            }
            if !g.noq {
                ln.setq(pl, 0, clamp(q0v - d, 0, max));
            }
        }
    }

    // ================================================================== SAO
    /// hvl_sao -> 0 (see the file header)
    pub(crate) fn sao(&mut self) -> i32 {
        for c in 0..self.u.ncomp as usize {
            let spw = self.pw[c];
            let sph = self.ph[c];
            let src: Vec<u16> = self.pl[c].clone(); // deblocked copy
            let mut dst = core::mem::take(&mut self.pl[c]);
            let (ssw, ssh, bd) = if c == 0 { (1, 1, self.s.bdy) } else { (self.s.subw, self.s.subh, self.s.bdc) };
            let max = (1i32 << bd) - 1;
            let shift = bd.wrapping_sub(5);
            let l = self.s.log2ctb;
            for rs in 0..self.nctb as usize {
                let rec = self.csao[rs][c];
                if rec.ty == 0 {
                    continue;
                }
                // saregion: the region of CTB rs in the component
                let lx = (rs as u32 % self.s.pwctb) << l;
                let ly = (rs as u32 / self.s.pwctb) << l;
                let sx0 = lx / ssw;
                let sy0 = ly / ssh;
                let sw = (self.s.ctbsize / ssw).min(spw.wrapping_sub(sx0));
                let sh = (self.s.ctbsize / ssh).min(sph.wrapping_sub(sy0));
                let reg = Region { sx0, sy0, sw, sh, ssw, ssh, spw, sph, rs };
                self.saapply(&src, &mut dst, &rec, &reg, max, shift);
            }
            self.pl[c] = dst;
        }
        0
    }

    /// saapply: band / edge offsets of rec over the CTB region
    fn saapply(&self, src: &[u16], dst: &mut [u16], rec: &crate::dec::Sao, g: &Region, max: i32, shift: u32) {
        let pos = rec.pos as u32; // band position / EO class
        let (dx, dy) = if rec.ty == 2 {
            let e = EO[(pos & 3) as usize];
            (e[0] as i32, e[1] as i32)
        } else {
            (0, 0)
        };
        for y in g.sy0..g.sy0 + g.sh {
            for x in g.sx0..g.sx0 + g.sw {
                // filter-off samples
                if self.mbyp[self.mapidx(x * g.ssw, y * g.ssh)] & (MB_BYPASS | MB_PCMOFF) != 0 {
                    continue;
                }
                let i = (y * g.spw + x) as usize; // sample index
                let s = src[i] as u32;
                let off;
                if rec.ty == 1 {
                    let b = ((s >> (shift & 31)).wrapping_sub(pos)) & 31;
                    if b >= 4 {
                        continue;
                    }
                    off = rec.off[b as usize] as i32;
                } else {
                    let Some(a) = self.nbval(src, g, x as i32 + dx, y as i32 + dy) else { continue };
                    let Some(b) = self.nbval(src, g, x as i32 - dx, y as i32 - dy) else { continue };
                    let mut e = 2i32;
                    if s != a {
                        e += if s < a { -1 } else { 1 };
                    }
                    if s != b {
                        e += if s < b { -1 } else { 1 };
                    }
                    let k = EOMAP[e as usize] as usize;
                    if k == 0 {
                        continue;
                    }
                    off = rec.off[k - 1] as i32;
                }
                dst[i] = clamp(off + s as i32, 0, max) as u16;
            }
        }
    }

    /// nbval: the deblocked neighbour sample, or None when it is outside the plane or across
    /// a slice / tile boundary that may not be used (current sample in CTB g.rs)
    fn nbval(&self, src: &[u16], g: &Region, xn: i32, yn: i32) -> Option<u32> {
        if xn < 0 || yn < 0 || xn as u32 >= g.spw || yn as u32 >= g.sph {
            return None;
        }
        let (xn, yn) = (xn as u32, yn as u32);
        // inside the CTB region: same CTB (the assembly computes both CTB addresses)
        let inside = xn >= g.sx0 && xn < g.sx0 + g.sw && yn >= g.sy0 && yn < g.sy0 + g.sh;
        if !inside {
            let cur = g.rs;
            let nb = self.ctbat(xn * g.ssw, yn * g.ssh);
            if cur != nb {
                if self.cslice[cur] != self.cslice[nb] {
                    // different slices: the flag of the slice later in decoding order
                    let later = if self.rs2ts[cur] > self.rs2ts[nb] { cur } else { nb };
                    if self.cdb[later] & 2 == 0 {
                        return None;
                    }
                }
                if self.p.lftiles == 0 && self.tileof(cur) != self.tileof(nb) {
                    return None;
                }
            }
        }
        Some(src[(yn * g.spw + xn) as usize] as u32)
    }
}

/// the CTB region of saapply (HD_S*)
struct Region {
    sx0: u32,
    sy0: u32,
    sw: u32,
    sh: u32,
    ssw: u32,
    ssh: u32,
    spw: u32,
    sph: u32,
    rs: usize,
}
