// vdc -- JPEG XL VarDCT DC side (quantizer.cc, entropy_coder.cc DecodeBlockCtxMap,
// chroma_from_luma.cc, dec_modular.cc DecodeVarDCTDC / DecodeAcMetadata, compressed_dc.cc,
// epf.cc ComputeSigma; model: tools/jxl_ref.py Frame.vardct_global .. dc_smoothing)
//                                                                        [src/img/jxl_vdc.s]
//
//   Jv                                      the VarDCT state (JV)
//   jv_global(jx, jf, jb) -> 0 / ERR_*      quantizer, block context map, colour correlation
//                                           DC; allocates jf.vd and its per block arrays
//   jv_dcgroup(jx, jf, g, jb) -> 0 / ERR_*  VarDCT DC of DC group g (dequantized, DC buckets)
//   jv_acmeta(jx, jf, g, jb) -> 0 / ERR_*   AC metadata of DC group g: CfL tiles, strategies,
//                                           quant field, EPF sharpness and sigma
//   jv_smooth(jf) -> 0 / ERR_*              adaptive DC smoothing
//   jv_dcrect(jf, g) -> [x0, y0, w, h]      DC group rectangle in blocks
//
// Float arithmetic is f32 in the model's order (no fused multiply-add).

use alloc::boxed::Box;
use alloc::vec::Vec;

use img_core::ERR_HDR;

use crate::bits::Jb;
use crate::ent::{je_ctxmap, Jc};
use crate::frm::Jf;
use crate::hdr::{unpack_signed, Jx};
use crate::modular::{jm_decode, zvec, Mi};
use rt::fx::maxss;

static D_GSCALE: [u32; 8] = [1, 11, 2049, 11, 4097, 12, 8193, 16];
static D_QDC: [u32; 8] = [16, 0, 1, 5, 1, 8, 1, 16];
static D_DCTHR: [u32; 8] = [0, 4, 16, 8, 272, 16, 65808, 32];
static D_QFTHR: [u32; 8] = [0, 2, 4, 3, 12, 5, 44, 8];
static D_CF: [u32; 8] = [84, 0, 256, 0, 2, 8, 258, 16];
static DEFCTX: [u8; 39] = [
    0, 1, 2, 2, 3, 3, 4, 5, 6, 6, 6, 6, 6, 7, 8, 9, 9, 10, 11, 12, 13, 14, 14, 14, 14, 14, 7, 8, 9, 9, 10, 11, 12,
    13, 14, 14, 14, 14, 14,
];
static YCOMP: [usize; 3] = [1, 0, 2];
static BUCKETORDER: [usize; 3] = [0, 2, 1];
/// AC strategy: covered blocks x / y  [vd_acscx, vd_acscy]
pub static ACSCX: [u8; 27] = [1, 1, 1, 1, 2, 4, 1, 2, 1, 4, 2, 4, 1, 1, 1, 1, 1, 1, 8, 4, 8, 16, 8, 16, 32, 16, 32];
pub static ACSCY: [u8; 27] = [1, 1, 1, 1, 2, 4, 2, 1, 4, 1, 4, 2, 1, 1, 1, 1, 1, 1, 8, 8, 4, 16, 16, 8, 32, 32, 16];

const ONE: f32 = 1.0;
const INVSIGMANUM: u32 = 0xbf95f61a; // kInvSigmaNum = -1.1715728752538099
const M1EM4: u32 = 0xb8d1b717; // -1e-4
const SW0: u32 = 0x3d561170; // DC smoothing: 1 - 4 (w1 + w2)
const SW1: u32 = 0x3e505590; // 0.20345139757231578
const SW2: u32 = 0x3d092563; // 0.0334829185968739

/// a quantization encoding (QE)
#[derive(Clone)]
pub struct Qe {
    pub mode: u32, // 0 library, 1 identity, 2 DCT2, 3 DCT4, 4 DCT4X8, 5 AFV, 6 DCT, 7 raw
    pub nb: u32,
    pub nb2: u32,
    pub b1: [f32; 48],
    pub b2: [f32; 48],
    pub w: [f32; 27],
    pub den: f32,
    pub raw: Vec<i32>,
}

impl Default for Qe {
    fn default() -> Self {
        Qe { mode: 0, nb: 0, nb2: 0, b1: [0.0; 48], b2: [0.0; 48], w: [0.0; 27], den: 0.0, raw: Vec::new() }
    }
}

impl Qe {
    /// library encoding k (jxt_qlib record)
    pub fn library(k: usize) -> Qe {
        let r = &crate::tab::QLIB[k * 132..k * 132 + 132];
        let f = |i: usize| f32::from_bits(r[i]);
        let mut q = Qe { mode: r[0], nb: r[1], nb2: r[2], ..Default::default() };
        for i in 0..48 {
            q.b1[i] = f(4 + i);
            q.b2[i] = f(52 + i);
        }
        for i in 0..27 {
            q.w[i] = f(100 + i);
        }
        q.den = f(127);
        q
    }
}

/// the VarDCT state (JV)
#[derive(Default)]
pub struct Jv {
    pub gscale: u32,
    pub quantdc: u32,
    pub igs: f32,
    pub gsf: f32,
    pub muldc: [f32; 3],
    pub ndcctx: u32,
    pub nqft: u32,
    pub nctx: u32,
    pub ndct: [u32; 3],
    pub dct: [[i32; 16]; 3],
    pub qft: [u32; 16],
    pub cf: u32,
    pub basex: f32,
    pub baseb: f32,
    pub ytoxdc: i32,
    pub ytobdc: i32,
    pub cscale: f32,
    pub cfldc: [f32; 3],
    pub is444: u32,
    pub usedacs: u32,
    pub dc: [Vec<f32>; 3],
    pub qdci: Vec<u8>,
    pub acs: Vec<u8>, // strategy << 1 | first block (0xFF = unset)
    pub qf: Vec<i32>,
    pub sharp: Vec<u8>,
    pub isig: Vec<f32>,
    pub ytox: Vec<i32>,
    pub ytob: Vec<i32>,
    pub cw: u32,
    pub ch: u32,
    pub ctxmap: Vec<u8>, // 2560
    pub qe: Vec<Qe>,     // 17 (jv_acglobal)
    pub qtab: Vec<Vec<f32>>, // 17, empty = not computed
    pub nhist: u32,
    pub nacctx: u32,
    pub ord: Vec<Vec<u16>>, // [passes][13][3] coefficient orders (empty = none)
    pub acc: Vec<Jc>,       // AC histograms per pass
    pub xdm: f32,
    pub bdm: f32,
    pub coef: Vec<Vec<i32>>, // coefficient buffers (3 planes of 65536 each, empty until used)
    pub px: [Vec<f32>; 3],   // pixel planes
    pub pxw: [u32; 3],
    pub pxh: [u32; 3],
    pub np: u32,
    pub ncb: u32,
    pub nz: Vec<i32>, // [3][32][32] non-zero counts of a group
    pub rw: Vec<f32>, // reconstruction work buffer (3 f32 blocks)
    pub scr: Option<Box<crate::dct::Scr>>, // transform scratch
    pub jcap: Option<Box<crate::jpeg::Cap>>, // jxl-plus: JPEG reconstruction (coefficients)
}

/// jv_dcrect: DC group rectangle in blocks: x0, y0, w, h
pub fn jv_dcrect(jf: &Jf, g: u32) -> [u32; 4] {
    let gy = g / jf.xdg;
    let gx = g % jf.xdg;
    let d = jf.gdim;
    let x0 = gx.wrapping_mul(d);
    let y0 = gy.wrapping_mul(d);
    [x0, y0, jf.xb.wrapping_sub(x0).min(d), jf.yb.wrapping_sub(y0).min(d)]
}

/// base + f32( f ) * colour_scale  [CFLRATIO]
#[inline]
pub fn cfl_ratio(jv: &Jv, f: i32, base: f32) -> f32 {
    f as f32 * jv.cscale + base
}

/// jv_global: quantizer, block context map, colour correlation DC, per block arrays
pub fn jv_global(jx: &Jx, jf: &mut Jf, jb: &mut Jb) -> i32 {
    let _ = jx;
    let mut jv = Box::<Jv>::default();
    jv.ctxmap = alloc::vec![0u8; 2560];
    // ---- quantizer
    jv.gscale = jb.u32f(&D_GSCALE);
    jv.quantdc = jb.u32f(&D_QDC);
    let gsd = jv.gscale as f64;
    let igs = (65536.0f64 / gsd) as f32;
    jv.igs = igs;
    jv.gsf = (gsd * (1.0 / 65536.0)) as f32;
    let m = igs / jv.quantdc as f32;
    for c in 0..3 {
        jv.muldc[c] = m * jf.dcq[c];
    }
    // ---- block context map
    jv.ndcctx = 1;
    let nent: u32;
    if jb.f(1) != 0 {
        jv.ctxmap[..39].copy_from_slice(&DEFCTX);
        nent = 39;
    } else {
        for c in 0..3 {
            let n = jb.f(4);
            jv.ndct[c] = n;
            jv.ndcctx = (n + 1).wrapping_mul(jv.ndcctx);
            for i in 0..n as usize {
                jv.dct[c][i] = unpack_signed(jb.u32f(&D_DCTHR));
            }
        }
        jv.nqft = jb.f(4);
        for i in 0..jv.nqft as usize {
            jv.qft[i] = jb.u32f(&D_QFTHR).wrapping_add(1);
        }
        let s = (jv.nqft + 1).wrapping_mul(jv.ndcctx);
        if s > 64 {
            jf.vd = Some(jv);
            return ERR_HDR;
        }
        nent = s * 39;
        let r = je_ctxmap(jb, nent, &mut jv.ctxmap);
        if r != 0 {
            jf.vd = Some(jv);
            return r;
        }
    }
    let mut mx = 0u32;
    for &v in jv.ctxmap.iter().take(nent.max(1) as usize) {
        mx = mx.max(v as u32);
    }
    if mx + 1 > 16 {
        jf.vd = Some(jv);
        return ERR_HDR;
    }
    jv.nctx = mx + 1;
    // ---- colour correlation DC
    jv.cf = 84;
    jv.basex = 0.0;
    jv.baseb = 1.0;
    jv.ytoxdc = 0;
    jv.ytobdc = 0;
    if jb.f(1) == 0 {
        jv.cf = jb.u32f(&D_CF);
        let bx = jb.f16();
        let bb = jb.f16();
        jv.basex = f32::from_bits(bx);
        jv.baseb = f32::from_bits(bb);
        if bx & 0x7FFF_FFFF > 0x4080_0000 || bb & 0x7FFF_FFFF > 0x4080_0000 {
            jf.vd = Some(jv);
            return ERR_HDR;
        }
        jv.ytoxdc = jb.f(8) as i32 - 128;
        jv.ytobdc = jb.f(8) as i32 - 128;
    }
    if jv.cf == 0 {
        jf.vd = Some(jv);
        return ERR_HDR;
    }
    jv.cscale = ONE / jv.cf as f32;
    jv.cfldc[0] = cfl_ratio(&jv, jv.ytoxdc, jv.basex);
    jv.cfldc[1] = 0.0;
    jv.cfldc[2] = cfl_ratio(&jv, jv.ytobdc, jv.baseb);
    // ---- per block arrays
    jv.is444 = ((jf.maxhs | jf.maxvs) == 0) as u32;
    let nb = jf.xb as u64 * jf.yb as u64;
    let nbu = nb as usize;
    let alloc_all = |jv: &mut Jv| -> Result<(), i32> {
        for c in 0..3 {
            jv.dc[c] = zvec(nbu)?;
        }
        jv.qdci = zvec(nbu)?;
        jv.acs = zvec(nbu)?;
        jv.acs.fill(0xFF);
        jv.sharp = zvec(nbu)?;
        jv.qf = zvec(nbu)?;
        jv.isig = zvec(nbu)?;
        jv.cw = (jf.xb + 7) >> 3;
        jv.ch = (jf.yb + 7) >> 3;
        let nt = jv.cw as usize * jv.ch as usize;
        jv.ytox = zvec(nt)?;
        jv.ytob = zvec(nt)?;
        Ok(())
    };
    let r = alloc_all(&mut jv);
    jf.vd = Some(jv);
    if let Err(e) = r {
        return e;
    }
    jb.check()
}

/// vd_modimg: jm_decode of an internal stream with the global tree / histograms (undo)
fn modimg(jf: &Jf, jb: &mut Jb, mi: &mut Mi, stream: u32) -> i32 {
    let gtree = if jf.gtree.nodes.is_empty() { None } else { Some(&jf.gtree) };
    let gcode = jf.gcode.as_deref();
    jm_decode(jb, mi, stream, 0xFF_FFFF, gtree, gcode, true, None)
}

/// jv_dcgroup: VarDCT DC of DC group g
pub fn jv_dcgroup(jx: &Jx, jf: &mut Jf, g: u32, jb: &mut Jb) -> i32 {
    match dc_decode(jx, jf, g, jb) {
        Ok((eb, mi)) => dc_apply(jf, g, eb, &mi),
        Err(e) => e,
    }
}

/// the entropy decoded part of jv_dcgroup: (extra precision, quantized DC image)
pub fn dc_decode(jx: &Jx, jf: &Jf, g: u32, jb: &mut Jb) -> Result<(u32, Mi), i32> {
    let rect = jv_dcrect(jf, g);
    let eb = jb.f(2);
    let mut mi = Mi::new(rect[2], rect[3], jx.bits);
    for i in 0..3 {
        let c = YCOMP[i];
        let r = mi.insert(i as u32, rect[2] >> (jf.hshift(c) & 31), rect[3] >> (jf.vshift(c) & 31), 0, 0);
        if r != 0 {
            return Err(r);
        }
    }
    let r = modimg(jf, jb, &mut mi, g + 1);
    if r != 0 {
        return Err(r);
    }
    Ok((eb, mi))
}

/// the rest of jv_dcgroup: dequantized DC and the DC buckets of DC group g
pub fn dc_apply(jf: &mut Jf, g: u32, eb: u32, mi: &Mi) -> i32 {
    let rect = jv_dcrect(jf, g);
    let mut hs = [0u32; 3];
    let mut vs = [0u32; 3];
    for c in 0..3 {
        hs[c] = jf.hshift(c);
        vs[c] = jf.vshift(c);
    }
    let mul = ONE / (1u32 << eb) as i32 as f32;
    let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
    // components: X = channel 1, Y = 0, B = 2
    let q: [&[i32]; 3] = [&mi.ch[1].d, &mi.ch[0].d, &mi.ch[2].d];
    let qw = [mi.ch[1].w, mi.ch[0].w, mi.ch[2].w];
    let xb = jf.xb;
    if jv.is444 != 0 {
        let mut f = [0f32; 3];
        for c in 0..3 {
            f[c] = jv.muldc[c] * mul;
        }
        for y in 0..rect[3] {
            let dst = (rect[1] + y).wrapping_mul(xb).wrapping_add(rect[0]) as usize;
            let src = qw[0].wrapping_mul(y) as usize;
            for x in 0..rect[2] as usize {
                let k = src + x;
                let inx = q[0][k] as f32 * f[0];
                let iny = q[1][k] as f32 * f[1];
                let inb = q[2][k] as f32 * f[2];
                let d = dst + x;
                jv.dc[1][d] = iny;
                jv.dc[0][d] = iny * jv.cfldc[0] + inx;
                jv.dc[2][d] = iny * jv.cfldc[2] + inb;
            }
        }
    } else {
        for c in 0..3 {
            let m = jv.muldc[c] * mul;
            let w = rect[2] >> (hs[c] & 31);
            let x0 = rect[0] >> (hs[c] & 31);
            let h = rect[3] >> (vs[c] & 31);
            let y0 = rect[1] >> (vs[c] & 31);
            for y in 0..h {
                let dst = (y0 + y).wrapping_mul(xb).wrapping_add(x0) as usize;
                let src = qw[c].wrapping_mul(y) as usize;
                for x in 0..w as usize {
                    jv.dc[c][dst + x] = q[c][src + x] as f32 * m;
                }
            }
        }
    }
    // DC buckets
    if jv.ndcctx > 1 {
        for y in 0..rect[3] {
            for x in 0..rect[2] {
                let mut acc = 0u32;
                for &c in &BUCKETORDER {
                    let k = ((y >> (vs[c] & 31)).wrapping_mul(qw[c]).wrapping_add(x >> (hs[c] & 31))) as usize;
                    let s = q[c][k];
                    let mut cnt = 0u32;
                    for t in 0..jv.ndct[c] as usize {
                        if s > jv.dct[c][t] {
                            cnt += 1;
                        }
                    }
                    acc = acc.wrapping_mul(jv.ndct[c] + 1).wrapping_add(cnt);
                }
                let d = (rect[1] + y).wrapping_mul(xb).wrapping_add(rect[0]).wrapping_add(x) as usize;
                jv.qdci[d] = acc as u8;
            }
        }
    }
    0
}

/// jv_dcframe: the DC of DC group g from the output of the DC frame one level up (kUseDcFrame:
/// no quantized DC, DC bucket 0)  [jxl-plus]
pub fn jv_dcframe(jx: &Jx, jf: &mut Jf, g: u32) -> i32 {
    let rect = jv_dcrect(jf, g);
    let lv = jf.dclevel as usize;
    if lv >= 4 {
        return ERR_HDR;
    }
    let Some(d) = jx.dcf[lv].as_ref() else { return ERR_HDR };
    let xb = jf.xb;
    let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
    if d.p.len() < 3 {
        return ERR_HDR;
    }
    for c in 0..3 {
        let p = &d.p[c];
        if p.w < rect[0] + rect[2] || p.h < rect[1] + rect[3] {
            return ERR_HDR;
        }
        for y in 0..rect[3] {
            let src = ((rect[1] + y) * p.s + rect[0]) as usize;
            let dst = ((rect[1] + y) * xb + rect[0]) as usize;
            let n = rect[2] as usize;
            jv.dc[c][dst..dst + n].copy_from_slice(&p.d[src..src + n]);
            jv.qdci[dst..dst + n].fill(0);
        }
    }
    0
}

/// jv_acmeta: AC metadata of DC group g
pub fn jv_acmeta(jx: &Jx, jf: &mut Jf, g: u32, jb: &mut Jb) -> i32 {
    match acmeta_decode(jx, jf, g, jb) {
        Ok((cnt, mi)) => acmeta_apply(jf, g, cnt, &mi),
        Err(e) => e,
    }
}

/// the entropy decoded part of jv_acmeta: (number of varblocks, the metadata image)
pub fn acmeta_decode(jx: &Jx, jf: &Jf, g: u32, jb: &mut Jb) -> Result<(u32, Mi), i32> {
    let rect = jv_dcrect(jf, g);
    let n = rect[2].wrapping_mul(rect[3]);
    let nb = if n <= 1 { 0 } else { 32 - (n - 1).leading_zeros() };
    let cnt = jb.f(nb).wrapping_add(1);
    let cw = (rect[2] + 7) >> 3;
    let ch = (rect[3] + 7) >> 3;
    let mut mi = Mi::new(rect[2], rect[3], jx.bits);
    for i in 0..4u32 {
        let (w, h, s) = match i {
            0 | 1 => (cw, ch, 3),
            2 => (cnt, 2, 0),
            _ => (rect[2], rect[3], 0),
        };
        let r = mi.insert(i, w, h, s, s);
        if r != 0 {
            return Err(r);
        }
    }
    let stream = jf.ndg.wrapping_mul(2).wrapping_add(g).wrapping_add(1);
    let r = modimg(jf, jb, &mut mi, stream);
    if r != 0 {
        return Err(r);
    }
    Ok((cnt, mi))
}

/// the rest of jv_acmeta: CfL tiles, strategies, quant field, sharpness, EPF sigma
pub fn acmeta_apply(jf: &mut Jf, g: u32, cnt: u32, mi: &Mi) -> i32 {
    let rect = jv_dcrect(jf, g);
    let cw = (rect[2] + 7) >> 3;
    let ch = (rect[3] + 7) >> 3;
    let xb = jf.xb;
    let yb = jf.yb;
    let qmul = jf.qmul;
    let epfit = jf.epfit;
    let sharp_lut = jf.sharp;
    let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
    // colour correlation tiles (clamped to i8)
    for y in 0..ch {
        for x in 0..cw {
            let s = (cw * y + x) as usize;
            let d = ((rect[1] >> 3) + y).wrapping_mul(jv.cw).wrapping_add(rect[0] >> 3).wrapping_add(x) as usize;
            for p in 0..2 {
                let v = mi.ch[p].d[s].clamp(-128, 127);
                if p == 0 {
                    jv.ytox[d] = v;
                } else {
                    jv.ytob[d] = v;
                }
            }
        }
    }
    // strategies, quant field, sharpness
    let xlim = (rect[0] + rect[2]).min(xb);
    let ylim = (rect[1] + rect[3]).min(yb);
    let acsd = &mi.ch[2].d;
    let shd = &mi.ch[3].d;
    let mut num = 0u32;
    for iy in 0..rect[3] {
        for ix in 0..rect[2] {
            let sv = shd[(rect[2] * iy + ix) as usize];
            if sv as u32 >= 8 {
                return ERR_HDR;
            }
            let bi = (rect[1] + iy).wrapping_mul(xb).wrapping_add(rect[0]).wrapping_add(ix) as usize;
            jv.sharp[bi] = sv as u8;
            if jv.acs[bi] != 0xFF {
                continue;
            }
            if num >= cnt {
                return ERR_HDR;
            }
            let a = acsd[num as usize] as u32;
            if a >= 27 {
                return ERR_HDR;
            }
            jv.usedacs |= 1 << a;
            let cx = ACSCX[a as usize] as u32;
            let cy = ACSCY[a as usize] as u32;
            if (cx | cy) > 1 && jv.is444 == 0 {
                return ERR_HDR;
            }
            let x = rect[0] + ix;
            let ex = x + cx;
            if ex > ((x >> 5) + 1) << 5 || ex > xlim {
                return ERR_HDR;
            }
            let y = rect[1] + iy;
            let ey = y + cy;
            if ey > ((y >> 5) + 1) << 5 || ey > ylim {
                return ERR_HDR;
            }
            let v = (a << 1) as u8;
            for yy in 0..cy {
                let row = yy.wrapping_mul(xb).wrapping_add(bi as u32);
                for xx in 0..cx {
                    let k = (row + xx) as usize;
                    if jv.acs[k] != 0xFF {
                        return ERR_HDR;
                    }
                    jv.acs[k] = v;
                    if yy == 0 && xx == 0 {
                        jv.acs[k] |= 1;
                    }
                }
            }
            let q = acsd[num.wrapping_add(cnt) as usize].clamp(0, 255) + 1;
            jv.qf[bi] = q;
            num += 1;
        }
    }
    // EPF sigma of the first blocks
    if epfit != 0 {
        for iy in 0..rect[3] {
            for ix in 0..rect[2] {
                let bi = (rect[1] + iy).wrapping_mul(xb).wrapping_add(rect[0]).wrapping_add(ix) as usize;
                let a = jv.acs[bi];
                if a & 1 == 0 {
                    continue;
                }
                let a = (a >> 1) as usize;
                let cx = ACSCX[a.min(26)] as u32;
                let cy = ACSCY[a.min(26)] as u32;
                let sq = qmul / ((jv.qf[bi] as f32 * jv.gsf) * f32::from_bits(INVSIGMANUM));
                for yy in 0..cy {
                    let row = yy.wrapping_mul(xb).wrapping_add(bi as u32);
                    for xx in 0..cx {
                        let k = (row + xx) as usize;
                        let mut s = sq * sharp_lut[jv.sharp[k] as usize & 7];
                        let m = f32::from_bits(M1EM4);
                        if !(s < m || s.is_nan()) {
                            s = m;
                        }
                        jv.isig[k] = ONE / s;
                    }
                }
            }
        }
    }
    0
}

/// the EPF sharpness of every block replaced (the writer's reconstructions), the inverse
/// sigmas recomputed as jv_acmeta does  [jxl-plus]
pub fn jv_sharpness(jf: &mut Jf, sharp: &[u8]) -> i32 {
    let (xb, yb) = (jf.xb, jf.yb);
    let qmul = jf.qmul;
    let sharp_lut = jf.sharp;
    let Some(jv) = jf.vd.as_mut() else { return ERR_HDR };
    let n = (xb * yb) as usize;
    if sharp.len() < n || jv.sharp.len() < n || jv.isig.len() < n {
        return ERR_HDR;
    }
    jv.sharp[..n].copy_from_slice(&sharp[..n]);
    for bi in 0..n {
        let a = jv.acs[bi];
        if a & 1 == 0 {
            continue;
        }
        let a = (a >> 1) as usize;
        let cx = ACSCX[a.min(26)] as usize;
        let cy = ACSCY[a.min(26)] as usize;
        let sq = qmul / ((jv.qf[bi] as f32 * jv.gsf) * f32::from_bits(INVSIGMANUM));
        for yy in 0..cy {
            for xx in 0..cx {
                let k = bi + yy * xb as usize + xx;
                let mut s = sq * sharp_lut[jv.sharp[k] as usize & 7];
                let m = f32::from_bits(M1EM4);
                if !(s < m || s.is_nan()) {
                    s = m;
                }
                jv.isig[k] = ONE / s;
            }
        }
    }
    0
}

/// jv_smooth: AdaptiveDCSmoothing (interior of the xb x yb DC planes)
pub fn jv_smooth(jf: &mut Jf) -> i32 {
    let w = jf.xb as usize;
    let h = jf.yb as usize;
    let Some(jv) = jf.vd.as_mut() else { return 0 };
    if w <= 2 || h <= 2 {
        return 0;
    }
    let mut new: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for c in 0..3 {
        new[c] = jv.dc[c].clone();
    }
    let (sw0, sw1, sw2) = (f32::from_bits(SW0), f32::from_bits(SW1), f32::from_bits(SW2));
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let mut gap = 0.5f32;
            let mut mc = [0f32; 3];
            let mut sm = [0f32; 3];
            for c in 0..3 {
                let d = &jv.dc[c];
                let corner = (d[i - w - 1] + d[i - w + 1]) + (d[i + w - 1] + d[i + w + 1]);
                let side = (d[i - 1] + d[i + 1]) + (d[i - w] + d[i + w]);
                let m = d[i];
                let s = corner * sw2 + (side * sw1 + m * sw0);
                mc[c] = m;
                sm[c] = s;
                let v = f32::from_bits(((m - s) / jv.muldc[c]).to_bits() & 0x7FFF_FFFF);
                gap = maxss(gap, v);
            }
            let f = maxss(gap * -4.0 + 3.0, 0.0);
            for c in 0..3 {
                new[c][i] = (sm[c] - mc[c]) * f + mc[c];
            }
        }
    }
    jv.dc = new;
    0
}
