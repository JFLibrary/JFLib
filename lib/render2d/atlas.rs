// atlas -- the glyphs' distance fields of a scene (G6c; docs/RENDER2D.md "Text"): tiles of two size classes in pages of
// PAGE x PAGE texels (RGBA8, the layers of one texture array), a glyph table the shaders read, and what goes up.
//
// A glyph (font, glyph id) gets a row of the glyph table when a text first uses it, and a tile per size class when a text
// needs that class: S (CLASS_S, small and medium text) at once, L (CLASS_L, large text) when a text near the view is drawn
// that large.  A tile goes into the first page with room on a shelf (rows of tiles of about its height, left to right);
// a full page: the next, a new one when all are full.  The texture holds whole pages; when the pages outgrow its layers,
// the scene makes one with twice as many and sends every page again (from the copy here, so that the bytes stay the
// same); at the device's most layers, a glyph that fits no page gets no tile (its text shows nothing of it, Scene::
// text_info counts it).  A new tile sends only its own rectangle: rows of 256 bytes (WebGPU's rule) from the frame's
// upload buffer.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use crate::msdf::{self, Params, Tile};
use crate::text::FontId;

/// a page's side in texels
pub const PAGE: u32 = 1024;
/// size class S: 32 texels per em, distances 8 texels across (4 each side: down to 5.7 px per em a pixel's corners lie
/// within the range); L: 64 texels per em, 4 across (docs/RENDER2D.md "Entscheidungen in G6c": the numbers)
pub const CLASS_S: Params = Params { em: 32.0, range: 8.0, pad: 5 };
pub const CLASS_L: Params = Params { em: 64.0, range: 4.0, pad: 3 };
/// text.wgsl's NONE: a class without a tile
pub const NO_TILE: u32 = 0xFFFF_FFFF;
/// glyph table rows (text.wgsl Glyph): 16 words
pub const GLYPH_WORDS: usize = 16;

/// where a tile lies: its texel box in the page (x | y << 10 | page << 20), its size, its texel origin in the class's em
/// texels (msdf::Tile::x0, y0)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub loc: u32,
    pub w: u32,
    pub h: u32,
    pub x0: i32,
    pub y0: i32,
}

struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

struct Page {
    px: Vec<u8>,
    shelves: Vec<Shelf>,
    top: u32,
}

/// a glyph of the table
#[derive(Clone, Debug)]
pub(crate) struct GlyphRow {
    pub font: FontId,
    pub glyph: u32,
    /// its ink's box (em, from its pen; None: no outline)
    pub ink: Option<[f64; 4]>,
    pub tiles: [Option<Place>; 2],
    /// tried and found no page (the device's most layers)
    pub failed: [bool; 2],
}

impl GlyphRow {
    /// text.wgsl's Glyph
    pub fn words(&self) -> [u32; GLYPH_WORDS] {
        let mut w = [0u32; GLYPH_WORDS];
        let b = self.ink.unwrap_or([0.0; 4]);
        for k in 0..4 {
            w[k] = (b[k] as f32).to_bits();
        }
        for c in 0..2 {
            let o = 4 + 4 * c;
            match self.tiles[c] {
                Some(p) => {
                    w[o] = p.loc;
                    w[o + 1] = p.w | p.h << 16;
                    w[o + 2] = (p.x0 as f32).to_bits();
                    w[o + 3] = (p.y0 as f32).to_bits();
                }
                None => w[o] = NO_TILE,
            }
        }
        w
    }
}

/// the scene's atlas: pages (their bytes here as on the GPU), the glyph table, what changed since the last upload
#[derive(Default)]
pub(crate) struct Atlas {
    pages: Vec<Page>,
    /// the device's most layers (0: not known yet: no limit here)
    pub max_pages: u32,
    pub rows: Vec<GlyphRow>,
    index: BTreeMap<(u32, u32), u32>,
    /// tiles to send: (page, x, y, w, h)
    pub dirty: Vec<(u32, u32, u32, u32, u32)>,
    /// glyph table rows to send (first, end), all of them when the table grew
    pub rows_dirty: Option<(u32, u32)>,
    /// tiles made, glyphs without a page
    pub made: u32,
    pub failed: u32,
    /// what the last prepare sent: bytes of tiles, rows of the table
    pub sent_bytes: u64,
    pub sent_rows: u32,
}

impl Atlas {
    /// an empty atlas of at most max_pages pages (the device's layers)
    pub fn new(max_pages: u32) -> Atlas {
        Atlas { max_pages, ..Atlas::default() }
    }

    pub fn pages(&self) -> u32 {
        self.pages.len() as u32
    }

    /// a page's texels (RGBA8, rows of PAGE)
    pub fn page(&self, k: u32) -> &[u8] {
        &self.pages[k as usize].px
    }

    /// the glyph's row of the table (made with its ink box when new)
    pub fn row(&mut self, font: FontId, glyph: u32, ink: Option<[f64; 4]>) -> u32 {
        if let Some(&r) = self.index.get(&(font.0, glyph)) {
            return r;
        }
        let r = self.rows.len() as u32;
        self.rows.push(GlyphRow { font, glyph, ink, tiles: [None, None], failed: [false, false] });
        self.index.insert((font.0, glyph), r);
        self.mark_row(r);
        r
    }

    fn mark_row(&mut self, r: u32) {
        self.rows_dirty = Some(match self.rows_dirty {
            Some((a, b)) => (a.min(r), b.max(r + 1)),
            None => (r, r + 1),
        });
    }

    /// row r has a tile of class c (made from its contours now when not); false when no page takes it
    pub fn ensure(&mut self, r: u32, c: usize, contours: &[alloc::vec::Vec<msdf::Seg>]) -> bool {
        let row = &self.rows[r as usize];
        if row.tiles[c].is_some() || row.ink.is_none() {
            return true;
        }
        if row.failed[c] {
            return false;
        }
        let p = if c == 0 { CLASS_S } else { CLASS_L };
        let Some(tile) = msdf::tile(contours, &p) else { return true };
        self.made += 1;
        match self.place(&tile) {
            Some(place) => {
                self.rows[r as usize].tiles[c] = Some(place);
                self.mark_row(r);
                true
            }
            None => {
                self.rows[r as usize].failed[c] = true;
                self.failed += 1;
                false
            }
        }
    }

    /// a tile into the first page with room (a new page when none has), its bytes copied there
    fn place(&mut self, t: &Tile) -> Option<Place> {
        if t.w > PAGE || t.h > PAGE {
            return None;
        }
        // (shelves of heights in steps of 4 texels, so that tiles of about one size share them)
        let sh = t.h.div_ceil(4) * 4;
        let mut at = None;
        'pages: for (k, pg) in self.pages.iter_mut().enumerate() {
            for s in pg.shelves.iter_mut() {
                if s.h >= sh && s.h <= sh + sh / 4 + 4 && s.x + t.w <= PAGE {
                    at = Some((k, s.x, s.y));
                    s.x += t.w;
                    break 'pages;
                }
            }
            if pg.top + sh <= PAGE {
                pg.shelves.push(Shelf { y: pg.top, h: sh, x: t.w });
                at = Some((k, 0, pg.top));
                pg.top += sh;
                break;
            }
        }
        if at.is_none() {
            if self.max_pages != 0 && self.pages.len() as u32 >= self.max_pages {
                return None;
            }
            self.pages.push(Page { px: vec![0u8; (PAGE * PAGE * 4) as usize], shelves: vec![Shelf { y: 0, h: sh, x: t.w }], top: sh });
            at = Some((self.pages.len() - 1, 0, 0));
        }
        let (k, x, y) = at?;
        let pg = &mut self.pages[k];
        for j in 0..t.h {
            let src = (j * t.w * 4) as usize;
            let dst = (((y + j) * PAGE + x) * 4) as usize;
            pg.px[dst..dst + (t.w * 4) as usize].copy_from_slice(&t.px[src..src + (t.w * 4) as usize]);
        }
        self.dirty.push((k as u32, x, y, t.w, t.h));
        Some(Place { loc: x | y << 10 | (k as u32) << 20, w: t.w, h: t.h, x0: t.x0, y0: t.y0 })
    }

    /// FNV-1a of every page's bytes: the same atlas on every target (tests)
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for p in &self.pages {
            for &b in &p.px {
                h ^= b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }
}
