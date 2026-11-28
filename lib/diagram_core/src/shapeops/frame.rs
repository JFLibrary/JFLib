// shapeops::frame -- (JD21) a frame that follows its geometry (SP-05, "Rahmen folgt der Geometrie")
// and Ausrichtungsfeld aktualisieren (SP-16)
//
//   geometry_bounds(doc, shape)          the box of a shape's geometry in its local coordinates
//   fit_frame(tx, eng, shape)            Width, Height, Pin to that box (after vertices changed)
//   update_alignment_box(tx, eng, shape) a group's box around its members; a shape's: fit_frame
//
// Visio recomputes a shape's box when a vertex is edited or a row type changes, unless LockCalcWH
// is set (belegt: [MS-VSDX]/ShapeSheet reference of LockCalcWH, "Alignment Boxes" of the Visio
// 2002 developer reference); Ausrichtungsfeld aktualisieren does it on request, for a group around
// its members, also only without LockCalcWH (belegt: VBA Selection.UpdateAlignmentBox).  How it
// writes the cells is not documented; here (angenommen): the shape stays where it is on the page,
// Angle and the flips stay, Width and Height become the box's size (a proportional formula keeps
// its form), LocPin follows its own formula (mostly Width*0.5), the pin goes where the old frame
// had the new local pin, and every geometry cell moves into the new box: absolute rows keep
// "Width*k" with a new k (constants stay constants), relative rows get new fractions (the
// ellipse of RelEllipticalArcTo a new C and D), NURBS(...) and POLYLINE(...) values their
// points, a D of "Width/Height*k" a new k; a cell that names another geometry cell (a closing
// row's Geometry1.X1) follows by itself.  Connection points, control points and the text block
// keep their formulas (they are relative to the box) -- also an assumption.  A group's members get
// their pins (and 1-D members their free ends) in the new box, "Sheet.<group>!Width*k" with a new
// k, sizes that are proportional to the group keep their value.  Bézier pieces count with their
// extremes (not their control points), arcs with theirs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{grow, shape_paths, Piece};
use crate::cell::Formula;
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::{cubic_at, parse_args, sqrt, Conic, Seg};
use crate::names;
use crate::ops::{self, Put, Writer};
use crate::shape::ShapeKind;
use crate::sheet::{CellAddr, RowKey};
use crate::strings::Sym;

// the box of a cubic Bézier piece: its ends and the extremes between
fn cubic_bounds(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2]) -> [f64; 4] {
    let mut b = [p0[0].min(p3[0]), p0[1].min(p3[1]), p0[0].max(p3[0]), p0[1].max(p3[1])];
    for k in 0..2 {
        // B'(t) / 3 = a t^2 + b t + c
        let a = -p0[k] + 3.0 * p1[k] - 3.0 * p2[k] + p3[k];
        let bb = 2.0 * (p0[k] - 2.0 * p1[k] + p2[k]);
        let c = p1[k] - p0[k];
        let mut ts: Vec<f64> = Vec::new();
        if a.abs() < 1e-12 * (bb.abs() + c.abs() + 1e-300) {
            if bb != 0.0 {
                ts.push(-c / bb);
            }
        } else {
            let disc = bb * bb - 4.0 * a * c;
            if disc >= 0.0 {
                let r = sqrt(disc);
                ts.push((-bb + r) / (2.0 * a));
                ts.push((-bb - r) / (2.0 * a));
            }
        }
        for t in ts {
            if t > 0.0 && t < 1.0 {
                let q = cubic_at(p0, p1, p2, p3, t);
                b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
            }
        }
    }
    b
}

/// the box of a shape's geometry in its local coordinates (x0, y0, x1, y1): lines by their ends,
/// arcs and Bézier pieces with their extremes; hidden sections (NoShow) and InfiniteLine rows
/// left out.  None: no geometry
pub fn geometry_bounds(doc: &Document, shape: ShapeRef) -> Option<[f64; 4]> {
    let pt = |p: [f64; 2]| [p[0], p[1], p[0], p[1]];
    let mut b = None;
    for sec in shape_paths(doc, shape) {
        if sec.no_show {
            continue;
        }
        for s in &sec.subs {
            if s.pieces.is_empty() {
                continue;
            }
            grow(&mut b, pt(s.start));
            let mut at = s.start;
            for p in &s.pieces {
                match p {
                    Piece::Line(e) => grow(&mut b, pt(*e)),
                    Piece::Arc(c, e) => {
                        grow(&mut b, c.bounds());
                        grow(&mut b, pt(*e));
                    }
                    Piece::Free(segs) => {
                        let mut prev = at;
                        for sg in segs {
                            match *sg {
                                Seg::Line(e) => grow(&mut b, pt(e)),
                                Seg::Cubic(c1, c2, e) => grow(&mut b, cubic_bounds(prev, c1, c2, e)),
                            }
                            prev = sg.end();
                        }
                    }
                }
                at = p.end();
            }
        }
    }
    b
}

fn calc_locked(eng: &mut Engine, doc: &Document, shape: ShapeRef) -> bool {
    ops::locks(eng, doc, shape).has(ops::Locks::CALC_WH)
}

/// The frame follows the geometry (SP-05): Width, Height, Pin and LocPin to the box of the
/// shape's geometry (geometry_bounds), the geometry rows into the new box, the shape staying where
/// it is on the page (see the module comment).  Only for a 2-D shape that is no group, and not
/// with LockCalcWH or a guarded Width or Height -> whether it changed
pub fn fit_frame(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef) -> Result<bool, EditError> {
    let b = {
        let doc = tx.doc();
        let sh = doc.shape(shape).ok_or(EditError::NoShape)?;
        if sh.header().kind == ShapeKind::Group || ops::one_d(doc, shape) || calc_locked(eng, doc, shape) {
            return Ok(false);
        }
        match geometry_bounds(doc, shape) {
            Some(b) => b,
            None => return Ok(false),
        }
    };
    reframe(tx, eng, shape, b)
}

/// Ausrichtungsfeld aktualisieren (SP-16): a group's box around its members (2-D ones by their
/// frames, 1-D ones by their ends) and its own geometry, the members staying where they are on the
/// page; a shape that is no group gets the box of its geometry (fit_frame).  Not with LockCalcWH
/// -> whether it changed
pub fn update_alignment_box(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef) -> Result<bool, EditError> {
    let b = {
        let doc = tx.doc();
        let sh = doc.shape(shape).ok_or(EditError::NoShape)?;
        if sh.header().kind != ShapeKind::Group {
            return fit_frame(tx, eng, shape);
        }
        if calc_locked(eng, doc, shape) {
            return Ok(false);
        }
        let mut b = geometry_bounds(doc, shape);
        for k in sh.children() {
            let pts: Vec<[f64; 2]> = if ops::one_d(doc, k) {
                let (p, q) = ops::ends(doc, k);
                alloc::vec![p, q]
            } else {
                let s = SheetRef::Shape(k);
                let f = ops::Frame { xf: doc.local_xform(s), w: ops::val(doc, s, names::WIDTH), h: ops::val(doc, s, names::HEIGHT) };
                f.corners().to_vec()
            };
            for p in pts {
                grow(&mut b, [p[0], p[1], p[0], p[1]]);
            }
        }
        match b {
            Some(b) => b,
            None => return Ok(false),
        }
    };
    reframe(tx, eng, shape, b)
}

// what a geometry cell gets: a value in the new box (the proportional form kept), or a value or
// formula as it is
enum RowPut {
    Keep(f64),
    Num(f64),
    Formula(String),
}

fn fnum(x: f64) -> String {
    ops::text::num(x)
}

// the geometry cells of `sheet` in a box moved to `o` and sized nw x nh (old: w x h)
fn row_plan(doc: &Document, sheet: SheetRef, w: f64, h: f64, o: [f64; 2], nw: f64, nh: f64) -> Vec<(CellAddr, RowPut)> {
    let (x, y, a, b, c, d, e) = (names::X, names::Y, names::A, names::B, names::C, names::D, names::E);
    let mut out = Vec::new();
    let fx = |f: f64| if nw.abs() > 1e-12 { (f * w - o[0]) / nw } else { f };
    let fy = |f: f64| if nh.abs() > 1e-12 { (f * h - o[1]) / nh } else { f };
    for sk in doc.geometry(sheet) {
        for r in doc.rows(sheet, sk) {
            if !matches!(r.key, RowKey::Index(_)) {
                continue;
            }
            let kind = r.kind.map_or("", |k| doc.str(k));
            let addr = |cell: Sym| CellAddr::row(sk, r.key, cell);
            let res = |cell: Sym| doc.resolve(sheet, &addr(cell));
            // a cell that names another geometry cell follows by itself
            let follows = |cell: Sym| match res(cell).formula() {
                Formula::Expr(f) => {
                    let t = doc.str(f);
                    ops::text::proportional(t).is_none() && t.contains("Geometry")
                }
                _ => false,
            };
            let (ax, ay): (&[Sym], &[Sym]) = match kind {
                "MoveTo" | "LineTo" | "ArcTo" | "SplineStart" | "SplineKnot" | "NURBSTo" | "PolylineTo" => (&[x], &[y]),
                "EllipticalArcTo" | "InfiniteLine" => (&[x, a], &[y, b]),
                "Ellipse" => (&[x, a, c], &[y, b, d]),
                _ => (&[], &[]),
            };
            for (cells, k) in [(ax, 0usize), (ay, 1usize)] {
                for &cell in cells {
                    let r = res(cell);
                    if let (Some(v), false) = (r.num(), follows(cell)) {
                        out.push((addr(cell), RowPut::Keep(v - o[k])));
                    }
                }
            }
            let (rx, ry): (&[Sym], &[Sym]) = match kind {
                "RelMoveTo" | "RelLineTo" => (&[x], &[y]),
                "RelCubBezTo" => (&[x, a, c], &[y, b, d]),
                "RelQuadBezTo" | "RelEllipticalArcTo" => (&[x, a], &[y, b]),
                _ => (&[], &[]),
            };
            for &cell in rx {
                if let (Some(v), false) = (res(cell).num(), follows(cell)) {
                    out.push((addr(cell), RowPut::Num(fx(v))));
                }
            }
            for &cell in ry {
                if let (Some(v), false) = (res(cell).num(), follows(cell)) {
                    out.push((addr(cell), RowPut::Num(fy(v))));
                }
            }
            match kind {
                // the ellipse of a relative arc in the new relative space
                "RelEllipticalArcTo" if nw.abs() > 1e-12 && nh.abs() > 1e-12 => {
                    let (ang, ratio) = (res(c).num().unwrap_or(0.0), res(d).num().unwrap_or(1.0));
                    let (s, co) = crate::geometry::sincos(ang);
                    let (sx, sy) = (w / nw, h / nh);
                    let k = Conic { c: [0.0, 0.0], u: [ratio * co * sx, ratio * s * sy], v: [-s * sx, co * sy], t0: 0.0, sweep: 0.0 };
                    let (phi, s1, s2) = k.axes();
                    if s2 > 0.0 && s1.is_finite() {
                        out.push((addr(c), RowPut::Num(phi)));
                        out.push((addr(d), RowPut::Num(s1 / s2)));
                    }
                }
                // D proportional to the box: a new factor for the same value
                "EllipticalArcTo" => {
                    if let (Formula::Expr(f), Some(v)) = (res(d).formula(), res(d).num()) {
                        let t = doc.str(f).replace(' ', "");
                        if nw.abs() > 1e-12 && nh.abs() > 1e-12 {
                            if t.starts_with("Width/Height*") {
                                out.push((addr(d), RowPut::Formula(format!("Width/Height*{}", fnum(v * nh / nw)))));
                            } else if t.starts_with("Height/Width*") {
                                out.push((addr(d), RowPut::Formula(format!("Height/Width*{}", fnum(v * nw / nh)))));
                            }
                        }
                    }
                }
                // the points inside NURBS(...) and POLYLINE(...): fractions (type 0) or local inches
                "NURBSTo" | "PolylineTo" => {
                    let (cell, name, first, step) = if kind == "NURBSTo" { (e, "NURBS", 4usize, 4usize) } else { (a, "POLYLINE", 2usize, 2usize) };
                    let text = match res(cell).formula() {
                        Formula::Expr(f) => Some(String::from(doc.str(f))),
                        _ => match res(cell).value() {
                            crate::cell::Value::Text(t) => Some(String::from(doc.str(t))),
                            _ => None,
                        },
                    };
                    if let Some(mut v) = text.as_deref().and_then(|t| parse_args(t, name)) {
                        let ti = if kind == "NURBSTo" { 2 } else { 0 };
                        if v.len() >= first {
                            let rel = (v[ti] == 0.0, v[ti + 1] == 0.0);
                            let mut i = first;
                            while i + 1 < v.len() {
                                v[i] = if rel.0 { fx(v[i]) } else { v[i] - o[0] };
                                v[i + 1] = if rel.1 { fy(v[i + 1]) } else { v[i + 1] - o[1] };
                                i += step;
                            }
                            let parts: Vec<String> = v.iter().map(|&x| fnum(x)).collect();
                            out.push((addr(cell), RowPut::Formula(format!("{name}({})", parts.join(",")))));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

// the members of a group in a box moved to `o`: their pins, the free ends of 1-D ones, sizes that
// are proportional to the group (their values stay)
fn kid_plan(doc: &Document, group: ShapeRef, o: [f64; 2]) -> Vec<(ShapeRef, Vec<(CellAddr, f64)>)> {
    let Some(g) = doc.shape(group) else { return Vec::new() };
    let mut out = Vec::new();
    for k in g.children() {
        let s = SheetRef::Shape(k);
        let mut sets = Vec::new();
        if ops::one_d(doc, k) {
            let (gb, ge) = ops::glued_ends(doc, k);
            let (bp, ep) = ops::ends(doc, k);
            for (glued, cx, cy, p) in [(gb, names::BEGIN_X, names::BEGIN_Y, bp), (ge, names::END_X, names::END_Y, ep)] {
                if !glued {
                    sets.push((ops::single(cx), p[0] - o[0]));
                    sets.push((ops::single(cy), p[1] - o[1]));
                }
            }
        } else {
            sets.push((ops::single(names::PIN_X), ops::val(doc, s, names::PIN_X) - o[0]));
            sets.push((ops::single(names::PIN_Y), ops::val(doc, s, names::PIN_Y) - o[1]));
            for cell in [names::WIDTH, names::HEIGHT] {
                if let Formula::Expr(f) = doc.resolve(s, &ops::single(cell)).formula() {
                    if ops::text::proportional(doc.str(f)).is_some_and(|p| p.0.is_some()) {
                        sets.push((ops::single(cell), ops::val(doc, s, cell)));
                    }
                }
            }
        }
        out.push((k, sets));
    }
    out
}

// give a shape the box `nb` (x0, y0, x1, y1 in its local coordinates) without moving anything on
// the page -> whether it changed
fn reframe(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, nb: [f64; 4]) -> Result<bool, EditError> {
    let sheet = SheetRef::Shape(shape);
    let (nw, nh) = (nb[2] - nb[0], nb[3] - nb[1]);
    let o = [nb[0], nb[1]];
    let (old_xf, rows, kids) = {
        let doc = tx.doc();
        let (w, h) = (ops::val(doc, sheet, names::WIDTH), ops::val(doc, sheet, names::HEIGHT));
        let tiny = 1e-9 * (1.0 + w.abs().max(h.abs()));
        if o[0].abs() <= tiny && o[1].abs() <= tiny && (nw - w).abs() <= tiny && (nh - h).abs() <= tiny {
            return Ok(false);
        }
        if !(nw.is_finite() && nh.is_finite()) || (nw <= tiny && nh <= tiny) {
            return Ok(false);
        }
        if doc.is_guarded(sheet, &ops::single(names::WIDTH)) || doc.is_guarded(sheet, &ops::single(names::HEIGHT)) {
            return Ok(false);
        }
        (doc.local_xform(sheet), row_plan(doc, sheet, w, h, o, nw, nh), kid_plan(doc, shape, o))
    };
    let mut wr = Writer::new();
    // the sizes, then the local pin follows its formula
    let sets = {
        let doc = tx.doc();
        let (aw, ah) = (ops::single(names::WIDTH), ops::single(names::HEIGHT));
        alloc::vec![(aw, ops::keep_form(doc, sheet, &aw, nw)), (ah, ops::keep_form(doc, sheet, &ah, nh))]
    };
    wr.put_all(tx, eng, sheet, sets)?;
    wr.finish(tx, eng)?;
    // the pin where the old frame had the new local pin; the rows in the new box
    let sets = {
        let doc = tx.doc();
        let (lx, ly) = (ops::val(doc, sheet, names::LOC_PIN_X), ops::val(doc, sheet, names::LOC_PIN_Y));
        let at = old_xf.apply([o[0] + lx, o[1] + ly]);
        let (px, py) = (ops::single(names::PIN_X), ops::single(names::PIN_Y));
        let mut sets = alloc::vec![(px, ops::keep_form(doc, sheet, &px, at[0])), (py, ops::keep_form(doc, sheet, &py, at[1]))];
        for (addr, p) in rows {
            let put = match p {
                RowPut::Keep(v) => ops::keep_form(doc, sheet, &addr, v),
                RowPut::Num(v) => Put::Num(v),
                RowPut::Formula(f) => Put::Formula(f),
            };
            sets.push((addr, put));
        }
        sets
    };
    wr.put_all(tx, eng, sheet, sets)?;
    // the members
    let mut sheets = alloc::vec![sheet];
    for (k, plan) in kids {
        let ks = SheetRef::Shape(k);
        let sets: Vec<(CellAddr, Put)> = {
            let doc = tx.doc();
            plan.into_iter().map(|(a, v)| {
                let p = ops::keep_form(doc, ks, &a, v);
                (a, p)
            }).collect()
        };
        wr.put_all(tx, eng, ks, sets)?;
        sheets.push(ks);
    }
    wr.finish(tx, eng)?;
    eng.recalc(tx, &sheets)?;
    Ok(true)
}
