// pagesetup -- a page's size against its drawing and its printer paper (PG-04, PG-05, RW-01 ..
// RW-03): orientation, a size of the menu, the page fitted to its drawing, automatic size in
// tiles of the printed sheet
//
//   size(doc, page) -> [w, h]              PageWidth, PageHeight (drawing units, inches)
//   scale(doc, page)                       paper inches per drawing inch (PageScale / DrawingScale)
//   landscape(doc, page)                   the page is wider than high
//   set_orientation(tx, eng, page, landscape)   Hochformat / Querformat: width and height
//                                          swapped when the page lies the other way,
//                                          PrintPageOrientation 1 / 2
//   set_paper_size(tx, eng, page, [w, h])  the page to w x h paper inches in its orientation
//   auto_size(doc, page) / set_auto_size(tx, page, on)   Automatisch anpassen
//   content(doc, page) -> Option<[x0, y0, x1, y1]>   the box of the page's top-level shapes
//   fit_to_drawing(tx, eng, page, margins) the page around its drawing, the shapes moved in
//   Tiles::new(paper, margins, zoom); grow(tx, eng, page, &tiles)   automatic size after a
//                                          gesture: whole tiles added or taken away
//
// The cells (Visio's ShapeSheet reference, Page Properties and Print Properties sections):
// * DrawingSizeType: 0 same as printer (visPrintSetup), 1 fit page to drawing contents
//   (visTight), 2 standard (visStandard), 3 custom page size (visCustom), 4 custom scaled, 5
//   metric (ISO), 6 ANSI engineering, 7 ANSI architectural.
// * DrawingResizeType: 0 the page resizes automatically when DrawingSizeType is 0, else not;
//   1 the page resizes automatically; 2 it does not.  set_auto_size writes 1 or 2.
// * PrintPageOrientation: 0 same as printer, 1 portrait, 2 landscape.
// A size chosen from the menu or fitted to the drawing turns the automatic size off: the Page
// Size register of Visio's Page Setup offers "let Visio expand the page" and a fixed size as
// alternatives (Visio 2016 Step by Step, chapter 3).  Assumption, not checked in Visio: fitting
// to the drawing turns it off as well (DrawingResizeType 2), so the next gesture does not cut
// the fitted page back into tiles.
//
// Automatic size (Step by Step, chapter 3: "Auto Size works both horizontally and vertically",
// the extended page goes again when its last shape leaves it): a tile is the printed sheet,
// the first one the whole paper, each further one the paper less the margins left and right
// (top and bottom) -- a page of the local corpus agrees: A4 landscape with 0.25 in margins and
// two by two tiles is 2 x 11.69 - 0.5 = 22.89 in wide and 2 x 8.27 - 0.5 = 16.04 in high.  After a
// gesture the page gains a tile on each side where a shape juts out and loses an empty tile at
// an edge, never below the first one; a page that grows to the left or down moves its drawing
// (the origin stays at the bottom left corner, as every coordinate of the file says).  Nothing
// happens on opening a file (PLAN.md, "Fallen"): only the app's gesture asks.

use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::ops;
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;
use crate::units::Unit;

const EPS: f64 = 1e-6;
/// at most this many tiles are added or taken away per side and gesture (a shape far away
/// leaves the page as it is)
const MAX_STEPS: u32 = 1000;

// a cell of the page sheet with inheritance (None: missing or not a number)
fn num(doc: &Document, page: PageRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(SheetRef::Page(page), &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

/// PageWidth and PageHeight in drawing units (inches; Visio's 8.5 x 11 in when missing)
pub fn size(doc: &Document, page: PageRef) -> [f64; 2] {
    [num(doc, page, "PageWidth").unwrap_or(8.5), num(doc, page, "PageHeight").unwrap_or(11.0)]
}

/// paper inches per drawing inch: PageScale / DrawingScale (1 without a scale)
pub fn scale(doc: &Document, page: PageRef) -> f64 {
    let ps = num(doc, page, "PageScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    let ds = num(doc, page, "DrawingScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    ps / ds
}

/// whether the page lies landscape (wider than high)
pub fn landscape(doc: &Document, page: PageRef) -> bool {
    let [w, h] = size(doc, page);
    w > h
}

// the unit a page cell is shown in (inches when it names none)
fn unit_of(doc: &Document, page: PageRef, cell: &str) -> Unit {
    doc.sym(cell).and_then(|c| doc.resolve(SheetRef::Page(page), &CellAddr::single(c)).unit()).unwrap_or(Unit::Inches)
}

// a number into a page cell, unless it has it already
fn put(tx: &mut Transaction, page: PageRef, cell: &str, v: f64, unit: Option<Unit>) -> Result<bool, EditError> {
    if num(tx.doc(), page, cell).is_some_and(|now| (now - v).abs() <= EPS) {
        return Ok(false);
    }
    let a = tx.single(cell);
    tx.set_constant(SheetRef::Page(page), &a, Value::Num(v), unit)?;
    Ok(true)
}

// PageWidth and PageHeight (drawing inches), each in the unit it is shown in
fn put_size(tx: &mut Transaction, page: PageRef, w: f64, h: f64) -> Result<bool, EditError> {
    let (uw, uh) = (unit_of(tx.doc(), page, "PageWidth"), unit_of(tx.doc(), page, "PageHeight"));
    let a = put(tx, page, "PageWidth", w, Some(uw))?;
    let b = put(tx, page, "PageHeight", h, Some(uh))?;
    Ok(a || b)
}

/// Hochformat (landscape = false) / Querformat: width and height swapped when the page lies
/// the other way, PrintPageOrientation 1 or 2, then what depends on the page computed again
/// -> whether anything changed
pub fn set_orientation(tx: &mut Transaction, eng: &mut Engine, page: PageRef, landscape: bool) -> Result<bool, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let [w, h] = size(tx.doc(), page);
    let mut changed = false;
    if (w - h).abs() > EPS && (w > h) != landscape {
        changed |= put_size(tx, page, h, w)?;
    }
    changed |= put(tx, page, "PrintPageOrientation", if landscape { 2.0 } else { 1.0 }, None)?;
    if changed {
        eng.recalc(tx, &[SheetRef::Page(page)])?;
    }
    Ok(changed)
}

/// a size of the menu (paper inches, portrait or landscape alike): the page gets it in its
/// own orientation (DrawingSizeType 2, standard; the automatic size goes off) -> changed
pub fn set_paper_size(tx: &mut Transaction, eng: &mut Engine, page: PageRef, paper: [f64; 2]) -> Result<bool, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let s = scale(tx.doc(), page);
    let (lo, hi) = (paper[0].min(paper[1]), paper[0].max(paper[1]));
    let (w, h) = if landscape(tx.doc(), page) { (hi, lo) } else { (lo, hi) };
    let mut changed = put_size(tx, page, w / s, h / s)?;
    changed |= put(tx, page, "DrawingSizeType", 2.0, None)?;
    changed |= put(tx, page, "DrawingResizeType", 2.0, None)?;
    if changed {
        eng.recalc(tx, &[SheetRef::Page(page)])?;
    }
    Ok(changed)
}

/// whether the page grows and shrinks with its drawing (Automatisch anpassen): DrawingResizeType
/// 1, or 0 with DrawingSizeType 0
pub fn auto_size(doc: &Document, page: PageRef) -> bool {
    match num(doc, page, "DrawingResizeType").map(|v| v as i64) {
        Some(1) => true,
        Some(2) => false,
        _ => num(doc, page, "DrawingSizeType").unwrap_or(0.0) as i64 == 0,
    }
}

/// turn the automatic size on (DrawingResizeType 1) or off (2) -> changed
pub fn set_auto_size(tx: &mut Transaction, page: PageRef, on: bool) -> Result<bool, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    put(tx, page, "DrawingResizeType", if on { 1.0 } else { 2.0 }, None)
}

/// the box of the page's top-level shapes on the page (drawing units, y up; guides and deleted
/// shapes left out); None for a page without shapes
pub fn content(doc: &Document, page: PageRef) -> Option<[f64; 4]> {
    let tops: Vec<_> = doc.top_shapes(Owner::Page(page)).filter(|&s| doc.shape(s).is_some_and(|x| x.header().kind != ShapeKind::Guide && !x.is_deleted())).collect();
    ops::bounds(doc, &tops)
}

/// the page around its drawing (Größe an Zeichnung anpassen): the box of its shapes and
/// `margins` (left, top, right, bottom; paper inches) around it, the shapes moved so that the
/// box starts at the left and bottom margin; DrawingSizeType 1 (fit page to drawing contents),
/// the automatic size off -> changed (false for a page without shapes)
pub fn fit_to_drawing(tx: &mut Transaction, eng: &mut Engine, page: PageRef, margins: [f64; 4]) -> Result<bool, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let Some(b) = content(tx.doc(), page) else { return Ok(false) };
    let s = scale(tx.doc(), page);
    let m: Vec<f64> = margins.iter().map(|v| v.max(0.0) / s).collect();
    let (w, h) = (b[2] - b[0] + m[0] + m[2], b[3] - b[1] + m[1] + m[3]);
    let d = [m[0] - b[0], m[3] - b[1]];
    let mut changed = false;
    if d[0].abs() > EPS || d[1].abs() > EPS {
        ops::shift_all(tx, eng, Owner::Page(page), d)?;
        changed = true;
    }
    changed |= put_size(tx, page, w.max(EPS), h.max(EPS))?;
    changed |= put(tx, page, "DrawingSizeType", 1.0, None)?;
    changed |= put(tx, page, "DrawingResizeType", 2.0, None)?;
    if changed {
        eng.recalc(tx, &[SheetRef::Page(page)])?;
    }
    Ok(changed)
}

/// the tiles of the automatic size on paper (inches): the first tile, each further one
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Tiles {
    /// the first tile: the paper (over the print zoom)
    pub base: [f64; 2],
    /// each further tile: the paper less the margins (over the print zoom)
    pub step: [f64; 2],
}

impl Tiles {
    /// the tiles of a sheet of `paper` (width, height; inches, in the page's print
    /// orientation) with `margins` (left, top, right, bottom) printed at `zoom` (1 = 100 %)
    pub fn new(paper: [f64; 2], margins: [f64; 4], zoom: f64) -> Tiles {
        let z = if zoom.is_finite() && zoom > 0.0 { zoom } else { 1.0 };
        let step = [(paper[0] - margins[0] - margins[2]).max(0.0), (paper[1] - margins[1] - margins[3]).max(0.0)];
        Tiles { base: [paper[0] / z, paper[1] / z], step: [step[0] / z, step[1] / z] }
    }
}

// one axis of the automatic size: the page [0, len] against the drawing [a, b] (None: no
// shapes) in steps of `step`, never shorter than `base` -> (new low edge, new high edge) in the
// old coordinates
fn tile_axis(len: f64, ab: Option<(f64, f64)>, base: f64, step: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (0.0, len);
    let fits = |lo: f64, hi: f64| hi - lo - step >= base - EPS;
    match ab {
        Some((a, b)) => {
            let mut n = 0;
            while a < lo - EPS {
                lo -= step;
                n += 1;
                if n > MAX_STEPS {
                    return None;
                }
            }
            n = 0;
            while b > hi + EPS {
                hi += step;
                n += 1;
                if n > MAX_STEPS {
                    return None;
                }
            }
            n = 0;
            while fits(lo, hi) && b <= hi - step + EPS && n < MAX_STEPS {
                hi -= step;
                n += 1;
            }
            n = 0;
            while fits(lo, hi) && a >= lo + step - EPS && n < MAX_STEPS {
                lo += step;
                n += 1;
            }
        }
        None => {
            let mut n = 0;
            while fits(lo, hi) && n < MAX_STEPS {
                hi -= step;
                n += 1;
            }
        }
    }
    Some((lo, hi))
}

/// the automatic size after a gesture (see the file header): the page gains or loses whole
/// tiles where its drawing juts out or left empty space, the drawing moves by what the page
/// grew or shrank at its left and bottom edge -> that move (drawing units) when the page
/// changed, None when it stays (the automatic size is off, it fits, a step is too small)
pub fn grow(tx: &mut Transaction, eng: &mut Engine, page: PageRef, tiles: &Tiles) -> Result<Option<[f64; 2]>, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    if !auto_size(tx.doc(), page) {
        return Ok(None);
    }
    // a size held by GUARD stays (a gesture must not fail on it)
    let doc = tx.doc();
    if ["PageWidth", "PageHeight"].iter().any(|c| doc.sym(c).is_some_and(|s| doc.is_guarded(SheetRef::Page(page), &CellAddr::single(s)))) {
        return Ok(None);
    }
    let s = scale(tx.doc(), page);
    let (base, step) = ([tiles.base[0] / s, tiles.base[1] / s], [tiles.step[0] / s, tiles.step[1] / s]);
    if step[0] <= EPS || step[1] <= EPS {
        return Ok(None);
    }
    let [w, h] = size(tx.doc(), page);
    let b = content(tx.doc(), page);
    let (Some(x), Some(y)) = (tile_axis(w, b.map(|b| (b[0], b[2])), base[0], step[0]), tile_axis(h, b.map(|b| (b[1], b[3])), base[1], step[1])) else {
        return Ok(None);
    };
    let (nw, nh) = (x.1 - x.0, y.1 - y.0);
    let d = [-x.0, -y.0];
    if (nw - w).abs() <= EPS && (nh - h).abs() <= EPS && d[0].abs() <= EPS && d[1].abs() <= EPS {
        return Ok(None);
    }
    if d[0].abs() > EPS || d[1].abs() > EPS {
        ops::shift_all(tx, eng, Owner::Page(page), d)?;
    }
    put_size(tx, page, nw, nh)?;
    eng.recalc(tx, &[SheetRef::Page(page)])?;
    Ok(Some(d))
}

#[cfg(test)]
mod tests;
