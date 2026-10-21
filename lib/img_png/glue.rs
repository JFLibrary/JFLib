// glue -- PNG behind the codec interface: metadata scanner, probe, decode_ex, encode_opt
//                                                         [src/img/imgcore.s, PNG parts]
//
//   scan(data, meta, flags) -> 0 / ERR_TRUNC / ERR_HDR     png_meta: IHDR (size, depth, colour
//        type, interlace), tRNS, pHYs (-> dpi), gAMA, sRGB, cHRM (meta.chrm), cICP, iCCP
//        (IMGD_ICC, inflated, at most 16 MB), eXIf (orientation; IMGD_EXIF), iTXt
//        "XML:com.adobe.xmp" (IMGD_XMP), acTL / first fcTL; stops at IEND, a truncated chunk
//        or a length > 2^31 - 1
//   probe(data, meta) -> bool / decode_ex(data, sf, dec, meta) -> 0 / ERR_*
//   encode_opt(sf, opt, out) -> 0 / ERR_*   effort (deflate level, 0 = 6), bitdepth (0 / 32 /
//        24 / 8), dither / athresh (8 bit), and from opt.meta: pHYs (dpi), iCCP
//        (IOF_EMBED_ICC), eXIf (IOF_EMBED_EXIF, orientation reset to 1), iTXt XMP
//        (IOF_EMBED_XMP)

use alloc::vec;
use alloc::vec::Vec;

use crate::dec::{CHANS, decode_fl};
use crate::enc::{PngCfg, encode_cfg, put_chunk};
use img_core::deflate::zlib_deflate;
use img_core::exif::{exif_copy_orient1, exif_orientation};
use img_core::inflate::zlib_inflate;
use img_core::io::{core_decode, core_probe, dpi_ppm, ppm_dpi};
use img_core::meta::{Blob, meta_blob};
use img_core::quantize::opt_qcfg;
use img_core::*;

const PNG_DEF_LEVEL: u32 = 6;
const ZMETA_MAX: usize = 1 << 24; // largest inflated iCCP / iTXt payload (16 MB)

// PNG colour type -> IMM_* (types 0..6)
static CT_MODEL: [u8; 7] = [IMM_GRAY, 0, IMM_RGB, IMM_PAL, IMM_GRAY, 0, IMM_RGB];

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn blob_mut(m: &mut ImgMeta, b: Blob) -> &mut Vec<u8> {
    match b {
        Blob::Icc => &mut m.icc,
        Blob::Exif => &mut m.exif,
        Blob::Xmp => &mut m.xmp,
    }
}

// zmeta: inflate the zlib stream src into a new blob unless one is there already (`set`:
// an earlier zmeta stored one, possibly empty -- the assembly's blob pointer); the buffer
// grows until the data fits (at most ZMETA_MAX bytes) -> 0 / ERR_*
fn zmeta(m: &mut ImgMeta, which: Blob, src: &[u8], set: &mut bool) -> i32 {
    if *set || !blob_mut(m, which).is_empty() {
        return 0;
    }
    let mut cap = src.len() * 4 + 4096;
    loop {
        cap = cap.min(ZMETA_MAX);
        let mut buf = vec![0u8; cap];
        match zlib_inflate(src, &mut buf) {
            Ok(n) => {
                buf.truncate(n);
                *blob_mut(m, which) = buf;
                *set = true;
                return 0;
            }
            Err(ERR_OVERFLOW) => {
                if cap >= ZMETA_MAX {
                    return ERR_LIMIT;
                }
                cap <<= 1;
            }
            Err(e) => return e,
        }
    }
}

/// scan the chunks into meta (see the module header)  [png_meta]
pub fn scan(data: &[u8], m: &mut ImgMeta, flags: u32) -> i32 {
    let len = data.len();
    if len < 8 {
        return ERR_TRUNC;
    }
    if data[..8] != img_core::io::PNG_SIG {
        return ERR_HDR;
    }
    m.flags |= IMF_LOSSLESS;
    let mut p = 8usize;
    let mut ihdr = false;
    let mut fctl = false;
    let (mut zicc, mut zxmp) = (false, false); // a (possibly empty) inflated blob is there
    loop {
        if len - p < 12 {
            break;
        }
        let n = be32(data, p);
        if n > 0x7FFF_FFFF || n as usize > len - p - 12 {
            break; // truncated chunk
        }
        let n = n as usize;
        let typ: [u8; 4] = data[p + 4..p + 8].try_into().unwrap();
        let d = &data[p + 8..p + 8 + n];
        if &typ == b"IHDR" {
            if !ihdr {
                if n < 13 {
                    return ERR_HDR;
                }
                ihdr = true;
                m.w = be32(d, 0);
                m.h = be32(d, 4);
                let depth = d[8];
                m.bits = depth;
                if depth == 16 {
                    m.flags |= IMF_HDR;
                }
                let ct = d[9] as usize;
                if ct <= 6 {
                    m.model = CT_MODEL[ct];
                    m.channels = CHANS[ct] as u8;
                    if ct & 4 != 0 {
                        m.flags |= IMF_ALPHA;
                    }
                }
                if d[12] != 0 {
                    m.flags |= IMF_INTERLACED;
                }
            }
        } else if !ihdr {
            return ERR_HDR; // IHDR must come first
        } else {
            match &typ {
                b"IEND" => break,
                b"tRNS" => m.flags |= IMF_ALPHA,
                b"pHYs" => {
                    if n >= 9 && d[8] == 1 {
                        m.dpix = ppm_dpi(be32(d, 0));
                        m.dpiy = ppm_dpi(be32(d, 4));
                        m.resunit = IMR_CM;
                    }
                }
                b"gAMA" => {
                    if n >= 4 {
                        m.gamma = be32(d, 0);
                    }
                }
                b"sRGB" => m.flags |= IMF_SRGB,
                b"cHRM" => {
                    // 8 x u32 (value * 100000) -> u16 (value * 50000, rounded, clamped)
                    if n >= 32 {
                        for k in 0..8 {
                            let v = be32(d, k * 4);
                            m.chrm[k] = ((v >> 1) + (v & 1)).min(0xFFFF) as u16;
                        }
                    }
                }
                b"cICP" => {
                    if n >= 4 {
                        m.cicp = u32::from_le_bytes([d[0], d[1], d[2], d[3]]);
                    }
                }
                b"iCCP" => {
                    // name (1..79 bytes) NUL, method 0, zlib data
                    if flags & IMGD_ICC != 0
                        && let Some(z) = d.iter().take(80).position(|&b| b == 0)
                        && z + 2 <= n
                        && d[z + 1] == 0
                    {
                        zmeta(m, Blob::Icc, &d[z + 2..], &mut zicc);
                    }
                }
                b"eXIf" => {
                    if m.orient == 1 {
                        m.orient = exif_orientation(d) as u8; // the first eXIf only
                    }
                    if flags & IMGD_EXIF != 0 && m.exif.is_empty() {
                        meta_blob(m, Blob::Exif, d);
                    }
                }
                b"iTXt" => {
                    if flags & IMGD_XMP != 0 && m.xmp.is_empty() && !zxmp && n >= 22 && &d[..18] == b"XML:com.adobe.xmp\0" {
                        let comp = d[18];
                        // language tag, translated keyword (NUL-terminated)
                        let skipz = |i: usize| d[i..].iter().position(|&b| b == 0).map(|k| i + k + 1);
                        if let Some(i) = skipz(20)
                            && let Some(i) = skipz(i)
                        {
                            if comp == 0 {
                                meta_blob(m, Blob::Xmp, &d[i..]);
                            } else {
                                zmeta(m, Blob::Xmp, &d[i..], &mut zxmp);
                            }
                        }
                    }
                }
                b"acTL" => {
                    if n >= 8 {
                        let f = be32(d, 0);
                        if f != 0 {
                            m.frames = f;
                            if f > 1 {
                                m.flags |= IMF_ANIMATED;
                            }
                            m.loops = be32(d, 4);
                        }
                    }
                }
                b"fcTL" => {
                    if n >= 26 && !fctl {
                        fctl = true;
                        let num = u16::from_be_bytes([d[20], d[21]]) as u32;
                        let mut den = u16::from_be_bytes([d[22], d[23]]) as u32;
                        if den == 0 {
                            den = 100;
                        }
                        m.delay = num * 1000 / den;
                    }
                }
                _ => {}
            }
        }
        p += n + 12;
    }
    if ihdr { 0 } else { ERR_TRUNC }
}

/// recognised (meta filled, no blobs)  [png_probe]
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    core_probe(data, meta, scan)
}

/// IMGDEC-aware decode through png_decode_fl  [png_decode_ex]
pub fn decode_ex(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    core_decode(data, sf, dec, meta, scan, decode_fl)
}

// png_meta_chunks: pHYs / iCCP / eXIf / iTXt chunks from opt.meta
fn meta_chunks(opt: &ImgOpt) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(m) = opt.meta else { return out };
    let fl = opt.flags;
    if m.dpix != 0 && m.dpiy != 0 {
        let mut d = [0u8; 9];
        d[..4].copy_from_slice(&dpi_ppm(m.dpix).to_be_bytes());
        d[4..8].copy_from_slice(&dpi_ppm(m.dpiy).to_be_bytes());
        d[8] = 1;
        put_chunk(&mut out, b"pHYs", &d);
    }
    if fl & IOF_EMBED_ICC != 0 && !m.icc.is_empty() {
        let z = zlib_deflate(&m.icc, 9);
        let mut d = Vec::with_capacity(13 + z.len());
        d.extend_from_slice(b"ICC Profile\0\0"); // keyword, NUL, compression method 0
        d.extend_from_slice(&z);
        put_chunk(&mut out, b"iCCP", &d);
    }
    if fl & IOF_EMBED_EXIF != 0 && !m.exif.is_empty() {
        put_chunk(&mut out, b"eXIf", &exif_copy_orient1(&m.exif));
    }
    if fl & IOF_EMBED_XMP != 0 && !m.xmp.is_empty() {
        // keyword NUL, compression 0, method 0, language "" NUL, translated "" NUL, text
        let mut d = Vec::with_capacity(22 + m.xmp.len());
        d.extend_from_slice(b"XML:com.adobe.xmp\0");
        d.extend_from_slice(&[0, 0, 0, 0]);
        d.extend_from_slice(&m.xmp);
        put_chunk(&mut out, b"iTXt", &d);
    }
    out
}

/// IMGOPT-aware encode through png_encode_cfg  [png_encode_opt]
pub fn encode_opt(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let level = if opt.effort == 0 { PNG_DEF_LEVEL } else { opt.effort.min(9) };
    let depth = match opt.bitdepth {
        8 | 24 | 32 => opt.bitdepth,
        _ => 0, // anything else: auto
    };
    let chunks = meta_chunks(opt);
    let cfg = PngCfg { level, depth, qcfg: opt_qcfg(opt), chunks: &chunks, cancel: opt.cancel };
    match encode_cfg(sf, &cfg) {
        Ok(v) => {
            *out = v;
            0
        }
        Err(e) => e,
    }
}
