// img_jxr -- JPEG XR (ITU-T T.832 / ISO 29199-2, HD Photo) decoder and encoder: tag-based
// container (Annex A), codestream decode driver, metadata              [src/img/jxr.s]
//
//   probe(data, meta) -> recognised                    jxr_probe (container + IMAGE_HEADER
//                                                      fields into meta)
//   decode(data, out, dec, meta) -> 0 / ERR_*          jxr_decode
//   enc::encode(sf, opt, out) -> 0 / ERR_*             jxr_encode (jxr_enc.s)
//   dbg_planes(data, mode) -> (0 / ERR_*, blob)        jxr_dbg_planes -- test hook: the decoded
//       planes of the primary codestream as {u32 w, h, ml, mt, npl; per plane u32 cf, nch,
//       scaled; per component u32 width, height, i32 samples...}; mode 0 = reconstructed
//       samples, 1 = before the inverse transform (HP coefficients in the planes), 2 = the DC
//       grids after decoding, 3 = {u32 w, h; i32 [h][w][5]} output samples of the pixel format
//       (color.rs, before 8-bit)
//   CODEC                                              jxr_codec
//
// Modules: ent (jxr_ent.s: headers, entropy decoding, prediction, dequantisation), pct
// (jxr_pct.s: inverse transform, overlap filters), color (jxr_color.s: output formatting), enc
// (jxr_enc.s: the writer), tab (generated tables, tools/gen_jxr.py).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, FMT_PARGB, Surface};
use img_core::meta::{Blob, dims_check, meta_blob, surf_alloc};
use img_core::*;

mod color;
pub mod enc;
mod ent;
mod pct;
mod tab;

use ent::{Cs, JXF_PREMUL, JXW_QP, JXW_START, JXW_TRUNC};

/// the codec descriptor  [jxr_codec]
pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode),
    encode: Some(enc::encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_QUALITY | IOM_LOSSLESS | IOM_CHROMA | IOM_BITDEPTH | IOM_ICC | IOM_XMP,
    ..Codec::EMPTY
};

/// parsed container (first IFD)  [JR]
#[derive(Default)]
struct Jr {
    fmt: u32, // last byte of the pixel format GUID (0 = none / foreign)
    xf: u32,  // SPATIAL_XFRM_PRIMARY
    w: u32,
    h: u32,
    ioff: u64, // IMAGE_OFFSET / IMAGE_BYTE_COUNT
    ilen: u64,
    aoff: u64, // ALPHA_OFFSET / ALPHA_BYTE_COUNT (0 = none)
    alen: u64,
    icc: u64,
    iccl: u64,
    xmp: u64,
    xmpl: u64,
    exif: u64, // EXIF IFD offset (0 = none)
    dpix: u32, // resolution (IEEE float bits, 0 = none)
    dpiy: u32,
}

const MAXIFD: u32 = 4096; // entries accepted in the first IFD

// {6FDDC324-4E03-4BFE-B185-3D77768DC9xx}
static GUID: [u8; 15] = [0x24, 0xC3, 0xDD, 0x6F, 0x03, 0x4E, 0xFE, 0x4B, 0xB1, 0x85, 0x3D, 0x77, 0x76, 0x8D, 0xC9];
static BDBITS: [u8; 16] = [1, 8, 16, 16, 16, 0, 32, 32, 5, 10, 6, 0, 0, 0, 0, 1]; // bits per OUTPUT_BITDEPTH
static ORIENT: [u8; 8] = [1, 4, 2, 3, 6, 7, 5, 8]; // EXIF orientation per SPATIAL_XFRM value
static CFMODEL: [(u8, u8); 16] = [
    (IMM_GRAY, 1),
    (IMM_YCC, 3),
    (IMM_YCC, 3),
    (IMM_YCC, 3),
    (IMM_CMYK, 4),
    (IMM_CMYK, 4),
    (0, 0),
    (IMM_RGB, 3),
    (IMM_RGB, 3),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
    (0, 0),
]; // (IMM_*, channels) per OUTPUT_CLR_FMT

fn u16_at(d: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([d[o], d[o + 1]]) as u32
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// CHECK_SPAN: [off, off + len) inside 0..end
fn span_ok(off: u64, len: u64, end: u64) -> bool {
    end >= off && end - off >= len
}

/// "II" 0xBC version header and the first IFD; offsets / lengths checked against the data
/// [jxr_parse]
fn parse(d: &[u8]) -> Result<Jr, i32> {
    let mut jr = Jr::default();
    let len = d.len() as u64;
    if len < 8 {
        return Err(ERR_TRUNC);
    }
    if d[0] != 0x49 || d[1] != 0x49 || d[2] != 0xBC || d[3] > 1 {
        return Err(ERR_HDR);
    }
    let ifd = u32_at(d, 4) as u64;
    if ifd > len - 2 {
        return Err(ERR_TRUNC);
    }
    let n = u16_at(d, ifd as usize);
    if n > MAXIFD {
        return Err(ERR_HDR);
    }
    if ifd + n as u64 * 12 + 2 > len {
        return Err(ERR_TRUNC);
    }
    for i in 0..n as usize {
        let e = ifd as usize + 2 + i * 12;
        let tag = u16_at(d, e);
        let typ = u16_at(d, e + 2);
        let cnt = u32_at(d, e + 4);
        let val = u32_at(d, e + 8);
        let v = if typ == 3 { val & 0xFFFF } else { val }; // SHORT: value in the low half
        match tag {
            0xBC01 => {
                let o = val as u64;
                if cnt == 16 && o + 16 <= len && d[o as usize..o as usize + 15] == GUID {
                    jr.fmt = d[o as usize + 15] as u32;
                }
            }
            0xBC80 => jr.w = v,
            0xBC81 => jr.h = v,
            0xBCC0 => jr.ioff = v as u64,
            0xBCC1 => jr.ilen = v as u64,
            0xBCC2 => jr.aoff = v as u64,
            0xBCC3 => jr.alen = v as u64,
            0xBC02 => jr.xf = v,
            0xBC82 => jr.dpix = val,
            0xBC83 => jr.dpiy = val,
            0x8773 => {
                jr.icc = val as u64;
                jr.iccl = if cnt > 4 { cnt as u64 } else { 0 }; // inline data are not a profile
            }
            0x02BC => {
                jr.xmp = val as u64;
                jr.xmpl = if cnt > 4 { cnt as u64 } else { 0 };
            }
            0x8769 => jr.exif = v as u64,
            _ => {}
        }
    }
    // image codestream inside the data (required), optional blobs dropped when outside
    if jr.ilen == 0 {
        return Err(ERR_HDR);
    }
    if !span_ok(jr.ioff, jr.ilen, len) {
        return Err(ERR_TRUNC);
    }
    if !span_ok(jr.aoff, jr.alen, len) {
        jr.alen = 0;
    }
    if !span_ok(jr.icc, jr.iccl, len) {
        jr.iccl = 0;
    }
    if !span_ok(jr.xmp, jr.xmpl, len) {
        jr.xmpl = 0;
    }
    Ok(jr)
}

/// IEEE float bits -> 16.16 dpi (0 for <= 0, NaN, > 65535)  [f2dpi]
fn f2dpi(bits: u32) -> u32 {
    let x = f32::from_bits(bits);
    if !(x > 0.0) || x > 65535.0 {
        return 0;
    }
    (x * 65536.0) as i64 as u32
}

/// IM_FMT, dimensions, model / bits / channels / flags from the image header of the
/// codestream, resolution  [jxr_meta]
fn set_meta(m: &mut ImgMeta, d: &[u8], jr: &Jr) {
    m.fmt = IMGF_JXR;
    m.w = jr.w;
    m.h = jr.h;
    m.orient = ORIENT[if jr.xf > 7 { 0 } else { jr.xf as usize }];
    m.frames = 1;
    // image header: flag bits in byte 10, OUTPUT_CLR_FMT / OUTPUT_BITDEPTH in byte 11
    if jr.ilen >= 16 {
        let h = &d[jr.ioff as usize..];
        let (b10, b11) = (h[10] as u32, h[11] as u32);
        let mut fl = 0;
        if b10 & 1 != 0 || jr.alen != 0 {
            fl |= IMF_ALPHA;
            if b10 & 2 != 0 || matches!(jr.fmt, 0x10 | 0x17 | 0x1A) {
                fl |= IMF_SRC_PREMUL; // premultiplied pixel formats
            }
        }
        let ocf = (b11 >> 4) as usize;
        let obd = (b11 & 15) as usize;
        let bits = BDBITS[obd];
        m.bits = bits;
        if obd == 4 || obd == 7 {
            fl |= IMF_FLOAT;
        }
        if bits > 8 {
            fl |= IMF_HDR;
        }
        m.model = CFMODEL[ocf].0;
        m.channels = CFMODEL[ocf].1 + (fl & IMF_ALPHA != 0) as u8;
        m.flags = fl;
    }
    m.dpix = f2dpi(jr.dpix);
    m.dpiy = f2dpi(jr.dpiy);
    if m.dpix | m.dpiy != 0 {
        m.resunit = IMR_INCH;
    }
}

/// ICC / XMP / EXIF copies requested by the flags; EXIF: the container is TIFF-structured, so
/// the data with an "II*\0" header is a TIFF whose first IFD carries the EXIF IFD pointer
/// [jxr_blobs]
fn blobs(m: &mut ImgMeta, flags: u32, d: &[u8], jr: &Jr) {
    if flags & IMGD_ICC != 0 && jr.iccl != 0 {
        meta_blob(m, Blob::Icc, &d[jr.icc as usize..(jr.icc + jr.iccl) as usize]);
    }
    if flags & IMGD_XMP != 0 && jr.xmpl != 0 {
        meta_blob(m, Blob::Xmp, &d[jr.xmp as usize..(jr.xmp + jr.xmpl) as usize]);
    }
    if flags & IMGD_EXIF != 0 && jr.exif != 0 {
        meta_blob(m, Blob::Exif, d);
        if m.exif.len() >= 4 {
            m.exif[2] = 0x2A;
            m.exif[3] = 0x00;
        }
    }
}

/// the codestream data[off, off + len)
fn cs_slice(d: &[u8], off: u64, len: u64) -> &[u8] {
    &d[off as usize..(off + len) as usize]
}

/// decode and reconstruct one codestream  [cs_decode]
fn cs_decode(cs: &[u8], dec: Option<&ImgDec>) -> Result<Cs, i32> {
    let mut jx = ent::decode(cs, dec)?;
    pct::recon(&mut jx);
    Ok(jx)
}

/// container + IMAGE_HEADER fields into meta  [jxr_probe]
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    match parse(data) {
        Ok(jr) => {
            set_meta(meta, data, &jr);
            true
        }
        Err(_) => false,
    }
}

/// IMF_LOSSLESS when no quantisation, no discarded band, no damaged packet, no chroma
/// subsampling of RGB output and no 32-bit output  [jxr_lossless]
fn lossless(m: &mut ImgMeta, jx: &Cs, ja: Option<&Cs>) {
    let ok = |cs: &Cs, alpha: bool| {
        if cs.warn & (JXW_TRUNC | JXW_START | JXW_QP) != 0 || cs.obd == 6 || cs.obd == 7 || cs.pl[0].bands != 0 {
            return false;
        }
        if cs.npl >= 2 && cs.pl[1].bands != 0 {
            return false;
        }
        // (alpha codestream: subsampling irrelevant)
        alpha || !(cs.pl[0].cf.wrapping_sub(1) <= 1 && cs.ocf == 7)
    };
    if !ok(jx, false) {
        return;
    }
    if let Some(a) = ja
        && !ok(a, true)
    {
        return;
    }
    m.flags |= IMF_LOSSLESS;
}

/// decode a JPEG XR file  [jxr_decode]
pub fn decode(data: &[u8], out: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    let jr = match parse(data) {
        Ok(j) => j,
        Err(e) => return e,
    };
    set_meta(m, data, &jr);
    blobs(m, dec.flags, data, &jr);
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    let r = dims_check(jr.w, jr.h, dec);
    if r != 0 {
        return r;
    }
    let jx = match cs_decode(cs_slice(data, jr.ioff, jr.ilen), Some(dec)) {
        Ok(j) => j,
        Err(e) => return e,
    };
    // planar alpha (a separate Y_ONLY codestream) when the image has no interleaved alpha
    let mut ja = None;
    if jx.npl != 2 && jr.alen != 0 {
        let a = match cs_decode(cs_slice(data, jr.aoff, jr.alen), Some(dec)) {
            Ok(a) => a,
            Err(e) => return e,
        };
        if a.w == jx.w && a.h == jx.h && a.pl[0].cf == 0 {
            ja = Some(a); // else: unusable alpha, ignored
        }
    }
    lossless(m, &jx, ja.as_ref());
    // premultiplied source (header flag or a P... pixel format GUID): PARGB unless straight
    // output is wanted (then unpremultiplied like WIC); other sources: ARGB
    let mut fl = 0;
    if jx.flags & JXF_PREMUL != 0 || matches!(jr.fmt, 0x10 | 0x17 | 0x1A) {
        fl = 2;
    }
    fl |= dec.flags & IMGD_STRAIGHT;
    let sfmt = if fl == 2 { FMT_PARGB } else { FMT_ARGB };
    let mut sf = match surf_alloc(jx.w, jx.h, sfmt, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let stride = sf.stride as usize;
    let r = color::output(&jx, ja.as_ref(), &mut sf.px, stride, fl);
    if r == 0 {
        *out = sf;
    }
    r
}

fn put32(b: &mut [u8], o: &mut usize, v: u32) {
    b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
    *o += 4;
}

/// see the module header  [jxr_dbg_planes]
pub fn dbg_planes(data: &[u8], mode: u32) -> (i32, Vec<u8>) {
    let mode = mode & 255;
    let jr = match parse(data) {
        Ok(j) => j,
        Err(e) => return (e, Vec::new()),
    };
    let mut jx = match ent::decode(cs_slice(data, jr.ioff, jr.ilen), None) {
        Ok(j) => j,
        Err(e) => return (e, Vec::new()),
    };
    if mode != 1 && mode != 2 {
        pct::recon(&mut jx);
        if mode == 3 {
            // {u32 w, h; i32 [h][w][5] native samples (channels 0..3, alpha)} (interleaved alpha only)
            let (w, h) = (jx.w as usize, jx.h as usize);
            let mut b = vec![0u8; w * h * 20 + 8];
            b[0..4].copy_from_slice(&jx.w.to_le_bytes());
            b[4..8].copy_from_slice(&jx.h.to_le_bytes());
            let r = color::output(&jx, None, &mut b[8..], w * 20, 4);
            return (r, b);
        }
    }
    let grid = mode == 2;
    // blob size: 20 + planes * 12 + components * 8 + samples * 4
    let mut n = 20;
    for pl in jx.pl.iter().take(jx.npl as usize) {
        n += 12;
        for c in 0..pl.nch as usize {
            let (w, h) = if grid { (pl.gw[c], pl.gh[c]) } else { (pl.pw[c], pl.ph[c]) };
            n += 8 + w as usize * h as usize * 4;
        }
    }
    let mut b = vec![0u8; n];
    let mut o = 0;
    for v in [jx.w, jx.h, jx.ml, jx.mt, jx.npl] {
        put32(&mut b, &mut o, v);
    }
    for pl in jx.pl.iter().take(jx.npl as usize) {
        for v in [pl.cf, pl.nch, pl.scaled] {
            put32(&mut b, &mut o, v);
        }
        for c in 0..pl.nch as usize {
            let (w, h, s) = if grid { (pl.gw[c], pl.gh[c], &pl.dcg[c]) } else { (pl.pw[c], pl.ph[c], &pl.buf[c]) };
            put32(&mut b, &mut o, w);
            put32(&mut b, &mut o, h);
            let k = w as usize * h as usize;
            for (d, v) in b[o..o + k * 4].chunks_exact_mut(4).zip(&s[..k]) {
                d.copy_from_slice(&v.to_le_bytes());
            }
            o += k * 4;
        }
    }
    (0, b)
}
