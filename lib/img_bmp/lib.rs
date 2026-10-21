// img_bmp -- Windows bitmap (BMP) decoder and encoder, and the BMP parts of the core-format
// glue                                             [src/img/bmp.s, src/img/imgcore.s]
//
//   decode(data, sf) -> 0 / ERR_*                  bmp_decode: FMT_PARGB
//   decode_fl(data, sf, flags) -> 0 / ERR_*        bmp_decode_fl: JDF_STRAIGHT -> FMT_ARGB
//   encode(sf) -> Some(file) / None                bmp_encode (the classic rule)
//   encode_cfg(sf, cfg, out) -> 0 / ERR_*          bmp_encode_cfg (BmpCfg = BMPCFG)
//   meta(data, meta, flags) -> 0 / ERR_*           bmp_meta (imgcore.s)
//   probe(data, meta) -> recognised                bmp_probe
//   decode_ex(data, sf, dec, meta) -> 0 / ERR_*    bmp_decode_ex
//   encode_opt(sf, opt, out) -> 0 / ERR_*          bmp_encode_opt
//   CODEC                                          bmp_codec
//
// Decoder: BITMAPCOREHEADER (12) and BITMAPINFOHEADER..V5 (40..124) headers; 1/4/8-bit palette,
// 16-bit (default 5-5-5 or bit fields), 24-bit, 32-bit (BI_RGB = opaque, BI_BITFIELDS /
// BI_ALPHABITFIELDS with any masks and an optional alpha mask); bottom-up and top-down rows.
// Channels narrower than 8 bits are scaled with rounding (v * 255 / max), wider ones keep their
// top 8 bits.  Palette indices past the palette give opaque black.
// BI_RLE8 / BI_RLE4: encoded runs, absolute runs (word aligned), end of line, end of bitmap,
// delta; pixels the stream skips (delta, early end of line / bitmap, data that ends early) stay
// transparent; runs past the right edge are clipped, everything is bounds-checked.  BI_JPEG /
// BI_PNG: the embedded file (bfOffBits, biSizeImage bytes or up to the end of the data) is
// decoded by img_jpeg / img_png (the flags are passed on).  OS/2 2.x (64-byte header) Huffman
// 1D and RLE24, CMYK and other compressions: ERR_UNSUPPORTED.  Result: FMT_PARGB surface, or
// FMT_ARGB with JDF_STRAIGHT (the file's straight values: colour kept at alpha 0).
//
// Encoder (encode): 24-bit BI_RGB when the surface is opaque (XRGB, all-opaque PARGB / ARGB,
// A8 as grey), else 32-bit BITMAPV4HEADER with BI_BITFIELDS and an alpha mask (straight alpha).
// Rows bottom-up, 72 dpi.  encode_cfg with a BmpCfg: depth 0 auto (not opaque -> 32; opaque
// with <= 256 colours -> exact 8-bit palette; else 24), 32, 24 (colour composited over white),
// 8 (quantize_ex with qcfg's dither level, over white: BMP palettes have no alpha),
// BMPD_CLASSIC (encode's rule); ppmx / ppmy pixels per metre (0 = 2835, 72 dpi).
//
// Metadata (bmp_meta): header (size, depth, compression, alpha mask), pels per metre -> dpi,
// V4/V5 colour space (sRGB flag, embedded profile for IMGD_ICC); BI_JPEG / BI_PNG: the embedded
// file's own header (size, depth, model, channels, alpha / lossless / interlace / HDR flags).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_A8, FMT_ARGB, FMT_PARGB, FMT_XRGB, Surface};
use img_core::io::{core_decode, core_probe, dpi_ppm, ppm_dpi};
use img_core::meta::{Blob, all_opaque, get_row, meta_blob};
use img_core::quantize::{self, IQF_KEEPALPHA, IQF_WHITE};
use img_core::*;

const MAX_DIM: i32 = 32768;
const MAX_PIXELS: u64 = 1 << 28;
const HSIZES: [u32; 7] = [12, 40, 52, 56, 64, 108, 124]; // known info header sizes (bmp_meta)

pub const BMPD_CLASSIC: u32 = 0xFFFF_FFFF; // BmpCfg::depth: encode's rule

/// bmp_codec
pub static CODEC: Codec = Codec {
    probe: Some(probe),
    decode: Some(decode_ex),
    encode: Some(encode_opt),
    caps: IFC_DECODE | IFC_ENCODE,
    iom: BMP_IOM,
    scan: Some(meta),
    decode_fl: Some(decode_fl),
    save: Some(save),
};

// the classic encoder of img_save (bmp_encode)
fn save(sf: &Surface, _quality: u32) -> Option<Vec<u8>> {
    encode(sf)
}

/// encoder options (BMPCFG)
#[derive(Clone, Copy, Default)]
pub struct BmpCfg<'a> {
    pub depth: u32, // 0 auto / 32 / 24 / 8 / BMPD_CLASSIC
    pub qcfg: u32,  // quantize_ex cfg for 8 bit (IQF_WHITE is added)
    pub ppmx: u32,  // pixels per metre (0 = 2835, 72 dpi)
    pub ppmy: u32,
    pub cancel: Option<&'a AtomicU32>,
}

#[inline(always)]
fn u16_at(d: &[u8], o: usize) -> u32 {
    d[o] as u32 | (d[o + 1] as u32) << 8
}

#[inline(always)]
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

// ================================================================== decoder

// the decoder context (BD_*)
struct Bd {
    w: u32,
    h: u32,
    topdown: bool,
    bpp: u32,
    comp: u32,
    hsz: u32,
    off: usize,          // pixel data offset
    clrused: u32,
    palent: usize,       // bytes per palette entry (3 or 4)
    straight: bool,      // JDF_STRAIGHT
    mask: [u32; 4],      // R, G, B, A
    shift: [u32; 4],     // right shift per channel
    cmask: [u32; 4],     // value mask after the shift
    lut: [u32; 256],     // palette -> PARGB (opaque)
    clut: [[u8; 256]; 4], // channel scale tables R, G, B, A
}

/// bmp_decode: a BMP file as a new FMT_PARGB surface (sf is replaced; untouched on error)
pub fn decode(data: &[u8], sf: &mut Surface) -> i32 {
    decode_fl(data, sf, 0)
}

/// bmp_decode_fl: bmp_decode with JDF_* flags (JDF_STRAIGHT: FMT_ARGB output; passed on to an
/// embedded JPEG / PNG)
pub fn decode_fl(d: &[u8], sf: &mut Surface, flags: u32) -> i32 {
    let len = d.len();
    if len < 18 {
        return ERR_TRUNC;
    }
    if d[0] != b'B' || d[1] != b'M' {
        return ERR_HDR;
    }
    let mut c = Bd {
        w: 0,
        h: 0,
        topdown: false,
        bpp: 0,
        comp: 0,
        hsz: u32_at(d, 14),
        off: u32_at(d, 10) as usize,
        clrused: 0,
        palent: 4,
        straight: flags & JDF_STRAIGHT != 0,
        mask: [0; 4],
        shift: [0; 4],
        cmask: [0; 4],
        lut: [0; 256],
        clut: [[0; 256]; 4],
    };
    let (w, mut h): (i32, i32);
    if c.hsz == 12 {
        if len < 14 + 12 {
            return ERR_TRUNC;
        }
        w = u16_at(d, 18) as i32;
        h = u16_at(d, 20) as i32;
        c.bpp = u16_at(d, 24);
        c.palent = 3;
    } else {
        if c.hsz < 40 || c.hsz > 1024 {
            return ERR_HDR;
        }
        if len < 14 + 40 {
            return ERR_TRUNC;
        }
        w = u32_at(d, 18) as i32;
        h = u32_at(d, 22) as i32; // < 0 = top-down
        c.bpp = u16_at(d, 28);
        c.comp = u32_at(d, 30);
        c.clrused = u32_at(d, 46);
    }
    if w <= 0 || h == 0 {
        return ERR_DIMS;
    }
    if h < 0 {
        if h < -MAX_DIM {
            return ERR_DIMS;
        }
        h = -h;
        c.topdown = true;
    }
    if w > MAX_DIM || h > MAX_DIM || w as u64 * h as u64 > MAX_PIXELS {
        return ERR_DIMS;
    }
    c.w = w as u32;
    c.h = h as u32;
    // bits per pixel and compression
    let (bpp, comp) = (c.bpp, c.comp);
    match comp {
        4 if c.hsz == 64 => return ERR_UNSUPPORTED, // OS/2 RLE24
        4 | 5 => return decode_embedded(d, &c, sf, flags), // BI_JPEG / BI_PNG
        _ => {}
    }
    if bpp > 32 || (0x1_0101_0112u64 >> bpp) & 1 == 0 {
        return ERR_UNSUPPORTED; // valid: 1 4 8 16 24 32
    }
    match comp {
        0 => {}
        1 | 2 => {
            if bpp != if comp == 1 { 8 } else { 4 } {
                return ERR_UNSUPPORTED;
            }
            palette(d, &mut c);
            if c.off > len {
                return ERR_TRUNC;
            }
            let mut out = Surface::new(c.w as i32, c.h as i32, if c.straight { FMT_ARGB } else { FMT_PARGB });
            if out.px.is_empty() {
                return ERR_NOMEM;
            }
            rle(d, &c, &mut out);
            *sf = out;
            return 0;
        }
        3 if c.hsz == 64 => return ERR_UNSUPPORTED, // OS/2 Huffman 1D
        3 | 6 => {
            if bpp != 16 && bpp != 32 {
                return ERR_UNSUPPORTED;
            }
        }
        _ => return ERR_UNSUPPORTED,
    }
    // masks
    if comp == 0 {
        c.mask = if bpp == 16 { [0x7C00, 0x03E0, 0x001F, 0] } else { [0xFF_0000, 0xFF00, 0xFF, 0] };
    } else {
        // masks at file offset 54: inside V2+ headers, after a 40-byte header
        let need = if c.hsz >= 56 || (comp == 6 && c.hsz != 52) { 16 } else { 12 };
        if len < 54 + need {
            return ERR_TRUNC;
        }
        for k in 0..need / 4 {
            c.mask[k] = u32_at(d, 54 + k * 4);
        }
    }
    setup_masks(&mut c);
    if bpp <= 8 {
        palette(d, &mut c);
    }
    // rows: stride = ((w * bpp + 31) / 32) * 4, the last row may lack its padding
    let bits = c.w as u64 * bpp as u64;
    let stride = ((bits + 31) >> 5 << 2) as usize;
    let need = (c.h as usize - 1) * stride + bits.div_ceil(8) as usize;
    if c.off > len || need > len - c.off {
        return ERR_TRUNC;
    }
    let mut out = Surface::new(c.w as i32, c.h as i32, if c.straight { FMT_ARGB } else { FMT_PARGB });
    if out.px.is_empty() {
        return ERR_NOMEM;
    }
    let ostride = out.stride as usize;
    let rowb = c.w as usize * 4;
    for y in 0..c.h as usize {
        let src = &d[c.off + y * stride..];
        let dy = if c.topdown { y } else { c.h as usize - 1 - y };
        conv_row(&c, src, &mut out.px[dy * ostride..dy * ostride + rowb]);
    }
    *sf = out;
    0
}

// BI_JPEG / BI_PNG: the embedded file (bfOffBits, biSizeImage bytes or up to the end)
fn decode_embedded(d: &[u8], c: &Bd, sf: &mut Surface, flags: u32) -> i32 {
    if c.off > d.len() {
        return ERR_TRUNC;
    }
    let mut n = d.len() - c.off;
    if c.hsz >= 40 {
        let s = u32_at(d, 34) as usize; // biSizeImage (0 = up to the end)
        if s != 0 && s < n {
            n = s;
        }
    }
    let emb = &d[c.off..c.off + n];
    if c.comp == 5 {
        png_decode_fl(emb, sf, flags & JDF_STRAIGHT)
    } else {
        jpeg_decode_fl(emb, sf, flags & (JDF_STRAIGHT | JDF_NO_ORIENT))
    }
}

// png_decode_fl
fn png_decode_fl(data: &[u8], sf: &mut Surface, flags: u32) -> i32 {
    img_png::decode_fl(data, sf, flags)
}

// jpeg_decode_fl
fn jpeg_decode_fl(data: &[u8], sf: &mut Surface, flags: u32) -> i32 {
    match img_jpeg::decode_fl(data, flags) {
        Ok(s) => {
            *sf = s;
            0
        }
        Err(e) => e,
    }
}

// bmp_setup_masks: per channel shift, value mask and 8-bit scale table
fn setup_masks(c: &mut Bd) {
    for ch in 0..4 {
        let m = c.mask[ch];
        let (mut shift, mut bits) = (0u32, 0u32);
        if m != 0 {
            shift = m.trailing_zeros();
            bits = 31 - m.leading_zeros() - shift + 1;
            if bits > 8 {
                shift += bits - 8; // keep the top 8 bits
                bits = 8;
            }
        }
        c.shift[ch] = shift;
        let max = (1u32 << bits) - 1;
        c.cmask[ch] = max;
        // table: v -> round(v * 255 / max); no mask: colour 0, alpha 255
        let def = if ch == 3 { 255 } else { 0 };
        for (v, t) in c.clut[ch].iter_mut().enumerate() {
            let v = v as u32;
            *t = if max == 0 {
                def
            } else if v > max {
                255
            } else {
                ((v * 255 + (max >> 1)) / max) as u8
            };
        }
    }
}

// bmp_palette: the colour table into lut (opaque PARGB; entries past it opaque black)
fn palette(d: &[u8], c: &mut Bd) {
    let mut n = 1u32 << c.bpp;
    if c.clrused != 0 && c.clrused < n {
        n = c.clrused;
    }
    let at = c.hsz as usize + 14; // palette offset
    let avail = if at < d.len() { (d.len() - at) / c.palent } else { 0 };
    let n = (n as usize).min(avail);
    for (i, l) in c.lut.iter_mut().enumerate() {
        *l = if i < n {
            let e = &d[at + i * c.palent..];
            0xFF00_0000 | (e[2] as u32) << 16 | (e[1] as u32) << 8 | e[0] as u32
        } else {
            0xFF00_0000
        };
    }
}

// round(v * a / 255) as the decoder computes it
#[inline(always)]
fn mul255(v: u32, a: u32) -> u32 {
    let t = v * a + 128;
    (t + (t >> 8)) >> 8
}

// bmp_conv_row: one row to PARGB (or straight ARGB)
fn conv_row(c: &Bd, src: &[u8], dst: &mut [u8]) {
    let w = c.w as usize;
    match c.bpp {
        16 | 32 => {
            let step = (c.bpp / 8) as usize;
            for (o, s) in dst.chunks_exact_mut(4).zip(src.chunks(step)).take(w) {
                let v = if step == 2 { u16_at(s, 0) } else { u32_at(s, 0) };
                let ch = |k: usize| c.clut[k][((v >> c.shift[k]) & c.cmask[k]) as usize] as u32;
                let a = ch(3);
                let px = if a == 0 && !c.straight {
                    0
                } else {
                    let (mut r, mut g, mut b) = (ch(0), ch(1), ch(2));
                    if a != 255 && !c.straight {
                        r = mul255(r, a);
                        g = mul255(g, a);
                        b = mul255(b, a);
                    }
                    a << 24 | r << 16 | g << 8 | b
                };
                o.copy_from_slice(&px.to_le_bytes());
            }
        }
        24 => {
            for (o, s) in dst.chunks_exact_mut(4).zip(src.chunks(3)).take(w) {
                o.copy_from_slice(&[s[0], s[1], s[2], 255]);
            }
        }
        8 => {
            for (o, &i) in dst.chunks_exact_mut(4).zip(src).take(w) {
                o.copy_from_slice(&c.lut[i as usize].to_le_bytes());
            }
        }
        _ => {
            // 1 / 4 bits per pixel, the first pixel in the high bits
            let b = c.bpp;
            let m = (1u32 << b) - 1;
            for (x, o) in dst.chunks_exact_mut(4).take(w).enumerate() {
                let bit = x as u32 * b;
                let byte = src[(bit >> 3) as usize] as u32;
                let i = (byte >> (8 - b - (bit & 7))) & m;
                o.copy_from_slice(&c.lut[i as usize].to_le_bytes());
            }
        }
    }
}

// bmp_rle: BI_RLE8 / BI_RLE4 data (off .. end) into the zeroed surface: pixels through lut,
// skipped ones stay 0; x is kept <= w and y < h (file row order, bottom-up unless top-down);
// stops at end of bitmap, past the last row or when the data runs out
fn rle(d: &[u8], c: &Bd, sf: &mut Surface) {
    let (w, h) = (c.w, c.h);
    let rle8 = c.bpp == 8;
    let stride = sf.stride as usize;
    let mut pos = c.off;
    let (mut x, mut y) = (0u32, 0u32);
    let row_of = |y: u32| (if c.topdown { y } else { h - 1 - y }) as usize * stride;
    let put = |px: &mut [u8], y: u32, x: u32, i: u32| {
        let o = row_of(y) + x as usize * 4;
        px[o..o + 4].copy_from_slice(&c.lut[i as usize].to_le_bytes());
    };
    loop {
        if y >= h || d.len() - pos < 2 {
            return;
        }
        let (cnt, val) = (d[pos] as u32, d[pos + 1] as u32);
        pos += 2;
        if cnt != 0 {
            // encoded run: cnt pixels of index val (RLE4: high / low nibble alternating)
            for p in 0..cnt {
                if x >= w {
                    break; // clipped at the right edge
                }
                let i = if rle8 { val } else if p & 1 == 0 { val >> 4 } else { val & 15 };
                put(&mut sf.px, y, x, i);
                x += 1;
            }
            continue;
        }
        match val {
            0 => {
                x = 0; // end of line
                y += 1;
            }
            1 => return, // end of bitmap
            2 => {
                if d.len() - pos < 2 {
                    return;
                }
                x = (x + d[pos] as u32).min(w); // delta
                y += d[pos + 1] as u32;
                pos += 2;
            }
            _ => {
                // absolute run: val pixels in val (RLE8) / (val + 1) / 2 (RLE4) bytes + pad to 16 bits
                let mut npx = val;
                let mut nb = if rle8 { val as usize } else { (val as usize + 1) / 2 };
                let left = d.len() - pos;
                if left < nb {
                    nb = left; // data ends inside the run
                    npx = if rle8 { left as u32 } else { left as u32 * 2 };
                }
                for p in 0..npx {
                    if x >= w {
                        break;
                    }
                    let i = if rle8 {
                        d[pos + p as usize] as u32
                    } else {
                        let b = d[pos + (p >> 1) as usize] as u32;
                        if p & 1 == 0 { b >> 4 } else { b & 15 }
                    };
                    put(&mut sf.px, y, x, i);
                    x += 1;
                }
                nb += nb & 1;
                pos += nb.min(d.len() - pos);
            }
        }
    }
}

// ================================================================== encoder

/// bmp_encode: BMP file, 24-bit when opaque, else 32-bit BITMAPV4HEADER with alpha (straight)
pub fn encode(sf: &Surface) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let cfg = BmpCfg { depth: BMPD_CLASSIC, ..Default::default() };
    if encode_cfg(sf, &cfg, &mut out) == 0 { Some(out) } else { None }
}

/// bmp_encode_cfg: encode sf (FMT_XRGB, FMT_PARGB, FMT_ARGB or FMT_A8) with a BmpCfg (see the
/// module header); out is empty on error.  Errors: ERR_DIMS, ERR_UNSUPPORTED (surface format),
/// ERR_NOMEM (8-bit quantization failed), ERR_CANCEL
pub fn encode_cfg(sf: &Surface, cfg: &BmpCfg, out: &mut Vec<u8>) -> i32 {
    out.clear();
    if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() {
        return ERR_DIMS;
    }
    if sf.fmt > FMT_ARGB {
        return ERR_UNSUPPORTED;
    }
    let mut pal = [0u32; 256];
    let mut idx: Vec<u8> = Vec::new();
    let mut ncol = 0usize;
    let bpp: u32 = match cfg.depth {
        32 => 32,
        24 => 24,
        8 => match quantize::quantize_alloc(sf, 256, (cfg.qcfg & !IQF_KEEPALPHA) | IQF_WHITE, &mut pal) {
            Some((n, _, i)) => {
                (ncol, idx) = (n as usize, i);
                8
            }
            None => return ERR_NOMEM,
        },
        depth => {
            if sf.fmt != FMT_XRGB && sf.fmt != FMT_A8 && !all_opaque(sf) {
                32
            } else if depth == BMPD_CLASSIC {
                24
            } else {
                // auto, opaque: an exact palette when there are <= 256 colours
                match quantize::quantize_alloc(sf, 256, IQF_KEEPALPHA, &mut pal) {
                    Some((n, _, i)) => {
                        (ncol, idx) = (n as usize, i);
                        8
                    }
                    None => 24,
                }
            }
        }
    };
    let hsz: usize = if bpp == 32 { 108 } else { 40 };
    let (w, h) = (sf.w as usize, sf.h as usize);
    // stride = ((w * bpp + 31) / 32) * 4, file = 14 + header + palette + rows
    let stride = (w * bpp as usize + 31) >> 5 << 2;
    let img = stride * h;
    let off = 14 + hsz + ncol * 4;
    let total = img + off;
    let mut f = vec![0u8; total];
    let put = |f: &mut [u8], o: usize, v: u32| f[o..o + 4].copy_from_slice(&v.to_le_bytes());
    f[0] = b'B';
    f[1] = b'M';
    put(&mut f, 2, total as u32);
    put(&mut f, 10, off as u32); // pixel data offset
    put(&mut f, 14, hsz as u32);
    put(&mut f, 18, sf.w as u32);
    put(&mut f, 22, sf.h as u32); // positive: bottom-up
    f[26] = 1;
    f[28] = bpp as u8;
    put(&mut f, 34, img as u32);
    put(&mut f, 38, if cfg.ppmx == 0 { 2835 } else { cfg.ppmx }); // 72 dpi
    put(&mut f, 42, if cfg.ppmy == 0 { 2835 } else { cfg.ppmy });
    put(&mut f, 46, ncol as u32); // colours used (8 bit)
    if bpp == 32 {
        put(&mut f, 30, 3); // BI_BITFIELDS
        put(&mut f, 54, 0x00FF_0000);
        put(&mut f, 58, 0x0000_FF00);
        put(&mut f, 62, 0x0000_00FF);
        put(&mut f, 66, 0xFF00_0000);
        put(&mut f, 70, 0x7352_4742); // LCS_sRGB
    }
    // colour table: B G R 0
    for (i, &c) in pal.iter().enumerate().take(ncol) {
        put(&mut f, 14 + hsz + i * 4, c & 0x00FF_FFFF);
    }
    // rows bottom-up
    let mut row = vec![0u8; w * 4];
    let mut o = off;
    for y in (0..h).rev() {
        if cfg.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0) {
            return ERR_CANCEL;
        }
        match bpp {
            8 => f[o..o + w].copy_from_slice(&idx[y * w..y * w + w]),
            24 => {
                get_row(sf, y, &mut row, FMT_PARGB);
                for (d, s) in f[o..o + w * 3].chunks_exact_mut(3).zip(row.chunks_exact(4)) {
                    // premultiplied over white: c + (255 - a) per byte, as one 32-bit add
                    let c = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    let v = c.wrapping_add((s[3] ^ 255) as u32 * 0x010101).to_le_bytes();
                    d.copy_from_slice(&v[..3]);
                }
            }
            _ => get_row(sf, y, &mut f[o..o + w * 4], FMT_ARGB),
        }
        o += stride;
    }
    *out = f;
    0
}

// ================================================================== metadata, IMGCODEC glue

/// bmp_meta: file and info header -> 0 / ERR_TRUNC / ERR_HDR (see the module header)
pub fn meta(d: &[u8], m: &mut ImgMeta, flags: u32) -> i32 {
    let len = d.len();
    if len < 18 {
        return ERR_TRUNC;
    }
    if d[0] != b'B' || d[1] != b'M' {
        return ERR_HDR;
    }
    let hsz = u32_at(d, 14);
    if !HSIZES.contains(&hsz) {
        return ERR_HDR;
    }
    m.frames = 1;
    let (bpp, comp);
    if hsz == 12 {
        if len < 26 {
            return ERR_TRUNC; // BITMAPCOREHEADER
        }
        m.w = u16_at(d, 18);
        m.h = u16_at(d, 20);
        bpp = u16_at(d, 24);
        comp = 0;
    } else {
        if len < 54 {
            return ERR_TRUNC;
        }
        let w = u32_at(d, 18) as i32;
        if w <= 0 {
            return ERR_HDR;
        }
        m.w = w as u32;
        m.h = (u32_at(d, 22) as i32).wrapping_abs() as u32; // top-down: negative height
        bpp = u16_at(d, 28);
        comp = u32_at(d, 30);
        let px = u32_at(d, 38) as i32; // pels per metre
        if px > 0 {
            m.dpix = ppm_dpi(px as u32);
            m.resunit = IMR_CM;
        }
        let py = u32_at(d, 42) as i32;
        if py > 0 {
            m.dpiy = ppm_dpi(py as u32);
        }
    }
    m.bits = bpp as u8;
    m.model = IMM_PAL;
    m.channels = 1;
    if (comp == 4 || comp == 5) && hsz != 64 {
        embed_meta(d, m, comp); // embedded file: its own header decides
    } else if comp == 4 {
        m.model = IMM_YCC; // OS/2 RLE24
        m.channels = 3;
    } else {
        m.flags |= IMF_LOSSLESS;
        if bpp > 8 {
            m.model = IMM_RGB;
            m.channels = 3;
            m.bits = if bpp == 16 { 5 } else { 8 };
            // alpha mask: BI_BITFIELDS with a V3+ header or BI_ALPHABITFIELDS
            if (bpp == 16 || bpp == 32) && (comp == 6 || (comp == 3 && hsz >= 56)) && len >= 70 && u32_at(d, 66) != 0 {
                m.flags |= IMF_ALPHA;
                m.channels = 4;
            }
        }
    }
    // V4 / V5 colour space
    if hsz >= 108 && len >= 74 {
        match u32_at(d, 70) {
            0x7352_4742 | 0x5769_6E20 => m.flags |= IMF_SRGB, // 'sRGB', 'Win '
            0x4D42_4544 if hsz >= 124 && flags & IMGD_ICC != 0 && len >= 138 => {
                // 'MBED': profile offset from the info header, size
                let off = u32_at(d, 126) as usize + 14;
                let n = u32_at(d, 130) as usize;
                if off <= len && n <= len - off {
                    meta_blob(m, Blob::Icc, &d[off..off + n]);
                }
            }
            _ => {}
        }
    }
    0
}

// bmp_embed_meta: BI_JPEG (4) / BI_PNG (5): scan the embedded file (bfOffBits, biSizeImage bytes
// or up to the end) and take its size, depth, model, channels and alpha / lossless / interlace /
// HDR flags, so that img_decode_ex checks the real size
fn embed_meta(d: &[u8], m: &mut ImgMeta, comp: u32) {
    if d.len() < 54 {
        return;
    }
    let off = u32_at(d, 10) as usize;
    if off > d.len() {
        return;
    }
    let mut n = d.len() - off;
    let s = u32_at(d, 34) as usize;
    if s != 0 && s < n {
        n = s;
    }
    let mut e = ImgMeta::default();
    let ok = if comp == 5 { png_meta(&d[off..off + n], &mut e) } else { jpeg_meta(&d[off..off + n], &mut e) };
    if !ok {
        return;
    }
    m.w = e.w;
    m.h = e.h;
    m.bits = e.bits;
    m.model = e.model;
    m.channels = e.channels;
    m.flags |= e.flags & (IMF_ALPHA | IMF_LOSSLESS | IMF_INTERLACED | IMF_HDR);
}

// png_meta with flags 0
fn png_meta(d: &[u8], m: &mut ImgMeta) -> bool {
    img_png::glue::scan(d, m, 0) == 0
}

// jpeg_meta with flags 0
fn jpeg_meta(d: &[u8], m: &mut ImgMeta) -> bool {
    img_jpeg::meta(d, m, 0) == 0
}

/// bmp_probe: header + metadata scan, no pixels
pub fn probe(data: &[u8], m: &mut ImgMeta) -> bool {
    core_probe(data, m, meta)
}

/// bmp_decode_ex: BMP through decode_fl (core_decode: the metadata scan, IMGD_PROBE, frame 0
/// only, the size check, the cancel flag)
pub fn decode_ex(data: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    core_decode(data, sf, dec, m, meta, decode_fl)
}

/// bmp_encode_opt: encode_cfg with ImgOpt::bitdepth (0 / 32 / 24 / 8), ImgOpt::dither (8 bit)
/// and the pixels per metre of ImgOpt::meta's dpi (none: 72 dpi)
pub fn encode_opt(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let mut cfg = BmpCfg {
        depth: if matches!(opt.bitdepth, 8 | 24 | 32) { opt.bitdepth } else { 0 },
        qcfg: quantize::opt_qcfg(opt),
        cancel: opt.cancel,
        ..Default::default()
    };
    if let Some(m) = opt.meta {
        cfg.ppmx = dpi_ppm(m.dpix);
        cfg.ppmy = dpi_ppm(m.dpiy);
    }
    encode_cfg(sf, &cfg, out)
}
