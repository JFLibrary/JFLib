// raster -- a page as pixels (EX-04 PNG, EX-05 JPEG) through the CPU path
//
//   render(view, &RasterOptions) -> Option<Surface>   the page (or a part of it) at a resolution:
//                                         XRGB on a colour, straight ARGB when transparent
//   size(view, &RasterOptions) -> (w, h)  the pixels render would make
//   png(sf, dpi) -> Option<Vec<u8>>       PNG (lossless, palette when it fits, pHYs = dpi)
//   jpeg(sf, quality, dpi, under)         JPEG (transparency over the colour `under`, JFIF density)
//   flatten(sf, color) -> Surface         straight ARGB over a colour -> XRGB
//
// The picture's top left pixel is the region's top left corner; one pixel is 1/dpi inch.  A
// transparent picture is drawn twice, on black and on white: where they differ the page let
// the background through, and colour and alpha follow from the two (exact for anything that
// blends linearly, which the CPU path does), so render2d needs no premultiplied target.

use alloc::vec::Vec;

use gfx::{Dc, Surface, FMT_ARGB, FMT_XRGB};
use render2d::Camera;

use crate::PageView;

/// the largest side render makes (larger requests give None)
pub const MAX_SIDE: u32 = 30_000;
/// the most pixels render makes (larger requests give None)
pub const MAX_PIXELS: u64 = 200_000_000;

/// what lies under the page's shapes
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Background {
    /// the page's own colour (white unless the page says otherwise)
    Page,
    /// a colour 0xRRGGBB
    Color(u32),
    /// nothing: alpha 0 where no shape is
    Transparent,
}

/// what render draws
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RasterOptions {
    /// pixels per paper inch (Visio: screen 96, printer 300, own)
    pub dpi: f64,
    /// the part of the page (paper inches, y up: x0, y0, x1, y1); None: the whole page
    pub region: Option<[f64; 4]>,
    pub background: Background,
}

impl Default for RasterOptions {
    fn default() -> RasterOptions {
        RasterOptions { dpi: 96.0, region: None, background: Background::Page }
    }
}

fn region(view: &PageView, opts: &RasterOptions) -> [f64; 4] {
    let d = view.drawing();
    match opts.region {
        Some(r) if r.iter().all(|v| v.is_finite()) && r[2] > r[0] && r[3] > r[1] => r,
        _ => [0.0, 0.0, d.width.max(1e-6), d.height.max(1e-6)],
    }
}

/// the size in pixels render makes for these options (0 when the resolution is not usable)
pub fn size(view: &PageView, opts: &RasterOptions) -> (u32, u32) {
    if !(opts.dpi.is_finite() && opts.dpi > 0.0) {
        return (0, 0);
    }
    let r = region(view, opts);
    let side = |v: f64| {
        let px = v * opts.dpi;
        let c = crate::floor(px);
        let c = if px - c > 1e-6 { c + 1.0 } else { c };
        c.clamp(1.0, 1e9) as u32
    };
    (side(r[2] - r[0]), side(r[3] - r[1]))
}

/// the page drawn at the options' resolution: FMT_XRGB on a colour, FMT_ARGB (straight) when
/// transparent; None when it would be larger than MAX_SIDE / MAX_PIXELS or memory is short
pub fn render(view: &mut PageView, opts: &RasterOptions) -> Option<Surface> {
    let (w, h) = size(view, opts);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w as u64 * h as u64 > MAX_PIXELS {
        return None;
    }
    let r = region(view, opts);
    let s = 1.0 / opts.dpi;
    // the region's top left corner at pixel (0, 0)
    let cam = Camera::new([r[0] + w as f64 * s * 0.5, r[3] - h as f64 * s * 0.5], s);
    let page = view.drawing().color & 0xFF_FFFF;
    match opts.background {
        Background::Page => draw(view, w, h, &cam, 0xFF00_0000 | page),
        Background::Color(c) => draw(view, w, h, &cam, 0xFF00_0000 | (c & 0xFF_FFFF)),
        Background::Transparent => {
            let black = draw(view, w, h, &cam, 0xFF00_0000)?;
            let white = draw(view, w, h, &cam, 0xFFFF_FFFF)?;
            let mut out = Surface::try_new(w as i32, h as i32, FMT_ARGB)?;
            let (sb, so) = (black.stride as usize, out.stride as usize);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let (i, o) = (y * sb + x * 4, y * so + x * 4);
                    let b = &black.px[i..i + 4];
                    let wp = &white.px[i..i + 4];
                    // alpha from the channel that differs most (white - black = 255 - alpha)
                    let mut diff = 0i32;
                    for k in 0..3 {
                        diff = diff.max(wp[k] as i32 - b[k] as i32);
                    }
                    let a = (255 - diff).clamp(0, 255) as u32;
                    let d = &mut out.px[o..o + 4];
                    if a == 0 {
                        d.copy_from_slice(&[0, 0, 0, 0]);
                        continue;
                    }
                    for k in 0..3 {
                        d[k] = ((b[k] as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                    d[3] = a as u8;
                }
            }
            Some(out)
        }
    }
}

fn draw(view: &mut PageView, w: u32, h: u32, cam: &Camera, under: u32) -> Option<Surface> {
    let mut sf = Surface::try_new(w as i32, h as i32, FMT_XRGB)?;
    let mut dc = Dc::new(w as i32, h as i32);
    dc.begin(&sf);
    dc.clear(&mut sf, under);
    view.draw(&mut dc, &mut sf, [0, 0, w as i32, h as i32], cam);
    dc.end();
    Some(sf)
}

/// a surface over a colour (0xRRGGBB) as XRGB: straight ARGB and premultiplied PARGB blend,
/// anything else counts as opaque
pub fn flatten(sf: &Surface, color: u32) -> Surface {
    let mut out = Surface::new(sf.w, sf.h, FMT_XRGB);
    let bg = [(color & 255), (color >> 8) & 255, (color >> 16) & 255];
    let (si, so) = (sf.stride as usize, out.stride as usize);
    for y in 0..sf.h.max(0) as usize {
        for x in 0..sf.w.max(0) as usize {
            let (i, o) = (y * si + x * 4, y * so + x * 4);
            let a = sf.px[i + 3] as u32;
            for k in 0..3 {
                let c = sf.px[i + k] as u32;
                out.px[o + k] = match sf.fmt {
                    FMT_ARGB => ((c * a + bg[k] * (255 - a) + 127) / 255) as u8,
                    gfx::FMT_PARGB => (c + (bg[k] * (255 - a) + 127) / 255).min(255) as u8,
                    _ => c as u8,
                };
            }
            out.px[o + 3] = 255;
        }
    }
    out
}

/// PNG of a surface (render's): RGB or RGBA, a palette when there are few colours, the
/// resolution as pHYs
pub fn png(sf: &Surface, dpi: f64) -> Option<Vec<u8>> {
    let mut chunks = Vec::new();
    if dpi.is_finite() && dpi > 0.0 {
        let ppm = crate::floor(dpi / 0.0254 + 0.5).clamp(1.0, u32::MAX as f64) as u32;
        let mut d = Vec::with_capacity(9);
        d.extend_from_slice(&ppm.to_be_bytes());
        d.extend_from_slice(&ppm.to_be_bytes());
        d.push(1); // metres
        img_png::enc::put_chunk(&mut chunks, b"pHYs", &d);
    }
    img_png::enc::encode_cfg(sf, &img_png::enc::PngCfg { level: 6, depth: 0, chunks: &chunks, ..Default::default() }).ok()
}

/// JPEG of a surface at quality 1..100; a transparent one over `under` (0xRRGGBB); the
/// resolution as JFIF density
pub fn jpeg(sf: &Surface, quality: i32, dpi: f64, under: u32) -> Option<Vec<u8>> {
    let flat;
    let src = if sf.fmt == FMT_ARGB {
        flat = flatten(sf, under);
        &flat
    } else {
        sf
    };
    let d = if dpi.is_finite() && dpi >= 1.0 { crate::floor(dpi + 0.5).min(65535.0) as u32 } else { 96 };
    let cfg = img_jpeg::enc::EncCfg { chroma: 0, unit: 1, xd: d, yd: d };
    img_jpeg::enc::encode_ex(src, quality.clamp(1, 100), Some(&cfg))
}
