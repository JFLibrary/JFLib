// venc -- JPEG XL writer, lossy VarDCT (model: tools/jxl_enc_ref.py encode_vardct; byte-exact)
//                                                                       [src/img/jxl_venc.s]
//
//   jxl_venc(sf, p) -> file / ERR_*    quality p.quality (0 = 90) -> distance
//                                      (JxlEncoderDistanceFromQuality)
//
// Stage A of the model: XYB, DCT8 for every block, one quant field value, library
// quantization tables, no loop filters, B predicted from the reconstructed Y (default colour
// correlation), natural coefficient orders, 3 block contexts (one per channel), AC contexts
// in 7 fixed clusters, ANS.  DC and the AC metadata are modular sub-images with their own
// trees; alpha is an extra channel coded with a global tree.  The image is processed in bands
// of 32 block rows, twice (token statistics, then the sections).

use alloc::vec;
use alloc::vec::Vec;

use gfx::FMT_ARGB;
use img_core::meta::get_row;
use img_core::{Surface, ERR_CANCEL, ERR_DIMS, IMG_MAXDIM};
use rt::fx::{cvt, cvtt, maxss, minss};

use crate::dct::{jd_fdct8, Scr};
use crate::enc::{cgrad, je_container, je_sizehdr, pack_signed, tree_write, EncParams, Sink};
use crate::ewr::{Eh, Es, Jw};
use crate::tab;
use crate::vac::jv_natorder;
use crate::vqt::jq_powf;

const GDIM: u32 = 256;
const NACCTX: u32 = 3 * (37 + 458); // AC contexts (3 block contexts)
static D_GS: [u32; 8] = [1, 11, 2049, 11, 4097, 12, 8193, 16];
static D_QDC: [u32; 8] = [16, 0, 1, 5, 1, 8, 1, 16];
static D_TOC: [u32; 8] = [0, 10, 1024, 14, 17408, 22, 4211712, 30];
static CORDER: [usize; 3] = [1, 0, 2];
static BCTX: [u32; 3] = [1, 0, 2]; // block context of channel X, Y, B

/// the writer constant VENC[i] (jxl_tab.s; shared with the new writer, jxl-module)
#[inline(always)]
pub fn v(i: usize) -> f32 {
    tab::VENC[i]
}

/// ve_distance: JxlEncoderDistanceFromQuality (f64)
pub fn distance(q: u32) -> f32 {
    let qd = q as i32 as f64;
    if q as i32 >= 30 {
        (((100 - q as i32) as f64) * tab::VENCD[1] + tab::VENCD[0]) as f32
    } else {
        let mut d = tab::VENCD[2] * qd;
        d = d * qd;
        let t = tab::VENCD[3] * qd;
        d = d - t;
        d = d + tab::VENCD[4];
        d as f32
    }
}

/// ve_cbrt: cbrt_f32: 0 for a <= 0, else bits / 3 + 0x2a5137a0 and three steps
#[inline]
pub fn cbrt(a: f32) -> f32 {
    if !(a > 0.0) {
        return 0.0;
    }
    let mut x = f32::from_bits((a.to_bits() / 3).wrapping_add(0x2a51_37a0));
    for _ in 0..3 {
        let xx = x + x;
        let x2 = x * x;
        let t = a / x2;
        x = (xx + t) * v(10);
    }
    x
}

/// ve_qr: rint( val ) when |val| >= thr, else 0
#[inline]
fn qr(val: f32, thr: f32) -> i32 {
    let a = f32::from_bits(val.to_bits() & 0x7FFF_FFFF);
    if !(a >= thr) { 0 } else { cvt(val) }
}

/// ve_adj: AdjustQuantBias with an exact division
#[inline]
fn adj(q: i32, c: usize) -> f32 {
    let qf = q as f32;
    let a = f32::from_bits(qf.to_bits() & 0x7FFF_FFFF);
    if a >= v(34) {
        return qf - tab::OPSIN_DEF[15] / qf;
    }
    if !(a > 0.0) {
        return 0.0;
    }
    let b = tab::OPSIN_DEF[12 + c];
    if qf > 0.0 { b } else { f32::from_bits(b.to_bits() ^ 0x8000_0000) }
}

/// a channel of ve_modlocal: samples (offset into a buffer), width, height, row stride
#[derive(Clone, Copy)]
struct Cd {
    off: usize,
    w: u32,
    h: u32,
    s: u32,
}

/// ve_res: clamped gradient residuals of a channel
fn res(buf: &[i32], cd: &Cd, sink: &mut Sink, leaf: u32) {
    for y in 0..cd.h as usize {
        let row = cd.off + y * cd.s as usize;
        for x in 0..cd.w as usize {
            let v = buf[row + x];
            let (l, t, tl) = if y == 0 {
                let l = if x > 0 { buf[row + x - 1] } else { 0 };
                (l, l, l)
            } else {
                let a = row - cd.s as usize;
                let t = buf[a + x];
                if x > 0 { (buf[row + x - 1], t, buf[a + x - 1]) } else { (t, t, t) }
            };
            sink.emit(leaf, pack_signed(v.wrapping_sub(cgrad(l, t, tl))));
        }
    }
}

/// the VarDCT encoder state (VE)
struct Ve<'a> {
    sf: &'a Surface,
    p: &'a EncParams<'a>,
    w: u32,
    h: u32,
    alpha: bool,
    xb: u32,
    yb: u32,
    xg: u32,
    yg: u32,
    ng: u32,
    xdg: u32,
    ndg: u32,
    gs: u32,
    qdc: u32,
    qf: u32,
    small: bool,
    by0: u32,
    bh: u32,
    pw: u32,
    dist: f32,
    muldc: [f32; 3],
    row: Vec<u8>,
    pl: Vec<f32>,   // XYB planes of a block row [3][8][pw]
    dc: Vec<i32>,   // quantized DC [3][yb][xb] (X, Y, B)
    band: Vec<i16>, // quantized AC of the band [32][xb][3][64]
    scr: Scr,
    ehac: Eh,
    eha: Eh,
    alb: Vec<i32>,  // alpha of the band [256][w]
    meta: Vec<i32>, // AC metadata channels scratch
    soff: Vec<usize>,
    step: [[f32; 64]; 3],
    thr: [[f32; 64]; 2],
    ord: [u16; 64],
    co: [[f32; 64]; 3],
    dy: [f32; 64],
    nz: Vec<i32>, // [3][32][32]
    gw: Jw,
    secw: Jw,
    outw: Jw,
}

/// jxl_venc
pub fn jxl_venc(sf: &Surface, p: &EncParams) -> Result<Vec<u8>, i32> {
    let q = if p.quality != 0 { p.quality } else { 90 };
    let dist = distance(q);
    let w = sf.w;
    let h = sf.h;
    if w <= 0 || h <= 0 || w as u32 > IMG_MAXDIM || h as u32 > IMG_MAXDIM {
        return Err(ERR_DIMS);
    }
    let mut ve = Ve::setup(sf, p, dist);
    ve.run()?;
    let mut cs = core::mem::take(&mut ve.outw.buf);
    je_container(p, &mut cs)?;
    Ok(cs)
}

impl<'a> Ve<'a> {
    /// ve_setup: sizes, quantizer, tables, buffers, histogram sets
    fn setup(sf: &'a Surface, p: &'a EncParams<'a>, dist: f32) -> Ve<'a> {
        let w = sf.w as u32;
        let h = sf.h as u32;
        let xb = (w + 7) >> 3;
        let yb = (h + 7) >> 3;
        let xdg = (xb + GDIM - 1) >> 8;
        let ydg = (yb + GDIM - 1) >> 8;
        let xg = (w + GDIM - 1) >> 8;
        let yg = (h + GDIM - 1) >> 8;
        // ---- quantizer (quant_params)
        let d = dist;
        let mut pp = jq_powf(d * v(17), v(16));
        pp = pp * v(18);
        let dcd = maxss(minss(pp, d), d * v(11));
        let qdcf = minss(v(14) / dcd, v(19));
        let qac = v(15) / d;
        let mut sc = (v(20) * qac) / v(21);
        sc = minss(maxss(sc, v(35)), v(36));
        let mut gs = cvtt(sc);
        let sq = cvtt(qdcf * v(22));
        if gs > sq {
            gs = sq;
            if gs < 1 {
                gs = 1;
            }
        }
        let inv = (tab::VENCD[5] / gs as f64) as f32;
        let mut qdc = cvtt(minss(qdcf * inv + v(11), v(20)));
        if qdc < 1 {
            qdc = 1;
        }
        let qf = cvtt(qac * inv + v(11)).clamp(1, 256);
        let t = inv / qdc as f32;
        let muldc = [t * v(23), t * v(24), t * v(25)];
        let sds = inv / qf as f32;
        let xs = sds * v(12);
        let bs = sds * v(13);
        let mut step = [[0f32; 64]; 3];
        let mut thr = [[0f32; 64]; 2];
        for k in 0..64 {
            step[0][k] = tab::DQ8[k] * xs;
            step[1][k] = tab::DQ8[64 + k] * sds;
            step[2][k] = tab::DQ8[128 + k] * bs;
            let qd = ((k >> 4) & 2) | ((k >> 2) & 1);
            thr[0][k] = v(26 + qd);
            thr[1][k] = v(30 + qd);
        }
        let mut ord = [0u16; 64];
        jv_natorder(0, &mut ord);
        // AC context clusters: non-zero counts cluster 0, then per block context the 458
        // coefficient ones
        let mut acmap = vec![0u8; NACCTX as usize + 16];
        for i in 0..3 * 458 {
            acmap[3 * 37 + i] = tab::ACCLUS[i % 458];
        }
        acmap.truncate(NACCTX as usize);
        let mut ve = Ve {
            sf,
            p,
            w,
            h,
            alpha: false,
            xb,
            yb,
            xg,
            yg,
            ng: xg * yg,
            xdg,
            ndg: xdg * ydg,
            gs: gs as u32,
            qdc: qdc as u32,
            qf: qf as u32,
            small: w <= GDIM && h <= GDIM,
            by0: 0,
            bh: 0,
            pw: xb * 8,
            dist,
            muldc,
            row: vec![0; w as usize * 4 + 64],
            pl: vec![0.0; xb as usize * 8 * 3 * 8],
            dc: vec![0; xb as usize * yb as usize * 3],
            band: vec![0; xb as usize * 32 * 3 * 64],
            scr: Scr::new(),
            ehac: Eh::new(NACCTX, Some(acmap)),
            eha: Eh::new(1, None),
            alb: Vec::new(),
            meta: vec![0; (GDIM * GDIM * 3) as usize],
            soff: vec![0; (xdg * ydg + xg * yg + 4) as usize],
            step,
            thr,
            ord,
            co: [[0.0; 64]; 3],
            dy: [0.0; 64],
            nz: vec![0; 3 * 1024],
            gw: Jw::new(),
            secw: Jw::new(),
            outw: Jw::new(),
        };
        // ---- alpha?
        'a: for y in 0..h as usize {
            get_row(sf, y, &mut ve.row, FMT_ARGB);
            for x in 0..w as usize {
                if ve.row[x * 4 + 3] != 255 {
                    ve.alpha = true;
                    break 'a;
                }
            }
        }
        if ve.alpha {
            ve.alb = vec![0; w as usize * 256];
        }
        ve
    }

    fn run(&mut self) -> Result<(), i32> {
        let _ = self.dist;
        // ---- pass 1: blocks, DC, AC token statistics (and alpha residuals)
        let mut ehac = core::mem::replace(&mut self.ehac, Eh::new(0, None));
        let mut eha = core::mem::replace(&mut self.eha, Eh::new(0, None));
        for gy in 0..self.yg {
            self.band_load(gy)?;
            for gx in 0..self.xg {
                self.gtok(gx, &mut Sink::Count(&mut ehac));
            }
            if self.alpha {
                self.aband(gy);
                for gx in 0..self.xg {
                    self.agroup(gx, &mut Sink::Count(&mut eha));
                }
            }
        }
        let r = ehac.finish();
        if r != 0 {
            return Err(r);
        }
        if self.alpha {
            let r = eha.finish();
            if r != 0 {
                return Err(r);
            }
        }
        // ---- DC global section
        let mut gw = core::mem::take(&mut self.gw);
        let r = self.dcglobal(&mut gw, &eha);
        if r != 0 {
            return Err(r);
        }
        let mut secw = core::mem::take(&mut self.secw);
        let mut nsec = 0usize;
        if self.ng == 1 {
            // one section: DC group 0, AC global, AC group 0 follow
            let r = self.dcgroup(0, &mut gw);
            if r != 0 {
                return Err(r);
            }
            let r = self.acglobal(&mut gw, &ehac);
            if r != 0 {
                return Err(r);
            }
            self.acgroup(0, &mut gw, &ehac, &eha); // (the band of pass 1 is still loaded)
            gw.pad();
        } else {
            gw.pad();
            for g in 0..self.ndg {
                let r = self.dcgroup(g, &mut secw);
                if r != 0 {
                    return Err(r);
                }
                secw.pad();
                self.soff[nsec] = secw.len();
                nsec += 1;
            }
            let r = self.acglobal(&mut secw, &ehac);
            if r != 0 {
                return Err(r);
            }
            secw.pad();
            self.soff[nsec] = secw.len();
            nsec += 1;
            // ---- pass 2: AC groups
            for gy in 0..self.yg {
                self.band_load(gy)?;
                if self.alpha {
                    self.aband(gy);
                }
                for gx in 0..self.xg {
                    self.acgroup(gx, &mut secw, &ehac, &eha);
                    secw.pad();
                    self.soff[nsec] = secw.len();
                    nsec += 1;
                }
            }
        }
        // ---- codestream
        self.headers(gw.len(), nsec);
        self.outw.data(&gw.buf);
        self.outw.data(&secw.buf);
        Ok(())
    }

    /// ve_xybrow: image row min( y, h - 1 ) as X, Y, B into plane row r, padded by
    /// repeating the last column
    fn xybrow(&mut self, y: u32, r: usize) {
        let y = y.min(self.h - 1);
        get_row(self.sf, y as usize, &mut self.row, FMT_ARGB);
        let cb = cbrt(v(9));
        let pw = self.pw as usize;
        let ps = pw * 8; // floats per plane (8 rows)
        let o = r * pw;
        for x in 0..self.w as usize {
            let rr = tab::SRGBLIN[self.row[x * 4 + 2] as usize];
            let gg = tab::SRGBLIN[self.row[x * 4 + 1] as usize];
            let bb = tab::SRGBLIN[self.row[x * 4] as usize];
            let mut l = [0f32; 3];
            for (i, li) in l.iter_mut().enumerate() {
                let mut m = v(3 * i) * rr + v(3 * i + 1) * gg;
                m = m + v(3 * i + 2) * bb;
                m = m + v(9);
                m = maxss(m, 0.0);
                *li = cbrt(m) - cb;
            }
            self.pl[o + x] = (l[0] - l[1]) * v(11);
            self.pl[ps + o + x] = (l[0] + l[1]) * v(11);
            self.pl[2 * ps + o + x] = l[2];
        }
        let last = self.w as usize - 1;
        for x in self.w as usize..pw {
            for c in 0..3 {
                self.pl[c * ps + o + x] = self.pl[c * ps + o + last];
            }
        }
    }

    /// ve_band: every block of group row gy: XYB, DCT8, quantized DC and AC
    fn band_load(&mut self, gy: u32) -> Result<(), i32> {
        if self.p.cancelled() {
            return Err(ERR_CANCEL);
        }
        self.by0 = gy << 5;
        self.bh = (self.yb - self.by0).min(32);
        let pw = self.pw as usize;
        for by in 0..self.bh {
            for r in 0..8u32 {
                self.xybrow((self.by0 + by) * 8 + r, r as usize);
            }
            for bx in 0..self.xb {
                for c in 0..3 {
                    let mut patch = [0f32; 64];
                    let base = c * pw * 8 + bx as usize * 8;
                    for r in 0..8 {
                        patch[r * 8..r * 8 + 8].copy_from_slice(&self.pl[base + r * pw..base + r * pw + 8]);
                    }
                    let mut out = [0f32; 64];
                    jd_fdct8(&patch, &mut out, &mut self.scr);
                    self.co[c] = out;
                }
                self.quant(bx, by);
            }
        }
        Ok(())
    }

    /// ve_quant: block (bx, band row by): DC (Y, X, B - reconstructed Y) and AC
    fn quant(&mut self, bx: u32, by: u32) {
        let n = self.xb as usize * self.yb as usize;
        let di = ((self.by0 + by) * self.xb + bx) as usize;
        let qy = cvt(self.co[1][0] / self.muldc[1]);
        self.dc[n + di] = qy;
        let ry = qy as f32 * self.muldc[1];
        let qx = cvt(self.co[0][0] / self.muldc[0]);
        self.dc[di] = qx;
        let qb = cvt((self.co[2][0] - ry) / self.muldc[2]);
        self.dc[2 * n + di] = qb;
        let ri = ((by * self.xb + bx) as usize) * 3 * 64;
        for k in 0..64 {
            let mut q = qr(self.co[1][k] / self.step[1][k], self.thr[0][k]);
            if k == 0 {
                q = 0;
            }
            self.band[ri + 64 + k] = q as i16;
            self.dy[k] = adj(q, 1) * self.step[1][k];
        }
        for k in 0..64 {
            let mut q = qr(self.co[0][k] / self.step[0][k], self.thr[1][k]);
            if k == 0 {
                q = 0;
            }
            self.band[ri + k] = q as i16;
            let mut q = qr((self.co[2][k] - self.dy[k]) / self.step[2][k], self.thr[1][k]);
            if k == 0 {
                q = 0;
            }
            self.band[ri + 128 + k] = q as i16;
        }
    }

    /// ve_gtok: the AC tokens of group gx of the loaded band
    fn gtok(&mut self, gx: u32, sink: &mut Sink) {
        let x0 = gx << 5;
        let gw = (self.xb - x0).min(32);
        self.nz.fill(0);
        for by in 0..self.bh {
            for bx in 0..gw {
                for &c in &CORDER {
                    let bctx = BCTX[c];
                    let ri = (((by * self.xb + x0 + bx) * 3) as usize + c) * 64;
                    let nzp = c * 1024;
                    let idx = (by * 32 + bx) as usize;
                    let mut pred = if bx != 0 {
                        let l = self.nz[nzp + idx - 1] as u32;
                        if by != 0 { (l + self.nz[nzp + idx - 32] as u32 + 1) >> 1 } else { l }
                    } else if by != 0 {
                        self.nz[nzp + idx - 32] as u32
                    } else {
                        32
                    };
                    if pred > 64 {
                        pred = 64;
                    }
                    if pred >= 8 {
                        pred = (pred >> 1) + 4;
                    }
                    let ctx = pred * 3 + bctx;
                    let mut nze = 0u32;
                    for k in 1..64 {
                        if self.band[ri + self.ord[k] as usize] != 0 {
                            nze += 1;
                        }
                    }
                    self.nz[nzp + idx] = nze as i32;
                    sink.emit(ctx, nze);
                    let hbase = bctx * 458 + 3 * 37;
                    let mut prev = (nze <= 4) as u32;
                    let mut k = 1usize;
                    while k < 64 && nze != 0 {
                        let cx = (tab::NZCTX[nze as usize] as u32 + tab::FREQCTX[k] as u32) * 2 + prev + hbase;
                        let val = pack_signed(self.band[ri + self.ord[k] as usize] as i32);
                        prev = (val != 0) as u32;
                        nze -= prev;
                        sink.emit(cx, val);
                        k += 1;
                    }
                }
            }
        }
    }

    /// ve_aband: alpha of the 256 pixel rows of group row gy
    fn aband(&mut self, gy: u32) {
        let y0 = gy << 8;
        let w = self.w as usize;
        for r in 0..GDIM {
            let y = y0 + r;
            if y >= self.h {
                break;
            }
            get_row(self.sf, y as usize, &mut self.row, FMT_ARGB);
            for x in 0..w {
                self.alb[r as usize * w + x] = self.row[x * 4 + 3] as i32;
            }
        }
    }

    /// ve_agroup: alpha residuals of group gx (band in alb)
    fn agroup(&self, gx: u32, sink: &mut Sink) {
        let x0 = gx << 8;
        let gw = (self.w - x0).min(GDIM);
        let gh = self.h.wrapping_sub(self.by0 << 3).min(GDIM);
        let cd = Cd { off: x0 as usize, w: gw, h: gh, s: self.w };
        res(&self.alb, &cd, sink, 0);
    }

    /// ve_modlocal: modular sub-image with its own tree and histograms: header (local tree,
    /// default WP, no transforms), tree, histograms, residuals
    fn modlocal(jw: &mut Jw, buf: &[i32], cds: &[Cd]) -> i32 {
        let nch = cds.len() as u32;
        jw.bits(2, 4);
        let r = tree_write(jw, nch);
        if r != 0 {
            return r;
        }
        let mut eh = Eh::new(nch, None);
        for (i, cd) in cds.iter().enumerate() {
            res(buf, cd, &mut Sink::Count(&mut eh), nch - 1 - i as u32);
        }
        let r = eh.finish();
        if r != 0 {
            return r;
        }
        let r = eh.write(jw);
        if r != 0 {
            return r;
        }
        let mut es = Es::new();
        for (i, cd) in cds.iter().enumerate() {
            res(buf, cd, &mut Sink::Put(&eh, jw, &mut es), nch - 1 - i as u32);
        }
        es.flush(&eh, jw);
        0
    }

    /// ve_dcgroup: DC group g: extra precision 0, the DC image (Y, X, B), the strategy
    /// count, the AC metadata image (CfL tiles, strategies / quant field, sharpness)
    fn dcgroup(&mut self, g: u32, jw: &mut Jw) -> i32 {
        let x0 = (g % self.xdg) << 8;
        let y0 = (g / self.xdg) << 8;
        let gw = (self.xb - x0).min(GDIM);
        let gh = (self.yb - y0).min(GDIM);
        jw.bits(0, 2);
        let n = self.xb as usize * self.yb as usize;
        let o = (y0 * self.xb + x0) as usize;
        let mk = |off: usize| Cd { off, w: gw, h: gh, s: self.xb };
        let cds = [mk(o + n), mk(o), mk(o + 2 * n)];
        let r = Self::modlocal(jw, &self.dc, &cds);
        if r != 0 {
            return r;
        }
        let cnt = gw * gh;
        let nb = if cnt - 1 == 0 { 0 } else { 32 - (cnt - 1).leading_zeros() };
        jw.bits((cnt - 1) as u64, nb);
        // metadata channels: zeros (tiles x 2, strategies, sharpness), qf - 1
        let z = (GDIM * GDIM * 2) as usize;
        self.meta[..z].fill(0);
        for i in 0..cnt as usize {
            self.meta[cnt as usize + i] = self.qf as i32 - 1;
        }
        let cw = (gw + 7) >> 3;
        let ch = (gh + 7) >> 3;
        let cds = [
            Cd { off: z, w: cw, h: ch, s: cw },
            Cd { off: z, w: cw, h: ch, s: cw },
            Cd { off: 0, w: cnt, h: 2, s: cnt },
            Cd { off: z, w: gw, h: gh, s: gw },
        ];
        Self::modlocal(jw, &self.meta, &cds)
    }

    /// ve_acglobal: library quantization tables, one histogram set, natural orders, the AC
    /// histograms
    fn acglobal(&self, jw: &mut Jw, ehac: &Eh) -> i32 {
        jw.bits(1, 1); // library tables
        if self.ng > 1 {
            let nb = 32 - (self.ng - 1).leading_zeros();
            jw.bits(0, nb);
        }
        jw.bits(2, 2); // used orders 0 (U32 selector 2)
        ehac.write(jw)
    }

    /// ve_acgroup: AC tokens of group gx of the loaded band; then its alpha (several groups)
    fn acgroup(&mut self, gx: u32, jw: &mut Jw, ehac: &Eh, eha: &Eh) {
        let mut es = Es::new();
        self.gtok(gx, &mut Sink::Put(ehac, jw, &mut es));
        es.flush(ehac, jw);
        if !self.alpha || self.small {
            return;
        }
        jw.bits(3, 4); // group header: global tree, no transforms
        let mut es = Es::new();
        self.agroup(gx, &mut Sink::Put(eha, jw, &mut es));
        es.flush(eha, jw);
    }

    /// ve_dcglobal: DC global section
    fn dcglobal(&mut self, jw: &mut Jw, eha: &Eh) -> i32 {
        jw.bits(1, 1); // default DC quantization
        jw.u32(self.gs, &D_GS);
        jw.u32(self.qdc, &D_QDC);
        // custom block context map: 0, no DC thresholds (3 x 4 bits), no qf thresholds
        // (4 bits), simple context map with 2 bits per entry: 1, 2
        jw.bits(1 << 17 | 2 << 18, 20);
        for i in 0..39u32 {
            jw.bits((i / 13) as u64, 2);
        }
        jw.bits(1, 1); // default colour correlation
        if !self.alpha {
            jw.bits(0, 1); // no global tree
            return 0;
        }
        jw.bits(1, 1); // global tree (one leaf), alpha histograms
        let r = tree_write(jw, 1);
        if r != 0 {
            return r;
        }
        let r = eha.write(jw);
        if r != 0 {
            return r;
        }
        jw.bits(3, 4); // global modular header: global tree, no transforms
        if !self.small {
            return 0;
        }
        // small image: the alpha channel follows
        self.aband(0);
        self.by0 = 0;
        let mut es = Es::new();
        self.agroup(0, &mut Sink::Put(eha, jw, &mut es));
        es.flush(eha, jw);
        0
    }

    /// ve_headers: signature, size header, image metadata, transform data, frame header, TOC
    fn headers(&mut self, glen: usize, nsec: usize) {
        let o = &mut self.outw;
        o.bits(0x0AFF, 16);
        je_sizehdr(o, self.w, self.h);
        if !self.alpha {
            o.bits(3, 2); // all_default metadata (8-bit sRGB, XYB) and default transform data
        } else {
            o.bits(0b10011101100000, 14);
        }
        o.pad();
        o.bits(0, 8); // frame: 0, regular, VarDCT, flags, upsampling
        if self.alpha {
            o.bits(0, 2); // extra channel upsampling 1
        }
        o.bits(3 | 2 << 3, 11); // x_qm_scale 3, b_qm_scale 2, passes 1, no custom size, replace
        if self.alpha {
            o.bits(0, 2);
        }
        o.bits(1, 12); // is_last, name, loop filter, extensions, TOC no permutation
        o.pad();
        o.u32(glen as u32, &D_TOC);
        if self.ng != 1 {
            let mut prev = 0u32;
            for i in 0..nsec {
                let e = self.soff[i] as u32;
                o.u32(e.wrapping_sub(prev), &D_TOC);
                prev = e;
            }
        }
        o.pad();
    }
}
