// raster -- anti-aliased scanline rasterizer (signed-area accumulation)  [src/gfx/raster.s]
//
// Every line deposits its signed area (integers in 1/65536 of a pixel) into an
// accumulation buffer; the running sum along a row is the coverage.  Rows go in bands
// of 64.  The float operations and their order are those of the assembly, so the
// coverage bytes are identical (see raster.s for the clip-determinism argument).

use alloc::vec::Vec;

use super::blend;
use super::{Dc, Surface, FMT_A8};
use rt::fx::{ceil_i, cvt, cvtt, floor_i, maxss, minss};
use simd::{Simd, Sse2};

const BAND: i32 = 64;
const CMIN: f32 = -4194304.0;
const CMAX: f32 = 4194304.0;
static I0123: [i32; 4] = [0, 1, 2, 3];

#[derive(Clone, Copy)]
struct Rl {
    xa: f32,
    ya: f32,
    xb: f32,
    yb: f32,
    dir: f32,
    dxdy: f32,
    xmin: f32,
    xmax: f32,
}

pub struct Raster {
    lines: Vec<[f32; 4]>,
    lbox: [f32; 4],
    acc: Vec<i32>,
    cov: Vec<u8>,
    rl: Vec<Rl>,
    rowmin: [i32; 64],
    rowmax: [i32; 64],
}

impl Default for Raster {
    fn default() -> Raster {
        Raster {
            lines: Vec::new(),
            lbox: [0.0; 4],
            acc: Vec::new(),
            cov: Vec::new(),
            rl: Vec::new(),
            rowmin: [0; 64],
            rowmax: [0; 64],
        }
    }
}

#[inline]
fn clampf(v: f32) -> f32 {
    minss(maxss(v, CMIN), CMAX)
}

impl Raster {
    pub fn lines_begin(&mut self) {
        self.lines.clear();
        self.lbox = [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX];
    }

    pub fn emit_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.lines.push([x0, y0, x1, y1]);
        let b = &mut self.lbox;
        b[0] = minss(minss(b[0], x0), x1);
        b[1] = minss(minss(b[1], y0), y1);
        b[2] = maxss(maxss(b[2], x0), x1);
        b[3] = maxss(maxss(b[3], y0), y1);
    }

    pub fn has_lines(&self) -> bool {
        !self.lines.is_empty()
    }

    /// the bounding box of the line list (f32 minx, miny, maxx, maxy)
    pub fn lbox(&self) -> [f32; 4] {
        self.lbox
    }

    /// ras_fill: rasterize the line list with the current colour into the target, clipped
    pub fn fill(&mut self, dc: &Dc, sf: &mut Surface, rule: u32) {
        if sf.px.is_empty() {
            return;
        }
        let stride = sf.stride as usize;
        let a8 = sf.fmt == FMT_A8;
        let px = &mut sf.px;
        self.fill_rows(dc.clip, rule, &mut |ty, tx, cov| {
            let o = ty as usize * stride;
            let tx = tx as usize;
            if a8 {
                blend::a8_over(&mut px[o + tx..o + tx + cov.len()], cov, dc.pcol32 >> 24);
            } else {
                blend::mask_over(&mut px[o + tx * 4..o + (tx + cov.len()) * 4], cov, &dc.pcol);
            }
        });
    }

    /// rasterize the line list inside clip (x0, y0, x1, y1 exclusive): emit(y, x, coverage)
    /// for every touched row part, rows top to bottom (the coverage of a pixel does not
    /// depend on the clip)
    pub fn fill_rows(&mut self, clip: [i32; 4], rule: u32, emit: &mut dyn FnMut(i32, i32, &[u8])) {
        if self.lines.is_empty() {
            return;
        }
        let x0 = floor_i(clampf(self.lbox[0])).max(clip[0]);
        let y0 = floor_i(clampf(self.lbox[1])).max(clip[1]);
        let x1 = (ceil_i(clampf(self.lbox[2])) + 1).min(clip[2]); // spill pixel
        let y1 = ceil_i(clampf(self.lbox[3])).min(clip[3]);
        let w = x1 - x0;
        let h = y1 - y0;
        if w <= 0 || h <= 0 {
            return;
        }
        let rowcells = ((w + 8 + 3) & !3) as usize;
        if self.acc.len() < rowcells * BAND as usize {
            self.acc.resize(rowcells * BAND as usize, 0);
        }
        if self.cov.len() < w as usize + 64 {
            self.cov.resize(w as usize + 64, 0);
        }
        self.prepare(y0, y0 + h);
        let mut by = 0;
        while by < h {
            let rows = (h - by).min(BAND);
            let b0i = y0 + by;
            let b1i = b0i + rows;
            self.rowmin = [0x7FFF_FFFF; 64];
            self.rowmax = [-1; 64];
            for i in 0..self.rl.len() {
                let l = self.rl[i];
                if !(l.yb > b0i as f32) || l.ya >= b1i as f32 {
                    continue;
                }
                self.deposit(&l, b0i, b1i, x0, w, rowcells);
            }
            for r in 0..rows as usize {
                let rmin = self.rowmin[r];
                let rmax = self.rowmax[r];
                if rmax < 0 {
                    continue;
                }
                let n = (rmax - rmin + 1) as usize;
                self.sum_row(r * rowcells + rmin as usize, rmin as usize, n, rule);
                let count = (rmax + 1).min(w) - rmin;
                if count <= 0 {
                    continue;
                }
                emit(b0i + r as i32, x0 + rmin, &self.cov[rmin as usize..(rmin + count) as usize]);
            }
            by += BAND;
        }
    }

    // ras_prepare: clamped lines ordered top to bottom that reach into rows [y0, y1)
    fn prepare(&mut self, y0: i32, y1: i32) {
        let fy0 = y0 as f32;
        let fy1 = y1 as f32;
        self.rl.clear();
        for ln in &self.lines {
            let (mut xa, mut ya, mut xb, mut yb) = (clampf(ln[0]), clampf(ln[1]), clampf(ln[2]), clampf(ln[3]));
            let mut dir = 65536.0f32;
            if ya == yb {
                continue; // horizontal: no area
            }
            if !(ya < yb) {
                core::mem::swap(&mut xa, &mut xb);
                core::mem::swap(&mut ya, &mut yb);
                dir = -65536.0;
            }
            if !(yb > fy0) || ya >= fy1 {
                continue;
            }
            let dy = yb - ya;
            self.rl.push(Rl {
                xa,
                ya,
                xb,
                yb,
                dir,
                dxdy: (xb - xa) / dy,
                xmin: minss(xa, xb),
                xmax: maxss(xa, xb),
            });
        }
    }

    // ras_deposit: accumulate one line into the rows of the band [b0i, b1i)
    fn deposit(&mut self, l: &Rl, b0i: i32, b1i: i32, cl: i32, rw: i32, rowcells: usize) {
        let mut row = cvtt(l.ya).max(b0i);
        while row < b1i {
            let y = row as f32;
            if y >= l.yb {
                break;
            }
            // top of the row part
            let (mut tx, mut ty) = (l.xa, l.ya);
            if y > ty {
                ty = y;
                tx = minss(maxss((y - l.ya) * l.dxdy + l.xa, l.xmin), l.xmax);
            }
            // bottom
            let y1 = y + 1.0;
            let (mut bx, mut byy) = (l.xb, l.yb);
            if !(y1 >= l.yb) {
                byy = y1;
                bx = minss(maxss((y1 - l.ya) * l.dxdy + l.xa, l.xmin), l.xmax);
            }
            let ds = (byy - ty) * l.dir;
            let d = cvt(ds);
            let lo = minss(tx, bx);
            let hi = maxss(tx, bx);
            let mut x0i = cvtt(lo);
            let mut x0ff = x0i as f32;
            if x0ff > lo {
                x0i -= 1;
                x0ff = x0i as f32;
            }
            let mut x1i = cvtt(hi);
            let mut x1ff = x1i as f32;
            if x1ff < hi {
                x1i += 1;
                x1ff = x1i as f32;
            }
            let (a0, a1, s, alast);
            if x1i <= x0i + 1 {
                // one cell: A(x0i) = 1 - (mean x - x0i)
                a0 = 1.0 - ((lo + hi) * 0.5 - x0ff);
                a1 = 0.0;
                s = 0.0;
                alast = hi; // (register content of the assembly; never used here)
            } else {
                let x1f = hi - x1ff + 1.0;
                s = 1.0 / (hi - lo);
                let x0f = lo - x0ff;
                let am = x1f * x1f * s * 0.5;
                alast = 1.0 - am;
                let t = 1.0 - x0f;
                a0 = t * t * s * 0.5;
                a1 = (1.5 - x0f) * s;
            }
            // visible cells: e = clamp(x1i, cl, cl + W), c0 = clamp(x0i, cl, e)
            let e = x1i.max(cl).min(cl + rw);
            let mut c = x0i.max(cl).min(e);
            let r = (row - b0i) as usize;
            if c - cl < self.rowmin[r] {
                self.rowmin[r] = c - cl;
            }
            if e - cl > self.rowmax[r] {
                self.rowmax[r] = e - cl;
            }
            let base = r * rowcells;
            let acc = &mut self.acc[base..base + rowcells];
            let mut f = 0i32;
            if c < e {
                if c == x0i {
                    f = cvt(a0 * ds);
                    acc[(c - cl) as usize] = acc[(c - cl) as usize].wrapping_add(f);
                    c += 1;
                }
                let lim = (x1i - 1).min(e);
                // middle cells, 4 at a time: F(c .. c + 3) by cvtps2dq, lane by lane the
                // scalar formula (SSE2 as in the assembly)
                let x = Sse2;
                let (vs, va1, vds) = (x.splatf(s), x.splatf(a1), x.splatf(ds));
                while c + 4 <= lim {
                    let k = x.add32(x.splat32((c - x0i - 1) as u32), x.load_i32(&I0123));
                    let fv = x.cvtr_f32_i32(x.mulf(x.addf(x.mulf(x.cvt_i32_f32(k), vs), va1), vds));
                    let prev = x.or(x.bsll::<4>(fv), x.from_i32(f)); // F(c - 1 .. c + 2)
                    let i = (c - cl) as usize;
                    let v = x.add32(x.load_i32(&acc[i..i + 4]), x.sub32(fv, prev));
                    x.store_i32(&mut acc[i..i + 4], v);
                    f = x.to_i32(x.shuf32::<0xFF>(fv));
                    c += 4;
                }
                while c < lim {
                    let k = c - x0i - 1;
                    let fc = cvt(((k as f32) * s + a1) * ds);
                    let i = (c - cl) as usize;
                    acc[i] = acc[i].wrapping_add(fc.wrapping_sub(f));
                    f = fc;
                    c += 1;
                }
                if c < e {
                    let fc = cvt(alast * ds);
                    let i = (c - cl) as usize;
                    acc[i] = acc[i].wrapping_add(fc.wrapping_sub(f));
                    f = fc;
                    c += 1;
                }
            }
            let i = (c - cl) as usize;
            acc[i] = acc[i].wrapping_add(d.wrapping_sub(f));
            row += 1;
        }
    }

    // ras_sum_row: running sum -> 8-bit coverage of n cells (groups of 4: up to 3 cells
    // past n, the buffers are padded), clearing the accumulator
    fn sum_row(&mut self, acc0: usize, cov0: usize, n: usize, rule: u32) {
        let n4 = (n + 3) & !3;
        let acc = &mut self.acc[acc0..acc0 + n4];
        let cov = &mut self.cov[cov0..cov0 + n4];
        // perf-raster: one loop per rule (the assembly's two loops), no bounds checks inside
        if rule == 0 {
            sum_cells::<false>(acc, cov);
        } else {
            sum_cells::<true>(acc, cov);
        }
    }
}

// the cells of ras_sum_row, groups of 4 with SSE2 as in the assembly (the sums are exact, the
// grouping changes nothing): non-zero min(|sum|, 65536), even-odd 65536 - |(sum mod 131072) -
// 65536|, then (v * 255 + 32768) >> 16.  perf-raster: a group without deposits (all four
// cells 0) has the carry as its running sum in every lane, so its coverage is 4 times the
// last coverage byte of the group before (0 at the start: the carry is 0) -- the long inner
// runs of a shape cost a compare and a store instead of the sum (identical bytes; the cells
// are 0 already).
#[inline(always)]
fn sum_cells<const EO: bool>(acc: &mut [i32], cov: &mut [u8]) {
    let x = Sse2;
    let z = x.zero();
    let n = acc.len().min(cov.len()) & !3;
    let mut st = SumState { carry: z, cc: 0, ccv: z };
    let mut k = 0;
    // blocks of 16 cells: without deposits 16 coverage bytes at once, else group by group
    while k + 16 <= n {
        let (a, cv) = (&mut acc[k..k + 16], &mut cov[k..k + 16]);
        let o = x.or(x.or(x.load_i32(&a[0..4]), x.load_i32(&a[4..8])), x.or(x.load_i32(&a[8..12]), x.load_i32(&a[12..16])));
        if x.movemask8(x.cmpeq32(o, z)) == 0xFFFF {
            x.store(cv, st.ccv);
        } else {
            for g in 0..4 {
                sum_group::<EO>(&mut st, &mut a[g * 4..g * 4 + 4], &mut cv[g * 4..g * 4 + 4]);
            }
        }
        k += 16;
    }
    while k < n {
        sum_group::<EO>(&mut st, &mut acc[k..k + 4], &mut cov[k..k + 4]);
        k += 4;
    }
}

// running sum state of sum_cells: the carry (in every lane) and the coverage of a group
// without deposits (4 equal bytes, also in every dword of ccv)
struct SumState {
    carry: <Sse2 as Simd>::V,
    cc: u32,
    ccv: <Sse2 as Simd>::V,
}

// one group of 4 cells
#[inline(always)]
fn sum_group<const EO: bool>(st: &mut SumState, a: &mut [i32], cv: &mut [u8]) {
    let x = Sse2;
    let (z, one, half) = (x.zero(), x.splat32(65536), x.splat32(32768));
    let mut v = x.load_i32(a);
    if x.movemask8(x.cmpeq32(v, z)) == 0xFFFF {
        cv[..4].copy_from_slice(&st.cc.to_le_bytes());
        return;
    }
    x.store_i32(a, z);
    v = x.add32(v, x.bsll::<4>(v));
    v = x.add32(v, x.bsll::<8>(v));
    v = x.add32(v, st.carry);
    st.carry = x.shuf32::<0xFF>(v);
    let c = if EO {
        let t = x.sub32(x.and(v, x.splat32(131071)), one); // sum mod 2 - 1
        let sg = x.sra32::<31>(t);
        x.sub32(one, x.sub32(x.xor(t, sg), sg)) // 1 - |t|
    } else {
        let sg = x.sra32::<31>(v);
        let m = x.sub32(x.xor(v, sg), sg); // |sum|
        let over = x.cmpgts32(m, one);
        x.sub32(m, x.and(x.sub32(m, one), over)) // min(|sum|, 65536)
    };
    let c = x.srl32::<16>(x.add32(x.sub32(x.sll32::<8>(c), c), half));
    let w = x.to_i32(x.packus16(x.packs32(c, c), z)) as u32;
    cv[..4].copy_from_slice(&w.to_le_bytes());
    st.cc = (w >> 24) * 0x0101_0101;
    st.ccv = x.splat32(st.cc);
}
