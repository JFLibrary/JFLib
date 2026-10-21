// vrc -- JPEG XL VarDCT reconstruction of a group: dequantization with colour correlation,
// LLF from the DC image and inverse transforms into the pixel planes (dec_group.cc
// DecodeGroupImpl, DequantBlock; model: tools/jxl_ref.py Frame.vardct_pixels,
// adjust_quant_bias)                                                     [src/img/jxl_vrc.s]
//
//   jv_recon(jx, jf, g) -> 0 / ERR_NOMEM    all varblocks of group g (after its last pass);
//                                           the group's coefficients are zeroed afterwards

use alloc::boxed::Box;

use img_core::ERR_HDR;

use crate::dct::{jd_llf, jd_transform, Scr};
use crate::frm::Jf;
use crate::hdr::Jx;
use crate::tab;
use crate::vac::{jv_coefbuf, JV_COEFSZ};
use crate::vdc::{Jv, ACSCX, ACSCY};

static CORDER: [usize; 3] = [1, 0, 2];

/// dequantization parameters
struct Dq {
    sds: f32, // inverse global scale / quant field
    xs: f32,  // sds * x_dm
    bs: f32,  // sds * b_dm
    xcc: f32, // x from y factor
    bcc: f32, // b from y factor
    bias: [f32; 4],
}

/// AdjustQuantBias of one quantized value of channel c
#[inline(always)]
fn adj(q: i32, c: usize, p: &Dq) -> f32 {
    let qf = q as f32;
    let a = f32::from_bits(qf.to_bits() & 0x7FFF_FFFF);
    let safe = if qf == 0.0 { 1.0 } else { qf };
    let big = qf - p.bias[3] / safe;
    if a < 1.125 {
        if 0.0 < a { f32::from_bits((qf.to_bits() & 0x8000_0000) ^ p.bias[c].to_bits()) } else { 0.0 }
    } else {
        big
    }
}

/// vr_dequant: three channels of one varblock (coefficient channels JV_COEFSZ apart) into
/// the work blocks (wk, JV_COEFSZ apart); zeroes the coefficients
fn dequant(coef: &mut [i32], mat: &[f32], wk: &mut [f32], size: usize, p: &Dq) {
    let (m0, m1, m2) = (&mat[..size], &mat[size..2 * size], &mat[2 * size..3 * size]);
    for i in 0..size {
        let dy = (m1[i] * p.sds) * adj(coef[JV_COEFSZ + i], 1, p);
        wk[JV_COEFSZ + i] = dy;
        let dx = p.xcc * dy + adj(coef[i], 0, p) * (m0[i] * p.xs);
        wk[i] = dx;
        let db = p.bcc * dy + adj(coef[2 * JV_COEFSZ + i], 2, p) * (m2[i] * p.bs);
        wk[2 * JV_COEFSZ + i] = db;
        coef[i] = 0;
        coef[JV_COEFSZ + i] = 0;
        coef[2 * JV_COEFSZ + i] = 0;
    }
}

/// jv_recon: all varblocks of group g
pub fn jv_recon(jx: &Jx, jf: &mut Jf, g: u32) -> i32 {
    let Some(mut jv) = jf.vd.take() else { return ERR_HDR };
    let r = recon(jx, jf, &mut jv, g);
    jf.vd = Some(jv);
    r
}

fn recon(jx: &Jx, jf: &Jf, jv: &mut Jv, g: u32) -> i32 {
    if jv.rw.is_empty() {
        jv.rw = alloc::vec![0.0; 3 * JV_COEFSZ];
    }
    let bufk = match jv_coefbuf(jv, g) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let mut scr: Box<Scr> = jv.scr.take().unwrap_or_default();
    let mut coef = core::mem::take(&mut jv.coef[bufk]);
    let mut rw = core::mem::take(&mut jv.rw);
    let mut px = core::mem::take(&mut jv.px);
    let st = [jv.pxw[0] as usize, jv.pxw[1] as usize, jv.pxw[2] as usize];
    let r = recon_into(jx, jf, jv, g, &mut coef, &mut rw, &mut scr, &mut px, st, false);
    jv.coef[bufk] = coef;
    jv.rw = rw;
    jv.px = px;
    jv.scr = Some(scr);
    r
}

/// jv_recon of group g with caller buffers: coef (3 * JV_COEFSZ, zeroed afterwards), rw
/// (3 * JV_COEFSZ), the planes out (row strides st): the image planes, or with local a tile
/// of the group (its top left at 0, 0)  [jxl-plus: jv read-only, for the worker threads]
#[allow(clippy::too_many_arguments)]
pub fn recon_into(jx: &Jx, jf: &Jf, jv: &Jv, g: u32, coef: &mut [i32], rw: &mut [f32], scr: &mut Scr, out: &mut [alloc::vec::Vec<f32>; 3], st: [usize; 3], local: bool) -> i32 {
    let gy = g / jf.xg;
    let gx = g % jf.xg;
    let x0 = gx << 5;
    let y0 = gy << 5;
    let w = jf.xb.wrapping_sub(x0).min(32);
    let h = jf.yb.wrapping_sub(y0).min(32);
    let mut hs = [0u32; 3];
    let mut vs = [0u32; 3];
    for c in 0..3 {
        hs[c] = jf.hshift(c) & 31;
        vs[c] = jf.vshift(c) & 31;
    }
    let xb = jf.xb;
    let mut p = Dq { sds: 0.0, xs: 0.0, bs: 0.0, xcc: 0.0, bcc: 0.0, bias: jx.qbias };
    let mut off = 0usize;
    for by in 0..h {
        for bx in 0..w {
            let bi = (y0 + by).wrapping_mul(xb).wrapping_add(x0).wrapping_add(bx) as usize;
            let a8 = jv.acs[bi];
            if a8 & 1 == 0 {
                continue;
            }
            let a = (a8 >> 1) as usize;
            if a >= 27 {
                continue;
            }
            let size = 64usize << tab::ACSLOG2[a];
            let cy = ACSCY[a] as usize;
            let cx = ACSCX[a] as usize;
            // colour correlation of the 64 x 64 tile
            let ti = (((y0 + by) >> 3).wrapping_mul(jv.cw).wrapping_add((x0 + bx) >> 3)) as usize;
            p.xcc = jv.ytox[ti] as f32 * jv.cscale + jv.basex;
            p.bcc = jv.ytob[ti] as f32 * jv.cscale + jv.baseb;
            p.sds = jv.igs / jv.qf[bi] as f32;
            p.xs = p.sds * jv.xdm;
            p.bs = p.sds * jv.bdm;
            let t = tab::ACSQT[a] as usize;
            if !jv.qtab[t].is_empty() {
                {
                    let coef = &mut coef[off..];
                    let mat = &jv.qtab[t];
                    dequant(coef, mat, rw, size, &p);
                }
                // DC / lowest frequencies of every channel
                for c in 0..3 {
                    let sbx = (x0 >> hs[c]) + (bx >> hs[c]);
                    let sby = (y0 >> vs[c]) + (by >> vs[c]);
                    let di = sby.wrapping_mul(xb).wrapping_add(sbx) as usize;
                    let wk = &mut rw[c * JV_COEFSZ..];
                    if cy * cx == 1 {
                        wk[0] = jv.dc[c][di];
                    } else {
                        jd_llf(a as u32, &jv.dc[c][di..], xb as usize, wk, scr);
                    }
                }
                // inverse transforms, channel order 1, 0, 2
                for &c in &CORDER {
                    if (bx >> hs[c]) << hs[c] != bx || (by >> vs[c]) << vs[c] != by {
                        continue;
                    }
                    let sbx = (x0 >> hs[c]) + (bx >> hs[c]);
                    let sby = (y0 >> vs[c]) + (by >> vs[c]);
                    let pw = st[c];
                    let (ox, oy) = if local { ((x0 >> hs[c]) as usize, (y0 >> vs[c]) as usize) } else { (0, 0) };
                    let o = ((sby as usize - oy) * 8) * pw + (sbx as usize - ox) * 8;
                    let (wk, plane) = (&mut rw[c * JV_COEFSZ..c * JV_COEFSZ + size], &mut out[c]);
                    jd_transform(a as u32, wk, &mut plane[o..], pw, scr);
                }
            }
            off += size;
        }
    }
    0
}
