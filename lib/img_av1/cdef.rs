// cdef -- AV1 constrained directional enhancement filter (specification 7.15)
//                                                                    [src/img/av1_cdef.s]
//
//   cdef(h, f)     CDEF process: CdefFrame (a copy of CurrFrame made by the caller) filtered per
//                  8x8 block from CurrFrame; the block rows run on the worker pool (they only
//                  read CurrFrame, and item i writes the CdefFrame rows of block row i)
//
// Blocks whose taps all lie inside the plane take the assembly's SSE2 path (cd_fast: 8 or 4
// samples of a row in 16-bit lanes, simd::Sse2 like the assembly); the others the scalar one.
// Both give the same values (the sums fit in 16 bits for every bit depth).

use alloc::vec::Vec;

use rt::sync::SpinLock;
use simd::{Simd, Sse2};

use crate::blk::MIB_SKIP;
use crate::dec::{Frame, Hdr};
use crate::tab::*;

/// cdef_filter arguments (CD_*)
#[derive(Clone, Copy)]
struct Cf {
    plane: usize,
    pri: u32,
    sec: u32,
    damp: i32,
    dir: u32,
}

struct Cd<'a> {
    h: &'a Hdr,
    src: [&'a [u16]; 3], // CurrFrame
    st: [usize; 3],
    mi: &'a [[u8; 16]],
    mis: usize,
    cdidx: &'a [i8],
    cds: usize,
}

/// av1_cdef
pub(crate) fn cdef(h: &Hdr, f: &mut Frame) {
    let Frame { pl, cpl, ps, mi, mis, cdidx, cds, .. } = f;
    let np = h.np as usize;
    let mut dch: [Vec<SpinLock<&mut [u16]>>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for (p, c) in cpl.iter_mut().enumerate().take(np) {
        let sy = if p > 0 { h.ssy } else { 0 };
        let rows = (8 >> sy) as usize;
        dch[p] = c.chunks_mut(rows * ps[p] as usize).map(SpinLock::new).collect();
    }
    let cd = Cd {
        h,
        src: [&pl[0], &pl[1], &pl[2]],
        st: [ps[0] as usize, ps[1] as usize, ps[2] as usize],
        mi,
        mis: *mis as usize,
        cdidx,
        cds: *cds as usize,
    };
    let item = |i: usize, _: &plat::JobCtl| {
        let mut g0 = dch[0][i].lock();
        if np > 1 {
            let mut g1 = dch[1][i].lock();
            let mut g2 = dch[2][i].lock();
            cd.row(i, [&mut **g0, &mut **g1, &mut **g2]);
        } else {
            cd.row(i, [&mut **g0, &mut [], &mut []]);
        }
    };
    let _ = plat::parallel_for((h.mirows >> 1) as usize, &item);
}

impl Cd<'_> {
    /// cd_row: the 8x8 blocks of block row index (r = 2 * index); dst = its CdefFrame rows
    fn row(&self, index: usize, dst: [&mut [u16]; 3]) {
        let h = self.h;
        let [d0, d1, d2] = dst;
        let r = 2 * index as u32;
        let mut c = 0u32;
        while c < h.micols {
            let idx = self.cdidx[(r >> 4) as usize * self.cds + (c >> 4) as usize];
            if idx >= 0 {
                let idx = (idx & 7) as usize;
                // skip = all four 4x4 blocks skipped
                let m = r as usize * self.mis + c as usize;
                let sk = self.mi[m][MIB_SKIP] & self.mi[m + 1][MIB_SKIP] & self.mi[m + self.mis][MIB_SKIP] & self.mi[m + self.mis + 1][MIB_SKIP];
                if sk == 0 {
                    let (ydir, var) = self.direction(r, c);
                    // luma
                    let cs = h.bd - 8; // coeffShift
                    let mut pri = (h.cdefyp[idx] as u32) << cs;
                    let sec = (h.cdefys[idx] as u32) << cs;
                    let dir = if pri != 0 { ydir } else { 0 };
                    let vs = if var >> 6 != 0 { (31 - (var >> 6).leading_zeros()).min(12) } else { 0 }; // varStr
                    pri = if var != 0 { (pri * (vs + 4) + 8) >> 4 } else { 0 };
                    let cf = Cf { plane: 0, pri, sec, damp: (h.cdefdamp + cs) as i32, dir };
                    self.filter(&cf, r, c, d0, index);
                    if h.np > 1 {
                        // chroma
                        let pri = (h.cdefuvp[idx] as u32) << cs;
                        let sec = (h.cdefuvs[idx] as u32) << cs;
                        let dir = if pri != 0 { CDEF_UV_DIR[((h.ssx * 2 + h.ssy) * 8 + ydir) as usize] as u32 } else { 0 };
                        let damp = (h.cdefdamp + cs) as i32 - 1;
                        let cf = Cf { plane: 1, pri, sec, damp, dir };
                        self.filter(&cf, r, c, d1, index);
                        let cf = Cf { plane: 2, ..cf };
                        self.filter(&cf, r, c, d2, index);
                    }
                }
            }
            c += 2;
        }
    }

    /// cd_direction: CDEF direction process -> (yDir, var)
    fn direction(&self, r: u32, c: u32) -> (u32, u32) {
        let mut part = [[0i32; 15]; 8];
        let mut cost = [0i64; 8];
        let st = self.st[0];
        let src = self.src[0];
        let sh = self.h.bd - 8;
        let mut o = (r as usize * 4) * st + c as usize * 4;
        for i in 0..8 {
            for j in 0..8 {
                let x = (src[o + j] >> sh) as i32 - 128;
                part[0][i + j] += x;
                part[1][i + j / 2] += x;
                part[2][i] += x;
                part[3][3 + i - j / 2] += x;
                part[4][7 + i - j] += x;
                part[5][3 - i / 2 + j] += x;
                part[6][j] += x;
                part[7][i / 2 + j] += x;
            }
            o += st;
        }
        let d8 = DIV_TABLE[8] as i64;
        let sq = |v: i32| v as i64 * v as i64;
        for i in 0..8 {
            cost[2] += sq(part[2][i]);
            cost[6] += sq(part[6][i]);
        }
        cost[2] *= d8;
        cost[6] *= d8;
        for i in 0..7 {
            let dv = DIV_TABLE[i + 1] as i64;
            cost[0] += (sq(part[0][i]) + sq(part[0][14 - i])) * dv;
            cost[4] += (sq(part[4][i]) + sq(part[4][14 - i])) * dv;
        }
        cost[0] += sq(part[0][7]) * d8;
        cost[4] += sq(part[4][7]) * d8;
        for i in [1usize, 3, 5, 7] {
            let mut s = 0i64;
            for j in 3..8 {
                s += sq(part[i][j]);
            }
            let mut ci = s * d8;
            for j in 0..3 {
                ci += (sq(part[i][j]) + sq(part[i][10 - j])) * DIV_TABLE[2 * j + 2] as i64;
            }
            cost[i] = ci;
        }
        // best direction
        let mut best = 0i64;
        let mut ydir = 0u32;
        for (d, &cv) in cost.iter().enumerate() {
            if cv > best {
                best = cv;
                ydir = d as u32;
            }
        }
        let var = ((best - cost[((ydir + 4) & 7) as usize]) >> 10) as u32;
        (ydir, var)
    }

    /// cd_filter: CDEF filter process for cf.plane of the 8x8 block (r, c) into the CdefFrame
    /// rows of block row `index` (dst)
    fn filter(&self, cf: &Cf, r: u32, c: u32, dst: &mut [u16], index: usize) {
        let h = self.h;
        let p = cf.plane;
        let (sx, sy) = if p > 0 { (h.ssx, h.ssy) } else { (0, 0) };
        let aw = ((h.micols << 2) >> sx) as i32; // available width
        let ah = ((h.mirows << 2) >> sy) as i32;
        let w = (8u32 >> sx) as usize;
        let hh = (8u32 >> sy) as usize;
        let x0 = ((c << 2) >> sx) as i32;
        let y0 = ((r << 2) >> sy) as i32;
        let st = self.st[p];
        // taps
        let k = ((cf.pri >> (h.bd - 8)) & 1) as usize;
        let pt = [CDEF_PRI_TAPS[k * 2] as i32, CDEF_PRI_TAPS[k * 2 + 1] as i32];
        let stp = [CDEF_SEC_TAPS[k * 2] as i32, CDEF_SEC_TAPS[k * 2 + 1] as i32];
        // dampingAdj = Max( 0, damping - FloorLog2( threshold ) )
        let adj = |s: u32| if s != 0 { (cf.damp - (31 - s.leading_zeros()) as i32).max(0) } else { 0 };
        let padj = adj(cf.pri);
        let sadj = adj(cf.sec);
        let dirs = [cf.dir & 7, cf.dir.wrapping_sub(2) & 7, (cf.dir + 2) & 7];
        let src = self.src[p];
        // destination: rows of block row index (y0 - its first row)
        let dy0 = (index as u32 * (8 >> sy)) as i32;
        // interior block (every tap inside the plane): SSE2 path
        if x0 >= 2 && y0 >= 2 && x0 + w as i32 + 2 <= aw && y0 + hh as i32 + 2 <= ah {
            self.fast(cf, src, st, dst, x0, y0, dy0, w, hh, &dirs, &pt, &stp, padj, sadj);
            return;
        }
        for i in 0..hh as i32 {
            for j in 0..w as i32 {
                let xv = src[((y0 + i) as usize) * st + (x0 + j) as usize] as i32;
                let mut mx = xv;
                let mut mn = xv;
                let mut sum = 0i32;
                for kk in 0..2usize {
                    for sign in [-1i32, 1] {
                        for (t, &d) in dirs.iter().enumerate() {
                            let dd = (d as usize) * 4 + kk * 2;
                            let dy = CDEF_DIRECTIONS[dd] as i32 * sign;
                            let dx = CDEF_DIRECTIONS[dd + 1] as i32 * sign;
                            let y = y0 + i + dy;
                            let x = x0 + j + dx;
                            if y < 0 || y >= ah || x < 0 || x >= aw {
                                continue;
                            }
                            let pv = src[y as usize * st + x as usize] as i32;
                            if pv > mx {
                                mx = pv;
                            }
                            if pv < mn {
                                mn = pv;
                            }
                            let (thr, ad, wt) = if t == 0 { (cf.pri, padj, pt[kk]) } else { (cf.sec, sadj, stp[kk]) };
                            if thr == 0 {
                                continue;
                            }
                            let diff = pv - xv;
                            let a = diff.unsigned_abs();
                            let v = (thr as i32 - (a >> ad) as i32).max(0).min(a as i32);
                            let v = if diff < 0 { -v } else { v };
                            sum += v * wt;
                        }
                    }
                }
                // Clip3( min, max, x + ( ( 8 + sum - ( sum < 0 ) ) >> 4 ) )
                let v = (xv + ((8 + sum - (sum < 0) as i32) >> 4)).max(mn).min(mx);
                dst[((y0 + i - dy0) as usize) * st + (x0 + j) as usize] = v as u16;
            }
        }
    }

    /// cd_fast: CDEF filter of an interior block with SSE2, 8 (or 4) samples of a row at once
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn fast(
        &self,
        cf: &Cf,
        src: &[u16],
        st: usize,
        dst: &mut [u16],
        x0: i32,
        y0: i32,
        dy0: i32,
        w: usize,
        hh: usize,
        dirs: &[u32; 3],
        pt: &[i32; 2],
        stp: &[i32; 2],
        padj: i32,
        sadj: i32,
    ) {
        let s = Sse2;
        // tap offsets: for the directions dir, dir - 2, dir + 2: k = 0, 1 x sign -1, +1
        let mut off = [0isize; 12];
        for (t, o) in off.iter_mut().enumerate() {
            let d = dirs[t >> 2] as usize;
            let k = (t >> 1) & 1;
            let dy = CDEF_DIRECTIONS[d * 4 + k * 2] as isize;
            let dx = CDEF_DIRECTIONS[d * 4 + k * 2 + 1] as isize;
            let v = dy * st as isize + dx;
            *o = if t & 1 != 0 { v } else { -v };
        }
        let k = Kc {
            off,
            wp: [s.splat16(pt[0] as u16), s.splat16(pt[1] as u16)],
            ws: [s.splat16(stp[0] as u16), s.splat16(stp[1] as u16)],
            tp: s.splat16(cf.pri as u16),
            ts: s.splat16(cf.sec as u16),
            padj: padj as u32,
            sadj: sadj as u32,
        };
        let o0 = y0 as usize * st + x0 as usize;
        let d0 = (y0 - dy0) as usize * st + x0 as usize;
        match (w == 8, cf.pri != 0, cf.sec != 0) {
            (true, true, true) => rows::<true, true, true>(&k, src, st, dst, o0, d0, hh),
            (true, true, false) => rows::<true, true, false>(&k, src, st, dst, o0, d0, hh),
            (true, false, true) => rows::<true, false, true>(&k, src, st, dst, o0, d0, hh),
            (true, false, false) => rows::<true, false, false>(&k, src, st, dst, o0, d0, hh),
            (false, true, true) => rows::<false, true, true>(&k, src, st, dst, o0, d0, hh),
            (false, true, false) => rows::<false, true, false>(&k, src, st, dst, o0, d0, hh),
            (false, false, true) => rows::<false, false, true>(&k, src, st, dst, o0, d0, hh),
            (false, false, false) => rows::<false, false, false>(&k, src, st, dst, o0, d0, hh),
        }
    }
}

type V = <Sse2 as Simd>::V;

/// the constants of cd_fast for one block: tap offsets (samples), weights, thresholds, shifts
struct Kc {
    off: [isize; 12],
    wp: [V; 2],
    ws: [V; 2],
    tp: V,
    ts: V,
    padj: u32,
    sadj: u32,
}

/// W8 (8 or 4) samples of src at q
#[inline(always)]
fn ldw<const W8: bool>(src: &[u16], q: usize) -> V {
    if W8 { Sse2.load_u16(&src[q..]) } else { Sse2.load_u16_lo(&src[q..]) }
}

/// weight * constrain( p - x, threshold, dampingAdj ) (CDLANE)
#[inline(always)]
fn tap(p: V, x: V, thr: V, adj: u32, wt: V) -> V {
    let s = Sse2;
    let diff = s.sub16(p, x);
    let sg = s.sra16::<15>(diff); // sign mask
    let ab = s.sub16(s.xor(diff, sg), sg); // Abs( diff )
    let sh = s.srl16x(ab, adj); // Abs >> dampingAdj
    let th = s.subus16(thr, sh); // Max( 0, threshold - ( Abs >> adj ) )
    let v = s.mins16(th, ab); // Clip3( 0, Abs, . )
    let v = s.sub16(s.xor(v, sg), sg); // sign
    s.mullo16(v, wt)
}

/// the rows of cd_fast (CDROWS): W8 = 8 samples (else 4), HP / HS = primary / secondary
/// strength not 0
#[inline(never)]
fn rows<const W8: bool, const HP: bool, const HS: bool>(k: &Kc, src: &[u16], st: usize, dst: &mut [u16], o0: usize, d0: usize, hh: usize) {
    let s = Sse2;
    let c8 = s.splat16(8);
    for i in 0..hh {
        let o = o0 + i * st;
        let x = ldw::<W8>(src, o);
        let mut mx = x;
        let mut mn = x;
        let mut sum = s.zero();
        macro_rules! one {
            ($t:expr, pri, $w:expr) => {
                let p = ldw::<W8>(src, (o as isize + k.off[$t]) as usize);
                mx = s.maxs16(mx, p);
                mn = s.mins16(mn, p);
                if HP {
                    sum = s.add16(sum, tap(p, x, k.tp, k.padj, $w));
                }
            };
            ($t:expr, sec, $w:expr) => {
                let p = ldw::<W8>(src, (o as isize + k.off[$t]) as usize);
                mx = s.maxs16(mx, p);
                mn = s.mins16(mn, p);
                if HS {
                    sum = s.add16(sum, tap(p, x, k.ts, k.sadj, $w));
                }
            };
        }
        one!(0, pri, k.wp[0]);
        one!(1, pri, k.wp[0]);
        one!(2, pri, k.wp[1]);
        one!(3, pri, k.wp[1]);
        one!(4, sec, k.ws[0]);
        one!(5, sec, k.ws[0]);
        one!(6, sec, k.ws[1]);
        one!(7, sec, k.ws[1]);
        one!(8, sec, k.ws[0]);
        one!(9, sec, k.ws[0]);
        one!(10, sec, k.ws[1]);
        one!(11, sec, k.ws[1]);
        // x + ( ( 8 + sum - ( sum < 0 ) ) >> 4 ), clipped to [ min, max ]
        let v = s.add16(sum, s.sra16::<15>(sum));
        let v = s.sra16::<4>(s.add16(v, c8));
        let v = s.mins16(s.maxs16(s.add16(v, x), mn), mx);
        let d = d0 + i * st;
        if W8 {
            s.store_u16(&mut dst[d..], v);
        } else {
            s.store_u16_lo(&mut dst[d..], v);
        }
    }
}
