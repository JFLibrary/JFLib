// image -- raster images for a document: JPEG files as they are (DCTDecode), all other pixels
// FlateDecode with PNG predictors and their alpha as a soft mask (SMask)

use alloc::vec::Vec;

use crate::Error;

#[derive(Clone, Debug)]
pub(crate) enum Pixels {
    /// a JPEG file: its components (1 gray, 3 YCbCr/RGB, 4 CMYK/YCCK), Adobe's inverted CMYK,
    /// the colour transform the decoder must apply (None: DCTDecode's default)
    Jpeg { data: Vec<u8>, comps: u8, invert: bool, transform: Option<u8> },
    /// 8-bit samples, 1 (gray) or 3 (RGB) per pixel, rows from the top
    Raw { colors: u8, data: Vec<u8> },
}

/// an image for Document::add_image: a JPEG passed through, or 8-bit pixels (gray, RGB, RGBA,
/// a gfx surface) with an optional alpha channel
#[derive(Clone, Debug)]
pub struct Image {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) px: Pixels,
    pub(crate) alpha: Option<Vec<u8>>,
    pub(crate) interpolate: bool,
}

/// the largest side PDF readers are asked to take
const MAX_SIDE: u32 = 1 << 16;

fn check_size(w: u32, h: u32, n: usize, len: usize) -> Result<(), Error> {
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(Error::Image("a side of 1 to 65536 pixels"));
    }
    if (w as usize).checked_mul(h as usize).and_then(|p| p.checked_mul(n)) != Some(len) {
        return Err(Error::Image("pixel data of width x height x components bytes"));
    }
    Ok(())
}

impl Image {
    /// a JPEG file, embedded unchanged: baseline, extended or progressive, 8 bits, 1, 3 or 4
    /// components (Adobe's inverted CMYK understood)
    pub fn jpeg(data: Vec<u8>) -> Result<Image, Error> {
        let j = parse_jpeg(&data)?;
        Ok(Image { w: j.w, h: j.h, px: Pixels::Jpeg { data, comps: j.comps, invert: j.invert, transform: j.transform }, alpha: None, interpolate: false })
    }

    /// 8-bit gray, rows from the top
    pub fn gray(w: u32, h: u32, px: &[u8]) -> Result<Image, Error> {
        check_size(w, h, 1, px.len())?;
        Ok(Image { w, h, px: Pixels::Raw { colors: 1, data: px.to_vec() }, alpha: None, interpolate: false })
    }

    /// 8-bit RGB, rows from the top
    pub fn rgb(w: u32, h: u32, px: &[u8]) -> Result<Image, Error> {
        check_size(w, h, 3, px.len())?;
        Ok(Image::raw(w, h, px.to_vec(), None))
    }

    /// 8-bit RGBA with straight (not premultiplied) alpha, rows from the top
    pub fn rgba(w: u32, h: u32, px: &[u8]) -> Result<Image, Error> {
        check_size(w, h, 4, px.len())?;
        let n = w as usize * h as usize;
        let mut rgb = Vec::with_capacity(n * 3);
        let mut alpha = Vec::with_capacity(n);
        for p in px.chunks_exact(4) {
            rgb.extend_from_slice(&p[..3]);
            alpha.push(p[3]);
        }
        Ok(Image::raw(w, h, rgb, Some(alpha)))
    }

    /// a gfx surface: XRGB (opaque), PARGB (premultiplied, un-premultiplied here), ARGB, A8
    /// (as gray)
    pub fn from_surface(s: &gfx::Surface) -> Result<Image, Error> {
        if s.w <= 0 || s.h <= 0 || s.stride < 0 {
            return Err(Error::Image("an empty surface"));
        }
        let (w, h, stride) = (s.w as usize, s.h as usize, s.stride as usize);
        let bpp = if s.fmt == gfx::FMT_A8 { 1 } else { 4 };
        if stride < w * bpp || s.px.len() < stride * (h - 1) + w * bpp {
            return Err(Error::Image("a surface smaller than its size"));
        }
        if s.fmt == gfx::FMT_A8 {
            let mut g = Vec::with_capacity(w * h);
            for y in 0..h {
                g.extend_from_slice(&s.px[y * stride..y * stride + w]);
            }
            return Image::gray(w as u32, h as u32, &g);
        }
        let mut rgb = Vec::with_capacity(w * h * 3);
        let mut alpha = Vec::with_capacity(w * h);
        for y in 0..h {
            for p in s.px[y * stride..y * stride + w * 4].chunks_exact(4) {
                // bytes B, G, R, A
                let a = if s.fmt == gfx::FMT_XRGB { 255 } else { p[3] };
                let un = |c: u8| -> u8 {
                    if s.fmt != gfx::FMT_PARGB || a == 255 {
                        c
                    } else if a == 0 {
                        0
                    } else {
                        ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8
                    }
                };
                rgb.extend_from_slice(&[un(p[2]), un(p[1]), un(p[0])]);
                alpha.push(a);
            }
        }
        check_size(w as u32, h as u32, 3, rgb.len())?;
        Ok(Image::raw(w as u32, h as u32, rgb, Some(alpha)))
    }

    /// RGB samples, gray when every pixel is; the alpha dropped when every pixel is opaque
    fn raw(w: u32, h: u32, rgb: Vec<u8>, alpha: Option<Vec<u8>>) -> Image {
        let alpha = alpha.filter(|a| a.iter().any(|&v| v != 255));
        let gray = rgb.chunks_exact(3).all(|p| p[0] == p[1] && p[1] == p[2]);
        let px = if gray { Pixels::Raw { colors: 1, data: rgb.chunks_exact(3).map(|p| p[0]).collect() } } else { Pixels::Raw { colors: 3, data: rgb } };
        Image { w, h, px, alpha, interpolate: false }
    }

    /// an alpha channel (one byte per pixel, rows from the top) for any image, a JPEG too
    pub fn with_alpha(mut self, alpha: Vec<u8>) -> Result<Image, Error> {
        check_size(self.w, self.h, 1, alpha.len())?;
        self.alpha = if alpha.iter().all(|&v| v == 255) { None } else { Some(alpha) };
        Ok(self)
    }

    /// smooth the image when a reader scales it up (PDF /Interpolate)
    pub fn interpolate(mut self, on: bool) -> Image {
        self.interpolate = on;
        self
    }

    /// width and height in pixels
    pub fn size(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    pub fn is_jpeg(&self) -> bool {
        matches!(self.px, Pixels::Jpeg { .. })
    }

    pub fn has_alpha(&self) -> bool {
        self.alpha.is_some()
    }
}

struct JpegInfo {
    w: u32,
    h: u32,
    comps: u8,
    invert: bool,
    transform: Option<u8>,
}

/// the frame header and Adobe's APP14 marker of a JPEG file
fn parse_jpeg(d: &[u8]) -> Result<JpegInfo, Error> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return Err(Error::Image("not a JPEG file"));
    }
    let mut i = 2;
    let mut adobe: Option<u8> = None;
    loop {
        // markers may be preceded by fill bytes 0xFF
        while i < d.len() && d[i] == 0xFF && d.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if i + 4 > d.len() || d[i] != 0xFF {
            return Err(Error::Image("a JPEG file without a frame header"));
        }
        let m = d[i + 1];
        if matches!(m, 0xD0..=0xD7 | 0x01) {
            i += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return Err(Error::Image("a JPEG file without a frame header"));
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if len < 2 || i + 2 + len > d.len() {
            return Err(Error::Image("a truncated JPEG segment"));
        }
        let seg = &d[i + 4..i + 2 + len];
        match m {
            0xEE if seg.len() >= 12 && &seg[..5] == b"Adobe" => adobe = Some(seg[11]),
            0xC0..=0xC2 => {
                if seg.len() < 6 {
                    return Err(Error::Image("a truncated JPEG frame header"));
                }
                if seg[0] != 8 {
                    return Err(Error::Unsupported("JPEG with other than 8 bits per sample"));
                }
                let h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                let w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                let comps = seg[5];
                if w == 0 || h == 0 {
                    return Err(Error::Unsupported("JPEG whose height comes later (DNL)"));
                }
                if !matches!(comps, 1 | 3 | 4) {
                    return Err(Error::Unsupported("JPEG with other than 1, 3 or 4 components"));
                }
                // the frame header may come before APP14: look for it in the rest too
                if adobe.is_none() {
                    adobe = find_adobe(&d[i + 2 + len..]);
                }
                let transform = match (comps, adobe) {
                    (3, Some(0)) => Some(0),
                    (4, Some(t)) if t != 0 => Some(1),
                    _ => None,
                };
                return Ok(JpegInfo { w, h, comps, invert: comps == 4 && adobe.is_some(), transform });
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::Unsupported("lossless, hierarchical or arithmetic-coded JPEG"));
            }
            _ => {}
        }
        i += 2 + len;
    }
}

fn find_adobe(d: &[u8]) -> Option<u8> {
    let mut i = 0;
    while i + 4 <= d.len() && d[i] == 0xFF {
        let m = d[i + 1];
        if m == 0xDA || m == 0xD9 {
            return None;
        }
        let len = u16::from_be_bytes([d[i + 2], d[i + 3]]) as usize;
        if len < 2 || i + 2 + len > d.len() {
            return None;
        }
        let seg = &d[i + 4..i + 2 + len];
        if m == 0xEE && seg.len() >= 12 && &seg[..5] == b"Adobe" {
            return Some(seg[11]);
        }
        i += 2 + len;
    }
    None
}

/// rows of `bpp`-byte pixels with a PNG filter byte each (the one with the smallest sum of
/// absolute differences, as libpng's heuristic), for /Predictor 15
pub(crate) fn png_filter(data: &[u8], w: usize, bpp: usize) -> Vec<u8> {
    let row = w * bpp;
    let h = data.len().checked_div(row).unwrap_or(0);
    let mut out = Vec::with_capacity(h * (row + 1));
    let zero = alloc::vec![0u8; row];
    let mut cand = [Vec::with_capacity(row), Vec::with_capacity(row), Vec::with_capacity(row), Vec::with_capacity(row), Vec::with_capacity(row)];
    for y in 0..h {
        let cur = &data[y * row..(y + 1) * row];
        let up = if y > 0 { &data[(y - 1) * row..y * row] } else { &zero[..] };
        for c in cand.iter_mut() {
            c.clear();
        }
        for x in 0..row {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let b = up[x];
            let c = if x >= bpp { up[x - bpp] } else { 0 };
            let v = cur[x];
            cand[0].push(v);
            cand[1].push(v.wrapping_sub(a));
            cand[2].push(v.wrapping_sub(b));
            cand[3].push(v.wrapping_sub(((a as u16 + b as u16) / 2) as u8));
            cand[4].push(v.wrapping_sub(paeth(a, b, c)));
        }
        let cost = |v: &Vec<u8>| v.iter().map(|&x| (x as i8).unsigned_abs() as u64).sum::<u64>();
        let mut best = 0;
        let mut best_cost = u64::MAX;
        for (k, c) in cand.iter().enumerate() {
            let s = cost(c);
            if s < best_cost {
                best = k;
                best_cost = s;
            }
        }
        out.push(best as u8);
        out.extend_from_slice(&cand[best]);
    }
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let pa = (p - a as i16).abs();
    let pb = (p - b as i16).abs();
    let pc = (p - c as i16).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}
