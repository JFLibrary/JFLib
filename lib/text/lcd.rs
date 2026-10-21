// lcd -- subpixel (LCD, ClearType-like) glyph smoothing
//
//   set_subpixel(SUBPIXEL_*) / subpixel()   the order of the display's subpixels; NONE (the
//        default) keeps greyscale masks.  RGB / BGR: three columns per pixel, VRGB / VBGR:
//        three rows (rotated panels)
//
// A glyph is rasterized at three times the resolution across the subpixels (x for RGB / BGR,
// y for VRGB / VBGR), FreeType's default 5-tap FIR filter (08 4D 56 4D 08, sum 256) spreads
// every subpixel's coverage over its neighbours (fewer colour fringes, the energy of each
// pixel kept), and each colour channel is blended with its own coverage.  The cache entry
// holds 3 bytes per pixel in the physical order of the subpixels (left to right, or top to
// bottom), so RGB and BGR share it; the mask is one pixel wider (taller) on each side than
// the greyscale one.
//
// Blending is gamma-correct-ish: target and text colour go to linear light (gamma 1.6, a
// 256-entry table), each channel is mixed there and comes back through a 2 x 4096-entry
// table.  Linear light makes dark text thinner and light text bolder, so the coverage is
// adjusted first by the text colour's luminance (DirectWrite's "enhanced contrast",
// c' = c (1 + k) / (1 + k c), k = LCD_DARK for black text down to -LCD_LIGHT for white):
// black, white and grey text keep the weight of greyscale text within a few per cent
// (fonttest --selftest measures it).
//
// Used only where it is safe: an opaque target (FMT_XRGB, the window framebuffer) and a
// glyph from the cache.  A8 and PARGB targets (layers, images, anything composited later),
// colour glyphs (COLR layers, CBDT bitmaps), the icon font, glyphs too large for the cache
// (filled as paths) and black-and-white masks (set_antialias(false)) stay greyscale.  The
// text colour's alpha and the global opacity scale the coverage (the target stays opaque).
// Mirrored (RTL) windows mirror the command lists, not the pixels: text keeps the physical
// subpixel order.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{Dc, Surface};

use crate::glyphcache::GcEntry;
use crate::Text;

/// set_subpixel: greyscale smoothing (no subpixels)
pub const SUBPIXEL_NONE: u8 = 0;
/// set_subpixel: red, green, blue from left to right
pub const SUBPIXEL_RGB: u8 = 1;
/// set_subpixel: blue, green, red from left to right
pub const SUBPIXEL_BGR: u8 = 2;
/// set_subpixel: red, green, blue from top to bottom
pub const SUBPIXEL_VRGB: u8 = 3;
/// set_subpixel: blue, green, red from top to bottom
pub const SUBPIXEL_VBGR: u8 = 4;

/// FreeType's FT_LCD_FILTER_DEFAULT (the weights add up to 256)
const FIR: [u32; 5] = [0x08, 0x4D, 0x56, 0x4D, 0x08];
/// the gamma of the blend
const LCD_GAMMA: f64 = 1.6;
/// the coverage boost of black text (DirectWrite's enhanced contrast)
const LCD_DARK: f64 = 1.0;
/// ... and the reduction for white text
const LCD_LIGHT: f64 = 0.35;

/// the blend tables (built by set_subpixel)
pub(crate) struct Lut {
    /// sRGB byte -> linear 0..65535
    to_lin: [u16; 256],
    /// linear 0..4095 -> byte (exact near black)
    from_lo: Vec<u8>,
    /// linear >> 4 -> byte
    from_hi: Vec<u8>,
    /// coverage -> boosted coverage for the current text colour (adj_for)
    adj: [u8; 256],
    adj_for: u32,
}

impl Lut {
    fn new() -> Lut {
        let g = LCD_GAMMA;
        let mut to_lin = [0u16; 256];
        for (v, o) in to_lin.iter_mut().enumerate() {
            *o = rt::fx::cvtd(rt::x87::pow(v as f64 / 255.0, g) * 65535.0).clamp(0, 65535) as u16;
        }
        let back = |l: f64| rt::fx::cvtd(rt::x87::pow(l / 65535.0, 1.0 / g) * 255.0).clamp(0, 255) as u8;
        let from_lo = (0..4096).map(|i| back(i as f64)).collect();
        let from_hi = (0..4096).map(|i| back(i as f64 * 16.0 + 7.5)).collect();
        let mut l = Lut { to_lin, from_lo, from_hi, adj: [0; 256], adj_for: u32::MAX };
        l.prepare(0);
        l
    }

    #[inline]
    fn from_lin(&self, l: i32) -> u8 {
        if l < 4096 { self.from_lo[l.max(0) as usize] } else { self.from_hi[(l.min(65535) >> 4) as usize] }
    }

    // the coverage adjustment for a text colour (0x??RRGGBB): k from LCD_DARK (black) to
    // -LCD_LIGHT (white)
    fn prepare(&mut self, rgb: u32) {
        let rgb = rgb & 0xFF_FFFF;
        if self.adj_for == rgb {
            return;
        }
        self.adj_for = rgb;
        let (r, g, b) = ((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        let luma = (r * 54 + g * 183 + b * 19) as f64 / (255.0 * 256.0);
        let k = LCD_DARK * (1.0 - luma) - LCD_LIGHT * luma;
        for (c, o) in self.adj.iter_mut().enumerate() {
            let x = c as f64 / 255.0;
            *o = rt::fx::cvtd(x * (1.0 + k) / (1.0 + k * x) * 255.0).clamp(0, 255) as u8;
        }
    }

    /// (tests) the byte that comes back from linear light: the tables are exact for every byte
    pub(crate) fn round_trip(&self, v: u8) -> u8 {
        self.from_lin(self.to_lin[v as usize] as i32)
    }
}

/// one row (or column, with step) of subpixel coverage through the FIR filter
fn fir(src: &[u8], n: usize, step: usize, out: &mut [u8]) {
    for j in 0..n {
        let mut s = 128u32;
        for (k, w) in FIR.iter().enumerate() {
            let t = j as isize + k as isize - 2;
            if t >= 0 && (t as usize) < n {
                s += w * src[t as usize * step] as u32;
            }
        }
        out[j * step] = (s >> 8).min(255) as u8;
    }
}

/// the filtered mask of a glyph rasterized at 3x across: horizontal (w3 = 3 w columns per
/// row): the rows themselves; vertical (3 h rows of w): regrouped to 3 bytes per pixel
pub(crate) fn filter(px: &[u8], w: usize, h: usize, vertical: bool) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 3];
    if !vertical {
        let w3 = w * 3;
        for r in 0..h {
            fir(&px[r * w3..r * w3 + w3], w3, 1, &mut out[r * w3..r * w3 + w3]);
        }
        return out;
    }
    let h3 = h * 3;
    let mut col = vec![0u8; h3 * w];
    for x in 0..w {
        fir(&px[x..], h3, w, &mut col[x..]);
    }
    for y in 0..h {
        for x in 0..w {
            for i in 0..3 {
                out[(y * w + x) * 3 + i] = col[(y * 3 + i) * w + x];
            }
        }
    }
    out
}

/// blend a subpixel mask (3 bytes per pixel in physical order; bgr: the first one is blue)
/// with the colour of dc on an XRGB target at pen x / baseline (absolute), clipped
pub(crate) fn blit(dc: &Dc, sf: &mut Surface, pen_x: i32, base: i32, e: &GcEntry, bgr: bool, lut: &Lut) {
    let mut x0 = pen_x.wrapping_add(e.left);
    let mut y0 = base.wrapping_add(e.top);
    let mut sx = 0i32;
    let mut sy = 0i32;
    let x1 = x0.wrapping_add(e.w).min(dc.clip[2]);
    let y1 = y0.wrapping_add(e.h).min(dc.clip[3]);
    if dc.clip[0] > x0 {
        sx = dc.clip[0] - x0;
        x0 = dc.clip[0];
    }
    if dc.clip[1] > y0 {
        sy = dc.clip[1] - y0;
        y0 = dc.clip[1];
    }
    let w = x1 - x0;
    let h = y1 - y0;
    let alpha = dc.pcol[3];
    if w <= 0 || h <= 0 || alpha == 0 {
        return;
    }
    // the text colour in linear light: B, G, R (the byte order of the target)
    let c = dc.color;
    let src = [c & 255, (c >> 8) & 255, (c >> 16) & 255];
    let src_lin = src.map(|v| lut.to_lin[v as usize] as i32);
    // mask byte of each target byte B, G, R
    let pick: [usize; 3] = if bgr { [0, 1, 2] } else { [2, 1, 0] };
    let stride = sf.stride as usize;
    for r in 0..h as usize {
        let mo = ((sy as usize + r) * e.w as usize + sx as usize) * 3;
        let o = (y0 as usize + r) * stride + x0 as usize * 4;
        let row = &mut sf.px[o..o + w as usize * 4];
        let mask = &e.px[mo..mo + w as usize * 3];
        for (p, m) in row.chunks_exact_mut(4).zip(mask.chunks_exact(3)) {
            if m[0] | m[1] | m[2] == 0 {
                continue;
            }
            for ch in 0..3 {
                let cov = lut.adj[m[pick[ch]] as usize] as u32;
                let a = (cov * alpha + 127) / 255;
                if a == 0 {
                    continue;
                }
                if a >= 255 {
                    p[ch] = src[ch] as u8;
                    continue;
                }
                let d = lut.to_lin[p[ch] as usize] as i32;
                let l = d + ((src_lin[ch] - d) * a as i32 + if src_lin[ch] >= d { 127 } else { -127 }) / 255;
                p[ch] = lut.from_lin(l);
            }
        }
    }
}

impl Text {
    /// the subpixel order of the display (SUBPIXEL_*): RGB / BGR / VRGB / VBGR draw glyphs on
    /// opaque (FMT_XRGB) targets with subpixel smoothing, NONE (the default) in greyscale;
    /// see lcd.rs for where it applies
    pub fn set_subpixel(&mut self, order: u8) {
        let order = if order <= SUBPIXEL_VBGR { order } else { SUBPIXEL_NONE };
        self.lcd = order;
        if order != SUBPIXEL_NONE && self.lcd_lut.is_none() {
            self.lcd_lut = Some(alloc::boxed::Box::new(Lut::new()));
        }
    }

    /// the subpixel order in use (SUBPIXEL_*)
    pub fn subpixel(&self) -> u8 {
        self.lcd
    }

    // subpixel smoothing for this glyph on this target?
    pub(crate) fn lcd_for(&self, sf: &Surface, font: u32) -> bool {
        self.lcd != SUBPIXEL_NONE && sf.fmt == gfx::FMT_XRGB && font != crate::FONT_ICON && self.antialias()
    }

    // set the coverage boost for the colour about to be drawn
    pub(crate) fn lcd_prepare(&mut self, argb: u32) {
        if let Some(l) = self.lcd_lut.as_mut() {
            l.prepare(argb);
        }
    }

    /// (tests) every byte survives the trip through linear light unchanged
    pub fn lcd_tables_exact(&mut self) -> bool {
        let l = Lut::new();
        (0..=255u8).all(|v| l.round_trip(v) == v)
    }
}
