// pdf -- pages as a vector PDF through lib/pdf (EX-01), and drawings placed on sheets of paper
//
//   write(doc, fonts, &PdfOptions) -> Result<bytes, pdf::Error>   the export: a PDF page per
//                                     page in the page's size (points), text as text in embedded
//                                     subsets, the shapes' hyperlinks (addresses and pages of the
//                                     export), a bookmark per page, the document information
//   Sheets::new(fonts)                drawings on sheets of any size (printing: paper, margins,
//                                     scale, tiles)
//     sheets.add(doc, drawing, (w, h), place, clip, links) -> pdf::PageId
//     sheets.pdf() -> &mut pdf::Document   outlines, information, language
//     sheets.finish() -> Result<bytes, pdf::Error>
//
// The display list (paper inches, y up) is drawn in the sheet's space through `place` (paper
// inches -> points, y up from the sheet's bottom left corner).  Fills: solid with their alpha,
// gradients as PDF shadings (axial, radial; varying alpha as a soft mask, lib/pdf), hatches as
// lines through the page's origin (a pixel at 96 DPI wide) stroked inside the clipped area
// (lib/pdf's fill_hatch: tiling patterns showed seams in pdfium); lines with
// width (0: the thinnest line the device draws), dashes, caps, joins, miter limit 4; pictures
// clipped to the shape's box (JPEG passed through, PNG as pixels with alpha, metafiles as the
// screen's placeholder); texts per run in the face the screen uses (diagram_view's FontSet with
// its substitution), scaled horizontally so that the run is as wide as the layout measured it,
// Letterspace as added advance, a synthetic italic slanted like the screen's.  A font whose
// licence forbids embedding (or that lib/pdf refuses) is replaced by the set's face of its class.
// Pages larger than PDF allows (200 inches a side) are scaled down to fit.

use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::geometry::{Path, Seg, Xform};
use diagram_core::render::{self, Cap, Drawing, Gradient, ImageItem, ItemKind, Join, Paint, Stroke, TextItem};
use diagram_core::{Document, PageRef};
use pdf::{Canvas, Dest, FillRule, FontId, ImageId, LineCap, LineJoin, Matrix, Origin, PageId, Rgb};

use super::{links, page_name, rgba, LinkTarget, HATCH_WIDTH, ITALIC_TAN};
use crate::fonts::{class_of, Class, Face, FontSet};
use crate::image::{self as picture, Decoded};
use crate::PageView;

/// what write puts into the PDF
#[derive(Clone, Debug, PartialEq)]
pub struct PdfOptions {
    /// the pages in order (export::pages)
    pub pages: Vec<PageRef>,
    /// background pages under the pages, hidden layers
    pub render: render::Options,
    /// the shapes' hyperlinks as PDF links
    pub links: bool,
    /// a bookmark per page (its name)
    pub bookmarks: bool,
    /// the document information (title, author ...)
    pub info: pdf::Info,
    /// the document's language ("de-DE"; empty: none)
    pub language: String,
}

impl Default for PdfOptions {
    fn default() -> PdfOptions {
        PdfOptions { pages: Vec::new(), render: render::Options::default(), links: true, bookmarks: true, info: pdf::Info::default(), language: String::new() }
    }
}

/// the export: the pages of `opts` as a PDF document
pub fn write(doc: &Document, fonts: &mut FontSet, opts: &PdfOptions) -> Result<Vec<u8>, pdf::Error> {
    if opts.pages.is_empty() {
        return Err(pdf::Error::Invalid("no page"));
    }
    let targets: Vec<(PageRef, PageId)> = opts.pages.iter().enumerate().map(|(k, p)| (*p, PageId(k as u32))).collect();
    let mut sheets = Sheets::new(fonts);
    for &page in &opts.pages {
        let view = PageView::new(doc, page, sheets.fonts, &opts.render);
        let d = view.drawing();
        // points, within what PDF allows
        let (w, h) = (d.width * 72.0, d.height * 72.0);
        let k = (pdf::PAGE_MAX / w.max(1e-9)).min(pdf::PAGE_MAX / h.max(1e-9)).min(1.0);
        let size = ((w * k).clamp(pdf::PAGE_MIN, pdf::PAGE_MAX), (h * k).clamp(pdf::PAGE_MIN, pdf::PAGE_MAX));
        let place = Xform { a: 72.0 * k, b: 0.0, c: 0.0, d: 72.0 * k, e: 0.0, f: 0.0 };
        let id = sheets.add(doc, d, size, place, None, if opts.links { &targets } else { &[] }, opts.links);
        if opts.bookmarks {
            sheets.pdf.add_outline(None, &page_name(doc, page), Dest::Fit(id));
        }
    }
    sheets.pdf.set_info(opts.info.clone());
    if !opts.language.is_empty() {
        sheets.pdf.set_language(&opts.language);
    }
    sheets.finish()
}

/// a resolved face for a text run: its font in the PDF, whether to slant it
#[derive(Clone, Copy)]
struct RunFont {
    id: FontId,
    synthetic: bool,
}

/// drawings on sheets of paper
pub struct Sheets<'f> {
    pdf: pdf::Document,
    fonts: &'f mut FontSet,
    /// faces embedded so far: (bytes, face index) -> font
    embedded: Vec<(Rc<[u8]>, u32, Option<FontId>)>,
    /// per family / bold / italic as runs ask
    runs: BTreeMap<(String, bool, bool), Option<RunFont>>,
    /// pictures by their bytes' hash and length
    pictures: BTreeMap<(u64, usize), Option<ImageId>>,
    /// text runs left out (no usable font)
    skipped: u32,
}

impl<'f> Sheets<'f> {
    /// an empty document whose texts take their faces from `fonts`
    pub fn new(fonts: &'f mut FontSet) -> Sheets<'f> {
        Sheets { pdf: pdf::Document::new(), fonts, embedded: Vec::new(), runs: BTreeMap::new(), pictures: BTreeMap::new(), skipped: 0 }
    }

    /// the document being written (outlines, information, language, compression)
    pub fn pdf(&mut self) -> &mut pdf::Document {
        &mut self.pdf
    }

    /// text runs left out so far because no font could be embedded
    pub fn skipped_runs(&self) -> u32 {
        self.skipped
    }

    /// a sheet of `size` points with the drawing on it: `place` maps paper inches (y up) to
    /// sheet points (y up from the bottom left corner), `clip` (x0, y0, x1, y1 in sheet points)
    /// cuts it (None: the sheet); `targets` names the PDF pages of the pages links may go to;
    /// `with_links` writes the shapes' hyperlinks
    pub fn add(&mut self, doc: &Document, d: &Drawing, size: (f64, f64), place: Xform, clip: Option<[f64; 4]>, targets: &[(PageRef, PageId)], with_links: bool) -> PageId {
        self.prepare(doc, d);
        let id = self.pdf.add_page(size.0, size.1);
        let links = if with_links { links(doc, d) } else { Vec::new() };
        let runs = &self.runs;
        let pictures = &self.pictures;
        let mut c = self.pdf.canvas(id, Origin::BottomLeft);
        if let Some(r) = clip {
            c.rect(r[0], r[1], r[2] - r[0], r[3] - r[1]);
            c.clip(FillRule::NonZero);
            c.end_path();
        }
        c.transform(Matrix::new(place.a, place.b, place.c, place.d, place.e, place.f));
        c.set_miter_limit(4.0);
        // the page's colour (white is the paper's own)
        if d.color & 0xFF_FFFF != 0xFF_FFFF {
            c.set_fill_color(Rgb::hex(d.color));
            c.rect(0.0, 0.0, d.width, d.height);
            c.fill(FillRule::NonZero);
        }
        for it in &d.items {
            match &it.kind {
                ItemKind::Fill { path, paint } => fill(&mut c, path, paint),
                ItemKind::Stroke { path, stroke } => line(&mut c, path, stroke),
                ItemKind::Text(t) => text(&mut c, runs, t),
                ItemKind::Image(img) => image(&mut c, doc, pictures, img),
            }
        }
        for l in &links {
            let b = l.bbox;
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            if !(w > 0.0 && h > 0.0) {
                continue;
            }
            match &l.target {
                LinkTarget::Uri(u) => c.link_uri(b[0], b[1], w, h, u),
                LinkTarget::Page(p) => {
                    if let Some(&(_, pid)) = targets.iter().find(|(r, _)| r == p) {
                        c.link_dest(b[0], b[1], w, h, Dest::Fit(pid));
                    }
                }
            }
        }
        drop(c);
        id
    }

    /// the PDF's bytes
    pub fn finish(self) -> Result<Vec<u8>, pdf::Error> {
        self.pdf.write()
    }

    // fonts and pictures go into the document before the page's canvas takes it
    fn prepare(&mut self, doc: &Document, d: &Drawing) {
        for it in &d.items {
            match &it.kind {
                ItemKind::Text(t) => {
                    for run in &t.runs {
                        let key = (run.family.to_ascii_lowercase(), run.bold, run.italic);
                        if !self.runs.contains_key(&key) {
                            let f = self.run_font(&run.family, run.bold, run.italic);
                            if f.is_none() {
                                self.skipped += 1;
                            }
                            self.runs.insert(key, f);
                        }
                    }
                }
                ItemKind::Image(img) => {
                    let Some(bytes) = picture::bytes(doc, &img.source) else { continue };
                    let key = (hash(&bytes), bytes.len());
                    if self.pictures.contains_key(&key) {
                        continue;
                    }
                    let im = if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) { pdf::Image::jpeg(bytes.clone()).ok() } else { None };
                    let im = im.or_else(|| match picture::decode(&bytes) {
                        Decoded::Picture(p) => {
                            let mut px = Vec::with_capacity(p.px.len() * 4);
                            for c in &p.px {
                                px.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, *c as u8, (c >> 24) as u8]);
                            }
                            pdf::Image::rgba(p.w, p.h, &px).ok().map(|i| i.interpolate(true))
                        }
                        Decoded::Placeholder(_) => None,
                    });
                    let id = im.map(|i| self.pdf.add_image(i));
                    self.pictures.insert(key, id);
                }
                _ => {}
            }
        }
    }

    // the face the screen draws a run with, embedded; else the set's faces of its class
    fn run_font(&mut self, family: &str, bold: bool, italic: bool) -> Option<RunFont> {
        if let Some((face, synthetic, _)) = self.fonts.pick(family, bold, italic) {
            if let Some(id) = self.embed(&face) {
                return Some(RunFont { id, synthetic });
            }
        }
        // the licence or the format refused: a face of the class, then any
        let class = class_of(family);
        let mut cands: Vec<Face> = self.fonts.faces.iter().filter(|f| f.class == Some(class)).cloned().collect();
        cands.extend(self.fonts.faces.iter().filter(|f| f.class == Some(Class::Sans)).cloned());
        cands.extend(self.fonts.faces.iter().cloned());
        cands.sort_by_key(|f| (f.bold != bold) as u8 * 2 + (f.italic != italic) as u8);
        for f in cands {
            if let Some(id) = self.embed(&f) {
                return Some(RunFont { id, synthetic: italic && !f.italic });
            }
        }
        None
    }

    fn embed(&mut self, f: &Face) -> Option<FontId> {
        if let Some(e) = self.embedded.iter().find(|e| Rc::ptr_eq(&e.0, &f.data) && e.1 == f.index) {
            return e.2;
        }
        let id = self.pdf.add_font(f.data.to_vec(), f.index).ok();
        self.embedded.push((f.data.clone(), f.index, id));
        id
    }
}

/// FNV-1a over the bytes (pictures used twice are embedded once)
fn hash(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h = (h ^ x as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// the path on the canvas; a fill closes every sub-path
fn append(c: &mut Canvas, path: &Path, fill: bool) -> bool {
    let mut any = false;
    for sub in &path.subs {
        if sub.segs.is_empty() {
            continue;
        }
        any = true;
        c.move_to(sub.start[0], sub.start[1]);
        for s in &sub.segs {
            match *s {
                Seg::Line(p) => c.line_to(p[0], p[1]),
                Seg::Cubic(a, b, p) => c.curve_to(a[0], a[1], b[0], b[1], p[0], p[1]),
            }
        }
        if sub.closed || fill {
            c.close();
        }
    }
    any
}

fn fill(c: &mut Canvas, path: &Path, paint: &Paint) {
    match paint {
        Paint::Solid(col) => {
            let (rgb, a) = rgba(*col);
            if a <= 0.0 {
                return;
            }
            c.set_fill_color(Rgb::new(rgb[0], rgb[1], rgb[2]));
            c.set_fill_alpha(a);
            if append(c, path, true) {
                c.fill(FillRule::EvenOdd);
            }
        }
        Paint::Gradient(g) => {
            // (lib/pdf multiplies the stops' alphas by the canvas's fill alpha)
            c.set_fill_alpha(1.0);
            let sh = shading(g);
            if append(c, path, true) {
                c.fill_shading(FillRule::EvenOdd, &sh);
            }
        }
        Paint::Hatch { color, lines } => {
            let (rgb, a) = rgba(*color);
            // (lib/pdf multiplies the hatch's alpha by the canvas's fill alpha)
            c.set_fill_alpha(1.0);
            for &(angle, spacing) in lines {
                if !(spacing.is_finite() && spacing > 1e-4 && angle.is_finite()) {
                    continue;
                }
                let h = pdf::Hatch { angle, spacing, width: HATCH_WIDTH, color: Rgb::new(rgb[0], rgb[1], rgb[2]), alpha: a, origin: [0.0, 0.0] };
                if append(c, path, true) {
                    c.fill_hatch(FillRule::EvenOdd, &h);
                }
            }
        }
    }
}

fn shading(g: &Gradient) -> pdf::Shading {
    let kind = if g.radial {
        let r = crate::sqrt((g.to[0] - g.from[0]) * (g.to[0] - g.from[0]) + (g.to[1] - g.from[1]) * (g.to[1] - g.from[1]));
        pdf::ShadingKind::Radial { from: g.from, r0: 0.0, to: g.from, r1: r }
    } else {
        pdf::ShadingKind::Axial { from: g.from, to: g.to }
    };
    let stops = g
        .stops
        .iter()
        .map(|&(t, col)| {
            let (rgb, a) = rgba(col);
            (t, Rgb::new(rgb[0], rgb[1], rgb[2]), a)
        })
        .collect();
    pdf::Shading { kind, stops, extend: (true, true) }
}

fn line(c: &mut Canvas, path: &Path, st: &Stroke) {
    let (rgb, a) = rgba(st.color);
    if a <= 0.0 {
        return;
    }
    c.set_stroke_color(Rgb::new(rgb[0], rgb[1], rgb[2]));
    c.set_stroke_alpha(a);
    c.set_line_width(if st.width.is_finite() && st.width > 0.0 { st.width } else { 0.0 });
    c.set_line_cap(match st.cap {
        Cap::Round => LineCap::Round,
        Cap::Butt => LineCap::Butt,
        Cap::Square => LineCap::Square,
    });
    c.set_line_join(match st.join {
        Join::Round => LineJoin::Round,
        Join::Miter => LineJoin::Miter,
        Join::Bevel => LineJoin::Bevel,
    });
    let dash_ok = !st.dash.is_empty() && st.dash.iter().all(|v| v.is_finite() && *v >= 0.0) && st.dash.iter().sum::<f64>() > 0.0;
    c.set_dash(if dash_ok { &st.dash } else { &[] }, 0.0);
    if append(c, path, false) {
        c.stroke();
    }
}

fn text(c: &mut Canvas, runs: &BTreeMap<(String, bool, bool), Option<RunFont>>, t: &TextItem) {
    let x = &t.xform;
    for run in &t.runs {
        if !super::run_visible(&run.text, run.size) {
            continue;
        }
        let Some(Some(f)) = runs.get(&(run.family.to_ascii_lowercase(), run.bold, run.italic)) else { continue };
        let (rgb, a) = rgba(run.color);
        if a <= 0.0 {
            continue;
        }
        let Some(upem) = c.document().font_metrics(f.id).map(|m| m.units_per_em.max(1) as f64) else { continue };
        let mut glyphs = c.document().shape(&[f.id], &run.text);
        if glyphs.is_empty() {
            continue;
        }
        // as wide as the layout measured the run (FontScale, the substitution's width factor)
        let natural: f64 = glyphs.iter().map(|g| g.advance as f64).sum::<f64>() * run.size / upem;
        let n = run.text.chars().count() as f64;
        let spacing = if run.spacing.is_finite() { run.spacing } else { 0.0 };
        let want = run.width - spacing * n;
        let sx = if natural > 1e-9 && want > 1e-9 { (want / natural).clamp(0.25, 4.0) } else { run.scale_x.clamp(0.25, 4.0) };
        if spacing != 0.0 {
            let add = (spacing / sx / run.size * upem) as i32;
            for g in glyphs.iter_mut() {
                g.advance = g.advance.saturating_add(add);
            }
        }
        c.set_fill_color(Rgb::new(rgb[0], rgb[1], rgb[2]));
        c.set_fill_alpha(a);
        c.save();
        c.transform(Matrix::new(x.a, x.b, x.c, x.d, x.e, x.f));
        c.translate(run.at[0], run.at[1]);
        c.transform(Matrix::new(sx, 0.0, if f.synthetic { ITALIC_TAN } else { 0.0 }, 1.0, 0.0, 0.0));
        c.glyphs(&[f.id], run.size, 0.0, 0.0, &glyphs, &run.text);
        c.restore();
    }
}

fn image(c: &mut Canvas, doc: &Document, pictures: &BTreeMap<(u64, usize), Option<ImageId>>, img: &ImageItem) {
    let id = picture::bytes(doc, &img.source).and_then(|b| pictures.get(&(hash(&b), b.len())).copied().flatten());
    c.save();
    let q = &img.clip;
    c.move_to(q[0][0], q[0][1]);
    for p in &q[1..] {
        c.line_to(p[0], p[1]);
    }
    c.close();
    c.clip(FillRule::NonZero);
    c.end_path();
    let m = &img.xform;
    match id {
        Some(id) => c.image_transform(id, Matrix::new(m.a, m.b, m.c, m.d, m.e, m.f)),
        None => {
            // the screen's placeholder (direct.rs): a light grey box with a frame and a cross
            let p: Vec<[f64; 2]> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().map(|v| m.apply(*v)).collect();
            c.set_fill_color(Rgb::hex(0xE4E6EA));
            c.set_fill_alpha(1.0);
            c.set_stroke_color(Rgb::hex(0x9AA0A8));
            c.set_stroke_alpha(1.0);
            c.set_line_width(1.0 / 96.0);
            c.set_dash(&[], 0.0);
            c.move_to(p[0][0], p[0][1]);
            for v in &p[1..] {
                c.line_to(v[0], v[1]);
            }
            c.close();
            c.fill_stroke(FillRule::NonZero);
            c.move_to(p[0][0], p[0][1]);
            c.line_to(p[2][0], p[2][1]);
            c.move_to(p[1][0], p[1][1]);
            c.line_to(p[3][0], p[3][1]);
            c.stroke();
        }
    }
    c.restore();
}
