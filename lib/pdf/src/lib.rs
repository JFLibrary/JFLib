// pdf -- a vector PDF writer: pages of their own size drawn on a canvas (paths, fills with the
// non-zero or even-odd rule, strokes with width, dashes, caps, joins and miter limit,
// transforms, clipping, alpha, RGB colours, axial and radial gradients with any number of stops
// and alpha, hatches of parallel lines), images (JPEG passed through, other pixels with
// FlateDecode and their alpha as a soft mask), text in embedded TrueType / OpenType subsets
// (CID fonts with Identity-H, ToUnicode for search and copy, glyphs and widths from the
// library's shaping), links to URIs and to places in the document, nested outlines
// (bookmarks), the document information and XMP metadata.  PDF 1.7 with a classic xref table;
// the same document always gives the same bytes.  Not here: PDF/A, encryption, forms.  And a
// reader, pdf::read: structure, encryption, text, outlines, forms, attachments, and drawing pages
// (below).
//
//   let mut doc = Document::new();
//   let page = doc.add_page(A4.0, A4.1);              // points (1/72 inch); mm(210.0) converts
//   let font = doc.add_font(bytes, 0)?;                // TrueType / OpenType (CFF), a collection's face
//   {
//       let mut c = doc.canvas(page, Origin::TopLeft); // or BottomLeft (PDF's own page space)
//       c.set_fill_color(Rgb::hex(0x2060C0));
//       c.rect(40.0, 40.0, 200.0, 100.0);
//       c.fill(FillRule::NonZero);
//       c.text(font, 12.0, 40.0, 180.0, "Hallo, Welt");  // shaped; returns the advance
//       c.link_uri(40.0, 168.0, 80.0, 16.0, "https://example.org/");
//   }                                                  // the drawing goes into the page here
//   doc.add_outline(None, "Start", Dest::Fit(page));
//   let bytes = doc.write()?;
//
//   Document     add_page / page_size / page_count, canvas(page, origin) -> Canvas,
//                add_font -> FontId, font_metrics, shape, text_width, add_image(Image) -> ImageId,
//                add_outline -> OutlineId, set_outline_open, set_info(Info), set_language,
//                set_compression, write() -> bytes
//   Canvas       save / restore, transform / translate / scale / rotate, fill / stroke colour and
//                alpha, line width, cap, join, miter limit, dash, path building (move_to,
//                line_to, curve_to, quad_to, rect, rounded_rect, ellipse, circle, arc, close,
//                append_path), fill / stroke / fill_stroke / clip / end_path, shade /
//                fill_shading (gradients), fill_hatch, image / image_transform, text /
//                text_fallback / glyphs, set_text_mode, link_uri / link_dest, dest (a
//                destination at a point of this canvas)
//   Path         the same path building, kept and drawn later (Canvas::fill_path, stroke_path)
//   Shading      a gradient: ShadingKind Axial / Radial (PDF shading types 2 and 3), stops
//                (t, colour, alpha), extend; Shading::axial / radial
//   Hatch        parallel lines: angle, spacing, width, colour, alpha, a point on a line
//   Image        jpeg(file), gray / rgb / rgba(w, h, pixels), from_surface(gfx::Surface),
//                with_alpha, interpolate
//
// Text: Canvas::text shapes one line with lib/text_shape (HarfRust: kerning, ligatures, marks,
// Arabic joining, bidi); Canvas::glyphs takes glyphs a layout shaped already (pdf::Glyph).  Each glyph is
// placed where the shaping put it (TJ adjustments against the font's widths); the CIDs are the
// font's glyph ids.  The first glyph of each cluster maps to the cluster's text in ToUnicode.
// Variable fonts are embedded at their default instance, colour glyphs as their outlines.
// Fonts whose OS/2 fsType forbids embedding are refused (Error::FontLicense); fsType "no
// subsetting" embeds every glyph.
//
// Gradients (shading.rs): DeviceRGB shadings with a Type 2 function per stretch between stops,
// stitched by a Type 3 function; one alpha for all stops is the fill alpha (ExtGState ca), varying
// alpha a luminosity soft mask (a transparency group painting the alphas as a DeviceGray shading
// in the same user space).  Degenerate gradients paint the last stop's colour as SVG does.
// Hatches are clipped, stroked lines in one path, not tiling patterns (pdfium shows seams between
// rotated pattern cells, and readers round the cell step to device pixels).
//
// Reading: pdf::read (read/mod.rs) opens PDF files of every version, also encrypted and damaged
// ones, and gives their objects, pages, text with the position of every glyph, information,
// outlines, links and other annotations, form fields, attachments and page labels.  It shares
// Info and Date with the writer and nothing else.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod canvas;
mod cff;
mod font;
mod geom;
mod image;
mod obj;
pub mod read;
mod shading;
mod write;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

pub use canvas::{Canvas, Origin};
pub use font::FontMetrics;
pub use geom::{FillRule, LineCap, LineJoin, Matrix, Path, Rgb, TextMode};
pub use image::Image;
pub use shading::{Hatch, Shading, ShadingKind};

/// A4 portrait in points
pub const A4: (f64, f64) = (595.275_590_551_181_1, 841.889_763_779_527_6);
/// A3 portrait in points
pub const A3: (f64, f64) = (841.889_763_779_527_6, 1_190.551_181_102_362_2);
/// A5 portrait in points
pub const A5: (f64, f64) = (419.527_559_055_118_1, 595.275_590_551_181_1);
/// US Letter portrait in points
pub const LETTER: (f64, f64) = (612.0, 792.0);
/// US Legal portrait in points
pub const LEGAL: (f64, f64) = (612.0, 1008.0);

/// millimetres in points
pub fn mm(v: f64) -> f64 {
    v * 72.0 / 25.4
}

/// inches in points
pub fn inch(v: f64) -> f64 {
    v * 72.0
}

/// the smallest and largest page side PDF readers take (ISO 32000-1, Annex C)
pub const PAGE_MIN: f64 = 3.0;
pub const PAGE_MAX: f64 = 14_400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// the font file cannot be used: damaged, or a kind not taken
    Font(&'static str),
    /// the font's licence (OS/2 fsType) does not allow embedding
    FontLicense(&'static str),
    /// image data that does not fit what it claims
    Image(&'static str),
    /// valid input this writer does not handle (CFF2 fonts, arithmetic-coded JPEG, ...)
    Unsupported(&'static str),
    /// the document cannot be written as it is (no pages, a link to a page it lacks, ...)
    Invalid(&'static str),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Font(s) => write!(f, "font: {s}"),
            Error::FontLicense(s) => write!(f, "font licence: {s}"),
            Error::Image(s) => write!(f, "image: {s}"),
            Error::Unsupported(s) => write!(f, "not supported: {s}"),
            Error::Invalid(s) => write!(f, "invalid document: {s}"),
        }
    }
}

impl core::error::Error for Error {}

/// a shaped glyph for Canvas::glyphs: Document::shape gives them, a text layout that shaped
/// its lines already builds its own
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Glyph {
    /// which of the fonts passed along (0: the first)
    pub font: usize,
    /// the glyph id in that font
    pub id: u32,
    /// how far the pen moves after the glyph, in font units
    pub advance: i32,
    /// the glyph's offset from the pen in font units (y up)
    pub x: i32,
    pub y: i32,
    /// the byte range of the text the glyph stands for (its cluster): cluster..end
    pub cluster: usize,
    pub end: usize,
}

/// a page of a document (Document::add_page), counted from 0
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PageId(pub u32);

/// a font of a document (Document::add_font)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontId(u32);

/// an image of a document (Document::add_image)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageId(u32);

/// an outline item (Document::add_outline)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutlineId(u32);

/// a place in the document for links and outline items
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dest {
    /// the whole page in the window
    Fit(PageId),
    /// the point (left, top) of PDF page space (points from the bottom left, y up) at the top
    /// left of the window; zoom None keeps the reader's (Canvas::dest builds one from a
    /// canvas's own coordinates)
    Xyz { page: PageId, left: f64, top: f64, zoom: Option<f64> },
}

impl Dest {
    fn page(&self) -> PageId {
        match *self {
            Dest::Fit(p) | Dest::Xyz { page: p, .. } => p,
        }
    }
}

/// a date of the document information: local time and its offset from UTC
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// minutes east of UTC (+60: Central European Time), -1439 to 1439
    pub offset_minutes: i16,
}

/// the document information (the Info dictionary and the same in XMP metadata); unset fields
/// are left out
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    /// the program the document was made with
    pub creator: Option<String>,
    /// the program that wrote the PDF (None: "JFLib")
    pub producer: Option<String>,
    pub created: Option<Date>,
    pub modified: Option<Date>,
}

#[derive(Clone, Debug)]
pub(crate) enum LinkTarget {
    Uri(String),
    Dest(Dest),
}

#[derive(Clone, Debug)]
pub(crate) struct Link {
    /// x0, y0, x1, y1 in page space
    pub(crate) rect: [f64; 4],
    pub(crate) target: LinkTarget,
}

pub(crate) struct PageData {
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) content: Vec<u8>,
    pub(crate) fonts: BTreeSet<usize>,
    pub(crate) images: BTreeSet<usize>,
    pub(crate) gstates: BTreeSet<usize>,
    pub(crate) shadings: BTreeSet<usize>,
    pub(crate) masks: BTreeSet<usize>,
    pub(crate) links: Vec<Link>,
    pub(crate) transparency: bool,
}

pub(crate) struct OutlineItem {
    pub(crate) title: String,
    pub(crate) dest: Dest,
    pub(crate) parent: Option<usize>,
    pub(crate) open: bool,
}

/// a PDF document being built: its pages, fonts, images, outlines and information
pub struct Document {
    pub(crate) pages: Vec<PageData>,
    pub(crate) fonts: Vec<font::FontData>,
    pub(crate) images: Vec<Image>,
    /// ExtGState table: (fill alpha, stroke alpha) in thousandths
    pub(crate) alphas: Vec<(u16, u16)>,
    pub(crate) alpha_ids: BTreeMap<(u16, u16), usize>,
    /// shading dictionaries (without << >>), each written once
    pub(crate) shadings: Vec<Vec<u8>>,
    pub(crate) shading_ids: BTreeMap<Vec<u8>, usize>,
    /// soft masks: the DeviceGray shading their group paints and the group's /BBox
    pub(crate) masks: Vec<(usize, [f64; 4])>,
    pub(crate) mask_ids: BTreeMap<(usize, Vec<u8>), usize>,
    pub(crate) outlines: Vec<OutlineItem>,
    pub(crate) info: Info,
    pub(crate) lang: Option<String>,
    pub(crate) level: u8,
}

impl Default for Document {
    fn default() -> Document {
        Document::new()
    }
}

impl Document {
    pub fn new() -> Document {
        Document {
            pages: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            alphas: Vec::new(),
            alpha_ids: BTreeMap::new(),
            shadings: Vec::new(),
            shading_ids: BTreeMap::new(),
            masks: Vec::new(),
            mask_ids: BTreeMap::new(),
            outlines: Vec::new(),
            info: Info::default(),
            lang: None,
            level: 6,
        }
    }

    /// a new page at the end, width and height in points (clamped to PAGE_MIN .. PAGE_MAX)
    pub fn add_page(&mut self, width: f64, height: f64) -> PageId {
        let side = |v: f64| if v.is_finite() { v.clamp(PAGE_MIN, PAGE_MAX) } else { A4.0 };
        self.pages.push(PageData {
            width: side(width),
            height: side(height),
            content: Vec::new(),
            fonts: BTreeSet::new(),
            images: BTreeSet::new(),
            gstates: BTreeSet::new(),
            shadings: BTreeSet::new(),
            masks: BTreeSet::new(),
            links: Vec::new(),
            transparency: false,
        });
        PageId(self.pages.len() as u32 - 1)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// width and height in points
    pub fn page_size(&self, page: PageId) -> Option<(f64, f64)> {
        self.pages.get(page.0 as usize).map(|p| (p.width, p.height))
    }

    /// drawing on a page (panics for a page this document does not have)
    pub fn canvas(&mut self, page: PageId, origin: Origin) -> Canvas<'_> {
        assert!((page.0 as usize) < self.pages.len(), "pdf: no page {}", page.0);
        Canvas::new(self, page.0 as usize, origin)
    }

    /// a TrueType or OpenType font (glyf or CFF outlines; `face`: of a collection, else 0)
    pub fn add_font(&mut self, data: Vec<u8>, face: u32) -> Result<FontId, Error> {
        let f = font::FontData::parse(data, face)?;
        self.fonts.push(f);
        Ok(FontId(self.fonts.len() as u32 - 1))
    }

    pub fn font_metrics(&self, font: FontId) -> Option<FontMetrics> {
        self.fonts.get(font.0 as usize).map(|f| f.metrics)
    }

    /// one line shaped with the fonts (the first, then fallbacks per script run); empty for an
    /// unknown font or a line beyond 64 KiB
    pub fn shape(&self, fonts: &[FontId], text: &str) -> Vec<Glyph> {
        let Some(first) = fonts.first().and_then(|f| self.fonts.get(f.0 as usize)) else { return Vec::new() };
        let mut rest = Vec::new();
        for f in &fonts[1..] {
            match self.fonts.get(f.0 as usize) {
                Some(fd) => rest.push(&fd.engine),
                None => return Vec::new(),
            }
        }
        let glyphs = first.engine.shape_with_fallback(text, &rest).unwrap_or_default();
        glyphs.iter().map(|g| Glyph { font: g.font, id: g.id, advance: g.advance, x: g.x, y: g.y, cluster: g.cluster, end: g.end }).collect()
    }

    /// the advance of one shaped line at `size` points
    pub fn text_width(&self, fonts: &[FontId], size: f64, text: &str) -> f64 {
        let mut w = 0.0;
        for g in self.shape(fonts, text) {
            if let Some(f) = fonts.get(g.font).and_then(|f| self.fonts.get(f.0 as usize)) {
                w += g.advance as f64 * size / f.upem as f64;
            }
        }
        w
    }

    pub fn add_image(&mut self, img: Image) -> ImageId {
        self.images.push(img);
        ImageId(self.images.len() as u32 - 1)
    }

    /// an outline item (bookmark) under `parent` (None: at the top), after its siblings; open
    /// (its children shown) until set_outline_open says otherwise
    pub fn add_outline(&mut self, parent: Option<OutlineId>, title: &str, dest: Dest) -> OutlineId {
        let parent = parent.map(|p| p.0 as usize).filter(|&p| p < self.outlines.len());
        self.outlines.push(OutlineItem { title: String::from(title), dest, parent, open: true });
        OutlineId(self.outlines.len() as u32 - 1)
    }

    pub fn set_outline_open(&mut self, item: OutlineId, open: bool) {
        if let Some(o) = self.outlines.get_mut(item.0 as usize) {
            o.open = open;
        }
    }

    pub fn set_info(&mut self, info: Info) {
        self.info = info;
    }

    /// the document's language (BCP 47, e.g. "de-DE")
    pub fn set_language(&mut self, lang: &str) {
        self.lang = Some(String::from(lang));
    }

    /// DEFLATE level 0 (streams uncompressed, for reading the file) to 9; default 6
    pub fn set_compression(&mut self, level: u8) {
        self.level = level.min(9);
    }

    /// the PDF file
    pub fn write(&self) -> Result<Vec<u8>, Error> {
        write::write(self)
    }

    /// a shading dictionary's index (the same dictionary twice: one object)
    pub(crate) fn add_shading(&mut self, dict: Vec<u8>) -> usize {
        let next = self.shadings.len();
        let i = *self.shading_ids.entry(dict.clone()).or_insert(next);
        if i == next {
            self.shadings.push(dict);
        }
        i
    }

    /// a soft mask's index: the group painting gray shading g over bbox
    pub(crate) fn add_mask(&mut self, g: usize, bbox: [f64; 4]) -> usize {
        let mut key = Vec::new();
        obj::nums(&mut key, &bbox);
        let next = self.masks.len();
        let i = *self.mask_ids.entry((g, key)).or_insert(next);
        if i == next {
            self.masks.push((g, bbox));
        }
        i
    }
}
