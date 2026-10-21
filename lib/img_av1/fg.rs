// fg -- AV1 film grain synthesis (specification 7.18.3)                  [src/img/av1_fg.s]
//
//   grain(h, f) -> 0 / ERR_NOMEM   film grain on a copy of the output planes (Frame::gpl, then
//                                  the output = those planes)
//
// The noise image is produced one 32-row luma stripe at a time (the current and the previous
// noise stripe are kept for the vertical overlap); chroma noise of a stripe is added before
// luma noise, as chroma blending reads the luma samples without noise.

use alloc::vec::Vec;

use img_core::ERR_NOMEM;

use crate::dec::{zvec, Fg, Frame, Hdr, Out};
use crate::tab::GAUSSIAN_SEQUENCE;

const GW: usize = 82; // grain array width / height (luma)
const GH: usize = 73;

/// the locals of av1_grain (FX_*) and its scratch
struct Fx<'a> {
    g: &'a Fg,
    sw: usize, // stripe row length (elements)
    rand: u32, // RandomRegister
    w: u32,
    h: u32,
    subx: u32,
    suby: u32,
    gmin: i32, // GrainMin / GrainMax
    gmax: i32,
    shift: u32, // 12 - BitDepth + grain_scale_shift
    ln: u32,    // lumaNum
    cur: usize, // which stripe buffer is current
    minv: i32,  // minValue / maxLuma / maxChroma
    maxl: i32,
    maxc: i32,
    cw: usize, // chromaW / chromaH
    ch: usize,
    bd: u32,
    ovl: u32, // overlap_flag
    grain: [Vec<i32>; 3], // LumaGrain, CbGrain, CrGrain [ 73 ][ 82 ]
    lut: [[i32; 256]; 3], // ScalingLut
    stripes: Vec<i32>,    // [ 2 ][ 3 ][ 34 ][ sw ] noise stripes
}

#[inline(always)]
fn rnd2(v: i32, n: u32) -> i32 {
    v.wrapping_add((1i32 << n) >> 1) >> n
}

/// av1_grain
pub(crate) fn grain(h: &Hdr, f: &mut Frame) -> i32 {
    let np = h.np as usize;
    // output copies
    for p in 0..np {
        let n = f.ops[p] as usize * f.ph[p] as usize;
        let mut v: Vec<u16> = Vec::new();
        if v.try_reserve_exact(n).is_err() {
            return ERR_NOMEM;
        }
        v.extend_from_slice(&f.out_pl(p)[..n]);
        f.gpl[p] = v;
    }
    f.out = Out::Grain;
    let g = &h.fg;
    let w = h.upw;
    let sw = w as usize + 64;
    let (Ok(l), Ok(cb), Ok(cr), Ok(st)) = (zvec::<i32>(GW * GH), zvec::<i32>(GW * GH), zvec::<i32>(GW * GH), zvec::<i32>(sw * 2 * 3 * 34))
    else {
        return ERR_NOMEM;
    };
    let bd = h.bd;
    let center = 128i32 << (bd - 8); // GrainCenter
    let mut fx = Fx {
        g,
        sw,
        rand: 0,
        w,
        h: h.fh,
        subx: h.ssx,
        suby: h.ssy,
        gmin: -center,
        gmax: (256i32 << (bd - 8)) - 1 - center,
        shift: (12 - bd).wrapping_add(g.gshift),
        ln: 0,
        cur: 0,
        minv: 0,
        maxl: 0,
        maxc: 0,
        cw: 0,
        ch: 0,
        bd,
        ovl: g.overlap,
        grain: [l, cb, cr],
        lut: [[0; 256]; 3],
        stripes: st,
    };
    // luma grain
    fx.rand = g.seed;
    fx.white(0, g.ny != 0, GW, GH);
    fx.arluma();
    if h.mono == 0 {
        // chroma grain
        fx.cw = if fx.subx != 0 { 44 } else { GW };
        fx.ch = if fx.suby != 0 { 38 } else { GH };
        let (cw, ch) = (fx.cw, fx.ch);
        fx.rand = g.seed ^ 0xB524;
        fx.white(1, (g.ncb | g.csfl) != 0, cw, ch);
        fx.rand = g.seed ^ 0x49D8;
        fx.white(2, (g.ncr | g.csfl) != 0, cw, ch);
        fx.archroma();
    }
    fx.mklut(np);
    // clipping range
    let s = bd - 8;
    if g.clip != 0 {
        fx.minv = 16 << s;
        fx.maxl = 235 << s;
        fx.maxc = if h.mc == 0 { fx.maxl } else { 240 << s }; // MC_IDENTITY
    } else {
        fx.minv = 0;
        fx.maxl = (256 << s) - 1;
        fx.maxc = fx.maxl;
    }
    // noise stripes: ( ( h + 1 ) / 2 + 15 ) / 16
    let ns = ((fx.h + 1) / 2 + 15) / 16;
    fx.ln = 0;
    fx.cur = 0;
    while fx.ln < ns {
        fx.stripe(np);
        fx.apply(f, np);
        fx.cur ^= 1;
        fx.ln += 1;
    }
    0
}

impl Fx<'_> {
    /// fx_rand: get_random_number( bits )
    fn rand(&mut self, bits: u32) -> u32 {
        let r = self.rand;
        let bit = (r ^ (r >> 1) ^ (r >> 3) ^ (r >> 12)) & 1;
        let r = (r >> 1) | (bit << 15);
        self.rand = r;
        (r >> (16 - bits)) & ((1u32 << bits) - 1)
    }

    /// fx_white: grain[ y ][ x ] = draw ? Round2( Gaussian_Sequence[ get_random_number( 11 ) ], shift ) : 0
    fn white(&mut self, p: usize, draw: bool, w: usize, h: usize) {
        for y in 0..h {
            for x in 0..w {
                let mut v = 0;
                if draw {
                    let r = self.rand(11);
                    v = rnd2(GAUSSIAN_SEQUENCE[r as usize] as i32, self.shift);
                }
                self.grain[p][y * GW + x] = v;
            }
        }
    }

    /// fx_arluma: auto-regressive filter of LumaGrain
    fn arluma(&mut self) {
        let g = self.g;
        let lag = g.lag as i32;
        let l = &mut self.grain[0];
        for y in 3..GH {
            for x in 3..GW - 3 {
                let mut s = 0i32;
                let mut pos = 0usize;
                'dr: for dr in -lag..=0 {
                    for dc in -lag..=lag {
                        if dr == 0 && dc == 0 {
                            break 'dr;
                        }
                        let c = g.ary[pos] as i32 - 128;
                        let q = (y as i32 + dr) as usize * GW + (x as i32 + dc) as usize;
                        s = s.wrapping_add(c.wrapping_mul(l[q]));
                        pos += 1;
                    }
                }
                let q = y * GW + x;
                l[q] = rnd2(s, g.arshift).wrapping_add(l[q]).max(self.gmin).min(self.gmax);
            }
        }
    }

    /// fx_archroma: auto-regressive filter of CbGrain / CrGrain (with the luma term)
    fn archroma(&mut self) {
        let g = self.g;
        let lag = g.lag as i32;
        let (sx, sy) = (self.subx, self.suby);
        let [l, cb, cr] = &mut self.grain;
        for y in 3..self.ch {
            for x in 3..self.cw - 3 {
                let mut s0 = 0i32;
                let mut s1 = 0i32;
                let mut pos = 0usize;
                'dr: for dr in -lag..=0 {
                    for dc in -lag..=lag {
                        let c0 = g.arcb[pos] as i32 - 128;
                        let c1 = g.arcr[pos] as i32 - 128;
                        if dr == 0 && dc == 0 {
                            // luma term
                            if g.ny != 0 {
                                let lx = ((x - 3) << sx) + 3;
                                let ly = ((y - 3) << sy) + 3;
                                let q = ly * GW + lx;
                                let mut v = l[q];
                                if sx != 0 {
                                    v = v.wrapping_add(l[q + 1]);
                                }
                                if sy != 0 {
                                    v = v.wrapping_add(l[q + GW]);
                                    if sx != 0 {
                                        v = v.wrapping_add(l[q + GW + 1]);
                                    }
                                }
                                let v = rnd2(v, sx + sy);
                                s0 = s0.wrapping_add(v.wrapping_mul(c0));
                                s1 = s1.wrapping_add(v.wrapping_mul(c1));
                            }
                            break 'dr;
                        }
                        let q = (y as i32 + dr) as usize * GW + (x as i32 + dc) as usize;
                        s0 = s0.wrapping_add(cb[q].wrapping_mul(c0));
                        s1 = s1.wrapping_add(cr[q].wrapping_mul(c1));
                        pos += 1;
                    }
                }
                let q = y * GW + x;
                cb[q] = rnd2(s0, g.arshift).wrapping_add(cb[q]).max(self.gmin).min(self.gmax);
                cr[q] = rnd2(s1, g.arshift).wrapping_add(cr[q]).max(self.gmin).min(self.gmax);
            }
        }
    }

    /// fx_lut: scaling lookup initialization (ScalingLut[ plane ][ 256 ])
    fn mklut(&mut self, np: usize) {
        let g = self.g;
        for p in 0..np {
            let (pts, n): (&[[u8; 2]], u32) = if p == 0 || g.csfl != 0 {
                (&g.py, g.ny)
            } else if p == 1 {
                (&g.pcb, g.ncb)
            } else {
                (&g.pcr, g.ncr)
            };
            let lut = &mut self.lut[p];
            if n == 0 {
                lut.fill(0);
                continue;
            }
            let y0 = pts[0][1] as i32; // get_y( 0 )
            for v in &mut lut[..pts[0][0] as usize] {
                *v = y0;
            }
            for i in 0..(n - 1) as usize {
                let yi = pts[i][1] as i32;
                let dy = pts[i + 1][1] as i32 - yi; // deltaY
                let xi = pts[i][0] as i32;
                let dx = pts[i + 1][0] as i32 - xi; // deltaX
                if dx <= 0 {
                    continue; // (invalid order: nothing)
                }
                let delta = dy.wrapping_mul(((65536 + (dx as u32 >> 1)) / dx as u32) as i32);
                for x in 0..dx {
                    let v = (x.wrapping_mul(delta).wrapping_add(32768) >> 16).wrapping_add(yi);
                    lut[((xi + x) & 255) as usize] = v;
                }
            }
            let last = pts[(n - 1) as usize];
            for v in &mut lut[last[0] as usize..] {
                *v = last[1] as i32;
            }
        }
    }

    // index of noiseStripe[ buf ][ plane ][ row ][ 0 ]
    #[inline(always)]
    fn sidx(&self, buf: usize, p: usize, row: usize) -> usize {
        ((buf * 3 + p) * 34 + row) * self.sw
    }

    /// fx_stripe: noise stripe lumaNum into the current stripe buffer
    fn stripe(&mut self, np: usize) {
        let g = self.g;
        let ln = self.ln;
        self.rand = g.seed ^ ((ln.wrapping_mul(37).wrapping_add(178) & 255) << 8) ^ (ln.wrapping_mul(173).wrapping_add(105) & 255);
        let mut x = 0u32;
        while x < (self.w + 1) >> 1 {
            let r = self.rand(8);
            let ox = r >> 4; // offsetX
            let oy = r & 15; // offsetY
            for p in 0..np {
                let (psx, psy) = if p > 0 { (self.subx, self.suby) } else { (0, 0) };
                let pox = if psx != 0 { ox + 6 } else { ox * 2 + 9 }; // planeOffsetX
                let poy = if psy != 0 { oy + 6 } else { oy * 2 + 9 };
                for i in 0..(34 >> psy) as usize {
                    for j in 0..(34 >> psx) as usize {
                        let mut gv = self.grain[p][(poy as usize + i) * GW + pox as usize + j];
                        let col = if psx != 0 { x as usize + j } else { x as usize * 2 + j };
                        let idx = self.sidx(self.cur, p, i) + col;
                        if g.overlap != 0 && x != 0 {
                            let old = self.stripes[idx];
                            let mix = if psx == 0 {
                                match j {
                                    0 => Some(old.wrapping_mul(27).wrapping_add(gv.wrapping_mul(17))),
                                    1 => Some(old.wrapping_mul(17).wrapping_add(gv.wrapping_mul(27))),
                                    _ => None,
                                }
                            } else if j == 0 {
                                Some(old.wrapping_mul(23).wrapping_add(gv.wrapping_mul(22)))
                            } else {
                                None
                            };
                            if let Some(m) = mix {
                                gv = (m.wrapping_add(16) >> 5).max(self.gmin).min(self.gmax);
                            }
                        }
                        self.stripes[idx] = gv;
                    }
                }
            }
            x += 16;
        }
    }

    /// fx_noise: noiseImage[ plane ][ y ][ x ] of the rows of the current stripe (vertical overlap
    /// with the previous one)
    #[inline]
    fn noise(&self, p: usize, y: u32, x: u32) -> i32 {
        let psy = if p > 0 { self.suby } else { 0 };
        let i = y.wrapping_sub(self.ln << (5 - psy));
        let gv = self.stripes[self.sidx(self.cur, p, i as usize) + x as usize];
        if self.ln == 0 || self.ovl == 0 {
            return gv;
        }
        let prev = self.cur ^ 1;
        let m = if psy == 0 {
            if i >= 2 {
                return gv;
            }
            let old = self.stripes[self.sidx(prev, p, i as usize + 32) + x as usize];
            if i == 0 {
                old.wrapping_mul(27).wrapping_add(gv.wrapping_mul(17))
            } else {
                old.wrapping_mul(17).wrapping_add(gv.wrapping_mul(27))
            }
        } else {
            if i != 0 {
                return gv;
            }
            let old = self.stripes[self.sidx(prev, p, 16) + x as usize];
            old.wrapping_mul(23).wrapping_add(gv.wrapping_mul(22))
        };
        (m.wrapping_add(16) >> 5).max(self.gmin).min(self.gmax)
    }

    /// fx_scalelut: scale_lut( plane, index )
    #[inline]
    fn scalelut(&self, p: usize, index: i32) -> i32 {
        let sh = self.bd - 8;
        let x = ((index as u32) >> sh) & 255;
        let rem = index.wrapping_sub((x << sh) as i32);
        let start = self.lut[p][x as usize];
        if sh == 0 || x == 255 {
            return start;
        }
        let end = self.lut[p][x as usize + 1];
        start.wrapping_add(rnd2(end.wrapping_sub(start).wrapping_mul(rem), sh))
    }

    /// fx_apply: add the noise of the current stripe to the output rows
    fn apply(&self, f: &mut Frame, np: usize) {
        let g = self.g;
        let ops = f.ops;
        if np != 1 {
            // chroma rows of the stripe: y = ln << ( 5 - subY ) .. + ( 32 >> subY ), < ( h + subY ) >> subY
            let sh = 5 - self.suby;
            let y0 = self.ln << sh;
            let y1 = (y0 + (1 << sh)).min((self.h + self.suby) >> self.suby);
            let cwid = (self.w + self.subx) >> self.subx;
            let [gy, gu, gv] = &mut f.gpl;
            for y in y0..y1 {
                let lrow = ((y << self.suby) * ops[0]) as usize;
                for x in 0..cwid {
                    // averageLuma
                    let lx = x << self.subx;
                    let mut avg = gy[lrow + lx as usize] as i32;
                    if self.subx != 0 {
                        let nx = (lx + 1).min(self.w - 1); // lumaNextX
                        avg = (avg + gy[lrow + nx as usize] as i32 + 1) >> 1;
                    }
                    for p in 1..3usize {
                        let n = if p == 1 { g.ncb } else { g.ncr };
                        if n | g.csfl == 0 {
                            continue;
                        }
                        let pl: &mut Vec<u16> = if p == 1 { gu } else { gv };
                        let o = (y * ops[p] + x) as usize;
                        let orig = pl[o] as i32;
                        let mut merged = avg;
                        if g.csfl == 0 {
                            let (lm, m, off) = if p == 1 { (g.cblmult, g.cbmult, g.cboff) } else { (g.crlmult, g.crmult, g.croff) };
                            let comb = avg.wrapping_mul(lm as i32 - 128).wrapping_add(orig.wrapping_mul(m as i32 - 128));
                            merged = ((comb >> 6).wrapping_add((off as i32 - 256).wrapping_shl(self.bd - 8)))
                                .max(0)
                                .min((1i32 << self.bd) - 1);
                        }
                        let sc = self.scalelut(p, merged);
                        let nz = rnd2(sc.wrapping_mul(self.noise(p, y, x)), g.scaling);
                        pl[o] = orig.wrapping_add(nz).max(self.minv).min(self.maxc) as u16;
                    }
                }
            }
        }
        // luma
        if g.ny == 0 {
            return;
        }
        let y0 = self.ln << 5;
        let y1 = (y0 + 32).min(self.h);
        let gy = &mut f.gpl[0];
        for y in y0..y1 {
            for x in 0..self.w {
                let o = (y * ops[0] + x) as usize;
                let orig = gy[o] as i32;
                let sc = self.scalelut(0, orig);
                let nz = rnd2(sc.wrapping_mul(self.noise(0, y, x)), g.scaling);
                gy[o] = orig.wrapping_add(nz).max(self.minv).min(self.maxl) as u16;
            }
        }
    }
}
