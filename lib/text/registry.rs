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
// Variable fonts (var.rs):
//   var_axes(font) / var_instances(font)   fvar's axes and named instances (empty: not variable)
//   var_values(font) -> user values per axis (an instance's, else the defaults)
//   vary(font, &[(tag, value)]) -> id / ERR_*   an instance (VarFont: the base font at
//        normalized coordinates; own id, serial and metrics, so glyph cache entries, outline
//        buffer and shaping engines never mix instances); axes not named keep font's values;
//        all-default coordinates give the variable font itself; the same coordinates the same
//        id; synthetic styles of font carry over; unload(base) unloads its instances
//   vary_named(font, i) -> id / ERR_*    named instance i
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
use crate::var::{VarAxis, VarInstance};
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

/// an instance of a variable font (vary): the base font's tables at normalized coordinates
pub(crate) struct VarFont {
    pub hdr: Hdr,
    pub fm: Metrics,
    pub base: u32,
    /// user values per axis and the normalized F2Dot14 coordinates (avar applied)
    pub user: Vec<f32>,
    pub coords: Vec<i16>,
    pub serial: u64,
    /// every glyph's advance when gvar's phantom points give it (no HVAR); else empty
    pub adv: Vec<i32>,
}

pub(crate) enum Face {
    Free,
    Sgf(Font),
    Tt(Box<TtFont>),
    Syn(Box<SynFont>),
    Var(Box<VarFont>),
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

    // the fast glyph table of a runtime face (code points below FAST_CP)
    fn fast_map(&self, face: &Face) -> Vec<u16> {
        let mut fmap = Vec::with_capacity(FAST_CP as usize);
        for cp in 0..FAST_CP {
            let g = match face {
                Face::Tt(t) => t.glyph(cp),
                Face::Syn(s) => self.glyph(s.base, cp),
                Face::Var(v) => self.glyph(v.base, cp),
                _ => 0,
            };
            fmap.push(g as u16);
        }
        fmap
    }

    // register a runtime face; the fast table through its glyph op  [font_add_rt]
    fn add_rt(&mut self, face: Face) -> i32 {
        let fmap = self.fast_map(&face);
        let Some(i) = self.slot_alloc() else { return ERR_LIMIT };
        let raster = match &face {
            Face::Syn(s) => self.font_raster(s.base),
            Face::Var(v) => self.font_raster(v.base),
            _ => crate::FontRaster::Legacy,
        };
        self.slots[i] = Slot { face, raster, fmap };
        i as i32
    }

    /// replace the embedded fonts 0..3 (SGF1 or TrueType / OpenType; a style switched at run
    /// time): the ids keep their meaning, fonts derived from them (style) take the new
    /// metrics, cached glyphs are dropped; each face starts with the legacy raster policy
    pub fn set_embedded(&mut self, fonts: [&'static [u8]; 4], names: [[&'static str; 5]; 4]) {
        for (i, f) in fonts.iter().enumerate() {
            let face = if f.starts_with(b"SGF1") {
                Font::parse(f).map_or(Face::Free, Face::Sgf)
            } else {
                let serial = self.next_serial();
                TtFont::open(Vec::from(*f), 0, &mut self.ol, serial).map_or(Face::Free, Face::Tt)
            };
            let fmap = if matches!(face, Face::Tt(_)) { self.fast_map(&face) } else { Vec::new() };
            while self.slots.len() <= i {
                self.slots.push(Slot::free());
            }
            self.slots[i] = Slot { face, raster: crate::FontRaster::Legacy, fmap };
        }
        self.names = names;
        // instances of the embedded ones: the same axis values on the new base (none: its
        // default)
        for i in FONT_EMBEDDED as usize..self.slots.len() {
            let (base, user) = match &self.slots[i].face {
                Face::Var(v) if v.base < FONT_EMBEDDED => (v.base, v.user.clone()),
                _ => continue,
            };
            let serial = self.next_serial();
            let Some((hdr, fm, coords, adv)) = self.var_build(base, &user) else { continue };
            if let Face::Var(v) = &mut self.slots[i].face {
                (v.hdr, v.fm, v.coords, v.adv, v.serial) = (hdr, fm, coords, adv, serial);
            }
            let fmap = self.fast_map(&self.slots[i].face);
            self.slots[i].fmap = fmap;
            self.slots[i].raster = self.font_raster(base);
        }
        // the derived fonts of the embedded ones: header, metrics and glyph table of the new base
        for i in FONT_EMBEDDED as usize..self.slots.len() {
            let (base, syn) = match &self.slots[i].face {
                Face::Syn(s) if s.base < FONT_EMBEDDED => (s.base, s.syn),
                _ => continue,
            };
            let Some((hdr, fm, strength)) = self.syn_metrics(base, syn) else { continue };
            let serial = self.next_serial();
            if let Face::Syn(s) = &mut self.slots[i].face {
                (s.hdr, s.fm, s.strength, s.serial) = (hdr, fm, strength, serial);
            }
            let fmap = self.fast_map(&self.slots[i].face);
            self.slots[i].fmap = fmap;
            self.slots[i].raster = self.font_raster(base);
        }
        self.gc_flush();
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
                Face::Var(v) => v.hdr.baseid == id + 1,
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
            Some(Face::Var(v)) => v.hdr,
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
            Some(Face::Var(v)) => self.glyph(v.base, cp),
            _ => 0,
        }
    }

    /// font_has(font, cp)
    pub fn has(&self, font: u32, cp: u32) -> bool {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.lookup(cp).is_some(),
            Some(Face::Tt(t)) => t.has(cp),
            Some(Face::Syn(s)) => self.has(s.base, cp),
            Some(Face::Var(v)) => self.has(v.base, cp),
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
            Some(Face::Var(v)) => match (v.adv.get(g as usize), self.face(v.base)) {
                (Some(&a), _) => a,
                (None, Some(Face::Tt(t))) => t.adv_var(g, &v.coords),
                _ => 0,
            },
            _ => 0,
        }
    }

    /// font_kern(font, g1, g2): kerning in font units (signed)
    pub fn kern(&self, font: u32, g1: u32, g2: u32) -> i32 {
        match self.face(font) {
            Some(Face::Sgf(f)) => f.kern(g1, g2),
            Some(Face::Tt(t)) => t.kern(g1, g2),
            Some(Face::Syn(s)) => self.kern(s.base, g1, g2),
            Some(Face::Var(v)) => match self.face(v.base) {
                Some(Face::Tt(t)) => t.kern_var(g1, g2, &v.coords),
                _ => 0,
            },
            _ => 0,
        }
    }

    /// font_kern_table: the 'kern' table only (runtime fonts; others: font_kern)
    pub fn kern_table(&self, font: u32, g1: u32, g2: u32) -> i32 {
        match self.face(font) {
            Some(Face::Tt(t)) => t.kern_table(g1, g2),
            Some(Face::Var(v)) => self.kern_table(v.base, g1, g2),
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
            Some(Face::Var(v)) if v.hdr.flags & SGH_COLR != 0 => self.layers(v.base, g, cap, out),
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
            Face::Var(v) => self.name(v.base, which),
            Face::Free => b"",
        }
    }

    /// font_metrics(font) -> FONTMETRICS (None when no font exists)
    pub fn metrics(&self, font: u32) -> Option<Metrics> {
        let i = self.slot_idx(font)?;
        match &self.slots[i].face {
            Face::Tt(t) => Some(t.fm),
            Face::Syn(s) => Some(s.fm),
            Face::Var(v) => Some(v.fm),
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
            Face::Var(_) => {
                if self.var_prep(i, g) { self.ol.bbox() } else { None }
            }
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
            Face::Var(v) => {
                let serial = v.serial;
                if !self.ol.owner_is(serial, g) {
                    self.var_prep(i, g);
                }
                if self.ol.npts() != 0 {
                    self.ol.emit(&mut self.path, &self.gp, None);
                }
            }
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
                    Some(Face::Tt(_)) | Some(Face::Syn(_)) | Some(Face::Var(_)) => self.op_bitmap(base, g, size_fx),
                    _ => None,
                }
            }
            Face::Var(v) => {
                let base = v.base;
                self.op_bitmap(base, g, size_fx)
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------ variable fonts

    // the variable font under font: (its id, the instance's user values or None for the
    // defaults, the synthetic style flags on top)
    fn var_root(&self, font: u32) -> Option<(u32, Option<&[f32]>, u32)> {
        let (mut f, mut syn) = (font, 0);
        if let Some(Face::Syn(s)) = self.face(f) {
            syn = s.syn;
            f = s.base;
        }
        let i = self.slot_idx(f)?;
        match &self.slots[i].face {
            Face::Var(v) => Some((v.base, Some(&v.user), syn)),
            Face::Tt(t) if t.var.is_some() => Some((i as u32, None, syn)),
            _ => None,
        }
    }

    fn var_tt(&self, base: u32) -> Option<&TtFont> {
        match self.face(base) {
            Some(Face::Tt(t)) => Some(t),
            _ => None,
        }
    }

    /// the variation axes of a font (fvar; empty: not a variable font); instances and the
    /// fonts derived from them report their variable font's
    pub fn var_axes(&self, font: u32) -> Vec<VarAxis> {
        self.var_root(font).and_then(|(b, _, _)| self.var_tt(b)).and_then(|t| t.var.as_ref()).map_or(Vec::new(), |v| v.axes.clone())
    }

    /// the named instances of a variable font (fvar; empty: none or not variable)
    pub fn var_instances(&self, font: u32) -> Vec<VarInstance> {
        self.var_root(font).and_then(|(b, _, _)| self.var_tt(b)).and_then(|t| t.var.as_ref()).map_or(Vec::new(), |v| v.instances.clone())
    }

    /// the axis values of a font in user units, one per axis of var_axes (an instance's own,
    /// else the defaults)
    pub fn var_values(&self, font: u32) -> Vec<f32> {
        match self.var_root(font) {
            Some((_, Some(u), _)) => u.to_vec(),
            Some(_) => self.var_axes(font).iter().map(|a| a.def).collect(),
            None => Vec::new(),
        }
    }

    /// the normalized coordinates of an instance: F2Dot14 bits per axis with avar applied
    /// (what HarfBuzz's hb_font_get_var_coords_normalized reports); empty for a variable font
    /// itself (its default instance) and for fonts that are not variable
    pub fn var_coords(&self, font: u32) -> Vec<i16> {
        let mut f = font;
        if let Some(Face::Syn(s)) = self.face(f) {
            f = s.base;
        }
        match self.face(f) {
            Some(Face::Var(v)) => v.coords.clone(),
            _ => Vec::new(),
        }
    }

    /// an instance of a variable font with axis values in user units (tag, value): axes not
    /// named keep font's values (an instance's, else the defaults); values are clamped to their
    /// axis, unknown tags ignored -> font id / ERR_*.  All-default coordinates give the variable
    /// font itself, the same coordinates the same id; synthetic styles of font (style) carry
    /// over; a font that is not variable gives itself.  Each instance has its own glyph cache
    /// entries and metrics (MVAR); unloading the variable font unloads its instances.
    pub fn vary(&mut self, font: u32, values: &[([u8; 4], f32)]) -> i32 {
        if !self.valid(font) {
            return ERR_FAIL;
        }
        let Some((base, cur, syn)) = self.var_root(font) else { return font as i32 };
        let axes = self.var_axes(base);
        let mut user: Vec<f32> = match cur {
            Some(u) => u.to_vec(),
            None => axes.iter().map(|a| a.def).collect(),
        };
        for &(tag, v) in values {
            if let Some(k) = axes.iter().position(|a| a.tag == tag) {
                user[k] = if v.is_nan() { axes[k].def } else { v.clamp(axes[k].min, axes[k].max) };
            }
        }
        let id = self.var_instance(base, &user);
        if id < 0 || syn == 0 { id } else { self.style(id as u32, syn) }
    }

    /// named instance i of a variable font (var_instances) -> id / ERR_* (ERR_FAIL: no such
    /// instance)
    pub fn vary_named(&mut self, font: u32, i: usize) -> i32 {
        let ins = self.var_instances(font);
        let Some(inst) = ins.get(i) else { return ERR_FAIL };
        let axes = self.var_axes(font);
        let vals: Vec<([u8; 4], f32)> = axes.iter().zip(&inst.coords).map(|(a, &c)| (a.tag, c)).collect();
        self.vary(font, &vals)
    }

    // the instance of variable font base at user values (all-default: base itself)
    fn var_instance(&mut self, base: u32, user: &[f32]) -> i32 {
        let Some(coords) = self.var_tt(base).and_then(|t| t.var.as_ref()).map(|v| v.normalize(user)) else { return base as i32 };
        if coords.iter().all(|&c| c == 0) {
            return base as i32;
        }
        for i in FONT_EMBEDDED as usize..self.slots.len() {
            if let Face::Var(v) = &self.slots[i].face
                && v.base == base
                && v.coords == coords
            {
                return i as i32;
            }
        }
        let Some((hdr, fm, coords, adv)) = self.var_build(base, user) else { return ERR_FAIL };
        let serial = self.next_serial();
        self.add_rt(Face::Var(Box::new(VarFont { hdr, fm, base, user: user.to_vec(), coords, serial, adv })))
    }

    // header, metrics, coordinates and phantom-point advances of base's instance at user
    // values (weight, italic and oblique flags from wght / ital / slnt when the font has them)
    fn var_build(&mut self, base: u32, user: &[f32]) -> Option<(Hdr, Metrics, Vec<i16>, Vec<i32>)> {
        let bi = self.slot_idx(base)?;
        let Face::Tt(t) = &self.slots[bi].face else { return None };
        let v = t.var.as_ref();
        let coords = v.map_or(Vec::new(), |v| v.normalize(user));
        let axes: &[VarAxis] = v.map_or(&[], |v| &v.axes);
        let val = |tag: &[u8; 4]| axes.iter().position(|a| &a.tag == tag).and_then(|k| user.get(k).copied());
        let (wght, ital, slnt) = (val(b"wght"), val(b"ital"), val(b"slnt"));
        let (mut hdr, mut fm) = {
            let (m, h) = t.metrics_var(&coords, wght, &mut self.ol);
            (h, m)
        };
        let adv = t.adv_table(&coords, &mut self.ol);
        hdr.baseid = base + 1;
        if let Some(w) = wght {
            fm.flags = (fm.flags & !FMF_BOLD) | if w >= 600.0 { FMF_BOLD } else { 0 };
        }
        if let Some(i) = ital {
            fm.flags = (fm.flags & !FMF_ITALIC) | if i >= 0.5 { FMF_ITALIC } else { 0 };
        }
        if let Some(s) = slnt {
            fm.flags = (fm.flags & !FMF_OBLIQUE) | if s != 0.0 { FMF_OBLIQUE } else { 0 };
        }
        Some((hdr, fm, coords, adv))
    }

    // the outline of the instance in slot i into the buffer (owner: the instance)
    fn var_prep(&mut self, i: usize, g: u32) -> bool {
        let Face::Var(v) = &self.slots[i].face else { return false };
        let Some(bi) = self.slot_idx(v.base) else { return false };
        let (Face::Var(v), Face::Tt(t)) = (&self.slots[i].face, &self.slots[bi].face) else { return false };
        t.prep_var(g, &v.coords, v.serial, &mut self.ol)
    }

    // (shaping.rs) a HarfRust engine for font: the font file at the instance's coordinates
    // (synthetic styles shape like their base)
    pub(crate) fn shape_engine(&self, mut font: u32) -> Option<text_shape::Engine> {
        let mut coords: &[i16] = &[];
        for _ in 0..16 {
            match self.face(font)? {
                Face::Tt(t) => {
                    let e = text_shape::Engine::new(&t.data, t.face_index)?;
                    return Some(if coords.is_empty() { e } else { e.with_coords(coords) });
                }
                Face::Syn(s) => font = s.base,
                Face::Var(v) => {
                    if coords.is_empty() {
                        coords = &v.coords;
                    }
                    font = v.base;
                }
                _ => return None,
            }
        }
        None
    }

    // (shaping.rs) the serial that names font's shaping engine (an instance's own)
    pub(crate) fn shape_serial(&self, mut font: u32) -> u64 {
        for _ in 0..16 {
            match self.face(font) {
                Some(Face::Tt(t)) => return t.serial,
                Some(Face::Var(v)) => return v.serial,
                Some(Face::Syn(s)) => font = s.base,
                _ => return 0,
            }
        }
        0
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
        let Some((hdr, fm, strength)) = self.syn_metrics(base, flags) else { return ERR_FAIL };
        let serial = self.next_serial();
        self.add_rt(Face::Syn(Box::new(SynFont { hdr, fm, base, syn: flags, strength, serial })))
    }

    // header, metrics and embolden strength of the font derived from base with flags
    fn syn_metrics(&self, base: u32, flags: u32) -> Option<(Hdr, Metrics, f32)> {
        let mut hdr = self.hdr(base);
        let mut fm = self.metrics(base)?;
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
        Some((hdr, fm, strength))
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
                Face::Var(_) => self.var_prep(bi, g),
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
