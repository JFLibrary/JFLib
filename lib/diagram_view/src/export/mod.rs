// export -- the pages of a diagram as files (JD12: EX-01 PDF, EX-03 SVG, EX-04 PNG, EX-05 JPEG)
//
//   pages(doc, &Pages) -> Vec<PageRef>     which pages: the one shown, every foreground page, a
//                                          range of them (from 1), a list
//   page_name(doc, page) -> String         Name, else NameU, else its number among the pages
//   links(doc, drawing) -> Vec<Link>       the shapes' hyperlinks with their boxes on the page
//   pdf::write(doc, fonts, &PdfOptions)    a vector PDF: a PDF page per page in its size, text
//                                          as text in embedded subsets, links, a bookmark per page
//   pdf::Sheets                            drawings placed on sheets of paper (printing: tiles,
//                                          margins, scaling) through the same drawing code
//   svg::write(doc, drawing, &SvgOptions)  one page as SVG: paths, text, pictures, gradients,
//                                          hatches, links
//   raster::render(view, &RasterOptions)   a page (or a part of it) at a resolution, on a colour
//                                          or transparent; raster::png / raster::jpeg encode it
//
// Why here and not in diagram_io: a page is exported as the screen shows it, so the texts must
// be measured with the fonts they are drawn with (PageView builds the display list with this
// crate's FontSet and render2d's text layout), the PDF must embed the very faces the screen
// uses (FontSet's substitution, TX-09), and PNG / JPEG are this crate's renderer.  diagram_io
// stays the file format crate without render2d; lib/pdf adds no crate this one did not pull in
// already (gfx, img_core, text_shape).
//
// Every export reads the display list (diagram_core::render::Drawing: paper inches, y up, the
// origin at the page's bottom left corner).  PDF and SVG write it in points (1/72 inch); SVG
// with y down (its own convention), PDF with y up.  Text runs keep the width the layout
// measured: the PDF scales each run horizontally to it, the SVG gives textLength.

pub mod pdf;
pub mod raster;
pub mod svg;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::render::Drawing;
use diagram_core::{Document, PageRef, ShapeRef, SheetRef};

/// which pages an export takes
#[derive(Clone, Debug, PartialEq)]
pub enum Pages {
    /// one page (the one shown; may be a background page)
    Current(PageRef),
    /// every foreground page in the document's order
    All,
    /// foreground pages from..=to, counted from 1 (clamped to the pages there are)
    Range { from: u32, to: u32 },
    /// these pages in this order
    List(Vec<PageRef>),
}

/// the foreground pages in the document's order
pub fn foreground_pages(doc: &Document) -> Vec<PageRef> {
    doc.pages().filter(|p| doc.page(*p).is_some_and(|pg| !pg.header.background)).collect()
}

/// the pages a selection names
pub fn pages(doc: &Document, sel: &Pages) -> Vec<PageRef> {
    match sel {
        Pages::Current(p) => doc.page(*p).map(|_| alloc::vec![*p]).unwrap_or_default(),
        Pages::All => foreground_pages(doc),
        Pages::Range { from, to } => {
            let fg = foreground_pages(doc);
            let a = (*from).max(1) as usize;
            let b = (*to as usize).min(fg.len());
            if a > b {
                return Vec::new();
            }
            fg[a - 1..b].to_vec()
        }
        Pages::List(l) => l.iter().copied().filter(|p| doc.page(*p).is_some()).collect(),
    }
}

/// a page's name: Name, else NameU, else its number among all pages (from 1)
pub fn page_name(doc: &Document, page: PageRef) -> String {
    if let Some(pg) = doc.page(page) {
        if let Some(s) = pg.header.name.or(pg.header.name_u) {
            let n = doc.str(s);
            if !n.is_empty() {
                return String::from(n);
            }
        }
    }
    let k = doc.pages().position(|p| p == page).map_or(0, |k| k + 1);
    format!("{k}")
}

/// the page a hyperlink's sub-address names ("Seite-2", "Seite-2/Sheet.5", "Page-2,200"), by
/// Name or NameU, case-insensitive
pub fn page_by_name(doc: &Document, sub_address: &str) -> Option<PageRef> {
    let name = sub_address.split(['/', ',']).next().unwrap_or("").trim();
    if name.is_empty() {
        return None;
    }
    doc.pages().find(|p| {
        doc.page(*p).is_some_and(|pg| [pg.header.name, pg.header.name_u].iter().flatten().any(|s| doc.str(*s).eq_ignore_ascii_case(name)))
    })
}

/// where a hyperlink goes
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// an address (URL, mail, file) as the file has it
    Uri(String),
    /// a page of the same document
    Page(PageRef),
}

/// a shape's hyperlink on the page
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub shape: ShapeRef,
    /// the box of what the shape draws (paper inches, y up)
    pub bbox: [f64; 4],
    pub target: LinkTarget,
    /// Description (may be empty)
    pub description: String,
}

/// the hyperlinks of the shapes a drawing shows: per shape its first Hyperlink row with an
/// address (else a sub-address naming a page), on the box of the items of the shape and its
/// subshapes; shapes without anything drawn have no link
pub fn links(doc: &Document, drawing: &Drawing) -> Vec<Link> {
    // the boxes per shape, a group's subshapes counted in for the group
    let mut boxes: Vec<(ShapeRef, [f64; 4])> = Vec::new();
    for it in &drawing.items {
        let mut cur = Some(it.shape);
        let mut depth = 0;
        while let Some(s) = cur {
            match boxes.iter_mut().find(|(r, _)| *r == s) {
                Some((_, b)) => *b = union(*b, it.bbox),
                None => boxes.push((s, it.bbox)),
            }
            depth += 1;
            if depth > 64 {
                break;
            }
            cur = doc.shape(s).and_then(|x| x.parent());
        }
    }
    let mut out = Vec::new();
    for (shape, bbox) in boxes {
        let rows = doc.hyperlinks(SheetRef::Shape(shape));
        let mut target = None;
        let mut description = String::new();
        for row in &rows {
            let address = row.text("Address").unwrap_or("").trim();
            let sub = row.text("SubAddress").unwrap_or("").trim();
            let t = if !address.is_empty() {
                Some(LinkTarget::Uri(String::from(address)))
            } else {
                page_by_name(doc, sub).map(LinkTarget::Page)
            };
            if t.is_some() {
                target = t;
                description = String::from(row.text("Description").unwrap_or(""));
                break;
            }
        }
        if let Some(target) = target {
            out.push(Link { shape, bbox, target, description });
        }
    }
    out
}

pub(crate) fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// 0xAARRGGBB -> (r, g, b) 0..1 and alpha 0..1
pub(crate) fn rgba(c: u32) -> ([f64; 3], f64) {
    let f = |s: u32| ((c >> s) & 255) as f64 / 255.0;
    ([f(16), f(8), f(0)], f(24))
}

/// a hatch line's width (inches): a pixel at 96 DPI, as the screen draws it at 100 %
pub(crate) const HATCH_WIDTH: f64 = 1.0 / 96.0;

/// tan of the synthetic italic's slant
pub(crate) const ITALIC_TAN: f64 = 0.202_710_035_508_672_5;

/// whether a text run is drawn (something to show, a size)
pub(crate) fn run_visible(text: &str, size: f64) -> bool {
    size.is_finite() && size > 0.0 && text.chars().any(|c| !c.is_whitespace() && !c.is_control())
}
