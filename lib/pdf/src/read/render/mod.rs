// Drawing pages (ISO 32000-1, 8 to 11): Document::render gives the page as pixels.  The content
// stream runs through a graphics state (q / Q, cm, the line style, colours in every colour
// space, ExtGState with alpha, blend mode and soft mask), paths are filled with the non-zero or
// even-odd rule and stroked with caps, joins, miter limit and dashes in user space (also under
// a non-uniform matrix), clipping paths and text clipping are masks.  Text through the glyph
// outlines of the embedded fonts (Type 1, CFF, TrueType, OpenType, Type 3) or substitutes
// (FontSource); every text render mode.  Images with every bit depth and colour space, Decode,
// stencil masks, colour-key masks, soft masks, the JPEG filter (through img_jpeg) and
// CCITTFax; shadings of every type (sh and as patterns), tiling patterns coloured and
// uncoloured; form XObjects, transparency groups and soft masks (luminosity and alpha, with
// backdrop and transfer function); the annotations' appearances; optional content (the
// default configuration, membership dictionaries).  Not here: JPXDecode and JBIG2Decode images
// (left out), knockout and non-isolated groups (drawn as isolated ones), overprint, halftones,
// transfer functions outside soft masks.
//
//   let doc = read::Document::open(bytes)?;
//   let page: gfx::Surface = doc.render(0, &RenderOptions { dpi: 150.0, ..Default::default() })?;
//
// Untrusted input: the work of a page is bounded (the operators, the pixels painted, forms,
// patterns, Type 3 glyphs and soft masks inside each other, the pixels of images and layers),
// a page that needs more gives Error::Limit.

mod canvas;
mod color;
mod fax;
mod fonts;
mod image;
mod shade;
mod type1;

use super::content::Ops;
use super::object::{Dict, Object, Ref};
use super::{Document, Error, Page};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use canvas::{Blend, Clip, Layer, Mask, Put, Rect, Src};
use color::Cs;
use core::cell::{OnceCell, RefCell};
use gfx::dash::{dash_stroke, DStyle};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{RULE_EVENODD, RULE_NONZERO};

/// how to draw a page
#[derive(Clone)]
pub struct RenderOptions {
    /// pixels per inch: 72 draws a point as a pixel
    pub dpi: f64,
    /// the part of the page image to draw, in pixels: x, y, width, height (None: all of it)
    pub region: Option<[u32; 4]>,
    /// the colour under the page, straight 0xAARRGGBB (None: transparent)
    pub background: Option<u32>,
    /// draw the annotations' appearances (form fields, stamps, comments)
    pub annotations: bool,
    /// fonts for the fonts a document does not embed (None: the system's, text::fontdb)
    pub fonts: Option<Rc<dyn FontSource>>,
    /// the largest image to draw, in pixels
    pub max_pixels: u64,
}

impl Default for RenderOptions {
    fn default() -> RenderOptions {
        RenderOptions { dpi: 72.0, region: None, background: Some(0xFFFF_FFFF), annotations: true, fonts: None, max_pixels: 1 << 28 }
    }
}

/// a font a page uses but does not embed
#[derive(Clone, Debug)]
pub struct FontQuery<'a> {
    /// its PostScript name without a subset tag (Helvetica-Bold, ArialMT, ...)
    pub name: &'a str,
    pub family: Option<&'a str>,
    pub bold: bool,
    pub italic: bool,
    /// serifs, a fixed pitch, a symbol font (by the descriptor's flags)
    pub serif: bool,
    pub fixed: bool,
    pub symbolic: bool,
    /// a simple font (one byte per code), not a composite one
    pub text: bool,
}

/// where substitutes for fonts that are not embedded come from: a TrueType, OpenType or Type 1
/// file and the face in it
pub trait FontSource {
    fn find(&self, q: &FontQuery) -> Option<(Vec<u8>, u32)>;
}

/// the system's fonts (text::fontdb), searched once: for the 14 standard fonts their
/// metric-compatible families (URW's Nimbus, Liberation, the Windows fonts), others by name and
/// flags
pub struct SystemFonts {
    db: RefCell<Option<text::fontdb::FontDb>>,
}

impl Default for SystemFonts {
    fn default() -> SystemFonts {
        SystemFonts { db: RefCell::new(None) }
    }
}

impl SystemFonts {
    pub fn new() -> SystemFonts {
        SystemFonts::default()
    }
}

/// the families to try for a font, best first
fn families(q: &FontQuery) -> Vec<&'static str> {
    let n = q.name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    let sans = ["Nimbus Sans", "Liberation Sans", "Arial", "Helvetica", "DejaVu Sans", "Noto Sans"];
    let serif = ["Nimbus Roman", "Liberation Serif", "Times New Roman", "Times", "DejaVu Serif", "Noto Serif"];
    let mono = ["Nimbus Mono PS", "Liberation Mono", "Courier New", "Courier", "DejaVu Sans Mono", "Noto Sans Mono"];
    if has("symbol") {
        return vec!["Standard Symbols PS", "Symbol", "OpenSymbol", "DejaVu Sans"];
    }
    if has("dingbats") {
        return vec!["D050000L", "Dingbats", "DejaVu Sans"];
    }
    if has("courier") || has("mono") || q.fixed {
        return mono.to_vec();
    }
    if has("times") || has("roman") || has("georgia") || has("garamond") || has("serif") && !has("sans") || has("cambria") || has("minion") || has("book") {
        return serif.to_vec();
    }
    if has("helvetica") || has("arial") || has("sans") || has("verdana") || has("calibri") || has("tahoma") || has("frutiger") || has("myriad") {
        return sans.to_vec();
    }
    if q.serif { serif.to_vec() } else { sans.to_vec() }
}

impl FontSource for SystemFonts {
    fn find(&self, q: &FontQuery) -> Option<(Vec<u8>, u32)> {
        let mut db = self.db.borrow_mut();
        let db = db.get_or_insert_with(|| {
            let mut d = text::fontdb::FontDb::new();
            d.scan(0);
            d
        });
        let weight = if q.bold { 700 } else { 400 };
        let mut tries: Vec<String> = Vec::new();
        // the font's own family first (Arial for ArialMT, Calibri, ...)
        let own = q.name.split(['-', ',']).next().unwrap_or(q.name).trim_end_matches("MT").trim_end_matches("PS");
        if !own.is_empty() && !q.name.to_ascii_lowercase().contains("helvetica") && !q.name.to_ascii_lowercase().starts_with("times") {
            tries.push(own.into());
        }
        tries.extend(families(q).into_iter().map(String::from));
        for fam in &tries {
            if let Some((i, _)) = db.find(fam, weight, q.italic) {
                let face = db.face(i)?;
                if !face.family.eq_ignore_ascii_case(fam) && !face.tfamily.eq_ignore_ascii_case(fam) {
                    continue;
                }
                if let Some(data) = plat::read_file(&face.path) {
                    return Some((data, face.face));
                }
            }
        }
        None
    }
}

/// what drawing keeps across the pages of a document: the fonts, the default font source
#[derive(Default)]
pub(crate) struct Cache {
    fonts: RefCell<BTreeMap<Ref, Rc<fonts::GFont>>>,
    /// glyphs drawn: (font, code, the glyph's device matrix, its quarter-pixel phase)
    glyphs: RefCell<BTreeMap<(usize, u32, [u64; 4], u8), (Rc<fonts::GFont>, Option<Rc<canvas::Cover>>)>>,
    glyph_bytes: core::cell::Cell<usize>,
    subst: RefCell<BTreeMap<(String, bool, bool, bool), Option<Rc<fonts::Prog>>>>,
    system: OnceCell<Rc<dyn FontSource>>,
}

pub(super) type M = [f64; 6];
const ID: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// a × b (a first, then b)
pub(super) fn mul(a: &M, b: &M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub(super) fn inv(m: &M) -> Option<M> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-300 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

#[inline]
pub(super) fn apply(m: &M, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn finite(m: M) -> M {
    if m.iter().all(|v| v.is_finite() && v.abs() < 1e12) { m } else { ID }
}

fn nums(ops: &[Object]) -> Vec<f64> {
    ops.iter().map(|o| o.as_f64().filter(|v| v.is_finite()).unwrap_or(0.0)).collect()
}

fn m6(n: &[f64]) -> Option<M> {
    (n.len() >= 6).then(|| [n[0], n[1], n[2], n[3], n[4], n[5]])
}

pub(super) fn to8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// forms, patterns, Type 3 glyphs and soft masks inside each other
const MAX_DEPTH: usize = 12;
const MAX_STACK: usize = 256;
/// the operators of a page, forms counted each time they are drawn
const MAX_OPS: u64 = 1 << 23;
/// the bytes of glyph coverage kept for a document
const GLYPH_CACHE: usize = 16 << 20;
/// the pixels painted for a page: this many per pixel of the page, at least MIN_WORK
const WORK_PER_PX: u64 = 1024;
const MIN_WORK: u64 = 1 << 27;

/// a colour of the graphics state: its space, components, RGB, or a pattern
#[derive(Clone)]
struct Paint {
    cs: Rc<Cs>,
    comps: Vec<f64>,
    rgb: [u8; 3],
    pat: Option<Rc<shade::Pattern>>,
}

impl Paint {
    fn new(cs: Rc<Cs>) -> Paint {
        let comps = cs.initial();
        let c = cs.rgb(&comps);
        Paint { cs, comps, rgb: [to8(c[0]), to8(c[1]), to8(c[2])], pat: None }
    }

    fn set(&mut self, comps: &[f64]) {
        let n = self.cs.n();
        self.comps = comps.iter().take(n).copied().collect();
        self.comps.resize(n, 0.0);
        let c = self.cs.rgb(&self.comps);
        self.rgb = [to8(c[0]), to8(c[1]), to8(c[2])];
    }
}

#[derive(Clone)]
struct Gs {
    ctm: M,
    clip: Clip,
    fill: Paint,
    stroke: Paint,
    lw: f64,
    cap: u32,
    join: u32,
    miter: f64,
    dash: Rc<Vec<f64>>,
    phase: f64,
    fill_alpha: f64,
    stroke_alpha: f64,
    blend: Blend,
    smask: Option<Rc<Mask>>,
    font: Option<Rc<fonts::GFont>>,
    size: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
    mode: i64,
}

#[derive(Clone, Copy)]
enum Seg {
    M(f64, f64),
    L(f64, f64),
    C([f64; 6]),
    Z,
}

/// the resources of a content stream, resolved once
struct Res {
    all: Dict,
    fonts: Dict,
    xobjects: Dict,
    ext: Dict,
    patterns: Dict,
    shadings: Dict,
    props: Dict,
}

impl Res {
    fn of(doc: &Document, res: &Dict) -> Res {
        let get = |k: &str| res.get(k).map(|o| doc.dict(o)).unwrap_or_default();
        Res {
            all: res.clone(),
            fonts: get("Font"),
            xobjects: get("XObject"),
            ext: get("ExtGState"),
            patterns: get("Pattern"),
            shadings: get("Shading"),
            props: get("Properties"),
        }
    }
}

/// a form XObject, decoded once per page
struct Form {
    data: Vec<u8>,
    res: Option<Rc<Res>>,
    matrix: M,
    bbox: Option<[f64; 4]>,
    /// a transparency group
    group: bool,
    oc: Option<Object>,
}

/// what an annotation or a pattern cell draws with: content, resources and the matrix of its
/// space
struct Run<'d> {
    doc: &'d Document,
    opt: &'d RenderOptions,
    layers: Vec<Layer>,
    ras: Raster,
    gs: Gs,
    path: Vec<Seg>,
    cur: (f64, f64),
    start: (f64, f64),
    /// the matrix of the current content stream's default space (patterns are relative to it)
    base: M,
    depth: usize,
    forms: Vec<Ref>,
    form_cache: BTreeMap<Ref, Option<Rc<Form>>>,
    images: BTreeMap<Ref, Option<Rc<image::Img>>>,
    cs_cache: BTreeMap<Ref, Option<Rc<Cs>>>,
    shadings: BTreeMap<Ref, Option<Rc<shade::Shading>>>,
    ops: u64,
    work: u64,
    budget: u64,
    /// the OCGs off in the default configuration, the marked content open (true: hidden)
    oc_off: BTreeSet<Ref>,
    mc: Vec<bool>,
    hidden: usize,
    /// Type 3 glyphs of d1 and uncoloured patterns: colours are not set by the content
    uncolored: bool,
    text_clip: Option<Path>,
    /// a Type 3 glyph was shown in a clipping mode (it does not clip, as in Poppler)
    t3_clip: bool,
    codes: Vec<(u32, u8)>,
    error: Option<Error>,
}

impl Document {
    /// the size of a page drawn at dpi: width and height in pixels (the crop box after /Rotate)
    pub fn page_pixels(&self, page: usize, dpi: f64) -> Result<(u32, u32), Error> {
        let p = self.page(page)?;
        let (w, h) = p.size();
        let s = dpi / 72.0;
        let px = |v: f64| -> u32 { color::ceil(v * s - 1e-6).clamp(1.0, 65_535.0) as u32 };
        Ok((px(w), px(h)))
    }

    /// draw a page into a new surface (premultiplied ARGB, gfx::FMT_PARGB)
    pub fn render(&self, page: usize, opt: &RenderOptions) -> Result<gfx::Surface, Error> {
        let p = self.page(page)?.clone();
        let (w, h) = self.page_pixels(page, opt.dpi)?;
        let [rx, ry, rw, rh] = opt.region.unwrap_or([0, 0, w, h]);
        let (rw, rh) = (rw.min(w.saturating_sub(rx)), rh.min(h.saturating_sub(ry)));
        if rw == 0 || rh == 0 {
            return Err(Error::Limit("the region is outside the page"));
        }
        if (rw as u64) * (rh as u64) > opt.max_pixels.min(1 << 30) {
            return Err(Error::Limit("pixels"));
        }
        // the page is drawn onto nothing (its blend modes see a transparent backdrop, 11.4.7),
        // then onto the paper
        let rect: Rect = [0, 0, rw as i32, rh as i32];
        let layer = Layer::new(rect).ok_or(Error::Limit("memory"))?;
        let dev = device_matrix(&p, opt.dpi, rx as f64, ry as f64);
        let pixels = (rw as u64) * (rh as u64);
        let mut run = Run::new(self, opt, layer, dev, pixels);
        run.oc_off = self.oc_off();
        let res = Rc::new(Res::of(self, &p.resources));
        let content = self.contents(page)?;
        // the page is clipped to its crop box
        run.clip_rect_user(&p.crop_box);
        run.run(&content, &res);
        if opt.annotations {
            run.annotations(&p);
        }
        if let Some(e) = run.error.take() {
            return Err(e);
        }
        let layer = run.layers.pop().ok_or(Error::Limit("layers"))?;
        let mut sf = gfx::Surface::try_new(rw as i32, rh as i32, gfx::FMT_PARGB).ok_or(Error::Limit("memory"))?;
        let stride = sf.stride as usize;
        let paper = opt.background.map(|bg| {
            let a = bg >> 24;
            let pm = |v: u32| canvas::mul255(v & 0xFF, a);
            [pm(bg >> 16), pm(bg >> 8), pm(bg), a]
        });
        let rowb = rw as usize * 4;
        for (src, dst) in layer.px.chunks_exact(rowb).zip(sf.px.chunks_mut(stride)) {
            for (s, d) in src.chunks_exact(4).zip(dst[..rowb].chunks_exact_mut(4)) {
                match (paper, s[3]) {
                    (Some(p), a) if a < 255 => {
                        let ia = 255 - a as u32;
                        d[0] = (s[2] as u32 + canvas::mul255(p[2], ia)) as u8;
                        d[1] = (s[1] as u32 + canvas::mul255(p[1], ia)) as u8;
                        d[2] = (s[0] as u32 + canvas::mul255(p[0], ia)) as u8;
                        d[3] = (a as u32 + canvas::mul255(p[3], ia)) as u8;
                    }
                    _ => {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = s[3];
                    }
                }
            }
        }
        Ok(sf)
    }

    /// the OCGs that are off in the default configuration of the document's optional content
    fn oc_off(&self) -> BTreeSet<Ref> {
        let mut off = BTreeSet::new();
        let props = self.dict(&self.catalog().get("OCProperties").cloned().unwrap_or_default());
        let d = self.dict(&props.get("D").cloned().unwrap_or_default());
        let refs = |o: Option<&Object>| -> Vec<Ref> {
            match o.map(|o| self.resolve(o)) {
                Some(Object::Array(a)) => a.iter().filter_map(Object::as_ref).take(1 << 16).collect(),
                _ => Vec::new(),
            }
        };
        if d.name("BaseState") == Some(b"OFF") {
            for r in refs(props.get("OCGs")) {
                off.insert(r);
            }
            for r in refs(d.get("ON")) {
                off.remove(&r);
            }
        }
        for r in refs(d.get("OFF")) {
            off.insert(r);
        }
        off
    }

    pub(crate) fn render_cache(&self) -> &Cache {
        &self.render
    }
}

/// default user space -> device pixels: the crop box's corner at the origin, y down, /Rotate,
/// UserUnit and dpi, the region's corner at the origin
fn device_matrix(p: &Page, dpi: f64, rx: f64, ry: f64) -> M {
    let s = dpi / 72.0 * p.user_unit;
    let [x0, y0, x1, y1] = p.crop_box;
    let m = match p.rotate {
        90 => [0.0, s, s, 0.0, -y0 * s, -x0 * s],
        180 => [-s, 0.0, 0.0, s, x1 * s, -y0 * s],
        270 => [0.0, -s, -s, 0.0, y1 * s, x1 * s],
        _ => [s, 0.0, 0.0, -s, -x0 * s, y1 * s],
    };
    mul(&m, &[1.0, 0.0, 0.0, 1.0, -rx, -ry])
}

impl<'d> Run<'d> {
    fn new(doc: &'d Document, opt: &'d RenderOptions, layer: Layer, dev: M, pixels: u64) -> Run<'d> {
        let rect = layer.rect;
        let gray = Rc::new(Cs::Gray);
        let gs = Gs {
            ctm: dev,
            clip: Clip { rect, mask: None },
            fill: Paint::new(gray.clone()),
            stroke: Paint::new(gray),
            lw: 1.0,
            cap: 0,
            join: 0,
            miter: 10.0,
            dash: Rc::new(Vec::new()),
            phase: 0.0,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            blend: Blend::Normal,
            smask: None,
            font: None,
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            rise: 0.0,
            mode: 0,
        };
        Run {
            doc,
            opt,
            layers: vec![layer],
            ras: Raster::default(),
            gs,
            path: Vec::new(),
            cur: (0.0, 0.0),
            start: (0.0, 0.0),
            base: dev,
            depth: 0,
            forms: Vec::new(),
            form_cache: BTreeMap::new(),
            images: BTreeMap::new(),
            cs_cache: BTreeMap::new(),
            shadings: BTreeMap::new(),
            ops: 0,
            work: 0,
            budget: pixels.saturating_mul(WORK_PER_PX).max(MIN_WORK),
            oc_off: BTreeSet::new(),
            mc: Vec::new(),
            hidden: 0,
            uncolored: false,
            text_clip: None,
            t3_clip: false,
            codes: Vec::new(),
            error: None,
        }
    }

    /// count painted pixels; false when the page has used up its work
    fn spend(&mut self, n: u64) -> bool {
        self.work = self.work.saturating_add(n);
        if self.work > self.budget {
            self.error.get_or_insert(Error::Limit("the work of a page"));
            return false;
        }
        true
    }

    fn stopped(&self) -> bool {
        self.error.is_some()
    }

    fn layer_rect(&self) -> Rect {
        self.layers.last().map_or([0, 0, 0, 0], |l| l.rect)
    }

    // ------------------------------------------------------------ paths

    /// the current path into the rasterizer's lines in device space (through m)
    fn lines(&mut self, segs: &[Seg], m: &M) {
        let mut p = Path::default();
        for s in segs {
            match *s {
                Seg::M(x, y) => {
                    let (a, b) = apply(m, x, y);
                    p.move_f(a as f32, b as f32);
                }
                Seg::L(x, y) => {
                    let (a, b) = apply(m, x, y);
                    p.line_f(a as f32, b as f32);
                }
                Seg::C(c) => {
                    let (a, b) = apply(m, c[0], c[1]);
                    let (e, f) = apply(m, c[2], c[3]);
                    let (g, h) = apply(m, c[4], c[5]);
                    p.cubic_f((a as f32, b as f32), (e as f32, f as f32), (g as f32, h as f32));
                }
                Seg::Z => p.close(),
            }
        }
        p.fill_lines(&mut self.ras);
    }

    /// the outline of the stroke of segs (user space, line style of gs) into the rasterizer
    fn stroke_lines(&mut self, segs: &[Seg], ctm: &M) -> bool {
        if self.adjusted_stroke(segs, ctm) {
            return true;
        }
        let gs = &self.gs;
        let det = (ctm[0] * ctm[3] - ctm[1] * ctm[2]).abs();
        let mut k = color::sqrt(det);
        if !(k > 1e-9) {
            k = ctm[0].abs().max(ctm[1].abs()).max(ctm[2].abs()).max(ctm[3].abs());
            if !(k > 1e-9) {
                return false;
            }
        }
        // the stroke in user space scaled by k (a unit there is about a pixel), then M / k
        let mut p = Path::default();
        for s in segs {
            match *s {
                Seg::M(x, y) => p.move_f((x * k) as f32, (y * k) as f32),
                Seg::L(x, y) => p.line_f((x * k) as f32, (y * k) as f32),
                Seg::C(c) => p.cubic_f(((c[0] * k) as f32, (c[1] * k) as f32), ((c[2] * k) as f32, (c[3] * k) as f32), ((c[4] * k) as f32, (c[5] * k) as f32)),
                Seg::Z => p.close(),
            }
        }
        let (width, unit) = if gs.lw * k < 1e-6 { (1.0, 1.0 / k) } else { (gs.lw * k, gs.lw) };
        let mut pat: Vec<f64> = gs.dash.iter().map(|d| d / unit).collect();
        if pat.iter().all(|&d| d <= 0.0) || pat.iter().any(|&d| d < 0.0 || !d.is_finite()) {
            pat.clear();
        }
        pat.truncate(gfx::dash::DASH_MAXPAT);
        let st = DStyle { width, pat: &pat, join: gs.join, cap0: gs.cap, cap1: gs.cap, dcap: gs.cap, offset: gs.phase / unit, miter: gs.miter.max(1.0) };
        if dash_stroke(&mut p, &st) != 0 {
            return false;
        }
        let m = [ctm[0] / k, ctm[1] / k, ctm[2] / k, ctm[3] / k, ctm[4], ctm[5]];
        for pt in p.pts_mut() {
            let (x, y) = apply(&m, pt.0 as f64, pt.1 as f64);
            *pt = (x as f32, y as f32);
        }
        p.fill_lines(&mut self.ras);
        true
    }

    /// stroke adjustment (as Poppler does it): a solid stroke of lines that are all horizontal or
    /// vertical on the device becomes rectangles whose edges are rounded to whole pixels, at
    /// least one pixel wide, so that thin rules and frames stay sharp and equally thick
    fn adjusted_stroke(&mut self, segs: &[Seg], ctm: &M) -> bool {
        let gs = &self.gs;
        let axis = ctm[1].abs() < 1e-9 && ctm[2].abs() < 1e-9;
        let swap = ctm[0].abs() < 1e-9 && ctm[3].abs() < 1e-9;
        if !gs.dash.is_empty() || !(axis || swap) || segs.iter().any(|s| matches!(s, Seg::C(_))) || segs.len() > 4096 {
            return false;
        }
        // half the thickness of horizontal and of vertical lines on the device
        let (sy, sx) = if axis { (ctm[3].abs(), ctm[0].abs()) } else { (ctm[1].abs(), ctm[2].abs()) };
        let hh = if gs.lw * sy < 1.0 { 0.5 } else { gs.lw * sy / 2.0 };
        let hv = if gs.lw * sx < 1.0 { 0.5 } else { gs.lw * sx / 2.0 };
        // round caps and joins other than miter only where they cannot be seen
        if (gs.cap == 1 || gs.join != 0) && (hh > 1.0 || hv > 1.0) {
            return false;
        }
        let cap = if gs.cap == 0 { 0.0 } else { 1.0 };
        // the subpaths as device points, closed or not
        let mut subs: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
        for s in segs {
            match *s {
                Seg::M(x, y) => subs.push((vec![apply(ctm, x, y)], false)),
                Seg::L(x, y) => match subs.last_mut() {
                    Some(sp) => sp.0.push(apply(ctm, x, y)),
                    None => return false,
                },
                Seg::Z => {
                    if let Some(sp) = subs.last_mut() {
                        sp.1 = true;
                    }
                }
                Seg::C(_) => return false,
            }
        }
        let mut rects: Vec<[f64; 4]> = Vec::new();
        for (pts, closed) in &subs {
            let mut p: Vec<(f64, f64)> = Vec::with_capacity(pts.len());
            for &q in pts {
                if p.last().is_none_or(|l: &(f64, f64)| (l.0 - q.0).abs() > 1e-6 || (l.1 - q.1).abs() > 1e-6) {
                    p.push(q);
                }
            }
            let closed = *closed || p.len() > 2 && (p[0].0 - p[p.len() - 1].0).abs() < 1e-6 && (p[0].1 - p[p.len() - 1].1).abs() < 1e-6;
            if closed && p.len() > 1 && (p[0].0 - p[p.len() - 1].0).abs() < 1e-6 && (p[0].1 - p[p.len() - 1].1).abs() < 1e-6 {
                p.pop();
            }
            let n = p.len();
            if n < 2 {
                continue;
            }
            let nseg = if closed { n } else { n - 1 };
            for i in 0..nseg {
                let (a, b) = (p[i], p[(i + 1) % n]);
                let horiz = (a.1 - b.1).abs() < 1e-6;
                let vert = (a.0 - b.0).abs() < 1e-6;
                if !horiz && !vert {
                    return false;
                }
                // a join or a cap at each end: miter and square reach half the width further
                let ext0 = if closed || i > 0 { 1.0 } else { cap };
                let ext1 = if closed || i + 1 < nseg { 1.0 } else { cap };
                if horiz {
                    let (x0, x1) = if a.0 <= b.0 { (a.0 - hv * ext0, b.0 + hv * ext1) } else { (b.0 - hv * ext1, a.0 + hv * ext0) };
                    rects.push([x0, a.1 - hh, x1, a.1 + hh]);
                } else {
                    let (y0, y1) = if a.1 <= b.1 { (a.1 - hh * ext0, b.1 + hh * ext1) } else { (b.1 - hh * ext1, a.1 + hh * ext0) };
                    rects.push([a.0 - hv, y0, a.0 + hv, y1]);
                }
            }
        }
        let round = |v: f64| color::floor(v + 0.5);
        self.ras.lines_begin();
        for r in rects {
            let (x0, mut x1) = (round(r[0]), round(r[2]));
            let (y0, mut y1) = (round(r[1]), round(r[3]));
            if x1 <= x0 {
                x1 = x0 + 1.0;
            }
            if y1 <= y0 {
                y1 = y0 + 1.0;
            }
            let (x0, x1, y0, y1) = (x0 as f32, x1 as f32, y0 as f32, y1 as f32);
            self.ras.emit_line(x0, y0, x1, y0);
            self.ras.emit_line(x1, y0, x1, y1);
            self.ras.emit_line(x1, y1, x0, y1);
            self.ras.emit_line(x0, y1, x0, y0);
        }
        true
    }

    /// a clipping path that is one rectangle along the device's axes: its edges rounded to whole
    /// pixels (as Poppler clips to rectangles), no mask
    fn clip_rect_fast(&mut self, segs: &[Seg], ctm: &M) -> bool {
        let mut pts: Vec<(f64, f64)> = Vec::with_capacity(5);
        for (i, s) in segs.iter().enumerate() {
            match *s {
                Seg::M(x, y) if i == 0 => pts.push(apply(ctm, x, y)),
                Seg::L(x, y) => pts.push(apply(ctm, x, y)),
                Seg::Z if i + 1 == segs.len() => {}
                _ => return false,
            }
        }
        if pts.len() == 5 && (pts[0].0 - pts[4].0).abs() < 1e-6 && (pts[0].1 - pts[4].1).abs() < 1e-6 {
            pts.pop();
        }
        if pts.len() != 4 {
            return false;
        }
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
        let along = (0..4).all(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            near(a.0, b.0) != near(a.1, b.1)
        });
        if !along {
            return false;
        }
        let xs = pts.iter().map(|p| p.0);
        let ys = pts.iter().map(|p| p.1);
        let (x0, x1) = (xs.clone().fold(f64::MAX, f64::min), xs.fold(f64::MIN, f64::max));
        let (y0, y1) = (ys.clone().fold(f64::MAX, f64::min), ys.fold(f64::MIN, f64::max));
        let r = |v: f64| color::floor(v + 0.5).clamp(-1.0e8, 1.0e8) as i32;
        let rect = canvas::intersect(self.gs.clip.rect, [r(x0), r(y0), r(x1), r(y1)]);
        self.gs.clip = Clip { rect, mask: if canvas::is_empty(rect) { None } else { self.gs.clip.mask.clone() } };
        true
    }

    fn clip_rect_user(&mut self, b: &[f64; 4]) {
        let segs = [Seg::M(b[0], b[1]), Seg::L(b[2], b[1]), Seg::L(b[2], b[3]), Seg::L(b[0], b[3]), Seg::Z];
        let ctm = self.gs.ctm;
        if self.clip_rect_fast(&segs, &ctm) {
            return;
        }
        self.lines(&segs, &ctm);
        if let Some(c) = self.gs.clip.and_path(&mut self.ras, RULE_NONZERO) {
            self.gs.clip = c;
        }
    }

    /// the source of a paint for the pixels of a fill
    fn paint(&mut self, stroke: bool, f: &mut dyn FnMut(&mut Run, &Src, u8)) {
        let p = if stroke { self.gs.stroke.clone() } else { self.gs.fill.clone() };
        let alpha = to8(if stroke { self.gs.stroke_alpha } else { self.gs.fill_alpha });
        if p.cs.paints_nothing() {
            return;
        }
        match &p.pat {
            None => f(self, &Src::Solid(p.rgb), alpha),
            Some(pat) => {
                let area = canvas::intersect(self.gs.clip.rect, self.layer_rect());
                let lb = self.ras.lbox();
                let area = canvas::intersect(area, [canvas::floor(lb[0]), canvas::floor(lb[1]), canvas::ceil(lb[2]) + 1, canvas::ceil(lb[3]) + 1]);
                if canvas::is_empty(area) {
                    return;
                }
                // the rasterizer's lines are kept while the pattern is prepared (it draws too)
                let saved = core::mem::take(&mut self.ras);
                let shader = shade::shader(self, pat, &p, area);
                self.ras = saved;
                if let Some(s) = shader {
                    f(self, &Src::Shade(&*s), alpha);
                }
            }
        }
    }

    /// fill the rasterizer's lines with the fill or stroke paint
    fn fill_ras(&mut self, rule: u32, stroke: bool) {
        if self.hidden > 0 || self.stopped() {
            return;
        }
        self.paint(stroke, &mut |run, src, alpha| {
            let Run { layers, ras, gs, .. } = run;
            let Some(layer) = layers.last_mut() else { return };
            let put = Put { clip: &gs.clip, alpha, blend: gs.blend, smask: gs.smask.as_deref() };
            let n = canvas::fill(layer, ras, rule, src, &put);
            run.spend(n);
        });
    }

    fn paint_path(&mut self, fill: Option<u32>, stroke: bool, close: bool) {
        if close {
            self.path.push(Seg::Z);
        }
        let segs = core::mem::take(&mut self.path);
        let ctm = self.gs.ctm;
        if let Some(rule) = fill {
            self.lines(&segs, &ctm);
            self.fill_ras(rule, false);
        }
        if stroke && self.stroke_lines(&segs, &ctm) {
            self.fill_ras(RULE_NONZERO, true);
        }
        self.path = segs;
    }

    fn end_path(&mut self, clip: Option<u32>) {
        if let Some(rule) = clip {
            let segs = core::mem::take(&mut self.path);
            let ctm = self.gs.ctm;
            if self.clip_rect_fast(&segs, &ctm) {
                self.path.clear();
                return;
            }
            self.lines(&segs, &ctm);
            match self.gs.clip.and_path(&mut self.ras, rule) {
                Some(c) => self.gs.clip = c,
                None => {
                    self.error.get_or_insert(Error::Limit("memory"));
                }
            }
        }
        self.path.clear();
    }

    // ------------------------------------------------------------ colours

    fn colorspace(&mut self, o: &Object, res: &Res) -> Option<Rc<Cs>> {
        // a named space of the resources by its reference, parsed once
        if let Object::Name(n) = o {
            if let Some(r) = res.all.get("ColorSpace").map(|c| self.doc.dict(c)).and_then(|d| d.get_bytes(&n.0).and_then(Object::as_ref)) {
                if let Some(c) = self.cs_cache.get(&r) {
                    return c.clone();
                }
                let c = Cs::parse(self.doc, &Object::Ref(r), &res.all);
                self.cs_cache.insert(r, c.clone());
                return c;
            }
        }
        Cs::parse(self.doc, o, &res.all)
    }

    fn set_color(&mut self, stroke: bool, a: &[Object], res: &Res) {
        if self.uncolored {
            return;
        }
        let p = if stroke { &mut self.gs.stroke } else { &mut self.gs.fill };
        // a pattern: its name last, the underlying space's components before it
        if let Some(Object::Name(n)) = a.last() {
            if matches!(&*p.cs, Cs::Pattern { .. }) {
                let comps = nums(&a[..a.len() - 1]);
                let under = match &*p.cs {
                    Cs::Pattern { under: Some(u) } => Some(u.clone()),
                    _ => None,
                };
                let pat = res.patterns.get_bytes(&n.0).cloned();
                let base = self.base;
                let doc = self.doc;
                let pat = pat.and_then(|o| shade::Pattern::parse(doc, &o, &base, self));
                let p = if stroke { &mut self.gs.stroke } else { &mut self.gs.fill };
                p.pat = pat;
                if let Some(u) = under {
                    let c = u.rgb(&comps);
                    p.rgb = [to8(c[0]), to8(c[1]), to8(c[2])];
                    p.comps = comps;
                }
                return;
            }
        }
        p.pat = None;
        p.set(&nums(a));
    }

    fn set_space(&mut self, stroke: bool, cs: Rc<Cs>) {
        if self.uncolored {
            return;
        }
        let p = Paint::new(cs);
        if stroke {
            self.gs.stroke = p;
        } else {
            self.gs.fill = p;
        }
    }

    // ------------------------------------------------------------ graphics state

    fn ext_gstate(&mut self, d: &Dict, res: &Res) {
        let doc = self.doc;
        if let Some(v) = doc.key(d, "LW").as_f64() {
            self.gs.lw = v.abs();
        }
        if let Some(v) = doc.key(d, "LC").as_int() {
            self.gs.cap = v.clamp(0, 2) as u32;
        }
        if let Some(v) = doc.key(d, "LJ").as_int() {
            self.gs.join = v.clamp(0, 2) as u32;
        }
        if let Some(v) = doc.key(d, "ML").as_f64() {
            self.gs.miter = v;
        }
        if let Object::Array(a) = doc.key(d, "D") {
            if let Some(Object::Array(p)) = a.first().map(|o| doc.resolve(o)) {
                self.gs.dash = Rc::new(p.iter().filter_map(|v| doc.resolve(v).as_f64()).take(64).collect());
                self.gs.phase = a.get(1).and_then(|v| doc.resolve(v).as_f64()).unwrap_or(0.0);
            }
        }
        if let Some(v) = doc.key(d, "CA").as_f64() {
            self.gs.stroke_alpha = v.clamp(0.0, 1.0);
        }
        if let Some(v) = doc.key(d, "ca").as_f64() {
            self.gs.fill_alpha = v.clamp(0.0, 1.0);
        }
        match doc.key(d, "BM") {
            Object::Name(n) => self.gs.blend = Blend::from_name(&n.0).unwrap_or(Blend::Normal),
            Object::Array(a) => {
                self.gs.blend = a.iter().find_map(|v| doc.resolve(v).as_name().and_then(Blend::from_name)).unwrap_or(Blend::Normal);
            }
            _ => {}
        }
        if let Object::Array(a) = doc.key(d, "Font") {
            if let (Some(f), Some(sz)) = (a.first(), a.get(1).and_then(|v| doc.resolve(v).as_f64())) {
                self.gs.font = self.font_of(f);
                self.gs.size = sz;
            }
        }
        match doc.key(d, "SMask") {
            Object::Name(n) if n.0 == b"None" => self.gs.smask = None,
            Object::Dict(sm) => self.gs.smask = self.soft_mask(&sm, res),
            _ => {}
        }
    }

    /// a soft mask: its group drawn now, with the CTM of now (11.6.5.2)
    fn soft_mask(&mut self, sm: &Dict, _res: &Res) -> Option<Rc<Mask>> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let doc = self.doc;
        let luminosity = sm.name("S") != Some(b"Alpha");
        let g = sm.get("G")?;
        let r = g.as_ref();
        let form = self.form(g)?;
        let gd = match doc.resolve(g) {
            Object::Stream(s) => s.dict.clone(),
            _ => return None,
        };
        let group_cs = doc.dict(&gd.get("Group").cloned().unwrap_or_default()).get("CS").cloned().and_then(|c| Cs::parse(doc, &c, &Dict::new()));
        let ctm = finite(mul(&form.matrix, &self.gs.ctm));
        // where the group can paint: its bbox on the device, within the page
        let page = self.layers[0].rect;
        let rect = match form.bbox {
            Some(b) => canvas::intersect(page, bbox_rect(&b, &ctm)),
            None => page,
        };
        let bc: Vec<f64> = match doc.key(sm, "BC") {
            Object::Array(a) => a.iter().filter_map(|v| doc.resolve(v).as_f64()).collect(),
            _ => Vec::new(),
        };
        let back = if luminosity {
            let cs = group_cs.clone().unwrap_or_else(|| Rc::new(Cs::Rgb));
            let c = if bc.is_empty() { [0.0; 3] } else { cs.rgb(&bc) };
            [to8(c[0]), to8(c[1]), to8(c[2]), 255]
        } else {
            [0, 0, 0, 0]
        };
        let tr = match doc.key(sm, "TR") {
            Object::Name(_) | Object::Null => None,
            o => color::Func::parse(doc, &o),
        };
        let lum = |c: [u8; 4]| -> u8 { ((c[0] as u32 * 77 + c[1] as u32 * 151 + c[2] as u32 * 28 + 128) >> 8) as u8 };
        let lut: Vec<u8> = (0..256u32)
            .map(|v| match &tr {
                Some(f) => {
                    let mut out = Vec::new();
                    f.eval(&[v as f64 / 255.0], &mut out);
                    to8(out.first().copied().unwrap_or(v as f64 / 255.0))
                }
                None => v as u8,
            })
            .collect();
        let outside = lut[if luminosity { lum(back) } else { 0 } as usize];
        if canvas::is_empty(rect) {
            return Some(Rc::new(Mask { rect, a: Vec::new(), outside }));
        }
        let mut layer = Layer::new(rect)?;
        layer.fill(back);
        let saved_gs = self.gs.clone();
        self.layers.push(layer);
        self.gs.ctm = ctm;
        self.gs.clip = Clip { rect, mask: None };
        self.gs.fill_alpha = 1.0;
        self.gs.stroke_alpha = 1.0;
        self.gs.blend = Blend::Normal;
        self.gs.smask = None;
        let res = form.res.clone().unwrap_or_else(|| Rc::new(Res::of(doc, &Dict::new())));
        self.depth += 1;
        self.forms.push(r.unwrap_or(Ref { num: u32::MAX, generation: 0 }));
        let saved_base = self.base;
        self.base = ctm;
        if let Some(b) = form.bbox {
            self.clip_rect_user(&b);
        }
        self.run(&form.data, &res);
        self.base = saved_base;
        self.forms.pop();
        self.depth -= 1;
        let layer = self.layers.pop()?;
        self.gs = saved_gs;
        let a: Vec<u8> = layer
            .px
            .chunks_exact(4)
            .map(|p| {
                let v = if luminosity { lum([p[0], p[1], p[2], p[3]]) } else { p[3] };
                lut[v as usize]
            })
            .collect();
        Some(Rc::new(Mask { rect, a, outside }))
    }

    // ------------------------------------------------------------ fonts and text

    fn font_of(&mut self, o: &Object) -> Option<Rc<fonts::GFont>> {
        let doc = self.doc;
        let cache = doc.render_cache();
        let r = o.as_ref();
        if let Some(f) = r.and_then(|r| cache.fonts.borrow().get(&r).cloned()) {
            return Some(f);
        }
        let d = doc.dict(o);
        if d.is_empty() {
            return None;
        }
        let source = self.opt.fonts.clone().unwrap_or_else(|| cache.system.get_or_init(|| Rc::new(SystemFonts::new())).clone());
        let mut subst = |q: &FontQuery| -> Option<Rc<fonts::Prog>> {
            let key = (String::from(q.name), q.bold, q.italic, q.fixed);
            if let Some(p) = cache.subst.borrow().get(&key) {
                return p.clone();
            }
            let p = source.find(q).and_then(|(data, face)| fonts::Prog::open(data, face, 2)).map(Rc::new);
            cache.subst.borrow_mut().insert(key, p.clone());
            p
        };
        let f = Rc::new(fonts::GFont::load(doc, &d, &mut subst));
        if let Some(r) = r {
            let mut c = cache.fonts.borrow_mut();
            if c.len() < 4096 {
                c.insert(r, f.clone());
            }
        }
        Some(f)
    }

    fn show(&mut self, tm: &mut M, s: &[u8], res: &Rc<Res>) {
        let Some(font) = self.gs.font.clone() else { return };
        let mut codes = core::mem::take(&mut self.codes);
        font.base.codes(s, &mut codes);
        for &(code, len) in &codes {
            if self.stopped() {
                break;
            }
            let gs = &self.gs;
            let w0 = font.base.width(code);
            let mut trm = mul(&[gs.size * gs.th, 0.0, 0.0, gs.size, 0.0, gs.rise], tm);
            if font.base.vertical {
                // the origin of a vertical glyph: half its width left, 0.88 up
                let hw = font.base.h_width(code);
                trm = mul(&[1.0, 0.0, 0.0, 1.0, -hw / 2.0 * gs.size * gs.th, -0.88 * gs.size], &trm);
            }
            self.glyph(&font, code, &trm, res);
            let w0 = font.width(code).unwrap_or(w0);
            let gs = &self.gs;
            let space = if len == 1 && code == 32 { gs.tw } else { 0.0 };
            let (tx, ty) = if font.base.vertical { (0.0, w0 * gs.size + gs.tc + space) } else { ((w0 * gs.size + gs.tc + space) * gs.th, 0.0) };
            *tm = finite(mul(&[1.0, 0.0, 0.0, 1.0, tx, ty], tm));
        }
        self.codes = codes;
    }

    /// one glyph: trm is glyph-independent text space -> user space (before the CTM)
    fn glyph(&mut self, font: &Rc<fonts::GFont>, code: u32, trm: &M, res: &Rc<Res>) {
        let mode = self.gs.mode;
        if let Some(t3) = &font.type3 {
            if mode >= 4 {
                self.t3_clip = true;
            }
            if mode != 3 && mode != 7 {
                self.type3(t3, code, &mul(trm, &self.gs.ctm), res);
            }
            return;
        }
        let Some(g) = font.glyph(code) else { return };
        let to_user = mul(&font.matrix, trm);
        let segs = outline_segs(&g);
        let ctm = self.gs.ctm;
        let mut dev = mul(&to_user, &ctm);
        // the origin as Poppler places glyphs: down to a whole pixel, across to a quarter (a
        // baseline stays on one row of pixels, a glyph looks the same wherever it is)
        let (ox, oy) = apply(&mul(trm, &ctm), 0.0, 0.0);
        let placed = ox.abs() < 1e7 && oy.abs() < 1e7;
        // the origin in quarter pixels across and whole pixels down
        let (qx, py) = if placed { (color::floor(ox * 4.0) as i64, color::floor(oy) as i64) } else { (0, 0) };
        if placed {
            dev[4] += qx as f64 / 4.0 - ox;
            dev[5] += py as f64 - oy;
        }
        let fill = matches!(mode, 0 | 2 | 4 | 6);
        let stroke = matches!(mode, 1 | 2 | 5 | 6);
        let clip = mode >= 4;
        if fill && placed && self.hidden == 0 && !self.stopped() {
            // a glyph drawn once per font, code, size and quarter pixel, then placed
            let (gx, gy) = (qx.div_euclid(4), py);
            let phase = qx.rem_euclid(4) as u8;
            let key = (Rc::as_ptr(font) as usize, code, [dev[0].to_bits(), dev[1].to_bits(), dev[2].to_bits(), dev[3].to_bits()], phase);
            let cache = self.doc.render_cache();
            let hit = cache.glyphs.borrow().get(&key).map(|e| e.1.clone());
            let cover = match hit {
                Some(c) => c,
                None => {
                    // drawn where every coordinate is positive (the rasterizer's rows start there)
                    let (mut mx, mut my) = (0f64, 0f64);
                    for &(x, y) in &g.pts {
                        let (a, b) = apply(&[dev[0], dev[1], dev[2], dev[3], 0.0, 0.0], x as f64, y as f64);
                        mx = mx.min(a);
                        my = my.min(b);
                    }
                    let (sx, sy) = (color::ceil(-mx).clamp(0.0, 1e6) + 2.0, color::ceil(-my).clamp(0.0, 1e6) + 2.0);
                    let local = [dev[0], dev[1], dev[2], dev[3], phase as f64 / 4.0 + sx, sy];
                    self.lines(&segs, &local);
                    let c = canvas::Cover::of(&mut self.ras, sx as i32, sy as i32).map(Rc::new);
                    let bytes = c.as_ref().map_or(0, |c| c.a.len() + 64);
                    if cache.glyph_bytes.get() + bytes > GLYPH_CACHE {
                        cache.glyphs.borrow_mut().clear();
                        cache.glyph_bytes.set(0);
                    }
                    cache.glyph_bytes.set(cache.glyph_bytes.get() + bytes);
                    // the font is kept with its glyphs: its address stays its own
                    cache.glyphs.borrow_mut().insert(key, (font.clone(), c.clone()));
                    c
                }
            };
            if let Some(c) = cover {
                {
                    let (ox, oy) = (gx as i32, gy as i32);
                    let saved = core::mem::take(&mut self.ras);
                    // a pattern paint needs the area: the glyph's box
                    self.ras.lines_begin();
                    let (x0, y0) = ((ox + c.x) as f32, (oy + c.y) as f32);
                    self.ras.emit_line(x0, y0, x0 + c.w as f32, y0 + c.h as f32);
                    self.paint(false, &mut |run, src, alpha| {
                        let Run { layers, gs, .. } = run;
                        let Some(layer) = layers.last_mut() else { return };
                        let put = Put { clip: &gs.clip, alpha, blend: gs.blend, smask: gs.smask.as_deref() };
                        let n = canvas::fill_cover(layer, &c, ox, oy, src, &put);
                        run.spend(n);
                    });
                    self.ras = saved;
                }
            }
        } else if fill {
            self.lines(&segs, &dev);
            self.fill_ras(RULE_NONZERO, false);
        }
        if stroke {
            let user: Vec<Seg> = segs
                .iter()
                .map(|s| match *s {
                    Seg::M(x, y) => {
                        let (a, b) = apply(&to_user, x, y);
                        Seg::M(a, b)
                    }
                    Seg::L(x, y) => {
                        let (a, b) = apply(&to_user, x, y);
                        Seg::L(a, b)
                    }
                    Seg::C(c) => {
                        let (a, b) = apply(&to_user, c[0], c[1]);
                        let (e, f) = apply(&to_user, c[2], c[3]);
                        let (g, h) = apply(&to_user, c[4], c[5]);
                        Seg::C([a, b, e, f, g, h])
                    }
                    Seg::Z => Seg::Z,
                })
                .collect();
            if self.stroke_lines(&user, &ctm) {
                self.fill_ras(RULE_NONZERO, true);
            }
        }
        if clip {
            let tc = self.text_clip.get_or_insert_with(Path::default);
            add_segs(tc, &segs, &dev);
        }
    }

    fn type3(&mut self, t3: &fonts::Type3, code: u32, trm_dev: &M, res: &Rc<Res>) {
        let Some(Some(proc_)) = t3.procs.get(code as usize) else { return };
        if self.depth >= MAX_DEPTH {
            return;
        }
        let Ok(data) = self.doc.decode(proc_) else { return };
        let fres = match &t3.res {
            Some(r) => Rc::new(Res::of(self.doc, r)),
            None => res.clone(),
        };
        let saved = self.gs.clone();
        let saved_path = core::mem::take(&mut self.path);
        let saved_unc = self.uncolored;
        let saved_base = self.base;
        let saved_tc = self.text_clip.take();
        self.gs.ctm = finite(mul(&t3.matrix, trm_dev));
        self.base = self.gs.ctm;
        self.depth += 1;
        self.run(&data, &fres);
        self.depth -= 1;
        self.gs = saved;
        self.path = saved_path;
        self.uncolored = saved_unc;
        self.base = saved_base;
        self.text_clip = saved_tc;
    }

    // ------------------------------------------------------------ XObjects

    fn form(&mut self, o: &Object) -> Option<Rc<Form>> {
        let r = o.as_ref();
        if let Some(f) = r.and_then(|r| self.form_cache.get(&r)) {
            return f.clone();
        }
        let doc = self.doc;
        let built = match doc.resolve(o) {
            Object::Stream(s) => doc.decode(&s).ok().map(|data| {
                let res = s.dict.get("Resources").map(|x| Rc::new(Res::of(doc, &doc.dict(x))));
                let matrix = m6(&color::nums(doc, &doc.key(&s.dict, "Matrix"))).map(finite).unwrap_or(ID);
                let bbox = doc.key(&s.dict, "BBox").as_rect();
                let group = doc.dict(&s.dict.get("Group").cloned().unwrap_or_default()).is("S", "Transparency");
                Rc::new(Form { data, res, matrix, bbox, group, oc: s.dict.get("OC").cloned() })
            }),
            _ => None,
        };
        if let Some(r) = r {
            self.form_cache.insert(r, built.clone());
        }
        built
    }

    fn draw_form(&mut self, o: &Object, res: &Rc<Res>, extra: Option<M>) {
        let r = o.as_ref();
        if self.depth >= MAX_DEPTH || r.is_some_and(|r| self.forms.contains(&r)) {
            return;
        }
        let Some(form) = self.form(o) else { return };
        if let Some(oc) = &form.oc {
            if !self.oc_visible(oc) {
                return;
            }
        }
        let saved = self.gs.clone();
        let saved_path = core::mem::take(&mut self.path);
        let saved_base = self.base;
        let ctm0 = match extra {
            Some(m) => mul(&m, &self.gs.ctm),
            None => self.gs.ctm,
        };
        self.gs.ctm = finite(mul(&form.matrix, &ctm0));
        self.base = self.gs.ctm;
        if let Some(b) = form.bbox {
            self.clip_rect_user(&b);
        }
        let fres = form.res.clone().unwrap_or_else(|| res.clone());
        self.forms.push(r.unwrap_or(Ref { num: u32::MAX, generation: 0 }));
        self.depth += 1;
        let group = form.group && (saved.fill_alpha < 1.0 || saved.blend != Blend::Normal || saved.smask.is_some());
        if group {
            // a transparency group: drawn into a layer of its own, then composited as one
            let rect = canvas::intersect(self.gs.clip.rect, self.layer_rect());
            match Layer::new(rect) {
                Some(l) if !canvas::is_empty(rect) => {
                    self.layers.push(l);
                    self.gs.fill_alpha = 1.0;
                    self.gs.stroke_alpha = 1.0;
                    self.gs.blend = Blend::Normal;
                    self.gs.smask = None;
                    self.run(&form.data, &fres);
                    let l = self.layers.pop();
                    if let (Some(l), Some(dst)) = (l, self.layers.last_mut()) {
                        let put = Put { clip: &saved.clip, alpha: to8(saved.fill_alpha), blend: saved.blend, smask: saved.smask.as_deref() };
                        let n = canvas::composite(dst, &l, &put);
                        self.spend(n);
                    }
                }
                _ => {}
            }
        } else {
            self.run(&form.data, &fres);
        }
        self.depth -= 1;
        self.forms.pop();
        self.gs = saved;
        self.path = saved_path;
        self.base = saved_base;
    }

    fn xobject(&mut self, name: &[u8], res: &Rc<Res>) {
        let Some(o) = res.xobjects.get_bytes(name).cloned() else { return };
        let sub = match self.doc.resolve(&o) {
            Object::Stream(s) => s.dict.name("Subtype").map(<[u8]>::to_vec),
            _ => None,
        };
        match sub.as_deref() {
            Some(b"Form") => self.draw_form(&o, res, None),
            Some(b"Image") => {
                if self.hidden > 0 {
                    return;
                }
                let img = match o.as_ref() {
                    Some(r) => match self.images.get(&r) {
                        Some(i) => i.clone(),
                        None => {
                            let i = image::Img::load_xobject(self, &o, res);
                            if self.images.len() < 256 {
                                self.images.insert(r, i.clone());
                            }
                            i
                        }
                    },
                    None => image::Img::load_xobject(self, &o, res),
                };
                if let Some(img) = img {
                    if img.oc.as_ref().is_none_or(|oc| self.oc_visible(oc)) {
                        image::draw(self, &img);
                    }
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ optional content

    fn oc_visible(&self, o: &Object) -> bool {
        let doc = self.doc;
        let r = o.as_ref();
        let d = doc.dict(o);
        if d.is("Type", "OCMD") {
            let refs: Vec<Ref> = match doc.key(&d, "OCGs") {
                Object::Array(a) => a.iter().filter_map(Object::as_ref).collect(),
                _ => d.get("OCGs").and_then(Object::as_ref).into_iter().collect(),
            };
            if refs.is_empty() {
                return true;
            }
            let on = |r: &Ref| !self.oc_off.contains(r);
            return match d.name("P") {
                Some(b"AllOn") => refs.iter().all(on),
                Some(b"AnyOff") => refs.iter().any(|r| !on(r)),
                Some(b"AllOff") => refs.iter().all(|r| !on(r)),
                _ => refs.iter().any(on),
            };
        }
        r.is_none_or(|r| !self.oc_off.contains(&r))
    }

    // ------------------------------------------------------------ the content stream

    fn run(&mut self, content: &[u8], res: &Rc<Res>) {
        let mut stack: Vec<Gs> = Vec::new();
        let mut extra_q = 0usize;
        let (mut tm, mut tlm) = (ID, ID);
        let mc_base = self.mc.len();
        let mut clip: Option<u32> = None;
        for op in Ops::new(content) {
            self.ops += 1;
            if self.ops > MAX_OPS {
                self.error.get_or_insert(Error::Limit("the operators of a page"));
            }
            if self.stopped() {
                break;
            }
            let a = &op.operands;
            let n = |i: usize| a.get(i).and_then(Object::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
            match op.operator {
                b"q" => {
                    if stack.len() < MAX_STACK {
                        stack.push(self.gs.clone());
                    } else {
                        extra_q += 1;
                    }
                }
                b"Q" => {
                    if extra_q > 0 {
                        extra_q -= 1;
                    } else if let Some(g) = stack.pop() {
                        self.gs = g;
                    }
                }
                b"cm" if a.len() >= 6 => {
                    let v = nums(&a[a.len() - 6..]);
                    self.gs.ctm = finite(mul(&[v[0], v[1], v[2], v[3], v[4], v[5]], &self.gs.ctm));
                }
                b"w" => self.gs.lw = n(0).abs(),
                b"J" => self.gs.cap = n(0).clamp(0.0, 2.0) as u32,
                b"j" => self.gs.join = n(0).clamp(0.0, 2.0) as u32,
                b"M" => self.gs.miter = n(0),
                b"d" if a.len() >= 2 => {
                    if let Some(arr) = a[0].as_array() {
                        self.gs.dash = Rc::new(nums(arr).into_iter().take(64).collect());
                        self.gs.phase = n(1);
                    }
                }
                b"gs" => {
                    if let Some(d) = a.first().and_then(Object::as_name).and_then(|nm| res.ext.get_bytes(nm)).map(|o| self.doc.dict(o)) {
                        self.ext_gstate(&d, res);
                    }
                }
                // paths
                b"m" => {
                    self.cur = (n(0), n(1));
                    self.start = self.cur;
                    self.path.push(Seg::M(n(0), n(1)));
                }
                b"l" => {
                    self.cur = (n(0), n(1));
                    self.path.push(Seg::L(n(0), n(1)));
                }
                b"c" => {
                    self.path.push(Seg::C([n(0), n(1), n(2), n(3), n(4), n(5)]));
                    self.cur = (n(4), n(5));
                }
                b"v" => {
                    self.path.push(Seg::C([self.cur.0, self.cur.1, n(0), n(1), n(2), n(3)]));
                    self.cur = (n(2), n(3));
                }
                b"y" => {
                    self.path.push(Seg::C([n(0), n(1), n(2), n(3), n(2), n(3)]));
                    self.cur = (n(2), n(3));
                }
                b"h" => {
                    self.path.push(Seg::Z);
                    self.cur = self.start;
                }
                b"re" => {
                    let (x, y, w, h) = (n(0), n(1), n(2), n(3));
                    self.path.extend([Seg::M(x, y), Seg::L(x + w, y), Seg::L(x + w, y + h), Seg::L(x, y + h), Seg::Z]);
                    self.cur = (x, y);
                    self.start = (x, y);
                }
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    if self.path.len() > 1 << 22 {
                        self.path.clear();
                    }
                    let o = op.operator;
                    let close = matches!(o, b"s" | b"b" | b"b*");
                    let fill = match o {
                        b"f" | b"F" | b"B" | b"b" => Some(RULE_NONZERO),
                        b"f*" | b"B*" | b"b*" => Some(RULE_EVENODD),
                        _ => None,
                    };
                    let stroke = matches!(o, b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*");
                    if o != b"n" {
                        self.paint_path(fill, stroke, close);
                    }
                    self.end_path(clip.take());
                }
                b"W" => clip = Some(RULE_NONZERO),
                b"W*" => clip = Some(RULE_EVENODD),
                // colours
                b"CS" | b"cs" => {
                    if let Some(o) = a.first() {
                        let o = o.clone();
                        if let Some(c) = self.colorspace(&o, res) {
                            self.set_space(op.operator == b"CS", c);
                        }
                    }
                }
                b"SC" | b"SCN" => self.set_color(true, a, res),
                b"sc" | b"scn" => self.set_color(false, a, res),
                b"G" | b"g" | b"RG" | b"rg" | b"K" | b"k" => {
                    let stroke = op.operator[0].is_ascii_uppercase();
                    let cs = Rc::new(match op.operator {
                        b"G" | b"g" => Cs::Gray,
                        b"RG" | b"rg" => Cs::Rgb,
                        _ => Cs::Cmyk,
                    });
                    if !self.uncolored {
                        self.set_space(stroke, cs);
                        let p = if stroke { &mut self.gs.stroke } else { &mut self.gs.fill };
                        p.set(&nums(a));
                    }
                }
                b"sh" => {
                    if let Some(o) = a.first().and_then(Object::as_name).and_then(|nm| res.shadings.get_bytes(nm)).cloned() {
                        if self.hidden == 0 {
                            shade::sh(self, &o, res);
                        }
                    }
                }
                // text
                b"BT" => {
                    tm = ID;
                    tlm = ID;
                    self.text_clip = None;
                    self.t3_clip = false;
                }
                b"ET" => {
                    if let Some(p) = self.text_clip.take() {
                        p.fill_lines(&mut self.ras);
                        if let Some(c) = self.gs.clip.and_path(&mut self.ras, RULE_NONZERO) {
                            self.gs.clip = c;
                        }
                    } else if self.gs.mode >= 4 && !self.t3_clip {
                        // clip text that drew nothing clips everything away
                        self.gs.clip = Clip { rect: [0, 0, 0, 0], mask: None };
                    }
                }
                b"Tf" if a.len() >= 2 => {
                    if let Some(nm) = a[a.len() - 2].as_name() {
                        let f = res.fonts.get_bytes(nm).cloned();
                        self.gs.font = f.and_then(|o| self.font_of(&o));
                    }
                    self.gs.size = n(a.len() - 1);
                }
                b"Tc" => self.gs.tc = n(0),
                b"Tw" => self.gs.tw = n(0),
                b"Tz" => self.gs.th = n(0) / 100.0,
                b"TL" => self.gs.tl = n(0),
                b"Ts" => self.gs.rise = n(0),
                b"Tr" => self.gs.mode = n(0).clamp(0.0, 7.0) as i64,
                b"Td" | b"TD" if a.len() >= 2 => {
                    if op.operator == b"TD" {
                        self.gs.tl = -n(1);
                    }
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, n(0), n(1)], &tlm));
                    tm = tlm;
                }
                b"Tm" if a.len() >= 6 => {
                    let v = nums(&a[a.len() - 6..]);
                    tlm = finite([v[0], v[1], v[2], v[3], v[4], v[5]]);
                    tm = tlm;
                }
                b"T*" => {
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -self.gs.tl], &tlm));
                    tm = tlm;
                }
                b"Tj" => {
                    if let Some(s) = a.last().and_then(Object::as_bytes) {
                        if self.hidden == 0 {
                            self.show(&mut tm, s, res);
                        }
                    }
                }
                b"'" | b"\"" => {
                    if op.operator == b"\"" && a.len() >= 3 {
                        self.gs.tw = n(0);
                        self.gs.tc = n(1);
                    }
                    tlm = finite(mul(&[1.0, 0.0, 0.0, 1.0, 0.0, -self.gs.tl], &tlm));
                    tm = tlm;
                    if let Some(s) = a.last().and_then(Object::as_bytes) {
                        if self.hidden == 0 {
                            self.show(&mut tm, s, res);
                        }
                    }
                }
                b"TJ" => {
                    let Some(arr) = a.last().and_then(Object::as_array) else { continue };
                    let vertical = self.gs.font.as_ref().is_some_and(|f| f.base.vertical);
                    for e in arr {
                        match e {
                            Object::String(s) => {
                                if self.hidden == 0 {
                                    self.show(&mut tm, s, res);
                                }
                            }
                            o => {
                                if let Some(v) = o.as_f64().filter(|v| v.is_finite()) {
                                    let adj = -v / 1000.0 * self.gs.size;
                                    let t = if vertical { [1.0, 0.0, 0.0, 1.0, 0.0, adj] } else { [1.0, 0.0, 0.0, 1.0, adj * self.gs.th, 0.0] };
                                    tm = finite(mul(&t, &tm));
                                }
                            }
                        }
                    }
                }
                b"d0" => {}
                b"d1" => self.uncolored = true,
                // XObjects and inline images
                b"Do" => {
                    if let Some(nm) = a.first().and_then(Object::as_name) {
                        let nm = nm.to_vec();
                        self.xobject(&nm, res);
                    }
                }
                b"BI" if a.len() >= 2 => {
                    if self.hidden == 0 {
                        if let (Object::Dict(d), Some(data)) = (&a[0], a[1].as_bytes()) {
                            if let Some(img) = image::Img::load_inline(self, d, data, res) {
                                image::draw(self, &img);
                            }
                        }
                    }
                }
                // marked content: optional content hides what it holds
                b"BDC" | b"BMC" => {
                    let mut hide = false;
                    if op.operator == b"BDC" && a.first().and_then(Object::as_name) == Some(b"OC") {
                        let p = match a.get(1) {
                            Some(Object::Name(nm)) => res.props.get_bytes(&nm.0).cloned(),
                            Some(o) => Some(o.clone()),
                            None => None,
                        };
                        if let Some(p) = p {
                            hide = !self.oc_visible(&p);
                        }
                    }
                    if self.mc.len() < 1 << 16 {
                        self.mc.push(hide);
                        if hide {
                            self.hidden += 1;
                        }
                    }
                }
                b"EMC" => {
                    if self.mc.len() > mc_base {
                        if self.mc.pop() == Some(true) {
                            self.hidden -= 1;
                        }
                    }
                }
                _ => {}
            }
        }
        // marked content left open by this stream ends with it
        while self.mc.len() > mc_base {
            if self.mc.pop() == Some(true) {
                self.hidden -= 1;
            }
        }
    }

    // ------------------------------------------------------------ annotations

    fn annotations(&mut self, page: &Page) {
        let doc = self.doc;
        let Object::Array(annots) = doc.key(&page.dict, "Annots") else { return };
        let dev = self.base;
        for a in annots.iter().take(1 << 14) {
            if self.stopped() {
                break;
            }
            let d = doc.dict(a);
            let flags = doc.key(&d, "F").as_int().unwrap_or(0);
            if flags & (2 | 32) != 0 || d.is("Subtype", "Popup") {
                continue;
            }
            if let Some(oc) = d.get("OC") {
                if !self.oc_visible(oc) {
                    continue;
                }
            }
            let Some(rect) = doc.key(&d, "Rect").as_rect() else { continue };
            let ap = doc.dict(&d.get("AP").cloned().unwrap_or_default());
            let Some(n) = ap.get("N").cloned() else { continue };
            let stream = match doc.resolve(&n) {
                Object::Stream(_) => n,
                Object::Dict(states) => {
                    let Some(st) = d.name("AS") else { continue };
                    match states.get_bytes(st) {
                        Some(o) => o.clone(),
                        None => continue,
                    }
                }
                _ => continue,
            };
            let Some(form) = self.form(&stream) else { continue };
            let Some(bbox) = form.bbox else { continue };
            // Algorithm 8.1: the form's bbox through its matrix, fitted onto Rect
            let r = [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])];
            let tb = transform_box(&bbox, &form.matrix);
            let (bw, bh) = (tb[2] - tb[0], tb[3] - tb[1]);
            let sx = if bw.abs() > 1e-9 { (r[2] - r[0]) / bw } else { 1.0 };
            let sy = if bh.abs() > 1e-9 { (r[3] - r[1]) / bh } else { 1.0 };
            let fit = [sx, 0.0, 0.0, sy, r[0] - tb[0] * sx, r[1] - tb[1] * sy];
            let res = Rc::new(Res::of(doc, &page.resources));
            let saved = self.gs.clone();
            self.gs.ctm = dev;
            self.gs.clip = Clip { rect: self.layers[0].rect, mask: None };
            self.draw_form(&stream, &res, Some(fit));
            self.gs = saved;
        }
    }
}

/// the corners of a box through m, as a box
fn transform_box(b: &[f64; 4], m: &M) -> [f64; 4] {
    let pts = [apply(m, b[0], b[1]), apply(m, b[2], b[1]), apply(m, b[2], b[3]), apply(m, b[0], b[3])];
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    [x0, y0, x1, y1]
}

/// a box in user space through the CTM as a device rectangle (pixels touched)
pub(super) fn bbox_rect(b: &[f64; 4], m: &M) -> Rect {
    let t = transform_box(b, m);
    let c = |v: f64| v.clamp(-1.0e8, 1.0e8);
    [canvas::floor(c(t[0]) as f32), canvas::floor(c(t[1]) as f32), canvas::ceil(c(t[2]) as f32) + 1, canvas::ceil(c(t[3]) as f32) + 1]
}

/// a glyph outline as path segments (TrueType's quadratic contours as cubics)
fn outline_segs(g: &fonts::Glyph) -> Vec<Seg> {
    use text::outline::{OLT_CUBIC, OLT_OFF};
    let mut out = Vec::new();
    let mut start = 0usize;
    for &e in &g.ends {
        let e = (e as usize).min(g.pts.len().saturating_sub(1));
        if e < start || start >= g.pts.len() {
            start = e + 1;
            continue;
        }
        let pts = &g.pts[start..=e];
        let tags = &g.tags[start..=e];
        let n = pts.len();
        let p = |i: usize| (pts[i % n].0 as f64, pts[i % n].1 as f64);
        let quad = tags.iter().any(|&t| t == OLT_OFF);
        if quad {
            // start at an on-curve point (or the middle of two off-curve ones)
            let first_on = tags.iter().position(|&t| t != OLT_OFF);
            let (s, origin) = match first_on {
                Some(i) => (i, p(i)),
                None => {
                    let (a, b) = (p(0), p(1));
                    (0, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0))
                }
            };
            out.push(Seg::M(origin.0, origin.1));
            let mut cur = origin;
            let mut ctrl: Option<(f64, f64)> = None;
            for k in 1..=n {
                let i = (s + k) % n;
                let q = p(i);
                let end = k == n;
                if tags[i] == OLT_OFF {
                    if let Some(c) = ctrl {
                        let mid = ((c.0 + q.0) / 2.0, (c.1 + q.1) / 2.0);
                        out.push(quad_seg(cur, c, mid));
                        cur = mid;
                    }
                    ctrl = Some(q);
                    if end {
                        out.push(quad_seg(cur, q, origin));
                    }
                } else {
                    let target = if end { origin } else { q };
                    match ctrl.take() {
                        Some(c) => out.push(quad_seg(cur, c, target)),
                        None => out.push(Seg::L(target.0, target.1)),
                    }
                    cur = target;
                }
            }
            out.push(Seg::Z);
        } else {
            out.push(Seg::M(p(0).0, p(0).1));
            let mut i = 1;
            while i < n {
                if tags[i] == OLT_CUBIC && i + 1 < n + 1 {
                    let c1 = p(i);
                    let c2 = p(i + 1);
                    let e2 = if i + 2 <= n { p((i + 2) % n) } else { p(0) };
                    out.push(Seg::C([c1.0, c1.1, c2.0, c2.1, e2.0, e2.1]));
                    i += 3;
                } else {
                    out.push(Seg::L(p(i).0, p(i).1));
                    i += 1;
                }
            }
            out.push(Seg::Z);
        }
        start = e + 1;
    }
    out
}

fn quad_seg(p0: (f64, f64), c: (f64, f64), p1: (f64, f64)) -> Seg {
    let c1 = (p0.0 + 2.0 / 3.0 * (c.0 - p0.0), p0.1 + 2.0 / 3.0 * (c.1 - p0.1));
    let c2 = (p1.0 + 2.0 / 3.0 * (c.0 - p1.0), p1.1 + 2.0 / 3.0 * (c.1 - p1.1));
    Seg::C([c1.0, c1.1, c2.0, c2.1, p1.0, p1.1])
}

fn add_segs(p: &mut Path, segs: &[Seg], m: &M) {
    for s in segs {
        match *s {
            Seg::M(x, y) => {
                let (a, b) = apply(m, x, y);
                p.move_f(a as f32, b as f32);
            }
            Seg::L(x, y) => {
                let (a, b) = apply(m, x, y);
                p.line_f(a as f32, b as f32);
            }
            Seg::C(c) => {
                let (a, b) = apply(m, c[0], c[1]);
                let (e, f) = apply(m, c[2], c[3]);
                let (g, h) = apply(m, c[4], c[5]);
                p.cubic_f((a as f32, b as f32), (e as f32, f as f32), (g as f32, h as f32));
            }
            Seg::Z => p.close(),
        }
    }
}

