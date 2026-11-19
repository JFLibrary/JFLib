// image -- the pictures of Foreign shapes: bytes from the package or inline base64, decoded
//
//   bytes(doc, &ImageSource) -> Option<Vec<u8>>   a part's bytes, or the inline data decoded
//   decode(bytes) -> Picture                      PNG and JPEG as pixels (straight ARGB rows);
//                                                 EMF, WMF and anything else: a placeholder
//   base64(text)                                  RFC 4648 with padding and line breaks
//
// The format is told by the bytes (the signatures), not by the file's attributes.  A picture
// larger than MAX_SIDE on a side is not decoded (a placeholder): a page shows it scaled down
// anyway, and a hostile file must not make the view allocate gigabytes.

use alloc::vec::Vec;

use diagram_core::render::ImageSource;
use diagram_core::Document;
use gfx::{Surface, FMT_ARGB, FMT_PARGB, FMT_XRGB};

/// the largest side of a picture the view decodes
pub const MAX_SIDE: u32 = 8192;

/// a decoded picture: w x h pixels, 0xAARRGGBB straight, rows top to bottom
pub struct Picture {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u32>,
}

/// what a picture became
pub enum Decoded {
    Picture(Picture),
    /// not decodable here: the format's name ("EMF", "WMF", "?")
    Placeholder(&'static str),
}

/// the bytes of a picture
pub fn bytes(doc: &Document, src: &ImageSource) -> Option<Vec<u8>> {
    match src {
        ImageSource::Part(name) => doc.parts.iter().find(|p| p.name == *name).map(|p| p.data.clone()),
        ImageSource::Inline(b) => base64(b.as_bytes()),
    }
}

/// decode a picture by its signature
pub fn decode(data: &[u8]) -> Decoded {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        if let Some((w, h)) = png_size(data) {
            if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
                return Decoded::Placeholder("PNG");
            }
        }
        return match img_png::decode_pargb(data) {
            Some(sf) => from_surface(&sf).map_or(Decoded::Placeholder("PNG"), Decoded::Picture),
            None => Decoded::Placeholder("PNG"),
        };
    }
    if data.starts_with(&[0xFF, 0xD8]) {
        return match img_jpeg::dec::decode_fl_max(data, 0, MAX_SIDE, MAX_SIDE) {
            Ok(sf) => from_surface(&sf).map_or(Decoded::Placeholder("JPEG"), Decoded::Picture),
            Err(_) => Decoded::Placeholder("JPEG"),
        };
    }
    if data.len() >= 44 && &data[40..44] == b" EMF" {
        return Decoded::Placeholder("EMF");
    }
    if data.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || data.starts_with(&[0x01, 0x00, 0x09, 0x00]) {
        return Decoded::Placeholder("WMF");
    }
    if data.starts_with(b"GIF8") {
        return Decoded::Placeholder("GIF");
    }
    if data.starts_with(b"BM") {
        return Decoded::Placeholder("BMP");
    }
    Decoded::Placeholder("?")
}

fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    let w = u32::from_be_bytes(data.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// a decoded surface as straight ARGB pixels
fn from_surface(sf: &Surface) -> Option<Picture> {
    if sf.w <= 0 || sf.h <= 0 || sf.px.is_empty() || sf.fmt == gfx::FMT_A8 {
        return None;
    }
    let (w, h) = (sf.w as usize, sf.h as usize);
    let stride = sf.stride as usize;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = sf.px.get(y * stride..y * stride + w * 4)?;
        for p in row.chunks_exact(4) {
            let (b, g, r, a) = (p[0] as u32, p[1] as u32, p[2] as u32, p[3] as u32);
            let c = match sf.fmt {
                FMT_XRGB => 0xFF00_0000 | r << 16 | g << 8 | b,
                FMT_PARGB => {
                    if a == 0 {
                        0
                    } else {
                        let un = |v: u32| ((v * 255 + a / 2) / a).min(255);
                        a << 24 | un(r) << 16 | un(g) << 8 | un(b)
                    }
                }
                FMT_ARGB => a << 24 | r << 16 | g << 8 | b,
                _ => 0xFF00_0000 | r << 16 | g << 8 | b,
            };
            px.push(c);
        }
    }
    Some(Picture { w: w as u32, h: h as u32, px })
}

/// base64 with padding; whitespace is skipped; None: a character outside the alphabet
pub fn base64(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}
