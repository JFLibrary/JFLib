// img_jxl -- JPEG XL reader (ISO/IEC 18181) and writer (lossless modular, lossy VarDCT)
//                                                           [src/img/jxl.s, jxl_*.s]
//
//   CODEC                         the codec descriptor (jxl_codec)
//   probe(data, meta) -> bool     jxl_probe
//   decode(data, sf, dec, meta) -> 0 / ERR_*   jxl_decode: displayed frame dec.frame
//                                 (ERR_LIMIT past the last) as FMT_ARGB, 8 bits; meta.delay of
//                                 that frame; ICC, EXIF (Exif box), XMP (xml box) blobs;
//                                 ERR_UNSUPPORTED: float extra channels, PQ / HLG / 709
//                                 output.  jxl-plus: DC frames (progressive DC); the groups of
//                                 a frame decode on the worker pool (same pixels).  jxl-tools:
//                                 splines (spline.rs: libjxl 0.7's rendering, same pixels)
//   open(data) -> Dec / ERR_*     jx_open: codestream copy (bare FF 0A or the box container:
//                                 jxlc / jxlp), image headers
//   jx_frames(jx, br) -> u32      displayed frames (walks the frame headers)
//   jx_meta(jx, br, meta)         image properties into an ImgMeta
//   enc::encode(sf, opt, out) -> 0 / ERR_*   jxl_encode: the jxl-plus writer (crate img_jxl_enc:
//                                 enew.rs lossless modular, evdct.rs lossy VarDCT, options
//                                 below) when the app has one (writer.rs, jxl-module: linked in
//                                 with img_jxl_enc::use_static(), or the module file jxlenc.dll /
//                                 .so with writer::use_module); with IOF_JXL_COMPAT, without a
//                                 new writer or without the module file the assembly's writer
//                                 (enc.rs / venc.rs)
//   jpeg::reconstruct(data) -> JPEG bytes / ERR_*   jxl-plus: the original JPEG file of a JPEG
//                                 transcode (jbrd box; byte-identical to libjxl's); jpeg::has_jbrd
//   box_contents(data, box)       jxl-plus: an Exif / xml box (Brotli decompressed when it is a
//                                 brob box: libjxl's JPEG transcodes; decode reads them as EXIF /
//                                 XMP too)
//
// Writer options (ImgOpt) -- the save dialog's controls                        [jxl-plus]
//   quality   1..99 lossy (VarDCT), 100 lossless; 0 = 90.  Butteraugli distance as libjxl:
//             q >= 30: 0.1 + (100 - q) * 0.09, else 53 / 3000 q^2 - 23 / 20 q + 25
//   effort    1..9 like libjxl's (0 = 7, 1 works like 2); higher = smaller, slower.  What
//             each effort adds:
//               lossless  1-2 gradient predictor, fixed tree; 3 weighted predictor, fixed
//                         tree; 1-4 YCoCg; 4+ learned MA tree (both predictors); 5+ local
//                         palettes, RCT search per group, patches (jxl-tools: repeated
//                         text glyphs / UI pieces in a reference frame; 9 keeps the smaller
//                         file with / without); 8+ WP parameter search, LZ77, more tree
//                         properties
//               lossy     1-3 DCT8, one quant value; 4 near-lossless DC, block context
//                         model, own coefficient orders; 5 adaptive quantization, AC
//                         strategy search (DCT2 .. DCT64, identity), chroma from luma,
//                         gaborish; 6 DCT4x8 / AFV, unaligned blocks; 7 (default) better
//                         CfL, chroma scaling from the pixels, patches (jxl-tools; from
//                         distance 3 also dots when there are no text-like patches); 8
//                         learned DC / metadata trees, the Butteraugli quantization loop
//                         (jxl-tools: 2 round trips of the frame, libjxl's
//                         FindBestQuantization); 9 LZ77, 4 round trips
//   flags     IOF_LOSSLESS          lossless whatever the quality
//             IOF_EMBED_ICC         write meta.icc (RGB / gray profiles).  Lossy: an sRGB
//                                   profile is written as plain sRGB; other profiles keep
//                                   their values (coded as YCbCr, no XYB without colour
//                                   management) and the profile
//             IOF_EMBED_EXIF / _XMP meta.exif / meta.xmp as Exif / xml boxes (container)
//             IOF_JXL_PROGRESSIVE   0x400000 (enew.rs): lossy: a 1:8 DC frame and three AC
//                                   passes; lossless: squeeze (coarse to fine).  A few %
//                                   larger
//             IOF_JXL_1THREAD       0x200000: no worker threads (the same file, slower)
//             IOF_JXL_COMPAT        0x100000: the assembly-compatible writer (byte-identical
//                                   to the asm; quality / lossless / EXIF / XMP only)
//             IOF_JXL_NOPATCHES     0x800000 (jxl-tools): no patches at any effort
//             bits 24..31           (jxl-tools, IOF_JXL_NOISE_SHIFT / _MASK) photon noise:
//                                   ISO / 100 (0 = off, the default; 16 = ISO 1600): lossy XYB
//                                   frames carry libjxl's noise LUT for that ISO (cjxl
//                                   --photon_noise_iso), the decoder adds the grain; lossless
//                                   and YCbCr frames ignore it
//   cancel    polled between the stages: ERR_CANCEL
// The file never depends on the number of threads.  Extra channels other than alpha, more
// than 8 bits, animation: not written; splines only by the test-only espline.rs (codectest
// op 197).
//
// Modules (one per assembly file): bits (jxl_bits.s), hdr (jxl_hdr.s), icc (jxl_icc.s), ent
// (jxl_ent.s), frm (jxl_frm.s), dec (jxl_dec.s), modular (jxl_mod.s), mtr (jxl_mtr.s), vdc
// (jxl_vdc.s), vac (jxl_vac.s), vqt (jxl_vqt.s), vrc (jxl_vrc.s), dct (jxl_dct.s), rnd
// (jxl_rnd.s), epf (jxl_epf.s), blend (jxl_blend.s), noise (jxl_noise.s), tab (jxl_tab.s,
// generated by tools/gen_jxl.py); the writer: ewr (jxl_ewr.s), enc (jxl_enc.s), venc
// (jxl_venc.s).  jxl-plus (no asm): emath (libjxl's fast math), jpeg (JPEG reconstruction) with
// brotli (RFC 7932 decoder; brotli_tab.rs / brotli_dict.bin generated by
// tools/gen_brotli_dict.py), par (the pool helpers).  jxl-tools (no asm): spline (decoder:
// splines).  jxl-module: writer (the new writer's options, samples and hook).  The new writer
// itself (libjxl 0.11.1's encoder: enew, evdct, emod, ema, eans, efrm, eicc, etx, dctk, eaq,
// eacs, ecfl, epatch, edots, espline, enoise, efeat, ebutter, ebq) is the crate img_jxl_enc
// (lib/img_jxl_enc/lib.rs), linked into codectest and built into the module jxlenc
// (modules/jxlenc) that PaintCraft loads; Recon and decode_linear below are its reconstruction
// helpers.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use gfx::FMT_ARGB;
use img_core::meta::{meta_blob, surf_alloc, Blob};
use img_core::{
    Codec, ImgDec, ImgMeta, Surface, ERR_HDR, ERR_LIMIT, ERR_NOMEM, IFC_DECODE, IFC_ENCODE, IMF_ALPHA, IMF_ANIMATED, IMF_FLOAT,
    IMF_HDR, IMGD_EXIF, IMGD_ICC, IMGD_PROBE, IMGD_XMP, IMGF_JXL, IMM_GRAY, IMM_RGB, IOM_EFFORT, IOM_EXIF, IOM_ICC,
    IOM_LOSSLESS, IOM_QUALITY, IOM_XMP,
};

pub mod bits;
pub mod blend;
pub mod brotli;
pub mod brotli_tab;
pub mod dct;
pub mod dec;
pub mod emath;
pub mod enc;
pub mod ent;
pub mod epf;
pub mod ewr;
pub mod frm;
pub mod hdr;
pub mod icc;
pub mod jpeg;
pub mod modular;
pub mod mtr;
pub mod noise;
pub mod par; // jxl-module: the pool helpers (from emod.rs)
pub mod rnd;
pub mod spline;
pub mod tab;
pub mod vac;
pub mod vdc;
pub mod venc;
pub mod vqt;
pub mod vrc;
pub mod writer; // jxl-module: the new writer's options / samples / hook

use bits::{Jb, JXB_PAD};
use frm::{jf_header, jf_toc, Jf, JBM_REPLACE, JCT_NONE, JFF_NOISE, JFF_PATCHES, JFF_SPLINES, JFT_REGULAR, JFT_SKIPPROG};
use hdr::{Jx, JCS_GRAY};

/// an opened file: the codestream copy (with JXB_PAD zero bytes) and the decoder record
pub struct Dec {
    pub cs: Vec<u8>,
    pub jx: Jx,
}

impl Dec {
    /// the header reader at the first frame header
    pub fn reader(&self) -> Jb<'_> {
        let mut br = Jb::init(&self.cs, self.jx.cslen);
        br.pos = self.jx.hdrend;
        br
    }
}

#[inline]
fn be32(d: &[u8], p: usize) -> u32 {
    u32::from_be_bytes([d[p], d[p + 1], d[p + 2], d[p + 3]])
}

/// xc_box: the box at pos -> (body start, body end, next box, type)
fn xc_box(d: &[u8], pos: u64) -> Result<(u64, u64, u64, u32), i32> {
    let len = d.len() as u64;
    if pos + 8 > len {
        return Err(ERR_HDR);
    }
    let p = pos as usize;
    let mut size = be32(d, p) as u64;
    let typ = u32::from_le_bytes([d[p + 4], d[p + 5], d[p + 6], d[p + 7]]);
    let mut body = pos + 8;
    if size == 1 {
        if pos + 16 > len {
            return Err(ERR_HDR);
        }
        size = u64::from_be_bytes([d[p + 8], d[p + 9], d[p + 10], d[p + 11], d[p + 12], d[p + 13], d[p + 14], d[p + 15]]);
        body = pos + 16;
    } else if size == 0 {
        size = len - pos;
    }
    if size < body - pos {
        return Err(ERR_HDR);
    }
    let end = pos.checked_add(size).ok_or(ERR_HDR)?;
    Ok((body, end.min(len), end, typ))
}

/// the contents of a metadata box (jxl-plus): (start, size, brob) from Jx.exifbox / xmlbox;
/// a brob box is Brotli decompressed
pub fn box_contents(data: &[u8], b: (usize, usize, bool)) -> Result<Vec<u8>, i32> {
    let s = data.get(b.0..b.0.wrapping_add(b.1)).ok_or(ERR_HDR)?;
    if !b.2 {
        let mut v = Vec::new();
        if v.try_reserve_exact(s.len()).is_err() {
            return Err(ERR_NOMEM);
        }
        v.extend_from_slice(s);
        return Ok(v);
    }
    brotli::decompress(s, 1 << 28).map(|(v, _)| v).map_err(|e| if e == ERR_NOMEM { e } else { ERR_HDR })
}

/// jx_open: codestream copy and image headers
pub fn open(data: &[u8]) -> Result<Dec, i32> {
    let len = data.len();
    if len < 2 {
        return Err(ERR_HDR);
    }
    let mut jx = Jx::default();
    let cs: Vec<u8>;
    if data[0] == 0xFF && data[1] == 0x0A {
        let mut v: Vec<u8> = Vec::new();
        if v.try_reserve_exact(len + JXB_PAD).is_err() {
            return Err(ERR_NOMEM);
        }
        v.extend_from_slice(data);
        v.resize(len + JXB_PAD, 0);
        cs = v;
        jx.cslen = len as u64;
    } else {
        if len < 12 || data[..8] != [0, 0, 0, 0x0C, b'J', b'X', b'L', b' '] || data[8..12] != [0x0D, 0x0A, 0x87, 0x0A] {
            return Err(ERR_HDR);
        }
        let mut total: u64 = 0;
        let mut v: Vec<u8> = Vec::new();
        for pass in 0..2 {
            let mut pos: u64 = 0;
            while pos + 8 <= len as u64 {
                let (bs, be, next, typ) = xc_box(data, pos)?;
                if pass == 0 {
                    // jxl-plus: boxes of the JPEG reconstruction
                    let n = be.wrapping_sub(bs) as usize;
                    let (mut t, mut c) = (typ, (bs as usize, n, false));
                    if typ == 0x626F_7262 && n >= 4 {
                        // "brob": the type of the box, its Brotli compressed contents
                        let p = bs as usize;
                        t = u32::from_le_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]);
                        c = (p + 4, n - 4, true);
                    }
                    match t {
                        0x6472_626A if jx.jbrd.is_none() && !c.2 => jx.jbrd = Some((c.0, c.1)), // "jbrd"
                        0x6669_7845 if jx.exifbox.is_none() => jx.exifbox = Some(c),
                        0x206C_6D78 if jx.xmlbox.is_none() => jx.xmlbox = Some(c),
                        _ => {}
                    }
                    if typ == 0x6669_7845 {
                        // "Exif": u32 BE offset of the TIFF header, data
                        let n = be.wrapping_sub(bs);
                        if n >= 4 {
                            let off = be32(data, bs as usize) as u64;
                            let n = n - 4;
                            if off <= n {
                                jx.exif = Some(((bs + 4 + off) as usize, (n - off) as usize));
                                pos = next;
                                continue;
                            }
                        }
                    } else if typ == 0x206C_6D78 {
                        // "xml "
                        jx.xmp = Some((bs as usize, be.wrapping_sub(bs) as usize));
                        pos = next;
                        continue;
                    }
                }
                let start = match typ {
                    0x636C_786A => Some(bs),     // "jxlc"
                    0x706C_786A => Some(bs + 4), // "jxlp"
                    _ => None,
                };
                if let Some(st) = start
                    && be > st
                {
                    if pass == 0 {
                        total += be - st;
                    } else {
                        v.extend_from_slice(&data[st as usize..be as usize]);
                    }
                }
                pos = next;
            }
            if pass == 0 {
                if total == 0 {
                    return Err(ERR_HDR);
                }
                if v.try_reserve_exact(total as usize + JXB_PAD).is_err() {
                    return Err(ERR_NOMEM);
                }
            }
        }
        v.resize(total as usize + JXB_PAD, 0);
        cs = v;
        jx.cslen = total;
    }
    let mut br = Jb::init(&cs, jx.cslen);
    let r = jx.headers(&mut br);
    if r != 0 {
        return Err(r);
    }
    Ok(Dec { cs, jx })
}

/// jx_frames: displayed frames (0 on a damaged frame list); br is the header reader
pub fn jx_frames(jx: &Jx, br: &mut Jb) -> u32 {
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    let mut n = 0u32;
    let mut preview = jx.haveprev != 0;
    loop {
        let mut jf = Jf::default();
        if jf_header(jx, br, &mut jf, preview) != 0 {
            break;
        }
        let r = jf_toc(br, &mut jf);
        let end = jf.end;
        if r != 0 {
            break;
        }
        if !preview && (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG) && (jf.islast != 0 || jf.duration != 0) {
            n += 1;
        }
        if !preview && jf.islast != 0 {
            break;
        }
        preview = false;
        if end >= jx.cslen {
            break;
        }
        br.pos = end << 3;
    }
    n
}

/// jx_meta: image properties into meta
pub fn jx_meta(jx: &Jx, br: &mut Jb, m: &mut ImgMeta) {
    m.fmt = IMGF_JXL;
    m.w = jx.w;
    m.h = jx.h;
    if jx.flt != 0 {
        m.flags |= IMF_FLOAT;
    }
    m.bits = jx.bits as u8;
    if jx.bits > 8 {
        m.flags |= IMF_HDR;
    }
    let (mut model, mut ch) = (IMM_RGB, 3u8);
    if jx.cspace == JCS_GRAY {
        model = IMM_GRAY;
        ch = 1;
    }
    m.model = model;
    if jx.ec.iter().take(jx.nec as usize).any(|e| e.typ == 0) {
        m.flags |= IMF_ALPHA;
        ch += 1;
    }
    m.channels = ch;
    m.orient = jx.orient;
    m.frames = 1;
    if jx.haveanim != 0 {
        m.loops = jx.loops;
        let n = jx_frames(jx, br);
        if n != 0 {
            m.frames = n;
            if n > 1 {
                m.flags |= IMF_ANIMATED;
            }
        }
    }
}

/// jxl_probe
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    match open(data) {
        Ok(d) => {
            let mut br = d.reader();
            jx_meta(&d.jx, &mut br, meta);
            true
        }
        Err(_) => false,
    }
}

/// xo_scale: sample of `bits` bits to 0..255 (rounded, clamped)
fn xo_scale(v: i32, bits: u32) -> u32 {
    if bits == 8 {
        return v.clamp(0, 255) as u32;
    }
    if v <= 0 {
        return 0;
    }
    let max = (1u64 << (bits & 63)).wrapping_sub(1);
    let v = v as u64;
    if v >= max {
        return 255;
    }
    ((v * 510 + max) / (max + max)) as u32
}

/// xo_int: integer samples of a modular frame into the ARGB surface
fn xo_int(jx: &Jx, jf: &Jf, sf: &mut Surface) -> i32 {
    let Some(full) = jf.full.as_ref() else { return ERR_HDR };
    let nb = jf.nbcol as usize;
    let mut alpha = None;
    let mut abits = 8u32;
    for e in 0..jx.nec as usize {
        if jx.ec[e].typ == 0 {
            abits = jx.ec[e].bits;
            alpha = full.ch.get(nb + e);
            break;
        }
    }
    let ch0 = &full.ch[0];
    let stride = sf.stride as usize;
    for y in 0..sf.h as usize {
        for x in 0..sf.w as usize {
            let k = ch0.w as usize * y + x;
            let mut px = 0u32;
            for c in 0..3 {
                let ch = if nb == 1 { ch0 } else { &full.ch[c] };
                px = px << 8 | xo_scale(ch.d[k], jx.bits);
            }
            px |= 0xFF00_0000;
            if let Some(a) = alpha {
                px = (px & 0x00FF_FFFF) | xo_scale(a.d[k], abits) << 24;
            }
            sf.px[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&px.to_le_bytes());
        }
    }
    0
}

/// jxl_decode (see the module header)
pub fn decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    decode_opt(data, sf, dec, Some(meta))
}

/// jxl_decode with an optional ImgMeta (the assembly's ID_META may be 0: then the image
/// properties are not read, which leaves the header reader's state alone)
pub fn decode_opt(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: Option<&mut ImgMeta>) -> i32 {
    *sf = Surface::default();
    let r = decode_body(data, sf, dec, meta);
    if r != 0 {
        *sf = Surface::default(); // no surface on failure
    }
    r
}

fn decode_body(data: &[u8], sf: &mut Surface, dec: &ImgDec, mut meta: Option<&mut ImgMeta>) -> i32 {
    let d = match open(data) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let mut jx = d.jx;
    let cs = d.cs;
    let mut br = Jb::init(&cs, jx.cslen);
    if let Some(m) = meta.as_deref_mut() {
        jx_meta(&jx, &mut br, m);
        if dec.flags & IMGD_ICC != 0 && !jx.icc.is_empty() {
            meta_blob(m, Blob::Icc, &jx.icc);
        }
        if dec.flags & IMGD_EXIF != 0 {
            if let Some((o, n)) = jx.exif {
                meta_blob(m, Blob::Exif, &data[o..o + n]);
            } else if let Some(b) = jx.exifbox
                && let Ok(v) = box_contents(data, b)
                && v.len() >= 4
            {
                // jxl-plus: a Brotli compressed Exif box (libjxl's JPEG transcodes)
                let off = be32(&v, 0) as usize;
                if off <= v.len() - 4 {
                    meta_blob(m, Blob::Exif, &v[4 + off..]);
                }
            }
        }
        if dec.flags & IMGD_XMP != 0 {
            if let Some((o, n)) = jx.xmp {
                meta_blob(m, Blob::Xmp, &data[o..o + n]);
            } else if let Some(b) = jx.xmlbox
                && let Ok(v) = box_contents(data, b)
            {
                meta_blob(m, Blob::Xmp, &v);
            }
        }
    }
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    match surf_alloc(jx.w, jx.h, FMT_ARGB, dec) {
        Ok(s) => *sf = s,
        Err(e) => return e,
    }
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    let mut jf = Jf::default();
    if jx.haveprev != 0 {
        let r = jf_header(&jx, &mut br, &mut jf, true);
        if r != 0 {
            return r;
        }
        noise::jx_count(&mut jx, &mut jf);
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            return r;
        }
        br.pos = jf.end << 3;
        jf = Jf::default();
    }
    let mut k = 0u32;
    loop {
        let r = jf_header(&jx, &mut br, &mut jf, false);
        if r != 0 {
            return r;
        }
        noise::jx_count(&mut jx, &mut jf);
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            return r;
        }
        // integer path: the first frame is the only one, regular modular, no colour
        // transform, upsampling, blending or filters; full-size extra channels
        let intpath = dec.frame == 0
            && k == 0
            && jf.islast != 0
            && jf.typ == JFT_REGULAR
            && jf.modular != 0
            && jf.ct == JCT_NONE
            && jf.ups == 1
            && jf.custom == 0
            && jf.bmode == JBM_REPLACE
            && jf.gab == 0
            && jf.epfit == 0
            && jf.flags & (JFF_PATCHES | JFF_NOISE | JFF_SPLINES) == 0
            && jx.flt == 0;
        let r = dec::jx_frame(&jx, &cs, &mut jf);
        if r != 0 {
            return r;
        }
        if intpath {
            let full = jf.full.as_ref().unwrap();
            let fits = full.nch() == jf.nbcol + jx.nec
                && full.ch.iter().all(|c| c.w == sf.w as u32 && c.h == sf.h as u32);
            if fits {
                return xo_int(&jx, &jf, sf);
            }
        }
        let r = rnd::jx_render(&mut jx, &mut jf);
        if r != 0 {
            return r;
        }
        if (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG) && (jf.islast != 0 || jf.duration != 0) {
            let kk = k;
            k += 1;
            if kk == dec.frame {
                let r = rnd::jx_out8(&jf, sf);
                if r != 0 {
                    return r;
                }
                if let Some(m) = meta.as_deref_mut()
                    && jx.tpsnum != 0
                {
                    let v = (jf.duration as u64 * 1000).wrapping_mul(jx.tpsden as u64) / jx.tpsnum as u64;
                    m.delay = v.min(0xFFFF_FFFF) as u32;
                }
                return 0;
            }
        }
        if jf.islast != 0 || jf.end >= jx.cslen {
            return ERR_LIMIT; // past the last frame
        }
        br.pos = jf.end << 3;
        jf = Jf::default();
    }
}

/// the writer's reconstruction for its EPF sharpness search (jxl-plus; img_jxl_enc): the displayed frame
/// decoded once up to the EPF; planes(sharpness) runs the EPF with that sharpness per block
/// on a copy
pub struct Recon {
    jx: Jx,
    jf: Jf,
    pre: rnd::Jrn,
    pad0: Vec<Vec<f32>>,   // pre padded for the first EPF stage (jxl-speed: once)
    out: Option<rnd::Jrn>, // the planes of the last call (reused)
    pads: [Vec<Vec<f32>>; 2], // the paddings of the later EPF stages (reused)
}

impl Recon {
    pub fn new(cs: &[u8]) -> Result<Recon, i32> {
        let d = open(cs)?;
        let mut jx = d.jx;
        let cs = d.cs;
        let mut br = Jb::init(&cs, jx.cslen);
        br.pos = jx.hdrend;
        br.end = jx.cslen << 3;
        loop {
            let mut jf = Jf::default();
            let r = jf_header(&jx, &mut br, &mut jf, false);
            if r != 0 {
                return Err(r);
            }
            noise::jx_count(&mut jx, &mut jf);
            let r = jf_toc(&mut br, &mut jf);
            if r != 0 {
                return Err(r);
            }
            let shown = (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG) && (jf.islast != 0 || jf.duration != 0);
            if shown {
                jf.dbg = 8;
            }
            let r = dec::jx_frame(&jx, &cs, &mut jf);
            if r != 0 {
                return Err(r);
            }
            let r = rnd::jx_render(&mut jx, &mut jf);
            if r != 0 {
                return Err(r);
            }
            if shown {
                let Some(pre) = jf.rnd.take() else { return Err(ERR_HDR) };
                if pre.p.len() < 3 || jf.modular != 0 {
                    return Err(ERR_HDR);
                }
                return Ok(Recon { jx, jf, pre: *pre, pad0: Vec::new(), out: None, pads: [Vec::new(), Vec::new()] });
            }
            if jf.islast != 0 || jf.end >= jx.cslen {
                return Err(ERR_LIMIT);
            }
            br.pos = jf.end << 3;
        }
    }

    /// (width, height, row stride, X / Y / B) after the EPF with this sharpness per block
    /// (jxl-speed: the padding of the frame for the first stage is made once, the output
    /// planes are reused: every sample the EPF writes is rewritten each call, the rest is the
    /// frame's as in a fresh copy)
    pub fn planes(&mut self, sharp: &[u8]) -> Result<(u32, u32, usize, [&[f32]; 3]), i32> {
        let r = vdc::jv_sharpness(&mut self.jf, sharp);
        if r != 0 {
            return Err(r);
        }
        if self.out.is_none() {
            // a copy of the frame: zeroed planes (fresh pages) filled by bands on the pool
            let mut o = rnd::Jrn { np: self.pre.np, alpha: self.pre.alpha, xyb: self.pre.xyb, p: Vec::with_capacity(self.pre.p.len()) };
            for jp in &self.pre.p {
                let mut d = modular::zvec::<f32>(jp.d.len())?;
                let s = (jp.s as usize).max(1);
                let rows = jp.d.len().div_ceil(s);
                par::par_rows(alloc::vec![(&mut d[..], s)], rows, 32, true, |y0, n, b| {
                    let a = y0 * s;
                    let e = (a + n * s).min(jp.d.len());
                    b[0][..e - a].copy_from_slice(&jp.d[a..e]);
                });
                o.p.push(rnd::Jrp { d, w: jp.w, h: jp.h, s: jp.s });
            }
            self.out = Some(o);
        }
        let Some(jrn) = self.out.as_mut() else { return Err(ERR_HDR) };
        if self.jf.epfit != 0 {
            if self.pad0.is_empty() {
                let r = epf::pads_into(&self.pre, epf::first_stage(&self.jf), &mut self.pad0);
                if r != 0 {
                    return Err(r);
                }
            }
            let r = epf::jr_epf_from(&self.jx, &self.jf, &self.pad0, &mut self.pads, jrn);
            if r != 0 {
                return Err(r);
            }
        }
        if jrn.p.len() < 3 {
            return Err(ERR_HDR);
        }
        let (w, h, s) = (jrn.p[0].w, jrn.p[0].h, jrn.p[0].s as usize);
        if jrn.p[1].s as usize != s || jrn.p[2].s as usize != s {
            return Err(ERR_HDR);
        }
        Ok((w, h, s, [&jrn.p[0].d[..], &jrn.p[1].d[..], &jrn.p[2].d[..]]))
    }
}

/// the writer's Butteraugli round trips (jxl-tools): the shown frame of a codestream of the
/// writer (XYB, no blending) decoded to linear sRGB planes of w x h
pub fn decode_linear(cs: &[u8], w: usize, h: usize) -> Result<[Vec<f32>; 3], i32> {
    let d = open(cs)?;
    let mut jx = d.jx;
    let cs = d.cs;
    let mut br = Jb::init(&cs, jx.cslen);
    br.pos = jx.hdrend;
    br.end = jx.cslen << 3;
    loop {
        let mut jf = Jf::default();
        let r = jf_header(&jx, &mut br, &mut jf, false);
        if r != 0 {
            return Err(r);
        }
        noise::jx_count(&mut jx, &mut jf);
        let r = jf_toc(&mut br, &mut jf);
        if r != 0 {
            return Err(r);
        }
        let shown = (jf.typ == JFT_REGULAR || jf.typ == JFT_SKIPPROG) && (jf.islast != 0 || jf.duration != 0);
        if shown {
            jf.dbg |= rnd::JDBG_LINEAR;
        }
        let r = dec::jx_frame(&jx, &cs, &mut jf);
        if r != 0 {
            return Err(r);
        }
        let r = rnd::jx_render(&mut jx, &mut jf);
        if r != 0 {
            return Err(r);
        }
        if shown {
            let Some(jrn) = jf.rnd.take() else { return Err(ERR_HDR) };
            if jrn.p.len() < 3 {
                return Err(ERR_HDR);
            }
            let mut out: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
            for (c, o) in out.iter_mut().enumerate() {
                let p = &jrn.p[c];
                let s = p.s as usize;
                if (p.w as usize) < w || (p.h as usize) < h || p.d.len() < (h.max(1) - 1) * s + w {
                    return Err(ERR_HDR);
                }
                o.reserve_exact(w * h);
                for y in 0..h {
                    o.extend_from_slice(&p.d[y * s..y * s + w]);
                }
            }
            return Ok(out);
        }
        if jf.islast != 0 || jf.end >= jx.cslen {
            return Err(ERR_LIMIT);
        }
        br.pos = jf.end << 3;
    }
}

/// the codec descriptor  [jxl_codec]
pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode),
    encode: Some(enc::encode),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: IOM_QUALITY | IOM_LOSSLESS | IOM_EFFORT | IOM_ICC | IOM_EXIF | IOM_XMP, // jxl-plus: ICC, EXIF, XMP
    ..Codec::EMPTY
};
