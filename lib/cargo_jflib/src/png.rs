// PNG in and out, as RGBA with 8 bits: the screenshots for the setup's previews and the app's
// icons for the .deb.  It reads 8 and 16 bits, grey, RGB, palette and alpha, not interlaced
// files; it writes RGBA without filters (tools/pngutil.py), downscales by area (the same numbers)
// and draws the placeholder icon.

use crate::deflate;
use crate::zip::crc32;

pub(crate) struct Image {
    pub w: usize,
    pub h: usize,
    /// RGBA, row after row
    pub px: Vec<u8>,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub(crate) fn decode(data: &[u8]) -> Result<Image, String> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("not a PNG file".to_string());
    }
    let bad = || "a PNG file that ends early".to_string();
    let (mut w, mut h, mut depth, mut ctype) = (0usize, 0usize, 0u8, 0u8);
    let (mut palette, mut trns, mut idat) = (Vec::new(), Vec::new(), Vec::new());
    let mut at = 8;
    loop {
        let len = be32(data, at).ok_or_else(bad)? as usize;
        let kind = data.get(at + 4..at + 8).ok_or_else(bad)?;
        let body = data.get(at + 8..at + 8 + len).ok_or_else(bad)?;
        match kind {
            b"IHDR" => {
                w = be32(body, 0).ok_or_else(bad)? as usize;
                h = be32(body, 4).ok_or_else(bad)? as usize;
                depth = body[8];
                ctype = body[9];
                if body[12] != 0 {
                    return Err("an interlaced PNG file: save it without interlacing".to_string());
                }
            }
            b"PLTE" => palette = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + len;
    }
    let channels = match ctype {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(format!("a PNG file of colour type {ctype}")),
    };
    if !(depth == 8 || depth == 16 && ctype != 3) || w == 0 || h == 0 || w > 1 << 14 || h > 1 << 14 {
        return Err(format!("a PNG file with {depth}-bit samples or {w} x {h} pixels: save it with 8 bits"));
    }
    let bpp = channels * depth as usize / 8;
    let stride = w * bpp;
    let raw = deflate::unzlib(&idat, h * (stride + 1))?;
    if raw.len() != h * (stride + 1) {
        return Err("a PNG file with too little image data".to_string());
    }
    let mut cur = vec![0u8; stride];
    let mut prior = vec![0u8; stride];
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let line = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let f = line[0];
        for x in 0..stride {
            let a = if x >= bpp { cur[x - bpp] } else { 0 };
            let (b, c) = (prior[x], if x >= bpp { prior[x - bpp] } else { 0 });
            cur[x] = line[1 + x].wrapping_add(match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => ((a as u16 + b as u16) / 2) as u8,
                4 => paeth(a, b, c),
                _ => return Err(format!("a PNG row with filter {f}")),
            });
        }
        let step = depth as usize / 8; // 16 bits: the high byte
        for x in 0..w {
            let s = |k: usize| cur[x * bpp + k * step];
            let rgba = match ctype {
                0 => [s(0), s(0), s(0), 255],
                2 => [s(0), s(1), s(2), 255],
                3 => {
                    let i = s(0) as usize;
                    let p = palette.get(i * 3..i * 3 + 3).ok_or("a PNG palette index outside the palette")?;
                    [p[0], p[1], p[2], trns.get(i).copied().unwrap_or(255)]
                }
                4 => [s(0), s(0), s(0), s(1)],
                _ => [s(0), s(1), s(2), s(3)],
            };
            px.extend_from_slice(&rgba);
        }
        std::mem::swap(&mut cur, &mut prior);
    }
    Ok(Image { w, h, px })
}

pub(crate) fn encode(img: &Image) -> Vec<u8> {
    let mut raw = Vec::with_capacity(img.h * (img.w * 4 + 1));
    for row in img.px.chunks(img.w * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = (img.w as u32).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&(img.h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &deflate::zlib(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// nw x nh by averaging the area each pixel covers, colours weighted by alpha
pub(crate) fn downscale(img: &Image, nw: usize, nh: usize) -> Image {
    let (w, h) = (img.w, img.h);
    let mut px = vec![0u8; nw * nh * 4];
    for ny in 0..nh {
        let (y0, y1) = (ny * h / nh, ((ny + 1) * h / nh).max(ny * h / nh + 1));
        for nx in 0..nw {
            let (x0, x1) = (nx * w / nw, ((nx + 1) * w / nw).max(nx * w / nw + 1));
            let mut acc = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &img.px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    let a = p[3] as u64;
                    acc[0] += p[0] as u64 * a;
                    acc[1] += p[1] as u64 * a;
                    acc[2] += p[2] as u64 * a;
                    acc[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            let o = (ny * nw + nx) * 4;
            for k in 0..3 {
                px[o + k] = acc[k].checked_div(acc[3]).unwrap_or(0) as u8;
            }
            px[o + 3] = ((acc[3] + n / 2) / n) as u8;
        }
    }
    Image { w: nw, h: nh, px }
}

/// a rounded square with a blue gradient, for an app without res/app_*.png (tools/pkgmeta.py)
pub(crate) fn placeholder(size: usize) -> Image {
    let mut px = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        let t = y as f64 / (size.max(2) - 1) as f64;
        let rgb = [(30.0 + 40.0 * t) as u8, (120.0 - 50.0 * t) as u8, (230.0 - 60.0 * t) as u8, 255];
        for _ in 0..size {
            px.extend_from_slice(&rgb);
        }
    }
    let r = size as f64 * 0.18;
    for y in 0..size {
        for x in 0..size {
            let (xf, yf, s) = (x as f64, y as f64, size as f64);
            if (xf >= r && xf < s - r) || (yf >= r && yf < s - r) {
                continue;
            }
            let (cx, cy) = (if xf < r { r } else { s - r }, if yf < r { r } else { s - r });
            let mut inside = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (qx, qy) = (xf + (sx as f64 + 0.5) / 4.0, yf + (sy as f64 + 0.5) / 4.0);
                    inside += ((qx - cx).powi(2) + (qy - cy).powi(2) <= r * r) as u32;
                }
            }
            let p = (y * size + x) * 4 + 3;
            px[p] = (px[p] as u32 * inside / 16) as u8;
        }
    }
    Image { w: size, h: size, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_scale() {
        let img = placeholder(64);
        assert_eq!(img.px[3], 0); // the corner is transparent
        let back = decode(&encode(&img)).unwrap();
        assert_eq!((back.w, back.h), (64, 64));
        assert_eq!(back.px, img.px);
        let small = downscale(&img, 16, 16);
        assert_eq!(small.px.len(), 16 * 16 * 4);
        assert_eq!(small.px[(8 * 16 + 8) * 4 + 3], 255);
        assert!(small.px[3] < 64);
    }
}
