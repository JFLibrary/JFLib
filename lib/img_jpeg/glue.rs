// glue -- JPEG behind the codec interface: metadata scan, IMGDEC / IMGOPT glue
//                                                           [src/img/imgcore.s, JPEG parts]
//
//   meta(data, m, flags) -> 0 / ERR_TRUNC / ERR_HDR       jpeg_meta: the marker segments up to
//                                                          the first SOS
//   probe(data, m) -> recognised                           jpeg_probe
//   decode_ex(data, out, dec, m) -> 0 / ERR_*              jpeg_decode_ex
//   encode_opt(sf, opt, out) -> 0 / ERR_*                  jpeg_encode_opt
//   decode_fl_codec / save_codec                           Codec::decode_fl / save (the
//                                                          classic img_decode / img_save)
//
// meta: SOFn (size, precision, components, progressive / lossless), JFIF density, EXIF
// (orientation exactly as dec.rs reads it, dpi when JFIF has none, ColorSpace; IMGD_EXIF),
// XMP (IMGD_XMP), Adobe APP14 (colour model), ICC (IMGD_ICC).  decode_ex: frame > 0 ->
// ERR_UNSUPPORTED; the header size is checked first (ERR_LIMIT); the decoder applies the
// EXIF orientation itself (IMF_ORIENT_APPLIED) unless IMGD_NO_ORIENT.  encode_opt:
// IO_QUALITY (0 = 95), IO_CHROMA 444 / 422 / 420 / 400 (else the quality rule), the JFIF
// density from the metadata's dpi (IMR_CM: dots per cm) and after APP0 the segments APP1
// EXIF (IOF_EMBED_EXIF, orientation 1, skipped when larger than one segment), APP2 ICC
// (IOF_EMBED_ICC), APP1 XMP (IOF_EMBED_XMP, skipped when larger than one segment).

use alloc::vec::Vec;

use gfx::Surface;
use img_core::exif::{exif_copy_orient1, exif_read};
use img_core::icc::{icc_jpeg_collect, icc_jpeg_segments};
use img_core::io::{core_decode, core_probe};
use img_core::meta::{jpg_seg_next, meta_blob, Blob, JSeg};
use img_core::{
    ImgDec, ImgMeta, ImgOpt, ERR_CANCEL, ERR_NOMEM, ERR_TRUNC, IMF_INTERLACED, IMF_LOSSLESS, IMF_ORIENT_APPLIED,
    IMF_SRGB, IMGD_EXIF, IMGD_ICC, IMGD_NO_ORIENT, IMGD_PROBE, IMGD_XMP, IMM_CMYK, IMM_GRAY, IMM_RGB, IMM_YCC,
    IMM_YCCK, IMR_CM, IMR_INCH, IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP,
};

use crate::{dec, enc};

const JPEG_DEF_QUALITY: u32 = 95; // paint.net's default

/// the XMP namespace header of an APP1 XMP segment (29 bytes incl. the NUL)
pub const XMP_NS: &[u8; 29] = b"http://ns.adobe.com/xap/1.0/\0";

/// jpg_orient_compat: the orientation of an APP1 payload exactly as the decoder determines
/// it: 0 = not an EXIF APP1, else 1..8 (1 when it has no valid entry)
fn orient_compat(p: &[u8]) -> u32 {
    if p.len() < 14 || &p[..6] != b"Exif\0\0" {
        return 0;
    }
    let t = &p[6..]; // TIFF header
    let n = t.len() as u32;
    let be = match (t[0], t[1]) {
        (0x49, 0x49) => false,
        (0x4D, 0x4D) => true,
        _ => return 1,
    };
    let rd16 = |o: u32| {
        let (a, b) = (t[o as usize], t[o as usize + 1]);
        if be { (a as u32) << 8 | b as u32 } else { (b as u32) << 8 | a as u32 }
    };
    let mut ifd = u32::from_le_bytes([t[4], t[5], t[6], t[7]]);
    if be {
        ifd = ifd.swap_bytes();
    }
    if ifd > n - 2 {
        return 1;
    }
    let mut cnt = rd16(ifd);
    let mut e = ifd + 2;
    while cnt != 0 {
        if e + 12 > n {
            return 1;
        }
        if rd16(e) == 0x0112 {
            if rd16(e + 2) != 3 {
                return 1;
            }
            let v = rd16(e + 8);
            return if (1..=8).contains(&v) { v } else { 1 };
        }
        e += 12;
        cnt -= 1;
    }
    1
}

/// jpg_density: 16.16 dpi from a JFIF density: units 1 = dots per inch, 2 = per cm
fn density(v: u32, units: u32) -> u32 {
    let v = (v as u64) << 16;
    if units == 2 { ((v * 254 + 50) / 100) as u32 } else { v as u32 }
}

/// jpeg_meta: scan the marker segments up to the first SOS (see the file header)
pub fn meta(data: &[u8], m: &mut ImgMeta, flags: u32) -> i32 {
    let mut sof = false;
    let mut adobe: i32 = -1; // transform, -1 none
    let mut jfif = false; // JFIF density seen
    let mut exif: Option<&[u8]> = None;
    let mut xmp: Option<&[u8]> = None;
    let mut ids = 0u32; // RGB component ids
    if data.len() < 3 {
        return ERR_TRUNC;
    }
    if data[0] != 0xFF || data[1] != 0xD8 || data[2] != 0xFF {
        return img_core::ERR_HDR;
    }
    let mut js = JSeg::new();
    loop {
        let mk = jpg_seg_next(data, &mut js);
        if mk == 0 || mk == 0xDA || mk == 0xD9 {
            break;
        }
        let p = &data[js.ptr..js.ptr + js.len];
        match mk {
            0xE0 => {
                if p.len() < 12 || &p[..5] != b"JFIF\0" || jfif {
                    continue;
                }
                let units = p[7] as u32;
                if units != 1 && units != 2 {
                    continue;
                }
                let xd = (p[8] as u32) << 8 | p[9] as u32;
                let yd = (p[10] as u32) << 8 | p[11] as u32;
                if xd == 0 || yd == 0 {
                    continue;
                }
                jfif = true;
                m.dpix = density(xd, units);
                m.dpiy = density(yd, units);
                m.resunit = if units == 2 { IMR_CM } else { IMR_INCH };
            }
            0xE1 => {
                if exif.is_none() && p.len() <= 0xFFFF {
                    let o = orient_compat(p);
                    if o != 0 {
                        exif = Some(&p[6..]);
                        m.orient = o as u8;
                        continue;
                    }
                }
                if xmp.is_none() && p.len() >= 29 && &p[..29] == XMP_NS {
                    xmp = Some(&p[29..]);
                }
            }
            0xEE => {
                if p.len() >= 12 && &p[..5] == b"Adobe" {
                    adobe = p[11] as i32;
                }
            }
            0xC0..=0xCF if mk != 0xC4 && mk != 0xC8 && mk != 0xCC => {
                // SOFn
                if sof || p.len() < 6 {
                    continue;
                }
                sof = true;
                m.bits = p[0];
                m.h = (p[1] as u32) << 8 | p[2] as u32;
                m.w = (p[3] as u32) << 8 | p[4] as u32;
                m.channels = p[5];
                match mk & 3 {
                    2 => m.flags |= IMF_INTERLACED, // C2 C6 CA CE progressive
                    3 => m.flags |= IMF_LOSSLESS,   // C3 C7 CB CF lossless
                    _ => {}
                }
                if p[5] == 3 && p.len() >= 15 {
                    ids = p[6] as u32 | (p[9] as u32) << 8 | (p[12] as u32) << 16;
                }
            }
            _ => {}
        }
    }
    if !sof {
        return ERR_TRUNC;
    }
    m.frames = 1;
    // colour model
    match m.channels {
        1 => m.model = IMM_GRAY,
        3 => m.model = if adobe == 0 || ids == 0x424752 { IMM_RGB } else { IMM_YCC }, // ids 'R' 'G' 'B'
        4 => m.model = if adobe == 2 { IMM_YCCK } else { IMM_CMYK },
        _ => {}
    }
    // EXIF: resolution when JFIF has none, colour space, blob
    if let Some(ex) = exif {
        let (_, info) = exif_read(ex); // (errors leave the defaults / what was read)
        if info.cspace == 1 {
            m.flags |= IMF_SRGB;
        }
        if !jfif && info.dpix != 0 {
            m.dpix = info.dpix;
            m.dpiy = info.dpiy;
            m.resunit = if info.unit == 3 { IMR_CM } else { IMR_INCH };
        }
        if flags & IMGD_EXIF != 0 {
            meta_blob(m, Blob::Exif, ex);
        }
    }
    if flags & IMGD_XMP != 0 {
        meta_blob(m, Blob::Xmp, xmp.unwrap_or(&[]));
    }
    if flags & IMGD_ICC != 0 {
        let (r, icc) = icc_jpeg_collect(data);
        if r == 1 {
            m.icc = icc;
        }
    }
    0
}

/// jpeg_probe: header + metadata scan, no pixels
pub fn probe(data: &[u8], m: &mut ImgMeta) -> bool {
    core_probe(data, m, meta)
}

/// jpeg_decode_fl as Codec::decode_fl: data -> sf (overwritten on success) -> 0 / ERR_*
pub fn decode_fl_codec(data: &[u8], sf: &mut Surface, jdf: u32) -> i32 {
    match dec::decode_fl(data, jdf) {
        Ok(s) => {
            *sf = s;
            0
        }
        Err(e) => e,
    }
}

/// jpeg_encode as Codec::save (img_save)
pub fn save_codec(sf: &Surface, quality: u32) -> Option<Vec<u8>> {
    enc::encode(sf, quality as i32)
}

/// jpeg_decode_ex: JPEG through decode_fl, which applies the EXIF orientation itself unless
/// IMGD_NO_ORIENT: IMF_ORIENT_APPLIED is set when it did
pub fn decode_ex(data: &[u8], out: &mut Surface, d: &ImgDec, m: &mut ImgMeta) -> i32 {
    let r = core_decode(data, out, d, m, meta, decode_fl_codec);
    if r == 0 && d.flags & (IMGD_PROBE | IMGD_NO_ORIENT) == 0 && (2..=8).contains(&m.orient) {
        m.flags |= IMF_ORIENT_APPLIED;
    }
    r
}

/// jq_density: JFIF density 1..65535 from a 16.16 dpi value: unit 1 dots per inch, 2 dots
/// per cm (rounded)
fn jq_density(dpi: u32, unit: u32) -> u32 {
    let v = if unit == 2 {
        (dpi as u64 * 100 + 127 * 65536) / (254 * 65536)
    } else {
        (dpi as u64 + 32768) >> 16
    };
    v.clamp(1, 65535) as u32
}

/// jpeg_encode_opt: encode_ex with the IMGOPT options and metadata segments (see the file
/// header)
pub fn encode_opt(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    if opt.cancelled() {
        return ERR_CANCEL;
    }
    let mut q = opt.quality;
    if q == 0 {
        q = JPEG_DEF_QUALITY;
    }
    q = q.min(100);
    let mut cfg = enc::EncCfg { chroma: opt.chroma, ..Default::default() };
    if !matches!(cfg.chroma, 444 | 422 | 420 | 400) {
        cfg.chroma = 0;
    }
    // density from the metadata
    if let Some(m) = opt.meta {
        if m.dpix != 0 && m.dpiy != 0 {
            cfg.unit = if m.resunit == IMR_CM { 2 } else { 1 };
            cfg.xd = jq_density(m.dpix, cfg.unit);
            cfg.yd = jq_density(m.dpiy, cfg.unit);
        }
    }
    let Some(jpg) = enc::encode_ex(sf, q as i32, Some(&cfg)) else { return ERR_NOMEM };
    let Some(m) = opt.meta else {
        *out = jpg;
        return 0;
    };
    // metadata segments
    let fl = opt.flags;
    let mut exif = Vec::new();
    if fl & IOF_EMBED_EXIF != 0 && !m.exif.is_empty() {
        exif = exif_copy_orient1(&m.exif);
        if exif.len() > 65533 - 6 {
            exif = Vec::new(); // too big for one APP1: not written
        }
    }
    let mut icc = Vec::new();
    if fl & IOF_EMBED_ICC != 0 && !m.icc.is_empty() {
        match icc_jpeg_segments(&m.icc) {
            Ok(s) => icc = s,
            Err(ERR_NOMEM) => return ERR_NOMEM,
            Err(_) => {} // ERR_LIMIT (> 255 segments): not written
        }
    }
    let xmp: &[u8] = if fl & IOF_EMBED_XMP != 0 && m.xmp.len() <= 65533 - 29 { &m.xmp } else { &[] };
    if exif.is_empty() && icc.is_empty() && xmp.is_empty() {
        *out = jpg;
        return 0;
    }
    // new file: SOI + APP0 (20) | APP1 EXIF | APP2 ICC | APP1 XMP | rest
    let mut o = Vec::new();
    let extra = if exif.is_empty() { 0 } else { exif.len() + 10 } + icc.len() + if xmp.is_empty() { 0 } else { xmp.len() + 33 };
    if o.try_reserve_exact(jpg.len() + extra).is_err() {
        return ERR_NOMEM;
    }
    o.extend_from_slice(&jpg[..20]);
    if !exif.is_empty() {
        let l = exif.len() + 2 + 6;
        o.extend_from_slice(&[0xFF, 0xE1, (l >> 8) as u8, l as u8]);
        o.extend_from_slice(b"Exif\0\0");
        o.extend_from_slice(&exif);
    }
    o.extend_from_slice(&icc);
    if !xmp.is_empty() {
        let l = xmp.len() + 2 + 29;
        o.extend_from_slice(&[0xFF, 0xE1, (l >> 8) as u8, l as u8]);
        o.extend_from_slice(XMP_NS);
        o.extend_from_slice(xmp);
    }
    o.extend_from_slice(&jpg[20..]);
    *out = o;
    0
}
