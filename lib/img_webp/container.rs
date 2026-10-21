// container -- WebP container (RIFF "WEBP") reader and writer: simple VP8 / VP8L, extended
// VP8X (ALPH, ICCP, EXIF, XMP, ANIM / ANMF)                              [src/img/webp.s]
//
//   probe(data, meta) -> recognised                     webp_probe
//   decode(data, sf, dec, meta) -> 0 / ERR_*            webp_decode
//   encode(sf, opt, out) -> 0 / ERR_*                   webp_encode
//   enc_alpha(px, w, h, cfg) -> Ok(ALPH payload)        webp_enc_alpha
//   parse(data) -> (0 / ERR_*, Info)                    webp_parse
//   alpha(alph, plane, w, h, dec) -> 0 / ERR_*          webp_alpha
//
// Container rules (libwebp 1.3 webp_dec.c / demux.c): RIFF size 12 .. 2^32 - 10 and not
// beyond the data (else ERR_TRUNC); bytes after the RIFF are ignored.  The first chunk is
// "VP8 " / "VP8L" (simple file) or "VP8X" (size exactly 10, canvas 24-bit + 1, w * h <
// 2^32).  Chunks are walked with padded 64-bit sizes that must stay inside the RIFF.
// Still VP8X files: ICCP / EXIF / XMP are taken when their VP8X flag is set (first chunk of
// each kind, like WebPDemuxGetChunk), the last ALPH before the image chunk applies to a VP8
// image (whatever the VP8X alpha flag says; ignored for VP8L), the image size must equal the
// canvas; chunk errors after the image only end the metadata walk.  Animations (VP8X
// animation flag): ANIM (>= 6 bytes, before the frames) gives the loop count, every ANMF
// (>= 16 bytes) counts as a frame; the first frame (optional ALPH + VP8, or VP8L, inside the
// ANMF, frame rectangle inside the canvas) is decoded like WebPAnimDecoder does: onto a
// transparent-black canvas (the ANIM background colour is only a hint and, as in libwebp,
// not used), without blending.  Errors after the first frame end the frame count.
// Bitstreams may read up to the end of the data like libwebp (VP8's last partition, VP8L's
// end-of-stream limit); frames up to their padded chunk end.
//
// Result: FMT_ARGB (straight; lossy images without ALPH get SFF_OPAQUE).  ImgMeta: size =
// canvas, bits 8, model YCC (VP8) / RGB (VP8L), channels 3 / 4, IMF_ALPHA (VP8X alpha flag,
// VP8L alpha bit, ALPH chunk), IMF_LOSSLESS, IMF_ANIMATED (> 1 frame), frames, loops, delay
// (first frame), orientation from EXIF, blobs with IMGD_ICC / IMGD_EXIF (a leading
// "Exif\0\0" is dropped) / IMGD_XMP.  dec.frame > 0: ERR_UNSUPPORTED (ERR_LIMIT past the
// last frame).

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, SFF_OPAQUE, Surface};
use img_core::exif::{exif_copy_orient1, exif_orientation};
use img_core::meta::{Blob, dims_check, get_row, mem_check, meta_blob, surf_alloc};
use img_core::*;

use crate::{Cfg, WEF_NOEXACT, WEF_NOHDR};
use crate::{vp8, vp8_enc, vp8l, vp8l_enc};

const TAG_RIFF: u32 = 0x4646_4952;
const TAG_WEBP: u32 = 0x5042_4557;
const TAG_VP8X: u32 = 0x5838_5056;
const TAG_VP8: u32 = 0x2038_5056;
const TAG_VP8L: u32 = 0x4C38_5056;
const TAG_ALPH: u32 = 0x4850_4C41;
const TAG_ICCP: u32 = 0x5043_4349;
const TAG_EXIF: u32 = 0x4649_5845;
const TAG_XMP: u32 = 0x2050_4D58;
const TAG_ANIM: u32 = 0x4D49_4E41;
const TAG_ANMF: u32 = 0x464D_4E41;

const VP8X_ICC: u32 = 0x20;
const VP8X_ALPHA: u32 = 0x10;
const VP8X_EXIF: u32 = 0x08;
const VP8X_XMP: u32 = 0x04;
const VP8X_ANIM: u32 = 0x02;
const MAX_PAYLOAD: u32 = 0xFFFF_FFF6; // libwebp MAX_CHUNK_PAYLOAD

pub const WK_VP8: u32 = 1;
pub const WK_VP8L: u32 = 2;

/// the container walk (WPINFO): positions are offsets into the data
#[derive(Clone, Copy, Default)]
pub struct Info {
    pub cw: u32,              // canvas width
    pub ch: u32,              // canvas height
    pub flags: u32,           // VP8X flags word (0 = simple file)
    pub kind: u32,            // WK_VP8 / WK_VP8L (image / first frame)
    pub img: Option<usize>,   // bitstream (VP8 / VP8L chunk payload)
    pub ilen: u64,            // chunk payload size
    pub iavail: u64,          // bytes the decoder may read from img
    pub alph: Option<usize>,  // ALPH payload
    pub alen: u64,
    pub icc: Option<usize>,
    pub icclen: u64,
    pub exif: Option<usize>,
    pub exiflen: u64,
    pub xmp: Option<usize>,
    pub xmplen: u64,
    pub iw: u32,              // bitstream width
    pub ih: u32,
    pub fx: u32,              // frame offset
    pub fy: u32,
    pub frames: u32,          // 1 for still images
    pub loops: u32,           // ANIM loop count
    pub dur: u32,             // first frame duration (ms)
    pub bg: u32,              // ANIM background colour (B, G, R, A bytes)
    pub alpha: u32,           // 1 = has alpha
    pub anim: u32,            // 1 = animation flag
    pub vp8x: u32,            // 1 = extended format
    pub animseen: u32,        // ANIM chunk read
}

#[inline]
fn rd32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

#[inline]
fn rd24(d: &[u8], o: usize) -> u32 {
    d[o] as u32 | (d[o + 1] as u32) << 8 | (d[o + 2] as u32) << 16
}

// ------------------------------------------------------------------ public entry points

/// container + bitstream headers into meta (size, model, channels, flags, frames, loops,
/// delay, EXIF orientation; no blobs) -> recognised  [webp_probe]
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    let (r, mut info) = parse(data);
    if r != 0 {
        return false;
    }
    wp_meta(meta, &mut info, data);
    true
}

/// decode the still image or the first animation frame into a new FMT_ARGB surface
/// [webp_decode]
pub fn decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    *sf = Surface::default();
    let (r, mut info) = parse(data);
    if r != 0 {
        return r;
    }
    wp_meta(meta, &mut info, data);
    wp_blobs(meta, dec.flags, &info, data);
    if dec.frame != 0 {
        return if dec.frame < info.frames { ERR_UNSUPPORTED } else { ERR_LIMIT };
    }
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    let r = dims_check(info.cw, info.ch, dec);
    if r != 0 {
        return r;
    }
    // does the image cover the canvas?
    if info.iw == info.cw && info.ih == info.ch {
        return match image_decode(&info, data, dec) {
            Ok(s) => {
                *sf = s;
                0
            }
            Err(e) => e,
        };
    }
    // animation frame smaller than the canvas: decode, then place on a transparent canvas
    let tmp = match image_decode(&info, data, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let mut c = match surf_alloc(info.cw, info.ch, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (cs, ts) = (c.stride as usize, tmp.stride as usize);
    let n = info.iw as usize * 4;
    for y in 0..info.ih as usize {
        let d = (info.fy as usize + y) * cs + info.fx as usize * 4;
        c.px[d..d + n].copy_from_slice(&tmp.px[y * ts..y * ts + n]);
    }
    *sf = c;
    0
}

// ------------------------------------------------------------------ container

/// walk the RIFF container (see the file header; ERR_TRUNC / ERR_HDR)  [webp_parse]
pub fn parse(data: &[u8]) -> (i32, Info) {
    let mut i = Info { frames: 1, ..Default::default() };
    let r = parse_body(data, &mut i);
    (r, i)
}

fn parse_body(d: &[u8], i: &mut Info) -> i32 {
    let len = d.len();
    if len < 12 {
        return ERR_TRUNC;
    }
    if rd32(d, 0) != TAG_RIFF || rd32(d, 8) != TAG_WEBP {
        return ERR_HDR;
    }
    let riff = rd32(d, 4);
    if !(12..=MAX_PAYLOAD).contains(&riff) {
        return ERR_HDR;
    }
    if riff as usize > len - 8 {
        return ERR_TRUNC;
    }
    let rend = riff as usize + 8; // end of the RIFF
    let mut p = 12; // first chunk
    let t = rd32(d, p);
    if t == TAG_VP8 || t == TAG_VP8L {
        // "VP8 " / "VP8L" first: size <= RIFF size - 12, inside the data
        if rd32(d, p + 4) > riff - 12 {
            return ERR_HDR;
        }
        let r = wp_image(i, d, p, len);
        if r != 0 {
            return r;
        }
        i.cw = i.iw;
        i.ch = i.ih;
        return 0;
    }
    if t != TAG_VP8X {
        return ERR_HDR;
    }
    if rd32(d, p + 4) != 10 || p + 18 > rend {
        return ERR_HDR;
    }
    i.vp8x = 1;
    i.flags = rd32(d, p + 8);
    if i.flags & VP8X_ALPHA != 0 {
        i.alpha = 1;
    }
    if i.flags & VP8X_ANIM != 0 {
        i.anim = 1;
        i.frames = 0;
    }
    i.cw = rd24(d, p + 12) + 1;
    i.ch = rd24(d, p + 15) + 1;
    if (i.cw as u64 * i.ch as u64) >> 32 != 0 {
        return ERR_HDR; // w * h >= 2^32
    }
    p += 18;
    let mut bad = false;
    while p + 8 <= rend {
        let sz = rd32(d, p + 4);
        if sz > MAX_PAYLOAD {
            bad = true;
            break;
        }
        let next = p + ((sz as usize + 1) & !1) + 8; // padded
        if next > rend {
            bad = true;
            break;
        }
        let t = rd32(d, p);
        let sz = sz as u64;
        if t == TAG_ICCP {
            if i.flags & VP8X_ICC != 0 && i.icc.is_none() {
                i.icc = Some(p + 8);
                i.icclen = sz;
            }
        } else if t == TAG_EXIF {
            if i.flags & VP8X_EXIF != 0 && i.exif.is_none() {
                i.exif = Some(p + 8);
                i.exiflen = sz;
            }
        } else if t == TAG_XMP {
            if i.flags & VP8X_XMP != 0 && i.xmp.is_none() {
                i.xmp = Some(p + 8);
                i.xmplen = sz;
            }
        } else if i.anim != 0 {
            if t == TAG_ANIM {
                if i.animseen == 0 {
                    if sz < 6 {
                        bad = true;
                        break;
                    }
                    i.animseen = 1;
                    i.bg = rd32(d, p + 8);
                    i.loops = u16::from_le_bytes([d[p + 12], d[p + 13]]) as u32;
                }
            } else if t == TAG_ANMF {
                if i.animseen == 0 || sz < 16 {
                    bad = true;
                    break;
                }
                if i.img.is_none() {
                    let r = wp_frame(i, d, p, next);
                    if r != 0 {
                        return r;
                    }
                }
                i.frames += 1;
            } else if t == TAG_ALPH || t == TAG_VP8 || t == TAG_VP8L {
                bad = true; // bare images in an animation
                break;
            }
        } else if t == TAG_ALPH {
            if i.img.is_none() {
                i.alph = Some(p + 8);
                i.alen = sz;
            }
        } else if (t == TAG_VP8 || t == TAG_VP8L) && i.img.is_none() {
            let r = wp_image(i, d, p, len);
            if r != 0 {
                return r;
            }
            if i.kind != WK_VP8 {
                i.alph = None; // VP8L carries its own alpha
                i.alen = 0;
            }
        }
        p = next;
    }
    // a broken chunk: an error before the image / first frame, else the end
    if bad && i.img.is_none() {
        return ERR_HDR;
    }
    if i.img.is_none() {
        return if i.anim != 0 { ERR_HDR } else { ERR_TRUNC };
    }
    if i.anim != 0 {
        return 0;
    }
    if i.iw != i.cw || i.ih != i.ch {
        return ERR_HDR; // still image = canvas
    }
    0
}

// wp_frame: the first ANMF: offsets, duration, sub-chunks (first ALPH, then VP8; or VP8L
// without ALPH; padded, inside the ANMF), frame rectangle inside the canvas
fn wp_frame(i: &mut Info, d: &[u8], c: usize, end: usize) -> i32 {
    i.fx = rd24(d, c + 8) * 2;
    i.fy = rd24(d, c + 11) * 2;
    i.dur = rd24(d, c + 20);
    let mut p = c + 24; // sub-chunks
    let mut aseen = false;
    while p + 8 <= end {
        let sz = rd32(d, p + 4);
        if sz > MAX_PAYLOAD {
            return ERR_HDR;
        }
        let next = p + ((sz as usize + 1) & !1) + 8;
        if next > end {
            return ERR_HDR;
        }
        let t = rd32(d, p);
        if t == TAG_ALPH {
            if aseen || i.img.is_some() {
                break;
            }
            aseen = true;
            i.alph = Some(p + 8);
            i.alen = sz as u64;
            p = next;
            continue;
        }
        if t == TAG_VP8L {
            if aseen {
                return ERR_HDR;
            }
        } else if t != TAG_VP8 {
            break;
        }
        if i.img.is_some() {
            break;
        }
        let r = wp_image(i, d, p, next);
        if r != 0 {
            return r;
        }
        p = next;
    }
    if i.img.is_none() {
        return ERR_HDR;
    }
    if i.fx + i.iw > i.cw || i.fy + i.ih > i.ch {
        return ERR_HDR;
    }
    0
}

// wp_image: the VP8 / VP8L chunk at c: bitstream header check (libwebp VP8GetInfo /
// VP8LGetInfo on the bytes up to aend), kind / img / ilen / iavail / iw / ih / alpha bit
fn wp_image(i: &mut Info, d: &[u8], c: usize, aend: usize) -> i32 {
    let sz = rd32(d, c + 4) as u64;
    let pl = c + 8; // payload
    if aend < pl {
        return ERR_TRUNC;
    }
    let avail = (aend - pl) as u64;
    if sz > avail {
        return ERR_TRUNC;
    }
    i.img = Some(pl);
    i.ilen = sz;
    i.iavail = avail;
    if rd32(d, c) == TAG_VP8L {
        i.kind = WK_VP8L;
        if avail < 5 {
            return ERR_TRUNC;
        }
        if d[pl] != 0x2F {
            return ERR_HDR;
        }
        let h = rd32(d, pl + 1);
        if h & 0xE000_0000 != 0 {
            return ERR_HDR; // version 0
        }
        i.iw = (h & 0x3FFF) + 1;
        i.ih = ((h >> 14) & 0x3FFF) + 1;
        if h & (1 << 28) != 0 {
            i.alpha = 1; // alpha hint
        }
        return 0;
    }
    // VP8: frame tag, key frame, profile <= 3, shown, first partition inside the chunk,
    // start code 9D 01 2A, 14-bit size
    i.kind = WK_VP8;
    if avail < 10 {
        return ERR_TRUNC;
    }
    let tag = rd24(d, pl);
    if tag & 1 != 0 || (tag >> 1) & 7 > 3 || tag & 16 == 0 || (tag >> 5) as u64 >= sz {
        return ERR_HDR;
    }
    if d[pl + 3] != 0x9D || d[pl + 4] != 0x01 || d[pl + 5] != 0x2A {
        return ERR_HDR;
    }
    let w = u16::from_le_bytes([d[pl + 6], d[pl + 7]]) as u32 & 0x3FFF;
    if w == 0 {
        return ERR_HDR;
    }
    i.iw = w;
    let h = u16::from_le_bytes([d[pl + 8], d[pl + 9]]) as u32 & 0x3FFF;
    if h == 0 {
        return ERR_HDR;
    }
    i.ih = h;
    0
}

// a blob without a leading "Exif\0\0"
fn exif_skip(b: &[u8]) -> &[u8] {
    if b.len() >= 6 && &b[..6] == b"Exif\0\0" { &b[6..] } else { b }
}

// wp_meta: ImgMeta scalars from an Info (EXIF orientation read in place)
fn wp_meta(m: &mut ImgMeta, i: &mut Info, d: &[u8]) {
    m.w = i.cw;
    m.h = i.ch;
    m.bits = 8;
    let mut fl = 0;
    m.model = IMM_YCC;
    if i.kind == WK_VP8L {
        m.model = IMM_RGB;
        fl |= IMF_LOSSLESS;
    }
    if i.alph.is_some() {
        i.alpha = 1;
    }
    m.channels = 3;
    if i.alpha != 0 {
        m.channels = 4;
        fl |= IMF_ALPHA;
    }
    m.frames = i.frames;
    if i.frames > 1 {
        fl |= IMF_ANIMATED;
    }
    m.flags |= fl;
    m.loops = i.loops;
    m.delay = i.dur;
    if let Some(e) = i.exif {
        m.orient = exif_orientation(exif_skip(&d[e..e + i.exiflen as usize])) as u8;
    }
}

// wp_blobs: copy ICC / EXIF / XMP into meta
fn wp_blobs(m: &mut ImgMeta, flags: u32, i: &Info, d: &[u8]) {
    if flags & IMGD_ICC != 0
        && let Some(o) = i.icc
    {
        meta_blob(m, Blob::Icc, &d[o..o + i.icclen as usize]);
    }
    if flags & IMGD_EXIF != 0
        && let Some(o) = i.exif
    {
        meta_blob(m, Blob::Exif, exif_skip(&d[o..o + i.exiflen as usize]));
    }
    if flags & IMGD_XMP != 0
        && let Some(o) = i.xmp
    {
        meta_blob(m, Blob::Xmp, &d[o..o + i.xmplen as usize]);
    }
}

// ------------------------------------------------------------------ image decoding

// wp_image_decode: decode the image / frame bitstream of info into a new FMT_ARGB surface
// (VP8 + ALPH, or VP8L)
fn image_decode(i: &Info, d: &[u8], dec: &ImgDec) -> Result<Surface, i32> {
    let o = i.img.unwrap_or(0);
    let bs = &d[o..o + i.iavail as usize];
    if i.kind == WK_VP8L {
        return vp8l::decode(bs, Some(dec));
    }
    let mut sf = vp8::decode(bs, Some(dec))?;
    let Some(ao) = i.alph else {
        sf.flags |= SFF_OPAQUE;
        return Ok(sf);
    };
    let (w, h) = (sf.w as usize, sf.h as usize);
    if mem_check((w * h) as u64, dec) != 0 {
        return Err(ERR_LIMIT);
    }
    let mut plane = vec![0u8; w * h + 64];
    let r = alpha(&d[ao..ao + i.alen as usize], &mut plane, w as u32, h as u32, Some(dec));
    if r != 0 {
        return Err(r);
    }
    // alpha plane -> byte 3 of every pixel
    let st = sf.stride as usize;
    for y in 0..h {
        let row = &mut sf.px[y * st..y * st + w * 4];
        for (p, &a) in row.chunks_exact_mut(4).zip(&plane[y * w..y * w + w]) {
            p[3] = a;
        }
    }
    Ok(sf)
}

/// decode an ALPH chunk payload into plane[w * h]: header byte (compression 0 raw / 1
/// VP8L, filter none / horizontal / vertical / gradient, pre-processing 0 / 1 (level
/// reduction: no decoder action, as libwebp without alpha dithering), reserved 0), then
/// the unfiltering of libwebp's filters.c (row 0 predicted from the left, column 0 of
/// later rows from above)  [webp_alpha]
pub fn alpha(a: &[u8], plane: &mut [u8], w: u32, h: u32, dec: Option<&ImgDec>) -> i32 {
    if a.len() <= 1 {
        return ERR_HDR;
    }
    let hd = a[0] as u32;
    let comp = hd & 3;
    if comp > 1 || hd & 0xE0 != 0 {
        return ERR_HDR; // compression, pre-processing <= 1, reserved 0
    }
    let n = w as usize * h as usize;
    if comp == 0 {
        if a.len() - 1 < n {
            return ERR_TRUNC;
        }
        plane[..n].copy_from_slice(&a[1..1 + n]);
    } else {
        let r = vp8l::decode_alpha(&a[1..], plane, w, h, dec);
        if r != 0 {
            return r;
        }
    }
    let f = (hd >> 2) & 3;
    if f != 0 {
        unfilter(&mut plane[..n], w as usize, h as usize, f);
    }
    0
}

// wp_unfilter: undo alpha filter 1 horizontal / 2 vertical / 3 gradient in place (libwebp
// HorizontalUnfilter / VerticalUnfilter / GradientUnfilter)
fn unfilter(p: &mut [u8], w: usize, h: usize, f: u32) {
    // row 0: horizontal from 0 for every filter
    let mut acc = 0u8;
    for v in p[..w].iter_mut() {
        acc = acc.wrapping_add(*v);
        *v = acc;
    }
    for y in 1..h {
        let (prev, cur) = p[(y - 1) * w..(y + 1) * w].split_at_mut(w);
        match f {
            1 => {
                // horizontal: pred = pixel above for column 0, then left
                let mut acc = prev[0];
                for v in cur.iter_mut() {
                    acc = acc.wrapping_add(*v);
                    *v = acc;
                }
            }
            2 => {
                for (v, &t) in cur.iter_mut().zip(prev.iter()) {
                    *v = v.wrapping_add(t);
                }
            }
            _ => {
                // gradient: top-left = left = top of column 0
                let mut tl = prev[0] as i32;
                let mut l = tl;
                for (v, &t) in cur.iter_mut().zip(prev.iter()) {
                    let t = t as i32;
                    let g = (l + t - tl).clamp(0, 255);
                    *v = v.wrapping_add(g as u8);
                    l = *v as i32;
                    tl = t;
                }
            }
        }
    }
}

// ------------------------------------------------------------------ encoder

/// the straight BGRA pixels of sf as u32 0xAARRGGBB (1..16383 per side; ERR_DIMS)
/// [wpe_load_argb]
pub fn load_argb(sf: &Surface) -> Result<(Vec<u32>, u32, u32), i32> {
    let (w, h) = (sf.w as u32, sf.h as u32);
    if w.wrapping_sub(1) >= 16383 || h.wrapping_sub(1) >= 16383 {
        return Err(ERR_DIMS);
    }
    let (wu, hu) = (w as usize, h as usize);
    let mut px = vec![0u32; wu * hu];
    let mut row = vec![0u8; wu * 4];
    for y in 0..hu {
        get_row(sf, y, &mut row, FMT_ARGB);
        for (d, s) in px[y * wu..(y + 1) * wu].iter_mut().zip(row.chunks_exact(4)) {
            *d = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
        }
    }
    Ok((px, w, h))
}

/// WEBPENC from an ImgOpt: effort, quality, preset, flags (IOF_WEBP_NOEXACT,
/// IOF_WEBP_SHARPYUV, IOF_WEBP_1THREAD), cancel flag  [wpe_cfg_from_opt]
pub fn cfg_from_opt<'a>(o: &ImgOpt<'a>) -> Cfg<'a> {
    let mut fl = 0;
    if o.flags & crate::IOF_WEBP_NOEXACT != 0 {
        fl |= WEF_NOEXACT;
    }
    if o.flags & crate::IOF_WEBP_SHARPYUV != 0 {
        fl |= crate::WEF_SHARPYUV;
    }
    if o.flags & crate::IOF_WEBP_1THREAD != 0 {
        fl |= crate::WEF_1THREAD;
    }
    Cfg { effort: o.effort, quality: o.quality, flags: fl, preset: o.ddsfmt, cancel: o.cancel }
}

/// WebP file of sf (any get_row format, 1..16383 pixels per side): IOF_LOSSLESS -> VP8L,
/// else VP8 (quality 0 = 95, IO_WEBPPRESET) with the alpha plane as a VP8L-compressed ALPH
/// chunk (filter chosen by estimate / trial).  VP8X with ICCP / EXIF (orientation reset to
/// 1) / XMP when IOF_EMBED_* and opt.meta hold them, or for lossy images with alpha;
/// otherwise a simple "VP8 " / "VP8L" file  [webp_encode]
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    match encode_body(sf, opt) {
        Ok(v) => {
            *out = v;
            0
        }
        Err(e) => e,
    }
}

fn encode_body(sf: &Surface, opt: &ImgOpt) -> Result<Vec<u8>, i32> {
    let (px, w, h) = load_argb(sf)?;
    let cfg = cfg_from_opt(opt);
    let lossless = opt.flags & IOF_LOSSLESS != 0;
    // alpha in use?
    let has_alpha = px.iter().any(|&p| p >> 24 != 255);
    // metadata to embed
    let mut flags = 0u32;
    let mut icc: &[u8] = &[];
    let mut xmp: &[u8] = &[];
    let mut exif: Vec<u8> = Vec::new();
    if let Some(m) = opt.meta {
        if opt.flags & IOF_EMBED_ICC != 0 && !m.icc.is_empty() {
            icc = &m.icc;
            flags |= 0x20;
        }
        if opt.flags & IOF_EMBED_XMP != 0 && !m.xmp.is_empty() {
            xmp = &m.xmp;
            flags |= 0x04;
        }
        if opt.flags & IOF_EMBED_EXIF != 0 && !m.exif.is_empty() {
            exif = exif_copy_orient1(&m.exif);
            flags |= 0x08;
        }
    }
    let img;
    let mut alph = Vec::new();
    if lossless {
        img = vp8l_enc::enc_argb(&px, w, h, &cfg)?;
        if has_alpha && flags != 0 {
            flags |= 0x10; // VP8X alpha flag for a VP8L image
        }
    } else {
        img = vp8_enc::enc_argb(&px, w, h, &cfg)?;
        if has_alpha {
            alph = enc_alpha(&px, w, h, &cfg)?;
            flags |= 0x10;
        }
    }
    // file size: 8 + the padded payload per chunk
    let mut total = if flags != 0 { 30usize } else { 12 };
    for n in [icc.len(), alph.len(), img.len(), exif.len(), xmp.len()] {
        if n != 0 {
            total = (total + n + 9) & !1;
        }
    }
    if (total as u64) >> 32 != 0 {
        return Err(ERR_LIMIT);
    }
    let mut f = Vec::with_capacity(total);
    f.extend_from_slice(&TAG_RIFF.to_le_bytes());
    f.extend_from_slice(&((total - 8) as u32).to_le_bytes());
    f.extend_from_slice(&TAG_WEBP.to_le_bytes());
    if flags != 0 {
        f.extend_from_slice(&TAG_VP8X.to_le_bytes());
        f.extend_from_slice(&10u32.to_le_bytes());
        f.extend_from_slice(&flags.to_le_bytes()); // flags + 3 reserved bytes
        let cw = (w - 1).to_le_bytes();
        f.extend_from_slice(&cw[..3]); // canvas width - 1 (24 bits)
        let chh = (h - 1).to_le_bytes();
        f.extend_from_slice(&chh[..3]);
    }
    let mut chunk = |t: u32, p: &[u8]| {
        if p.is_empty() {
            return;
        }
        f.extend_from_slice(&t.to_le_bytes());
        f.extend_from_slice(&(p.len() as u32).to_le_bytes());
        f.extend_from_slice(p);
        if p.len() & 1 != 0 {
            f.push(0);
        }
    };
    chunk(TAG_ICCP, icc);
    chunk(TAG_ALPH, &alph);
    chunk(if lossless { TAG_VP8L } else { TAG_VP8 }, &img);
    chunk(TAG_EXIF, &exif);
    chunk(TAG_XMP, xmp);
    Ok(f)
}

/// ALPH chunk payload of the alpha channel of px (u32 0xAARRGGBB): header byte (method 1
/// = VP8L, filter << 2) and a header-less VP8L stream of the filtered plane in the green
/// channel.  The filter is the one with the smallest residual entropy estimate (effort >=
/// 8: every filter is encoded and the smallest stream kept)  [webp_enc_alpha]
pub fn enc_alpha(px: &[u32], w: u32, h: u32, cfg: &Cfg) -> Result<Vec<u8>, i32> {
    let mut c = *cfg;
    c.flags |= WEF_NOHDR;
    c.flags &= !WEF_NOEXACT;
    let (wu, hu) = (w as usize, h as usize);
    let n = wu * hu;
    let plane: Vec<u8> = px[..n].iter().map(|&p| (p >> 24) as u8).collect();
    let mut argb = vec![0u32; n];
    // residual histograms of the four filters -> estimated best
    let est = estimate(&plane, wu, hu);
    let mut best: Option<Vec<u8>> = None;
    let mut bestf = est;
    let mut try_f = |f: u32, best: &mut Option<Vec<u8>>, bestf: &mut u32| -> Result<(), i32> {
        filter(&plane, &mut argb, wu, hu, f);
        let s = vp8l_enc::enc_argb(&argb, w, h, &c)?;
        match best {
            Some(b) if s.len() >= b.len() => {}
            _ => {
                *best = Some(s);
                *bestf = f;
            }
        }
        Ok(())
    };
    if c.effort >= 8 {
        for f in 0..4 {
            try_f(f, &mut best, &mut bestf)?;
        }
    } else {
        try_f(est, &mut best, &mut bestf)?;
    }
    let b = best.unwrap_or_default();
    let mut out = Vec::with_capacity(b.len() + 1);
    out.push((bestf << 2 | 1) as u8);
    out.extend_from_slice(&b);
    Ok(out)
}

// wa_filter: libwebp alpha filter (0 none, 1 horizontal, 2 vertical, 3 gradient; row 0
// from the left, column 0 of later rows from above) of the plane into the green channel
fn filter(p: &[u8], argb: &mut [u32], w: usize, h: usize, f: u32) {
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let v = p[i] as i32;
            let mut pred = 0i32;
            if f != 0 {
                if y == 0 {
                    if x != 0 {
                        pred = p[i - 1] as i32; // row 0: left
                    }
                } else if x == 0 {
                    pred = p[i - w] as i32; // column 0: above
                } else if f == 2 {
                    pred = p[i - w] as i32;
                } else {
                    pred = p[i - 1] as i32; // left
                    if f != 1 {
                        pred = (pred + p[i - w] as i32 - p[i - w - 1] as i32).clamp(0, 255);
                    }
                }
            }
            argb[i] = ((v - pred) as u8 as u32) << 8;
        }
    }
}

// wa_estimate: the filter 0..3 with the smallest residual entropy (sum of -c log2(c / n)
// approximated by n log2 n - sum c log2 c, integer log2 x 16)
fn estimate(p: &[u8], w: usize, h: usize) -> u32 {
    let mut hist = vec![0u32; 1024];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let c = p[i] as i32;
            hist[c as usize] += 1; // none
            let l = if x != 0 { p[i - 1] as i32 } else { 0 };
            let (t, tl) = if y != 0 { (p[i - w] as i32, if x != 0 { p[i - w - 1] as i32 } else { 0 }) } else { (0, 0) };
            // horizontal: left (column 0 of rows > 0: above)
            let ph = if x == 0 { t } else { l };
            hist[256 + (c - ph) as u8 as usize] += 1;
            // vertical: above (row 0: left)
            let pv = if y == 0 { l } else { t };
            hist[512 + (c - pv) as u8 as usize] += 1;
            // gradient
            let pg = if y == 0 {
                l
            } else if x == 0 {
                t
            } else {
                (l + t - tl).clamp(0, 255)
            };
            hist[768 + (c - pg) as u8 as usize] += 1;
        }
    }
    // cost = -sum c * log2(c) (the n log2 n term is common); log2 via bsr (x 16 + 4
    // fraction bits) is enough to rank the filters
    let mut best = 0u32;
    let mut bestc = u64::MAX;
    for f in 0..4u32 {
        let mut s = 0u64;
        for &c in &hist[f as usize * 256..f as usize * 256 + 256] {
            if c == 0 {
                continue;
            }
            let b = 31 - c.leading_zeros();
            let fr = if b < 4 { (c << (4 - b)) & 15 } else { (c >> (b - 4)) & 15 };
            let l = (b << 4) | fr; // log2(c) * 16
            s += l as u64 * c as u64;
        }
        // entropy ~ n log n - sum: larger sum = smaller entropy
        if bestc == u64::MAX || s > bestc {
            bestc = s;
            best = f;
        }
    }
    best
}
