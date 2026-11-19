// ops -- the editing gestures on shapes: move, size, turn, flip, pin, control points, vertices,
// groups, z-order, new shapes, text, copy and paste (JD7: SP-03..07, TX-01, TX-03, DM-02)
//
//   frame(doc, shape) -> Frame          where a shape is: local -> page, Width, Height
//   Frame::corners / bounds / centre / grip(gx, gy) / drag(gx, gy, to, keep_aspect)
//   bounds(doc, shapes)                 the box of a selection on the page
//   locks(eng, doc, shape) -> Locks     the protection cells that are set (Locks::MOVE_X ...)
//   move_shapes(tx, eng, shapes, d)     2D: the pin; 1D: BeginX/Y, EndX/Y of the free ends
//   shift_all(tx, eng, owner, d)        every top-level shape, locks or not (the page's origin
//                                       moves: pagesetup::grow, fit_to_drawing)
//   set_frame(tx, eng, shape, &frame)   size and place, same turn (the eight grips)
//   scale_shapes(tx, eng, shapes, from, to)   a selection from one box into another
//   rotate_shapes(tx, eng, shapes, centre, angle) / flip_shapes(tx, eng, shapes, horizontal)
//   set_pin(tx, eng, shape, at)         the pin to another place, the shape stays
//   controls / set_control              the yellow control points (SP-04)
//   vertices / set_vertex / insert_vertex / delete_vertex     Geometry rows (SP-05)
//   group(tx, eng, owner, shapes) / ungroup(tx, eng, group)   (SP-06)
//   reorder(tx, shapes, Z::Front | Back | Forward | Backward) (SP-07)
//   new_shape(tx, eng, owner, Basic::Rectangle | Ellipse | TextBox, a, b)
//   set_text(tx, eng, shape, text) / text_frame / set_text_frame   (TX-01, TX-03)
//   copy(doc, shapes) -> Clip / paste(tx, eng, owner, &clip, offset)
//
// Every position is on the page in drawing units (PinX of a top-level shape as the file has
// it), y up; local coordinates have their origin at the bottom left of the shape's box.  A
// gesture runs inside the caller's transaction and becomes one undo step (DM-02):
//
//   let mut tx = doc.transaction("Verschieben");
//   ops::move_shapes(&mut tx, &mut eng, &sel, [dx, dy])?;
//   let conns = glue::dependents(tx.doc(), owner, &sel);
//   glue::update(&mut tx, &mut eng, &conns)?;
//   let change = tx.commit();            after undo/redo: eng.refresh(&doc, &change.sheets())
//
// How cells are written (as Visio writes them):
// * A value the user gives goes through the rules of a user's edit (Engine::user_set): a GUARD
//   formula refuses it and the cell is left alone (the gesture goes on without it), SETATREF
//   sends it on; the formula engine recomputes what depends on it once per step.
// * A formula of the proportional form keeps its form: a subshape's PinX "Sheet.7!Width*0.25"
//   becomes "Sheet.7!Width*<new k>", a LocPinX "Width*0.5" becomes "Width*<k>", a vertex
//   "Width*1" "Width*<k>".  Anything else becomes a constant.
// * Sizes first, then the pin: the local pin follows its own formula (mostly Width*0.5), and
//   the pin goes where the frame puts the local pin.
// * The protection cells are asked per shape (a formula is evaluated): LockMoveX/Y keep that
//   part of a move away, LockWidth/LockHeight keep a side, LockAspect the proportions,
//   LockRotate leaves a shape out of a turn, LockBegin/LockEnd keep an end of a 1D shape on a
//   resize; what only makes sense unlocked is refused with EditError::Invalid (text with
//   LockTextEdit, vertices with LockVtxEdit, ungrouping with LockGroup).
// * Glued ends of a 1D shape (a Connect entry with FromCell BeginX or EndX) are not moved here:
//   glue::update routes them.
//
// The transform of a shape (MS-VSDX "Shape Transform"; geometry.rs and formula/geom.rs use the
// same): parent = Pin + R(Angle) Flip (local - LocPin), and a subshape continues through its
// groups up to the page.  Flipping a turned shape changes its angle: a mirror M times R(a)
// equals R(-a) times M, so a horizontal flip toggles FlipX and makes the angle 2φ - a + 180°
// (φ the direction of the mirror line in the parent's coordinates, 90° on the page: -a), a
// vertical flip toggles FlipY and makes it 2φ - a.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::{math, Axis, Engine, Input, Outcome, LOCKS};
use crate::geometry::Xform;
use crate::names;
use crate::page::Connect;
use crate::shape::{ShapeData, ShapeKind, StyleRefs, Text};
use crate::sheet::{CellAddr, Row, RowKey, SecKey, Section};
use crate::strings::Sym;
use crate::units::Unit;

mod clip;
mod points;
pub(crate) mod text;

pub use clip::{copy, paste, Clip};
pub use points::{controls, delete_vertex, insert_vertex, set_control, set_vertex, vertices, Control, Vertex};

#[cfg(test)]
mod tests;

const EPS: f64 = 1e-9;

// ---------------------------------------------------------------- frames

/// where a shape is: its local coordinates to the page and its size (a box from (0, 0) to
/// (w, h) in local coordinates; the transform is rigid, flips included)
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Frame {
    /// local -> page
    pub xf: Xform,
    /// Width
    pub w: f64,
    /// Height
    pub h: f64,
}

impl Frame {
    /// an upright box on the page (x0, y0, x1, y1)
    pub fn from_box(b: [f64; 4]) -> Frame {
        Frame { xf: Xform::translate(b[0], b[1]), w: b[2] - b[0], h: b[3] - b[1] }
    }

    /// a local point on the page
    pub fn at(&self, p: [f64; 2]) -> [f64; 2] {
        self.xf.apply(p)
    }

    /// a page point in local coordinates (None: a degenerate transform)
    pub fn local(&self, p: [f64; 2]) -> Option<[f64; 2]> {
        self.xf.inverse().map(|m| m.apply(p))
    }

    /// the corners on the page: (0, 0), (w, 0), (w, h), (0, h)
    pub fn corners(&self) -> [[f64; 2]; 4] {
        [self.at([0.0, 0.0]), self.at([self.w, 0.0]), self.at([self.w, self.h]), self.at([0.0, self.h])]
    }

    /// the upright box around the corners (x0, y0, x1, y1)
    pub fn bounds(&self) -> [f64; 4] {
        let c = self.corners();
        let mut b = [c[0][0], c[0][1], c[0][0], c[0][1]];
        for p in &c[1..] {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        }
        b
    }

    /// the middle of the box on the page
    pub fn centre(&self) -> [f64; 2] {
        self.at([self.w * 0.5, self.h * 0.5])
    }

    /// a grip on the page: gx -1, 0, 1 is the left edge, the middle, the right edge of the
    /// local box, gy -1, 0, 1 the bottom, the middle, the top (a flipped shape has its local
    /// left edge on the right)
    pub fn grip(&self, gx: i8, gy: i8) -> [f64; 2] {
        self.at([self.w * (gx.clamp(-1, 1) + 1) as f64 * 0.5, self.h * (gy.clamp(-1, 1) + 1) as f64 * 0.5])
    }

    /// the frame after grip (gx, gy) is dragged to the page point `to`: the opposite edges
    /// stay, sizes stop at 0; `keep_aspect` (a corner grip, SP-03) scales both sides by the
    /// larger factor and keeps the opposite corner
    pub fn drag(&self, gx: i8, gy: i8, to: [f64; 2], keep_aspect: bool) -> Frame {
        let Some(q) = self.local(to) else { return *self };
        let (mut x0, mut x1, mut y0, mut y1) = (0.0, self.w, 0.0, self.h);
        match gx {
            -1 => x0 = q[0].min(x1),
            1 => x1 = q[0].max(x0),
            _ => {}
        }
        match gy {
            -1 => y0 = q[1].min(y1),
            1 => y1 = q[1].max(y0),
            _ => {}
        }
        if keep_aspect && gx != 0 && gy != 0 && self.w > EPS && self.h > EPS {
            let s = ((x1 - x0) / self.w).max((y1 - y0) / self.h);
            let (w, h) = (self.w * s, self.h * s);
            if gx < 0 {
                x0 = x1 - w;
            } else {
                x1 = x0 + w;
            }
            if gy < 0 {
                y0 = y1 - h;
            } else {
                y1 = y0 + h;
            }
        }
        Frame { xf: Xform::translate(x0, y0).then(&self.xf), w: x1 - x0, h: y1 - y0 }
    }
}

/// where a shape is on its page (cached values, nothing recomputed)
pub fn frame(doc: &Document, shape: ShapeRef) -> Frame {
    let s = SheetRef::Shape(shape);
    Frame { xf: doc.page_xform(shape), w: val(doc, s, names::WIDTH), h: val(doc, s, names::HEIGHT) }
}

/// the upright box of shapes on the page: the corners of 2D shapes, the ends of 1D shapes
pub fn bounds(doc: &Document, shapes: &[ShapeRef]) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    let mut add = |p: [f64; 2]| {
        b = Some(match b {
            None => [p[0], p[1], p[0], p[1]],
            Some(b) => [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
        });
    };
    for &s in shapes {
        if doc.shape(s).is_none() {
            continue;
        }
        if one_d(doc, s) {
            let par = parent_xf(doc, s);
            let (bp, ep) = ends(doc, s);
            add(par.apply(bp));
            add(par.apply(ep));
        } else {
            for c in frame(doc, s).corners() {
                add(c);
            }
        }
    }
    b
}

// ---------------------------------------------------------------- locks

/// the protection cells of a shape that are set (SS-15), one bit per name of formula::LOCKS
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Locks(pub u32);

impl Locks {
    pub const WIDTH: u32 = 1 << 0;
    pub const HEIGHT: u32 = 1 << 1;
    pub const ASPECT: u32 = 1 << 2;
    pub const MOVE_X: u32 = 1 << 3;
    pub const MOVE_Y: u32 = 1 << 4;
    pub const ROTATE: u32 = 1 << 5;
    pub const BEGIN: u32 = 1 << 6;
    pub const END: u32 = 1 << 7;
    pub const DELETE: u32 = 1 << 8;
    pub const SELECT: u32 = 1 << 9;
    pub const FORMAT: u32 = 1 << 10;
    pub const TEXT_EDIT: u32 = 1 << 11;
    pub const VTX_EDIT: u32 = 1 << 12;
    pub const CROP: u32 = 1 << 13;
    pub const GROUP: u32 = 1 << 14;
    pub const CALC_WH: u32 = 1 << 15;

    /// whether a lock (one of the constants) is set
    pub fn has(self, lock: u32) -> bool {
        self.0 & lock != 0
    }

    /// the bit of a lock cell's name ("LockWidth" ...; 0: not a lock cell)
    pub fn bit(name: &str) -> u32 {
        LOCKS.iter().position(|n| *n == name).map_or(0, |i| 1 << i)
    }
}

/// the protection cells of a shape that are set, with inheritance and formulas
pub fn locks(eng: &mut Engine, doc: &Document, shape: ShapeRef) -> Locks {
    Locks(eng.locks(doc, SheetRef::Shape(shape)).iter().fold(0, |m, n| m | Locks::bit(n)))
}

// one lock cell: a formula is evaluated, a plain value read
fn locked(eng: &mut Engine, doc: &Document, shape: ShapeRef, name: &str) -> bool {
    let Some(c) = doc.sym(name) else { return false };
    let sheet = SheetRef::Shape(shape);
    let a = CellAddr::single(c);
    let r = doc.resolve(sheet, &a);
    match r.formula() {
        Formula::Expr(_) => match eng.evaluate(doc, sheet, &a) {
            Outcome::Value(v) => v.num(Axis::X).is_ok_and(|x| x != 0.0),
            Outcome::Kept(_) => r.num().is_some_and(|x| x != 0.0),
        },
        _ => r.num().is_some_and(|x| x != 0.0),
    }
}

// ---------------------------------------------------------------- writing cells

// what a gesture writes into a cell
#[derive(Clone, Debug)]
pub(crate) enum Put {
    // a value in internal units (the unit it is shown in stays)
    Num(f64),
    // a formula (its value comes from the recalculation)
    Formula(String),
}

// a user's changes of cells, recomputed together
pub(crate) struct Writer {
    cells: Vec<(SheetRef, CellAddr)>,
}

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer { cells: Vec::new() }
    }

    // write one cell as a user's edit -> false when GUARD refused it (the cell stays)
    pub(crate) fn put(&mut self, tx: &mut Transaction, eng: &mut Engine, sheet: SheetRef, addr: CellAddr, v: Put) -> Result<bool, EditError> {
        let r = tx.doc().resolve(sheet, &addr);
        let mut unit = r.unit();
        if unit.is_none() && (addr.cell == names::ANGLE || addr.cell == names::TXT_ANGLE) {
            unit = Some(Unit::Degrees);
        }
        let redirect = match r.formula() {
            Formula::Expr(f) => text::calls(tx.doc().str(f), "SETATREF"),
            _ => false,
        };
        if redirect {
            let input = match &v {
                Put::Num(x) => Input::Value(Value::Num(*x), unit),
                Put::Formula(f) => Input::Formula(f),
            };
            return match eng.user_set(tx, sheet, &addr, input) {
                Ok(_) => Ok(true),
                Err(EditError::Guarded) => Ok(false),
                Err(e) => Err(e),
            };
        }
        let res = match v {
            Put::Num(x) => tx.set_constant(sheet, &addr, Value::Num(x), unit),
            Put::Formula(f) => tx.set_formula(sheet, &addr, &f),
        };
        match res {
            Ok(()) => {
                self.cells.push((sheet, addr));
                Ok(true)
            }
            Err(EditError::Guarded) => Ok(false),
            Err(e) => Err(e),
        }
    }

    // several cells of one sheet -> whether all were written
    pub(crate) fn put_all(&mut self, tx: &mut Transaction, eng: &mut Engine, sheet: SheetRef, sets: Vec<(CellAddr, Put)>) -> Result<bool, EditError> {
        let mut all = true;
        for (a, v) in sets {
            all &= self.put(tx, eng, sheet, a, v)?;
        }
        Ok(all)
    }

    // recompute what the written cells change
    pub(crate) fn finish(&mut self, tx: &mut Transaction, eng: &mut Engine) -> Result<(), EditError> {
        if !self.cells.is_empty() {
            let cells = core::mem::take(&mut self.cells);
            eng.recalc_cells(tx, &cells)?;
        }
        Ok(())
    }
}

// the shape a Sheet.N of a formula of `sheet` names, the formula coming from `source`: a
// formula inherited from a master names the master's shape N, which the instance holds as the
// subshape with MasterShape = N (formula/eval.rs does the same)
pub(crate) fn shape_of_id(doc: &Document, sheet: SheetRef, source: Option<SheetRef>, n: u32) -> Option<ShapeRef> {
    let SheetRef::Shape(ctx) = sheet else { return None };
    if let Some(SheetRef::Shape(src)) = source {
        if let (Owner::Master(m), true) = (src.owner, src.owner != ctx.owner) {
            let mid = doc.master(m)?.header.id;
            let tree = doc.tree(ctx.owner)?;
            let mut cur = Some(ctx.slot);
            let mut root = None;
            while let Some(s) = cur {
                if tree.get(s)?.header.master == Some(mid) {
                    root = Some(s);
                    break;
                }
                cur = tree.parent(s);
            }
            let root = root?;
            let ms = doc.master(m)?;
            if ms.master_shape().and_then(|slot| ms.shapes.id(slot)) == Some(n) {
                return Some(ShapeRef { owner: ctx.owner, slot: root });
            }
            let mut stack = alloc::vec![root];
            while let Some(s) = stack.pop() {
                if s != root && tree.get(s).is_some_and(|d| d.header.master_shape == Some(n)) {
                    return Some(ShapeRef { owner: ctx.owner, slot: s });
                }
                stack.extend(tree.children(s).iter().copied());
            }
            return None;
        }
    }
    doc.shape_by_id(ctx.owner, n)
}

// the value to write for a cell that gets `v`: the proportional form ([Sheet.N!]Width*k,
// Height*k) when its formula has it, else the value
pub(crate) fn keep_form(doc: &Document, sheet: SheetRef, addr: &CellAddr, v: f64) -> Put {
    let r = doc.resolve(sheet, addr);
    if let Formula::Expr(f) = r.formula() {
        if let Some((sid, height, _)) = text::proportional(doc.str(f)) {
            let (base, id) = match sid {
                None => (Some(sheet), None),
                Some(n) => match shape_of_id(doc, sheet, r.source, n) {
                    Some(s) => (Some(SheetRef::Shape(s)), doc.shape(s).map(|x| x.id())),
                    None => (None, None),
                },
            };
            if let Some(b) = base {
                let size = val(doc, b, if height { names::HEIGHT } else { names::WIDTH });
                if size.abs() > EPS && (sid.is_none() || id.is_some()) {
                    let cell = if height { "Height" } else { "Width" };
                    let k = text::num(v / size);
                    return Put::Formula(match id {
                        Some(id) => format!("Sheet.{id}!{cell}*{k}"),
                        None => format!("{cell}*{k}"),
                    });
                }
            }
        }
    }
    Put::Num(v)
}

// ---------------------------------------------------------------- small helpers

pub(crate) fn val(doc: &Document, sheet: SheetRef, cell: Sym) -> f64 {
    doc.resolve(sheet, &CellAddr::single(cell)).num().unwrap_or(0.0)
}

pub(crate) fn single(cell: Sym) -> CellAddr {
    CellAddr::single(cell)
}

// whether a shape is 1D (has BeginX, IS1D of the engine)
pub(crate) fn one_d(doc: &Document, shape: ShapeRef) -> bool {
    doc.resolve(SheetRef::Shape(shape), &single(names::BEGIN_X)).exists()
}

// BeginX/Y and EndX/Y (the parent's coordinates)
pub(crate) fn ends(doc: &Document, shape: ShapeRef) -> ([f64; 2], [f64; 2]) {
    let s = SheetRef::Shape(shape);
    ([val(doc, s, names::BEGIN_X), val(doc, s, names::BEGIN_Y)], [val(doc, s, names::END_X), val(doc, s, names::END_Y)])
}

// the parent's local coordinates to the page (the page: identity)
pub(crate) fn parent_xf(doc: &Document, shape: ShapeRef) -> Xform {
    doc.shape(shape).and_then(|s| s.parent()).map_or(Xform::IDENTITY, |p| doc.page_xform(p))
}

pub(crate) fn connects(doc: &Document, owner: Owner) -> &[Connect] {
    match owner {
        Owner::Page(p) => doc.page(p).map_or(&[][..], |x| &x.connects),
        Owner::Master(m) => doc.master(m).map_or(&[][..], |x| &x.connects),
    }
}

// whether the begin and the end of a 1D shape are glued (Connect entries from it)
pub(crate) fn glued_ends(doc: &Document, shape: ShapeRef) -> (bool, bool) {
    let Some(id) = doc.shape(shape).map(|s| s.id()) else { return (false, false) };
    let mut out = (false, false);
    for c in connects(doc, shape.owner).iter().filter(|c| c.from_sheet == id) {
        match c.from_cell {
            Some(n) if n == names::BEGIN_X => out.0 = true,
            Some(n) if n == names::END_X => out.1 = true,
            _ => {}
        }
    }
    out
}

// the shapes of a list without repeats and without those inside another one of the list
pub(crate) fn outermost(doc: &Document, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    let set: BTreeSet<ShapeRef> = shapes.iter().copied().filter(|&s| doc.shape(s).is_some()).collect();
    let mut seen = BTreeSet::new();
    shapes
        .iter()
        .copied()
        .filter(|&s| {
            if !set.contains(&s) || !seen.insert(s) {
                return false;
            }
            let mut up = doc.shape(s).and_then(|x| x.parent());
            while let Some(p) = up {
                if set.contains(&p) {
                    return false;
                }
                up = doc.shape(p).and_then(|x| x.parent());
            }
            true
        })
        .collect()
}

// an angle in (-180°, 180°], tiny ones 0
pub(crate) fn norm_angle(a: f64) -> f64 {
    let tau = core::f64::consts::TAU;
    let mut r = a - tau * math::floor(a / tau);
    if r > core::f64::consts::PI {
        r -= tau;
    }
    if r.abs() < 1e-12 { 0.0 } else { r }
}

fn rotate_about(p: [f64; 2], c: [f64; 2], s: f64, co: f64) -> [f64; 2] {
    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
    [c[0] + dx * co - dy * s, c[1] + dx * s + dy * co]
}

fn pt_sets(x: Sym, y: Sym, p: [f64; 2], doc: &Document, sheet: SheetRef) -> [(CellAddr, Put); 2] {
    [(single(x), keep_form(doc, sheet, &single(x), p[0])), (single(y), keep_form(doc, sheet, &single(y), p[1]))]
}

// ---------------------------------------------------------------- move, size, turn, flip

/// move shapes by `d` (page): a 2D shape's pin, a 1D shape's free ends (glued ones stay for
/// glue::update); a shape inside another one of the list moves with it; LockMoveX/LockMoveY
/// keep that part of the move away
pub fn move_shapes(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], d: [f64; 2]) -> Result<(), EditError> {
    move_by(tx, eng, shapes, d, true)
}

/// move every top-level shape of a page or master by `d`, whatever their locks say: the page's
/// origin moves under the drawing (a page that grows to the left or down, a page fitted to
/// its drawing; pagesetup.rs).  Free 1D ends move, glued ones follow their formulas
pub fn shift_all(tx: &mut Transaction, eng: &mut Engine, owner: Owner, d: [f64; 2]) -> Result<(), EditError> {
    let tops: Vec<ShapeRef> = tx.doc().top_shapes(owner).collect();
    move_by(tx, eng, &tops, d, false)
}

fn move_by(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], d: [f64; 2], obey_locks: bool) -> Result<(), EditError> {
    let list = outermost(tx.doc(), shapes);
    let mut w = Writer::new();
    for s in list {
        let sheet = SheetRef::Shape(s);
        let sets = {
            let doc = tx.doc();
            let mut dp = d;
            if obey_locks && locked(eng, doc, s, "LockMoveX") {
                dp[0] = 0.0;
            }
            if obey_locks && locked(eng, doc, s, "LockMoveY") {
                dp[1] = 0.0;
            }
            if dp == [0.0, 0.0] {
                continue;
            }
            let Some(inv) = parent_xf(doc, s).inverse() else { continue };
            let dl = inv.apply_vec(dp);
            let mut sets = Vec::new();
            if one_d(doc, s) {
                let (gb, ge) = glued_ends(doc, s);
                let (b, e) = ends(doc, s);
                if !gb {
                    sets.extend(pt_sets(names::BEGIN_X, names::BEGIN_Y, [b[0] + dl[0], b[1] + dl[1]], doc, sheet));
                }
                if !ge {
                    sets.extend(pt_sets(names::END_X, names::END_Y, [e[0] + dl[0], e[1] + dl[1]], doc, sheet));
                }
            } else {
                let p = [val(doc, sheet, names::PIN_X) + dl[0], val(doc, sheet, names::PIN_Y) + dl[1]];
                sets.extend(pt_sets(names::PIN_X, names::PIN_Y, p, doc, sheet));
            }
            sets
        };
        w.put_all(tx, eng, sheet, sets)?;
    }
    w.finish(tx, eng)
}

/// give a shape another size and place with the same turn and flips (`to` from Frame::drag
/// of frame(doc, shape)): Width and Height first, then the pin where the frame puts the local
/// pin; a 1D shape gets its free ends (not glued, no LockBegin/LockEnd) on the frame's line
/// through the local pin.  LockWidth, LockHeight keep a side, LockAspect the proportions
pub fn set_frame(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, to: &Frame) -> Result<(), EditError> {
    tx.doc().shape(shape).ok_or(EditError::NoShape)?;
    set_frames(tx, eng, &[(shape, *to)])
}

fn set_frames(tx: &mut Transaction, eng: &mut Engine, list: &[(ShapeRef, Frame)]) -> Result<(), EditError> {
    // the sizes
    let mut w = Writer::new();
    let mut plan: Vec<(ShapeRef, Frame, bool)> = Vec::new();
    for &(s, f) in list {
        let sheet = SheetRef::Shape(s);
        let sets = {
            let doc = tx.doc();
            if doc.shape(s).is_none() {
                continue;
            }
            let old = frame(doc, s);
            let (lw, lh) = (locked(eng, doc, s, "LockWidth"), locked(eng, doc, s, "LockHeight"));
            let mut nw = if lw { old.w } else { f.w.max(0.0) };
            let mut nh = if lh { old.h } else { f.h.max(0.0) };
            if !lw && !lh && old.w > EPS && old.h > EPS && locked(eng, doc, s, "LockAspect") {
                let (sw, sh) = (nw / old.w, nh / old.h);
                let k = if (sw - 1.0).abs() >= (sh - 1.0).abs() { sw } else { sh };
                nw = old.w * k;
                nh = old.h * k;
            }
            let od = one_d(doc, s);
            let mut sets = Vec::new();
            if !od && (nw - old.w).abs() > EPS {
                sets.push((single(names::WIDTH), keep_form(doc, sheet, &single(names::WIDTH), nw)));
            }
            if (nh - old.h).abs() > EPS {
                sets.push((single(names::HEIGHT), keep_form(doc, sheet, &single(names::HEIGHT), nh)));
            }
            plan.push((s, Frame { xf: f.xf, w: nw, h: nh }, od));
            sets
        };
        w.put_all(tx, eng, sheet, sets)?;
    }
    w.finish(tx, eng)?;
    // the places
    for (s, f, od) in plan {
        let sheet = SheetRef::Shape(s);
        let sets = {
            let doc = tx.doc();
            let Some(inv) = parent_xf(doc, s).inverse() else { continue };
            let (lx, ly) = (val(doc, sheet, names::LOC_PIN_X), val(doc, sheet, names::LOC_PIN_Y));
            let mut sets = Vec::new();
            if od {
                // glued ends stay for glue::update, locked ends stay
                let (gb, ge) = glued_ends(doc, s);
                let (gb, ge) = (gb || locked(eng, doc, s, "LockBegin"), ge || locked(eng, doc, s, "LockEnd"));
                if !gb {
                    sets.extend(pt_sets(names::BEGIN_X, names::BEGIN_Y, inv.apply(f.at([0.0, ly])), doc, sheet));
                }
                if !ge {
                    sets.extend(pt_sets(names::END_X, names::END_Y, inv.apply(f.at([f.w, ly])), doc, sheet));
                }
            } else {
                sets.extend(pt_sets(names::PIN_X, names::PIN_Y, inv.apply(f.at([lx, ly])), doc, sheet));
            }
            sets
        };
        w.put_all(tx, eng, sheet, sets)?;
    }
    w.finish(tx, eng)
}

/// scale shapes from the page box `from` into `to` (x0, y0, x1, y1; a selection's grips):
/// each shape's middle goes where the boxes map it, its sides grow by how much the boxes
/// stretch them along its own axes (upright shapes exactly, turned ones by their axes); 1D
/// shapes map their ends
pub fn scale_shapes(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], from: [f64; 4], to: [f64; 4]) -> Result<(), EditError> {
    let fw = from[2] - from[0];
    let fh = from[3] - from[1];
    let sx = if fw.abs() > EPS { (to[2] - to[0]) / fw } else { 1.0 };
    let sy = if fh.abs() > EPS { (to[3] - to[1]) / fh } else { 1.0 };
    let map = |p: [f64; 2]| [to[0] + (p[0] - from[0]) * sx, to[1] + (p[1] - from[1]) * sy];
    let mut list = Vec::new();
    {
        let doc = tx.doc();
        for s in outermost(doc, shapes) {
            let f = frame(doc, s);
            if one_d(doc, s) {
                let ly = val(doc, SheetRef::Shape(s), names::LOC_PIN_Y);
                let (b, e) = (map(f.at([0.0, ly])), map(f.at([f.w, ly])));
                let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
                let len = math::sqrt(dx * dx + dy * dy);
                let xf = Xform::translate(0.0, -ly).then(&Xform::rotate(math::atan2(dy, dx))).then(&Xform::translate(b[0], b[1]));
                list.push((s, Frame { xf, w: len, h: f.h }));
            } else {
                let c = map(f.centre());
                let u = f.xf.apply_vec([1.0, 0.0]);
                let v = f.xf.apply_vec([0.0, 1.0]);
                let w = f.w * math::sqrt(u[0] * u[0] * sx * sx + u[1] * u[1] * sy * sy);
                let h = f.h * math::sqrt(v[0] * v[0] * sx * sx + v[1] * v[1] * sy * sy);
                let lin = Xform { e: 0.0, f: 0.0, ..f.xf };
                let xf = Xform::translate(-w * 0.5, -h * 0.5).then(&lin).then(&Xform::translate(c[0], c[1]));
                list.push((s, Frame { xf, w, h }));
            }
        }
    }
    set_frames(tx, eng, &list)
}

/// turn shapes by `angle` (radians, counter-clockwise) about the page point `centre`: Angle
/// grows (falls for a subshape in a mirrored group), the pin turns along; 1D shapes turn
/// their free ends; LockRotate leaves a shape out
pub fn rotate_shapes(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], centre: [f64; 2], angle: f64) -> Result<(), EditError> {
    let (s_, c_) = math::sincos(angle);
    let list = outermost(tx.doc(), shapes);
    let mut w = Writer::new();
    for s in list {
        let sheet = SheetRef::Shape(s);
        let (angle_set, sets) = {
            let doc = tx.doc();
            if locked(eng, doc, s, "LockRotate") {
                continue;
            }
            let par = parent_xf(doc, s);
            let Some(inv) = par.inverse() else { continue };
            let turn = |p: [f64; 2]| inv.apply(rotate_about(par.apply(p), centre, s_, c_));
            let mut sets = Vec::new();
            if one_d(doc, s) {
                let (gb, ge) = glued_ends(doc, s);
                let (b, e) = ends(doc, s);
                if !gb {
                    sets.extend(pt_sets(names::BEGIN_X, names::BEGIN_Y, turn(b), doc, sheet));
                }
                if !ge {
                    sets.extend(pt_sets(names::END_X, names::END_Y, turn(e), doc, sheet));
                }
                (None, sets)
            } else {
                let delta = if par.det() < 0.0 { -angle } else { angle };
                let a = norm_angle(val(doc, sheet, names::ANGLE) + delta);
                let p = [val(doc, sheet, names::PIN_X), val(doc, sheet, names::PIN_Y)];
                sets.extend(pt_sets(names::PIN_X, names::PIN_Y, turn(p), doc, sheet));
                (Some(a), sets)
            }
        };
        if let Some(a) = angle_set {
            // a guarded angle keeps the shape where it is
            if !w.put(tx, eng, sheet, single(names::ANGLE), Put::Num(a))? {
                continue;
            }
        }
        w.put_all(tx, eng, sheet, sets)?;
    }
    w.finish(tx, eng)
}

/// flip shapes about the middle of their box (`horizontal`: left and right change places):
/// a horizontal flip toggles FlipX, a vertical one FlipY, a turned shape's angle changes
/// with it (see the module comment), the pin is mirrored; a 1D shape mirrors its free ends
/// and toggles FlipY (its local y axis turns over)
pub fn flip_shapes(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], horizontal: bool) -> Result<(), EditError> {
    let list = outermost(tx.doc(), shapes);
    let Some(b) = bounds(tx.doc(), &list) else { return Ok(()) };
    let c = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let mirror = |p: [f64; 2]| if horizontal { [2.0 * c[0] - p[0], p[1]] } else { [p[0], 2.0 * c[1] - p[1]] };
    let mut w = Writer::new();
    for s in list {
        let sheet = SheetRef::Shape(s);
        let (flip, rest) = {
            let doc = tx.doc();
            let par = parent_xf(doc, s);
            let Some(inv) = par.inverse() else { continue };
            let m = |p: [f64; 2]| inv.apply(mirror(par.apply(p)));
            let mut sets = Vec::new();
            if one_d(doc, s) {
                let (gb, ge) = glued_ends(doc, s);
                if gb && ge {
                    continue;
                }
                let (bp, ep) = ends(doc, s);
                if !gb {
                    sets.extend(pt_sets(names::BEGIN_X, names::BEGIN_Y, m(bp), doc, sheet));
                }
                if !ge {
                    sets.extend(pt_sets(names::END_X, names::END_Y, m(ep), doc, sheet));
                }
                let fy = val(doc, sheet, names::FLIP_Y) != 0.0;
                ((names::FLIP_Y, if fy { 0.0 } else { 1.0 }), sets)
            } else {
                let axis = if horizontal { [0.0, 1.0] } else { [1.0, 0.0] };
                let d = inv.apply_vec(axis);
                let phi = math::atan2(d[1], d[0]);
                let a = val(doc, sheet, names::ANGLE);
                let na = if horizontal { 2.0 * phi - a + core::f64::consts::PI } else { 2.0 * phi - a };
                let cell = if horizontal { names::FLIP_X } else { names::FLIP_Y };
                let on = val(doc, sheet, cell) != 0.0;
                let p = [val(doc, sheet, names::PIN_X), val(doc, sheet, names::PIN_Y)];
                sets.push((single(names::ANGLE), Put::Num(norm_angle(na))));
                sets.extend(pt_sets(names::PIN_X, names::PIN_Y, m(p), doc, sheet));
                ((cell, if on { 0.0 } else { 1.0 }), sets)
            }
        };
        // a guarded flip cell keeps the shape as it is
        if !w.put(tx, eng, sheet, single(flip.0), Put::Num(flip.1))? {
            continue;
        }
        w.put_all(tx, eng, sheet, rest)?;
    }
    w.finish(tx, eng)
}

/// move the pin of a shape (its centre of rotation) to the page point `at` without moving
/// the shape: LocPinX/LocPinY get the local point (proportional "Width*k" when their formula
/// has that form), the pin follows
pub fn set_pin(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, at: [f64; 2]) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(shape);
    let (old, sets) = {
        let doc = tx.doc();
        doc.shape(shape).ok_or(EditError::NoShape)?;
        let old = doc.page_xform(shape);
        let q = old.inverse().ok_or(EditError::Invalid)?.apply(at);
        (old, pt_sets(names::LOC_PIN_X, names::LOC_PIN_Y, q, doc, sheet))
    };
    let mut w = Writer::new();
    w.put_all(tx, eng, sheet, sets.into())?;
    w.finish(tx, eng)?;
    let sets = {
        let doc = tx.doc();
        let loc = [val(doc, sheet, names::LOC_PIN_X), val(doc, sheet, names::LOC_PIN_Y)];
        let inv = parent_xf(doc, shape).inverse().ok_or(EditError::Invalid)?;
        pt_sets(names::PIN_X, names::PIN_Y, inv.apply(old.apply(loc)), doc, sheet)
    };
    w.put_all(tx, eng, sheet, sets.into())?;
    w.finish(tx, eng)
}

// ---------------------------------------------------------------- groups

// a cell to its value as a constant (structural: also over GUARD), keeping its unit
fn freeze(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr) -> Result<(), EditError> {
    let r = tx.doc().resolve(sheet, addr);
    if !r.exists() {
        return Ok(());
    }
    let (v, u) = (r.value(), r.unit());
    tx.force_constant(sheet, addr, v, u)
}

// the cells of a Connect entry's FromCell (BeginX -> BeginX, BeginY; Controls.Row_1 -> its X
// and Y; PinX -> PinX, PinY; anything else that cell)
pub(crate) fn glue_cells(doc: &Document, from_cell: Sym) -> Vec<CellAddr> {
    let name = doc.str(from_cell);
    match name {
        "BeginX" => alloc::vec![single(names::BEGIN_X), single(names::BEGIN_Y)],
        "EndX" => alloc::vec![single(names::END_X), single(names::END_Y)],
        "PinX" => alloc::vec![single(names::PIN_X), single(names::PIN_Y)],
        _ => match name.strip_prefix("Controls.").and_then(|r| doc.sym(r)) {
            Some(row) => {
                let sk = SecKey::one(names::CONTROL);
                alloc::vec![CellAddr::row(sk, RowKey::Named(row), names::X), CellAddr::row(sk, RowKey::Named(row), names::Y)]
            }
            None => alloc::vec![single(from_cell)],
        },
    }
}

// the cells of a sheet whose own formula names shape `id` by Sheet.N (an inherited formula's
// Sheet.N names a shape of its master)
fn cells_naming(doc: &Document, sheet: SheetRef, id: u32) -> Vec<CellAddr> {
    let want = format!("Sheet.{id}");
    crate::formula::sheet_formula_cells(doc, sheet)
        .into_iter()
        .map(|k| k.addr())
        .filter(|a| {
            let r = doc.resolve(sheet, a);
            match (r.formula(), r.source) {
                (Formula::Expr(f), Some(src)) if src == sheet => {
                    text::items(doc.str(f)).iter().any(|it| matches!(it, text::Item::Sheet { text, foreign: false, .. } if *text == want))
                }
                _ => false,
            }
        })
        .collect()
}

/// group shapes of one parent (SP-06): a new group shape (Type Group, the style "Normal",
/// ResizeMode 0) around the shapes, at the place of the topmost of them in the z-order; the
/// box is the upright hull of the shapes in the parent's coordinates (turned shapes with their
/// turned corners, 1D shapes with their ends).  The shapes move into it in their order and
/// get Visio's formulas for a group that scales its members: PinX "Sheet.<group>!Width*k",
/// PinY "Sheet.<group>!Height*k", and for upright members (turned by a multiple of 90°) whose
/// size is a constant Width/Height "Sheet.<group>!Width*k" (ResizeMode 1 of the member: only
/// the pin); free ends of 1D members alike, glued ends keep their formulas -> the group
pub fn group(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef]) -> Result<ShapeRef, EditError> {
    let list = outermost(tx.doc(), shapes);
    if list.is_empty() {
        return Err(EditError::Invalid);
    }
    let (parent, b, siblings) = {
        let doc = tx.doc();
        let parent = doc.shape(list[0]).ok_or(EditError::NoShape)?.parent();
        let mut b: Option<[f64; 4]> = None;
        for &s in &list {
            let sh = doc.shape(s).ok_or(EditError::NoShape)?;
            let h = sh.header();
            if s.owner != owner || sh.parent() != parent || (h.master_shape.is_some() && h.master.is_none()) {
                return Err(EditError::Invalid);
            }
            let pts: Vec<[f64; 2]> = if one_d(doc, s) {
                let (bp, ep) = ends(doc, s);
                alloc::vec![bp, ep]
            } else {
                let lx = doc.local_xform(SheetRef::Shape(s));
                let f = Frame { xf: lx, w: val(doc, SheetRef::Shape(s), names::WIDTH), h: val(doc, SheetRef::Shape(s), names::HEIGHT) };
                f.corners().to_vec()
            };
            for p in pts {
                b = Some(match b {
                    None => [p[0], p[1], p[0], p[1]],
                    Some(b) => [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])],
                });
            }
        }
        let siblings: Vec<ShapeRef> = match parent {
            Some(p) => doc.shape(p).ok_or(EditError::NoShape)?.children().collect(),
            None => doc.top_shapes(owner).collect(),
        };
        (parent, b.ok_or(EditError::Invalid)?, siblings)
    };
    let (gw, gh) = (b[2] - b[0], b[3] - b[1]);
    // the members in z-order, the group's place: past the other shapes below the topmost member
    let members: Vec<ShapeRef> = siblings.iter().copied().filter(|s| list.contains(s)).collect();
    let top = siblings.iter().rposition(|s| list.contains(s)).unwrap_or(0);
    let pos = siblings[..top].iter().filter(|s| !list.contains(s)).count();
    // the group shape
    let (unit, normal) = {
        let doc = tx.doc();
        (page_unit(doc, owner), doc.style_by_name_u("Normal").and_then(|s| doc.style(s)).map(|s| s.header.id))
    };
    let mut data = ShapeData::new(ShapeKind::Group);
    if let Some(id) = normal {
        data.header.styles = StyleRefs::all(id);
    }
    data.sheet.cells = basic_cells(tx, [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5], gw, gh, unit);
    data.sheet.cells.push(Cell::constant(names::RESIZE_MODE, Value::Num(0.0), None));
    let g = tx.add_shape_with(owner, parent, data)?;
    let gid = tx.doc().shape(g).ok_or(EditError::NoShape)?.id();
    for &m in &members {
        tx.move_shape(m, Some(g), None)?;
    }
    tx.move_shape(g, parent, Some(pos))?;
    // the members' cells in the group's coordinates
    let group_rm = 0.0;
    let mut w = Writer::new();
    for &m in &members {
        let sheet = SheetRef::Shape(m);
        let (sets, results) = {
            let doc = tx.doc();
            let prop = |v: f64, size: f64, cell: &str| if size.abs() > EPS { Put::Formula(format!("Sheet.{gid}!{cell}*{}", text::num(v / size))) } else { Put::Num(v) };
            let mut sets: Vec<(CellAddr, Put)> = Vec::new();
            let mut results: Vec<(CellAddr, f64)> = Vec::new();
            if one_d(doc, m) {
                let (gb, ge) = glued_ends(doc, m);
                let (bp, ep) = ends(doc, m);
                for (glued, cx, cy, p) in [(gb, names::BEGIN_X, names::BEGIN_Y, bp), (ge, names::END_X, names::END_Y, ep)] {
                    let q = [p[0] - b[0], p[1] - b[1]];
                    if glued {
                        results.push((single(cx), q[0]));
                        results.push((single(cy), q[1]));
                    } else {
                        sets.push((single(cx), prop(q[0], gw, "Width")));
                        sets.push((single(cy), prop(q[1], gh, "Height")));
                    }
                }
            } else {
                let q = [val(doc, sheet, names::PIN_X) - b[0], val(doc, sheet, names::PIN_Y) - b[1]];
                sets.push((single(names::PIN_X), prop(q[0], gw, "Width")));
                sets.push((single(names::PIN_Y), prop(q[1], gh, "Height")));
                let rm = val(doc, sheet, names::RESIZE_MODE);
                let rm = if rm == 0.0 { group_rm } else { rm };
                if rm != 1.0 {
                    let a = norm_angle(val(doc, sheet, names::ANGLE));
                    let quarter = core::f64::consts::FRAC_PI_2;
                    let k = math::round(a / quarter);
                    if (a - k * quarter).abs() < 1e-9 {
                        let swap = (k as i64).rem_euclid(2) == 1;
                        let (w0, h0) = (val(doc, sheet, names::WIDTH), val(doc, sheet, names::HEIGHT));
                        let plain = |c: Sym| match doc.resolve(sheet, &single(c)).formula() {
                            Formula::Expr(f) => text::proportional(doc.str(f)).is_some(),
                            _ => true,
                        };
                        if plain(names::WIDTH) {
                            sets.push((single(names::WIDTH), if swap { prop(w0, gh, "Height") } else { prop(w0, gw, "Width") }));
                        }
                        if plain(names::HEIGHT) {
                            sets.push((single(names::HEIGHT), if swap { prop(h0, gw, "Width") } else { prop(h0, gh, "Height") }));
                        }
                    }
                }
            }
            (sets, results)
        };
        for (a, v) in results {
            tx.set_result(sheet, &a, Value::Num(v), None)?;
        }
        w.put_all(tx, eng, sheet, sets)?;
    }
    let mut sheets: Vec<SheetRef> = members.iter().map(|&m| SheetRef::Shape(m)).collect();
    sheets.push(SheetRef::Shape(g));
    eng.recalc(tx, &sheets)?;
    Ok(g)
}

/// take a group apart (SP-06): its members go to the group's parent at the group's place in
/// the z-order, with the group's pin, angle and flips worked into their own; formulas that
/// name the group become their values, glue to the group is let go (Formula -> value, Connect
/// entry gone), the group shape goes -> the former members.  Instances of masters and groups
/// with LockGroup are refused (Invalid)
pub fn ungroup(tx: &mut Transaction, eng: &mut Engine, group: ShapeRef) -> Result<Vec<ShapeRef>, EditError> {
    let (gid, gp, parent, pos, kids, detach) = {
        let doc = tx.doc();
        let g = doc.shape(group).ok_or(EditError::NoShape)?;
        let h = g.header();
        if h.kind != ShapeKind::Group || h.master.is_some() || h.master_shape.is_some() {
            return Err(EditError::Invalid);
        }
        if locked(eng, doc, group, "LockGroup") {
            return Err(EditError::Invalid);
        }
        let parent = g.parent();
        let siblings: Vec<ShapeRef> = match parent {
            Some(p) => doc.shape(p).ok_or(EditError::NoShape)?.children().collect(),
            None => doc.top_shapes(group.owner).collect(),
        };
        let pos = siblings.iter().position(|&s| s == group).unwrap_or(siblings.len());
        let gid = g.id();
        // glue to the group: the glued cells of other shapes let go
        let detach: Vec<(ShapeRef, Sym)> = connects(doc, group.owner)
            .iter()
            .filter(|c| c.to_sheet == gid)
            .filter_map(|c| Some((doc.shape_by_id(group.owner, c.from_sheet)?, c.from_cell?)))
            .collect();
        (gid, doc.local_xform(SheetRef::Shape(group)), parent, pos, g.children().collect::<Vec<_>>(), detach)
    };
    let mirrored = gp.det() < 0.0;
    let (gfx, gfy, ga) = {
        let doc = tx.doc();
        let s = SheetRef::Shape(group);
        (val(doc, s, names::FLIP_X) != 0.0, val(doc, s, names::FLIP_Y) != 0.0, val(doc, s, names::ANGLE))
    };
    // the new values in the parent's coordinates, read before anything moves
    struct Kid {
        shape: ShapeRef,
        sets: Vec<(CellAddr, Put)>,
        results: Vec<(CellAddr, f64)>,
    }
    let mut plan: Vec<Kid> = Vec::new();
    {
        let doc = tx.doc();
        for &k in &kids {
            let sheet = SheetRef::Shape(k);
            let mut sets = Vec::new();
            let mut results = Vec::new();
            if one_d(doc, k) {
                let (gb, ge) = glued_ends(doc, k);
                let (bp, ep) = ends(doc, k);
                for (glued, cx, cy, p) in [(gb, names::BEGIN_X, names::BEGIN_Y, bp), (ge, names::END_X, names::END_Y, ep)] {
                    let q = gp.apply(p);
                    if glued {
                        results.push((single(cx), q[0]));
                        results.push((single(cy), q[1]));
                    } else {
                        sets.push((single(cx), Put::Num(q[0])));
                        sets.push((single(cy), Put::Num(q[1])));
                    }
                }
                if mirrored {
                    let fy = val(doc, sheet, names::FLIP_Y) != 0.0;
                    sets.push((single(names::FLIP_Y), Put::Num(if fy { 0.0 } else { 1.0 })));
                }
            } else {
                let p = gp.apply([val(doc, sheet, names::PIN_X), val(doc, sheet, names::PIN_Y)]);
                let a = val(doc, sheet, names::ANGLE);
                let fx = (val(doc, sheet, names::FLIP_X) != 0.0) != gfx;
                let fy = (val(doc, sheet, names::FLIP_Y) != 0.0) != gfy;
                sets.push((single(names::PIN_X), Put::Num(p[0])));
                sets.push((single(names::PIN_Y), Put::Num(p[1])));
                sets.push((single(names::ANGLE), Put::Num(norm_angle(ga + if mirrored { -a } else { a }))));
                sets.push((single(names::FLIP_X), Put::Num(if fx { 1.0 } else { 0.0 })));
                sets.push((single(names::FLIP_Y), Put::Num(if fy { 1.0 } else { 0.0 })));
            }
            plan.push(Kid { shape: k, sets, results });
        }
    }
    // formulas that name the group: values (every shape inside the group)
    let inside: Vec<ShapeRef> = {
        let doc = tx.doc();
        let mut v = Vec::new();
        let mut stack: Vec<ShapeRef> = kids.clone();
        while let Some(s) = stack.pop() {
            v.push(s);
            if let Some(sh) = doc.shape(s) {
                stack.extend(sh.children());
            }
        }
        v
    };
    let mut touched: Vec<SheetRef> = Vec::new();
    for &s in &inside {
        let sheet = SheetRef::Shape(s);
        for a in cells_naming(tx.doc(), sheet, gid) {
            freeze(tx, sheet, &a)?;
        }
        touched.push(sheet);
    }
    for &(s, cell) in &detach {
        let sheet = SheetRef::Shape(s);
        for a in glue_cells(tx.doc(), cell) {
            freeze(tx, sheet, &a)?;
        }
        touched.push(sheet);
    }
    // the members out of the group, at its place
    for (i, &k) in kids.iter().enumerate() {
        tx.move_shape(k, parent, Some(pos + i))?;
    }
    let mut w = Writer::new();
    for kid in plan {
        let sheet = SheetRef::Shape(kid.shape);
        for (a, v) in kid.results {
            tx.set_result(sheet, &a, Value::Num(v), None)?;
        }
        w.put_all(tx, eng, sheet, kid.sets)?;
    }
    tx.delete_shapes(&[group])?;
    touched.push(SheetRef::Shape(group));
    eng.recalc(tx, &touched)?;
    w.finish(tx, eng)?;
    Ok(kids)
}

/// where shapes go in the z-order among their siblings (SP-07)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Z {
    /// on top
    Front,
    /// below all
    Back,
    /// one step up (past the next shape that is not moved too)
    Forward,
    /// one step down
    Backward,
}

/// change the z-order of shapes among their siblings; shapes of several groups move within
/// their own group, moved shapes keep their order among themselves
pub fn reorder(tx: &mut Transaction, shapes: &[ShapeRef], z: Z) -> Result<(), EditError> {
    let list = outermost(tx.doc(), shapes);
    let mut parents: Vec<(Owner, Option<ShapeRef>)> = Vec::new();
    for &s in &list {
        let p = (s.owner, tx.doc().shape(s).and_then(|x| x.parent()));
        if !parents.contains(&p) {
            parents.push(p);
        }
    }
    for (owner, parent) in parents {
        let cur: Vec<ShapeRef> = match parent {
            Some(p) => tx.doc().shape(p).ok_or(EditError::NoShape)?.children().collect(),
            None => tx.doc().top_shapes(owner).collect(),
        };
        let sel: Vec<bool> = cur.iter().map(|s| list.contains(s)).collect();
        let mut order: Vec<(ShapeRef, bool)> = cur.iter().copied().zip(sel.iter().copied()).collect();
        match z {
            Z::Front => order.sort_by_key(|&(_, s)| s),
            Z::Back => order.sort_by_key(|&(_, s)| !s),
            Z::Forward => {
                for i in (0..order.len().saturating_sub(1)).rev() {
                    if order[i].1 && !order[i + 1].1 {
                        order.swap(i, i + 1);
                    }
                }
            }
            Z::Backward => {
                for i in 1..order.len() {
                    if order[i].1 && !order[i - 1].1 {
                        order.swap(i, i - 1);
                    }
                }
            }
        }
        // put each shape at its place, bottom up
        let mut now = cur.clone();
        for (k, &(s, _)) in order.iter().enumerate() {
            if now[k] != s {
                tx.move_shape(s, parent, Some(k))?;
                let from = now.iter().position(|&x| x == s).unwrap_or(k);
                let x = now.remove(from);
                now.insert(k, x);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- new shapes, text

/// the shapes the drawing tools make
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Basic {
    /// the rectangle tool: Geometry RelMoveTo/RelLineTo around the box, style "Normal"
    Rectangle,
    /// the ellipse tool: one Ellipse row (X Width*0.5, Y Height*0.5, A Width*1, B Height*0.5,
    /// C Width*0.5, D Height*1), style "Normal"
    Ellipse,
    /// the text tool: a box without line and fill (style "Text Only", as Visio's text tool
    /// gives it; JD11: so that its text keeps the style's dark font style in every design)
    /// whose Height follows its text: TEXTHEIGHT(TheText,Width)
    TextBox,
}

// the unit a page shows lengths in (DrawingScale's, else PageWidth's; None: inches)
fn page_unit(doc: &Document, owner: Owner) -> Option<Unit> {
    let sheet = match owner {
        Owner::Page(p) => SheetRef::Page(p),
        Owner::Master(m) => SheetRef::Master(m),
    };
    doc.resolve(sheet, &single(names::DRAWING_SCALE)).unit().or_else(|| doc.resolve(sheet, &single(names::PAGE_WIDTH)).unit())
}

// pin, size, local pin "Width*0.5"/"Height*0.5", no turn or flip
fn basic_cells(tx: &mut Transaction, pin: [f64; 2], w: f64, h: f64, unit: Option<Unit>) -> Vec<Cell> {
    let fx = tx.intern("Width*0.5");
    let fy = tx.intern("Height*0.5");
    let f = |name: Sym, v: f64, s: Sym| Cell { name, value: Value::Num(v), formula: Formula::Expr(s), unit, error: None, stale: false };
    alloc::vec![
        Cell::constant(names::PIN_X, Value::Num(pin[0]), unit),
        Cell::constant(names::PIN_Y, Value::Num(pin[1]), unit),
        Cell::constant(names::WIDTH, Value::Num(w), unit),
        Cell::constant(names::HEIGHT, Value::Num(h), unit),
        f(names::LOC_PIN_X, w * 0.5, fx),
        f(names::LOC_PIN_Y, h * 0.5, fy),
        Cell::constant(names::ANGLE, Value::Num(0.0), Some(Unit::Degrees)),
        Cell::constant(names::FLIP_X, Value::Num(0.0), None),
        Cell::constant(names::FLIP_Y, Value::Num(0.0), None),
    ]
}

/// a new shape of a drawing tool between the page points `a` and `b` (the dragged box), on
/// top of the owner's shapes; a box without area is refused (Invalid), except for the text
/// tool, which makes a box 1 inch wide around `a` -> the shape
pub fn new_shape(tx: &mut Transaction, eng: &mut Engine, owner: Owner, kind: Basic, a: [f64; 2], b: [f64; 2]) -> Result<ShapeRef, EditError> {
    tx.doc().tree(owner).ok_or(EditError::NoPage)?;
    let (mut w, mut h) = ((b[0] - a[0]).abs(), (b[1] - a[1]).abs());
    let mut pin = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    if kind == Basic::TextBox && w < EPS {
        w = 1.0;
        h = 0.25;
        pin = a;
    }
    if w < EPS || h < EPS {
        return Err(EditError::Invalid);
    }
    let (unit, normal, text_only) = {
        let doc = tx.doc();
        let id = |n: &str| doc.style_by_name_u(n).and_then(|s| doc.style(s)).map(|s| s.header.id);
        (page_unit(doc, owner), id("Normal"), id("Text Only"))
    };
    let mut d = ShapeData::new(ShapeKind::Shape);
    d.header.styles = match kind {
        Basic::TextBox => StyleRefs { line: text_only.or(normal), fill: text_only.or(normal), text: text_only.or(normal) },
        _ => StyleRefs { line: normal, fill: normal, text: normal },
    };
    d.sheet.cells = basic_cells(tx, pin, w, h, unit);
    let gk = SecKey { name: names::GEOMETRY, index: 0 };
    let mut geo = Section::new(gk);
    geo.ix_written = true;
    for c in [names::NO_FILL, names::NO_LINE, names::NO_SHOW, names::NO_SNAP, names::NO_QUICK_DRAG] {
        geo.cells.push(Cell::constant(c, Value::Num(0.0), None));
    }
    let row = |ix: u32, kind: Sym, cells: Vec<Cell>| {
        let mut r = Row::new(RowKey::Index(ix));
        r.kind = Some(kind);
        r.cells = cells;
        r
    };
    let num = |name: Sym, v: f64| Cell::constant(name, Value::Num(v), None);
    match kind {
        Basic::Rectangle | Basic::TextBox => {
            let pts = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)];
            for (i, (x, y)) in pts.into_iter().enumerate() {
                let k = if i == 0 { names::REL_MOVE_TO } else { names::REL_LINE_TO };
                geo.rows.push(row(i as u32 + 1, k, alloc::vec![num(names::X, x), num(names::Y, y)]));
            }
        }
        Basic::Ellipse => {
            let cells = [(names::X, "Width*0.5", w * 0.5), (names::Y, "Height*0.5", h * 0.5), (names::A, "Width*1", w), (names::B, "Height*0.5", h * 0.5), (names::C, "Width*0.5", w * 0.5), (names::D, "Height*1", h)];
            let cells = cells
                .into_iter()
                .map(|(n, f, v)| {
                    let f = tx.intern(f);
                    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
                })
                .collect();
            geo.rows.push(row(1, names::ELLIPSE, cells));
        }
    }
    d.sheet.sections.push(geo);
    if kind == Basic::TextBox {
        let f = tx.intern("TEXTHEIGHT(TheText,Width)");
        if let Some(c) = d.sheet.cells.iter_mut().find(|c| c.name == names::HEIGHT) {
            c.formula = Formula::Expr(f);
            c.stale = true;
        }
        tx.intern("TheText");
    }
    let s = tx.add_shape_with(owner, None, d)?;
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    Ok(s)
}

/// give a shape its own text (TX-01) and recompute what depends on it (TEXTWIDTH,
/// TEXTHEIGHT: a text box's Height, TxtHeight ...); LockTextEdit refuses (Invalid)
pub fn set_text(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, text: Text) -> Result<(), EditError> {
    tx.doc().shape(shape).ok_or(EditError::NoShape)?;
    if locked(eng, tx.doc(), shape, "LockTextEdit") {
        return Err(EditError::Invalid);
    }
    tx.set_text(shape, Some(text))?;
    let sheet = SheetRef::Shape(shape);
    let the_text = tx.intern("TheText");
    eng.recalc_cells(tx, &[(sheet, single(the_text))])?;
    eng.recalc(tx, &[sheet])?;
    Ok(())
}

/// where a shape's text block is (TX-03): the text transform (TxtPinX/Y, TxtLocPinX/Y,
/// TxtAngle inside the shape) to the page and TxtWidth/TxtHeight; cells the shape lacks have
/// Visio's defaults (the whole box, turned 0)
pub fn text_frame(doc: &Document, shape: ShapeRef) -> Frame {
    let s = SheetRef::Shape(shape);
    let get = |c: Sym, d: f64| doc.resolve(s, &single(c)).num().unwrap_or(d);
    let (w, h) = (val(doc, s, names::WIDTH), val(doc, s, names::HEIGHT));
    let tw = get(names::TXT_WIDTH, w);
    let th = get(names::TXT_HEIGHT, h);
    let pin = [get(names::TXT_PIN_X, w * 0.5), get(names::TXT_PIN_Y, h * 0.5)];
    let loc = [get(names::TXT_LOC_PIN_X, tw * 0.5), get(names::TXT_LOC_PIN_Y, th * 0.5)];
    let a = get(names::TXT_ANGLE, 0.0);
    let mut m = Xform::translate(-loc[0], -loc[1]);
    if a != 0.0 {
        m = m.then(&Xform::rotate(a));
    }
    let m = m.then(&Xform::translate(pin[0], pin[1])).then(&doc.page_xform(shape));
    Frame { xf: m, w: tw, h: th }
}

/// move and size a shape's text block (TX-03; `to` from Frame::drag of text_frame, same
/// turn): TxtWidth and TxtHeight, then TxtPinX/Y where the frame puts the text's local pin
/// (proportional formulas keep their form); LockTextEdit refuses (Invalid)
pub fn set_text_frame(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, to: &Frame) -> Result<(), EditError> {
    tx.doc().shape(shape).ok_or(EditError::NoShape)?;
    if locked(eng, tx.doc(), shape, "LockTextEdit") {
        return Err(EditError::Invalid);
    }
    let sheet = SheetRef::Shape(shape);
    let mut w = Writer::new();
    let sets = {
        let doc = tx.doc();
        let old = text_frame(doc, shape);
        let mut sets = Vec::new();
        if (to.w - old.w).abs() > EPS {
            sets.push((single(names::TXT_WIDTH), keep_form(doc, sheet, &single(names::TXT_WIDTH), to.w.max(0.0))));
        }
        if (to.h - old.h).abs() > EPS {
            sets.push((single(names::TXT_HEIGHT), keep_form(doc, sheet, &single(names::TXT_HEIGHT), to.h.max(0.0))));
        }
        sets
    };
    w.put_all(tx, eng, sheet, sets)?;
    w.finish(tx, eng)?;
    let sets = {
        let doc = tx.doc();
        let now = text_frame(doc, shape);
        let tw = now.w;
        let th = now.h;
        let loc = [doc.resolve(sheet, &single(names::TXT_LOC_PIN_X)).num().unwrap_or(tw * 0.5), doc.resolve(sheet, &single(names::TXT_LOC_PIN_Y)).num().unwrap_or(th * 0.5)];
        let inv = doc.page_xform(shape).inverse().ok_or(EditError::Invalid)?;
        pt_sets(names::TXT_PIN_X, names::TXT_PIN_Y, inv.apply(to.at(loc)), doc, sheet)
    };
    w.put_all(tx, eng, sheet, sets.into())?;
    w.finish(tx, eng)
}
