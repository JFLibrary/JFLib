// ops::arcs -- (JD21, SP-05) the handles of arcs in vertex editing: an arc's control point and
// the eccentricity handles of an elliptical arc
//
//   arc_handles(doc, shape) -> Vec<ArcHandle>    per ArcTo, EllipticalArcTo and
//                                                RelEllipticalArcTo row of the shown sections:
//                                                the control point on the page and (absolute
//                                                EllipticalArcTo) the two eccentricity handles
//   set_arc_point(tx, eng, shape, section, row, at)     the control point dragged to `at`
//   set_eccentricity(tx, eng, shape, section, row, at)  an eccentricity handle dragged to `at`
//
// Visio's vertex editing (pencil tool) shows a control point in the middle of an arc segment;
// dragging it bends the arc.  An ArcTo keeps its bow A (the distance of the arc's middle from
// the middle of its chord, positive right of the chord: geometry.rs), the control point is the
// arc's middle; an EllipticalArcTo goes through its control point A, B, which is where the
// handle shows (relative rows: fractions of Width and Height).  A clicked control point of an
// elliptical arc shows two eccentricity handles on a line through it (Visio: "drag the
// eccentricity handles to change the angle and the eccentricity"): the line runs along the
// major axis (C, its angle) and each handle lies D x L from the control point, L a quarter of the
// chord (assumed: Visio's own spacing is not documented).  Dragging one sets C to the
// direction and D to the distance over L (D between 0.01 and 100).  The written cells keep the
// form of their formulas where they are proportional (ops::keep_form); the box follows the
// geometry afterwards (shapeops::fit_frame), as after every vertex edit.

use alloc::vec::Vec;

use super::points::{geo_key, vertex_ctx};
use super::{keep_form, Put, Writer, EPS};
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::{atan2, sincos, sqrt};
use crate::names;
use crate::sheet::{CellAddr, RowKey};

/// the handles of one arc row
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ArcHandle {
    /// the Geometry section's index (Geometry1 is 0)
    pub section: u32,
    /// the row's IX
    pub row: u32,
    /// the control point on the page (an ArcTo: the arc's middle; an EllipticalArcTo: A, B)
    pub at: [f64; 2],
    /// the two eccentricity handles on the page (an absolute EllipticalArcTo only)
    pub ecc: Option<[[f64; 2]; 2]>,
}

// an arc row as the handles see it (local coordinates)
struct ArcRow {
    kind: Kind,
    from: [f64; 2],
    to: [f64; 2],
    // ArcTo: the bow; elliptical: A, B (relative rows: already times Width and Height)
    a: f64,
    ctl: [f64; 2],
    // the angle (C) and the ratio (D) of an elliptical arc
    angle: f64,
    ratio: f64,
}

#[derive(Copy, Clone, PartialEq)]
enum Kind {
    Arc,
    Elliptical,
    RelElliptical,
}

// the arc row `row` of section `sec`: its kind, where it starts and ends, its cells
fn arc_row(doc: &Document, shape: ShapeRef, section: u32, row: u32, w: f64, h: f64) -> Option<ArcRow> {
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let rows = super::points::vrows(doc, sheet, sk, w, h);
    let i = rows.iter().position(|v| v.ix == row)?;
    if i == 0 || rows[i].start {
        return None;
    }
    let eff = doc.rows(sheet, sk).into_iter().find(|r| r.key == RowKey::Index(row))?;
    let kind = match eff.kind.map_or("", |k| doc.str(k)) {
        "ArcTo" => Kind::Arc,
        "EllipticalArcTo" => Kind::Elliptical,
        "RelEllipticalArcTo" => Kind::RelElliptical,
        _ => return None,
    };
    let num = |c| doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(row), c)).num();
    let (a, b) = (num(names::A).unwrap_or(0.0), num(names::B).unwrap_or(0.0));
    let ctl = if kind == Kind::RelElliptical { [a * w, b * h] } else { [a, b] };
    Some(ArcRow { kind, from: rows[i - 1].p, to: rows[i].p, a, ctl, angle: num(names::C).unwrap_or(0.0), ratio: num(names::D).unwrap_or(1.0) })
}

// the middle of an ArcTo (local): the middle of the chord moved `bow` to its right
fn arc_middle(from: [f64; 2], to: [f64; 2], bow: f64) -> [f64; 2] {
    let d = [to[0] - from[0], to[1] - from[1]];
    let l = sqrt(d[0] * d[0] + d[1] * d[1]);
    let m = [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5];
    if l <= EPS {
        return m;
    }
    [m[0] + d[1] / l * bow, m[1] - d[0] / l * bow]
}

// the distance of the eccentricity handles from the control point per unit of D
fn ecc_unit(r: &ArcRow, w: f64, h: f64) -> f64 {
    let d = [r.to[0] - r.from[0], r.to[1] - r.from[1]];
    let l = sqrt(d[0] * d[0] + d[1] * d[1]) * 0.25;
    if l > EPS { l } else { (w.abs().max(h.abs()) * 0.125).max(EPS) }
}

// the eccentricity handles of an elliptical arc (local)
fn ecc_points(r: &ArcRow, w: f64, h: f64) -> [[f64; 2]; 2] {
    let (s, c) = sincos(r.angle);
    let k = ecc_unit(r, w, h) * r.ratio;
    [[r.ctl[0] + c * k, r.ctl[1] + s * k], [r.ctl[0] - c * k, r.ctl[1] - s * k]]
}

/// the arc handles of a shape, section by section (hidden sections left out)
pub fn arc_handles(doc: &Document, shape: ShapeRef) -> Vec<ArcHandle> {
    let sheet = SheetRef::Shape(shape);
    let mut out = Vec::new();
    if doc.shape(shape).is_none() {
        return out;
    }
    let xf = doc.page_xform(shape);
    let (w, h) = (super::val(doc, sheet, names::WIDTH), super::val(doc, sheet, names::HEIGHT));
    for sk in doc.geometry(sheet) {
        if doc.resolve(sheet, &CellAddr::section(sk, names::NO_SHOW)).num().is_some_and(|v| v != 0.0) {
            continue;
        }
        for v in super::points::vrows(doc, sheet, sk, w, h) {
            let Some(r) = arc_row(doc, shape, sk.index, v.ix, w, h) else { continue };
            let (at, ecc) = match r.kind {
                Kind::Arc => (arc_middle(r.from, r.to, r.a), None),
                Kind::Elliptical => {
                    let e = ecc_points(&r, w, h);
                    (r.ctl, Some([xf.apply(e[0]), xf.apply(e[1])]))
                }
                Kind::RelElliptical => (r.ctl, None),
            };
            out.push(ArcHandle { section: sk.index, row: v.ix, at: xf.apply(at), ecc });
        }
    }
    out
}

/// drag the control point of arc row `row` to the page point `at`: an ArcTo gets the bow that
/// puts its middle at the point's projection onto the chord's normal, an elliptical arc A and B
/// there.  NoRow: not an arc row; Invalid: LockVtxEdit
pub fn set_arc_point(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, section: u32, row: u32, at: [f64; 2]) -> Result<(), EditError> {
    let (inv, w, h, _) = vertex_ctx(tx, eng, shape, section)?;
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let q = inv.apply(at);
    let sets = {
        let doc = tx.doc();
        let r = arc_row(doc, shape, section, row, w, h).ok_or(EditError::NoRow)?;
        let (aa, ab) = (CellAddr::row(sk, RowKey::Index(row), names::A), CellAddr::row(sk, RowKey::Index(row), names::B));
        match r.kind {
            Kind::Arc => {
                let d = [r.to[0] - r.from[0], r.to[1] - r.from[1]];
                let l = sqrt(d[0] * d[0] + d[1] * d[1]);
                if l <= EPS {
                    return Ok(());
                }
                let m = [(r.from[0] + r.to[0]) * 0.5, (r.from[1] + r.to[1]) * 0.5];
                let bow = (q[0] - m[0]) * d[1] / l - (q[1] - m[1]) * d[0] / l;
                alloc::vec![(aa, keep_form(doc, sheet, &aa, bow))]
            }
            Kind::Elliptical => alloc::vec![(aa, keep_form(doc, sheet, &aa, q[0])), (ab, keep_form(doc, sheet, &ab, q[1]))],
            Kind::RelElliptical => {
                let fx = if w.abs() > EPS { q[0] / w } else { 0.0 };
                let fy = if h.abs() > EPS { q[1] / h } else { 0.0 };
                alloc::vec![(aa, Put::Num(fx)), (ab, Put::Num(fy))]
            }
        }
    };
    let mut wr = Writer::new();
    wr.put_all(tx, eng, sheet, sets)?;
    wr.finish(tx, eng)?;
    crate::shapeops::fit_frame(tx, eng, shape)?;
    Ok(())
}

/// drag an eccentricity handle of the elliptical arc row `row` to the page point `at`: the
/// major axis (C) turns towards it, the ratio (D) is its distance from the control point over
/// the handles' unit (see the module comment).  NoRow: not an absolute EllipticalArcTo
pub fn set_eccentricity(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, section: u32, row: u32, at: [f64; 2]) -> Result<(), EditError> {
    let (inv, w, h, _) = vertex_ctx(tx, eng, shape, section)?;
    let sheet = SheetRef::Shape(shape);
    let sk = geo_key(section);
    let q = inv.apply(at);
    let sets = {
        let doc = tx.doc();
        let r = arc_row(doc, shape, section, row, w, h).filter(|r| r.kind == Kind::Elliptical).ok_or(EditError::NoRow)?;
        let v = [q[0] - r.ctl[0], q[1] - r.ctl[1]];
        let len = sqrt(v[0] * v[0] + v[1] * v[1]);
        if len <= EPS {
            return Ok(());
        }
        // either handle: the axis is a line, so the angle is taken modulo a half turn the way
        // that stays nearest to the old one
        let mut angle = atan2(v[1], v[0]);
        let pi = core::f64::consts::PI;
        while angle - r.angle > pi * 0.5 {
            angle -= pi;
        }
        while r.angle - angle > pi * 0.5 {
            angle += pi;
        }
        let ratio = (len / ecc_unit(&r, w, h)).clamp(0.01, 100.0);
        alloc::vec![(CellAddr::row(sk, RowKey::Index(row), names::C), Put::Num(angle)), (CellAddr::row(sk, RowKey::Index(row), names::D), Put::Num(ratio))]
    };
    let mut wr = Writer::new();
    wr.put_all(tx, eng, sheet, sets)?;
    wr.finish(tx, eng)?;
    crate::shapeops::fit_frame(tx, eng, shape)?;
    Ok(())
}

#[cfg(test)]
#[path = "arcs_tests.rs"]
mod tests;
