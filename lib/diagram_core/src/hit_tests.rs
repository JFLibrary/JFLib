// hit_tests -- the hit tests on self-built pages: fills even-odd, lines with their width,
// curves, texts, shadows and background pages left out, groups, rectangles, boxes; and the
// builders the tests of render::rebuild use too
//
//   cargo test -p jflib-diagram-core --lib hit

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::hit::{all_at, at, in_rect, item_box, top_boxes};
use crate::render::{self, ApproxMeasure, Drawing, Options};
use crate::*;

// ---- building pages (also for render/rebuild_tests.rs)

/// a document with one page (8.5 x 11 in, the defaults)
pub(crate) fn new_doc() -> (Document, PageRef) {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap();
    (doc, page)
}

/// a background page that `page` shows under it
pub(crate) fn add_background(doc: &mut Document, page: PageRef) -> PageRef {
    let bg = doc.insert_page(Page::new(PageHeader { id: 7, background: true, ..PageHeader::default() })).unwrap();
    doc.page_mut(page).unwrap().header.back_page = Some(7);
    bg
}

fn cell(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

/// a Geometry section (IX ix) of MoveTo and LineTo rows through `pts` (local inches)
pub(crate) fn poly(doc: &mut Document, ix: u32, pts: &[[f64; 2]]) -> Section {
    let geo = doc.intern("Geometry");
    let mut sec = Section::new(SecKey { name: geo, index: ix });
    for (i, p) in pts.iter().enumerate() {
        let mut r = Row::new(RowKey::Index(i as u32 + 1));
        r.kind = Some(doc.intern(if i == 0 { "MoveTo" } else { "LineTo" }));
        r.cells.push(cell(doc, "X", p[0]));
        r.cells.push(cell(doc, "Y", p[1]));
        sec.rows.push(r);
    }
    sec
}

/// the local rectangle of a w x h shape, closed
pub(crate) fn rect_pts(w: f64, h: f64) -> Vec<[f64; 2]> {
    alloc::vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]]
}

/// a shape at pin (its local pin in the middle) of size w x h with sections, more cells and a
/// text, under `parent` (None: top level) -> its ref
#[allow(clippy::too_many_arguments)]
pub(crate) fn add(doc: &mut Document, page: PageRef, parent: Option<ShapeRef>, id: u32, pin: [f64; 2], size: [f64; 2], sections: Vec<Section>, cells: &[(&str, f64)], text: Option<&str>) -> ShapeRef {
    add_kind(doc, page, parent, id, ShapeKind::Shape, pin, size, sections, cells, text)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_kind(doc: &mut Document, page: PageRef, parent: Option<ShapeRef>, id: u32, kind: ShapeKind, pin: [f64; 2], size: [f64; 2], sections: Vec<Section>, cells: &[(&str, f64)], text: Option<&str>) -> ShapeRef {
    let mut d = ShapeData::new(kind);
    for (n, v) in [("PinX", pin[0]), ("PinY", pin[1]), ("Width", size[0]), ("Height", size[1]), ("LocPinX", size[0] * 0.5), ("LocPinY", size[1] * 0.5)] {
        let c = cell(doc, n, v);
        d.sheet.cells.push(c);
    }
    for &(n, v) in cells {
        let c = cell(doc, n, v);
        d.sheet.cells.push(c);
    }
    d.sheet.sections = sections;
    if let Some(t) = text {
        d.text = Some(Box::new(Text::plain(t)));
    }
    let owner = Owner::Page(page);
    let slot = doc.page_mut(page).unwrap().shapes.insert(parent.map(|p| p.slot), id, d).unwrap();
    ShapeRef { owner, slot }
}

/// a filled rectangle with its line
pub(crate) fn add_rect(doc: &mut Document, page: PageRef, id: u32, pin: [f64; 2], size: [f64; 2], cells: &[(&str, f64)], text: Option<&str>) -> ShapeRef {
    let g = poly(doc, 0, &rect_pts(size[0], size[1]));
    add(doc, page, None, id, pin, size, alloc::vec![g], cells, text)
}

pub(crate) fn draw(doc: &Document, page: PageRef) -> Drawing {
    render::build(doc, page, &mut ApproxMeasure, &Options::default())
}

// ---- tests

#[test]
fn topmost_fill_and_line_tolerance() {
    let (mut doc, page) = new_doc();
    let a = add_rect(&mut doc, page, 1, [2.0, 2.0], [2.0, 2.0], &[("LineWeight", 0.02)], None);
    let b = add_rect(&mut doc, page, 2, [3.0, 3.0], [2.0, 2.0], &[], None);
    let d = draw(&doc, page);
    // the overlap: the later shape is on top
    assert_eq!(at(&d, [2.5, 2.5], 0.0).map(|h| h.shape), Some(b));
    assert_eq!(at(&d, [1.5, 1.5], 0.0).map(|h| (h.shape, h.top)), Some((a, a)));
    assert_eq!(at(&d, [5.0, 1.0], 0.05), None);
    // just outside A's left edge (x = 1): half the line (0.01) plus the tolerance
    assert_eq!(at(&d, [0.97, 1.5], 0.025).map(|h| h.shape), Some(a));
    assert_eq!(at(&d, [0.96, 1.5], 0.025), None);
    // both under the point, the top one first, each once
    let all: Vec<ShapeRef> = all_at(&d, [2.5, 2.5], 0.0).iter().map(|h| h.shape).collect();
    assert_eq!(all, alloc::vec![b, a]);
}

#[test]
fn even_odd_holes_and_curves() {
    let (mut doc, page) = new_doc();
    // a frame: an outer and an inner rectangle in two sections, filled even-odd
    let outer = poly(&mut doc, 0, &rect_pts(3.0, 3.0));
    let inner = poly(&mut doc, 1, &[[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0], [1.0, 1.0]]);
    let frame = add(&mut doc, page, None, 1, [2.5, 2.5], [3.0, 3.0], alloc::vec![outer, inner], &[], None);
    // an ellipse filling a 2 x 1 box
    let geo = doc.intern("Geometry");
    let mut sec = Section::new(SecKey::one(geo));
    let mut r = Row::new(RowKey::Index(1));
    r.kind = Some(doc.intern("Ellipse"));
    for (n, v) in [("X", 1.0), ("Y", 0.5), ("A", 2.0), ("B", 0.5), ("C", 1.0), ("D", 1.0)] {
        let c = cell(&mut doc, n, v);
        r.cells.push(c);
    }
    sec.rows.push(r);
    let ell = add(&mut doc, page, None, 2, [6.0, 6.0], [2.0, 1.0], alloc::vec![sec], &[], None);
    let d = draw(&doc, page);
    assert_eq!(at(&d, [1.5, 1.5], 0.0).map(|h| h.shape), Some(frame), "in the ring");
    assert_eq!(at(&d, [2.5, 2.5], 0.0), None, "in the hole");
    assert_eq!(at(&d, [2.5, 2.02], 0.03).map(|h| h.shape), Some(frame), "near the hole's edge");
    assert_eq!(at(&d, [6.0, 6.0], 0.0).map(|h| h.shape), Some(ell));
    // the corner of the ellipse's box is outside the curve
    assert_eq!(at(&d, [5.1, 5.6], 0.0), None);
    // a point just inside the curve at the far right (x = 7 at y = 6)
    assert_eq!(at(&d, [6.995, 6.0], 0.0).map(|h| h.shape), Some(ell));
}

#[test]
fn shadows_background_and_text() {
    let (mut doc, page) = new_doc();
    let bg = add_background(&mut doc, page);
    let big = add_rect(&mut doc, bg, 1, [4.0, 5.0], [8.0, 10.0], &[], None);
    // a shadowed shape: the shadow is offset by the page's default (0.125, -0.125)
    let s = add_rect(&mut doc, page, 1, [2.0, 2.0], [1.0, 1.0], &[("ShdwPattern", 1.0)], None);
    // a text without geometry
    let t = add(&mut doc, page, None, 2, [5.0, 8.0], [3.0, 1.0], Vec::new(), &[], Some("Hallo"));
    let d = draw(&doc, page);
    assert!(d.items.iter().any(|i| i.shape == big), "the background page is drawn");
    assert!(d.items.iter().any(|i| i.role == render::Role::Shadow));
    assert_eq!(at(&d, [2.0, 2.0], 0.0).map(|h| h.shape), Some(s));
    // only the shadow (and the background page) lies at (2.6, 1.4)
    assert_eq!(at(&d, [2.6, 1.4], 0.0), None);
    assert_eq!(at(&d, [7.0, 1.0], 0.0), None, "a background page's shape is not the page's");
    assert_eq!(at(&d, [5.0, 8.0], 0.0).map(|h| h.shape), Some(t), "on the text");
    assert_eq!(at(&d, [3.6, 8.45], 0.0), None, "in the block, away from the text");
    // boxes: the page's own shapes only; the text-only shape gets its text's box
    let boxes = top_boxes(&d);
    assert_eq!(boxes.iter().map(|b| b.0).collect::<Vec<_>>(), alloc::vec![s, t]);
    let b = boxes[0].1;
    assert!((b[0] - 1.5).abs() < 1e-9 && (b[1] - 1.5).abs() < 1e-9 && (b[2] - 2.5).abs() < 1e-9 && (b[3] - 2.5).abs() < 1e-9, "{b:?}");
    let tb = boxes[1].1;
    assert!(tb[0] > 4.0 && tb[2] < 6.0 && tb[1] > 7.7 && tb[3] < 8.3, "{tb:?}");
}

#[test]
fn groups_and_subselection() {
    let (mut doc, page) = new_doc();
    let g = add_kind(&mut doc, page, None, 1, ShapeKind::Group, [3.0, 3.0], [4.0, 4.0], Vec::new(), &[], None);
    let pa = poly(&mut doc, 0, &rect_pts(2.0, 2.0));
    let a = add(&mut doc, page, Some(g), 2, [1.5, 1.5], [2.0, 2.0], alloc::vec![pa], &[], None);
    let pb = poly(&mut doc, 0, &rect_pts(2.0, 2.0));
    let b = add(&mut doc, page, Some(g), 3, [2.5, 2.5], [2.0, 2.0], alloc::vec![pb], &[], None);
    let d = draw(&doc, page);
    // (group local (2, 2) is page (3, 3))
    let h = at(&d, [3.0, 3.0], 0.0).unwrap();
    assert_eq!((h.shape, h.top), (b, g));
    let all: Vec<(ShapeRef, ShapeRef)> = all_at(&d, [3.0, 3.0], 0.0).iter().map(|h| (h.shape, h.top)).collect();
    assert_eq!(all, alloc::vec![(b, g), (a, g)]);
    assert_eq!(in_rect(&d, [0.0, 0.0, 6.0, 6.0], true), alloc::vec![g]);
    assert_eq!(d.range_of(g), Some(0..d.items.len()));
    assert_eq!(d.range_of(a), None, "a subshape is no top-level shape");
}

#[test]
fn rectangles_whole_and_touching() {
    let (mut doc, page) = new_doc();
    let a = add_rect(&mut doc, page, 1, [2.0, 2.0], [1.0, 1.0], &[], None);
    let b = add_rect(&mut doc, page, 2, [4.0, 2.0], [1.0, 1.0], &[], None);
    // a diagonal line from (5, 5) to (7, 7): its box reaches (5.2, 6.8), the line does not
    let line = poly(&mut doc, 0, &[[0.0, 0.0], [2.0, 2.0]]);
    let l = add(&mut doc, page, None, 3, [6.0, 6.0], [2.0, 2.0], alloc::vec![line], &[], None);
    let d = draw(&doc, page);
    let r = [1.0, 1.0, 3.8, 3.0];
    assert_eq!(in_rect(&d, r, true), alloc::vec![a]);
    assert_eq!(in_rect(&d, r, false), alloc::vec![a, b]);
    // reversed corners are the same rectangle
    assert_eq!(in_rect(&d, [3.8, 3.0, 1.0, 1.0], false), alloc::vec![a, b]);
    let corner = [5.1, 6.4, 5.5, 6.9];
    assert_eq!(in_rect(&d, corner, false), Vec::<ShapeRef>::new(), "the line's box, not the line");
    assert_eq!(in_rect(&d, [5.5, 5.0, 6.5, 6.2], false), alloc::vec![l]);
    // a rectangle inside a filled shape touches it
    assert_eq!(in_rect(&d, [1.9, 1.9, 2.1, 2.1], false), alloc::vec![a]);
    // the exact box of the line: without its width
    let it = d.items.iter().find(|i| i.shape == l).unwrap();
    let bx = item_box(it);
    assert!((bx[0] - 5.0).abs() < 1e-12 && (bx[3] - 7.0).abs() < 1e-12, "{bx:?}");
}
