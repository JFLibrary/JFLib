// icc_eval -- the profile model and the float64 colour maths of icc_xform: tag lookup,
// curves (curv / para), LUT elements (lut8 / lut16 / mAB / mBA), PCS encodings, CIE Lab,
// 3x3 matrices, inverse TRC tables                                    [src/img/icc_xform.s]
//
//   pm_parse(data) -> Pm | ERR_*          header, media white, LUT tags, colorants, TRCs
//   lt_parse(el, pcslab, dir) -> Lut | ERR_*
//   cv_eval / curves3 / lt_eval / pcs_decode / pcs_encode / mat_apply / clamp3
//   mat3_mul / mat3_inv / inv_build
//
// Every element is a slice of exactly its tag's bytes (icc_check validated the tag table);
// each element checks its own sizes before a read, so no read leaves its tag.  Float code
// keeps the order of the assembly's SSE2 instructions (no FMA; minsd / maxsd / cvttsd2si
// through rt::fx, x^y through rt::x87::pow = fx_pow).

use crate::icc::icc_check;
use crate::{ERR_HDR, ERR_LIMIT, ERR_UNSUPPORTED};
use rt::fx::{cvttd, maxsd, minsd};
use rt::x87::pow;

// ------------------------------------------------------------------ signatures (LE dwords)
pub(crate) const TAG_RXYZ: u32 = 0x5A595872; // rXYZ
pub(crate) const TAG_GXYZ: u32 = 0x5A595867; // gXYZ
pub(crate) const TAG_BXYZ: u32 = 0x5A595862; // bXYZ
pub(crate) const TAG_RTRC: u32 = 0x43525472; // rTRC
pub(crate) const TAG_GTRC: u32 = 0x43525467; // gTRC
pub(crate) const TAG_BTRC: u32 = 0x43525462; // bTRC
pub(crate) const TAG_KTRC: u32 = 0x4352546B; // kTRC
pub(crate) const TAG_WTPT: u32 = 0x74707477; // wtpt
pub(crate) const TAG_DESC: u32 = 0x63736564; // desc
pub(crate) const TAG_CPRT: u32 = 0x74727063; // cprt
pub(crate) const TAG_CHAD: u32 = 0x64616863; // chad
const TAG_A2B0: u32 = 0x30423241; // A2B0 (A2B1 = +0x01000000 ...)
const TAG_B2A0: u32 = 0x30413242; // B2A0
pub(crate) const TYP_XYZ: u32 = 0x205A5958; // 'XYZ '
const TYP_CURV: u32 = 0x76727563; // curv
pub(crate) const TYP_PARA: u32 = 0x61726170; // para
const TYP_MFT1: u32 = 0x3174666D; // mft1 (lut8)
const TYP_MFT2: u32 = 0x3274666D; // mft2 (lut16)
const TYP_MAB: u32 = 0x2042416D; // 'mAB '
const TYP_MBA: u32 = 0x2041426D; // 'mBA '
pub(crate) const TYP_MLUC: u32 = 0x63756C6D; // mluc
pub(crate) const TYP_DESC: u32 = 0x63736564; // desc
pub(crate) const TYP_SF32: u32 = 0x32336673; // sf32
pub(crate) const CS_RGB: u32 = 0x20424752; // 'RGB '
pub(crate) const CS_GRAY: u32 = 0x59415247; // GRAY
pub(crate) const PCS_XYZ: u32 = 0x205A5958; // 'XYZ '
pub(crate) const PCS_LAB: u32 = 0x2062614C; // 'Lab '
pub(crate) const CL_MNTR: u32 = 0x72746E6D; // mntr
const CL_SCNR: u32 = 0x726E6373; // scnr
const CL_PRTR: u32 = 0x72747270; // prtr
const CL_SPAC: u32 = 0x63617073; // spac
pub(crate) const SIG_ACSP: u32 = 0x70736361; // acsp

pub(crate) static TAG_COLS: [u32; 3] = [TAG_RXYZ, TAG_GXYZ, TAG_BXYZ];
static TAG_TRCS: [u32; 3] = [TAG_RTRC, TAG_GTRC, TAG_BTRC];
pub(crate) static PARA_NP: [u8; 5] = [1, 3, 4, 5, 7]; // para parameters per function type

// ------------------------------------------------------------------ constants (.rodata)
pub(crate) const D_ZERO: f64 = 0.0;
pub(crate) const D_ONE: f64 = 1.0;
pub(crate) const D_HALF: f64 = 0.5;
const D_I65536: f64 = 1.52587890625e-05;
const D_I65535: f64 = 1.5259021896696422e-05;
pub(crate) const D_I255: f64 = 0.00392156862745098;
const D_I256: f64 = 0.00390625;
pub(crate) static D_D50: [f64; 3] = [0.9642, 1.0, 0.8249];
pub(crate) static D_IDENT: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
const D_XYZDEC: f64 = 1.999969482421875; // 65535 / 32768
const D_XYZENC: f64 = 0.5000076295109483; // 32768 / 65535
const D_LAB16L: f64 = 100.390625; // 100 * 65535 / 65280
const D_LAB16AB: f64 = 255.99609375; // 65535 / 256
const D_LAB16LE: f64 = 0.009961089494163424; // 65280 / 6553500
const D_LAB16ABE: f64 = 0.0039063096054016935; // 256 / 65535
const D_100: f64 = 100.0;
const D_I100: f64 = 0.01;
pub(crate) const D_255: f64 = 255.0;
const D_128: f64 = 128.0;
const D_16: f64 = 16.0;
const D_116: f64 = 116.0;
const D_I116: f64 = 0.008620689655172414;
const D_500: f64 = 500.0;
const D_I500: f64 = 0.002;
const D_200: f64 = 200.0;
const D_I200: f64 = 0.005;
const D_629: f64 = 0.20689655172413793; // 6 / 29
const D_429: f64 = 0.13793103448275862; // 4 / 29
const D_108_841: f64 = 0.12841854934601665; // 108 / 841
const D_841_108: f64 = 7.787037037037037; // 841 / 108
const D_LABLIM: f64 = 0.008856451679035631; // 216 / 24389
const D_THIRD: f64 = 0.3333333333333333;
const D_I4096: f64 = 0.000244140625;
const D_INF: f64 = 0.0001220703125; // 1 / (NF - 1)
const D_EPSDET: f64 = 1.0e-12;
pub(crate) const D_EPSID: f64 = 1.0e-5;
pub(crate) const D_EPSW: f64 = 1.0e-6;
pub(crate) const D_EPST: f64 = 1.0e-9;
pub(crate) const D_I1024: f64 = 0.0009765625;
pub(crate) const D_1024: f64 = 1024.0;
pub(crate) const D_32: f64 = 32.0;

pub(crate) const INV_N: usize = 4096; // sqrt-companded inverse table: INV_N + 1 entries
pub(crate) const NF: usize = 8193; // forward samples per destination curve

// ------------------------------------------------------------------ byte helpers

#[inline]
pub(crate) fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
pub(crate) fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
fn be16(d: &[u8], o: usize) -> u32 {
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

/// a big-endian s15Fixed16 as f64  [S15]
#[inline]
fn s15(d: &[u8], o: usize) -> f64 {
    be32(d, o) as i32 as f64 * D_I65536
}

/// v clamped to 0..1 (maxsd 0, minsd 1: NaN -> 0)
#[inline]
fn clamp01(v: f64) -> f64 {
    minsd(maxsd(v, D_ZERO), D_ONE)
}

// ------------------------------------------------------------------ curves (CV)
pub(crate) const CVT_ID: u32 = 0;
pub(crate) const CVT_PARA: u32 = 1; // curv with one entry = para type 0
pub(crate) const CVT_TAB: u32 = 2;
const CV_TAB_MAX: u32 = 0x100000; // most curv entries accepted

/// a curv or para element  [CV]
#[derive(Clone, Copy)]
pub(crate) struct Curve<'a> {
    pub kind: u32,   // CVT_*
    pub pt: u32,     // para function type 0..4
    pub n: u32,      // table entries (CVT_TAB, >= 2)
    pub tab: &'a [u8], // big-endian u16 table inside the profile
    pub p: [f64; 7], // g a b c d e f
}

pub(crate) const CV0: Curve<'static> = Curve { kind: CVT_ID, pt: 0, n: 0, tab: &[], p: [0.0; 7] };

/// a curv or para element -> (curve, bytes used) | ERR_HDR / ERR_UNSUPPORTED / ERR_LIMIT
/// [cv_parse]
fn cv_parse(p: &[u8]) -> Result<(Curve<'_>, usize), i32> {
    let size = p.len();
    let mut cv = CV0;
    if size < 12 {
        return Err(ERR_HDR);
    }
    match le32(p, 0) {
        TYP_CURV => {
            let n = be32(p, 8);
            if n > CV_TAB_MAX {
                return Err(ERR_LIMIT);
            }
            let used = n as usize * 2 + 12;
            if used > size {
                return Err(ERR_HDR);
            }
            if n == 1 {
                cv.p[0] = be16(p, 12) as f64 * D_I256; // u8Fixed8 gamma
                cv.kind = CVT_PARA;
            } else if n > 1 {
                cv.kind = CVT_TAB;
                cv.n = n;
                cv.tab = &p[12..used];
            }
            Ok((cv, used))
        }
        TYP_PARA => {
            let ft = be16(p, 8);
            if ft > 4 {
                return Err(ERR_UNSUPPORTED);
            }
            cv.pt = ft;
            let np = PARA_NP[ft as usize] as usize;
            let used = np * 4 + 12;
            if used > size {
                return Err(ERR_HDR);
            }
            cv.kind = CVT_PARA;
            for i in 0..np {
                cv.p[i] = s15(p, 12 + i * 4);
            }
            Ok((cv, used))
        }
        _ => Err(ERR_UNSUPPORTED),
    }
}

/// the curve at x (clamped to 0..1)  [cv_eval]
pub(crate) fn cv_eval(cv: &Curve, x: f64) -> f64 {
    let x = clamp01(x);
    match cv.kind {
        CVT_PARA => {
            let p = &cv.p;
            match cv.pt {
                0 => pow(x, p[0]),
                1 | 2 => {
                    // t = a x + b; t > 0 ? t^g (+ c) : 0 (+ c)
                    let r = pow(x * p[1] + p[2], p[0]);
                    if cv.pt == 2 { r + p[3] } else { r }
                }
                _ => {
                    // x >= d ? (a x + b)^g (+ e) : c x (+ f)
                    if !(x >= p[4]) {
                        let r = x * p[3];
                        if cv.pt == 4 { r + p[6] } else { r }
                    } else {
                        let r = pow(x * p[1] + p[2], p[0]);
                        if cv.pt == 4 { r + p[5] } else { r }
                    }
                }
            }
        }
        CVT_TAB => {
            let n = cv.n;
            let xs = x * (n - 1) as i32 as f64;
            let mut i = cvttd(xs) as u32;
            if i > n - 2 {
                i = n - 2;
            }
            let f = xs - i as i32 as f64;
            let a = be16(cv.tab, i as usize * 2) as f64;
            let b = be16(cv.tab, i as usize * 2 + 2) as f64;
            (a + (b - a) * f) * D_I65535
        }
        _ => x,
    }
}

/// v[i] = cv_eval(cvs[i], v[i])  [curves3]
pub(crate) fn curves3(cvs: &[Curve; 3], v: &mut [f64; 3]) {
    for i in 0..3 {
        v[i] = cv_eval(&cvs[i], v[i]);
    }
}

// ------------------------------------------------------------------ profile model (PM)
pub(crate) const PMF_MATRIX: u32 = 1; // RGB colorants + TRCs usable
pub(crate) const PMF_GRAY: u32 = 2; // gray kTRC usable

/// one parsed profile  [PM]
pub(crate) struct Pm<'a> {
    pub cs: u32,           // colour space signature
    pub pcs: u32,          // PCS signature
    pub flags: u32,        // PMF_*
    pub mat: [f64; 9],     // colorants, row-major (X row = rX gX bX)
    pub wtpt: [f64; 3],    // media white (absolute intent)
    pub cv: [Curve<'a>; 3], // TRCs (gray: kTRC three times)
    pub a2b: [Option<&'a [u8]>; 3],
    pub b2a: [Option<&'a [u8]>; 3],
}

/// the data of tag sig (icc_check validated the table)  [tag_find]
pub(crate) fn tag_find(data: &[u8], sig: u32) -> Option<&[u8]> {
    let n = be32(data, 128) as usize;
    for t in 0..n {
        let o = 132 + t * 12;
        if le32(data, o) == sig {
            let off = be32(data, o + 4) as usize;
            let sz = be32(data, o + 8) as usize;
            return Some(&data[off..off + sz]);
        }
    }
    None
}

/// the three values of an 'XYZ ' element (None for anything else)  [rd_xyz]
fn rd_xyz(p: &[u8]) -> Option<[f64; 3]> {
    if p.len() < 20 || le32(p, 0) != TYP_XYZ {
        return None;
    }
    Some([s15(p, 8), s15(p, 12), s15(p, 16)])
}

/// header, white point, LUT tags, colorants, TRCs  [pm_parse]
pub(crate) fn pm_parse(data: &[u8]) -> Result<Pm<'_>, i32> {
    let r = icc_check(data);
    if r < 0 {
        return Err(r as i32);
    }
    let mut pm = Pm {
        cs: le32(data, 16),
        pcs: le32(data, 20),
        flags: 0,
        mat: [0.0; 9],
        wtpt: D_D50,
        cv: [CV0; 3],
        a2b: [None; 3],
        b2a: [None; 3],
    };
    let class = le32(data, 12);
    let ver = data[8] as u32;
    if !matches!(class, CL_MNTR | CL_SCNR | CL_PRTR | CL_SPAC)
        || !matches!(pm.cs, CS_RGB | CS_GRAY)
        || !matches!(pm.pcs, PCS_XYZ | PCS_LAB)
    {
        return Err(ERR_UNSUPPORTED);
    }
    // media white: wtpt, D50 when missing or for v2 display profiles (lcms rule)
    if ver >= 4 || class != CL_MNTR {
        if let Some(w) = tag_find(data, TAG_WTPT).and_then(rd_xyz) {
            pm.wtpt = w;
        }
    }
    for i in 0..3u32 {
        pm.a2b[i as usize] = tag_find(data, TAG_A2B0.wrapping_add(i << 24));
        pm.b2a[i as usize] = tag_find(data, TAG_B2A0.wrapping_add(i << 24));
    }
    if pm.cs == CS_GRAY {
        let Some(el) = tag_find(data, TAG_KTRC) else { return Ok(pm) };
        let Ok((c, _)) = cv_parse(el) else { return Ok(pm) };
        pm.cv = [c; 3];
        // gray -> XYZ: Y * D50 from the G channel (colorant columns 0, D50, 0)
        pm.mat[1] = D_D50[0];
        pm.mat[4] = D_D50[1];
        pm.mat[7] = D_D50[2];
        pm.flags |= PMF_GRAY;
        return Ok(pm);
    }
    // RGB colorants: columns of mat
    for i in 0..3 {
        let Some(c) = tag_find(data, TAG_COLS[i]).and_then(rd_xyz) else { return Ok(pm) };
        pm.mat[i] = c[0];
        pm.mat[3 + i] = c[1];
        pm.mat[6 + i] = c[2];
    }
    for i in 0..3 {
        let Some(el) = tag_find(data, TAG_TRCS[i]) else { return Ok(pm) };
        let Ok((c, _)) = cv_parse(el) else { return Ok(pm) }; // unusable TRC: no matrix path
        pm.cv[i] = c;
    }
    pm.flags |= PMF_MATRIX;
    Ok(pm)
}

// ------------------------------------------------------------------ LUT stage (LT)
pub(crate) const LTT_LUT8: u32 = 1;
pub(crate) const LTT_LUT16: u32 = 2;
pub(crate) const LTT_MAB: u32 = 3;
pub(crate) const LTT_MBA: u32 = 4;
const LTF_A: u32 = 1;
const LTF_M: u32 = 2;
const LTF_CLUT: u32 = 4;
const LTF_MAT: u32 = 8;

/// a lut8 / lut16 / mAB / mBA element with 3 inputs and 3 outputs  [LT]
#[derive(Clone, Copy)]
pub(crate) struct Lut<'a> {
    pub ty: u32,          // LTT_*
    pub pcslab: bool,     // the PCS side is Lab
    pub flags: u32,       // LTF_*
    pub prec: u32,        // bytes per CLUT / table value (1, 2)
    pub g: [u32; 3],      // grid points per input (first = slowest)
    pub clut: &'a [u8],   // CLUT values
    pub intab: &'a [u8],  // lut8 / lut16 input tables
    pub outtab: &'a [u8], // output tables
    pub inn: u32,         // entries per input table
    pub outn: u32,        // entries per output table
    pub mat: [f64; 12],   // m00..m22, o0..o2
    pub ca: [Curve<'a>; 3],
    pub cm: [Curve<'a>; 3],
    pub cb: [Curve<'a>; 3],
}

pub(crate) const LUT0: Lut<'static> = Lut {
    ty: 0,
    pcslab: false,
    flags: 0,
    prec: 0,
    g: [0; 3],
    clut: &[],
    intab: &[],
    outtab: &[],
    inn: 0,
    outn: 0,
    mat: [0.0; 12],
    ca: [CV0; 3],
    cm: [CV0; 3],
    cb: [CV0; 3],
};

/// three consecutive curves at off (each 4-byte padded)  [lt_curves]
fn lt_curves(p: &[u8], off: u32) -> Result<[Curve<'_>; 3], i32> {
    let mut cvs = [CV0; 3];
    let mut off = off as usize;
    for c in cvs.iter_mut() {
        if off >= p.len() {
            return Err(ERR_HDR);
        }
        let (cv, used) = cv_parse(&p[off..])?;
        *c = cv;
        off += (used + 3) & !3;
    }
    Ok(cvs)
}

/// a LUT element; dir 0 = A2B (device -> PCS), 1 = B2A (the lut16 / lut8 matrix is used
/// only on an XYZ input, i.e. for B2A with PCS XYZ)  [lt_parse]
pub(crate) fn lt_parse(p: &[u8], pcslab: bool, dir: u32) -> Result<Lut<'_>, i32> {
    let size = p.len();
    let mut lt = LUT0;
    lt.pcslab = pcslab;
    if size < 32 {
        return Err(ERR_HDR);
    }
    let ty = le32(p, 0);
    if ty == TYP_MFT2 || ty == TYP_MFT1 {
        let hdr0;
        if ty == TYP_MFT2 {
            if size < 52 {
                return Err(ERR_HDR);
            }
            if p[8] != 3 || p[9] != 3 {
                return Err(ERR_UNSUPPORTED);
            }
            if p[10] < 2 {
                return Err(ERR_HDR);
            }
            let inn = be16(p, 48);
            if inn.wrapping_sub(2) > 4096 - 2 {
                return Err(ERR_HDR);
            }
            let outn = be16(p, 50);
            if outn.wrapping_sub(2) > 4096 - 2 {
                return Err(ERR_HDR);
            }
            lt.inn = inn;
            lt.outn = outn;
            lt.ty = LTT_LUT16;
            lt.prec = 2;
            hdr0 = 52;
        } else {
            if size < 48 {
                return Err(ERR_HDR);
            }
            if p[8] != 3 || p[9] != 3 {
                return Err(ERR_UNSUPPORTED);
            }
            if p[10] < 2 {
                return Err(ERR_HDR);
            }
            lt.inn = 256;
            lt.outn = 256;
            lt.ty = LTT_LUT8;
            lt.prec = 1;
            hdr0 = 48;
        }
        // common lut8 / lut16 layout: input tables, CLUT, output tables
        let grid = p[10] as usize;
        lt.g = [grid as u32; 3];
        lt.flags |= LTF_CLUT;
        let prec = lt.prec as usize;
        let a = hdr0;
        let b = a + lt.inn as usize * prec * 3;
        let c = b + grid * grid * grid * prec * 3;
        let e = c + lt.outn as usize * prec * 3;
        if e > size {
            return Err(ERR_HDR);
        }
        lt.intab = &p[a..b];
        lt.clut = &p[b..c];
        lt.outtab = &p[c..e];
        if dir == 1 && !pcslab {
            // B2A with PCS XYZ: the 3x3 matrix (s15.16 at 12)
            for i in 0..9 {
                lt.mat[i] = s15(p, 12 + i * 4);
            }
            lt.flags |= LTF_MAT;
        }
        return Ok(lt);
    }
    if ty == TYP_MAB {
        lt.ty = LTT_MAB;
    } else if ty == TYP_MBA {
        lt.ty = LTT_MBA;
    } else {
        return Err(ERR_UNSUPPORTED);
    }
    if p[8] != 3 || p[9] != 3 {
        return Err(ERR_UNSUPPORTED);
    }
    // B curves (mandatory)
    let off = be32(p, 12);
    if off == 0 {
        return Err(ERR_HDR);
    }
    lt.cb = lt_curves(p, off)?;
    // M curves
    let off = be32(p, 20);
    if off != 0 {
        lt.cm = lt_curves(p, off)?;
        lt.flags |= LTF_M;
    }
    // A curves
    let off = be32(p, 28);
    if off != 0 {
        lt.ca = lt_curves(p, off)?;
        lt.flags |= LTF_A;
    }
    // matrix: 12 s15.16
    let off = be32(p, 16) as usize;
    if off != 0 {
        if off + 48 > size {
            return Err(ERR_HDR);
        }
        for i in 0..12 {
            lt.mat[i] = s15(p, off + i * 4);
        }
        lt.flags |= LTF_MAT;
    }
    // CLUT: 16 grid bytes, precision, 3 pad, values
    let off = be32(p, 24) as usize;
    if off == 0 {
        return Ok(lt);
    }
    let mut end = off + 20; // end of the CLUT header
    if end > size {
        return Err(ERR_HDR);
    }
    let prec = p[off + 16] as u32;
    if prec.wrapping_sub(1) > 1 {
        return Err(ERR_HDR);
    }
    lt.prec = prec;
    let mut n = prec as usize; // value bytes
    for i in 0..3 {
        let g = p[off + i] as u32;
        if g < 2 {
            return Err(ERR_HDR);
        }
        lt.g[i] = g;
        n *= g as usize;
    }
    end += n * 3;
    if end > size {
        return Err(ERR_HDR);
    }
    lt.clut = &p[off + 20..end];
    lt.flags |= LTF_CLUT;
    Ok(lt)
}

// ------------------------------------------------------------------ LUT evaluation

/// v = m v (+ m[9..11] when offs); m f64 row-major  [mat_apply]
pub(crate) fn mat_apply(m: &[f64], v: &mut [f64; 3], offs: bool) {
    let (a, b, c) = (v[0], v[1], v[2]);
    for r in 0..3 {
        let mut x = m[r * 3] * a;
        x = x + m[r * 3 + 1] * b;
        x = x + m[r * 3 + 2] * c;
        if offs {
            x = x + m[9 + r];
        }
        v[r] = x;
    }
}

/// v[i] clamped to 0..1  [clamp3]
pub(crate) fn clamp3(v: &mut [f64; 3]) {
    for x in v.iter_mut() {
        *x = clamp01(*x);
    }
}

/// a lut8 / lut16 table (n entries of prec bytes) interpolated at x (clamped)  [lt_tab]
fn lt_tab(tab: &[u8], n: u32, prec: u32, x: f64) -> f64 {
    let x = clamp01(x);
    let xs = x * (n as i32 - 1) as f64;
    let mut i = cvttd(xs) as u32;
    if i > n - 2 {
        i = n - 2;
    }
    let f = xs - i as i32 as f64;
    let i = i as usize;
    let (a, b, k) = if prec == 2 {
        (be16(tab, i * 2), be16(tab, i * 2 + 2), D_I65535)
    } else {
        (tab[i] as u32, tab[i + 1] as u32, D_I255)
    };
    let a = a as f64;
    (a + (b as f64 - a) * f) * k
}

/// v[i] = lt_tab(channel i of tab, n, prec, v[i])  [lt_tabs]
fn lt_tabs(lt: &Lut, v: &mut [f64; 3], tab: &[u8], n: u32) {
    let ch = n as usize * lt.prec as usize;
    for i in 0..3 {
        v[i] = lt_tab(&tab[ch * i..ch * (i + 1)], n, lt.prec, v[i]);
    }
}

/// one normalised CLUT value  [clut_ld]
#[inline]
fn clut_ld(clut: &[u8], o: usize, prec: u32) -> f64 {
    if prec == 2 { be16(clut, o) as f64 * D_I65535 } else { clut[o] as f64 * D_I255 }
}

/// v = tetrahedral interpolation of the 3-in 3-out CLUT at v (clamped)  [clut_eval]
fn clut_eval(lt: &Lut, v: &mut [f64; 3]) {
    let prec = lt.prec as usize;
    let mut st = [0usize; 3];
    st[2] = prec * 3;
    st[1] = st[2] * lt.g[2] as usize;
    st[0] = st[1] * lt.g[1] as usize;
    let mut f = [0f64; 3];
    let mut base = 0usize;
    for d in 0..3 {
        let x = clamp01(v[d]);
        let g = lt.g[d];
        let xs = x * (g as i32 - 1) as f64;
        let mut i = cvttd(xs) as u32;
        if i > g - 2 {
            i = g - 2;
        }
        f[d] = xs - i as i32 as f64;
        base += i as usize * st[d];
    }
    // order the (f, stride) pairs descending by f
    for (i, j) in [(0, 1), (1, 2), (0, 1)] {
        if !(f[i] >= f[j]) {
            f.swap(i, j);
            st.swap(i, j);
        }
    }
    let mut out = [0f64; 3];
    for (ch, o) in out.iter_mut().enumerate() {
        let mut p = base + ch * prec;
        let c0 = clut_ld(lt.clut, p, lt.prec);
        let mut acc = c0;
        let mut prev = c0;
        for k in 0..3 {
            p += st[k];
            let c = clut_ld(lt.clut, p, lt.prec);
            acc = (c - prev) * f[k] + acc;
            prev = c;
        }
        *o = acc;
    }
    *v = out;
}

/// v (3 normalised values) through the LUT element  [lt_eval]
pub(crate) fn lt_eval(lt: &Lut, v: &mut [f64; 3]) {
    match lt.ty {
        LTT_MAB => {
            if lt.flags & LTF_A != 0 {
                curves3(&lt.ca, v);
            }
            if lt.flags & LTF_CLUT != 0 {
                clut_eval(lt, v);
            }
            if lt.flags & LTF_M != 0 {
                curves3(&lt.cm, v);
            }
            if lt.flags & LTF_MAT != 0 {
                mat_apply(&lt.mat, v, true);
            }
            curves3(&lt.cb, v);
        }
        LTT_MBA => {
            curves3(&lt.cb, v);
            if lt.flags & LTF_MAT != 0 {
                mat_apply(&lt.mat, v, true);
            }
            if lt.flags & LTF_M != 0 {
                curves3(&lt.cm, v);
            }
            if lt.flags & LTF_CLUT != 0 {
                clut_eval(lt, v);
            }
            if lt.flags & LTF_A != 0 {
                curves3(&lt.ca, v);
            }
        }
        _ => {
            if lt.flags & LTF_MAT != 0 {
                mat_apply(&lt.mat, v, false);
            }
            lt_tabs(lt, v, lt.intab, lt.inn);
            clut_eval(lt, v);
            lt_tabs(lt, v, lt.outtab, lt.outn);
        }
    }
}

// ------------------------------------------------------------------ Lab and PCS encodings

/// inverse of the CIE f function  [lab_finv]
fn lab_finv(t: f64) -> f64 {
    if !(t > D_629) { (t - D_429) * D_108_841 } else { t * t * t }
}

/// CIE Lab (D50) -> XYZ in place  [lab_to_xyz]
fn lab_to_xyz(v: &mut [f64; 3]) {
    let fy = (v[0] + D_16) * D_I116;
    let fx = v[1] * D_I500 + fy;
    let x = lab_finv(fx) * D_D50[0];
    let fz = fy - v[2] * D_I200;
    let z = lab_finv(fz) * D_D50[2];
    let y = lab_finv(fy);
    *v = [x, y, z];
}

/// CIE f (cube root above (6/29)^3)  [lab_f]
fn lab_f(t: f64) -> f64 {
    if !(t > D_LABLIM) { t * D_841_108 + D_429 } else { pow(t, D_THIRD) }
}

/// XYZ (D50) -> CIE Lab in place  [xyz_to_lab]
fn xyz_to_lab(v: &mut [f64; 3]) {
    let fx = lab_f(v[0] / D_D50[0]);
    let fy = lab_f(v[1]);
    let fz = lab_f(v[2] / D_D50[2]);
    *v = [fy * D_116 - D_16, (fx - fy) * D_500, (fy - fz) * D_200];
}

/// the element's PCS encoding (normalised) -> XYZ (D50)  [pcs_decode]
pub(crate) fn pcs_decode(lt: &Lut, v: &mut [f64; 3]) {
    if !lt.pcslab {
        for x in v.iter_mut() {
            *x = *x * D_XYZDEC;
        }
        return;
    }
    let (k1, k2) = if lt.ty == LTT_LUT16 { (D_LAB16L, D_LAB16AB) } else { (D_100, D_255) };
    v[0] = v[0] * k1;
    v[1] = v[1] * k2 - D_128;
    v[2] = v[2] * k2 - D_128;
    lab_to_xyz(v);
}

/// XYZ (D50) -> the element's PCS encoding (normalised, not clamped)  [pcs_encode]
pub(crate) fn pcs_encode(lt: &Lut, v: &mut [f64; 3]) {
    if !lt.pcslab {
        for x in v.iter_mut() {
            *x = *x * D_XYZENC;
        }
        return;
    }
    xyz_to_lab(v);
    let (k1, k2) = if lt.ty == LTT_LUT16 { (D_LAB16LE, D_LAB16ABE) } else { (D_I100, D_I255) };
    v[0] = v[0] * k1;
    v[1] = (v[1] + D_128) * k2;
    v[2] = (v[2] + D_128) * k2;
}

// ------------------------------------------------------------------ matrices, inverse TRCs

/// a b (f64 3x3 row-major)  [mat3_mul]
pub(crate) fn mat3_mul(a: &[f64; 9], b: &[f64; 9]) -> [f64; 9] {
    let mut out = [0f64; 9];
    for r in 0..3 {
        for c in 0..3 {
            let mut x = a[r * 3] * b[c];
            x = x + a[r * 3 + 1] * b[c + 3];
            x = x + a[r * 3 + 2] * b[c + 6];
            out[r * 3 + c] = x;
        }
    }
    out
}

/// m[i1] m[i2] - m[i3] m[i4]  [MDET3]
#[inline]
fn mdet(m: &[f64; 9], i1: usize, i2: usize, i3: usize, i4: usize) -> f64 {
    m[i1] * m[i2] - m[i3] * m[i4]
}

/// the f64 3x3 inverse, None when |det| <= 1e-12  [mat3_inv]
pub(crate) fn mat3_inv(m: &[f64; 9]) -> Option<[f64; 9]> {
    let a = mdet(m, 4, 8, 5, 7); // e i - f h
    let b = mdet(m, 5, 6, 3, 8); // f g - d i
    let c = mdet(m, 3, 7, 4, 6); // d h - e g
    let mut det = m[0] * a;
    det = det + m[1] * b;
    det = det + m[2] * c;
    if !(det.abs() > D_EPSDET) {
        return None;
    }
    let inv = D_ONE / det;
    let mut o = [0f64; 9];
    o[0] = a * inv;
    o[3] = b * inv;
    o[6] = c * inv;
    o[1] = mdet(m, 2, 7, 1, 8) * inv; // c h - b i
    o[2] = mdet(m, 1, 5, 2, 4) * inv; // b f - c e
    o[4] = mdet(m, 0, 8, 2, 6) * inv; // a i - c g
    o[5] = mdet(m, 2, 3, 0, 5) * inv; // c d - a f
    o[7] = mdet(m, 1, 6, 0, 7) * inv; // b g - a h
    o[8] = mdet(m, 0, 4, 1, 3) * inv; // a e - b d
    Some(o)
}

/// tab[j] = x with cv(x) = (j / INV_N)^2 (j = 0..INV_N), from NF forward samples made
/// monotonic (fw = scratch of NF values)  [inv_build]
pub(crate) fn inv_build(cv: &Curve, tab: &mut [f64], fw: &mut [f64]) {
    for i in 0..NF {
        let mut y = cv_eval(cv, i as i32 as f64 * D_INF);
        if i > 0 {
            y = maxsd(y, fw[i - 1]);
        }
        fw[i] = y;
    }
    let mut k = 0usize;
    for (j, t) in tab.iter_mut().enumerate().take(INV_N + 1) {
        let mut y = j as i32 as f64 * D_I4096;
        y = y * y;
        while k < NF - 2 && y > fw[k + 1] {
            k += 1;
        }
        let lo = fw[k];
        let hi = fw[k + 1];
        let mut f = 0.0;
        if hi > lo {
            f = clamp01((y - lo) / (hi - lo));
        }
        *t = (k as i32 as f64 + f) * D_INF;
    }
}
