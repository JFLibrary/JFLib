// shapeops -- (JD21) Visio's shape operations (SP-08: Vereinigen, Kombinieren, Fragmentieren,
// Schneiden, Subtrahieren), a frame that follows its geometry (SP-05) and Ausrichtungsfeld
// aktualisieren (SP-16)
//
//   Operation::{Union, Combine, Fragment, Intersect, Subtract}
//   available(doc, shapes, op) -> bool       whether the operation can run on the selection
//   operate(tx, eng, shapes, op) -> Vec<ShapeRef>   the operation; shapes[0] is the primary
//                                            shape (the first one selected): the result(s), empty
//                                            when nothing is left (the inputs are gone then too)
//   fit_frame(tx, eng, shape) -> bool        Width, Height, Pin and the rows to the geometry's box,
//                                            the shape stays where it is on the page (frame.rs)
//   update_alignment_box(tx, eng, shape) -> bool    (SP-16) a group's box around its members, a
//                                            shape's around its geometry (frame.rs)
//   geometry_bounds(doc, shape) -> Option<[f64; 4]>  the box of a shape's geometry in its own
//                                            coordinates: arcs by their extremes (frame.rs)
//
// How an operation works:
// * Each input shape becomes an operand in page coordinates: its Geometry sections (hidden ones,
//   NoShow, and InfiniteLine rows left out) through the shape's transform (turns, flips, groups
//   around it).  Lines stay exact; arcs (ArcTo, EllipticalArcTo, RelEllipticalArcTo, Ellipse) are
//   flattened into short chords that remember their arc (geometry::Conic); other curves (NURBS,
//   splines, Bézier rows) into chords that remember their row.  Closed sub-paths are the area,
//   filled even-odd as Visio fills a shape's sections (overlapping sections make holes); open
//   sub-paths are lines.  A group is the union of its members (and of its own geometry).
// * The Boolean operations run in geom::region (exact decisions on a grid of 2^30 steps across
//   the inputs; jflib-geom, docs/GEOM.md "Ebene Regionen"); every edge of the result names the
//   input edge it lies on, so the result is rebuilt row by row: a run of edges on one arc is
//   EllipticalArcTo again (pieces of at most half a turn, the ends projected onto the arc), a
//   whole ellipse an Ellipse row, a run on one line one LineTo, other curves LineTo polylines.
//   Kombinieren needs no Boolean operation: the sections are taken over as they are, each one a
//   section of the result.
// * The result is a new shape without master: Angle 0, its box around the geometry (arcs by their
//   extremes), PinX/PinY in its middle, LocPin "Width*0.5"/"Height*0.5", every geometry cell as
//   "Width*k" / "Height*k" (the control points of arcs too, and D of an upright arc as
//   "Width/Height*k") so that the result stretches with its box; one Geometry section per contour
//   (Visio fills them even-odd: holes stay holes).  It goes into the primary shape's parent on top
//   of its siblings.  The inputs are deleted (glue to them let go, container memberships
//   forgotten); with an empty result nothing new comes.
//
// Visio's rules (the sources: Visio 5 User Guide ch. 24 pp. 210-215, the VBA reference of
// Selection.Union / Combine / Fragment / Intersect / Subtract / UpdateAlignmentBox, "Merged
// Shapes" and "Alignment Boxes" of the Visio 2002 developer reference, Visio 2003 Inside Out ch. 22;
// details in the JD21 notes):
// * belegt: format (line, fill) and text come from the primary shape (the first one selected);
//   Fragmentieren gives every piece the primary's format and no text; the result is the topmost
//   shape of its parent; the inputs are deleted; the result has a ShapeSheet of its own (formulas
//   of the inputs are gone); Vereinigen leaves open paths out; Schneiden keeps what all inputs
//   cover, of disjoint shapes nothing (and the inputs are gone); Subtrahieren takes the area of
//   the others away from the primary shape, the others are deleted even where they do not
//   overlap; Kombinieren keeps each input's sections, overlaps become holes; Fragmentieren makes a
//   shape of every region, and lines (1-D shapes, open paths) cut regions.
// * angenommen: the result is a plain shape without master (the primary's format is copied with
//   its inherited cells, styles stay referenced); shape data, user cells, hyperlinks and the
//   protection cells of the inputs are not taken over; LayerMember and the text block cells are;
//   a closed path of a NoFill section is area too; a shape without area does not take part in
//   Vereinigen, Schneiden and Subtrahieren and is left alone; a group takes part as the union of
//   its members; after an operation the result(s) are selected (the app does that); Vereinigen and
//   Fragmentieren also run on one shape (its own sections merged or cut), Kombinieren, Schneiden
//   and Subtrahieren need two; locks (LockDelete ...) do not protect the inputs; the shapes are
//   given in selection order, the primary first, and Subtrahieren takes the others away together.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use geom::region::{self, FillRule, Operand};

use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::{ceil, cubic_at, rem, sqrt, Conic, Seg, Xform};
use crate::names::{self, Category};
use crate::ops;
use crate::shape::{ShapeData, ShapeKind, StyleRefs, Text};
use crate::sheet::{CellAddr, Row, RowKey, SecKey, Section};
use crate::strings::Sym;
use crate::units::Unit;

mod frame;
#[cfg(test)]
mod tests;

pub use frame::{fit_frame, geometry_bounds, update_alignment_box};

/// the shape operations (Entwicklertools › Shape-Design › Vorgänge)
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    /// Vereinigen (Union): the outline of all of them
    Union,
    /// Kombinieren (Combine): all sections in one shape, overlaps become holes
    Combine,
    /// Fragmentieren (Fragment): a shape for every region the outlines and lines make
    Fragment,
    /// Schneiden (Intersect): what all of them cover
    Intersect,
    /// Subtrahieren (Subtract): the primary shape without the others
    Subtract,
}

// ---------------------------------------------------------------- the shapes as pieces

// a piece of a sub-path, ending at its end point
#[derive(Clone, Debug)]
enum Piece {
    Line([f64; 2]),
    Arc(Conic, [f64; 2]),
    Free(Vec<Seg>),
}

impl Piece {
    fn end(&self) -> [f64; 2] {
        match self {
            Piece::Line(p) | Piece::Arc(_, p) => *p,
            Piece::Free(s) => s.last().map_or([0.0, 0.0], |x| x.end()),
        }
    }

    fn map(&self, m: &Xform) -> Piece {
        let seg = |s: &Seg| match *s {
            Seg::Line(p) => Seg::Line(m.apply(p)),
            Seg::Cubic(a, b, p) => Seg::Cubic(m.apply(a), m.apply(b), m.apply(p)),
        };
        match self {
            Piece::Line(p) => Piece::Line(m.apply(*p)),
            Piece::Arc(c, p) => Piece::Arc(c.map(m), m.apply(*p)),
            Piece::Free(s) => Piece::Free(s.iter().map(seg).collect()),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PSub {
    start: [f64; 2],
    pieces: Vec<Piece>,
    closed: bool,
}

impl PSub {
    fn map(&self, m: &Xform) -> PSub {
        PSub { start: m.apply(self.start), pieces: self.pieces.iter().map(|p| p.map(m)).collect(), closed: self.closed }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PSec {
    no_fill: bool,
    no_line: bool,
    no_show: bool,
    subs: Vec<PSub>,
}

// the Geometry sections of a shape as pieces in its local coordinates (sub-paths of InfiniteLine
// rows left out: they are drawn 1000 inches long)
pub(crate) fn shape_paths(doc: &Document, shape: ShapeRef) -> Vec<PSec> {
    let sheet = SheetRef::Shape(shape);
    let (w, h) = (ops::val(doc, sheet, names::WIDTH), ops::val(doc, sheet, names::HEIGHT));
    let mut out = Vec::new();
    for (g, tags) in doc.geometry_tagged(sheet, w, h) {
        let sk = SecKey { name: names::GEOMETRY, index: g.index };
        let kinds: Vec<(u32, Option<Sym>)> = doc
            .rows(sheet, sk)
            .iter()
            .filter_map(|r| match r.key {
                RowKey::Index(i) => Some((i, r.kind)),
                RowKey::Named(_) => None,
            })
            .collect();
        let kind_of = |ix: u32| kinds.iter().find(|k| k.0 == ix).and_then(|k| k.1).map_or("", |k| doc.str(k));
        let mut subs = Vec::new();
        'sub: for (sub, tg) in g.path.subs.iter().zip(tags.iter()) {
            let mut pieces = Vec::new();
            let mut at = sub.start;
            let mut i = 0;
            while i < sub.segs.len() {
                let row = tg.get(i).copied().unwrap_or(0);
                let mut j = i + 1;
                while j < sub.segs.len() && tg.get(j).copied() == Some(row) {
                    j += 1;
                }
                let segs = &sub.segs[i..j];
                let end = segs[segs.len() - 1].end();
                if kind_of(row) == "InfiniteLine" {
                    continue 'sub;
                }
                let curved = segs.iter().any(|s| matches!(s, Seg::Cubic(..)));
                let conic = if curved { doc.row_conic(sheet, sk, RowKey::Index(row), at, w, h) } else { None };
                match conic {
                    Some(c) => pieces.push(Piece::Arc(c, end)),
                    None if !curved => pieces.extend(segs.iter().map(|s| Piece::Line(s.end()))),
                    None => pieces.push(Piece::Free(segs.to_vec())),
                }
                at = end;
                i = j;
            }
            subs.push(PSub { start: sub.start, pieces, closed: sub.closed });
        }
        out.push(PSec { no_fill: g.no_fill, no_line: g.no_line, no_show: g.no_show, subs });
    }
    out
}

// ---------------------------------------------------------------- flattening

// where an edge of a flattened path comes from
#[derive(Copy, Clone, Debug)]
enum Src {
    Line,
    Arc(Conic),
    Free,
}

// a flattened sub-path in page coordinates: points, the source of each edge (edge i from point i
// to i + 1; a closed one's last edge back to point 0), the flags of its section
#[derive(Clone, Debug)]
struct Flat {
    pts: Vec<[f64; 2]>,
    src: Vec<u32>,
    closed: bool,
    no_fill: bool,
    no_line: bool,
}

// how many chords an arc gets: none further than tol from it, at least eight per turn
fn arc_steps(c: &Conic, tol: f64) -> usize {
    let r = sqrt(c.u[0] * c.u[0] + c.u[1] * c.u[1]).max(sqrt(c.v[0] * c.v[0] + c.v[1] * c.v[1]));
    let step = if r > tol { 2.0 * sqrt(2.0 * tol / r) } else { core::f64::consts::PI };
    let n = ceil(c.sweep.abs() / step.max(1e-6));
    let min = ceil(c.sweep.abs() / core::f64::consts::FRAC_PI_4).max(2.0);
    n.max(min).min(720.0) as usize
}

// how many chords a cubic gets (its second differences bound the error)
fn cubic_steps(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], tol: f64) -> usize {
    let dd = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        let (x, y) = (a[0] - 2.0 * b[0] + c[0], a[1] - 2.0 * b[1] + c[1]);
        sqrt(x * x + y * y)
    };
    let d = dd(p0, p1, p2).max(dd(p1, p2, p3));
    ceil(sqrt(0.75 * d / tol.max(1e-12))).clamp(1.0, 64.0) as usize
}

fn flatten(sub: &PSub, tol: f64, srcs: &mut Vec<Src>, no_fill: bool, no_line: bool) -> Flat {
    let mut pts = alloc::vec![sub.start];
    let mut src = Vec::new();
    let mut at = sub.start;
    let add = |s: Src, srcs: &mut Vec<Src>| {
        srcs.push(s);
        (srcs.len() - 1) as u32
    };
    for p in &sub.pieces {
        match p {
            Piece::Line(e) => {
                let id = add(Src::Line, srcs);
                pts.push(*e);
                src.push(id);
            }
            Piece::Arc(c, e) => {
                let id = add(Src::Arc(*c), srcs);
                let n = arc_steps(c, tol);
                for k in 1..n {
                    pts.push(c.at(c.t0 + c.sweep * k as f64 / n as f64));
                    src.push(id);
                }
                pts.push(*e);
                src.push(id);
            }
            Piece::Free(segs) => {
                let id = add(Src::Free, srcs);
                let mut prev = at;
                for s in segs {
                    match *s {
                        Seg::Line(e) => {
                            pts.push(e);
                            src.push(id);
                        }
                        Seg::Cubic(a, b, e) => {
                            let m = cubic_steps(prev, a, b, e, tol);
                            for k in 1..=m {
                                pts.push(if k == m { e } else { cubic_at(prev, a, b, e, k as f64 / m as f64) });
                                src.push(id);
                            }
                        }
                    }
                    prev = s.end();
                }
            }
        }
        at = p.end();
    }
    if sub.closed && pts.len() > 1 {
        // the last point is the start again: the last edge closes the contour
        pts.pop();
    }
    Flat { pts, src, closed: sub.closed, no_fill, no_line }
}

// ---------------------------------------------------------------- the inputs

// a shape as an operand: closed contours (the area), open paths (lines), and its sections as they
// are (Kombinieren)
#[derive(Default)]
struct Input {
    closed: Vec<Flat>,
    open: Vec<Flat>,
    sections: Vec<Vec<Flat>>,
}

fn input_of(doc: &Document, shape: ShapeRef, tol: f64, srcs: &mut Vec<Src>, depth: usize) -> Result<Input, EditError> {
    let Some(sh) = doc.shape(shape) else { return Err(EditError::NoShape) };
    let xf = doc.page_xform(shape);
    let mut own = Input::default();
    for sec in shape_paths(doc, shape) {
        if sec.no_show {
            continue;
        }
        let mut flats = Vec::new();
        for sub in &sec.subs {
            if sub.pieces.is_empty() {
                continue;
            }
            let f = flatten(&sub.map(&xf), tol, srcs, sec.no_fill, sec.no_line);
            if f.closed { own.closed.push(f.clone()) } else { own.open.push(f.clone()) }
            flats.push(f);
        }
        if !flats.is_empty() {
            own.sections.push(flats);
        }
    }
    if sh.header().kind != ShapeKind::Group || depth > 32 {
        return Ok(own);
    }
    // a group: the union of its members and of its own geometry
    let kids: Vec<ShapeRef> = sh.children().collect();
    let mut parts = alloc::vec![own];
    for k in kids {
        parts.push(input_of(doc, k, tol, srcs, depth + 1)?);
    }
    let mut out = Input::default();
    let ops_in: Vec<Operand> = parts.iter().map(|p| Operand::closed(p.closed.iter().map(|f| f.pts.clone()).collect())).collect();
    if ops_in.iter().any(|o| !o.contours.is_empty()) {
        let res = region::union(&ops_in, FillRule::EvenOdd).map_err(|_| EditError::Invalid)?;
        for c in res {
            let src = c.marks.iter().map(|m| parts[m.operand as usize].closed[m.contour as usize].src[m.edge as usize]).collect();
            out.closed.push(Flat { pts: c.points, src, closed: true, no_fill: false, no_line: false });
        }
    }
    for p in parts {
        out.open.extend(p.open);
        out.sections.extend(p.sections);
    }
    Ok(out)
}

// ---------------------------------------------------------------- rebuilding rows

// a row of a result in its parent's coordinates
#[derive(Clone, Debug)]
enum RowOut {
    Move([f64; 2]),
    Line([f64; 2]),
    // EllipticalArcTo: the end, the control point, C, D, and the piece of the ellipse (its box)
    Arc { to: [f64; 2], ctl: [f64; 2], angle: f64, ratio: f64, conic: Conic },
    // Ellipse: the centre, the ends of its two axes
    Ellipse { conic: Conic, a: [f64; 2], b: [f64; 2] },
}

#[derive(Clone, Debug)]
struct SecOut {
    no_fill: bool,
    no_line: bool,
    rows: Vec<RowOut>,
}

// the angle and ratio of EllipticalArcTo for an ellipse: (C, D); None for a degenerate one.  An
// upright ellipse gets C = 0 (D the ratio of its x axis to its y axis), a circle C = 0, D = 1
fn arc_axes(c: &Conic) -> Option<(f64, f64)> {
    let (phi, s1, s2) = c.axes();
    if !(s1.is_finite() && s2 > 1e-12 * s1) {
        return None;
    }
    if s1 - s2 <= 1e-9 * s1 {
        return Some((0.0, 1.0));
    }
    let (s, co) = crate::geometry::sincos(phi);
    if s.abs() < 1e-9 {
        return Some((0.0, s1 / s2));
    }
    if co.abs() < 1e-9 {
        return Some((0.0, s2 / s1));
    }
    // the major axis in (-90, 90] degrees
    let pi = core::f64::consts::PI;
    let mut a = rem(phi, pi);
    if a > pi * 0.5 {
        a -= pi;
    }
    Some((a, s1 / s2))
}

// the rows of a run on an arc from ps to pe, `inner` a point after ps on it (the direction)
fn arc_rows(c: &Conic, ps: [f64; 2], pe: [f64; 2], inner: [f64; 2], out: &mut Vec<RowOut>) {
    let pi = core::f64::consts::PI;
    let tau = core::f64::consts::TAU;
    let (Some(ts), Some(te), Some(ti), Some((angle, ratio))) = (c.param(ps), c.param(pe), c.param(inner), arc_axes(c)) else {
        out.push(RowOut::Line(pe));
        return;
    };
    let wrap = |x: f64| rem(x + pi, tau) - pi;
    let dir = if wrap(ti - ts) >= 0.0 { 1.0 } else { -1.0 };
    let mut span = if dir > 0.0 { rem(te - ts, tau) } else { -rem(ts - te, tau) };
    if span.abs() < 1e-12 || (tau - span.abs()) < 1e-12 {
        if (pe[0] - ps[0]).abs() + (pe[1] - ps[1]).abs() > 0.0 {
            out.push(RowOut::Line(pe));
            return;
        }
        span = dir * tau;
    }
    let k = ceil(span.abs() / (pi * (1.0 - 1e-9))).max(1.0) as usize;
    for i in 0..k {
        let ta = ts + span * i as f64 / k as f64;
        let tb = ts + span * (i + 1) as f64 / k as f64;
        let to = if i + 1 == k { pe } else { c.at(tb) };
        out.push(RowOut::Arc { to, ctl: c.at((ta + tb) * 0.5), angle, ratio, conic: Conic { t0: ta, sweep: tb - ta, ..*c } });
    }
}

// the rows of a flattened contour or path; `m` takes page coordinates to the parent's
fn rows_of(f: &Flat, srcs: &[Src], m: &Xform) -> Vec<RowOut> {
    let np = f.pts.len();
    let ne = if f.closed { np } else { np.saturating_sub(1) };
    if ne == 0 || f.src.len() < ne {
        return Vec::new();
    }
    let pts: Vec<[f64; 2]> = f.pts.iter().map(|p| m.apply(*p)).collect();
    let src = &f.src;
    // runs of one source, starting where the source changes
    let first = if f.closed { (0..ne).find(|&i| src[i] != src[(i + ne - 1) % ne]) } else { Some(0) };
    let single = first.is_none();
    let s0 = first.unwrap_or(0);
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < ne {
        let id = src[(s0 + i) % ne];
        let mut j = i + 1;
        while j < ne && src[(s0 + j) % ne] == id {
            j += 1;
        }
        runs.push((i, j - i));
        i = j;
    }
    let vtx = |k: usize| pts[(s0 + k) % np];
    let conic_of = |r: usize| match srcs[src[(s0 + runs[r].0) % ne] as usize] {
        Src::Arc(c) => Some(c.map(m)),
        _ => None,
    };
    let mut out = Vec::new();
    // a whole ellipse
    if single {
        if let Some(c) = conic_of(0).filter(|c| c.is_full()) {
            let (phi, s1, s2) = c.axes();
            let (s, co) = crate::geometry::sincos(phi);
            let a = [c.c[0] + s1 * co, c.c[1] + s1 * s];
            let b = [c.c[0] - s2 * s, c.c[1] + s2 * co];
            out.push(RowOut::Ellipse { conic: Conic { t0: 0.0, sweep: core::f64::consts::TAU, ..c }, a, b });
            return out;
        }
    }
    // the ends of the runs, on the arc next to them
    let nb = runs.len();
    let mut bpos: Vec<[f64; 2]> = (0..=nb).map(|r| vtx(if r < nb { runs[r].0 } else { ne })).collect();
    for (r, b) in bpos.iter_mut().enumerate() {
        let here = if r < nb { conic_of(r) } else if f.closed { conic_of(0) } else { None };
        let before = if r > 0 { conic_of(r - 1) } else if f.closed { conic_of(nb - 1) } else { None };
        if let Some(c) = here.or(before) {
            if let Some(t) = c.param(*b) {
                *b = c.at(t);
            }
        }
    }
    if f.closed {
        bpos[nb] = bpos[0];
    }
    out.push(RowOut::Move(bpos[0]));
    for r in 0..nb {
        let (a, cnt) = runs[r];
        let (ps, pe) = (bpos[r], bpos[r + 1]);
        match srcs[src[(s0 + a) % ne] as usize] {
            Src::Line => out.push(RowOut::Line(pe)),
            Src::Free => {
                for k in 1..cnt {
                    out.push(RowOut::Line(vtx(a + k)));
                }
                out.push(RowOut::Line(pe));
            }
            Src::Arc(c) => {
                let inner = if cnt > 1 { vtx(a + 1) } else { pe };
                arc_rows(&c.map(m), ps, pe, inner, &mut out);
            }
        }
    }
    // rows that go nowhere (a run of a chord's sagitta, its end projected onto the arc)
    let mut at = bpos[0];
    out.retain(|r| {
        let p = match r {
            RowOut::Move(_) => return true,
            RowOut::Line(p) => *p,
            RowOut::Arc { to, .. } => *to,
            RowOut::Ellipse { .. } => return true,
        };
        let keep = (p[0] - at[0]).abs() + (p[1] - at[1]).abs() > 1e-9 * (1.0 + p[0].abs() + p[1].abs());
        if keep {
            at = p;
        }
        keep
    });
    out
}

fn grow(b: &mut Option<[f64; 4]>, r: [f64; 4]) {
    *b = Some(match *b {
        None => r,
        Some(x) => [x[0].min(r[0]), x[1].min(r[1]), x[2].max(r[2]), x[3].max(r[3])],
    });
}

fn rows_bounds(secs: &[SecOut]) -> Option<[f64; 4]> {
    let mut b = None;
    for s in secs {
        for r in &s.rows {
            match r {
                RowOut::Move(p) | RowOut::Line(p) => grow(&mut b, [p[0], p[1], p[0], p[1]]),
                RowOut::Arc { to, conic, .. } => {
                    grow(&mut b, [to[0], to[1], to[0], to[1]]);
                    grow(&mut b, conic.bounds());
                }
                RowOut::Ellipse { conic, .. } => grow(&mut b, conic.bounds()),
            }
        }
    }
    b
}

// ---------------------------------------------------------------- the primary shape's format

struct Fmt {
    styles: StyleRefs,
    cells: Vec<Cell>,
    sections: Vec<Section>,
    text: Option<Text>,
}

// the format cells a result takes over: line, fill, effects, text block, the theme and quick
// style cells, the layers
fn takes(doc: &Document, name: Sym) -> bool {
    let n = doc.str(name);
    Category::of(None, n).is_some()
        || name == names::LAYER_MEMBER
        || [names::TXT_PIN_X, names::TXT_PIN_Y, names::TXT_WIDTH, names::TXT_HEIGHT, names::TXT_LOC_PIN_X, names::TXT_LOC_PIN_Y, names::TXT_ANGLE].contains(&name)
}

// a cell as `sheet` sees it, for a shape of its own: a formula naming another sheet becomes its
// value
fn own_cell(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Option<Cell> {
    let r = doc.resolve(sheet, addr);
    if !matches!(r.source, Some(SheetRef::Shape(_))) {
        return None;
    }
    let formula = match r.formula() {
        Formula::Expr(f) if doc.str(f).contains('!') => Formula::Absent,
        Formula::Expr(f) => Formula::Expr(f),
        _ => Formula::Absent,
    };
    Some(Cell { name: addr.cell, value: r.value(), formula, unit: r.unit(), error: None, stale: false })
}

fn format_of(doc: &Document, primary: ShapeRef, with_text: bool) -> Fmt {
    let sheet = SheetRef::Shape(primary);
    let mut chain = alloc::vec![primary];
    while let Some(m) = chain.last().and_then(|&s| doc.master_shape_of(s)) {
        if chain.len() > 32 || chain.contains(&m) {
            break;
        }
        chain.push(m);
    }
    // the styles: the shape's own, else its master shape's
    let mut styles = StyleRefs::default();
    for &s in &chain {
        if let Some(sh) = doc.shape(s) {
            let st = sh.header().styles;
            styles.line = styles.line.or(st.line);
            styles.fill = styles.fill.or(st.fill);
            styles.text = styles.text.or(st.text);
        }
    }
    let mut names_: BTreeSet<Sym> = BTreeSet::new();
    for &s in &chain {
        if let Some(sh) = doc.sheet(SheetRef::Shape(s)) {
            names_.extend(sh.cells.iter().map(|c| c.name).filter(|&n| takes(doc, n)));
        }
    }
    let cells: Vec<Cell> = names_.into_iter().filter_map(|n| own_cell(doc, sheet, &CellAddr::single(n))).collect();
    let mut wanted = alloc::vec![names::LINE_GRADIENT, names::FILL_GRADIENT];
    if with_text {
        wanted.extend([names::CHARACTER, names::PARAGRAPH, names::TABS, names::FIELD]);
    }
    let mut sections = Vec::new();
    for name in wanted {
        let sk = SecKey::one(name);
        let mut sec = Section::new(sk);
        for e in doc.rows(sheet, sk) {
            let mut cnames: Vec<Sym> = Vec::new();
            for &s in &chain {
                if let Some(r) = doc.sheet(SheetRef::Shape(s)).and_then(|x| x.section(sk)).and_then(|x| x.row(e.key)) {
                    for c in &r.cells {
                        if !cnames.contains(&c.name) {
                            cnames.push(c.name);
                        }
                    }
                }
            }
            let rcells: Vec<Cell> = cnames.into_iter().filter_map(|n| own_cell(doc, sheet, &CellAddr::row(sk, e.key, n))).collect();
            if rcells.is_empty() {
                continue;
            }
            let mut row = Row::new(e.key);
            row.kind = e.kind;
            row.cells = rcells;
            sec.rows.push(row);
        }
        if !sec.rows.is_empty() {
            sections.push(sec);
        }
    }
    let text = if with_text { doc.shape(primary).and_then(|s| s.text().cloned()) } else { None };
    Fmt { styles, cells, sections, text }
}

// ---------------------------------------------------------------- the result shape

fn cform(tx: &mut Transaction, name: Sym, f: &str, v: f64) -> Cell {
    let f = tx.intern(f);
    Cell { name, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: true }
}

// a length as "Width*k" (or Height) of a box `size` across, a value when the box has none
fn prop(tx: &mut Transaction, name: Sym, v: f64, size: f64, height: bool) -> Cell {
    if size.abs() > 1e-9 {
        let cell = if height { "Height" } else { "Width" };
        cform(tx, name, &alloc::format!("{cell}*{}", ops::text::num(v / size)), v)
    } else {
        Cell::constant(name, Value::Num(v), None)
    }
}

fn geo_row(ix: u32, kind: Sym, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Index(ix));
    r.kind = Some(kind);
    r.cells = cells;
    r
}

// the Geometry sections of a result in a box from `o` (its local origin), w x h
fn geometry_sections(tx: &mut Transaction, secs: &[SecOut], o: [f64; 2], w: f64, h: f64) -> Vec<Section> {
    let mut out = Vec::new();
    for (si, s) in secs.iter().enumerate() {
        let mut sec = Section::new(SecKey { name: names::GEOMETRY, index: si as u32 });
        sec.ix_written = true;
        for (n, v) in [(names::NO_FILL, s.no_fill), (names::NO_LINE, s.no_line), (names::NO_SHOW, false), (names::NO_SNAP, false), (names::NO_QUICK_DRAG, false)] {
            sec.cells.push(Cell::constant(n, Value::Num(v as u8 as f64), None));
        }
        let l = |p: [f64; 2]| [p[0] - o[0], p[1] - o[1]];
        for (ri, r) in s.rows.iter().enumerate() {
            let ix = ri as u32 + 1;
            let row = match r {
                RowOut::Move(p) | RowOut::Line(p) => {
                    let q = l(*p);
                    let kind = if matches!(r, RowOut::Move(_)) { names::MOVE_TO } else { names::LINE_TO };
                    geo_row(ix, kind, alloc::vec![prop(tx, names::X, q[0], w, false), prop(tx, names::Y, q[1], h, true)])
                }
                RowOut::Arc { to, ctl, angle, ratio, .. } => {
                    let (q, c) = (l(*to), l(*ctl));
                    // an upright arc: D follows the box's proportions
                    let d = if *angle == 0.0 && w.abs() > 1e-9 && h.abs() > 1e-9 {
                        cform(tx, names::D, &alloc::format!("Width/Height*{}", ops::text::num(ratio * h / w)), *ratio)
                    } else {
                        Cell::constant(names::D, Value::Num(*ratio), None)
                    };
                    let cells = alloc::vec![
                        prop(tx, names::X, q[0], w, false),
                        prop(tx, names::Y, q[1], h, true),
                        prop(tx, names::A, c[0], w, false),
                        prop(tx, names::B, c[1], h, true),
                        Cell::constant(names::C, Value::Num(*angle), None),
                        d,
                    ];
                    geo_row(ix, names::ELLIPTICAL_ARC_TO, cells)
                }
                RowOut::Ellipse { conic, a, b } => {
                    let (c, a, b) = (l(conic.c), l(*a), l(*b));
                    let cells = alloc::vec![
                        prop(tx, names::X, c[0], w, false),
                        prop(tx, names::Y, c[1], h, true),
                        prop(tx, names::A, a[0], w, false),
                        prop(tx, names::B, a[1], h, true),
                        prop(tx, names::C, b[0], w, false),
                        prop(tx, names::D, b[1], h, true),
                    ];
                    geo_row(ix, names::ELLIPSE, cells)
                }
            };
            sec.rows.push(row);
        }
        out.push(sec);
    }
    out
}

// a new shape in `parent` (None: the page) with the sections `secs` (the parent's coordinates)
// and the format `fmt` -> the shape
fn new_result(tx: &mut Transaction, eng: &mut Engine, owner: Owner, parent: Option<ShapeRef>, secs: &[SecOut], fmt: &Fmt) -> Result<ShapeRef, EditError> {
    let b = rows_bounds(secs).ok_or(EditError::Invalid)?;
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = fmt.styles;
    let lx = cform(tx, names::LOC_PIN_X, "Width*0.5", w * 0.5);
    let ly = cform(tx, names::LOC_PIN_Y, "Height*0.5", h * 0.5);
    let mut cells = alloc::vec![
        Cell::constant(names::PIN_X, Value::Num((b[0] + b[2]) * 0.5), None),
        Cell::constant(names::PIN_Y, Value::Num((b[1] + b[3]) * 0.5), None),
        Cell::constant(names::WIDTH, Value::Num(w), None),
        Cell::constant(names::HEIGHT, Value::Num(h), None),
        lx,
        ly,
        Cell::constant(names::ANGLE, Value::Num(0.0), Some(Unit::Degrees)),
        Cell::constant(names::FLIP_X, Value::Num(0.0), None),
        Cell::constant(names::FLIP_Y, Value::Num(0.0), None),
    ];
    for c in &fmt.cells {
        if !cells.iter().any(|x| x.name == c.name) {
            cells.push(*c);
        }
    }
    d.sheet.cells = cells;
    d.sheet.sections = fmt.sections.clone();
    d.sheet.sections.extend(geometry_sections(tx, secs, [b[0], b[1]], w, h));
    d.text = fmt.text.clone().map(Box::new);
    let s = tx.add_shape_with(owner, parent, d)?;
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    Ok(s)
}

// ---------------------------------------------------------------- the operations

// whether a shape has closed geometry (an area) and whether it has any
fn has_geometry(doc: &Document, shape: ShapeRef, depth: usize) -> (bool, bool) {
    let mut area = false;
    let mut any = false;
    for sec in shape_paths(doc, shape) {
        if sec.no_show {
            continue;
        }
        for s in &sec.subs {
            if !s.pieces.is_empty() {
                any = true;
                area |= s.closed;
            }
        }
    }
    if let Some(sh) = doc.shape(shape) {
        if sh.header().kind == ShapeKind::Group && depth < 32 {
            for k in sh.children() {
                let (a, n) = has_geometry(doc, k, depth + 1);
                area |= a;
                any |= n;
            }
        }
    }
    (area, any)
}

// the shapes of a selection that can take part: existing, one owner, none inside another one of
// the list, in the given order
fn inputs(doc: &Document, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    let list = ops::outermost(doc, shapes);
    let Some(&first) = list.first() else { return list };
    list.into_iter().filter(|s| s.owner == first.owner).collect()
}

/// whether `op` can run on these shapes (shapes[0] the primary one): Vereinigen and
/// Fragmentieren need one shape with geometry (Vereinigen: with an area), Kombinieren two with
/// geometry, Schneiden two with an area, Subtrahieren a primary shape with an area and another one
/// with an area
pub fn available(doc: &Document, shapes: &[ShapeRef], op: Operation) -> bool {
    let list = inputs(doc, shapes);
    let geo: Vec<(bool, bool)> = list.iter().map(|&s| has_geometry(doc, s, 0)).collect();
    let areas = geo.iter().filter(|g| g.0).count();
    let any = geo.iter().filter(|g| g.1).count();
    match op {
        Operation::Union => areas >= 1,
        Operation::Fragment => areas >= 1,
        Operation::Combine => any >= 2,
        Operation::Intersect => areas >= 2,
        Operation::Subtract => geo.first().is_some_and(|g| g.0) && areas >= 2,
    }
}

/// Run a shape operation on shapes of one page or master, `shapes[0]` the primary shape (the
/// first one selected): its format and text go to the result (Fragmentieren: the format only),
/// the result goes into its parent on top.  The shapes that take part are deleted (Vereinigen,
/// Schneiden, Subtrahieren: those with an area; Kombinieren, Fragmentieren: all).  -> the new
/// shapes: one, a piece each for Fragmentieren, none when nothing is left.  Invalid when the
/// operation cannot run on these shapes (available)
pub fn operate(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], op: Operation) -> Result<Vec<ShapeRef>, EditError> {
    if !available(tx.doc(), shapes, op) {
        return Err(EditError::Invalid);
    }
    let list = inputs(tx.doc(), shapes);
    let primary = list[0];
    let owner = primary.owner;
    let (secs, take_part, fmt, parent) = {
        let doc = tx.doc();
        let parent = doc.shape(primary).ok_or(EditError::NoShape)?.parent();
        // the flattening tolerance: a 50 000th of the extent of the inputs
        let b = ops::bounds(doc, &list).unwrap_or([0.0, 0.0, 1.0, 1.0]);
        let tol = ((b[2] - b[0]).max(b[3] - b[1]) * 2e-5).max(1e-7);
        let mut srcs: Vec<Src> = Vec::new();
        let mut ins: Vec<Input> = Vec::new();
        for &s in &list {
            ins.push(input_of(doc, s, tol, &mut srcs, 0)?);
        }
        let to_parent = parent.map_or(Some(Xform::IDENTITY), |p| doc.page_xform(p).inverse()).ok_or(EditError::Invalid)?;
        let area: Vec<bool> = ins.iter().map(|i| !i.closed.is_empty()).collect();
        let closed_ops = |which: &[usize]| -> Vec<Operand> { which.iter().map(|&k| Operand::closed(ins[k].closed.iter().map(|f| f.pts.clone()).collect())).collect() };
        let geo_err = |_| EditError::Invalid;
        // per new shape: its sections
        let mut results: Vec<Vec<SecOut>> = Vec::new();
        let contours_out = |cs: &[region::Contour], which: &[usize], results_secs: &mut Vec<SecOut>| {
            for c in cs {
                let src = c.marks.iter().map(|m| ins[which[m.operand as usize]].closed[m.contour as usize].src[m.edge as usize]).collect();
                let f = Flat { pts: c.points.clone(), src, closed: true, no_fill: false, no_line: false };
                let rows = rows_of(&f, &srcs, &to_parent);
                if !rows.is_empty() {
                    results_secs.push(SecOut { no_fill: false, no_line: false, rows });
                }
            }
        };
        let take_part: Vec<ShapeRef> = match op {
            Operation::Union | Operation::Intersect | Operation::Subtract => {
                let which: Vec<usize> = (0..list.len()).filter(|&k| area[k]).collect();
                let bop = match op {
                    Operation::Union => region::BoolOp::Union,
                    Operation::Intersect => region::BoolOp::Intersection,
                    _ => region::BoolOp::Difference,
                };
                let cs = region::boolean(&closed_ops(&which), bop, FillRule::EvenOdd).map_err(geo_err)?;
                let mut s = Vec::new();
                contours_out(&cs, &which, &mut s);
                if !s.is_empty() {
                    results.push(s);
                }
                which.iter().map(|&k| list[k]).collect()
            }
            Operation::Fragment => {
                // the areas, then the lines of every shape as cutters (operand list.len() + k)
                let all: Vec<usize> = (0..list.len()).collect();
                let mut opsv = closed_ops(&all);
                for i in &ins {
                    opsv.push(Operand::open(i.open.iter().map(|f| f.pts.clone()).collect()));
                }
                let pieces = region::decompose(&opsv, FillRule::EvenOdd).map_err(geo_err)?;
                for p in pieces {
                    let mut s = Vec::new();
                    for c in &p.contours {
                        let src = c
                            .marks
                            .iter()
                            .map(|m| {
                                let o = m.operand as usize;
                                if o < list.len() { ins[o].closed[m.contour as usize].src[m.edge as usize] } else { ins[o - list.len()].open[m.contour as usize].src[m.edge as usize] }
                            })
                            .collect();
                        let f = Flat { pts: c.points.clone(), src, closed: true, no_fill: false, no_line: false };
                        let rows = rows_of(&f, &srcs, &to_parent);
                        if !rows.is_empty() {
                            s.push(SecOut { no_fill: false, no_line: false, rows });
                        }
                    }
                    if !s.is_empty() {
                        results.push(s);
                    }
                }
                list.clone()
            }
            Operation::Combine => {
                let mut s = Vec::new();
                for i in &ins {
                    for sec in &i.sections {
                        let mut rows = Vec::new();
                        for f in sec {
                            rows.extend(rows_of(f, &srcs, &to_parent));
                        }
                        if !rows.is_empty() {
                            s.push(SecOut { no_fill: sec[0].no_fill, no_line: sec[0].no_line, rows });
                        }
                    }
                }
                if !s.is_empty() {
                    results.push(s);
                }
                list.clone()
            }
        };
        let fmt = format_of(doc, primary, op != Operation::Fragment);
        (results, take_part, fmt, parent)
    };
    // the parent goes when it is one of the inputs' groups: then the result goes to the page
    let parent = parent.filter(|p| !take_part.contains(p) && tx.doc().shape(*p).is_some());
    let mut made = Vec::new();
    for s in &secs {
        made.push(new_result(tx, eng, owner, parent, s, &fmt)?);
    }
    // the inputs go: glue to them let go, memberships forgotten
    crate::structure::forget(tx, eng, &take_part)?;
    crate::glue::detach(tx, eng, owner, &take_part)?;
    tx.delete_shapes(&take_part)?;
    Ok(made)
}
