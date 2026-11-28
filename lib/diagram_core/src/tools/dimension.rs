// tools::dimension -- Maßlinien (AS-11): the ends of a dimension shape on points and shapes
//
//   Kind                     Horizontal, Vertical, Aligned (what the dimension shape measures:
//                            its master shows the horizontal or vertical distance of its ends or
//                            its own length)
//   Anchor                   where an end goes: a free point, a connection point of a shape
//                            (static glue), a point of a shape's box (Width and Height fractions)
//   anchor_at(doc, s, frac)  the connection point at that point of the box, else the box point
//   point(doc, &anchor)      where it is on the page now
//   of_shape(doc, s, kind)   the shape's own extent: its left and right side (bottom and top)
//   between(doc, a, b, kind) between two shapes: the sides that face each other (horizontal:
//                            the left shape's right side and the right shape's left side;
//                            vertical: the lower one's top and the upper one's bottom; aligned:
//                            the nearest pair)
//   attach(tx, eng, dim, end, &anchor)    one end there, glued or tied
//   place(tx, eng, dim, kind, &begin, &end, clear)   both ends; the dimension line (the control
//                            row "Line" of the master, if it has one) `clear` beyond the shapes:
//                            above them (horizontal), left of them (vertical)
//
// How an end follows (so it does in Visio too, the cells are Visio's own):
// * a connection point: static glue as glue::glue writes it -- BeginX = PAR(PNT(Sheet.N!
//   Connections.Xk,Sheet.N!Connections.Yk)), BegTrigger = _XFTRIGGER(Sheet.N!EventXFMod) and a
//   Connect entry;
// * any other point of a shape: the same formula on the box, BeginX = PAR(PNT(Sheet.N!Width*fx,
//   Sheet.N!Height*fy)) with the trigger, without a Connect entry (Visio has no Connect for it; the
//   formula alone moves the end, as PAR reads the shape's transform);
// * a free point: a constant.
// The candidates of a shape are its connection points (Type 0 and 2; outward ones are no glue
// target), a shape without any offers the middles of its four sides.

use alloc::format;
use alloc::vec::Vec;

use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, End, Target};
use crate::ops;
use crate::sheet::CellAddr;

/// what a dimension measures
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Kind {
    /// the horizontal distance of its ends
    Horizontal,
    /// the vertical distance of its ends
    Vertical,
    /// its length
    Aligned,
}

/// where an end of a dimension goes
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Anchor {
    /// a point of the page
    Free([f64; 2]),
    /// connection point `row` (IX) of a shape: static glue
    Point {
        /// the shape
        shape: ShapeRef,
        /// IX of its Connection row
        row: u32,
    },
    /// a point of a shape's box: fractions of Width and Height in its local coordinates
    Box {
        /// the shape
        shape: ShapeRef,
        /// x / Width, y / Height
        at: [f64; 2],
    },
}

impl Anchor {
    /// the shape it is on (None: a free point)
    pub fn shape(&self) -> Option<ShapeRef> {
        match *self {
            Anchor::Free(_) => None,
            Anchor::Point { shape, .. } | Anchor::Box { shape, .. } => Some(shape),
        }
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    crate::formula::math::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

/// where an anchor is on the page now
pub fn point(doc: &Document, a: &Anchor) -> [f64; 2] {
    match *a {
        Anchor::Free(p) => p,
        Anchor::Point { shape, row } => {
            glue::connection_points(doc, shape).into_iter().find(|c| c.row == row).map_or_else(|| ops::frame(doc, shape).centre(), |c| c.at)
        }
        Anchor::Box { shape, at } => {
            let f = ops::frame(doc, shape);
            f.at([f.w * at[0], f.h * at[1]])
        }
    }
}

// the connection points that take glue
fn points(doc: &Document, s: ShapeRef) -> Vec<glue::ConnPoint> {
    glue::connection_points(doc, s).into_iter().filter(|c| c.kind != 1).collect()
}

/// the connection point at the box point `frac` (fractions of Width and Height), else the box
/// point itself
pub fn anchor_at(doc: &Document, s: ShapeRef, frac: [f64; 2]) -> Anchor {
    let f = ops::frame(doc, s);
    let p = f.at([f.w * frac[0], f.h * frac[1]]);
    let tol = 1e-4 * (1.0 + f.w.abs().max(f.h.abs()));
    match points(doc, s).into_iter().find(|c| dist(c.at, p) <= tol) {
        Some(c) => Anchor::Point { shape: s, row: c.row },
        None => Anchor::Box { shape: s, at: frac },
    }
}

// what a shape offers an end: its connection points, else the middles of its sides
fn candidates(doc: &Document, s: ShapeRef) -> Vec<Anchor> {
    let pts = points(doc, s);
    if !pts.is_empty() {
        return pts.into_iter().map(|c| Anchor::Point { shape: s, row: c.row }).collect();
    }
    [[0.0, 0.5], [1.0, 0.5], [0.5, 0.0], [0.5, 1.0]].into_iter().map(|at| Anchor::Box { shape: s, at }).collect()
}

/// a shape's own extent: left and right side (Horizontal, Aligned: along its width), bottom and
/// top (Vertical); the first end left of (below) the second
pub fn of_shape(doc: &Document, s: ShapeRef, kind: Kind) -> (Anchor, Anchor) {
    let (a, b) = match kind {
        Kind::Vertical => (anchor_at(doc, s, [0.5, 0.0]), anchor_at(doc, s, [0.5, 1.0])),
        _ => (anchor_at(doc, s, [0.0, 0.5]), anchor_at(doc, s, [1.0, 0.5])),
    };
    let (pa, pb) = (point(doc, &a), point(doc, &b));
    let swap = match kind {
        Kind::Vertical => pa[1] > pb[1],
        _ => pa[0] > pb[0],
    };
    if swap { (b, a) } else { (a, b) }
}

// the candidate of `list` that is farthest in `dir` (ties: the one nearest to `towards` across)
fn extreme(doc: &Document, list: &[Anchor], dir: [f64; 2], towards: [f64; 2]) -> Anchor {
    let mut best: Option<(f64, f64, Anchor)> = None;
    for a in list {
        let p = point(doc, a);
        let along = p[0] * dir[0] + p[1] * dir[1];
        let across = ((p[0] - towards[0]) * dir[1] - (p[1] - towards[1]) * dir[0]).abs();
        let better = match best {
            None => true,
            Some((b_along, b_across, _)) => along > b_along + 1e-9 || ((along - b_along).abs() <= 1e-9 && across < b_across - 1e-9),
        };
        if better {
            best = Some((along, across, *a));
        }
    }
    best.map_or(Anchor::Free(towards), |b| b.2)
}

/// between two shapes (see the file header); the first end left of (below) the second
pub fn between(doc: &Document, a: ShapeRef, b: ShapeRef, kind: Kind) -> (Anchor, Anchor) {
    let (ca, cb) = (ops::frame(doc, a).centre(), ops::frame(doc, b).centre());
    let (la, lb) = (candidates(doc, a), candidates(doc, b));
    match kind {
        Kind::Horizontal => {
            let ((l, ll), (r, lr)) = if ca[0] <= cb[0] { ((ca, &la), (cb, &lb)) } else { ((cb, &lb), (ca, &la)) };
            (extreme(doc, ll, [1.0, 0.0], r), extreme(doc, lr, [-1.0, 0.0], l))
        }
        Kind::Vertical => {
            let ((d, ld), (u, lu)) = if ca[1] <= cb[1] { ((ca, &la), (cb, &lb)) } else { ((cb, &lb), (ca, &la)) };
            (extreme(doc, ld, [0.0, 1.0], u), extreme(doc, lu, [0.0, -1.0], d))
        }
        Kind::Aligned => {
            let mut best: Option<(f64, Anchor, Anchor)> = None;
            for x in &la {
                for y in &lb {
                    let d = dist(point(doc, x), point(doc, y));
                    if best.is_none_or(|b| d < b.0 - 1e-9) {
                        best = Some((d, *x, *y));
                    }
                }
            }
            let (_, x, y) = best.unwrap_or((0.0, Anchor::Free(ca), Anchor::Free(cb)));
            if point(doc, &x)[0] <= point(doc, &y)[0] { (x, y) } else { (y, x) }
        }
    }
}

fn end_cells(end: End) -> (&'static str, &'static str, &'static str) {
    match end {
        End::Begin => ("BeginX", "BeginY", "BegTrigger"),
        End::End => ("EndX", "EndY", "EndTrigger"),
    }
}

/// put an end of dimension shape `dim` (1-D) on `anchor`: glued to a connection point, tied to a
/// box point by formula, or free
pub fn attach(tx: &mut Transaction, eng: &mut Engine, dim: ShapeRef, end: End, anchor: &Anchor) -> Result<(), EditError> {
    let p = point(tx.doc(), anchor);
    match *anchor {
        Anchor::Free(_) => glue::glue(tx, eng, dim, end, p, Target::None),
        Anchor::Point { shape, row } => glue::glue(tx, eng, dim, end, p, Target::Point { shape, row }),
        Anchor::Box { shape, at } => {
            // free first: the end's Connect entries go, the cells are plain
            glue::glue(tx, eng, dim, end, p, Target::None)?;
            let id = tx.doc().shape(shape).ok_or(EditError::NoShape)?.id();
            if shape.owner != dim.owner || shape == dim {
                return Err(EditError::Invalid);
            }
            let f = format!("PAR(PNT(Sheet.{id}!Width*{},Sheet.{id}!Height*{}))", at[0], at[1]);
            let (cx, cy, ct) = end_cells(end);
            let sheet = SheetRef::Shape(dim);
            let (ax, ay, at_) = (tx.single(cx), tx.single(cy), tx.single(ct));
            tx.intern("EventXFMod");
            tx.set_formula(sheet, &ax, &f)?;
            tx.set_formula(sheet, &ay, &f)?;
            tx.set_formula(sheet, &at_, &format!("_XFTRIGGER(Sheet.{id}!EventXFMod)"))?;
            eng.recalc_cells(tx, &[(sheet, ax), (sheet, ay), (sheet, at_)])?;
            glue::update(tx, eng, &[dim])
        }
    }
}

// the box around the shapes the anchors are on and the anchors' points
fn reach(doc: &Document, anchors: &[&Anchor]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for a in anchors {
        let p = point(doc, a);
        let mut add = |q: [f64; 4]| {
            b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[2]), b[3].max(q[3])];
        };
        add([p[0], p[1], p[0], p[1]]);
        if let Some(s) = a.shape() {
            if let Some(q) = ops::bounds(doc, &[s]) {
                add(q);
            }
        }
    }
    b
}

/// both ends of `dim` on their anchors; the dimension line `clear` (drawing units) above the
/// shapes (Horizontal) or left of them (Vertical), when the master has the control row "Line"
pub fn place(tx: &mut Transaction, eng: &mut Engine, dim: ShapeRef, kind: Kind, begin: &Anchor, end: &Anchor, clear: f64) -> Result<(), EditError> {
    attach(tx, eng, dim, End::Begin, begin)?;
    attach(tx, eng, dim, End::End, end)?;
    if kind == Kind::Aligned {
        return Ok(());
    }
    let (line, b) = {
        let doc = tx.doc();
        let line = ops::controls(doc, dim).into_iter().find(|c| c.name.eq_ignore_ascii_case("Line"));
        (line, reach(doc, &[begin, end]))
    };
    let Some(c) = line else { return Ok(()) };
    let to = match kind {
        Kind::Horizontal => [c.at[0], b[3] + clear],
        _ => [b[0] - clear, c.at[1]],
    };
    ops::set_control(tx, eng, dim, c.row, to)
}

/// the formulas of BeginX and EndX of a 1-D shape (None: a constant), for checks
pub fn end_formulas(doc: &Document, dim: ShapeRef) -> (Option<alloc::string::String>, Option<alloc::string::String>) {
    let f = |n: &str| {
        let c = doc.sym(n)?;
        match doc.resolve(SheetRef::Shape(dim), &CellAddr::single(c)).formula() {
            crate::cell::Formula::Expr(f) => Some(alloc::string::String::from(doc.str(f))),
            _ => None,
        }
    };
    (f("BeginX"), f("EndX"))
}
