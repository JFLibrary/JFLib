// registry -- the font registry: embedded SGF1 fonts, runtime fonts (ttf.rs) and the derived
// fonts of font_style (synthetic bold / italic / sharp)       [src/text/font.s, outline.s]
//
//   register(data) -> id / -1            add an SGF1 font
//   load_mem(data, face) / load_file(path, face) -> id / ERR_*   runtime fonts (ttf.rs)
//   style(font, flags) -> id / ERR_*     derived font with FS_SYN_BOLD (outline dilated by
//        upem/30, advance + upem/30), FS_SYN_ITALIC (x += y * tan 12 deg) and/or FS_SHARP
//        (vertical grid fitting); flags 0 = font itself; the same (font, flags) pair always
//        gives the same id
//   unload(id) -> 0 / -1                 a runtime font and every font derived from it; the
//        glyph cache is flushed; the id may be reused later
//   count(), valid(id), hdr(font) (font_data), glyph(font, cp), has, adv, kern, kern_table,
//   upem, metrics, metric, is_color, name, layers, glyph_box, glyph_path
//
// Font ids 0..3 are the embedded fonts (never reused).  An unknown or unloaded id behaves
// like font 0.

use alloc::boxed::Box;
use alloc::vec::Vec;

use rt::fx::cvt;

use crate::colr;
use crate::font::*;
use crate::outline::{Zones, F_INV30, F_TAN12};
use crate::ttf::{self, TtFont, ERR_FAIL, ERR_IO, ERR_LIMIT, FILE_MAX};
use crate::Text;

pub const FONT_EMBEDDED: u32 = 4; // ids below this are the embedded fonts (never unloaded)
pub const FONT_MAX_IDS: u32 = 4096; // registry capacity

/// a font derived by font_style (TFK_SYN context)
pub struct SynFont {
    pub hdr: Hdr,
    pub fm: Metrics,
    pub base: u32,
    pub syn: u32,
    pub strength: f32, // embolden strength in font units
    pub serial: u64,
}

pub(crate) enum Face {
    Free,
    Sgf(Font),
    Tt(Box<TtFont>),
    Syn(Box<SynFont>),
}

pub(crate) struct Slot {
    pub face: Face,
    pub raster: crate::FontRaster,
    pub fmap: Vec<u16>, // glyph ids for cp < FAST_CP (runtime fonts; empty: slow path)
}

impl Slot {
    fn free() -> Slot {
        Slot { face: Face::Free, raster: crate::FontRaster::Legacy, fmap: Vec::new() }
    }

    fn is_free(&self) -> bool {
        matches!(self.face, Face::Free)
    }
}

static S_SGF1: &[u8] = b"SGF1";
static S_REGULAR: &[u8] = b"Regular";

impl Text {
    // ------------------------------------------------------------------ slots

    /// font_slot: the registry index used for font (unknown / free ids: font 0); None when no
    /// font is registered
    pub(crate) fn slot_idx(&self, font: u32) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let i = font as usize;
        if i < self.slots.len() && !self.slots[i].is_free() { Some(i) } else { Some(0) }
    }

    pub(crate) fn face(&self, font: u32) -> Option<&Face> {
        self.slot_idx(font).map(|i| &self.slots[i].face)
    }

    // a free id (ids below FONT_EMBEDDED are never reused) or None  [font_slot_alloc]
    fn slot_alloc(&mut self) -> Option<usize> {
        for i in FONT_EMBEDDED as usize..self.slots.len() {
            if self.slots[i].is_free() {
                return Some(i);
            }
        }
        if self.slots.len() >= FONT_MAX_IDS as usize {
            return None;
        }
        self.slots.push(Slot::free());
        Some(self.slots.len() - 1)
    }

    /// font_register(data) -> id / -1
    pub fn register(&mut self, data: &'static [u8]) -> i32 {
        let Some(f) = Font::parse(data) else { return -1 };
        let Some(i) = self.slot_alloc() else { return -1 };
        self.slots[i] = Slot { face: Face::Sgf(f), raster: crate::FontRaster::Legacy, fmap: Vec::new() };
        i as i32
    }

    // register a runtime face; the fast table through its glyph op  [font_add_rt]
    fn add_rt(&mut self, face: Face) -> i32 {
        let mut fmap = Vec::with_capacity(FAST_CP as usize);
        for cp in 0..FAST_CP {
            let g = match &face {
                Face::Tt(t) => t.glyph(cp),
                Face::Syn(s) => self.glyph(s.base, cp),
                _ => 0,
            };
            fmap.push(g as u16);
        }
        let Some(i) = self.slot_alloc() else { return ERR_LIMIT };
        let raster = match &face { Face::Syn(s) => self.font_raster(s.base), _ => crate::FontRaster::Legacy };
        self.slots[i] = Slot { face, raster, fmap };
        i as i32
    }

    fn next_serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    /// font_face_count(data) -> faces (1 for a plain sfnt, n for a TTC / OTC) / ERR_*
    pub fn face_count(data: &[u8]) -> i32 {
        ttf::face_count(data)
    }

    /// font_load_mem(data, face) -> id / ERR_* (the data is copied)
    pub fn load_mem(&mut self, data: &[u8], face: u32) -> i32 {
        self.load_vec(Vec::from(data), face)
    }

    fn load_vec(&mut self, data: Vec<u8>, face: u32) -> i32 {
        let serial = self.next_serial();
        match TtFont::open(data, face, &mut self.ol, serial) {
            Ok(t) => self.add_rt(Face::Tt(t)),
            Err(e) => e,
        }
    }

    /// font_load_file(path, face) -> id / ERR_* (ERR_IO when it cannot be read)
    pub fn load_file(&mut self, path: &str, face: u32) -> i32 {
        let Some(f) = plat::File::open(path, plat::FO_READ) else { return ERR_IO };
        let size = f.size().unwrap_or(0);
        if size == 0 {
            return ERR_IO;
        }
        if size > FILE_MAX {
            return ERR_LIMIT;
        }
        let mut buf = alloc::vec![0u8; size as usize];
        if f.read_at(0, &mut buf) != Some(size as usize) {
            return ERR_IO;
        }
        drop(f);
        self.load_vec(buf, face)
    }

    /// font_unload(id) -> 0 / -1
    pub fn unload(&mut self, id: u32) -> i32 {
        let i = id as usize;
        if id < FONT_EMBEDDED || i >= self.slots.len() || self.slots[i].is_free() {
            return -1;
        }
        // dependents no longer find it
        let old = core::mem::replace(&mut self.slots[i].face, Face::Free);
        // unload fonts derived from this one first
        let mut j = 0;
        while j < self.slots.len() {
            let dep = match &self.slots[j].face {
                Face::Tt(t) => t.hdr.baseid == id + 1,
                Face::Syn(s) => s.hdr.baseid == id + 1,
                _ => false,
            };
            if dep {
                self.unload(j as u32);
            }
            j += 1;
        }
        drop(old);
        if i < self.slots.len() {
            self.slots[i] = Slot::free();
        }
        // drop trailing free slots
        while self.slots.len() > FONT_EMBEDDED as usize && self.slots[self.slots.len() - 1].is_free() {
            self.slots.pop();
        }
        self.gc_flush(); // cached masks are keyed by the id
        0
    }

    /// font_count() -> ids in use are below this value
    pub fn count(&self) -> u32 {
        self.slots.len() as u32
    }

    /// font_valid(id): the id names a registered font
    pub fn valid(&self, id: u32) -> bool {
        (id as usize) < self.slots.len() && !self.slots[id as usize].is_free()
    }

    /// font_data: the SGF1-compatible header (an unknown id gives font 0)
    pub fn hdr(&self, font: u32) -> Hdr {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.hdr(),
            Some(Face::Tt(t)) => t.hdr,
            Some(Face::Syn(s)) => s.hdr,
            _ => Hdr::default(),
        }
    }

    pub fn upem(&self, font: u32) -> i32 {
        if self.slots.is_empty() { 2048 } else { self.hdr(font).upem }
    }

    /// font_glyph(font, cp) -> glyph (SGF1: fallback glyph, runtime: 0 = .notdef)
    pub fn glyph(&self, font: u32, cp: u32) -> u32 {
        if cp < FAST_CP && (font as usize) < self.slots.len() {
            let s = &self.slots[font as usize];
            if !s.fmap.is_empty() {
                return s.fmap[cp as usize] as u32;
            }
            if let Face::Sgf(f) = &s.face {
                return f.glyph(cp);
            }
        }
        match self.face(font) {
            Some(Face::Sgf(f)) => f.glyph(cp),
            Some(Face::Tt(t)) => t.glyph(cp),
            Some(Face::Syn(s)) => self.glyph(s.base, cp),
            _ => 0,
        }
    }

    /// font_has(font, cp)
    pub fn has(&self, font: u32, cp: u32) -> bool {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.lookup(cp).is_some(),
            Some(Face::Tt(t)) => t.has(cp),
            Some(Face::Syn(s)) => self.has(s.base, cp),
            _ => false,
        }
    }

    /// font_adv(font, glyph): advance in font units (synthetic bold included)
    pub fn adv(&self, font: u32, g: u32) -> i32 {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.adv(g),
            Some(Face::Tt(t)) => t.adv(g),
            Some(Face::Syn(s)) => {
                let a = self.adv(s.base, g);
                if s.syn & FS_SYN_BOLD != 0 { a.wrapping_add(cvt(s.strength)) } else { a }
            }
            _ => 0,
        }
    }

    /// font_kern(font, g1, g2): kerning in font units (signed)
    pub fn kern(&self, font: u32, g1: u32, g2: u32) -> i32 {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.kern(g1, g2),
            Some(Face::Tt(t)) => t.kern(g1, g2),
            Some(Face::Syn(s)) => self.kern(s.base, g1, g2),
            _ => 0,
        }
    }

    /// font_kern_table: the 'kern' table only (runtime fonts; others: font_kern)
    pub fn kern_table(&self, font: u32, g1: u32, g2: u32) -> i32 {
        match self.face(font) {
            Some(Face::Tt(t)) => t.kern_table(g1, g2),
            Some(_) => self.kern(font, g1, g2),
            None => 0,
        }
    }

    /// font_is_color: COLR layers or CBDT bitmaps
    pub fn is_color(&self, font: u32) -> bool {
        self.slot_idx(font).is_some() && self.hdr(font).flags & (SGH_COLR | SGH_CBDT) != 0
    }

    /// font_layers(font, glyph, cap) into out: {glyph | LAYER_FG, argb}; empty = draw it
    /// normally
    pub fn layers(&self, font: u32, g: u32, cap: usize, out: &mut Vec<(u32, u32)>) {
        out.clear();
        match self.face(font) {
            Some(Face::Tt(t)) if t.hdr.flags & SGH_COLR != 0 => colr::layers(t, g, cap, out),
            Some(Face::Syn(s)) if s.hdr.flags & SGH_COLR != 0 => self.layers(s.base, g, cap, out),
            _ => {}
        }
    }

    /// font_name(font, FN_*) -> UTF-8 ("" when unknown)
    pub fn name(&self, font: u32, which: u32) -> &[u8] {
        let Some(i) = self.slot_idx(font) else { return b"" };
        match &self.slots[i].face {
            Face::Sgf(_) => {
                if i < FONT_EMBEDDED as usize {
                    // embedded fonts: the style's names
                    let k = match which {
                        FN_FAMILY => 0,
                        FN_STYLE => 1,
                        FN_FULL => 2,
                        FN_TYPO_FAMILY => 3,
                        FN_TYPO_STYLE => 4,
                        _ => return b"",
                    };
                    return self.names[i][k].as_bytes();
                }
                match which {
                    FN_FAMILY | FN_TYPO_FAMILY | FN_FULL => S_SGF1,
                    FN_STYLE | FN_TYPO_STYLE => S_REGULAR,
                    _ => b"",
                }
            }
            Face::Tt(t) => t.name(which),
            Face::Syn(s) => self.name(s.base, which),
            Face::Free => b"",
        }
    }

    /// font_metrics(font) -> FONTMETRICS (None when no font exists)
    pub fn metrics(&self, font: u32) -> Option<Metrics> {
        let i = self.slot_idx(font)?;
        match &self.slots[i].face {
            Face::Tt(t) => Some(t.fm),
            Face::Syn(s) => Some(s.fm),
            Face::Sgf(f) => {
                let mut m = Metrics {
                    upem: f.upem,
                    asc: f.asc,
                    typo_asc: f.asc,
                    win_asc: f.asc,
                    desc: f.desc,
                    typo_desc: f.desc,
                    win_desc: f.desc.wrapping_neg(),
                    gap: f.gap,
                    typo_gap: f.gap,
                    xh: f.xh,
                    caph: f.caph,
                    ul_pos: f.ulpos,
                    ul_thick: f.ulthick,
                    st_thick: f.ulthick,
                    // strikeout: centred at half the x-height (top of the stroke is stored)
                    st_pos: (f.xh + f.ulthick) >> 1,
                    width: 5,
                    nglyphs: f.nglyphs as i32,
                    ..Default::default()
                };
                m.weight = match i as u32 {
                    1 => 600,
                    2 => 700,
                    _ => 400,
                };
                let mut fl = FMF_EMBED;
                if f.flags & SGH_KERN != 0 {
                    fl |= FMF_KERN;
                }
                if i == 2 {
                    fl |= FMF_BOLD;
                }
                m.flags = fl;
                Some(m)
            }
            Face::Free => None,
        }
    }

    /// font_metric(font, FM_*) -> one FONTMETRICS field
    pub fn metric(&self, font: u32, field: u32) -> i32 {
        self.metrics(font).unwrap_or_default().field(field)
    }

    /// glyph_box(font, glyph) -> {xmin, ymin, xmax, ymax} (font units, control box); None =
    /// empty.  Runtime fonts leave the glyph's outline in the outline buffer.
    pub fn glyph_box(&mut self, font: u32, g: u32) -> Option<[i32; 4]> {
        let i = self.slot_idx(font)?;
        match &self.slots[i].face {
            Face::Sgf(f) => f.glyph_box(g),
            Face::Tt(t) => t.bbox(g, &mut self.ol),
            Face::Syn(_) => self.syn_box(i, g),
            Face::Free => None,
        }
    }

    /// glyph_path(font, glyph): append the outline to the path, transformed by gp (origin
    /// x / y in float pixels, sx / sy float pixels per unit)
    pub fn glyph_path(&mut self, font: u32, g: u32) {
        let Some(i) = self.slot_idx(font) else { return };
        match &self.slots[i].face {
            Face::Sgf(f) => {
                let mut gp = self.gp;
                f.path(g, &mut gp, &mut self.path, 0);
            }
            Face::Tt(t) => t.path(g, &mut self.ol, &mut self.path, &self.gp),
            Face::Syn(_) => self.syn_path(i, g),
            Face::Free => {}
        }
    }

    /// OP_BITMAP: the CBDT bitmap of glyph (runtime fonts; derived fonts use their base's)
    pub(crate) fn op_bitmap(&mut self, font: u32, g: u32, size_fx: i32) -> Option<colr::Bitmap> {
        let i = self.slot_idx(font)?;
        match &self.slots[i].face {
            Face::Tt(t) => colr::bitmap(t, g, size_fx, self.png_decode, self.img_scale, &mut self.cb_metrics),
            Face::Syn(s) => {
                let base = s.base;
                match self.face(base) {
                    Some(Face::Tt(_)) | Some(Face::Syn(_)) => self.op_bitmap(base, g, size_fx),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------ synthetic styles

    pub(crate) fn classic_metrics(&self, font: u32) -> bool {
        matches!(self.face(font), Some(Face::Syn(s)) if s.syn & FS_CLASSIC != 0)
    }

    /// font_style(font, FS_*) -> id / ERR_*
    pub fn style(&mut self, font: u32, flags: u32) -> i32 {
        let mut flags = flags & FS_MASK;
        if !self.valid(font) {
            return ERR_FAIL;
        }
        if flags == 0 {
            return font as i32;
        }
        // a derived font: combine with its flags, derive from its base
        let mut base = font;
        if let Some(Face::Syn(s)) = self.face(font) {
            flags |= s.syn;
            base = s.base;
        }
        // existing (base, flags)?
        for i in 0..self.slots.len() {
            if let Face::Syn(s) = &self.slots[i].face
                && s.base == base
                && s.syn == flags
            {
                return i as i32;
            }
        }
        let mut hdr = self.hdr(base);
        let Some(mut fm) = self.metrics(base) else { return ERR_FAIL };
        hdr.flags = (hdr.flags & (SGH_COLR | SGH_CBDT)) | SGH_RUNTIME;
        hdr.baseid = base + 1;
        // strength = upem / 30
        let strength = fm.upem as f32 * F_INV30;
        fm.flags |= FMF_SYN;
        if flags & FS_SYN_BOLD != 0 {
            fm.flags |= FMF_BOLD;
            fm.weight = fm.weight.wrapping_add(300).clamp(700, 1000);
            fm.advmax = fm.advmax.wrapping_add(cvt(strength));
        }
        if flags & FS_SYN_ITALIC != 0 {
            fm.flags |= FMF_ITALIC;
        }
        let serial = self.next_serial();
        self.add_rt(Face::Syn(Box::new(SynFont { hdr, fm, base, syn: flags, strength, serial })))
    }

    // the transformed outline of glyph into the buffer: true = not empty  [syn_prep]
    fn syn_prep(&mut self, i: usize, g: u32) -> bool {
        let Face::Syn(s) = &self.slots[i].face else { return false };
        let (base, syn, strength, serial) = (s.base, s.syn, s.strength, s.serial);
        let have = match self.slot_idx(base) {
            None => false,
            Some(bi) => match &self.slots[bi].face {
                Face::Sgf(f) => {
                    self.ol.reset();
                    f.outline(g, 0, 0, 0, &mut self.ol);
                    true
                }
                Face::Tt(t) => t.bbox(g, &mut self.ol).is_some(),
                _ => false,
            },
        };
        if !have || self.ol.npts() == 0 {
            self.ol.reset();
            self.ol.owner_set(serial, g);
            return false;
        }
        if syn & FS_SYN_BOLD != 0 {
            self.ol.embolden(strength);
        }
        if syn & FS_SYN_ITALIC != 0 {
            self.ol.shear(F_TAN12);
        }
        self.ol.owner_set(serial, g);
        true
    }

    // [syn_box]
    fn syn_box(&mut self, i: usize, g: u32) -> Option<[i32; 4]> {
        if !self.syn_prep(i, g) {
            return None;
        }
        let mut b = self.ol.bbox()?;
        let Face::Syn(s) = &self.slots[i].face else { return Some(b) };
        if s.syn & FS_SHARP != 0 {
            // grid fitting moves points by up to half a pixel: grow the box vertically
            let grow = ((s.fm.upem as u32 >> 4) + 2) as i32;
            b[1] = b[1].wrapping_sub(grow);
            b[3] = b[3].wrapping_add(grow);
        }
        Some(b)
    }

    // append the transformed outline to the path  [syn_path]
    fn syn_path(&mut self, i: usize, g: u32) {
        let serial = match &self.slots[i].face {
            Face::Syn(s) => s.serial,
            _ => return,
        };
        if !self.ol.owner_is(serial, g) {
            self.syn_prep(i, g);
        }
        if self.ol.npts() == 0 {
            return;
        }
        let Face::Syn(s) = &self.slots[i].face else { return };
        if s.syn & FS_SHARP != 0 {
            let zn = Zones::fit(s.fm.desc, s.fm.xh, s.fm.caph, s.fm.asc, self.gp.sy);
            self.ol.emit(&mut self.path, &self.gp, Some(&zn));
        } else {
            self.ol.emit(&mut self.path, &self.gp, None);
        }
    }
}
