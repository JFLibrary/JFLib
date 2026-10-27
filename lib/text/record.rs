// record -- glyphs placed now and blended later from the glyph cache, by any thread (the row
// bands of ui's render_par.rs)
//
//   GlyphRec::default() / rec.begin(&text)   an empty record, bound to the cache as it is now
//   text.record_n(&mut rec, dc, sf, x, y, s, len) -> placed   draw_n's glyphs appended to rec
//        instead of blended (dc and sf as draw_n would get them; dc's colour is recorded with
//        every glyph): layout, shaping, fallback fonts and rasterizing the masks into the cache
//        happen now.  false: a glyph cannot come from the cache (too large: a filled path; a
//        colour bitmap above the cache's limit) -- that text must be drawn with draw_n
//   text.record_icon(&mut rec, dc, sf, cp, x, y, size_fx) -> placed   the same for icon_draw
//   rec.len()                          glyphs so far (a text's range: len before .. after)
//   rec.valid(&text)                   the cache was not flushed since begin (the keys hold)
//   text.glyphs() -> GlyphView         the cache read-only, Send + Sync
//   view.draw(&rec, from, to, dc, sf)  blend glyphs from .. to in their colours inside dc's
//                                      clip (dc.row0 honoured): the bytes draw_n gives there
//
// A glyph is its cache key, pen x and baseline (absolute target pixels), its box (cheap
// culling against the clip), the colour and how it is blended (mask, colour bitmap, subpixel
// mask with a copy of the colour's coverage adjustment).  Masks depend only on the key, so
// blending them later, clipped, gives the pixels of drawing them now.

use alloc::vec::Vec;

use gfx::{Dc, Surface};

use crate::glyphcache::GcEntry;
use crate::lcd::{self, Lut};
use crate::{bitmap_blit, mask_blit, Text};

const K_MASK: u8 = 0;
const K_BITMAP: u8 = 1;
const K_LCD_RGB: u8 = 2;
const K_LCD_BGR: u8 = 3;

// one recorded glyph
#[derive(Clone, Copy)]
struct Op {
    key: u64,
    x: i32,
    y: i32,
    /// x0, y0, x1, y1 of its pixels
    b: [i32; 4],
    color: u32,
    kind: u8,
    /// subpixel masks: the coverage adjustment (GlyphRec::adj)
    adj: u16,
}

/// glyphs placed by record_n / record_icon, blended later by GlyphView::draw
#[derive(Default)]
pub struct GlyphRec {
    ops: Vec<Op>,
    /// (rgb, coverage adjustment) of the subpixel colours
    adj: Vec<(u32, [u8; 256])>,
    epoch: u32,
    ok: bool,
}

impl GlyphRec {
    /// empty, bound to the glyph cache as it is now
    pub fn begin(&mut self, t: &Text) {
        self.ops.clear();
        self.adj.clear();
        self.epoch = t.gc.epoch;
        self.ok = true;
    }

    /// glyphs recorded so far
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// the glyph cache was not flushed since begin: every recorded key is still there
    pub fn valid(&self, t: &Text) -> bool {
        self.epoch == t.gc.epoch
    }

    fn push(&mut self, key: u64, e: &GcEntry, x: i32, y: i32, color: u32, kind: u8, adj: u16) {
        if e.w <= 0 || e.h <= 0 {
            return; // nothing to blend (a space)
        }
        let (x0, y0) = (x.wrapping_add(e.left), y.wrapping_add(e.top));
        let b = [x0, y0, x0.wrapping_add(e.w), y0.wrapping_add(e.h)];
        self.ops.push(Op { key, x, y, b, color, kind, adj });
    }

    pub(crate) fn mask(&mut self, key: u64, e: &GcEntry, x: i32, y: i32, color: u32) {
        self.push(key, e, x, y, color, K_MASK, 0);
    }

    pub(crate) fn bitmap(&mut self, key: u64, e: &GcEntry, x: i32, y: i32, color: u32) {
        self.push(key, e, x, y, color, K_BITMAP, 0);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lcd(&mut self, key: u64, e: &GcEntry, x: i32, y: i32, color: u32, bgr: bool, adj: &[u8; 256]) {
        let rgb = color & 0xFF_FFFF;
        let i = match self.adj.iter().position(|a| a.0 == rgb) {
            Some(i) => i,
            None => {
                self.adj.push((rgb, *adj));
                self.adj.len() - 1
            }
        };
        self.push(key, e, x, y, color, if bgr { K_LCD_BGR } else { K_LCD_RGB }, i as u16);
    }

    /// a glyph that cannot be blended from the cache
    pub(crate) fn fail(&mut self) {
        self.ok = false;
    }
}

impl Text {
    /// draw_n's glyphs appended to rec instead of blended (record.rs); false: one of them
    /// cannot be blended from the cache, draw that text with draw_n
    #[allow(clippy::too_many_arguments)]
    pub fn record_n(&mut self, rec: &mut GlyphRec, dc: &mut Dc, sf: &mut Surface, x: i32, y: i32, s: &[u8], len: usize) -> bool {
        rec.ok = true;
        self.rec = Some(core::mem::take(rec));
        self.draw_n(dc, sf, x, y, s, len);
        self.rec_end(rec)
    }

    /// icon_draw's glyph appended to rec (record_n)
    #[allow(clippy::too_many_arguments)]
    pub fn record_icon(&mut self, rec: &mut GlyphRec, dc: &mut Dc, sf: &mut Surface, cp: u32, x: i32, y: i32, size_fx: i32) -> bool {
        rec.ok = true;
        self.rec = Some(core::mem::take(rec));
        self.icon_draw(dc, sf, cp, x, y, size_fx);
        self.rec_end(rec)
    }

    fn rec_end(&mut self, rec: &mut GlyphRec) -> bool {
        *rec = self.rec.take().unwrap_or_default();
        rec.ok
    }

    /// the glyph cache read-only, for blending recorded glyphs on other threads
    pub fn glyphs(&self) -> GlyphView<'_> {
        GlyphView { map: &self.gc.map, lut: self.lcd_lut.as_deref() }
    }
}

/// the glyph cache read-only (Text::glyphs): blends recorded glyphs, from any thread
#[derive(Clone, Copy)]
pub struct GlyphView<'a> {
    map: &'a alloc::collections::BTreeMap<u64, GcEntry>,
    lut: Option<&'a Lut>,
}

impl GlyphView<'_> {
    /// blend the glyphs from .. to of rec in their colours inside dc's clip (rec.valid must
    /// hold: a missing mask is skipped)
    pub fn draw(&self, rec: &GlyphRec, from: usize, to: usize, dc: &mut Dc, sf: &mut Surface) {
        let Some(ops) = rec.ops.get(from..to) else { return };
        for op in ops {
            let c = dc.clip;
            if op.b[0] >= c[2] || op.b[2] <= c[0] || op.b[1] >= c[3] || op.b[3] <= c[1] {
                continue;
            }
            let Some(e) = self.map.get(&op.key) else { continue };
            if dc.color != op.color {
                dc.set_color(op.color);
            }
            match op.kind {
                K_MASK => mask_blit(dc, sf, op.x, op.y, e),
                K_BITMAP => bitmap_blit(dc, sf, op.x, op.y, e),
                _ => {
                    if let (Some(lut), Some(a)) = (self.lut, rec.adj.get(op.adj as usize)) {
                        lcd::blit(dc, sf, op.x, op.y, e, op.kind == K_LCD_BGR, lut, &a.1);
                    }
                }
            }
        }
    }
}
