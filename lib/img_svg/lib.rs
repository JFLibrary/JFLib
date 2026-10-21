// img_svg -- SVG images and icons: a practical SVG 1.1 / SVG Tiny subset parsed once and drawn
// at any size through gfx's path rasterizer (no libc, no std)
//
//   Svg::parse(bytes) -> Ok(svg) / Err(ERR_*)      UTF-8 XML, or gzip (.svgz)
//   svg.size() -> (w, h)        intrinsic size in px (width / height, else the viewBox, else
//                               300 x 150); view_box(), title(), truncated(), element_count()
//   svg.render(sf, &m, clip, &mut opts)            the document at its intrinsic size through
//                               the matrix m (intrinsic px -> surface px), only inside clip
//   svg.render_rect(sf, x, y, w, h, clip, &mut opts)   the rectangle is the viewport: the
//                               viewBox is mapped into it (preserveAspectRatio)
//   svg.rasterize(w, h, &mut opts) -> Ok(Surface)   a new FMT_PARGB surface of that size
//   Opts { color (currentColor), text: Option<&mut dyn TextHook> }   (Opts::default(): black,
//                               no text)
//   rasterize(bytes, w, h) -> Ok(Surface)   parse + rasterize (w or h 0: from the aspect
//                               ratio; both 0: the intrinsic size)
//   sniff(bytes) -> bool        an SVG document (after BOM, XML declaration, comments, DOCTYPE)
//   register()                  the format IMGF_SVG ("svg;svgz") in img_core's format table:
//                               img_core::io::load / load_ex / decode_ex read SVG files at
//                               their intrinsic size (ui's runtime calls it at start-up)
//   CODEC                       the codec descriptor (decode, classic decode_fl, probe)
//
// Supported: svg (nested: viewport, viewBox, preserveAspectRatio, overflow), g, defs, use
// (+ symbol), switch (first child), a; path (all commands incl. arcs), rect (rx / ry), circle,
// ellipse, line, polyline, polygon; transform (matrix translate scale rotate skewX skewY);
// fill / stroke: colours (#rgb, #rrggbb(aa), rgb(a), hsl(a), 148 names, currentColor,
// transparent), fill-opacity, fill-rule, stroke-width / -linecap / -linejoin / -miterlimit /
// -dasharray / -dashoffset / -opacity; linearGradient / radialGradient (stops with offset,
// stop-color, stop-opacity; gradientUnits, gradientTransform, spreadMethod pad / reflect /
// repeat, href inheritance, focal point fx / fy / fr); opacity of groups and elements
// (layers); visibility, display; clipPath (shapes, text, use; clipPathUnits, clip-rule,
// nested clip paths); <style> sheets with type / class / id / attribute selectors and
// descendant / child combinators, the style attribute, presentation attributes, inherit;
// lengths in px pt pc mm cm in em ex %; text / tspan through a TextHook (one line per chunk,
// x / y / dx / dy first values, text-anchor, font-size / -family / -weight / -style); image
// with data: URIs of the registered raster formats (PNG, JPEG, ...).
// Not supported (drawn without): pattern (the fallback colour), mask, marker, filter,
// textPath layout, per-glyph positions, letter-spacing, external references (other files),
// CSS pseudo-classes, @media, animation, scripts.
//
// Robustness: malformed input never panics.  The reader is forgiving (xml.rs); limits:
// MAX_INPUT bytes (also after inflating), 250 000 elements, 256 levels, 4096 CSS rules, a
// step budget for selector matching, 1 000 000 element visits per render, 64 levels of
// nesting while drawing, use cycles cut, 8 layers alive at once.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod css;
mod doc;
mod paint;
mod pathd;
mod render;
mod values;
mod xml;

use alloc::vec::Vec;

use gfx::xform::Matrix;
use gfx::{Surface, FMT_PARGB};
use img_core::*;

pub use doc::Svg;
pub use render::{FontSpec, Opts, TextHook};

/// the largest document (bytes; also the inflated size of .svgz)
pub const MAX_INPUT: usize = 32 << 20;

/// inflate a gzip member (.svgz)
pub(crate) fn gunzip(data: &[u8]) -> Result<Vec<u8>, i32> {
    let n = img_core::gzip::gzip_isize(data)? as usize;
    if n > MAX_INPUT {
        return Err(ERR_LIMIT);
    }
    let mut out = alloc::vec![0u8; n.max(1)];
    let (len, _) = img_core::gzip::gzip_decode(data, &mut out)?;
    out.truncate(len);
    Ok(out)
}

// base64 (standard and URL alphabet; blanks skipped; padding optional)
fn base64(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
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
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// the bytes of a base64 data: URI of a raster image (not SVG itself)
pub(crate) fn data_uri_bytes(href: &str) -> Option<Vec<u8>> {
    let h = href.trim();
    let rest = h.strip_prefix("data:")?;
    let comma = rest.find(',')?;
    let head = &rest[..comma];
    if !head.ends_with(";base64") || head.contains("svg") {
        return None;
    }
    let bytes = base64(rest[comma + 1..].as_bytes())?;
    if bytes.len() > MAX_INPUT || bytes.is_empty() {
        return None;
    }
    Some(bytes)
}

/// decode embedded image bytes with the registered decoders (at the first draw, so the
/// codecs may be registered after parsing)
pub(crate) fn decode_image(bytes: &[u8]) -> Option<Surface> {
    if img_core::io::detect(bytes, 0) == IMGF_SVG {
        return None;
    }
    let dec = ImgDec { maxpx: 1 << 26, ..ImgDec::default() };
    let sf = img_core::io::decode_ex(bytes, Some(&dec), None).ok()?;
    if sf.fmt == FMT_PARGB { Some(sf) } else { None }
}

fn blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\r' | b'\n')
}

/// the offset of the root element's '<' when data is an SVG document: BOM, blanks, the XML
/// declaration, comments, processing instructions and a DOCTYPE may come first (4 KB of them)
pub fn sniff_at(data: &[u8]) -> Option<usize> {
    let mut i = if data.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    let lim = data.len().min(i + 4096);
    loop {
        while i < data.len() && blank(data[i]) {
            i += 1;
        }
        if i >= lim {
            return None;
        }
        let r = &data[i..];
        let skip_to = |pat: &[u8], from: usize| -> Option<usize> { r.windows(pat.len()).skip(from).position(|w| w == pat).map(|k| i + from + k + pat.len()) };
        if r.starts_with(b"<?") {
            i = skip_to(b"?>", 2)?;
        } else if r.starts_with(b"<!--") {
            i = skip_to(b"-->", 4)?;
        } else if r.len() >= 9 && r[..9].eq_ignore_ascii_case(b"<!DOCTYPE") {
            // up to the '>' outside the internal subset
            let mut depth = 0;
            let mut j = 9;
            loop {
                let &c = r.get(j)?;
                if c == b'[' {
                    depth += 1;
                } else if c == b']' {
                    depth -= 1;
                } else if c == b'>' && depth <= 0 {
                    break;
                }
                j += 1;
            }
            i += j + 1;
        } else {
            let name_ok = |k: usize| r.get(k).is_some_and(|&c| blank(c) || c == b'>' || c == b'/');
            if (r.starts_with(b"<svg") && name_ok(4)) || (r.starts_with(b"<svg:svg") && name_ok(8)) {
                return Some(i);
            }
            return None;
        }
    }
}

/// data is an SVG document (see sniff_at)
pub fn sniff(data: &[u8]) -> bool {
    sniff_at(data).is_some()
}

// the intrinsic size from the root start tag only (probe: no full parse)
fn root_size(data: &[u8]) -> Option<(u32, u32)> {
    let at = sniff_at(data)?;
    let end = data[at..].iter().position(|&c| c == b'>').map(|k| at + k + 1)?;
    let svg = Svg::parse(&data[..end]).ok()?;
    let (w, h) = svg.size();
    Some((px_side(w), px_side(h)))
}

// a side in whole pixels (at least 1, at most 65535)
fn px_side(v: f64) -> u32 {
    let c = values::floor(v);
    let c = if c < v { c + 1.0 } else { c };
    c.clamp(1.0, 65535.0) as u32
}

fn fill_meta(meta: &mut ImgMeta, w: u32, h: u32) {
    meta.w = w;
    meta.h = h;
    meta.bits = 8;
    meta.model = IMM_RGB;
    meta.channels = 4;
}

fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    match root_size(data) {
        Some((w, h)) => {
            fill_meta(meta, w, h);
            true
        }
        None => false,
    }
}

/// the document at its intrinsic size (rounded up) into a new FMT_PARGB surface
fn render_natural(svg: &Svg, maxpx: u64) -> Result<Surface, i32> {
    let (w, h) = svg.size();
    let (pw, ph) = (px_side(w), px_side(h));
    if pw as u64 * ph as u64 > maxpx {
        return Err(ERR_LIMIT);
    }
    let mut sf = Surface::new(pw as i32, ph as i32, FMT_PARGB);
    if sf.px.is_empty() {
        return Err(ERR_NOMEM);
    }
    svg.render(&mut sf, &Matrix::IDENTITY, None, &mut Opts::default());
    Ok(sf)
}

fn decode(data: &[u8], out: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    let svg = match Svg::parse(data) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let (w, h) = svg.size();
    fill_meta(meta, px_side(w), px_side(h));
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    if dec.frame != 0 {
        return ERR_UNSUPPORTED;
    }
    if dec.cancelled() {
        return ERR_CANCEL;
    }
    match render_natural(&svg, dec.max_pixels()) {
        Ok(sf) => {
            *out = sf;
            0
        }
        Err(e) => e,
    }
}

// the classic decoder (img_core::io::decode / load): FMT_PARGB, FMT_ARGB with JDF_STRAIGHT
fn decode_fl(data: &[u8], out: &mut Surface, jdf: u32) -> i32 {
    let svg = match Svg::parse(data) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match render_natural(&svg, IMG_MAXPX_DEF) {
        Ok(mut sf) => {
            if jdf & JDF_STRAIGHT != 0 {
                gfx::straight::unpremul_inplace(&mut sf.px);
                sf.fmt = gfx::FMT_ARGB;
            }
            *out = sf;
            0
        }
        Err(e) => e,
    }
}

/// the codec descriptor of SVG (IMGF_SVG)
pub static CODEC: Codec = Codec { probe: Some(probe), decode: Some(decode), decode_fl: Some(decode_fl), caps: IFC_DECODE, ..Codec::EMPTY };

/// enter SVG into img_core's format table (IMGF_SVG, "svg;svgz=SVG", image/svg+xml)
pub fn register() -> i32 {
    img_core::io::register_slot(IMGF_SVG, "SVG", "svg;svgz=SVG", "image/svg+xml", IFC_ALPHA | IFC_LOSSLESS, Some(&CODEC))
}

/// parse and draw: w x h px (w or h 0: from the other and the aspect ratio; both 0: the
/// intrinsic size, rounded up)
pub fn rasterize(data: &[u8], w: i32, h: i32) -> Result<Surface, i32> {
    let svg = Svg::parse(data)?;
    let (iw, ih) = svg.size();
    let (w, h) = match (w > 0, h > 0) {
        (true, true) => (w, h),
        (true, false) => (w, px_side(w as f64 * ih / iw) as i32),
        (false, true) => (px_side(h as f64 * iw / ih) as i32, h),
        (false, false) => (px_side(iw) as i32, px_side(ih) as i32),
    };
    svg.rasterize(w, h, &mut Opts::default())
}
