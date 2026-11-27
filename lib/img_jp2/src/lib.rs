// img_jp2 -- JPEG 2000 decoder (ITU-T T.800 | ISO/IEC 15444-1, the JP2 file format and the parts
// of JPX that JP2 readers meet)
//
//   decode(data, &Options) -> Result<Image, Error>   a JP2 / JPX file or a raw codestream (J2K, J2C)
//   probe(data) -> bool                         the JP2 signature or the start of a codestream
//   Image { width, height, components, color, alpha, premultiplied }
//   Component { width, height, dx, dy, precision, signed, data }   samples, row by row
//   CODEC                                       img_core's codec (8-bit ARGB surfaces) for img_all;
//                                               CIELab files are ERR_UNSUPPORTED there
//
// Modules: boxes (the JP2 boxes), codestream (the markers), tile (geometry, packets, progression,
// dequantisation, assembly), t1 (the code-blocks), mq (the arithmetic decoder), dwt (the wavelets).
//
// What it decodes: every progression order and POC, tiles and tile-parts in any order, precincts,
// layers, SOP / EPH, PPM / PPT, every code-block style (bypass, reset, terminate-all, vertically
// causal, predictable termination, segmentation symbols), ROI max-shift, the 5/3 and 9/7 wavelets
// with any image and tile offsets, RCT and ICT, sub-sampled components, precision 1 to 24 bits,
// signed and unsigned.  From the JP2 header: the colour space (enumerated sRGB, greyscale, sYCC,
// CMYK, CIELab; an ICC profile), the palette with its component mapping (applied here) and the
// channel definitions (colour channels in their order, the opacity channel).  sYCC is converted to
// RGB here (unless `Options::convert_sycc` is false).  `Options::reduce` decodes a lower
// resolution (each level halves the size).
//
// Not here: Part 2 (other wavelets, multiple-component transforms beyond RCT / ICT, trellis
// quantisation), precision above 24 bits.  A file that is cut short gives what was decoded up to
// there (like the common readers); limits (`Options::max_pixels` also for the channels a palette
// makes, tiles times components, code-blocks, precincts, the packets the progression looks at)
// refuse hostile files before they cost memory or time.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod boxes;
mod codestream;
mod dwt;
mod mq;
mod t1;
mod tile;

use alloc::vec;
use alloc::vec::Vec;
use gfx::{Surface, FMT_ARGB};
use img_core::meta::surf_alloc;
use img_core::*;

/// why a file could not be decoded
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// neither a JP2 file nor a codestream
    NotJpeg2000,
    /// the headers end early
    Truncated,
    /// a header that breaks the standard
    Corrupt,
    /// valid, but not decoded here (Part 2, more than 24 bits)
    Unsupported,
    /// larger than the limits allow
    Limit,
}

/// what to decode
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// resolution levels to leave out (each halves width and height)
    pub reduce: u8,
    /// the largest width * height (the default: img_core's IMG_MAXPX_DEF)
    pub max_pixels: u64,
    /// convert sYCC to sRGB (the default); false leaves the components as they are coded, with
    /// `Color::Unknown` (a reader that takes the colour space from elsewhere, like PDF)
    pub convert_sycc: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options { reduce: 0, max_pixels: IMG_MAXPX_DEF, convert_sycc: true }
    }
}

/// the colour space the JP2 header names (Unknown for a raw codestream)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Color {
    Unknown,
    Gray,
    /// sRGB (also sYCC after the conversion here)
    Srgb,
    Cmyk,
    /// CIELab (the enumerated space of JPX)
    Lab,
    /// an ICC profile
    Icc(Vec<u8>),
}

/// one component (channel) of the image
#[derive(Clone, Debug)]
pub struct Component {
    pub width: u32,
    pub height: u32,
    /// its sub-sampling on the image grid
    pub dx: u32,
    pub dy: u32,
    pub precision: u8,
    pub signed: bool,
    /// width * height samples, row by row (unsigned: 0 .. 2^precision - 1)
    pub data: Vec<i32>,
}

/// a decoded image: the colour channels first, in their order, then the opacity channel
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub components: Vec<Component>,
    pub color: Color,
    /// the index of the opacity channel
    pub alpha: Option<usize>,
    /// the colour channels are premultiplied by the opacity
    pub premultiplied: bool,
}

/// a JP2 file or a raw codestream
pub fn probe(d: &[u8]) -> bool {
    d.starts_with(&boxes::SIGNATURE) || d.starts_with(&[0xFF, 0x4F, 0xFF, 0x51])
}

/// decode a JP2 / JPX file or a raw codestream
pub fn decode(d: &[u8], opt: &Options) -> Result<Image, Error> {
    let (cs_bytes, jp2) = if d.starts_with(&boxes::SIGNATURE) {
        let (c, j) = boxes::parse(d)?;
        (c, Some(j))
    } else if d.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        (d, None)
    } else {
        return Err(Error::NotJpeg2000);
    };
    let cs = codestream::parse(cs_bytes)?;
    let siz = &cs.siz;
    if siz.comps.iter().any(|c| c.prec > 24) {
        return Err(Error::Unsupported);
    }
    let area = (siz.x1 - siz.x0) as u64 * (siz.y1 - siz.y0) as u64;
    let samples: u64 = siz
        .comps
        .iter()
        .map(|c| (codestream::div_ceil(siz.x1, c.dx) - codestream::div_ceil(siz.x0, c.dx)) as u64 * (codestream::div_ceil(siz.y1, c.dy) - codestream::div_ceil(siz.y0, c.dy)) as u64)
        .sum();
    if area > opt.max_pixels || samples > opt.max_pixels.saturating_mul(4) {
        return Err(Error::Limit);
    }
    let red = opt.reduce.min(32) as u32;
    let mut out: Vec<tile::OutComp> = siz
        .comps
        .iter()
        .map(|c| {
            let (x0, y0) = (tile::ceil_shift(codestream::div_ceil(siz.x0, c.dx), red), tile::ceil_shift(codestream::div_ceil(siz.y0, c.dy), red));
            let (x1, y1) = (tile::ceil_shift(codestream::div_ceil(siz.x1, c.dx), red), tile::ceil_shift(codestream::div_ceil(siz.y1, c.dy), red));
            tile::OutComp { w: x1 - x0, h: y1 - y0, x0, y0, data: vec![0; (x1 - x0) as usize * (y1 - y0) as usize] }
        })
        .collect();
    let mut budget = tile::Budget::default();
    let mut t1 = t1::T1::new();
    for t in 0..cs.tiles.len() {
        tile::decode(&cs, t, red, &mut budget, &mut out, &mut t1)?;
    }
    let mut comps: Vec<Component> = out
        .into_iter()
        .zip(siz.comps.iter())
        .map(|(o, c)| Component { width: o.w, height: o.h, dx: c.dx, dy: c.dy, precision: c.prec, signed: c.signed, data: o.data })
        .collect();
    let width = tile::ceil_shift(siz.x1, red) - tile::ceil_shift(siz.x0, red);
    let height = tile::ceil_shift(siz.y1, red) - tile::ceil_shift(siz.y0, red);
    let mut img = Image { width, height, components: Vec::new(), color: Color::Unknown, alpha: None, premultiplied: false };
    let Some(jp2) = jp2 else {
        img.components = comps;
        return Ok(img);
    };
    // the palette: each channel of the component mapping, directly or through a column
    if let Some(pal) = &jp2.palette {
        if !jp2.cmap.is_empty() {
            // each channel is a copy as large as its component: the same budget as the samples
            let total: u64 = jp2.cmap.iter().map(|e| comps.get(e.0 as usize).map_or(0, |c| c.data.len() as u64)).sum();
            if total > opt.max_pixels.saturating_mul(4) {
                return Err(Error::Limit);
            }
            let mut mapped = Vec::with_capacity(jp2.cmap.len());
            for &(cmp, through, col) in &jp2.cmap {
                let src = comps.get(cmp as usize).ok_or(Error::Corrupt)?;
                if through {
                    let entries = pal.entries.get(col as usize).ok_or(Error::Corrupt)?;
                    let (depth, signed) = pal.cols[col as usize];
                    let last = entries.len() as i32 - 1;
                    let data = src.data.iter().map(|&v| entries[v.clamp(0, last) as usize]).collect();
                    mapped.push(Component { precision: depth.min(24), signed, data, ..src.clone() });
                } else {
                    mapped.push(src.clone());
                }
            }
            comps = mapped;
        }
    }
    // the channel definitions: colour channels by their association, then the opacity
    if !jp2.cdef.is_empty() {
        let mut order: Vec<usize> = Vec::new();
        let mut colour: Vec<(u16, usize)> = jp2.cdef.iter().filter(|e| e.1 == 0 && e.2 != 0 && e.2 != 0xFFFF).map(|e| (e.2, e.0 as usize)).collect();
        colour.sort();
        for &(_, ch) in &colour {
            if ch < comps.len() && !order.contains(&ch) {
                order.push(ch);
            }
        }
        if let Some(a) = jp2.cdef.iter().find(|e| (e.1 == 1 || e.1 == 2) && (e.0 as usize) < comps.len()) {
            if !order.contains(&(a.0 as usize)) {
                img.alpha = Some(order.len());
                img.premultiplied = a.1 == 2;
                order.push(a.0 as usize);
            }
        }
        for ch in 0..comps.len() {
            if !order.contains(&ch) {
                order.push(ch);
            }
        }
        let mut taken: Vec<Option<Component>> = comps.into_iter().map(Some).collect();
        comps = order.iter().filter_map(|&i| taken[i].take()).collect();
    }
    img.color = match jp2.colr {
        Some(boxes::Colr::Enum(16)) => Color::Srgb,
        Some(boxes::Colr::Enum(17)) => Color::Gray,
        Some(boxes::Colr::Enum(12)) => Color::Cmyk,
        Some(boxes::Colr::Enum(14)) => Color::Lab,
        Some(boxes::Colr::Enum(18)) => {
            if comps.len() >= 3 && opt.convert_sycc {
                sycc_to_rgb(&mut comps);
                Color::Srgb
            } else {
                Color::Unknown
            }
        }
        Some(boxes::Colr::Icc(p)) => Color::Icc(p),
        _ => Color::Unknown,
    };
    img.components = comps;
    Ok(img)
}

/// sYCC (the first three components) to sRGB, the chroma brought to the size of the luma
fn sycc_to_rgb(c: &mut [Component]) {
    let (w, h) = (c[0].width as usize, c[0].height as usize);
    let up = |s: &Component| -> Vec<i32> {
        if s.width as usize == w && s.height as usize == h {
            return s.data.clone();
        }
        let mut v = Vec::with_capacity(w * h);
        for y in 0..h {
            let sy = (y as u64 * s.height as u64 / h.max(1) as u64).min(s.height as u64 - 1) as usize;
            for x in 0..w {
                let sx = (x as u64 * s.width as u64 / w.max(1) as u64).min(s.width as u64 - 1) as usize;
                v.push(s.data[sy * s.width as usize + sx]);
            }
        }
        v
    };
    if c[1].width == 0 || c[2].width == 0 || c[1].height == 0 || c[2].height == 0 {
        return;
    }
    let (cb, cr) = (up(&c[1]), up(&c[2]));
    let prec = c[0].precision.min(24) as u32;
    let off = if c[1].signed { 0 } else { 1 << (prec - 1) } as f32;
    let max = ((1i64 << prec) - 1) as f32;
    let mut g = Vec::with_capacity(w * h);
    let mut b = Vec::with_capacity(w * h);
    for i in 0..w * h {
        let y = c[0].data[i] as f32;
        let (u, v) = (cb[i] as f32 - off, cr[i] as f32 - off);
        let r = y + 1.402 * v;
        let gg = y - 0.344_136 * u - 0.714_136 * v;
        let bb = y + 1.772 * u;
        let q = |x: f32| (x + 0.5).clamp(0.0, max) as i32;
        c[0].data[i] = q(r);
        g.push(q(gg));
        b.push(q(bb));
    }
    for k in 1..3 {
        c[k].width = w as u32;
        c[k].height = h as u32;
        c[k].dx = c[0].dx;
        c[k].dy = c[0].dy;
        c[k].precision = c[0].precision;
        c[k].signed = false;
    }
    c[1].data = g;
    c[2].data = b;
}

// ------------------------------------------------------------------ the codec of img_core

/// jp2_codec: JP2, JPX and raw codestreams as 8-bit ARGB
pub static CODEC: Codec = Codec { probe: Some(codec_probe), decode: Some(codec_decode), caps: IFC_DECODE, ..Codec::EMPTY };

fn codec_probe(d: &[u8], m: &mut ImgMeta) -> bool {
    if !probe(d) {
        return false;
    }
    // the size from SIZ
    let cs = if d.starts_with(&boxes::SIGNATURE) { boxes::parse(d).map(|x| x.0).ok() } else { Some(d) };
    if let Some(s) = cs.filter(|s| s.len() >= 42) {
        let be = |o: usize| u32::from_be_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]);
        let (x1, y1, x0, y0) = (be(8), be(12), be(16), be(20));
        m.w = x1.saturating_sub(x0);
        m.h = y1.saturating_sub(y0);
        m.channels = u16::from_be_bytes([s[40], s[41]]).min(255) as u8;
        m.bits = s.get(42).map_or(8, |b| (b & 0x7F) + 1);
    }
    true
}

fn codec_decode(d: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    let img = match decode(d, &Options { max_pixels: dec.max_pixels(), ..Options::default() }) {
        Ok(i) => i,
        Err(Error::NotJpeg2000) | Err(Error::Corrupt) => return ERR_HDR,
        Err(Error::Truncated) => return ERR_TRUNC,
        Err(Error::Unsupported) => return ERR_UNSUPPORTED,
        Err(Error::Limit) => return ERR_LIMIT,
    };
    if img.color == Color::Lab {
        // no conversion of CIELab (with the parameters of its colr box) to RGB here
        return ERR_UNSUPPORTED;
    }
    let (w, h) = (img.width, img.height);
    *sf = match surf_alloc(w, h, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let n = img.components.len();
    let ncol = if let Some(a) = img.alpha { a } else { n };
    let model = match (&img.color, ncol) {
        (Color::Cmyk, _) if ncol >= 4 => IMM_CMYK,
        (_, 1 | 2) => IMM_GRAY,
        _ => IMM_RGB,
    };
    m.w = w;
    m.h = h;
    m.channels = n.min(255) as u8;
    m.bits = img.components.first().map_or(8, |c| c.precision);
    m.model = model;
    if img.alpha.is_some() {
        m.flags |= IMF_ALPHA;
    }
    if dec.flags & IMGD_ICC != 0 {
        if let Color::Icc(p) = &img.color {
            m.icc = p.clone();
        }
    }
    // a sample of component c at image pixel (x, y), as 0..255
    let at = |c: &Component, x: u32, y: u32| -> u32 {
        if c.width == 0 || c.height == 0 {
            return 0;
        }
        let sx = (x / c.dx.max(1)).min(c.width - 1);
        let sy = (y / c.dy.max(1)).min(c.height - 1);
        let v = c.data[sy as usize * c.width as usize + sx as usize] as i64;
        let p = c.precision.clamp(1, 24) as u32;
        let v = if c.signed { v + (1i64 << (p - 1)) } else { v };
        let max = (1i64 << p) - 1;
        ((v.clamp(0, max) * 255 + max / 2) / max) as u32
    };
    let stride = sf.stride as usize;
    for y in 0..h {
        let row = &mut sf.px[y as usize * stride..][..w as usize * 4];
        for x in 0..w {
            let ch = |i: usize| img.components.get(i).map_or(0, |c| at(c, x, y));
            let (r, g, b) = match model {
                IMM_CMYK => {
                    let k = 255 - ch(3);
                    ((255 - ch(0)) * k / 255, (255 - ch(1)) * k / 255, (255 - ch(2)) * k / 255)
                }
                IMM_GRAY => (ch(0), ch(0), ch(0)),
                _ => (ch(0), ch(1), ch(2)),
            };
            let a = img.alpha.map_or(255, &ch);
            let (r, g, b) = if img.premultiplied && a > 0 { ((r * 255 / a).min(255), (g * 255 / a).min(255), (b * 255 / a).min(255)) } else { (r, g, b) };
            row[x as usize * 4..][..4].copy_from_slice(&[b as u8, g as u8, r as u8, a as u8]);
        }
    }
    0
}
