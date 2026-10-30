// text -- fonts (embedded SGF1 / TrueType, runtime OpenType / collections, synthetic styles,
// colour glyphs), the glyph cache and text runs                        [src/text/*.s]
//
//   Text::new(fonts, names)          register the style's four embedded fonts
//   set_font(font, size_fx)          select a font id and the pixel size (24.8)
//   draw(dc, sf, x, y, s) -> px      y = baseline (origin-relative); stops at NUL / newline
//   draw_n(dc, sf, x, y, s, len)     at most len bytes (a UTF-8 sequence cut by len ends it)
//   measure(s) -> 24.8   width(s) -> px (rounded up)   ascent() / descent() / line_h() -> px
//   hit(s, len, x) -> byte index     caret_x(s, index) -> px
//   decor(dc, sf, x, y, w, TD_*)     underline / strikeout rectangles
//   icon_draw(dc, sf, cp, x, y, size_fx)   FONT_ICON with its em box at x, y
//   gc_get / gc_get_bitmap / gc_box / gc_flush / gc_scale / gc_usage   the glyph cache
//                              (glyphcache.rs)
//   png_decode / img_scale     CBDT hooks (default: img_png::decode, img_core::ops::scale)
//   font registry: registry.rs; system font database: fontdb.rs; generic families,
//   substitutes and the languages' fonts (a light fontconfig): fontconf.rs
//   segment: grapheme clusters, words (UAX #29) and line break opportunities (UAX #14)
//   set_antialias(on) / antialias()   smoothed (default) or black-and-white glyph masks
//                                  (the system's "font smoothing off"; glyphs too large for
//                                  the cache stay smoothed)
//   set_subpixel(SUBPIXEL_*) / subpixel()   subpixel (LCD) smoothing in the display's order
//                                  on opaque targets (lcd.rs); NONE (default): greyscale
//   set_fallback_fonts / set_system_fallback / fallback_for   characters the font lacks come
//                                  from other fonts (fallback.rs; also when shaping)
//   record_n / record_icon / glyphs   glyphs placed now, blended later from the glyph cache by
//                                  any thread (record.rs: GlyphRec, GlyphView)
//   var_axes / var_instances / var_values / vary / vary_named   variable fonts: their axes,
//                                  named instances and instances at axis values (registry.rs,
//                                  var.rs: gvar, avar, HVAR, MVAR; shaping at the same
//                                  coordinates)
//
// Pen positions are 24.8 fixed point; glyphs are placed on quarter pixels and their
// coverage masks are cached per (font, glyph, size, phase, greyscale / subpixel).  Colour
// fonts: COLR glyphs are drawn layer by layer (palette 0; the foreground entry uses the
// current colour, palette alphas are multiplied by the current colour's alpha), CBDT glyphs
// as PARGB bitmaps (not drawn into A8 targets).  Glyphs too large for the glyph cache are
// filled directly.  U+00AD SOFT HYPHEN is not drawn and has no width (line wrapping shows a
// hyphen where it breaks: text::segment::wrap_end_hyph).

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cff;
pub mod colr;
mod fallback;
pub mod font;
pub mod fontconf;
pub mod fontdb;
pub mod glyphcache;
pub mod lcd;
pub mod outline;
mod record;
pub mod rendering;
pub mod registry;
pub mod segment;
mod segment_data;
pub mod ttf;
mod shaping;
pub mod var;

use alloc::vec::Vec;

use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{blend, Dc, Surface, FMT_A8};
use rt::fx::{cvt, div255};

pub use font::*;
pub use glyphcache::{GcEntry, GcRef};
pub use lcd::{SUBPIXEL_BGR, SUBPIXEL_NONE, SUBPIXEL_RGB, SUBPIXEL_VBGR, SUBPIXEL_VRGB};
pub use record::{GlyphRec, GlyphView};
pub use registry::{FONT_EMBEDDED, FONT_MAX_IDS};
pub use rendering::FontRaster;
pub use var::{VarAxis, VarInstance};

use colr::LAYER_FG;
use outline::Outline;
use registry::Slot;

pub const FONT_REG: u32 = 0;
pub const FONT_SEMI: u32 = 1;
pub const FONT_BOLD: u32 = 2;
pub const FONT_ICON: u32 = 3;
const LAYER_CAP: usize = 256; // colour layers drawn per glyph

pub struct Text {
    shaping: shaping::Cache,
    pub(crate) fb: fallback::Fallback,
    pub(crate) slots: Vec<Slot>,
    pub(crate) serial: u64,
    pub(crate) names: [[&'static str; 5]; 4],
    /// the outline buffer of runtime / derived fonts
    pub ol: Outline,
    /// the glyph transform used by glyph_path
    pub gp: Gp,
    pub path: Path,
    pub ras: Raster,
    pub(crate) gc: glyphcache::Gc,
    pub(crate) cb_metrics: [u8; 5],
    /// PNG decoder and bilinear scaler for CBDT colour bitmaps (set by the app / ui)
    pub png_decode: Option<colr::PngDecode>,
    pub img_scale: Option<colr::ImgScale>,
    font: u32,
    size: i32,
    sx: f32,
    run_y: i32,
    layers: Vec<(u32, u32)>,
    /// SUBPIXEL_* (lcd.rs) and its blend tables
    lcd: u8,
    lcd_lut: Option<alloc::boxed::Box<lcd::Lut>>,
    /// record_n / record_icon: glyph_draw appends here instead of blending (record.rs)
    rec: Option<record::GlyphRec>,
}

pub(crate) fn utf8_put(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

impl Text {
    /// font_init: register FONT_REG, FONT_SEMI, FONT_BOLD, FONT_ICON (the style's embedded
    /// fonts, SGF1 or TrueType/OpenType) with their names (family, style, full,
    /// typographic family / style). Each face starts with the legacy raster policy.
    pub fn new(fonts: [&'static [u8]; 4], names: [[&'static str; 5]; 4]) -> Text {
        let mut t = Text {
            shaping: Default::default(),
            fb: Default::default(),
            slots: Vec::new(),
            serial: 0,
            names,
            ol: Outline::default(),
            gp: Gp::default(),
            path: Path::default(),
            ras: Raster::default(),
            gc: glyphcache::Gc::default(),
            cb_metrics: [0; 5],
            png_decode: Some(png_decode),
            img_scale: Some(img_scale),
            font: 0,
            size: 0,
            sx: 0.0,
            run_y: 0,
            layers: Vec::new(),
            lcd: SUBPIXEL_NONE,
            lcd_lut: None,
            rec: None,
        };
        t.ol.reset();
        for f in fonts {
            if f.starts_with(b"SGF1") { t.register(f); } else { t.load_mem(f, 0); }
        }
        t.set_font(FONT_REG, 14 << 8);
        t
    }

    /// text_font(font, size_fx)
    pub fn set_font(&mut self, font: u32, size_fx: i32) {
        self.font = font;
        self.size = size_fx;
        self.sx = self.gc_scale(font, size_fx).0;
    }

    /// text_get_font() -> (font, size_fx)
    pub fn get_font(&self) -> (u32, i32) {
        (self.font, self.size)
    }

    // round(units * sx * 256)
    fn units_fx(&self, u: i32) -> i32 {
        cvt(u as f32 * self.sx * 256.0)
    }

    // units_fx for another font of the run (a fallback font) at the run's size
    fn units_fx_of(&self, font: u32, u: i32) -> i32 {
        if font == self.font { self.units_fx(u) } else { cvt(u as f32 * self.gc_scale(font, self.size).0 * 256.0) }
    }

    /// ceil(ascender px)
    pub fn ascent(&self) -> i32 {
        (self.units_fx(self.hdr(self.font).asc) + 255) >> 8
    }

    /// ceil(descender px), positive
    pub fn descent(&self) -> i32 {
        (self.units_fx(self.hdr(self.font).desc.wrapping_neg()) + 255) >> 8
    }

    /// round((asc - desc + gap) px)
    pub fn line_h(&self) -> i32 {
        let h = self.hdr(self.font);
        (self.units_fx(h.asc - h.desc + h.gap) + 128) >> 8
    }

    /// text_measure: advance of the run (24.8), up to NUL / newline
    pub fn measure(&mut self, s: &str) -> i32 {
        self.run(s.as_bytes(), 0, None, None)
    }

    /// text_measure_n(s, len)
    pub fn measure_n(&mut self, s: &[u8], len: usize) -> i32 {
        self.run(s, 0, Some(len), None)
    }

    /// text_width: px, rounded up
    pub fn width(&mut self, s: &str) -> i32 {
        (self.measure(s) + 255) >> 8
    }

    pub fn width_n(&mut self, s: &[u8], len: usize) -> i32 {
        (self.measure_n(s, len) + 255) >> 8
    }

    /// text_draw(x, y = baseline, s) -> width px
    pub fn draw(&mut self, dc: &mut Dc, sf: &mut Surface, x: i32, y: i32, s: &[u8]) -> i32 {
        self.draw_impl(dc, sf, x, y, s, None)
    }

    /// text_draw_n(x, y, s, len) -> width px
    pub fn draw_n(&mut self, dc: &mut Dc, sf: &mut Surface, x: i32, y: i32, s: &[u8], len: usize) -> i32 {
        self.draw_impl(dc, sf, x, y, s, Some(len))
    }

    fn draw_impl(&mut self, dc: &mut Dc, sf: &mut Surface, x: i32, y: i32, s: &[u8], len: Option<usize>) -> i32 {
        let pen = x.wrapping_add(dc.ox) << 8;
        self.run_y = y.wrapping_add(dc.oy);
        let w = self.run(s, pen, len, Some((dc, sf)));
        (w + 255) >> 8
    }

    // text_run: pen positions with kerning; draws when a target is given.  Stops at NUL,
    // newline, the end of s or after len bytes.
    fn run(&mut self, s: &[u8], pen0: i32, len: Option<usize>, mut target: Option<(&mut Dc, &mut Surface)>) -> i32 {
        if self.shape_line(s,len) { return self.shaped_draw(pen0,target); }
        let font = self.font;
        let classic = self.classic_metrics(font);
        let spacing = |v: i32| if classic { v.saturating_add(128) & !255 } else { v };
        let fallback = self.fallback_active();
        let mut pen = pen0;
        let mut prev: i32 = -1;
        let mut prev_font = font;
        let mut left = len.unwrap_or(usize::MAX);
        let mut i = 0;
        while left > 0 && i < s.len() {
            let b = s[i];
            if b == 0 || b == b'\n' {
                break;
            }
            if len.is_some() {
                // with a length: a UTF-8 sequence cut by it ends the run
                let n = if b < 0xC0 {
                    1
                } else if b < 0xE0 {
                    2
                } else if b < 0xF0 {
                    3
                } else {
                    4
                };
                if left < n {
                    break;
                }
            }
            let (cp, next) = base::utf8_decode(s, i);
            if len.is_some() {
                left = left.saturating_sub(next - i);
            }
            i = next;
            if cp == 0xAD {
                continue; // SOFT HYPHEN: invisible (wrapping draws a hyphen where it breaks)
            }
            // no-break, narrow and other spaces of number formats: a plain space when missing
            let cp = if matches!(cp, 0xA0 | 0x2000..=0x200A | 0x202F | 0x205F) && !self.has(font, cp) { 0x20 } else { cp };
            // a character the font lacks: the first font of the fallback chain that has it
            let mut f = font;
            if fallback && !self.has(font, cp) {
                if let Some(alt) = self.fallback_for(cp) {
                    f = alt;
                }
            }
            let g = self.glyph(f, cp);
            if prev >= 0 && f == prev_font {
                let k = self.kern(f, prev as u32, g);
                if k != 0 {
                    pen = pen.wrapping_add(spacing(self.units_fx_of(f, k)));
                }
            }
            if let Some((dc, sf)) = target.as_mut() {
                let flags = if self.slots.is_empty() { 0 } else { self.hdr(f).flags };
                let mut drawn = false;
                if flags & (SGH_COLR | SGH_CBDT) != 0 {
                    drawn = self.color_glyph(dc, sf, f, g, pen);
                }
                if !drawn {
                    self.glyph_draw(dc, sf, f, g, pen, true);
                }
            }
            let adv = self.adv(f, g);
            pen = pen.wrapping_add(spacing(self.units_fx_of(f, adv)));
            prev = g as i32;
            prev_font = f;
        }
        pen.wrapping_sub(pen0)
    }

    // draw one glyph with the current colour at pen (absolute 24.8) on the baseline run_y:
    // cached mask, or a direct fill when too large; lcd: subpixel smoothing allowed (not
    // for colour layers); while recording (record.rs) the glyph is appended to the record
    // instead  [glyph_draw]
    pub(crate) fn glyph_draw(&mut self, dc: &Dc, sf: &mut Surface, font: u32, g: u32, pen: i32, lcd: bool) {
        let size = self.size;
        let pen = self.font_raster(font).position(pen);
        let phase = (pen >> 6) & 3;
        let base = self.run_y;
        let rec = self.rec.is_some();
        if lcd && self.lcd_for(sf, font) {
            self.lcd_prepare(dc.color);
            let order = self.lcd;
            let vertical = order >= SUBPIXEL_VRGB;
            let bgr = order == SUBPIXEL_BGR || order == SUBPIXEL_VBGR;
            match self.gc_get_lcd(font, g, size, phase, vertical) {
                GcRef::None => return,
                GcRef::Entry(_) => {
                    let key = glyphcache::lcd_key(font, g, size, phase, vertical);
                    if let (Some(e), Some(lut)) = (self.gc.map.get(&key), self.lcd_lut.as_deref()) {
                        match self.rec.as_mut() {
                            Some(r) => r.lcd(key, e, pen >> 8, base, dc.color, bgr, lut.adj()),
                            None => lcd::blit(dc, sf, pen >> 8, base, e, bgr, lut, lut.adj()),
                        }
                    }
                    return;
                }
                GcRef::Direct => {} // too large for the cache: greyscale below
            }
        }
        match self.gc_get(font, g, size, phase) {
            GcRef::None => {}
            GcRef::Entry(e) if !rec => mask_blit(dc, sf, pen >> 8, base, e),
            GcRef::Entry(_) => {
                let key = glyphcache::gc_key(font, g, size, phase);
                if let (Some(e), Some(r)) = (self.gc.map.get(&key), self.rec.as_mut()) {
                    r.mask(key, e, pen >> 8, base, dc.color);
                }
            }
            GcRef::Direct if rec => {
                if let Some(r) = self.rec.as_mut() {
                    r.fail(); // a filled path: not in the cache
                }
            }
            GcRef::Direct => {
                // same placement as a cache entry: integer x + quarter-pixel phase
                let (sx, sy) = self.gc_scale(font, size);
                self.gp = Gp { x: (pen >> 6) as f32 * 0.25, y: base as f32, sx, sy, dx: 0, dy: 0 };
                self.path.begin(dc);
                self.glyph_path(font, g);
                self.path.fill(&mut self.ras, dc, sf, 0);
            }
        }
    }

    // a colour glyph: true drawn / false (not a colour glyph: draw it normally)  [color_glyph]
    fn color_glyph(&mut self, dc: &mut Dc, sf: &mut Surface, font: u32, g: u32, pen: i32) -> bool {
        if self.hdr(font).flags & SGH_CBDT != 0 {
            let size = self.size;
            let base = self.run_y;
            let rec = self.rec.is_some();
            match self.gc_get_bitmap(font, g, size) {
                Some(e) if !rec => {
                    bitmap_blit(dc, sf, pen >> 8, base, e);
                    return true;
                }
                Some(_) => {
                    // a bitmap above the cache's limit is not in the table: not recordable
                    let key = glyphcache::bitmap_key(font, g, size);
                    match (self.gc.map.get(&key), self.rec.as_mut()) {
                        (Some(e), Some(r)) => r.bitmap(key, e, pen >> 8, base, dc.color),
                        (None, Some(r)) => r.fail(),
                        _ => {}
                    }
                    return true;
                }
                None => {}
            }
        }
        let mut layers = core::mem::take(&mut self.layers);
        self.layers(font, g, LAYER_CAP, &mut layers);
        if layers.is_empty() {
            self.layers = layers;
            return false;
        }
        let saved = dc.get_color();
        for &(lg, argb) in &layers {
            let c = if lg & LAYER_FG != 0 {
                saved
            } else {
                // palette colour, alpha x the text colour's alpha
                let a = div255((argb >> 24) * (saved >> 24));
                (a << 24) | (argb & 0xFF_FFFF)
            };
            dc.set_color(c);
            self.glyph_draw(dc, sf, font, lg & 0xFFFF, pen, false);
        }
        dc.set_color(saved);
        self.layers = layers;
        true
    }

    /// text_caret_x(s, index) -> x px (rounded) of the caret before byte index
    pub fn caret_x(&mut self, s: &[u8], index: usize) -> i32 {
        if self.shape_line(s,None) { return self.shaped_caret(index); }
        (self.measure_n(s, index) + 128) >> 8
    }

    /// text_hit(s, len, x) -> byte index whose caret position is closest to x (len None:
    /// up to NUL)
    pub fn hit(&mut self, s: &[u8], len: Option<usize>, x: i32) -> usize {
        if self.shape_line(s,len) { return self.shaped_hit(x); }
        let xf = x << 8;
        let mut i = 0usize;
        let mut pen = 0i32;
        loop {
            if let Some(l) = len
                && i >= l
            {
                break;
            }
            if i >= s.len() || s[i] == 0 || s[i] == b'\n' {
                break;
            }
            let next = utf8_next(s, i);
            let p = self.measure_n(s, next);
            // past the midpoint between pen(index) and pen(next)?
            if xf < (pen.wrapping_add(p)) >> 1 {
                break;
            }
            pen = p;
            i = next;
        }
        i
    }

    /// text_decor(x, y, w, flags): underline / strikeout rectangles of the current font and
    /// size (filled from x, w px long; y = baseline)
    pub fn decor(&self, dc: &Dc, sf: &mut Surface, x: i32, y: i32, w: i32, flags: u32) {
        let m = self.metrics(self.font).unwrap_or_default();
        if flags & TD_UNDERLINE != 0 {
            self.td_rect(dc, sf, x, y, w, m.ul_pos, m.ul_thick);
        }
        if flags & TD_STRIKEOUT != 0 {
            self.td_rect(dc, sf, x, y, w, m.st_pos, m.st_thick);
        }
    }

    // the rectangle whose top is pos font units above the baseline
    fn td_rect(&self, dc: &Dc, sf: &mut Surface, x: i32, y: i32, w: i32, pos: i32, thick: i32) {
        let top = y.wrapping_sub(cvt(pos as f32 * self.sx));
        let th = cvt(thick as f32 * self.sx).max(1);
        dc.fill(sf, x, top, w, th);
    }

    /// icon_draw(cp, x, y, size_fx): the icon's em box has its top-left corner at x, y
    pub fn icon_draw(&mut self, dc: &mut Dc, sf: &mut Surface, cp: u32, x: i32, y: i32, size_fx: i32) {
        let saved = self.get_font();
        self.set_font(FONT_ICON, size_fx);
        let mut buf = Vec::new();
        utf8_put(&mut buf, cp);
        let asc = self.ascent(); // the icon font's ascender is its em height
        self.draw(dc, sf, x, y + asc, &buf);
        self.set_font(saved.0, saved.1);
    }
}

// the CBDT hooks' defaults: img_png's decoder (PARGB) and img_scale's bilinear copy
fn png_decode(d: &[u8]) -> Option<Surface> {
    let mut sf = Surface::new(0, 0, gfx::FMT_PARGB);
    if img_png::dec::decode(d, &mut sf) == 0 { Some(sf) } else { None }
}

fn img_scale(src: &Surface, w: i32, h: i32) -> Option<Surface> {
    img_core::ops::scale(src, w, h, true).ok()
}

// the index after the UTF-8 sequence at i (utf8_next)
fn utf8_next(s: &[u8], i: usize) -> usize {
    base::utf8_decode(s, i).1
}

// blend a glyph mask with the colour of dc at pen x / baseline (absolute target
// coordinates), clipped  [mask_blit]
pub(crate) fn mask_blit(dc: &Dc, sf: &mut Surface, pen_x: i32, base: i32, gl: &GcEntry) {
    let mut x0 = pen_x.wrapping_add(gl.left);
    let mut y0 = base.wrapping_add(gl.top);
    let mut sx = 0i32;
    let mut sy = 0i32;
    let x1 = x0.wrapping_add(gl.w).min(dc.clip[2]);
    let y1 = y0.wrapping_add(gl.h).min(dc.clip[3]);
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
    if w <= 0 || h <= 0 {
        return;
    }
    let stride = sf.stride as usize;
    for r in 0..h as usize {
        let mo = (sy as usize + r) * gl.w as usize + sx as usize;
        let mask = &gl.px[mo..mo + w as usize];
        let o = ((y0 - dc.row0) as usize + r) * stride;
        if sf.fmt == FMT_A8 {
            blend::a8_over(&mut sf.px[o + x0 as usize..o + x1 as usize], mask, dc.pcol32 >> 24);
        } else {
            blend::mask_over(&mut sf.px[o + x0 as usize * 4..o + x1 as usize * 4], mask, &dc.pcol);
        }
    }
}

// draw a PARGB glyph bitmap (absolute target coordinates), clipped, multiplied by the
// colour's alpha and the global opacity  [bitmap_blit]
pub(crate) fn bitmap_blit(dc: &Dc, sf: &mut Surface, pen_x: i32, base: i32, e: &GcEntry) {
    if sf.fmt == FMT_A8 {
        return;
    }
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
    if w <= 0 || h <= 0 {
        return;
    }
    let op = div255((dc.color >> 24) * dc.opac);
    let stride = sf.stride as usize;
    let sstride = e.w as usize * 4;
    for r in 0..h as usize {
        let so = (sy as usize + r) * sstride + sx as usize * 4;
        let o = ((y0 - dc.row0) as usize + r) * stride + x0 as usize * 4;
        blend::blit_over(&mut sf.px[o..o + w as usize * 4], &e.px[so..so + w as usize * 4], op);
    }
}
