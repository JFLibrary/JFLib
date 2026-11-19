// ops::draw -- the freeform and arc tools (RS-16, KB-09): a stroke smoothed into a NURBS curve,
// a quarter of an ellipse between two points.  Both are 1-D shapes like a line (glue::new_line):
// their ends glue to connection points (CN-12) and their geometry follows Width, so they keep
// their form when an end moves.
//
//   simplify(pts, tol) -> corners       Ramer-Douglas-Peucker: the points of a stroke that keep
//                                       every other point within tol of the polyline
//   Spline::of(points) -> Spline        a clamped B-spline (degree 3, fewer for 2 or 3 points)
//                                       with the points as its control polygon and evenly spaced
//                                       knots 0, 1, 2 ...; the curve starts and ends at the
//                                       first and last point and runs smoothly along the rest
//   quarter_arc(a, b, horizontal) -> Arc   the quarter ellipse from a to b with page-parallel
//                                       axes: horizontal = it leaves a horizontally (and meets b
//                                       vertically), else the other way round
//   new_freeform(tx, eng, owner, stroke, tol, close, begin, end) -> ShapeRef
//   new_arc(tx, eng, owner, begin, end, horizontal) -> ShapeRef
//
// How Visio keeps them (MS-VSDX, Geometry section rows; geometry.rs reads them the same way):
// * The freeform: MoveTo at the begin, then one NURBSTo whose X, Y is the end, A the knot of
//   the last control point, B its weight, C the knot of the first one (the current point), D
//   its weight, and E = NURBS(knotLast, degree, xType, yType, x1, y1, knot1, weight1, ...) the
//   control points between (type 0: a fraction of Width / Height, 1: local inches).  Visio's
//   own freeform tool writes NURBSTo too; its smoothing is its own (not documented), so the
//   curve here is a choice: the corners of the stroke as control points.  A 1-D shape has
//   Height 0 (its chord is the local x axis, as a line's), so the y values are local inches
//   written as Width*k inside the NURBS formula: the whole curve scales with Width when an
//   end moves, as a line's rows do (Annahme: Visio's 1-D freeforms keep y as constants of
//   their own; the formula keeps the form instead, and Visio evaluates NURBS() with
//   expressions like any function).
// * A stroke that ends where it began (within `close`) is a closed 2-D shape (Visio's freeform
//   tool closes a shape there and fills it): its box around the control points, the rows in
//   fractions of Width and Height (xType = yType = 0), filled.
// * The arc: MoveTo at the begin, an EllipticalArcTo to the end through the control point A, B
//   (the middle of the quarter: 45 degrees of the ellipse's parameter), C the angle of the
//   ellipse's major axis in the shape's local coordinates (the page's x axis, so -Angle at
//   the start), D the ratio of that axis to the other.  A and B are Width*k as well.  Visio's
//   arc tool draws a quarter ellipse whose bow follows the direction the drag starts in
//   (Annahme from its help: "drag in the direction you want the arc to bow"; the start's
//   axis decides here).
// The 1-D cells are those of new_line: PinX/Y in the middle, Width the chord, Angle its
// direction, LocPinX Width*0.5, Begin and End constants until glued.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::text::num as fnum;
use crate::cell::{Cell, Formula, Value};
use crate::document::{Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::{math, Engine};
use crate::glue::{self, Target};
use crate::names;
use crate::shape::{ShapeData, ShapeKind, StyleRefs};
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::strings::Sym;

/// the most control points a freeform keeps (a longer stroke is simplified harder)
pub const MAX_POINTS: usize = 400;

// ---------------------------------------------------------------- the stroke

/// the corners of a stroke: every point of `pts` lies within `tol` of the polyline through the
/// result, which starts and ends with the stroke's first and last point (repeated points go)
pub fn simplify(pts: &[[f64; 2]], tol: f64) -> Vec<[f64; 2]> {
    let mut p: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for &q in pts {
        if !(q[0].is_finite() && q[1].is_finite()) {
            continue;
        }
        if p.last().is_none_or(|l| dist(*l, q) > 1e-12) {
            p.push(q);
        }
    }
    if p.len() <= 2 {
        return p;
    }
    let mut keep = alloc::vec![false; p.len()];
    keep[0] = true;
    let n = p.len() - 1;
    keep[n] = true;
    // an explicit stack: a long stroke does not recurse deeply
    let mut stack = alloc::vec![(0usize, n)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut far, mut at) = (-1.0, a);
        for (i, &q) in p.iter().enumerate().take(b).skip(a + 1) {
            let d = seg_dist(q, p[a], p[b]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > tol {
            keep[at] = true;
            stack.push((a, at));
            stack.push((at, b));
        }
    }
    p.iter().zip(keep).filter(|(_, k)| *k).map(|(q, _)| *q).collect()
}

/// a clamped, uniform B-spline through its first and last control point
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    pub degree: usize,
    pub points: Vec<[f64; 2]>,
    /// the full knot vector: points + degree + 1 knots
    pub knots: Vec<f64>,
}

impl Spline {
    /// the spline with these control points: degree 3 (2 for three points, 1 for two), knots
    /// 0 .. 0, 1, 2 .. n - degree + 1 .. (None: fewer than two points)
    pub fn of(points: &[[f64; 2]]) -> Option<Spline> {
        if points.len() < 2 {
            return None;
        }
        let n = points.len() - 1;
        let p = n.min(3);
        let last = (n - p + 1) as f64;
        let mut knots = Vec::with_capacity(n + p + 2);
        for i in 0..=n {
            knots.push(i.saturating_sub(p) as f64);
        }
        for _ in 0..=p {
            knots.push(last);
        }
        knots.truncate(n + p + 2);
        Some(Spline { degree: p, points: points.to_vec(), knots })
    }

    /// the point at parameter t in 0 ..= 1 of the knot range (de Boor)
    pub fn at(&self, t: f64) -> [f64; 2] {
        let p = self.degree;
        let n = self.points.len();
        let (lo, hi) = (self.knots[p], self.knots[n]);
        let u = lo + (hi - lo) * t.clamp(0.0, 1.0);
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= u {
            k += 1;
        }
        let mut d: Vec<[f64; 2]> = (0..=p).map(|j| self.points[j + k - p]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = j + k - p;
                let den = self.knots[i + p + 1 - r] - self.knots[i];
                let a = if den > 0.0 { (u - self.knots[i]) / den } else { 0.0 };
                d[j] = [d[j - 1][0] * (1.0 - a) + d[j][0] * a, d[j - 1][1] * (1.0 - a) + d[j][1] * a];
            }
        }
        d[p]
    }
}

// ---------------------------------------------------------------- the arc

/// a quarter of an ellipse with axes parallel to the page's
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Arc {
    pub centre: [f64; 2],
    /// the half axes along x and y
    pub rx: f64,
    pub ry: f64,
    /// the point halfway (45 degrees of the parameter)
    pub mid: [f64; 2],
}

/// the quarter ellipse from `a` to `b`: leaving `a` horizontally (horizontal) or vertically
pub fn quarter_arc(a: [f64; 2], b: [f64; 2], horizontal: bool) -> Arc {
    let h = core::f64::consts::FRAC_1_SQRT_2;
    // a horizontal start puts a on the ellipse's vertical axis, b on its horizontal one
    let centre = if horizontal { [a[0], b[1]] } else { [b[0], a[1]] };
    let (rx, ry) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
    let (ux, uy) = if horizontal { (b[0] - a[0], a[1] - b[1]) } else { (a[0] - b[0], b[1] - a[1]) };
    Arc { centre, rx, ry, mid: [centre[0] + ux * h, centre[1] + uy * h] }
}

// ---------------------------------------------------------------- the shapes

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    math::sqrt(dx * dx + dy * dy)
}

// the distance of q from the segment a..b
fn seg_dist(q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if !(l2 > 0.0) {
        return dist(q, a);
    }
    let t = (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
    dist(q, [a[0] + dx * t, a[1] + dy * t])
}

fn cnum(tx: &mut Transaction, name: &str, v: f64) -> Cell {
    Cell::constant(tx.intern(name), Value::Num(v), None)
}

fn cform(tx: &mut Transaction, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (tx.intern(name), tx.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: true }
}

fn row(ix: u32, kind: Sym, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Index(ix));
    r.kind = Some(kind);
    r.cells = cells;
    r
}

// a Geometry section with the flags (NoFill as asked) and these rows
fn section(no_fill: bool, rows: Vec<Row>) -> Section {
    let mut sec = Section::new(SecKey::one(names::GEOMETRY));
    sec.ix_written = true;
    for (n, v) in [(names::NO_FILL, no_fill as u8 as f64), (names::NO_LINE, 0.0), (names::NO_SHOW, 0.0), (names::NO_SNAP, 0.0), (names::NO_QUICK_DRAG, 0.0)] {
        sec.cells.push(Cell::constant(n, Value::Num(v), None));
    }
    sec.rows = rows;
    sec
}

// the document's default styles (as a line drawn with the line tool gets them)
fn default_styles(tx: &Transaction) -> StyleRefs {
    let st = &tx.doc().settings;
    let known = |id: Option<u32>| id.filter(|&i| tx.doc().style_by_id(i).is_some());
    StyleRefs { line: known(st.default_line_style), fill: known(st.default_fill_style), text: known(st.default_text_style) }
}

// the cells of a 1-D shape from b to e (glue::new_line's)
fn one_d_cells(tx: &mut Transaction, b: [f64; 2], e: [f64; 2]) -> Vec<Cell> {
    let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
    let len = math::sqrt(dx * dx + dy * dy);
    alloc::vec![
        cform(tx, "PinX", "(BeginX+EndX)/2", (b[0] + e[0]) * 0.5),
        cform(tx, "PinY", "(BeginY+EndY)/2", (b[1] + e[1]) * 0.5),
        cform(tx, "Width", "SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)", len),
        cnum(tx, "Height", 0.0),
        cform(tx, "LocPinX", "Width*0.5", len * 0.5),
        cform(tx, "LocPinY", "Height*0.5", 0.0),
        cform(tx, "Angle", "ATAN2(EndY-BeginY,EndX-BeginX)", math::atan2(dy, dx)),
        cnum(tx, "BeginX", b[0]),
        cnum(tx, "BeginY", b[1]),
        cnum(tx, "EndX", e[0]),
        cnum(tx, "EndY", e[1]),
    ]
}

// a page point in the local coordinates of a 1-D shape from b (x along the chord, y left of it)
fn chord_local(b: [f64; 2], angle: f64, p: [f64; 2]) -> [f64; 2] {
    let (s, c) = math::sincos(angle);
    let (dx, dy) = (p[0] - b[0], p[1] - b[1]);
    [dx * c + dy * s, -dx * s + dy * c]
}

// the NURBSTo row of a spline: the inner points' texts in E through `xy`, the end, the knots
fn nurbs_row(tx: &mut Transaction, sp: &Spline, types: (u8, u8), xy: &dyn Fn(usize) -> (String, String), end: (Cell, Cell)) -> Row {
    let n = sp.points.len() - 1;
    let last = sp.knots[n + 1];
    let mut e = format!("NURBS({}, {}, {}, {}", fnum(last), sp.degree, types.0, types.1);
    for i in 1..n {
        let (x, y) = xy(i);
        e.push_str(&format!(", {x},{y},{},1", fnum(sp.knots[i])));
    }
    e.push(')');
    let ef = tx.intern(&e);
    // the value is the formula's text until the engine computes it (numbers only then)
    let mut cells = alloc::vec![end.0, end.1];
    cells.push(Cell::constant(names::A, Value::Num(sp.knots[n]), None));
    cells.push(Cell::constant(names::B, Value::Num(1.0), None));
    cells.push(Cell::constant(names::C, Value::Num(sp.knots[0]), None));
    cells.push(Cell::constant(names::D, Value::Num(1.0), None));
    cells.push(Cell { name: names::E, value: Value::Text(ef), formula: Formula::Expr(ef), unit: None, error: None, stale: true });
    row(2, names::NURBS_TO, cells)
}

/// a freeform shape from a stroke (page points in drawing units): its corners within `tol`
/// (simplify) smoothed into a NURBS curve; a stroke whose end lies within `close` of its start
/// (and that has a few corners) is a closed, filled 2-D shape, else a 1-D shape whose ends glue
/// like a line's (Target::Shape is refused, as for lines) -> its handle.  A stroke without
/// length is refused (Invalid)
#[allow(clippy::too_many_arguments)]
pub fn new_freeform(tx: &mut Transaction, eng: &mut Engine, owner: Owner, stroke: &[[f64; 2]], tol: f64, close: f64, begin: Target, end: Target) -> Result<ShapeRef, EditError> {
    tx.doc().tree(owner).ok_or(EditError::NoPage)?;
    let mut tol = tol.max(1e-9);
    let mut pts = simplify(stroke, tol);
    while pts.len() > MAX_POINTS {
        tol *= 1.5;
        pts = simplify(stroke, tol);
    }
    if pts.len() < 2 {
        return Err(EditError::Invalid);
    }
    let (b, e) = (pts[0], pts[pts.len() - 1]);
    let closed = pts.len() >= 4 && dist(b, e) <= close;
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = default_styles(tx);
    if closed {
        let last = pts.len() - 1;
        pts[last] = b;
        let mut bx = [b[0], b[1], b[0], b[1]];
        for q in &pts {
            bx = [bx[0].min(q[0]), bx[1].min(q[1]), bx[2].max(q[0]), bx[3].max(q[1])];
        }
        let (w, h) = (bx[2] - bx[0], bx[3] - bx[1]);
        if !(w > 1e-9 && h > 1e-9) {
            return Err(EditError::Invalid);
        }
        let fr = |q: [f64; 2]| [(q[0] - bx[0]) / w, (q[1] - bx[1]) / h];
        let sp = Spline::of(&pts).ok_or(EditError::Invalid)?;
        let fx = tx.intern("Width*0.5");
        let fy = tx.intern("Height*0.5");
        let f = |name: Sym, v: f64, s: Sym| Cell { name, value: Value::Num(v), formula: Formula::Expr(s), unit: None, error: None, stale: false };
        d.sheet.cells = alloc::vec![
            Cell::constant(names::PIN_X, Value::Num((bx[0] + bx[2]) * 0.5), None),
            Cell::constant(names::PIN_Y, Value::Num((bx[1] + bx[3]) * 0.5), None),
            Cell::constant(names::WIDTH, Value::Num(w), None),
            Cell::constant(names::HEIGHT, Value::Num(h), None),
            f(names::LOC_PIN_X, w * 0.5, fx),
            f(names::LOC_PIN_Y, h * 0.5, fy),
            Cell::constant(names::ANGLE, Value::Num(0.0), None),
        ];
        let s0 = fr(b);
        let x0 = cform(tx, "X", &format!("Width*{}", fnum(s0[0])), s0[0] * w);
        let y0 = cform(tx, "Y", &format!("Height*{}", fnum(s0[1])), s0[1] * h);
        let first = row(1, names::MOVE_TO, alloc::vec![x0, y0]);
        let x1 = cform(tx, "X", &format!("Width*{}", fnum(s0[0])), s0[0] * w);
        let y1 = cform(tx, "Y", &format!("Height*{}", fnum(s0[1])), s0[1] * h);
        let rel: Vec<[f64; 2]> = sp.points.iter().map(|q| fr(*q)).collect();
        let xy = |i: usize| (fnum(rel[i][0]), fnum(rel[i][1]));
        let second = nurbs_row(tx, &sp, (0, 0), &xy, (x1, y1));
        d.sheet.sections.push(section(false, alloc::vec![first, second]));
        let s = tx.add_shape_with(owner, None, d)?;
        eng.recalc(tx, &[SheetRef::Shape(s)])?;
        return Ok(s);
    }
    let len = dist(b, e);
    if !(len > 1e-9) {
        return Err(EditError::Invalid);
    }
    let angle = math::atan2(e[1] - b[1], e[0] - b[0]);
    d.sheet.cells = one_d_cells(tx, b, e);
    let local: Vec<[f64; 2]> = pts.iter().map(|q| chord_local(b, angle, *q)).collect();
    let first = row(1, names::MOVE_TO, alloc::vec![cform(tx, "X", "Width*0", 0.0), cform(tx, "Y", "Height*0.5", 0.0)]);
    let end_cells = (cform(tx, "X", "Width*1", len), cform(tx, "Y", "Height*0.5", 0.0));
    let second = if local.len() == 2 {
        row(2, names::LINE_TO, alloc::vec![end_cells.0, end_cells.1])
    } else {
        let sp = Spline::of(&local).ok_or(EditError::Invalid)?;
        // x as a fraction of Width (type 0), y in local inches as Width*k (type 1)
        let xy = |i: usize| (fnum(local[i][0] / len), format!("Width*{}", fnum(local[i][1] / len)));
        nurbs_row(tx, &sp, (0, 1), &xy, end_cells)
    };
    d.sheet.sections.push(section(true, alloc::vec![first, second]));
    let s = tx.add_shape_with(owner, None, d)?;
    glue::finish_new(tx, eng, s, (b, begin), (e, end))
}

/// a quarter-ellipse arc from `begin` to `end` (page points with their glue targets; lines
/// glue statically, Target::Shape is refused), leaving the begin horizontally or vertically
/// (quarter_arc) -> its handle.  Begin and end in one place are refused (Invalid)
pub fn new_arc(tx: &mut Transaction, eng: &mut Engine, owner: Owner, begin: ([f64; 2], Target), end: ([f64; 2], Target), horizontal: bool) -> Result<ShapeRef, EditError> {
    tx.doc().tree(owner).ok_or(EditError::NoPage)?;
    let (b, e) = (begin.0, end.0);
    let len = dist(b, e);
    if !(len > 1e-9) {
        return Err(EditError::Invalid);
    }
    let arc = quarter_arc(b, e, horizontal);
    let angle = math::atan2(e[1] - b[1], e[0] - b[0]);
    let mid = chord_local(b, angle, arc.mid);
    // a straight drag (one radius 0): the middle on the chord, a circle's ratio (a line)
    let ratio = if arc.rx > 1e-9 && arc.ry > 1e-9 { arc.rx / arc.ry } else { 1.0 };
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = default_styles(tx);
    d.sheet.cells = one_d_cells(tx, b, e);
    let first = row(1, names::MOVE_TO, alloc::vec![cform(tx, "X", "Width*0", 0.0), cform(tx, "Y", "Height*0.5", 0.0)]);
    let cells = alloc::vec![
        cform(tx, "X", "Width*1", len),
        cform(tx, "Y", "Height*0.5", 0.0),
        cform(tx, "A", &format!("Width*{}", fnum(mid[0] / len)), mid[0]),
        cform(tx, "B", &format!("Width*{}", fnum(mid[1] / len)), mid[1]),
        // the major axis is the page's x axis: in local coordinates -Angle
        cnum(tx, "C", -angle),
        cnum(tx, "D", ratio),
    ];
    let second = row(2, names::ELLIPTICAL_ARC_TO, cells);
    d.sheet.sections.push(section(true, alloc::vec![first, second]));
    let s = tx.add_shape_with(owner, None, d)?;
    glue::finish_new(tx, eng, s, begin, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * (1.0 + b.abs())
    }

    #[test]
    fn simplify_keeps_corners() {
        // an L with noise: the corner stays, the noise goes
        let mut pts = Vec::new();
        for i in 0..=20 {
            pts.push([i as f64 * 0.1, if i % 2 == 0 { 0.0 } else { 0.004 }]);
        }
        for i in 1..=20 {
            pts.push([2.0, i as f64 * 0.1]);
        }
        let s = simplify(&pts, 0.01);
        assert_eq!(s, alloc::vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]]);
        // every point stays within tol of the result
        let s = simplify(&pts, 0.001);
        for q in &pts {
            let d = s.windows(2).map(|w| seg_dist(*q, w[0], w[1])).fold(f64::MAX, f64::min);
            assert!(d <= 0.001 + 1e-12);
        }
        // repeated points and a single point
        assert_eq!(simplify(&[[1.0, 1.0], [1.0, 1.0]], 0.1), alloc::vec![[1.0, 1.0]]);
    }

    #[test]
    fn spline_is_clamped_and_smooth() {
        let pts = [[0.0, 0.0], [1.0, 1.0], [2.0, 0.0], [3.0, 1.0], [4.0, 0.0]];
        let sp = Spline::of(&pts).unwrap();
        assert_eq!(sp.degree, 3);
        assert_eq!(sp.knots, alloc::vec![0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 2.0]);
        assert_eq!(sp.at(0.0), [0.0, 0.0]);
        assert_eq!(sp.at(1.0), [4.0, 0.0]);
        // inside the control polygon's hull, symmetric about x = 2
        let m = sp.at(0.5);
        assert!(close(m[0], 2.0) && m[1] > 0.0 && m[1] < 1.0, "{m:?}");
        // three points: a quadratic, two: a line
        assert_eq!(Spline::of(&pts[..3]).unwrap().knots, alloc::vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        assert_eq!(Spline::of(&pts[..2]).unwrap().degree, 1);
        assert!(Spline::of(&pts[..1]).is_none());
    }

    #[test]
    fn quarter_arcs_bow_with_the_start() {
        let h = core::f64::consts::FRAC_1_SQRT_2;
        // right and down, leaving horizontally: the centre below the start
        let a = quarter_arc([0.0, 2.0], [3.0, 0.0], true);
        assert_eq!(a.centre, [0.0, 0.0]);
        assert_eq!((a.rx, a.ry), (3.0, 2.0));
        assert!(close(a.mid[0], 3.0 * h) && close(a.mid[1], 2.0 * h));
        // the same drag leaving vertically: the centre right of the start, bowing the other way
        let b = quarter_arc([0.0, 2.0], [3.0, 0.0], false);
        assert_eq!(b.centre, [3.0, 2.0]);
        assert!(close(b.mid[0], 3.0 - 3.0 * h) && close(b.mid[1], 2.0 - 2.0 * h));
        // the middle lies on the ellipse
        for arc in [a, b] {
            let (u, v) = ((arc.mid[0] - arc.centre[0]) / arc.rx, (arc.mid[1] - arc.centre[1]) / arc.ry);
            assert!(close(u * u + v * v, 1.0));
        }
    }
}
