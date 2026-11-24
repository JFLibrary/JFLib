// ops::points -- control points (SP-04) and vertices (SP-05) of a shape
//
//   controls(doc, shape) -> Vec<Control>         the visible control points on the page
//   set_control(tx, eng, shape, row, at)         a control point dragged to a page point
//   vertices(doc, shape) -> Vec<Vertex>          the points of the Geometry rows on the page
//   set_vertex / insert_vertex / delete_vertex   move one, add one after a row, take one out
//
// A control point is a row of the Controls section: X and Y in local coordinates, XDyn/YDyn
// the point its rubber band starts from, XCon/YCon how it behaves ([MS-VSDX] Controls; the
// Visio constants visCtlProportional 0, visCtlLocked 1, visCtlOffsetMin 2, visCtlOffsetMid 3,
// visCtlOffsetMax 4, the same plus 5 for a hidden point), CanGlue, Prompt (the tip).  Dragging
// writes X and Y the way Visio does (seen in the local files: X "Width*0.6734" for 0,
// "Width-0.217DL" for 4, a BOUND(...) keeps its form and gets the new value as its first
// argument): proportional "Width*k" / "Height*k" for 0, the value for 2, "Width*0.5+dDL" for 3,
// "Width-dDL" for 4; a locked direction stays.  A glued control point (a Connect entry with
// FromCell Controls.<row>) is let go.
//
// Vertices are the end points of the Geometry rows that have one (MoveTo, LineTo, ArcTo,
// EllipticalArcTo, NURBSTo, PolylineTo, the spline rows, the relative rows; not Ellipse and
// InfiniteLine); the last row of a closed sub-path, which repeats its start, is not a vertex of
// its own and moves with the start.  Relative rows keep fractions of Width and Height,
// absolute rows keep the proportional form of their formulas (Visio's "Width*0.5").  Adding and
// taking out a vertex inserts or removes a row in the middle of the section
// (Transaction::insert_row / remove_row: the rows after it are renumbered, formulas follow, an
// instance's inherited section becomes its own).  LockVtxEdit refuses (Invalid).  (JD21, SP-05)
// After each of them the box follows the geometry (shapeops::fit_frame: Width, Height, Pin and
// the rows into the box of the geometry, the shape stays on the page; not with LockCalcWH), as
// Visio does after a vertex edit.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{keep_form, locked, text, val, Put, Writer, EPS};
use crate::cell::{Formula, Value};
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::Xform;
use crate::names;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::strings::Sym;

/// a control point of a shape
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// its row of the Controls section (set_control takes it back)
    pub row: RowKey,
    /// the row name (Row_1, TextPosition ...)
    pub name: String,
    /// where it is on the page
    pub at: [f64; 2],
    /// where its rubber band starts on the page (XDyn, YDyn)
    pub anchor: [f64; 2],
    /// the tip (Prompt)
    pub tip: String,
    /// whether it can glue to other shapes (CanGlue)
    pub can_glue: bool,
    /// whether it cannot move sideways (XCon 1 or 6)
    pub x_locked: bool,
    /// whether it cannot move up and down (YCon 1 or 6)
    pub y_locked: bool,
}

fn control_key() -> SecKey {
    SecKey::one(names::CONTROL)
}

fn row_num(doc: &Document, sheet: SheetRef, row: RowKey, cell: Sym) -> Option<f64> {
    doc.resolve(sheet, &CellAddr::row(control_key(), row, cell)).num()
}

fn row_text(doc: &Document, sheet: SheetRef, row: RowKey, cell: &str) -> String {
    let Some(c) = doc.sym(cell) else { return String::new() };
    match doc.resolve(sheet, &CellAddr::row(control_key(), row, c)).value() {
        Value::Text(t) => String::from(doc.str(t)),
        _ => String::new(),
    }
}

fn behaviour(doc: &Document, sheet: SheetRef, row: RowKey, cell: &str) -> i64 {
    doc.sym(cell).and_then(|c| row_num(doc, sheet, row, c)).map_or(0, |v| v as i64)
}

/// the visible control points of a shape on the page (hidden ones, XCon or YCon 5 to 9, left
/// out)
pub fn controls(doc: &Document, shape: ShapeRef) -> Vec<Control> {
    let sheet = SheetRef::Shape(shape);
    let xf = doc.page_xform(shape);
    let mut out = Vec::new();
    for r in doc.rows(sheet, control_key()) {
        let (xc, yc) = (behaviour(doc, sheet, r.key, "XCon"), behaviour(doc, sheet, r.key, "YCon"));
        if xc >= 5 || yc >= 5 {
            continue;
        }
        let x = row_num(doc, sheet, r.key, names::X).unwrap_or(0.0);
        let y = row_num(doc, sheet, r.key, names::Y).unwrap_or(0.0);
        let dx = doc.sym("XDyn").and_then(|c| row_num(doc, sheet, r.key, c)).unwrap_or(x);
        let dy = doc.sym("YDyn").and_then(|c| row_num(doc, sheet, r.key, c)).unwrap_or(y);
        let name = match r.key {
            RowKey::Named(n) => String::from(doc.str(n)),
            RowKey::Index(i) => format!("Row_{}", i + 1),
        };
        out.push(Control {
            row: r.key,
            name,
            at: xf.apply([x, y]),
            anchor: xf.apply([dx, dy]),
            tip: row_text(doc, sheet, r.key, "Prompt"),
            can_glue: doc.sym("CanGlue").and_then(|c| row_num(doc, sheet, r.key, c)).is_some_and(|v| v != 0.0),
            x_locked: xc == 1,
            y_locked: yc == 1,
        });
    }
    out
}

// what a control point's X (or Y) gets for the local value v: by its behaviour, inside a
// BOUND(...) of its formula when it has one
fn control_put(doc: &Document, sheet: SheetRef, addr: &CellAddr, v: f64, size: f64, beh: i64, cell: &str) -> Option<Put> {
    // the expression of the new value, and whether it is a plain value
    let (expr, plain) = match beh {
        1 => return None,
        0 if size.abs() > EPS => (format!("{cell}*{}", text::num(v / size)), false),
        3 => {
            let d = v - size * 0.5;
            (if d < 0.0 { format!("{cell}*0.5-{}DL", text::num(-d)) } else { format!("{cell}*0.5+{}DL", text::num(d)) }, false)
        }
        4 => (format!("{cell}-{}DL", text::num(size - v)), false),
        _ => (format!("{}DL", text::num(v)), true),
    };
    if let Formula::Expr(f) = doc.resolve(sheet, addr).formula() {
        let f = doc.str(f);
        if let Some((s, e)) = text::call_arg(f, "BOUND") {
            return Some(Put::Formula(text::replace(f, alloc::vec![((s, e), expr)])));
        }
    }
    Some(if plain { Put::Num(v) } else { Put::Formula(expr) })
}

/// drag a control point (its row from controls()) to the page point `at`: X and Y as Visio
/// writes them for the point's behaviour (see the module comment), the point let go when it
/// was glued
pub fn set_control(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, row: RowKey, at: [f64; 2]) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(shape);
    let (sets, unglue) = {
        let doc = tx.doc();
        let sh = doc.shape(shape).ok_or(EditError::NoShape)?;
        if !doc.has_row(sheet, control_key(), row) {
            return Err(EditError::NoRow);
        }
        let q = doc.page_xform(shape).inverse().ok_or(EditError::Invalid)?.apply(at);
        let (w, h) = (val(doc, sheet, names::WIDTH), val(doc, sheet, names::HEIGHT));
        let (xc, yc) = (behaviour(doc, sheet, row, "XCon") % 5, behaviour(doc, sheet, row, "YCon") % 5);
        let ax = CellAddr::row(control_key(), row, names::X);
        let ay = CellAddr::row(control_key(), row, names::Y);
        let mut sets = Vec::new();
        if let Some(p) = control_put(doc, sheet, &ax, q[0], w, xc, "Width") {
            sets.push((ax, p));
        }
        if let Some(p) = control_put(doc, sheet, &ay, q[1], h, yc, "Height") {
            sets.push((ay, p));
        }
        // a Connect entry from this control point
        let id = sh.id();
        let name = match row {
            RowKey::Named(n) => format!("Controls.{}", doc.str(n)),
            RowKey::Index(i) => format!("Controls.Row_{}", i + 1),
        };
        let unglue: Vec<usize> = super::connects(doc, shape.owner)
            .iter()
            .enumerate()
            .filter(|(_, c)| c.from_sheet == id && c.from_cell.is_some_and(|f| doc.str(f) == name))
            .map(|(i, _)| i)
            .collect();
        (sets, unglue)
    };
    for &i in unglue.iter().rev() {
        tx.remove_connect(shape.owner, i)?;
    }
    let mut w = Writer::new();
    w.put_all(tx, eng, sheet, sets)?;
    w.finish(tx, eng)
}

/// a vertex of a shape: Geometry section (0-based index, Geometry1 is 0), row (IX) and where it
/// is on the page
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Vertex {
    /// the Geometry section's index
    pub section: u32,
    /// the row's IX
    pub row: u32,
    /// where it is on the page
    pub at: [f64; 2],
}

// a Geometry row as the vertex functions see it
#[derive(Copy, Clone, Debug)]
struct VRow {
    ix: u32,
    // its point in local coordinates
    p: [f64; 2],
    rel: bool,
    start: bool,
    // the last row of a closed sub-path (repeats the start)
    closing: bool,
    // the IX of its sub-path's start
    sub: u32,
}

fn is_rel(kind: &str) -> bool {
    kind.starts_with("Rel")
}

fn has_point(kind: &str) -> bool {
    !matches!(kind, "Ellipse" | "InfiniteLine" | "")
}

// the rows of a Geometry section with points, sub-paths and closing rows
fn vrows(doc: &Document, sheet: SheetRef, sk: SecKey, w: f64, h: f64) -> Vec<VRow> {
    let rows = doc.rows(sheet, sk);
    let mut out: Vec<VRow> = Vec::new();
    for r in &rows {
        let RowKey::Index(ix) = r.key else { continue };
        let kind = r.kind.map_or("", |k| doc.str(k));
        if !has_point(kind) {
            continue;
        }
        let num = |c: Sym| doc.resolve(sheet, &CellAddr::row(sk, r.key, c)).num().unwrap_or(0.0);
        let rel = is_rel(kind);
        let (x, y) = (num(names::X), num(names::Y));
        let p = if rel { [x * w, y * h] } else { [x, y] };
        let start = kind == "MoveTo" || kind == "RelMoveTo" || out.is_empty();
        let sub = if start { ix } else { out.last().map_or(ix, |v| v.sub) };
        out.push(VRow { ix, p, rel, start, closing: false, sub });
    }
    // a sub-path's last row closes it when it ends at its start
    for i in 0..out.len() {
        let last = i + 1 == out.len() || out[i + 1].start;
        if last && !out[i].start {
            if let Some(s) = out.iter().find(|v| v.ix == out[i].sub) {
                let tol = 1e-9 * (1.0 + s.p[0].abs().max(s.p[1].abs()));
                if (s.p[0] - out[i].p[0]).abs() <= tol && (s.p[1] - out[i].p[1]).abs() <= tol {
                    out[i].closing = true;
                }
            }
        }
    }
    out
}

fn geo_key(section: u32) -> SecKey {
    SecKey { name: names::GEOMETRY, index: section }
}

/// the vertices of a shape on the page, section by section (hidden sections left out)
pub fn vertices(doc: &Document, shape: ShapeRef) -> Vec<Vertex> {
    let sheet = SheetRef::Shape(shape);
    let xf = doc.page_xform(shape);
    let (w, h) = (val(doc, sheet, names::WIDTH), val(doc, sheet, names::HEIGHT));
    let mut out = Vec::new();
    for sk in doc.geometry(sheet) {
        let hidden = doc.resolve(sheet, &CellAddr::section(sk, names::NO_SHOW)).num().is_some_and(|v| v != 0.0);
        if hidden {
            continue;
        }
        for v in vrows(doc, sheet, sk, w, h) {
            if !v.closing {
                out.push(Vertex { section: sk.index, row: v.ix, at: xf.apply(v.p) });
            }
        }
    }
    out
}

// the values X and Y of a row get for the local point q
fn point_sets(doc: &Document, sheet: SheetRef, sk: SecKey, ix: u32, rel: bool, q: [f64; 2], w: f64, h: f64) -> Vec<(CellAddr, Put)> {
    let ax = CellAddr::row(sk, RowKey::Index(ix), names::X);
    let ay = CellAddr::row(sk, RowKey::Index(ix), names::Y);
    if rel {
        let fx = if w.abs() > EPS { q[0] / w } else { 0.0 };
        let fy = if h.abs() > EPS { q[1] / h } else { 0.0 };
        alloc::vec![(ax, Put::Num(fx)), (ay, Put::Num(fy))]
    } else {
        alloc::vec![(ax, keep_form(doc, sheet, &ax, q[0])), (ay, keep_form(doc, sheet, &ay, q[1]))]
    }
}

// whether a row's X or Y is computed from other cells (Geometry1.X1 in a closing row ...):
// such a row follows by itself
fn follows(doc: &Document, sheet: SheetRef, sk: SecKey, ix: u32) -> bool {
    [names::X, names::Y].iter().any(|&c| match doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(ix), c)).formula() {
        Formula::Expr(f) => text::proportional(doc.str(f)).is_none(),
        _ => false,
    })
}

fn vertex_ctx(tx: &Transaction, eng: &mut Engine, shape: ShapeRef, section: u32) -> Result<(Xform, f64, f64, Vec<VRow>), EditError> {
    let doc = tx.doc();
    doc.shape(shape).ok_or(EditError::NoShape)?;
    if locked(eng, doc, shape, "LockVtxEdit") {
        return Err(EditError::Invalid);
    }
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    if !doc.has_section(sheet, sk) {
        return Err(EditError::NoSection);
    }
    let inv = doc.page_xform(shape).inverse().ok_or(EditError::Invalid)?;
    let (w, h) = (val(doc, sheet, names::WIDTH), val(doc, sheet, names::HEIGHT));
    Ok((inv, w, h, vrows(doc, sheet, sk, w, h)))
}

/// move a vertex to the page point `at`; the start of a closed sub-path takes its closing row
/// along
pub fn set_vertex(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, section: u32, row: u32, at: [f64; 2]) -> Result<(), EditError> {
    let (inv, w, h, rows) = vertex_ctx(tx, eng, shape, section)?;
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let v = *rows.iter().find(|v| v.ix == row).ok_or(EditError::NoRow)?;
    let q = inv.apply(at);
    let sets = {
        let doc = tx.doc();
        let mut sets = point_sets(doc, sheet, sk, row, v.rel, q, w, h);
        if v.start {
            if let Some(c) = rows.iter().find(|c| c.closing && c.sub == v.ix) {
                if !follows(doc, sheet, sk, c.ix) {
                    sets.extend(point_sets(doc, sheet, sk, c.ix, c.rel, q, w, h));
                }
            }
        }
        sets
    };
    let mut wr = Writer::new();
    wr.put_all(tx, eng, sheet, sets)?;
    wr.finish(tx, eng)?;
    // (JD21, SP-05) the box follows the geometry
    crate::shapeops::fit_frame(tx, eng, shape)?;
    Ok(())
}

/// add a vertex at the page point `at` after row `after` (on the segment to the next row):
/// a LineTo (RelLineTo among relative rows) at IX after + 1 -> its IX
pub fn insert_vertex(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, section: u32, after: u32, at: [f64; 2]) -> Result<u32, EditError> {
    let (inv, w, h, rows) = vertex_ctx(tx, eng, shape, section)?;
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let i = rows.iter().position(|v| v.ix == after).ok_or(EditError::NoRow)?;
    let rel = rows.get(i + 1).filter(|n| !n.start).map_or(rows[i].rel, |n| n.rel);
    let kind = if rel { "RelLineTo" } else { "LineTo" };
    let RowKey::Index(ix) = tx.insert_row(sheet, sk, RowKey::Index(after + 1), Some(kind))? else { return Err(EditError::Invalid) };
    let q = inv.apply(at);
    let sets = point_sets(tx.doc(), sheet, sk, ix, rel, q, w, h);
    for (a, p) in sets {
        let v = match p {
            Put::Num(x) => x,
            Put::Formula(_) => continue,
        };
        tx.set_constant(sheet, &a, Value::Num(v), None)?;
    }
    eng.recalc(tx, &[sheet])?;
    crate::shapeops::fit_frame(tx, eng, shape)?;
    Ok(ix)
}

/// take a vertex out; at the start of a sub-path the next vertex becomes the start (and the
/// closing row of a closed one goes there); a sub-path keeps at least two vertices (Invalid)
pub fn delete_vertex(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, section: u32, row: u32) -> Result<(), EditError> {
    let (_, w, h, rows) = vertex_ctx(tx, eng, shape, section)?;
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let v = *rows.iter().find(|v| v.ix == row && !v.closing).ok_or(EditError::NoRow)?;
    let count = rows.iter().filter(|x| x.sub == v.sub && !x.closing).count();
    if count <= 2 {
        return Err(EditError::Invalid);
    }
    if v.start {
        let i = rows.iter().position(|x| x.ix == row).ok_or(EditError::NoRow)?;
        let next = rows[i + 1];
        tx.set_row_kind(sheet, sk, RowKey::Index(next.ix), if next.rel { "RelMoveTo" } else { "MoveTo" })?;
        // the closing row goes to the new start (a value: the old start it named is gone)
        if let Some(c) = rows.iter().find(|c| c.closing && c.sub == v.ix) {
            let p = if c.rel { [if w.abs() > EPS { next.p[0] / w } else { 0.0 }, if h.abs() > EPS { next.p[1] / h } else { 0.0 }] } else { next.p };
            for (cell, x) in [(names::X, p[0]), (names::Y, p[1])] {
                let a = CellAddr::row(sk, RowKey::Index(c.ix), cell);
                let unit = tx.doc().resolve(sheet, &a).unit();
                tx.force_constant(sheet, &a, Value::Num(x), unit)?;
            }
        }
    }
    tx.remove_row(sheet, sk, RowKey::Index(row))?;
    eng.recalc(tx, &[sheet])?;
    crate::shapeops::fit_frame(tx, eng, shape)?;
    Ok(())
}
