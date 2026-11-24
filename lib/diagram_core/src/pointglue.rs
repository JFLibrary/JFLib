// pointglue -- (JD21) glue beyond the ends of 1-D shapes: control points that glue (SP-04,
// SS-30), glue to vertices, handles, control points and guides (CN-02), the connection point
// tool (CN-01, RS-17)
//
//   glue_control(tx, eng, shape, row, at, to)    a control point dragged to `at` and glued to a
//                                                connection point (Target::Point) or let go
//                                                (Target::None)
//   glued_controls(doc, shape) -> [(row, Target)]  what its control points are glued to
//   followers(doc, owner, &moved)                 shapes whose control points are glued to these
//                                                 shapes: they changed with them (their own glued
//                                                 connectors follow in the app)
//   Anchor / anchor_point(tx, eng, target, anchor)  the connection point at a vertex, a handle
//                                                 or a control point of `target` (made when it
//                                                 has none there yet) -> its IX; glue::glue or
//                                                 glue_control glue to it
//   anchor_at(doc, target, anchor)                where that place is on the page
//   glue_guide(tx, eng, shape, end, at, guide)    an end of a 1-D shape on a guide
//   is_glue_guide(doc, g) / guide_of(doc, shape)  a guide ends glue to; the guides its ends are on
//   add_point / move_point / delete_point / set_point_kind / point_kind / point_at   the
//                                                 connection point tool: a Connection row added,
//                                                 moved, taken out, its Type (0 inward, 1
//                                                 outward, 2 both)
//
// The file's record, as Visio writes it (the local files: Ethernet taps glued to computers;
// learn.microsoft.com VisFromParts, VisToParts, Cell.GlueTo; Visio 2002 Developer Reference
// "Creating a Connected Drawing from a Program"):
// * a control point (Controls row k, 0-based position in its section) glued to connection point
//   n of shape N: X and Y = LOC(PNT(Sheet.N!Connections.Xn,Sheet.N!Connections.Yn)) (LOC: a
//   control point's X and Y are local), Connect FromCell=Controls.<row name> FromPart=100+k
//   ToCell=Connections.Xn ToPart=100+n-1.  No trigger cell: the formula follows the target.
//   Control points glue statically only (Visio: "control-handle connectors ... can only glue to
//   a specific point"), to inward points and points of both kinds (Type 0 and 2), not to
//   outward ones.
// * Glue to a vertex, a selection handle or (assumed: Visio's GlueTo lists them for neither
//   ends nor control points) a control point of another shape: Visio adds a connection point
//   there and glues to it ("A connection point is created that is glued to ... new formulas in
//   both the X and Y cells of the connection row").  The row's X and Y here: Geometry<s>.X<r> and
//   .Y<r> of the vertex (relative rows: times Width and Height), Width*fx and Height*fy of the
//   handle, Controls.<row> and Controls.<row>.Y of the control point (assumed); DirX, DirY 0,
//   Type 0.  A row with these formulas that is there already is used again.
// * An end on a guide (assumed, the Visio-Gegenprobe checks it): a horizontal guide gives the
//   end's Y the formula Sheet.G!PinY (vertical: X = Sheet.G!PinX), the other coordinate stays a
//   value; Connect FromCell=BeginX (EndX) FromPart=9 (12) ToSheet=G ToCell=Angle ToPart=2
//   (visGuideY; a vertical guide 1, visGuideX).  GlueTo's text names two connections "to the
//   guide's Angle cell"; one entry per end is written here (glue::glue replaces it like any).
//   Only top-level 1-D shapes and guides that run across or up the page.
//
// The connection point tool (Visio's: Ctrl+click on a selected shape adds a point, a drag moves
// it, Del takes it out; the point's menu sets its kind): a new row's X and Y are Width*k and
// Height*k (proportional, as Visio writes them), DirX and DirY 0, Type 0; a moved point keeps
// the form of its formulas (ops: proportional stays proportional).  Taking a point out lets go
// of what is glued to it (its cells keep their values) and renumbers the rows after it; what is
// glued to them is glued again to the same point under its new number.
//
// Which kinds glue (Visio's Type cell: 0 inward, 1 outward, 2 inward & outward): ends and
// control points glue to points of kind 0 and 2; a 2-D shape's points of kind 1 and 2 dock on
// points of kind 0 and 2 of another shape (dock.rs).  The edit layer leaves kind 1 out as a
// target (SPF_OUTWARD).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::{Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, End, Target};
use crate::names;
use crate::ops::{self, text, Put, Writer};
use crate::page::Connect;
use crate::shape::ShapeKind;
use crate::sheet::{CellAddr, Extras, RowKey, SecKey};

/// a place on a shape that gets a connection point when something glues there (CN-02)
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Anchor {
    /// a vertex: the Geometry section's index (Geometry1 is 0) and the row's IX
    Vertex { section: u32, row: u32 },
    /// a selection handle: fractions of Width and Height (0, 0.5, 1)
    Handle { fx: f64, fy: f64 },
    /// a control point (its row of the Controls section)
    Control(RowKey),
}

fn conn_key() -> SecKey {
    SecKey::one(names::CONNECTION)
}

fn control_key() -> SecKey {
    SecKey::one(names::CONTROL)
}

fn connects(doc: &Document, owner: Owner) -> &[Connect] {
    match owner {
        Owner::Page(p) => doc.page(p).map_or(&[][..], |x| x.connects.as_slice()),
        Owner::Master(m) => doc.master(m).map_or(&[][..], |x| x.connects.as_slice()),
    }
}

// whether `inner` is `outer` or lies inside it
fn within(doc: &Document, inner: ShapeRef, outer: ShapeRef) -> bool {
    let mut cur = Some(inner);
    let mut depth = 0;
    while let (Some(r), true) = (cur, depth < 1024) {
        if r == outer {
            return true;
        }
        cur = doc.shape(r).and_then(|s| s.parent());
        depth += 1;
    }
    false
}

// the name a control row has in formulas and Connect entries (Row_1 ... or its own name)
fn control_name(doc: &Document, row: RowKey) -> String {
    match row {
        RowKey::Named(n) => String::from(doc.str(n)),
        RowKey::Index(i) => format!("Row_{}", i + 1),
    }
}

// a formula's text, own or inherited ("" for none)
fn formula_of(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> String {
    match doc.resolve(sheet, addr).formula() {
        Formula::Expr(f) => String::from(doc.str(f)),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------- control points (SP-04)

/// drag a control point (its row) to the page point `at` and glue it to `to`: a connection
/// point of kind 0 or 2 of another shape on the same page (static glue, LOC(PNT(..)) formulas
/// and a Connect entry), or nothing (the point placed as ops::set_control places it).  Invalid:
/// a whole shape, a shape inside this one or around it, an outward point; NoRow: no such
/// connection point.  A point that was glued is let go first.  A GUARD on X or Y keeps the
/// point where it is unglued
pub fn glue_control(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, row: RowKey, at: [f64; 2], to: Target) -> Result<(), EditError> {
    let (t, k) = match to {
        Target::None => return ops::set_control(tx, eng, shape, row, at),
        Target::Shape(_) => return Err(EditError::Invalid),
        Target::Point { shape: t, row: k } => (t, k),
    };
    {
        let doc = tx.doc();
        doc.shape(shape).ok_or(EditError::NoShape)?;
        doc.shape(t).ok_or(EditError::NoShape)?;
        if t.owner != shape.owner || within(doc, t, shape) || within(doc, shape, t) {
            return Err(EditError::Invalid);
        }
        match glue::connection_points(doc, t).iter().find(|p| p.row == k) {
            None => return Err(EditError::NoRow),
            Some(p) if p.kind == 1 => return Err(EditError::Invalid),
            Some(_) => {}
        }
    }
    // placed (and let go when it was glued)
    ops::set_control(tx, eng, shape, row, at)?;
    let doc = tx.doc();
    let id = doc.shape(shape).ok_or(EditError::NoShape)?.id();
    let tid = doc.shape(t).ok_or(EditError::NoShape)?.id();
    let pos = doc.rows(SheetRef::Shape(shape), control_key()).iter().position(|r| r.key == row).ok_or(EditError::NoRow)?;
    let name = control_name(doc, row);
    let sheet = SheetRef::Shape(shape);
    let n = k + 1;
    let f = format!("LOC(PNT(Sheet.{tid}!Connections.X{n},Sheet.{tid}!Connections.Y{n}))");
    let (ax, ay) = (CellAddr::row(control_key(), row, names::X), CellAddr::row(control_key(), row, names::Y));
    match tx.set_formula(sheet, &ax, &f).and_then(|_| tx.set_formula(sheet, &ay, &f)) {
        Ok(()) => {}
        Err(EditError::Guarded) => return Ok(()),
        Err(e) => return Err(e),
    }
    let from_cell = tx.intern(&format!("Controls.{name}"));
    let to_cell = tx.intern(&format!("Connections.X{n}"));
    let c = Connect { from_sheet: id, from_cell: Some(from_cell), from_part: Some(100 + pos as i32), to_sheet: tid, to_cell: Some(to_cell), to_part: Some(100 + k as i32), extras: Extras::new() };
    tx.add_connect(shape.owner, c)?;
    eng.recalc_cells(tx, &[(sheet, ax), (sheet, ay)])?;
    Ok(())
}

/// the control points of a shape that are glued, with their targets (Connect entries from
/// Controls.<row> to a connection point)
pub fn glued_controls(doc: &Document, shape: ShapeRef) -> Vec<(RowKey, Target)> {
    let Some(s) = doc.shape(shape) else { return Vec::new() };
    let id = s.id();
    let rows = doc.rows(SheetRef::Shape(shape), control_key());
    let mut out = Vec::new();
    for c in connects(doc, shape.owner).iter().filter(|c| c.from_sheet == id) {
        let Some(name) = c.from_cell.map(|f| doc.str(f)).and_then(|f| f.strip_prefix("Controls.")) else { continue };
        let Some(r) = rows.iter().find(|r| control_name(doc, r.key) == name) else { continue };
        let Some(to) = doc.shape_by_id(shape.owner, c.to_sheet) else { continue };
        let target = match c.to_part {
            Some(p) if (100..100 + 65536).contains(&p) => Target::Point { shape: to, row: (p - 100) as u32 },
            _ => match c.to_cell.map(|t| doc.str(t)).and_then(|t| t.strip_prefix("Connections.X")).and_then(|k| k.parse::<u32>().ok()) {
                Some(k) if k > 0 => Target::Point { shape: to, row: k - 1 },
                _ => continue,
            },
        };
        out.push((r.key, target));
    }
    out
}

/// the shapes (not among `moved` nor inside them) whose control points are glued to these
/// shapes or to shapes inside them, and those glued to those in turn: their control points
/// moved with the targets (the formulas did it), so their own connectors have to follow
pub fn followers(doc: &Document, owner: Owner, moved: &[ShapeRef]) -> Vec<ShapeRef> {
    let mut ids: BTreeSet<u32> = BTreeSet::new();
    let mut stack: Vec<ShapeRef> = moved.iter().copied().filter(|s| s.owner == owner).collect();
    while let Some(r) = stack.pop() {
        if let Some(s) = doc.shape(r) {
            if ids.insert(s.id()) {
                stack.extend(s.children());
            }
        }
    }
    let mut out: Vec<ShapeRef> = Vec::new();
    // a chain of buses glued to buses: a few rounds at most
    for _ in 0..8 {
        let mut more = false;
        for c in connects(doc, owner) {
            if !ids.contains(&c.to_sheet) || ids.contains(&c.from_sheet) {
                continue;
            }
            if !c.from_cell.is_some_and(|f| doc.str(f).starts_with("Controls.")) {
                continue;
            }
            if let Some(f) = doc.shape_by_id(owner, c.from_sheet) {
                ids.insert(c.from_sheet);
                out.push(f);
                more = true;
            }
        }
        if !more {
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- anchors (CN-02)

// the X and Y formulas of the connection point at an anchor
fn anchor_formulas(doc: &Document, target: ShapeRef, anchor: Anchor) -> Result<(String, String), EditError> {
    let sheet = SheetRef::Shape(target);
    match anchor {
        Anchor::Vertex { section, row } => {
            let sk = SecKey { name: names::GEOMETRY, index: section };
            let r = doc.rows(sheet, sk).into_iter().find(|r| r.key == RowKey::Index(row)).ok_or(EditError::NoRow)?;
            let kind = r.kind.map_or("", |k| doc.str(k));
            if matches!(kind, "" | "Ellipse" | "InfiniteLine") {
                return Err(EditError::Invalid);
            }
            let g = section + 1;
            Ok(if kind.starts_with("Rel") {
                (format!("Geometry{g}.X{row}*Width"), format!("Geometry{g}.Y{row}*Height"))
            } else {
                (format!("Geometry{g}.X{row}"), format!("Geometry{g}.Y{row}"))
            })
        }
        Anchor::Handle { fx, fy } => Ok((format!("Width*{}", text::num(fx)), format!("Height*{}", text::num(fy)))),
        Anchor::Control(row) => {
            if !doc.has_row(sheet, control_key(), row) {
                return Err(EditError::NoRow);
            }
            let n = control_name(doc, row);
            Ok((format!("Controls.{n}"), format!("Controls.{n}.Y")))
        }
    }
}

/// where an anchor of a shape is on the page (None: no such vertex or control point)
pub fn anchor_at(doc: &Document, target: ShapeRef, anchor: Anchor) -> Option<[f64; 2]> {
    let sheet = SheetRef::Shape(target);
    doc.shape(target)?;
    match anchor {
        Anchor::Vertex { section, row } => ops::vertices(doc, target).into_iter().find(|v| v.section == section && v.row == row).map(|v| v.at),
        Anchor::Handle { fx, fy } => {
            let v = |c| doc.resolve(sheet, &CellAddr::single(c)).num().unwrap_or(0.0);
            Some(doc.page_xform(target).apply([v(names::WIDTH) * fx, v(names::HEIGHT) * fy]))
        }
        Anchor::Control(row) => {
            let n = |c| doc.resolve(sheet, &CellAddr::row(control_key(), row, c)).num();
            Some(doc.page_xform(target).apply([n(names::X)?, n(names::Y)?]))
        }
    }
}

/// the connection point of `target` at an anchor: an indexed Connection row whose X and Y have
/// the anchor's formulas, else a new one (DirX, DirY, Type 0) -> its IX.  Invalid: a section of
/// named rows, a row without a point (Ellipse, InfiniteLine)
pub fn anchor_point(tx: &mut Transaction, eng: &mut Engine, target: ShapeRef, anchor: Anchor) -> Result<u32, EditError> {
    let sheet = SheetRef::Shape(target);
    tx.doc().shape(target).ok_or(EditError::NoShape)?;
    let (fx, fy) = anchor_formulas(tx.doc(), target, anchor)?;
    {
        let doc = tx.doc();
        let rows = doc.rows(sheet, conn_key());
        if rows.iter().any(|r| matches!(r.key, RowKey::Named(_))) {
            return Err(EditError::Invalid);
        }
        for r in &rows {
            let RowKey::Index(ix) = r.key else { continue };
            let (x, y) = (CellAddr::row(conn_key(), r.key, names::X), CellAddr::row(conn_key(), r.key, names::Y));
            if formula_of(doc, sheet, &x) == fx && formula_of(doc, sheet, &y) == fy {
                return Ok(ix);
            }
        }
    }
    let rk = tx.add_row(sheet, conn_key(), None, None)?;
    let RowKey::Index(ix) = rk else { return Err(EditError::Invalid) };
    let (ax, ay) = (CellAddr::row(conn_key(), rk, names::X), CellAddr::row(conn_key(), rk, names::Y));
    tx.set_formula(sheet, &ax, &fx)?;
    tx.set_formula(sheet, &ay, &fy)?;
    for c in [names::DIR_X, names::DIR_Y, names::TYPE] {
        tx.set_constant(sheet, &CellAddr::row(conn_key(), rk, c), Value::Num(0.0), None)?;
    }
    eng.recalc_cells(tx, &[(sheet, ax), (sheet, ay)])?;
    Ok(ix)
}

// ---------------------------------------------------------------- guides (CN-02)

// a guide's line on the page: whether it runs across the page (true) or up it (false), and the
// pin it runs through; None: not a guide, or turned some other way, or its line misses its pin
fn guide_line(doc: &Document, g: ShapeRef) -> Option<(bool, [f64; 2])> {
    let s = doc.shape(g)?;
    if s.header().kind != ShapeKind::Guide || s.parent().is_some() || s.is_deleted() {
        return None;
    }
    let sheet = SheetRef::Shape(g);
    let xf = doc.page_xform(g);
    let mut dir = [1.0, 0.0];
    let mut through = [0.0, 0.0];
    if let Some(sk) = doc.geometry(sheet).into_iter().next() {
        for row in doc.rows(sheet, sk) {
            if row.kind.map(|k| doc.str(k)) == Some("InfiniteLine") {
                let c = |n| doc.resolve(sheet, &CellAddr::row(sk, row.key, n)).num().unwrap_or(0.0);
                dir = [c(names::A) - c(names::X), c(names::B) - c(names::Y)];
                through = [c(names::X), c(names::Y)];
                break;
            }
        }
    }
    let d = xf.apply_vec(dir);
    let p = xf.apply(through);
    let pin = [
        doc.resolve(sheet, &CellAddr::single(names::PIN_X)).num().unwrap_or(0.0),
        doc.resolve(sheet, &CellAddr::single(names::PIN_Y)).num().unwrap_or(0.0),
    ];
    let l = (d[0] * d[0] + d[1] * d[1]).max(1e-300);
    let tol = 1e-9 * (1.0 + pin[0].abs() + pin[1].abs());
    if d[1] * d[1] <= 1e-12 * l {
        // across the page: it must run through its pin's height
        ((p[1] - pin[1]).abs() <= tol).then_some((true, pin))
    } else if d[0] * d[0] <= 1e-12 * l {
        ((p[0] - pin[0]).abs() <= tol).then_some((false, pin))
    } else {
        None
    }
}

/// whether `g` is a guide an end can glue to (top-level, across or up the page)
pub fn is_glue_guide(doc: &Document, g: ShapeRef) -> bool {
    guide_line(doc, g).is_some()
}

/// put an end of a 1-D shape on a guide: `at` goes onto the guide's line, the coordinate the
/// guide fixes gets its formula (see the file header), a Connect entry records it; a dynamic
/// connector routes again.  Invalid: not a top-level 1-D shape, not such a guide, another page
pub fn glue_guide(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, end: End, at: [f64; 2], guide: ShapeRef) -> Result<(), EditError> {
    let (across, pin) = {
        let doc = tx.doc();
        let s = doc.shape(shape).ok_or(EditError::NoShape)?;
        if !glue::is_one_d(doc, shape) || s.parent().is_some() || guide.owner != shape.owner {
            return Err(EditError::Invalid);
        }
        guide_line(doc, guide).ok_or(EditError::Invalid)?
    };
    let p = if across { [at[0], pin[1]] } else { [pin[0], at[1]] };
    // the end there, free (its old glue goes)
    glue::glue(tx, eng, shape, end, p, Target::None)?;
    let doc = tx.doc();
    let id = doc.shape(shape).ok_or(EditError::NoShape)?.id();
    let gid = doc.shape(guide).ok_or(EditError::NoShape)?.id();
    let sheet = SheetRef::Shape(shape);
    let (cx, cy, part) = match end {
        End::Begin => (names::BEGIN_X, names::BEGIN_Y, 9),
        End::End => (names::END_X, names::END_Y, 12),
    };
    let (cell, f) = if across { (cy, format!("Sheet.{gid}!PinY")) } else { (cx, format!("Sheet.{gid}!PinX")) };
    let a = CellAddr::single(cell);
    match tx.set_formula(sheet, &a, &f) {
        Ok(()) => {}
        Err(EditError::Guarded) => return Ok(()),
        Err(e) => return Err(e),
    }
    let to_cell = tx.intern("Angle");
    let c = Connect { from_sheet: id, from_cell: Some(cx), from_part: Some(part), to_sheet: gid, to_cell: Some(to_cell), to_part: Some(if across { 2 } else { 1 }), extras: Extras::new() };
    tx.add_connect(shape.owner, c)?;
    eng.recalc_cells(tx, &[(sheet, a)])?;
    glue::update(tx, eng, &[shape])
}

/// the guides the begin and the end of a 1-D shape are glued to (Connect entries from BeginX or
/// EndX to a guide)
pub fn guide_of(doc: &Document, shape: ShapeRef) -> [Option<ShapeRef>; 2] {
    let mut out = [None; 2];
    let Some(s) = doc.shape(shape) else { return out };
    let id = s.id();
    for c in connects(doc, shape.owner).iter().filter(|c| c.from_sheet == id) {
        let end = match c.from_cell {
            Some(n) if n == names::BEGIN_X || n == names::BEGIN_Y => 0,
            Some(n) if n == names::END_X || n == names::END_Y => 1,
            _ => continue,
        };
        if let Some(g) = doc.shape_by_id(shape.owner, c.to_sheet).filter(|&g| doc.shape(g).is_some_and(|x| x.header().kind == ShapeKind::Guide)) {
            out[end] = Some(g);
        }
    }
    out
}

// ---------------------------------------------------------------- the connection point tool

/// the kind (Type) of connection point `row` of a shape: 0 inward, 1 outward, 2 both
pub fn point_kind(doc: &Document, shape: ShapeRef, row: u32) -> Option<u8> {
    glue::connection_points(doc, shape).iter().find(|p| p.row == row).map(|p| p.kind)
}

/// the page point of connection point `row` (None: no such row)
pub fn point_at(doc: &Document, shape: ShapeRef, row: u32) -> Option<[f64; 2]> {
    glue::connection_points(doc, shape).iter().find(|p| p.row == row).map(|p| p.at)
}

/// add a connection point at the page point `at` (the connection point tool's Ctrl+click):
/// X = Width*k, Y = Height*k (a value where the side is 0), DirX, DirY, Type 0 -> its IX.
/// Invalid: a section of named rows, a shape that cannot be turned back (no local point)
pub fn add_point(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, at: [f64; 2]) -> Result<u32, EditError> {
    let sheet = SheetRef::Shape(shape);
    let (q, w, h) = {
        let doc = tx.doc();
        doc.shape(shape).ok_or(EditError::NoShape)?;
        if doc.rows(sheet, conn_key()).iter().any(|r| matches!(r.key, RowKey::Named(_))) {
            return Err(EditError::Invalid);
        }
        let q = doc.page_xform(shape).inverse().ok_or(EditError::Invalid)?.apply(at);
        let v = |c| doc.resolve(sheet, &CellAddr::single(c)).num().unwrap_or(0.0);
        (q, v(names::WIDTH), v(names::HEIGHT))
    };
    let rk = tx.add_row(sheet, conn_key(), None, None)?;
    let RowKey::Index(ix) = rk else { return Err(EditError::Invalid) };
    let mut cells = Vec::new();
    for (c, v, size, side) in [(names::X, q[0], w, "Width"), (names::Y, q[1], h, "Height")] {
        let a = CellAddr::row(conn_key(), rk, c);
        if size.abs() > 1e-9 {
            tx.set_formula(sheet, &a, &format!("{side}*{}", text::num(v / size)))?;
        } else {
            tx.set_constant(sheet, &a, Value::Num(v), None)?;
        }
        cells.push((sheet, a));
    }
    for c in [names::DIR_X, names::DIR_Y, names::TYPE] {
        tx.set_constant(sheet, &CellAddr::row(conn_key(), rk, c), Value::Num(0.0), None)?;
    }
    eng.recalc_cells(tx, &cells)?;
    Ok(ix)
}

/// move connection point `row` to the page point `at`: X and Y keep the form of their formulas
/// (Width*k stays proportional, a value stays a value); what is glued to it follows with the
/// recalculation (ends through glue::update in the app)
pub fn move_point(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, row: u32, at: [f64; 2]) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(shape);
    let sets = {
        let doc = tx.doc();
        doc.shape(shape).ok_or(EditError::NoShape)?;
        let rk = RowKey::Index(row);
        if !doc.has_row(sheet, conn_key(), rk) {
            return Err(EditError::NoRow);
        }
        let q = doc.page_xform(shape).inverse().ok_or(EditError::Invalid)?.apply(at);
        let (ax, ay) = (CellAddr::row(conn_key(), rk, names::X), CellAddr::row(conn_key(), rk, names::Y));
        let px = ops::keep_form(doc, sheet, &ax, q[0]);
        let py = ops::keep_form(doc, sheet, &ay, q[1]);
        alloc::vec![(ax, px), (ay, py)]
    };
    let mut w = Writer::new();
    w.put_all(tx, eng, sheet, sets)?;
    w.finish(tx, eng)
}

/// give connection point `row` the kind `kind` (Type: 0 inward, 1 outward, 2 both)
pub fn set_point_kind(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, row: u32, kind: u8) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(shape);
    if tx.doc().shape(shape).is_none() {
        return Err(EditError::NoShape);
    }
    if kind > 2 {
        return Err(EditError::Invalid);
    }
    let rk = RowKey::Index(row);
    if !tx.doc().has_row(sheet, conn_key(), rk) {
        return Err(EditError::NoRow);
    }
    let mut w = Writer::new();
    w.put(tx, eng, sheet, CellAddr::row(conn_key(), rk, names::TYPE), Put::Num(kind as f64))?;
    w.finish(tx, eng)
}

// what is glued to a connection point of a shape
enum Glued {
    End(ShapeRef, End),
    Control(ShapeRef, RowKey),
}

// what is glued to the connection points of a shape, with the points' rows
fn glued_to(doc: &Document, shape: ShapeRef) -> Vec<(Glued, u32)> {
    let Some(s) = doc.shape(shape) else { return Vec::new() };
    let tid = s.id();
    let mut out = Vec::new();
    for c in connects(doc, shape.owner).iter().filter(|c| c.to_sheet == tid) {
        let Some(row) = c.to_part.filter(|p| (100..100 + 65536).contains(p)).map(|p| (p - 100) as u32) else { continue };
        let Some(from) = doc.shape_by_id(shape.owner, c.from_sheet) else { continue };
        let Some(fc) = c.from_cell else { continue };
        let g = if fc == names::BEGIN_X {
            Glued::End(from, End::Begin)
        } else if fc == names::END_X {
            Glued::End(from, End::End)
        } else if let Some(name) = doc.str(fc).strip_prefix("Controls.") {
            match doc.rows(SheetRef::Shape(from), control_key()).iter().find(|r| control_name(doc, r.key) == name) {
                Some(r) => Glued::Control(from, r.key),
                None => continue,
            }
        } else {
            continue;
        };
        out.push((g, row));
    }
    out
}

/// take connection point `row` out: what is glued to it is let go where it is, the rows after it
/// move one up (Transaction::remove_row) and what is glued to them is glued again under the
/// new number
pub fn delete_point(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, row: u32) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(shape);
    let rk = RowKey::Index(row);
    let (list, at_of) = {
        let doc = tx.doc();
        doc.shape(shape).ok_or(EditError::NoShape)?;
        if !doc.has_row(sheet, conn_key(), rk) {
            return Err(EditError::NoRow);
        }
        let pts = glue::connection_points(doc, shape);
        let list: Vec<(Glued, u32)> = glued_to(doc, shape).into_iter().filter(|(_, r)| *r >= row).collect();
        let at_of: Vec<[f64; 2]> = list.iter().map(|(_, r)| pts.iter().find(|p| p.row == *r).map_or([0.0, 0.0], |p| p.at)).collect();
        (list, at_of)
    };
    // let go of what is glued to this point, and (for now) of what is glued to the later ones
    for ((g, _), at) in list.iter().zip(&at_of) {
        match *g {
            Glued::End(s, e) => glue::glue(tx, eng, s, e, *at, Target::None)?,
            Glued::Control(s, r) => ops::set_control(tx, eng, s, r, *at)?,
        }
    }
    tx.remove_row(sheet, conn_key(), rk)?;
    eng.recalc(tx, &[sheet])?;
    // glued again under the new numbers
    for ((g, r), at) in list.iter().zip(&at_of) {
        if *r == row {
            continue;
        }
        let to = Target::Point { shape, row: r - 1 };
        match *g {
            Glued::End(s, e) => glue::glue(tx, eng, s, e, *at, to)?,
            Glued::Control(s, cr) => glue_control(tx, eng, s, cr, *at, to)?,
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "pointglue_tests.rs"]
mod tests;
