// jpeg -- JPEG reconstruction: the original JPEG file of a JPEG XL JPEG transcode, from its
// jbrd box and the quantized DCT coefficients of its VarDCT frame (libjxl 0.11.1's
// jpeg_data.cc, dec_jpeg_data.cc, dec_jpeg_data_writer.cc, decode_to_jpeg.cc and the JPEG path
// of dec_group.cc / dec_frame.cc)                                        [jxl-plus: no asm]
//
//   reconstruct(data) -> JPEG bytes / ERR_*   the JPEG file (byte-identical to libjxl's
//                                             JxlDecoderSetJPEGBuffer output); ERR_UNSUPPORTED
//                                             without a jbrd box or for a frame that is no
//                                             JPEG (XYB, modular, not only DCT8, ...)
//   has_jbrd(data) -> bool                    the file has a JPEG reconstruction box
//   capture(jf, g) -> 0 / ERR_*               decoder hook (dec.rs, jf.dbg & JDBG_JPEG): the
//                                             JPEG coefficients of group g instead of pixels
//
// The jbrd box: a bit-packed JPEGData bundle (markers, tables, scans, restart / zero run /
// padding details) and a Brotli stream with the APP markers that are not ICC / EXIF / XMP,
// the COM markers, the data between markers and the tail.  ICC, EXIF and XMP markers are
// refilled from the codestream's ICC profile and the Exif / xml boxes.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use img_core::{ERR_HDR, ERR_NOMEM, ERR_UNSUPPORTED};

use crate::bits::{Jb, JXB_PAD};
use crate::dec::{JDBG_GRAY, JDBG_JPEG};
use crate::frm::{jf_header, jf_toc, Jf, JCT_NONE, JCT_XYB, JCT_YCBCR, JFF_USEDC};
use crate::vac::{jv_coefbuf, JV_COEFSZ};
use crate::vdc::Jv;

/// JPEG zigzag position -> natural (row-major) coefficient index
static NATURAL: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20, 13, 6, 7, 14, 21,
    28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61,
    54, 47, 55, 62, 63,
];
static CSHX: [u32; 4] = [0, 1, 1, 0];
static CSHY: [u32; 4] = [0, 1, 0, 1];

// U32 distributions of the JPEGData bundle: (offset, bits) per selector
static D_APPTYPE: [u32; 8] = [0, 0, 1, 0, 2, 1, 4, 2];
static D_ONETO4: [u32; 8] = [1, 0, 2, 0, 3, 0, 4, 0];
static D_NHUFF: [u32; 8] = [4, 0, 2, 3, 10, 4, 26, 6];
static D_COUNT: [u32; 8] = [0, 0, 1, 0, 2, 3, 0, 8];
static D_VALUE: [u32; 8] = [0, 2, 4, 2, 8, 4, 1, 8];
static D_LASTPASS: [u32; 8] = [0, 0, 1, 0, 2, 0, 3, 3];
static D_NUMRUNS: [u32; 8] = [0, 0, 1, 2, 4, 4, 20, 16];
static D_BLOCK: [u32; 8] = [0, 0, 1, 3, 9, 5, 41, 28];
static D_ZRUN: [u32; 8] = [1, 0, 2, 2, 5, 4, 20, 8];
static D_TAIL: [u32; 8] = [0, 0, 1, 8, 257, 16, 65793, 22];

const APP_UNKNOWN: u32 = 0;
const APP_ICC: u32 = 1;
const APP_EXIF: u32 = 2;
const APP_XMP: u32 = 3;
const ICC_TAG: &[u8; 12] = b"ICC_PROFILE\0";
const EXIF_TAG: &[u8; 6] = b"Exif\0\0";
const XMP_TAG: &[u8; 29] = b"http://ns.adobe.com/xap/1.0/\0";

#[derive(Clone, Copy)]
struct Quant {
    values: [i32; 64], // natural order
    precision: u32,
    index: u32,
    is_last: bool,
}

#[derive(Default)]
struct Comp {
    id: u32,
    h: u32,
    v: u32,
    quant_idx: u32,
    wb: u32,
    hb: u32,
    coeffs: Vec<i16>,
}

#[derive(Clone)]
struct Huff {
    slot: u32,
    is_last: bool,
    counts: [u32; 17],
    values: [u32; 257],
}

#[derive(Clone, Copy, Default)]
struct ScanComp {
    comp: u32,
    ac: u32,
    dc: u32,
}

#[derive(Default)]
struct Scan {
    ncomp: u32,
    ss: u32,
    se: u32,
    al: u32,
    ah: u32,
    comps: [ScanComp; 4],
    resets: Vec<u32>,
    zruns: Vec<(u32, u32)>, // block index, number of extra zero runs
}

/// JPEGData
#[derive(Default)]
struct Jd {
    width: u32,
    height: u32,
    restart: u32,
    app: Vec<Vec<u8>>,
    app_type: Vec<u32>,
    com: Vec<Vec<u8>>,
    quant: Vec<Quant>,
    comps: Vec<Comp>,
    huff: Vec<Huff>,
    scans: Vec<Scan>,
    order: Vec<u8>,
    inter: Vec<Vec<u8>>,
    tail: Vec<u8>,
    padding: Option<Vec<u8>>, // padding bits (one per byte); None = all ones
}

/// Ok, or ERR_HDR (a damaged jbrd box / codestream)
#[inline]
fn hdr(c: bool) -> Result<(), i32> {
    if c { Ok(()) } else { Err(ERR_HDR) }
}

/// JPEGData::VisitFields (reading): the bundle at the start of the jbrd box; returns the
/// byte offset of the Brotli stream
fn read_fields(box_: &[u8], jd: &mut Jd) -> Result<usize, i32> {
    let mut buf = Vec::new();
    if buf.try_reserve_exact(box_.len() + JXB_PAD).is_err() {
        return Err(ERR_NOMEM);
    }
    buf.extend_from_slice(box_);
    buf.resize(box_.len() + JXB_PAD, 0);
    let mut br = Jb::init(&buf, box_.len() as u64);
    let _is_gray = br.f(1);
    let (mut napp, mut ncom, mut nscan, mut ninter, mut dri) = (0usize, 0usize, 0usize, 0usize, false);
    loop {
        let m = (br.f(6) + 0xC0) as u8;
        if m & 0xF0 == 0xE0 {
            napp += 1;
        }
        match m {
            0xFE => ncom += 1,
            0xDA => nscan += 1,
            0xFF => ninter += 1,
            0xDD => dri = true,
            _ => {}
        }
        jd.order.push(m);
        if jd.order.len() > 16384 || br.check() != 0 {
            return Err(ERR_HDR);
        }
        if m == 0xD9 {
            break;
        }
    }
    let mut app_len = Vec::with_capacity(napp);
    for _ in 0..napp {
        let t = br.u32f(&D_APPTYPE);
        hdr(t <= APP_XMP)?;
        jd.app_type.push(t);
        let n = br.f(16) as usize + 1;
        hdr(n >= 3)?;
        app_len.push(n);
    }
    let mut com_len = Vec::with_capacity(ncom);
    for _ in 0..ncom {
        let n = br.f(16) as usize + 1;
        hdr(n >= 3)?;
        com_len.push(n);
    }
    let nq = br.u32f(&D_ONETO4);
    hdr(nq != 4)?;
    for i in 0..nq {
        let precision = br.f(1);
        let index = br.f(2);
        let is_last = br.f(1) != 0;
        let _ = i;
        jd.quant.push(Quant { values: [0; 64], precision, index, is_last });
    }
    let ctype = br.f(2);
    let ncomp = match ctype {
        0 => 1,
        1 | 2 => 3,
        _ => {
            let n = br.u32f(&D_ONETO4);
            hdr(n == 1 || n == 3)?;
            n
        }
    };
    for _ in 0..ncomp {
        jd.comps.push(Comp::default());
    }
    match ctype {
        3 => {
            for c in jd.comps.iter_mut() {
                c.id = br.f(8);
            }
        }
        0 => jd.comps[0].id = 1,
        2 => {
            for (c, id) in jd.comps.iter_mut().zip([b'R', b'G', b'B']) {
                c.id = id as u32;
            }
        }
        _ => {
            for (k, c) in jd.comps.iter_mut().enumerate() {
                c.id = k as u32 + 1;
            }
        }
    }
    let mut used = 0u32;
    for c in jd.comps.iter_mut() {
        c.quant_idx = br.f(2);
        hdr(c.quant_idx < nq)?;
        used |= 1 << c.quant_idx;
    }
    // an unused table must equal the previous one (all values are still 0 here)
    hdr(used & 1 != 0)?;
    let nh = br.u32f(&D_NHUFF);
    for _ in 0..nh {
        let is_ac = br.f(1);
        let id = br.f(2);
        let is_last = br.f(1) != 0;
        let mut h = Huff { slot: is_ac << 4 | id, is_last, counts: [0; 17], values: [0; 257] };
        let mut nsym = 0usize;
        for i in 0..17 {
            h.counts[i] = br.u32f(&D_COUNT);
            nsym += h.counts[i] as usize;
        }
        if br.check() != 0 {
            return Err(ERR_HDR);
        }
        hdr(nsym >= 1 && nsym <= 257)?;
        let mut slots = [0u64; 5];
        for i in 0..nsym {
            let v = br.u32f(&D_VALUE);
            h.values[i] = v;
            slots[(v >> 6) as usize] |= 1u64 << (v & 63);
        }
        hdr(h.values[nsym - 1] == 256)?;
        hdr(slots[4] == 1)?;
        let nval = 1 + slots[..4].iter().map(|s| s.count_ones() as usize).sum::<usize>();
        hdr(nval == nsym)?;
        if is_ac == 0 {
            hdr((slots[0] >> 12) | slots[1] | slots[2] | slots[3] == 0)?;
        }
        jd.huff.push(h);
    }
    for _ in 0..nscan {
        let mut s = Scan { ncomp: br.u32f(&D_ONETO4), ..Default::default() };
        hdr(s.ncomp < 4)?;
        s.ss = br.f(6);
        s.se = br.f(6);
        s.al = br.f(4);
        s.ah = br.f(4);
        for i in 0..s.ncomp as usize {
            let comp = br.f(2);
            hdr((comp as usize) < jd.comps.len())?;
            let ac = br.f(2);
            let dc = br.f(2);
            s.comps[i] = ScanComp { comp, ac, dc };
        }
        let _last_needed_pass = br.u32f(&D_LASTPASS);
        jd.scans.push(s);
    }
    if dri {
        jd.restart = br.f(16);
    }
    for s in jd.scans.iter_mut() {
        let n = br.u32f(&D_NUMRUNS);
        if br.check() != 0 {
            return Err(ERR_HDR);
        }
        let mut last: u32 = u32::MAX; // -1
        for _ in 0..n {
            let idx = br.u32f(&D_BLOCK).wrapping_add(last.wrapping_add(1));
            hdr(idx < 3 << 26)?;
            s.resets.push(idx);
            last = idx;
            if br.check() != 0 {
                return Err(ERR_HDR);
            }
        }
        let n = br.u32f(&D_NUMRUNS);
        let mut last: u32 = u32::MAX;
        for _ in 0..n {
            let num = br.u32f(&D_ZRUN);
            let idx = br.u32f(&D_BLOCK).wrapping_add(last.wrapping_add(1));
            hdr(idx <= 3 << 26)?;
            s.zruns.push((idx, num));
            last = idx;
            if br.check() != 0 {
                return Err(ERR_HDR);
            }
        }
    }
    let mut inter_len = Vec::with_capacity(ninter);
    for _ in 0..ninter {
        inter_len.push(br.f(16) as usize);
    }
    let tail_len = br.u32f(&D_TAIL) as usize;
    if br.f(1) != 0 {
        let nbit = br.f(24) as u64;
        hdr(br.pos + nbit <= box_.len() as u64 * 8)?;
        let mut bits = Vec::new();
        for _ in 0..nbit {
            bits.push(br.f(1) as u8);
        }
        // libjxl keeps a null pointer for an empty list: then the padding is all ones
        if nbit != 0 {
            jd.padding = Some(bits);
        }
    }
    // the Huffman tables must be defined before the scans use them
    {
        let (mut dht, mut si, mut prog) = (0usize, 0usize, false);
        let (mut ac_ok, mut dc_ok) = ([false; 4], [false; 4]);
        for &m in &jd.order {
            if m == 0xC2 {
                prog = true;
            } else if m == 0xC4 {
                while dht < jd.huff.len() {
                    let h = &jd.huff[dht];
                    dht += 1;
                    if h.slot & 0x10 != 0 {
                        ac_ok[(h.slot & 3) as usize] = true;
                    } else {
                        dc_ok[(h.slot & 3) as usize] = true;
                    }
                    if h.is_last {
                        break;
                    }
                }
            } else if m == 0xDA {
                let s = &jd.scans[si];
                si += 1;
                for c in &s.comps[..s.ncomp as usize] {
                    if (!prog || s.ss == 0) && !dc_ok[c.dc as usize] {
                        return Err(ERR_HDR);
                    }
                    if (!prog || s.ss != 0 || s.se != 0) && !ac_ok[c.ac as usize] {
                        return Err(ERR_HDR);
                    }
                }
            }
        }
    }
    if br.check() != 0 {
        return Err(ERR_HDR);
    }
    br.align();
    if br.check() != 0 {
        return Err(ERR_HDR);
    }
    // the sizes, the contents follow from the Brotli stream (at most 256 MB of marker data)
    let total: usize = app_len.iter().chain(&com_len).chain(&inter_len).sum::<usize>() + tail_len;
    if total > 1 << 28 {
        return Err(ERR_HDR);
    }
    for n in app_len {
        jd.app.push(vec![0u8; n]);
    }
    for n in com_len {
        jd.com.push(vec![0u8; n]);
    }
    for n in inter_len {
        jd.inter.push(vec![0u8; n]);
    }
    jd.tail = vec![0u8; tail_len];
    Ok((br.pos >> 3) as usize)
}

/// DecodeJPEGData: the bundle, then the Brotli compressed marker contents
fn read_jbrd(box_: &[u8]) -> Result<Jd, i32> {
    let mut jd = Jd::default();
    let off = read_fields(box_, &mut jd)?;
    // what the Brotli stream holds, in this order
    let mut need = 0usize;
    for (a, &t) in jd.app.iter().zip(&jd.app_type) {
        if t == APP_UNKNOWN {
            need += a.len();
        }
    }
    need += jd.com.iter().map(|c| c.len()).sum::<usize>();
    need += jd.inter.iter().map(|c| c.len()).sum::<usize>();
    need += jd.tail.len();
    let (data, used) = crate::brotli::decompress(&box_[off..], need).map_err(|_| ERR_HDR)?;
    if data.len() != need || off + used != box_.len() {
        return Err(ERR_HDR);
    }
    let mut p = 0usize;
    let mut take = |dst: &mut Vec<u8>| {
        let n = dst.len();
        dst.copy_from_slice(&data[p..p + n]);
        p += n;
    };
    let mut nicc = 0u8;
    for (a, &t) in jd.app.iter_mut().zip(&jd.app_type) {
        if t != APP_UNKNOWN {
            let n = a.len() - 1;
            a[1] = (n >> 8) as u8;
            a[2] = n as u8;
            if t == APP_ICC {
                hdr(a.len() >= 17)?;
                a[0] = 0xE2;
                a[3..15].copy_from_slice(ICC_TAG);
                nicc = nicc.wrapping_add(1);
                a[15] = nicc;
            }
        } else {
            take(a);
            hdr(a[1] as usize * 256 + a[2] as usize + 1 == a.len())?;
        }
    }
    for (a, &t) in jd.app.iter_mut().zip(&jd.app_type) {
        if t == APP_ICC {
            a[16] = nicc;
        } else if t == APP_EXIF {
            a[0] = 0xE1;
            hdr(a.len() >= 3 + EXIF_TAG.len())?;
            a[3..9].copy_from_slice(EXIF_TAG);
        } else if t == APP_XMP {
            a[0] = 0xE1;
            hdr(a.len() >= 3 + XMP_TAG.len())?;
            a[3..32].copy_from_slice(XMP_TAG);
        }
    }
    for c in jd.com.iter_mut() {
        take(c);
        hdr(c[1] as usize * 256 + c[2] as usize + 1 == c.len())?;
    }
    for c in jd.inter.iter_mut() {
        take(c);
    }
    take(&mut jd.tail);
    Ok(jd)
}

// ------------------------------------------------------------------ coefficient capture

/// the JPEG side of the frame decoder (jv.jcap): checked tables and the components
pub struct Cap {
    gray: bool,
    ct: u32,
    qtable: [i32; 192],
    scaled: [i32; 192],
    dcoff: [i32; 3],
    cmap: [usize; 3], // JXL channel -> JPEG component (colour transform order)
    hsamp: [u32; 3],
    vsamp: [u32; 3],
    wb: [u32; 3],
    hb: [u32; 3],
    coeffs: [Vec<i16>; 3],
}

fn zvec16(n: usize) -> Result<Vec<i16>, i32> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return Err(ERR_NOMEM);
    }
    v.resize(n, 0);
    Ok(v)
}

/// the per frame checks and tables of DecodeGroupImpl / InitFrameOutput
fn setup(jf: &Jf, jv: &Jv) -> Result<Box<Cap>, i32> {
    let gray = jf.dbg & JDBG_GRAY != 0;
    // IsJPEGCompatible
    if !(jv.basex == 0.0 && jv.baseb == 0.0 && jv.ytoxdc == 0 && jv.ytobdc == 0 && jv.cf == 84) {
        return Err(ERR_UNSUPPORTED);
    }
    if jf.ct == JCT_XYB {
        return Err(ERR_UNSUPPORTED);
    }
    let q = jv.qe.first().ok_or(ERR_UNSUPPORTED)?;
    let dd = f32::from_bits((q.den - 1.0f32 / 2040.0f32).to_bits() & 0x7FFF_FFFF);
    if q.mode != 7 || dd > 1e-8f32 || q.raw.len() < 192 {
        return Err(ERR_UNSUPPORTED);
    }
    let mut cap = Box::new(Cap {
        gray,
        ct: jf.ct,
        qtable: [0; 192],
        scaled: [0; 192],
        dcoff: [0; 3],
        cmap: [0; 3],
        hsamp: [1; 3],
        vsamp: [1; 3],
        wb: [0; 3],
        hb: [0; 3],
        coeffs: [Vec::new(), Vec::new(), Vec::new()],
    });
    cap.qtable.copy_from_slice(&q.raw[..192]);
    let qt = &cap.qtable;
    for c in 0..3 {
        if jf.ct == JCT_NONE {
            cap.dcoff[c] = 1024 / qt[64 * c];
        }
        for i in 0..64 {
            let n = qt[64 + i];
            let d = qt[64 * c + i];
            if n <= 0 || d <= 0 || n >= 65536 || d >= 65536 {
                return Err(ERR_UNSUPPORTED);
            }
            cap.scaled[64 * c + (i % 8) * 8 + i / 8] = (1 << 11) * n / d;
        }
    }
    // components: sizes in the YCbCr order (InitFrameOutput), filled in the frame's order
    let ncomp = if gray { 1 } else { 3 };
    let ycc: [usize; 3] = if gray { [0; 3] } else { [1, 0, 2] };
    for c in 0..ncomp {
        let k = ycc[c];
        cap.wb[k] = jf.xb >> (jf.hshift(c) & 31);
        cap.hb[k] = jf.yb >> (jf.vshift(c) & 31);
        cap.hsamp[k] = 1 << CSHX[jf.chroma[c] as usize & 3];
        cap.vsamp[k] = 1 << CSHY[jf.chroma[c] as usize & 3];
        cap.coeffs[k] = zvec16(cap.wb[k] as usize * cap.hb[k] as usize * 64)?;
    }
    cap.cmap = if gray {
        [0; 3]
    } else if jf.ct == JCT_YCBCR {
        [1, 0, 2]
    } else {
        [0, 1, 2]
    };
    Ok(cap)
}

/// capture: the decoder hook in place of jv_recon (see the module header)
pub fn capture(jf: &mut Jf, g: u32) -> i32 {
    let Some(mut jv) = jf.vd.take() else { return ERR_HDR };
    let r = match capture_group(jf, &mut jv, g) {
        Ok(()) => 0,
        Err(e) => e,
    };
    jf.vd = Some(jv);
    r
}

#[inline]
fn sat16(v: i32) -> i16 {
    v.clamp(-32768, 32767) as i16
}

fn capture_group(jf: &Jf, jv: &mut Jv, g: u32) -> Result<(), i32> {
    if jv.jcap.is_none() {
        jv.jcap = Some(setup(jf, jv)?);
    }
    let bufk = jv_coefbuf(jv, g)?;
    let mut coef = core::mem::take(&mut jv.coef[bufk]);
    let r = capture_into(jf, jv, g, &mut coef);
    jv.coef[bufk] = coef;
    r
}

fn capture_into(jf: &Jf, jv: &mut Jv, g: u32, coef: &mut [i32]) -> Result<(), i32> {
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
    let is444 = (jf.maxhs | jf.maxvs) == 0;
    let xb = jf.xb;
    let Some(mut cap) = jv.jcap.take() else { return Err(ERR_HDR) };
    let mut off = 0usize;
    let mut res = Ok(());
    'blocks: for by in 0..h {
        for bx in 0..w {
            let bi = (y0 + by).wrapping_mul(xb).wrapping_add(x0).wrapping_add(bx) as usize;
            let a8 = jv.acs[bi];
            if a8 & 1 == 0 {
                continue;
            }
            if a8 >> 1 != 0 {
                // "Can only decode to JPEG if only DCT-8 is used."
                res = Err(ERR_UNSUPPORTED);
                break 'blocks;
            }
            if off + 64 > JV_COEFSZ {
                res = Err(ERR_HDR);
                break 'blocks;
            }
            let ti = (((y0 + by) >> 3).wrapping_mul(jv.cw).wrapping_add((x0 + bx) >> 3)) as usize;
            let fx = jv.ytox[ti];
            let fb = jv.ytob[ti];
            let cfl = is444 && (fx != 0 || fb != 0);
            let mut ty = [0i32; 64];
            for c in [1usize, 0, 2] {
                if cap.gray && c != 1 {
                    continue;
                }
                if (bx >> hs[c]) << hs[c] != bx || (by >> vs[c]) << vs[c] != by {
                    continue;
                }
                let blk = &mut coef[c * JV_COEFSZ + off..c * JV_COEFSZ + off + 64];
                for i in 0..8 {
                    for j in i + 1..8 {
                        blk.swap(i * 8 + j, j * 8 + i);
                    }
                }
                let mut out = [0i16; 64];
                if !cfl {
                    for i in 0..64 {
                        out[i] = sat16(blk[i]);
                    }
                } else if c == 1 {
                    for i in 0..64 {
                        ty[i] = blk[i];
                        out[i] = sat16(blk[i]);
                    }
                } else {
                    let f = if c == 0 { fx } else { fb };
                    let scale = f * (1 << 11) / 84; // RatioJPEG
                    for i in 0..64 {
                        let qt = cap.scaled[c * 64 + i];
                        let cs = qt.wrapping_mul(scale).wrapping_add(1024) >> 11;
                        let cf = ty[i].wrapping_mul(cs).wrapping_add(1024) >> 11;
                        out[i] = sat16(blk[i].wrapping_add(cf));
                    }
                }
                let sbx = (x0 >> hs[c]) + (bx >> hs[c]);
                let sby = (y0 >> vs[c]) + (by >> vs[c]);
                let dc = jv.dc[c][sby.wrapping_mul(xb).wrapping_add(sbx) as usize];
                let v = dc - cap.dcoff[c] as f32;
                out[0] = v.clamp(-2047.0, 2047.0) as i16;
                if out.iter().any(|&v| !(-4095..=4095).contains(&v)) {
                    // "JPEG DCT coefficients out of range"
                    res = Err(ERR_UNSUPPORTED);
                    break 'blocks;
                }
                let k = cap.cmap[c];
                let p = (cap.wb[k] as usize * sby as usize + sbx as usize) * 64;
                if sbx >= cap.wb[k] || p + 64 > cap.coeffs[k].len() {
                    res = Err(ERR_HDR);
                    break 'blocks;
                }
                cap.coeffs[k][p..p + 64].copy_from_slice(&out);
            }
            off += 64;
        }
    }
    // the coefficient buffer is zero again for the next group
    for c in 0..3 {
        coef[c * JV_COEFSZ..c * JV_COEFSZ + off.min(JV_COEFSZ)].fill(0);
    }
    jv.jcap = Some(cap);
    res
}

/// the first frame decoded up to its JPEG coefficients
fn frame(d: &mut crate::Dec, gray: bool) -> Result<(u32, u32, Box<Cap>), i32> {
    let jx = &mut d.jx;
    let cs = &d.cs;
    let mut br = Jb::init(cs, jx.cslen);
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    if jx.haveprev != 0 {
        let mut jf = Jf::default();
        let r = jf_header(jx, &mut br, &mut jf, true);
        if r != 0 {
            return Err(r);
        }
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            return Err(r);
        }
        br.pos = jf.end << 3;
    }
    let mut jf = Jf::default();
    let r = jf_header(jx, &mut br, &mut jf, false);
    if r != 0 {
        return Err(r);
    }
    crate::noise::jx_count(jx, &mut jf);
    let r = jf_toc(&mut br, &mut jf);
    if r != 0 {
        return Err(r);
    }
    // "Cannot output JPEG from Modular", "Cannot decode to JPEG an XYB image"
    if jf.modular != 0 || jx.xyb != 0 || jf.ct == JCT_XYB || jf.flags & JFF_USEDC != 0 {
        return Err(ERR_UNSUPPORTED);
    }
    jf.dbg = JDBG_JPEG | 1 | if gray { JDBG_GRAY } else { 0 };
    let r = crate::dec::jx_frame(jx, cs, &mut jf);
    if r != 0 {
        return Err(r);
    }
    let cap = jf.vd.as_mut().and_then(|jv| jv.jcap.take()).ok_or(ERR_UNSUPPORTED)?;
    Ok((jf.w, jf.h, cap))
}

/// has_jbrd: whether data is a container with a JPEG reconstruction box
pub fn has_jbrd(data: &[u8]) -> bool {
    crate::open(data).is_ok_and(|d| d.jx.jbrd.is_some())
}

/// reconstruct: the JPEG file of a JPEG transcode (see the module header)
pub fn reconstruct(data: &[u8]) -> Result<Vec<u8>, i32> {
    let mut d = crate::open(data)?;
    let Some((o, n)) = d.jx.jbrd else { return Err(ERR_UNSUPPORTED) };
    let mut jd = read_jbrd(&data[o..o + n])?;
    // ICC profile into the ICC markers (SetJPEGDataFromICC)
    let icc = &d.jx.icc;
    let mut ip = 0usize;
    for (a, &t) in jd.app.iter_mut().zip(&jd.app_type) {
        if t != APP_ICC {
            continue;
        }
        let len = a.len() - 17;
        hdr(ip + len <= icc.len())?;
        a[17..].copy_from_slice(&icc[ip..ip + len]);
        ip += len;
    }
    hdr(ip == icc.len() || ip == 0)?;
    // EXIF and XMP from the first Exif / xml box (the box sizes must fit the markers)
    for (t, tag) in [(APP_EXIF, 9usize), (APP_XMP, 32)] {
        let k: Vec<usize> = (0..jd.app.len()).filter(|&i| jd.app_type[i] == t).collect();
        if k.is_empty() {
            continue;
        }
        hdr(k.len() == 1)?;
        let a = &mut jd.app[k[0]];
        let bx = if t == APP_EXIF { d.jx.exifbox } else { d.jx.xmlbox };
        let content = crate::box_contents(data, bx.ok_or(ERR_HDR)?)?;
        // the Exif box starts with the offset of the TIFF header (not in the marker)
        let src = if t == APP_EXIF { content.get(4..).ok_or(ERR_HDR)? } else { &content[..] };
        hdr(src.len() == a.len() - tag)?;
        a[tag..].copy_from_slice(src);
    }
    // the coefficients and the quantization tables
    let gray = jd.comps.len() == 1;
    let (w, h, mut cap) = frame(&mut d, gray)?;
    jd.width = w;
    jd.height = h;
    for k in 0..jd.comps.len() {
        let c = &mut jd.comps[k];
        c.wb = cap.wb[k];
        c.hb = cap.hb[k];
        c.h = cap.hsamp[k];
        c.v = cap.vsamp[k];
        c.coeffs = core::mem::take(&mut cap.coeffs[k]);
    }
    let mut set = 0u32;
    for c in 0..jd.comps.len() {
        let qc = if gray { 1 } else { c };
        let qpos = jd.comps[cap.cmap[c]].quant_idx as usize;
        set |= 1 << qpos;
        for x in 0..8 {
            for y in 0..8 {
                jd.quant[qpos].values[x * 8 + y] = cap.qtable[qc * 64 + y * 8 + x];
            }
        }
    }
    for i in 0..jd.quant.len() {
        if set & (1 << i) != 0 {
            continue;
        }
        hdr(i != 0)?;
        jd.quant[i].values = jd.quant[i - 1].values;
    }
    let _ = cap.ct;
    write_jpeg(&jd)
}

// ------------------------------------------------------------------ JPEG writer

/// the bit writer of the entropy coded segments (JpegBitWriter: 64-bit buffer, 0xFF
/// stuffing)
struct Bw<'a> {
    out: &'a mut Vec<u8>,
    buf: u64,
    free: i32, // put_bits
    healthy: bool,
}

impl Bw<'_> {
    #[inline]
    fn emit(&mut self, b: u8) {
        self.out.push(b);
        if b == 0xFF {
            self.out.push(0);
        }
    }

    fn bits(&mut self, nbits: i32, bits: u64) {
        self.free -= nbits;
        if self.free < 0 {
            if nbits > 64 {
                self.free += nbits;
                self.healthy = false;
            } else {
                self.buf |= bits.checked_shr((-self.free) as u32).unwrap_or(0);
                for k in (0..8).rev() {
                    self.emit((self.buf >> (8 * k)) as u8);
                }
                self.free += 64;
                self.buf = bits.checked_shl(self.free as u32).unwrap_or(0);
            }
        } else {
            self.buf |= bits.checked_shl(self.free as u32).unwrap_or(0);
        }
    }

    /// JumpToByteBoundary with the stored padding bits (None: ones)
    fn byte_boundary(&mut self, pad: &mut Option<(&[u8], usize)>) -> bool {
        let mut n = (self.free & 7) as u32;
        let mut dangling = 0u8;
        let pattern: u8 = match pad {
            None => ((1u32 << n) - 1) as u8,
            Some((bits, pos)) => {
                let mut p = 0u8;
                while n > 0 {
                    n -= 1;
                    p <<= 1;
                    if *pos >= bits.len() {
                        return false;
                    }
                    let b = bits[*pos];
                    *pos += 1;
                    dangling |= b;
                    p |= b;
                }
                p
            }
        };
        if dangling & !1 != 0 {
            return false;
        }
        while self.free <= 56 {
            self.emit((self.buf >> 56) as u8);
            self.buf <<= 8;
            self.free += 8;
        }
        if self.free < 64 {
            let mask = 0xFFu32 >> (64 - self.free);
            let c = ((self.buf >> 56) as u32 & !mask) as u8 | pattern;
            self.emit(c);
        }
        self.buf = 0;
        self.free = 64;
        true
    }
}

/// HuffmanCodeTable
#[derive(Clone)]
struct Tab {
    depth: [u8; 256],
    code: [u32; 256],
    init: bool,
}

impl Default for Tab {
    fn default() -> Self {
        Tab { depth: [127; 256], code: [0; 256], init: false }
    }
}

impl Tab {
    #[inline]
    fn sym(&self, bw: &mut Bw, s: usize) {
        bw.bits(self.depth[s & 255] as i32, self.code[s & 255] as u64);
    }
    #[inline]
    fn sym_bits(&self, bw: &mut Bw, s: usize, nbits: i32, bits: u64) {
        bw.bits(nbits + self.depth[s & 255] as i32, bits | ((self.code[s & 255] as u64) << nbits));
    }
}

/// BuildHuffmanCodeTable
fn build_table(h: &Huff, t: &mut Tab) -> bool {
    let mut size = [0u32; 258];
    let mut code = [0u32; 257];
    let mut p = 0usize;
    for l in 1..=16usize {
        let n = h.counts[l] as usize;
        if p + n > 257 {
            return false;
        }
        for _ in 0..n {
            size[p] = l as u32;
            p += 1;
        }
    }
    if p == 0 {
        return true;
    }
    let last = p - 1;
    size[last] = 0;
    let mut c = 0u32;
    let mut si = size[0];
    p = 0;
    while size[p] != 0 {
        while size[p] == si {
            code[p] = c;
            p += 1;
            c = c.wrapping_add(1);
        }
        c <<= 1;
        si += 1;
        if si > 64 {
            break; // (cannot happen: the sizes are 1 ..= 16 and sorted)
        }
    }
    for p in 0..last {
        let v = h.values[p] as usize & 255;
        t.depth[v] = size[p] as u8;
        t.code[v] = code[p];
    }
    true
}

/// DCTCodingState: the EOB run and the buffered refinement bits of a progressive scan
#[derive(Default)]
struct Cs {
    eob_run: u32,
    huff: usize, // the AC table of the EOB run
    words: Vec<u16>,
    count: usize,
}

impl Cs {
    fn flush(&mut self, bw: &mut Bw, ac: &[Tab; 4]) {
        if self.eob_run > 0 {
            let nbits = 31 - self.eob_run.leading_zeros();
            ac[self.huff].sym(bw, (nbits << 4) as usize);
            if nbits > 0 {
                bw.bits(nbits as i32, (self.eob_run & ((1 << nbits) - 1)) as u64);
            }
            self.eob_run = 0;
        }
        let nw = self.count >> 4;
        for i in 0..nw {
            bw.bits(16, self.words[i] as u64);
        }
        let tail = self.count & 15;
        if tail != 0 {
            bw.bits(tail as i32, *self.words.last().unwrap_or(&0) as u64);
        }
        self.words.clear();
        self.count = 0;
    }

    /// BufferEndOfBand
    fn end_of_band(&mut self, huff: usize, new_bits: &[i32], bw: &mut Bw, ac: &[Tab; 4]) {
        if self.eob_run == 0 {
            self.huff = huff;
        }
        self.eob_run += 1;
        let mut cnt = new_bits.len();
        if cnt != 0 {
            let mut nb = 0u64;
            for &b in new_bits {
                nb = (nb << 1) | b as u64;
            }
            let tail = self.count & 15;
            if tail != 0 {
                let stuff = (16 - tail).min(cnt);
                let sb = ((nb >> (cnt - stuff)) as u16) & ((1u32 << stuff) - 1) as u16;
                if let Some(l) = self.words.last_mut() {
                    *l = ((*l as u32) << stuff) as u16 | sb;
                }
                cnt -= stuff;
                self.count += stuff;
            }
            while cnt >= 16 {
                self.words.push((nb >> (cnt - 16)) as u16);
                cnt -= 16;
                self.count += 16;
            }
            if cnt != 0 {
                self.words.push((nb & ((1u64 << cnt) - 1)) as u16);
                self.count += cnt;
            }
        }
        if self.eob_run == 0x7FFF {
            self.flush(bw, ac);
        }
    }
}

#[inline]
fn log2p1(v: u32) -> u32 {
    32 - v.leading_zeros()
}

/// EncodeDCTBlockSequential
fn block_seq(co: &[i16], dc: &Tab, ac: &Tab, zruns: u32, last_dc: &mut i16, bw: &mut Bw) -> bool {
    let mut temp2 = co[0];
    let mut temp = temp2.wrapping_sub(*last_dc);
    *last_dc = temp2;
    temp2 = temp >> 15;
    temp = temp.wrapping_add(temp2);
    temp2 ^= temp;
    let nbits = if temp2 == 0 { 0 } else { log2p1(temp2 as i32 as u32) };
    dc.sym(bw, nbits as usize);
    if nbits != 0 {
        bw.bits(nbits as i32, (temp as i32 as u32 & ((1u64 << nbits) - 1) as u32) as u64);
    }
    let mut r: i16 = 0;
    let mut litmus: i16 = 0;
    for i in 1..64 {
        let mut temp = co[NATURAL[i] as usize];
        if temp == 0 {
            r += 1;
        } else {
            let mut temp2 = temp >> 15;
            temp = temp.wrapping_add(temp2);
            temp2 ^= temp;
            if r > 15 {
                ac.sym(bw, 0xF0);
                r -= 16;
                if r > 15 {
                    ac.sym(bw, 0xF0);
                    r -= 16;
                }
                if r > 15 {
                    ac.sym(bw, 0xF0);
                    r -= 16;
                }
            }
            litmus |= temp2;
            let nb = log2p1(temp2 as u16 as u32);
            let s = ((r as i32) << 4) + nb as i32;
            ac.sym_bits(bw, s as usize, nb as i32, (temp as i32 & ((1 << nb) - 1)) as u64);
            r = 0;
        }
    }
    for _ in 0..zruns {
        ac.sym(bw, 0xF0);
        r = r.wrapping_sub(16);
    }
    if r > 0 {
        ac.sym(bw, 0);
    }
    litmus >= 0
}

/// EncodeDCTBlockProgressive (first scan of a band)
#[allow(clippy::too_many_arguments)]
fn block_prog(co: &[i16], dc: &Tab, ach: usize, ss: i32, se: i32, al: u32, zruns: u32, st: &mut Cs, last_dc: &mut i16, bw: &mut Bw, tabs: &[Tab; 4]) -> bool {
    let ac = &tabs[ach];
    let eob_ok = ss > 0;
    let mut ss = ss;
    if ss == 0 {
        let mut temp2 = ((co[0] as i32) >> al) as i16;
        let mut temp = temp2.wrapping_sub(*last_dc);
        *last_dc = temp2;
        temp2 = temp;
        if temp < 0 {
            temp = temp.wrapping_neg();
            if temp < 0 {
                return false;
            }
            temp2 = temp2.wrapping_sub(1);
        }
        let nbits = if temp == 0 { 0 } else { log2p1(temp as u32) };
        dc.sym(bw, nbits as usize);
        if nbits != 0 {
            bw.bits(nbits as i32, (temp2 as i32 & ((1 << nbits) - 1)) as u64);
        }
        ss += 1;
    }
    if ss > se {
        return true;
    }
    let mut r = 0i32;
    for k in ss..=se {
        let mut temp = co[NATURAL[(k & 63) as usize] as usize];
        if temp == 0 {
            r += 1;
            continue;
        }
        let temp2;
        if temp < 0 {
            temp = temp.wrapping_neg();
            if temp < 0 {
                return false;
            }
            temp = ((temp as i32) >> al) as i16;
            temp2 = !temp;
        } else {
            temp = ((temp as i32) >> al) as i16;
            temp2 = temp;
        }
        if temp == 0 {
            r += 1;
            continue;
        }
        st.flush(bw, tabs);
        while r > 15 {
            ac.sym(bw, 0xF0);
            r -= 16;
        }
        let nbits = log2p1(temp as u32);
        ac.sym(bw, ((r << 4) + nbits as i32) as usize);
        bw.bits(nbits as i32, (temp2 as i32 & ((1 << nbits) - 1)) as u64);
        r = 0;
    }
    if zruns > 0 {
        st.flush(bw, tabs);
        for _ in 0..zruns {
            ac.sym(bw, 0xF0);
            r -= 16;
        }
    }
    if r > 0 {
        st.end_of_band(ach, &[], bw, tabs);
        if !eob_ok {
            st.flush(bw, tabs);
        }
    }
    true
}

/// EncodeRefinementBits
fn block_refine(co: &[i16], ach: usize, ss: i32, se: i32, al: u32, st: &mut Cs, bw: &mut Bw, tabs: &[Tab; 4]) -> bool {
    let ac = &tabs[ach];
    let eob_ok = ss > 0;
    let mut ss = ss;
    if ss == 0 {
        bw.bits(1, (((co[0] as i32) >> al) & 1) as u64);
        ss += 1;
    }
    if ss > se {
        return true;
    }
    let mut absv = [0i32; 64];
    let mut eob = 0i32;
    for k in ss..=se {
        let a = (co[NATURAL[(k & 63) as usize] as usize] as i32).abs();
        absv[(k & 63) as usize] = a >> al;
        if absv[(k & 63) as usize] == 1 {
            eob = k;
        }
    }
    let mut r = 0i32;
    let mut rb = [0i32; 64];
    let mut nrb = 0usize;
    for k in ss..=se {
        let a = absv[(k & 63) as usize];
        if a == 0 {
            r += 1;
            continue;
        }
        while r > 15 && k <= eob {
            st.flush(bw, tabs);
            ac.sym(bw, 0xF0);
            r -= 16;
            for &b in &rb[..nrb] {
                bw.bits(1, b as u64);
            }
            nrb = 0;
        }
        if a > 1 {
            rb[nrb] = a & 1;
            nrb += 1;
            continue;
        }
        st.flush(bw, tabs);
        let nz = if co[NATURAL[(k & 63) as usize] as usize] < 0 { 0 } else { 1 };
        ac.sym(bw, ((r << 4) + 1) as usize);
        bw.bits(1, nz);
        for &b in &rb[..nrb] {
            bw.bits(1, b as u64);
        }
        nrb = 0;
        r = 0;
    }
    if r > 0 || nrb != 0 {
        st.end_of_band(ach, &rb[..nrb], bw, tabs);
        if !eob_ok {
            st.flush(bw, tabs);
        }
    }
    true
}

/// the serializer state across the markers
struct Ser<'a> {
    out: Vec<u8>,
    dc: [Tab; 4],
    ac: [Tab; 4],
    prog: bool,
    dri: bool,
    dht: usize,
    dqt: usize,
    app: usize,
    com: usize,
    inter: usize,
    scan: usize,
    pad: Option<(&'a [u8], usize)>,
}

/// one scan (DoEncodeScan)
fn scan(jd: &Jd, s: &mut Ser) -> bool {
    let Some(si) = jd.scans.get(s.scan) else { return false };
    // SOS
    let n = si.ncomp as usize;
    let len = 6 + 2 * n;
    s.out.extend_from_slice(&[0xFF, 0xDA, (len >> 8) as u8, len as u8, n as u8]);
    for c in &si.comps[..n] {
        let Some(comp) = jd.comps.get(c.comp as usize) else { return false };
        s.out.push(comp.id as u8);
        s.out.push(((c.dc << 4) + c.ac) as u8);
    }
    s.out.extend_from_slice(&[si.ss as u8, si.se as u8, ((si.ah << 4) | si.al) as u8]);
    let restart = if s.dri { jd.restart } else { 0 };
    let (al, ah, ss, se) = if s.prog { (si.al, si.ah, si.ss as i32, si.se as i32) } else { (0, 0, 0, 63) };
    let mode = if !s.prog || (ah == 0 && al == 0 && ss == 0 && se == 63) {
        0
    } else if ah == 0 {
        1
    } else {
        2
    };
    // CalculateMcuSize
    let inter = n > 1;
    let base = &jd.comps[si.comps[0].comp as usize];
    let (hg, vg) = if inter { (1, 1) } else { (base.h, base.v) };
    let mh = jd.comps.iter().map(|c| c.h).max().unwrap_or(1).max(1);
    let mv = jd.comps.iter().map(|c| c.v).max().unwrap_or(1).max(1);
    let mcus_x = (jd.width as u64 * hg as u64).div_ceil(8 * mh as u64) as usize;
    let mcus_y = (jd.height as u64 * vg as u64).div_ceil(8 * mv as u64) as usize;
    let want_ac = ss != 0 || se != 0;
    let want_dc = ss == 0;
    let mut st = Cs::default();
    let mut last_dc = [0i16; 4];
    let mut togo = restart;
    let mut next_rst = 0u8;
    let mut bsi = 0u32; // block scan index
    let mut zpos = 0usize;
    let mut rpos = 0usize;
    let next_reset = |rpos: &mut usize| -> i64 {
        if *rpos < si.resets.len() {
            *rpos += 1;
            si.resets[*rpos - 1] as i64
        } else {
            -1
        }
    };
    let mut nreset = next_reset(&mut rpos);
    let mut nzero: i64 = si.zruns.first().map_or(-1, |z| z.0 as i64);
    let dc_t = s.dc.clone();
    let ac_t = s.ac.clone();
    let mut pad = s.pad.take();
    let mut bw = Bw { out: &mut s.out, buf: 0, free: 64, healthy: true };
    let mut ok = true;
    'mcus: for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart > 0 && togo == 0 {
                st.flush(&mut bw, &ac_t);
                if !bw.byte_boundary(&mut pad) {
                    ok = false;
                    break 'mcus;
                }
                bw.out.push(0xFF);
                bw.out.push(0xD0 + next_rst);
                next_rst = (next_rst + 1) & 7;
                togo = restart;
                last_dc = [0; 4];
            }
            for c in &si.comps[..n] {
                let comp = &jd.comps[c.comp as usize];
                let (dct, act) = (c.dc as usize & 3, c.ac as usize & 3);
                if (want_dc && !dc_t[dct].init) || (want_ac && !ac_t[act].init) {
                    ok = false;
                    break 'mcus;
                }
                let (nby, nbx) = if inter { (comp.v as usize, comp.h as usize) } else { (1, 1) };
                for iy in 0..nby {
                    for ix in 0..nbx {
                        let byy = my * nby + iy;
                        let bxx = mx * nbx + ix;
                        let idx = byy * comp.wb as usize + bxx;
                        if bsi as i64 == nreset {
                            st.flush(&mut bw, &ac_t);
                            nreset = next_reset(&mut rpos);
                        }
                        let mut zr = 0u32;
                        if bsi as i64 == nzero {
                            zr = si.zruns[zpos].1;
                            zpos += 1;
                            nzero = si.zruns.get(zpos).map_or(-1, |z| z.0 as i64);
                        }
                        if bxx >= comp.wb as usize || (idx + 1) * 64 > comp.coeffs.len() {
                            ok = false;
                            break 'mcus;
                        }
                        let co = &comp.coeffs[idx * 64..idx * 64 + 64];
                        let ld = &mut last_dc[c.comp as usize & 3];
                        let r = match mode {
                            0 => block_seq(co, &dc_t[dct], &ac_t[act], zr, ld, &mut bw),
                            1 => block_prog(co, &dc_t[dct], act, ss, se, al, zr, &mut st, ld, &mut bw, &ac_t),
                            _ => block_refine(co, act, ss, se, al, &mut st, &mut bw, &ac_t),
                        };
                        if !r {
                            ok = false;
                            break 'mcus;
                        }
                        bsi += 1;
                    }
                }
            }
            togo = togo.wrapping_sub(1);
        }
    }
    if ok {
        st.flush(&mut bw, &ac_t);
        ok = bw.byte_boundary(&mut pad);
    }
    ok &= bw.healthy;
    s.pad = pad;
    s.scan += 1;
    ok
}

/// WriteJpeg
fn write_jpeg(jd: &Jd) -> Result<Vec<u8>, i32> {
    if jd.order.is_empty() {
        return Err(ERR_HDR);
    }
    let mut s = Ser {
        out: vec![0xFF, 0xD8],
        dc: Default::default(),
        ac: Default::default(),
        prog: false,
        dri: false,
        dht: 0,
        dqt: 0,
        app: 0,
        com: 0,
        inter: 0,
        scan: 0,
        pad: jd.padding.as_deref().map(|p| (p, 0)),
    };
    let fail = Err(ERR_HDR);
    for &m in &jd.order {
        match m {
            0xC0 | 0xC1 | 0xC2 | 0xC9 | 0xCA => {
                if m <= 0xC2 {
                    s.prog = m == 0xC2;
                }
                let n = jd.comps.len();
                let len = 8 + 3 * n;
                s.out.extend_from_slice(&[0xFF, m, (len >> 8) as u8, len as u8, 8]);
                s.out.extend_from_slice(&[(jd.height >> 8) as u8, jd.height as u8, (jd.width >> 8) as u8, jd.width as u8, n as u8]);
                for c in &jd.comps {
                    s.out.push(c.id as u8);
                    s.out.push(((c.h << 4) | c.v) as u8);
                    let Some(q) = jd.quant.get(c.quant_idx as usize) else { return fail };
                    s.out.push(q.index as u8);
                }
            }
            0xC4 => {
                let mut len = 2usize;
                for h in &jd.huff[s.dht.min(jd.huff.len())..] {
                    len += 16 + h.counts.iter().map(|&c| c as usize).sum::<usize>();
                    if h.is_last {
                        break;
                    }
                }
                let mut data = vec![0u8; len + 2];
                data[..4].copy_from_slice(&[0xFF, 0xC4, (len >> 8) as u8, len as u8]);
                let mut p = 4usize;
                loop {
                    let Some(h) = jd.huff.get(s.dht) else { return fail };
                    s.dht += 1;
                    let i = (h.slot & 3) as usize;
                    let t = if h.slot & 0x10 != 0 { &mut s.ac[i] } else { &mut s.dc[i] };
                    *t = Tab::default();
                    if !build_table(h, t) {
                        return fail;
                    }
                    t.init = true;
                    let mut total = 0usize;
                    let mut maxl = 0usize;
                    for (i, &c) in h.counts.iter().enumerate() {
                        if c != 0 {
                            maxl = i;
                        }
                        total += c as usize;
                    }
                    total = total.wrapping_sub(1);
                    let mut put = |b: u8| -> bool {
                        if p >= data.len() {
                            return false;
                        }
                        data[p] = b;
                        p += 1;
                        true
                    };
                    if !put(h.slot as u8) {
                        return fail;
                    }
                    for i in 1..=16 {
                        let v = if i == maxl { h.counts[i].wrapping_sub(1) } else { h.counts[i] };
                        if !put(v as u8) {
                            return fail;
                        }
                    }
                    for i in 0..total {
                        if !put(h.values.get(i).copied().unwrap_or(0) as u8) {
                            return fail;
                        }
                    }
                    if h.is_last {
                        break;
                    }
                }
                s.out.extend_from_slice(&data);
            }
            0xD0..=0xD7 => s.out.extend_from_slice(&[0xFF, m]),
            0xD9 => {
                s.out.extend_from_slice(&[0xFF, 0xD9]);
                s.out.extend_from_slice(&jd.tail);
            }
            0xDA => {
                if !scan(jd, &mut s) {
                    return fail;
                }
            }
            0xDB => {
                let mut len = 2usize;
                for q in &jd.quant[s.dqt.min(jd.quant.len())..] {
                    len += 1 + if q.precision != 0 { 2 } else { 1 } * 64;
                    if q.is_last {
                        break;
                    }
                }
                let start = s.out.len();
                s.out.extend_from_slice(&[0xFF, 0xDB, (len >> 8) as u8, len as u8]);
                loop {
                    let Some(q) = jd.quant.get(s.dqt) else { return fail };
                    s.dqt += 1;
                    s.out.push(((q.precision << 4) + q.index) as u8);
                    for i in 0..64 {
                        let v = q.values[NATURAL[i] as usize];
                        if q.precision != 0 {
                            s.out.push((v >> 8) as u8);
                        }
                        s.out.push(v as u8);
                    }
                    if q.is_last {
                        break;
                    }
                }
                // the marker buffer is len + 2 bytes (zero filled when the tables are shorter)
                s.out.resize(start + len + 2, 0);
            }
            0xDD => {
                s.dri = true;
                s.out.extend_from_slice(&[0xFF, 0xDD, 0, 4, (jd.restart >> 8) as u8, jd.restart as u8]);
            }
            0xE0..=0xEF => {
                let Some(a) = jd.app.get(s.app) else { return fail };
                s.app += 1;
                s.out.push(0xFF);
                s.out.extend_from_slice(a);
            }
            0xFE => {
                let Some(a) = jd.com.get(s.com) else { return fail };
                s.com += 1;
                s.out.push(0xFF);
                s.out.extend_from_slice(a);
            }
            0xFF => {
                let Some(a) = jd.inter.get(s.inter) else { return fail };
                s.inter += 1;
                s.out.extend_from_slice(a);
            }
            _ => return fail,
        }
    }
    if let Some((p, pos)) = s.pad
        && pos != p.len()
    {
        return fail;
    }
    Ok(s.out)
}
