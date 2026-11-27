// glue -- connection points, glue and the dynamic connector (CN-01..05, CN-12, CN-15)
//
//   connection_points(doc, shape) -> Vec<ConnPoint>   the Connection rows on the page: point,
//                                    direction (into the shape, as DirX/DirY mean it), Type
//   glued(doc, shape) -> [Target; 2]  what Begin and End are glued to (the Connect entries; glue
//                                    to a guide reads as None: pointglue.rs)
//   ends(doc, shape)                 Begin and End on the page
//   is_one_d / is_connector          a 1-D shape / one that routes (ObjType routable)
//   style(doc, shape) -> Style       right angle, straight or curved (its cells, else the page's)
//   path(doc, shape) -> points       the drawn way on the page (curves flattened)
//   glue(tx, eng, shape, end, at, to)          put an end at `at` and glue it (Target::None: free)
//   new_connector(tx, eng, owner, (at, to), (at, to)) / new_line(..)   new 1-D shapes
//   dependents(doc, owner, &moved)   the 1-D shapes glued to these shapes (or moved with them)
//   update(tx, eng, &shapes)         their ends follow, connectors route again (one gesture)
//   reset(tx, eng, &connectors)      route again, also where ConFixedCode keeps the way
//   set_style(tx, eng, &connectors, style)
//   detach(tx, eng, owner, &deleted) before deleting shapes: glue to them becomes constants
//
// Every point here is on the page in drawing units (y up); the cells hold the connector's
// parent coordinates (the page, or its group's local ones), the Geometry rows its local ones.
//
// The file's record of glue (CN-15), as Visio writes it ([MS-VSDX] Connect_Type and the cells
// of the 1-D Endpoints, Glue Info and Shape Layout sections; checked against Visio's own files):
// * static glue of Begin to connection point k (row IX k-1) of shape N:
//     BeginX, BeginY = PAR(PNT(Sheet.N!Connections.Xk,Sheet.N!Connections.Yk))
//     BegTrigger = _XFTRIGGER(Sheet.N!EventXFMod)
//     Connect FromSheet=<id> FromCell=BeginX FromPart=9 ToSheet=N ToCell=Connections.Xk
//             ToPart=100+k-1
//   (End: EndX, EndY, EndTrigger, FromCell=EndX, FromPart=12)
// * dynamic glue (to the whole shape): BeginX, BeginY = _WALKGLUE(BegTrigger,EndTrigger,
//   WalkPreference), EndX, EndY = _WALKGLUE(EndTrigger,BegTrigger,WalkPreference), the trigger as
//   above, Connect ToCell=PinX ToPart=3; the engine cannot compute _WALKGLUE, so the value comes
//   from here (Transaction::set_result, the formula stays)
// * a free end: a constant, its trigger _XFTRIGGER(EventXFMod); one Connect per glued end.
// The static formulas the engine computes itself: after the target moved, its recalculation has
// moved the end already; update() asks it again and then routes.
//
// The dynamic connector (ObjType 2 = routable, GlueType 2 = walking glue allowed) keeps its box
// on the line from Begin to End: PinX = GUARD((BeginX+EndX)/2), Width = GUARD(EndX-BeginX),
// LocPinX = GUARD(Width*0.5), Angle = GUARD(0DA), y alike; a side shorter than 5 mm becomes
// GUARD(+-0.19685039370079DL) as in Visio's files.  Its way is one Geometry section: MoveTo at
// Begin, a LineTo per corner, the last at End, constants in local coordinates (the curved style:
// MoveTo and one NURBSTo over the corners).  Controls.TextPosition goes to the middle of the
// way.  Rows of a master keep their indices where their type fits; else they get a Del mark
// and new rows follow.
//
// Dynamic glue docks on the outline of the target (the farthest point of its geometry on a line
// through it).  (JD21v) Where on it, and the way, the connector's routing style decides as Visio
// 16 does (route_styles::route, checked against Visio's own ways): right angles straight across
// in the middle of the overlap of two shapes beside each other, else an L leaving the begin's top
// or bottom; around obstacles in the free part of the sides; else the cheapest way over the sides
// (length + bends; route::best_through, which is also all ends glued to points or free get); where
// the other end lies within a side's reach, the point straight opposite it is offered too.
// (JD15, SS-18) WalkPreference changes none of these ways in Visio: kept, not read.  Obstacles
// are, as in Visio 16 (JD21v), the placeable shapes (ObjType with bit 1, not 4; a group with 8 by
// its placeable members; obstacles()), and for each connector the shapes its ends are glued to;
// the clearance is half the page's AvenueSize (opts_for).  The legs of Visio's right-angle router
// keep out of the way of the other connectors glued to the same shapes (related_lines: Visio
// spreads connectors leaving one side); after a move the connectors a placeable shape now crowds
// route again (crowded).  After routing ConFixedCode becomes 6 or 0 as
// Visio writes it (route_styles::mark).  ConFixedCode 1 and 2 (reroute manually / never) keep the
// corners and only move the first and last pieces; reset() routes them anew.  (JD21) 1 is laid
// anew when the kept way runs through a shape (route_edit.rs); 3 .. 6 route freely (Visio writes
// them for the connectors it routes itself); ShapePermeableX/Y let ways cross a shape.
//
// (JD21v) After routing, the line jumps of the connectors laid and of the connectors they cross
// go into their Geometry (jumps.rs, as Visio writes them); detach takes the jumps over deleted
// connectors away; path() reads a connector's way without them.
//
// Nothing happens on its own: opening a file routes nothing, only the shapes handed to update.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Value};
use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::{Engine, Input};
use crate::geometry::{sqrt, Nurbs, Xform};
use crate::names;
use crate::page::Connect;
use crate::route::{self, Opts, Port};
use crate::shape::{ShapeData, ShapeKind, StyleRefs};
use crate::sheet::{CellAddr, Extras, Row, RowKey, SecKey, Section};
use crate::strings::Sym;

/// a connection point of a shape on the page
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct ConnPoint {
    /// IX of its row (Connections.X<row + 1>)
    pub row: u32,
    /// where it is on the page
    pub at: [f64; 2],
    /// DirX/DirY turned onto the page, unit length: the direction the point faces into the
    /// shape (a connector arrives along it, a route leaves against it); None: 0, 0
    pub dir: Option<[f64; 2]>,
    /// Type: 0 inward, 1 outward, 2 both
    pub kind: u8,
}

/// what an end of a 1-D shape is glued to
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Target {
    /// nothing: a free end
    None,
    /// a connection point (static glue): the shape and the IX of its Connection row
    Point {
        /// the shape
        shape: ShapeRef,
        /// IX of the row
        row: u32,
    },
    /// the whole shape (dynamic glue)
    Shape(ShapeRef),
}

/// an end of a 1-D shape
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum End {
    /// BeginX, BeginY
    Begin,
    /// EndX, EndY
    End,
}

/// how a dynamic connector draws its way
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Style {
    /// right angles around obstacles (the default)
    RightAngle,
    /// one straight line
    Straight,
    /// a curve over the corners of the right-angle way
    Curved,
}

// Visio's smallest side of a connector's box (5 mm)
const MIN_SIDE: f64 = 0.19685039370079;
const WALK_BEGIN: &str = "_WALKGLUE(BegTrigger,EndTrigger,WalkPreference)";
const WALK_END: &str = "_WALKGLUE(EndTrigger,BegTrigger,WalkPreference)";
const FREE_TRIGGER: &str = "_XFTRIGGER(EventXFMod)";
// the sides a dynamic end can dock on, in the order ties are decided (side to side first)
const SIDES: [[f64; 2]; 4] = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];

// ---------------------------------------------------------------- reading

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num()
}

fn num_sym(doc: &Document, sheet: SheetRef, cell: Sym) -> Option<f64> {
    doc.resolve(sheet, &CellAddr::single(cell)).num()
}

fn end_cells(end: End) -> (Sym, Sym) {
    match end {
        End::Begin => (names::BEGIN_X, names::BEGIN_Y),
        End::End => (names::END_X, names::END_Y),
    }
}

fn trigger_name(end: End) -> &'static str {
    match end {
        End::Begin => "BegTrigger",
        End::End => "EndTrigger",
    }
}

fn connects(doc: &Document, owner: Owner) -> &[Connect] {
    match owner {
        Owner::Page(p) => doc.page(p).map_or(&[][..], |x| x.connects.as_slice()),
        Owner::Master(m) => doc.master(m).map_or(&[][..], |x| x.connects.as_slice()),
    }
}

// the transform of the shape's parent to the page (identity at the top level)
fn parent_xform(doc: &Document, shape: ShapeRef) -> Xform {
    match doc.shape(shape).and_then(|s| s.parent()) {
        Some(p) => doc.page_xform(p),
        None => Xform::IDENTITY,
    }
}

fn unit(v: [f64; 2]) -> Option<[f64; 2]> {
    let l = sqrt(v[0] * v[0] + v[1] * v[1]);
    (l.is_finite() && l > 1e-12).then(|| [v[0] / l, v[1] / l])
}

/// the connection points of a shape (its Connection rows with inheritance) on the page
pub fn connection_points(doc: &Document, shape: ShapeRef) -> Vec<ConnPoint> {
    let sheet = SheetRef::Shape(shape);
    let sk = SecKey::one(names::CONNECTION);
    if doc.shape(shape).is_none() {
        return Vec::new();
    }
    let m = doc.page_xform(shape);
    let mut out = Vec::new();
    for r in doc.rows(sheet, sk) {
        let RowKey::Index(ix) = r.key else { continue };
        let n = |c: Sym| doc.resolve(sheet, &CellAddr::row(sk, r.key, c)).num();
        let at = m.apply([n(names::X).unwrap_or(0.0), n(names::Y).unwrap_or(0.0)]);
        let dir = match (n(names::DIR_X), n(names::DIR_Y)) {
            (None, None) => None,
            (dx, dy) => unit(m.apply_vec([dx.unwrap_or(0.0), dy.unwrap_or(0.0)])),
        };
        let kind = n(names::TYPE).map_or(0, |t| t.clamp(0.0, 2.0) as u8);
        out.push(ConnPoint { row: ix, at, dir, kind });
    }
    out
}

/// whether the shape is 1-D (it has BeginX)
pub fn is_one_d(doc: &Document, shape: ShapeRef) -> bool {
    doc.shape(shape).is_some() && doc.resolve(SheetRef::Shape(shape), &CellAddr::single(names::BEGIN_X)).exists()
}

/// whether the shape is a dynamic connector: 1-D and routable (ObjType has the routable flag)
pub fn is_connector(doc: &Document, shape: ShapeRef) -> bool {
    is_one_d(doc, shape)
        && num(doc, SheetRef::Shape(shape), "ObjType").is_some_and(|v| {
            let f = v as i64;
            f & 2 != 0 && f & 4 == 0
        })
}

// GlueType with walking glue: a 1-D shape that is no connector may glue to whole shapes
fn walking(doc: &Document, shape: ShapeRef) -> bool {
    num(doc, SheetRef::Shape(shape), "GlueType").is_some_and(|v| (v as i64) & 2 != 0)
}

/// Begin and End of a 1-D shape on the page
pub fn ends(doc: &Document, shape: ShapeRef) -> Option<([f64; 2], [f64; 2])> {
    if !is_one_d(doc, shape) {
        return None;
    }
    let sheet = SheetRef::Shape(shape);
    let v = |c: Sym| num_sym(doc, sheet, c).unwrap_or(0.0);
    let m = parent_xform(doc, shape);
    Some((m.apply([v(names::BEGIN_X), v(names::BEGIN_Y)]), m.apply([v(names::END_X), v(names::END_Y)])))
}

fn target_of(doc: &Document, c: &Connect, to: ShapeRef) -> Target {
    match c.to_part {
        Some(3) => return Target::Shape(to),
        // (JD21 glue) a guide (visGuideX, visGuideY, visGuideIntersect): pointglue.rs
        Some(1 | 2 | 4) => return Target::None,
        Some(p) if (100..100 + 65536).contains(&p) => return Target::Point { shape: to, row: (p - 100) as u32 },
        _ => {}
    }
    let cell = c.to_cell.map_or("", |s| doc.str(s));
    if cell == "PinX" {
        return Target::Shape(to);
    }
    match cell.strip_prefix("Connections.X").and_then(|k| k.parse::<u32>().ok()) {
        Some(k) if k > 0 => Target::Point { shape: to, row: k - 1 },
        _ => Target::None,
    }
}

/// what Begin and End are glued to, from the Connect entries of the page or master (glue to
/// something else -- a guide, a vertex -- reads as None)
pub fn glued(doc: &Document, shape: ShapeRef) -> [Target; 2] {
    let mut out = [Target::None; 2];
    let Some(s) = doc.shape(shape) else { return out };
    let id = s.id();
    for c in connects(doc, shape.owner) {
        if c.from_sheet != id {
            continue;
        }
        let end = match c.from_cell {
            Some(n) if n == names::BEGIN_X => 0,
            Some(n) if n == names::END_X => 1,
            _ => continue,
        };
        if let Some(to) = doc.shape_by_id(shape.owner, c.to_sheet) {
            out[end] = target_of(doc, c, to);
        }
    }
    out
}

/// how a dynamic connector draws its way: ConLineRouteExt 2 curved, ShapeRouteStyle 2 or 16
/// straight, else right angles; 0 asks the page (LineRouteExt, RouteStyle)
pub fn style(doc: &Document, shape: ShapeRef) -> Style {
    let sheet = SheetRef::Shape(shape);
    let page = match shape.owner {
        Owner::Page(p) => Some(SheetRef::Page(p)),
        Owner::Master(_) => None,
    };
    let get = |own: &str, of_page: &str| num(doc, sheet, own).filter(|v| *v != 0.0).or_else(|| page.and_then(|p| num(doc, p, of_page)));
    if get("ConLineRouteExt", "LineRouteExt") == Some(2.0) {
        return Style::Curved;
    }
    match get("ShapeRouteStyle", "RouteStyle") {
        // (JD21v) network (9) routes with right angles in Visio (route_styles.rs)
        Some(v) if v == 2.0 || v == 16.0 => Style::Straight,
        _ => Style::RightAngle,
    }
}

/// the drawn way of a shape on the page: the first shown Geometry section, curves flattened;
/// (JD21 jumps) a connector's way without its line jumps (jumps::way)
pub fn path(doc: &Document, shape: ShapeRef) -> Vec<[f64; 2]> {
    let mut out = Vec::new();
    if doc.shape(shape).is_none() {
        return out;
    }
    if let Some(w) = crate::jumps::way(doc, shape) {
        return w;
    }
    let sheet = SheetRef::Shape(shape);
    let (w, h) = (num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
    let m = doc.page_xform(shape);
    if let Some(g) = doc.geometry_paths(sheet, w, h).into_iter().find(|g| !g.no_show) {
        for sub in &g.path.subs {
            let mut pts = Vec::new();
            sub.flatten(16, &mut pts);
            out.extend(pts.into_iter().map(|p| m.apply(p)));
        }
    }
    out
}

// the box of a shape on the page
pub(crate) fn page_box(doc: &Document, s: ShapeRef) -> Option<[f64; 4]> {
    let sheet = SheetRef::Shape(s);
    let (w, h) = (num_sym(doc, sheet, names::WIDTH)?, num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
    let m = doc.page_xform(s);
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]] {
        let p = m.apply(c);
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b.iter().all(|v| v.is_finite()).then_some(b)
}

pub(crate) fn centre(doc: &Document, s: ShapeRef) -> [f64; 2] {
    let sheet = SheetRef::Shape(s);
    let (w, h) = (num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
    doc.page_xform(s).apply([w * 0.5, h * 0.5])
}

// the outline of a shape and its subshapes on the page, as segments
pub(crate) fn outline(doc: &Document, s: ShapeRef) -> Vec<[[f64; 2]; 2]> {
    let mut segs = Vec::new();
    let mut stack = alloc::vec![s];
    while let Some(r) = stack.pop() {
        let Some(shape) = doc.shape(r) else { continue };
        stack.extend(shape.children());
        if shape.is_deleted() || (r != s && is_one_d(doc, r)) {
            continue;
        }
        let sheet = SheetRef::Shape(r);
        let (w, h) = (num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
        let m = doc.page_xform(r);
        for g in doc.geometry_paths(sheet, w, h) {
            if g.no_show {
                continue;
            }
            for sub in &g.path.subs {
                let mut pts = Vec::new();
                sub.flatten(8, &mut pts);
                for p in pts.windows(2) {
                    segs.push([m.apply(p[0]), m.apply(p[1])]);
                }
            }
        }
    }
    segs
}

// where the line c + t d meets the segment p-q -> t
fn line_hit(c: [f64; 2], d: [f64; 2], p: [f64; 2], q: [f64; 2]) -> Option<f64> {
    let e = [q[0] - p[0], q[1] - p[1]];
    let den = d[0] * e[1] - d[1] * e[0];
    if den.abs() <= 1e-15 {
        return None;
    }
    let w = [p[0] - c[0], p[1] - c[1]];
    let t = (w[0] * e[1] - w[1] * e[0]) / den;
    let u = (w[0] * d[1] - w[1] * d[0]) / den;
    (-1e-9..=1.0 + 1e-9).contains(&u).then_some(t)
}

// where a dynamic end docks: the farthest point of the outline in direction d on the line
// through the middle (also behind the middle: a shape open on that side), else where the line
// leaves the box
pub(crate) fn dock(segs: &[[[f64; 2]; 2]], c: [f64; 2], d: [f64; 2], own: Option<[f64; 4]>) -> [f64; 2] {
    let Some(d) = unit(d) else { return c };
    let far = segs.iter().filter_map(|s| line_hit(c, d, s[0], s[1])).fold(None::<f64>, |m, t| Some(m.map_or(t, |m| m.max(t))));
    let t = match (far, own) {
        (Some(t), _) => t,
        (None, Some(b)) => {
            let mut t = f64::INFINITY;
            for (dv, lo, hi, cv) in [(d[0], b[0], b[2], c[0]), (d[1], b[1], b[3], c[1])] {
                if dv > 1e-12 {
                    t = t.min((hi - cv) / dv);
                } else if dv < -1e-12 {
                    t = t.min((lo - cv) / dv);
                }
            }
            if t.is_finite() { t.max(0.0) } else { 0.0 }
        }
        (None, None) => 0.0,
    };
    [c[0] + d[0] * t, c[1] + d[1] * t]
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

// the IDs of these shapes and of everything inside them
fn subtree_ids(doc: &Document, owner: Owner, shapes: &[ShapeRef]) -> BTreeSet<u32> {
    let mut ids = BTreeSet::new();
    let mut stack: Vec<ShapeRef> = shapes.iter().copied().filter(|s| s.owner == owner).collect();
    while let Some(r) = stack.pop() {
        if let Some(s) = doc.shape(r) {
            if ids.insert(s.id()) {
                stack.extend(s.children());
            }
        }
    }
    ids
}

// ---------------------------------------------------------------- glue

/// put an end of a 1-D shape at `at` (page coordinates) and glue it to `to`: a connection
/// point (static: PAR(PNT(..)) formulas), a whole shape (dynamic, connectors and walking glue
/// only: _WALKGLUE), or nothing (constants); the Connect entries of the end are replaced.  A
/// dynamic connector routes again.  Invalid: not 1-D, a target on another page or inside the
/// shape, dynamic glue for a plain line; NoRow: no such connection point
pub fn glue(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, end: End, at: [f64; 2], to: Target) -> Result<(), EditError> {
    set_end(tx, eng, shape, end, at, to)?;
    update(tx, eng, &[shape])
}

fn set_end(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, end: End, at: [f64; 2], to: Target) -> Result<(), EditError> {
    let doc = tx.doc();
    let id = doc.shape(shape).ok_or(EditError::NoShape)?.id();
    if !is_one_d(doc, shape) {
        return Err(EditError::Invalid);
    }
    let owner = shape.owner;
    let tid = match to {
        Target::None => 0,
        Target::Point { shape: t, .. } | Target::Shape(t) => {
            if t.owner != owner || within(doc, t, shape) {
                return Err(EditError::Invalid);
            }
            doc.shape(t).ok_or(EditError::NoShape)?.id()
        }
    };
    match to {
        Target::Point { shape: t, row } if !connection_points(doc, t).iter().any(|p| p.row == row) => return Err(EditError::NoRow),
        Target::Shape(_) if !(is_connector(doc, shape) || walking(doc, shape)) => return Err(EditError::Invalid),
        _ => {}
    }
    let p = parent_xform(doc, shape).inverse().unwrap_or(Xform::IDENTITY).apply(at);
    let (cx, cy) = end_cells(end);
    // the old Connect entries of this end go (JD21: an end on a guide has one from each cell)
    let old: Vec<usize> = connects(doc, owner)
        .iter()
        .enumerate()
        .filter(|(_, c)| c.from_sheet == id && (c.from_cell == Some(cx) || c.from_cell == Some(cy)))
        .map(|(i, _)| i)
        .collect();
    for i in old.into_iter().rev() {
        tx.remove_connect(owner, i)?;
    }
    let sheet = SheetRef::Shape(shape);
    let (ax, ay) = (CellAddr::single(cx), CellAddr::single(cy));
    let trig = tx.single(trigger_name(end));
    tx.intern("EventXFMod");
    let part = if end == End::Begin { 9 } else { 12 };
    match to {
        Target::None => {
            let (ux, uy) = (tx.doc().resolve(sheet, &ax).unit(), tx.doc().resolve(sheet, &ay).unit());
            eng.user_set(tx, sheet, &ax, Input::Value(Value::Num(p[0]), ux))?;
            eng.user_set(tx, sheet, &ay, Input::Value(Value::Num(p[1]), uy))?;
            tx.set_formula(sheet, &trig, FREE_TRIGGER)?;
        }
        Target::Point { row, .. } => {
            let k = row + 1;
            let f = format!("PAR(PNT(Sheet.{tid}!Connections.X{k},Sheet.{tid}!Connections.Y{k}))");
            tx.set_formula(sheet, &ax, &f)?;
            tx.set_formula(sheet, &ay, &f)?;
            tx.set_formula(sheet, &trig, &format!("_XFTRIGGER(Sheet.{tid}!EventXFMod)"))?;
            let to_cell = tx.intern(&format!("Connections.X{k}"));
            let c = Connect {
                from_sheet: id,
                from_cell: Some(cx),
                from_part: Some(part),
                to_sheet: tid,
                to_cell: Some(to_cell),
                to_part: Some(100 + row as i32),
                extras: Extras::new(),
            };
            tx.add_connect(owner, c)?;
        }
        Target::Shape(_) => {
            let f = if end == End::Begin { WALK_BEGIN } else { WALK_END };
            tx.set_formula(sheet, &ax, f)?;
            tx.set_formula(sheet, &ay, f)?;
            tx.set_result(sheet, &ax, Value::Num(p[0]), None)?;
            tx.set_result(sheet, &ay, Value::Num(p[1]), None)?;
            tx.set_formula(sheet, &trig, &format!("_XFTRIGGER(Sheet.{tid}!EventXFMod)"))?;
            let c = Connect {
                from_sheet: id,
                from_cell: Some(cx),
                from_part: Some(part),
                to_sheet: tid,
                to_cell: Some(names::PIN_X),
                to_part: Some(3),
                extras: Extras::new(),
            };
            tx.add_connect(owner, c)?;
        }
    }
    eng.recalc_cells(tx, &[(sheet, ax), (sheet, ay), (sheet, trig)])?;
    // (JD21) a dynamic connector glued to a 2-D shape whose ObjType lets Visio decide (0) makes it
    // placeable (1), as Visio does (Visio check visio-run4 [objtype]: a drawn rectangle 0 -> 1 when
    // a connector's end is glued to it); Visio's Re-Layout Page moves only such shapes
    if let Target::Point { shape: t, .. } | Target::Shape(t) = to {
        let doc = tx.doc();
        let ts = SheetRef::Shape(t);
        if is_connector(doc, shape) && !is_one_d(doc, t) && num(doc, ts, "ObjType").unwrap_or(0.0) == 0.0 {
            let a = CellAddr::single(tx.intern("ObjType"));
            tx.set_constant(ts, &a, Value::Num(1.0), None)?;
        }
    }
    Ok(())
}

/// the 1-D shapes to update after these shapes moved (or were resized, turned ...): the ones
/// glued to them or to shapes inside them, and glued ones among them; in Connect order
pub fn dependents(doc: &Document, owner: Owner, moved: &[ShapeRef]) -> Vec<ShapeRef> {
    let ids = subtree_ids(doc, owner, moved);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for c in connects(doc, owner) {
        if !(ids.contains(&c.to_sheet) || ids.contains(&c.from_sheet)) {
            continue;
        }
        if let Some(f) = doc.shape_by_id(owner, c.from_sheet) {
            if is_one_d(doc, f) && seen.insert(f) {
                out.push(f);
            }
        }
    }
    out
}

/// before shapes are deleted: the glue of other shapes to them (or to shapes inside them)
/// ends -- the glued cells keep their values as constants, triggers point to the shape itself,
/// the Connect entries go (Transaction::delete_shapes removes the deleted shapes' own)
pub fn detach(tx: &mut Transaction, eng: &mut Engine, owner: Owner, deleted: &[ShapeRef]) -> Result<(), EditError> {
    // (JD21 jumps) the line jumps over their connectors go
    crate::jumps::forget(tx, eng, deleted)?;
    let ids = subtree_ids(tx.doc(), owner, deleted);
    let list: Vec<(usize, Connect)> = connects(tx.doc(), owner)
        .iter()
        .enumerate()
        .filter(|(_, c)| ids.contains(&c.to_sheet) && !ids.contains(&c.from_sheet))
        .map(|(i, c)| (i, c.clone()))
        .collect();
    let mut cells: Vec<(SheetRef, CellAddr)> = Vec::new();
    for (i, c) in list.iter().rev() {
        tx.remove_connect(owner, *i)?;
        let Some(f) = tx.doc().shape_by_id(owner, c.from_sheet) else { continue };
        let sheet = SheetRef::Shape(f);
        let from = String::from(c.from_cell.map_or("", |s| tx.doc().str(s)));
        let (addrs, trig) = match from.as_str() {
            "BeginX" => (alloc::vec![CellAddr::single(names::BEGIN_X), CellAddr::single(names::BEGIN_Y)], Some(End::Begin)),
            "EndX" => (alloc::vec![CellAddr::single(names::END_X), CellAddr::single(names::END_Y)], Some(End::End)),
            s => match s.strip_prefix("Controls.") {
                Some(row) => {
                    let rk = RowKey::Named(tx.intern(row));
                    let sk = SecKey::one(names::CONTROL);
                    (alloc::vec![CellAddr::row(sk, rk, names::X), CellAddr::row(sk, rk, names::Y)], None)
                }
                None => continue,
            },
        };
        for a in addrs {
            let r = tx.doc().resolve(sheet, &a);
            if matches!(r.formula(), Formula::Expr(_)) {
                let (v, u) = (r.value(), r.unit());
                tx.force_constant(sheet, &a, v, u)?;
                cells.push((sheet, a));
            }
        }
        if let Some(e) = trig {
            let a = tx.single(trigger_name(e));
            tx.force_formula(sheet, &a, FREE_TRIGGER)?;
            cells.push((sheet, a));
        }
    }
    if !cells.is_empty() {
        tx.intern("EventXFMod");
        eng.recalc_cells(tx, &cells)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- new shapes

fn cnum(tx: &mut Transaction, name: &str, v: f64) -> Cell {
    Cell::constant(tx.intern(name), Value::Num(v), None)
}

fn cform(tx: &mut Transaction, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (tx.intern(name), tx.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: true }
}

fn row_of(key: RowKey, kind: Option<Sym>, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(key);
    r.kind = kind;
    r.cells = cells;
    r
}

// a Geometry section: no fill, a MoveTo and a LineTo
fn geometry_section(tx: &mut Transaction, a: (Cell, Cell), b: (Cell, Cell)) -> Section {
    let mut sec = Section::new(SecKey::one(names::GEOMETRY));
    sec.ix_written = true;
    for (n, v) in [(names::NO_FILL, 1.0), (names::NO_LINE, 0.0), (names::NO_SHOW, 0.0), (names::NO_SNAP, 0.0), (names::NO_QUICK_DRAG, 0.0)] {
        sec.cells.push(Cell::constant(n, Value::Num(v), None));
    }
    let _ = tx;
    sec.rows.push(row_of(RowKey::Index(1), Some(names::MOVE_TO), alloc::vec![a.0, a.1]));
    sec.rows.push(row_of(RowKey::Index(2), Some(names::LINE_TO), alloc::vec![b.0, b.1]));
    sec
}

fn style_id(doc: &Document, name_u: &str) -> Option<u32> {
    doc.style_by_name_u(name_u).and_then(|s| doc.style(s)).map(|s| s.header.id)
}

// glue both ends of a new 1-D shape, then route or recalculate it
pub(crate) fn finish_new(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, begin: ([f64; 2], Target), end: ([f64; 2], Target)) -> Result<ShapeRef, EditError> {
    eng.recalc(tx, &[SheetRef::Shape(s)])?;
    for (e, (at, to)) in [(End::Begin, begin), (End::End, end)] {
        if to != Target::None {
            set_end(tx, eng, s, e, at, to)?;
        }
    }
    update(tx, eng, &[s])?;
    Ok(s)
}

/// add a dynamic connector on a page or in a master (no master shape: every cell of its own,
/// the style "Connector" when the document has one) from `begin` to `end` (page coordinates)
/// glued to the targets, routed -> its handle.  On an error the transaction holds what was
/// done so far: drop it
pub fn new_connector(tx: &mut Transaction, eng: &mut Engine, owner: Owner, begin: ([f64; 2], Target), end: ([f64; 2], Target)) -> Result<ShapeRef, EditError> {
    let (b, e) = (begin.0, end.0);
    let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
    let mut d = ShapeData::new(ShapeKind::Shape);
    if let Some(id) = style_id(tx.doc(), "Connector") {
        d.header.styles = StyleRefs::all(id);
    }
    let cells = alloc::vec![
        cform(tx, "PinX", "GUARD((BeginX+EndX)/2)", (b[0] + e[0]) * 0.5),
        cform(tx, "PinY", "GUARD((BeginY+EndY)/2)", (b[1] + e[1]) * 0.5),
        cform(tx, "Width", "GUARD(EndX-BeginX)", dx),
        cform(tx, "Height", "GUARD(EndY-BeginY)", dy),
        cform(tx, "LocPinX", "GUARD(Width*0.5)", dx * 0.5),
        cform(tx, "LocPinY", "GUARD(Height*0.5)", dy * 0.5),
        cform(tx, "Angle", "GUARD(0DA)", 0.0),
        cform(tx, "FlipX", "GUARD(FALSE)", 0.0),
        cform(tx, "FlipY", "GUARD(FALSE)", 0.0),
        cnum(tx, "BeginX", b[0]),
        cnum(tx, "BeginY", b[1]),
        cnum(tx, "EndX", e[0]),
        cnum(tx, "EndY", e[1]),
        cform(tx, "TxtPinX", "SETATREF(Controls.TextPosition)", dx * 0.5),
        cform(tx, "TxtPinY", "SETATREF(Controls.TextPosition.Y)", dy * 0.5),
        cform(tx, "TxtWidth", "MAX(TEXTWIDTH(TheText),5*Char.Size)", 0.0),
        cform(tx, "TxtHeight", "TEXTHEIGHT(TheText,TxtWidth)", 0.0),
        cform(tx, "TxtLocPinX", "TxtWidth*0.5", 0.0),
        cform(tx, "TxtLocPinY", "TxtHeight*0.5", 0.0),
        cnum(tx, "TxtAngle", 0.0),
        cnum(tx, "LockHeight", 1.0),
        cnum(tx, "LockCalcWH", 1.0),
        cnum(tx, "NoAlignBox", 1.0),
        cnum(tx, "DynFeedback", 2.0),
        cnum(tx, "GlueType", 2.0),
        cnum(tx, "ObjType", 2.0),
        cnum(tx, "NoLiveDynamics", 1.0),
        cnum(tx, "ShapeSplittable", 1.0),
        cform(tx, "BegTrigger", FREE_TRIGGER, 1.0),
        cform(tx, "EndTrigger", FREE_TRIGGER, 1.0),
    ];
    d.sheet.cells = cells;
    tx.intern("EventXFMod");
    let tp = RowKey::Named(tx.intern("TextPosition"));
    let ctl = alloc::vec![
        cnum(tx, "X", dx * 0.5),
        cnum(tx, "Y", dy * 0.5),
        cform(tx, "XDyn", "Controls.TextPosition", dx * 0.5),
        cform(tx, "YDyn", "Controls.TextPosition.Y", dy * 0.5),
        cform(tx, "XCon", "IF(OR(STRSAME(SHAPETEXT(TheText),\"\"),HideText),5,0)", 5.0),
        cnum(tx, "YCon", 0.0),
        cnum(tx, "CanGlue", 0.0),
    ];
    let mut control = Section::new(SecKey::one(names::CONTROL));
    control.rows.push(row_of(tp, None, ctl));
    d.sheet.sections.push(control);
    let a = (Cell::constant(names::X, Value::Num(0.0), None), Cell::constant(names::Y, Value::Num(0.0), None));
    let z = (Cell::constant(names::X, Value::Num(dx), None), Cell::constant(names::Y, Value::Num(dy), None));
    let geo = geometry_section(tx, a, z);
    d.sheet.sections.push(geo);
    let s = tx.add_shape_with(owner, None, d)?;
    finish_new(tx, eng, s, begin, end)
}

/// add a straight line (1-D, not routed: CN-12) from `begin` to `end`, glued to connection
/// points (Target::Shape is refused: lines glue statically); its Geometry follows Width, so it
/// keeps its form when an end moves -> its handle
pub fn new_line(tx: &mut Transaction, eng: &mut Engine, owner: Owner, begin: ([f64; 2], Target), end: ([f64; 2], Target)) -> Result<ShapeRef, EditError> {
    let (b, e) = (begin.0, end.0);
    let (dx, dy) = (e[0] - b[0], e[1] - b[1]);
    let len = sqrt(dx * dx + dy * dy);
    let mut d = ShapeData::new(ShapeKind::Shape);
    let st = tx.doc().settings.clone();
    let known = |id: Option<u32>| id.filter(|&i| tx.doc().style_by_id(i).is_some());
    d.header.styles = StyleRefs { line: known(st.default_line_style), fill: known(st.default_fill_style), text: known(st.default_text_style) };
    let cells = alloc::vec![
        cform(tx, "PinX", "(BeginX+EndX)/2", (b[0] + e[0]) * 0.5),
        cform(tx, "PinY", "(BeginY+EndY)/2", (b[1] + e[1]) * 0.5),
        cform(tx, "Width", "SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)", len),
        cnum(tx, "Height", 0.0),
        cform(tx, "LocPinX", "Width*0.5", len * 0.5),
        cform(tx, "LocPinY", "Height*0.5", 0.0),
        cform(tx, "Angle", "ATAN2(EndY-BeginY,EndX-BeginX)", crate::geometry::atan2(dy, dx)),
        cnum(tx, "BeginX", b[0]),
        cnum(tx, "BeginY", b[1]),
        cnum(tx, "EndX", e[0]),
        cnum(tx, "EndY", e[1]),
    ];
    d.sheet.cells = cells;
    let a = (cform(tx, "X", "Width*0", 0.0), cform(tx, "Y", "Height*0.5", 0.0));
    let z = (cform(tx, "X", "Width*1", len), cform(tx, "Y", "Height*0.5", 0.0));
    let geo = geometry_section(tx, a, z);
    d.sheet.sections.push(geo);
    let s = tx.add_shape_with(owner, None, d)?;
    finish_new(tx, eng, s, begin, end)
}

// ---------------------------------------------------------------- update and routing

/// after a gesture: the ends of these 1-D shapes follow what they are glued to (static ends
/// through the engine, dynamic ones dock anew) and dynamic connectors route again; lines keep
/// their form.  Only these shapes change
pub fn update(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef]) -> Result<(), EditError> {
    run(tx, eng, shapes, false)
}

/// "Verbinder zurücksetzen": route these connectors anew with few bends, also the ones whose
/// way ConFixedCode keeps
pub fn reset(tx: &mut Transaction, eng: &mut Engine, connectors: &[ShapeRef]) -> Result<(), EditError> {
    run(tx, eng, connectors, true)
}

/// give these connectors a style (ShapeRouteStyle and ConLineRouteExt: right angle 1/1,
/// straight 16/1, curved 1/2) and route them anew
pub fn set_style(tx: &mut Transaction, eng: &mut Engine, connectors: &[ShapeRef], style: Style) -> Result<(), EditError> {
    let (rs, ext) = match style {
        Style::RightAngle => (1.0, 1.0),
        Style::Straight => (16.0, 1.0),
        Style::Curved => (1.0, 2.0),
    };
    let (a, b) = (tx.single("ShapeRouteStyle"), tx.single("ConLineRouteExt"));
    for &c in connectors {
        if !is_connector(tx.doc(), c) {
            continue;
        }
        let sheet = SheetRef::Shape(c);
        tx.set_constant(sheet, &a, Value::Num(rs), None)?;
        tx.set_constant(sheet, &b, Value::Num(ext), None)?;
    }
    reset(tx, eng, connectors)
}

// the obstacles of a page or master, as Visio 16 routes around them (JD21v, the counter-check
// visio-obst-1): the boxes of the top-level shapes whose ObjType has the placeable flag (1, 9;
// a 1-D shape that is no connector too), not 0 (Visio decides: a shape becomes 1 when a
// connector is glued to it, set_end), 2, 8 or 4 ("do not lay out and route around", CN-17; it
// wins); a group with the group flag (8) and without the placeable one stands for its placeable
// members (and so on down).  Never guides, connectors, shapes permeable both ways
// (ShapePermeableX and Y) or containers (JD10: their members sit inside them; Visio's own
// containers are 0, a placeable member inside one is routed around)
fn obstacles(doc: &Document, owner: Owner) -> Vec<(ShapeRef, [f64; 4])> {
    let mut out = Vec::new();
    for s in doc.top_shapes(owner) {
        obstacle_into(doc, s, &mut out);
    }
    out
}

fn obstacle_into(doc: &Document, s: ShapeRef, out: &mut Vec<(ShapeRef, [f64; 4])>) {
    let Some(shape) = doc.shape(s) else { return };
    if shape.is_deleted() || shape.header().kind == ShapeKind::Guide || is_connector(doc, s) || !may_block(doc, s) {
        return;
    }
    let obj = num(doc, SheetRef::Shape(s), "ObjType").map_or(0, |v| v as i64);
    if obj & 1 != 0 {
        if let Some(b) = page_box(doc, s) {
            out.push((s, b));
        }
    } else if obj & 8 != 0 {
        for m in shape.children() {
            obstacle_into(doc, m, out);
        }
    }
}

/// (JD21v) the dynamic connectors whose way runs through one of these shapes, or closer to it than
/// the clearance (half the page's AvenueSize), where the shape keeps connectors out (an obstacle:
/// placeable, ObjType 1; a group with 8 by its placeable members): after the shapes moved Visio
/// routes them again (the counter-check visio-obst-1 [sprd] "beside": a way 0.05 in under a shape
/// moved next to it went round it anew); the shapes themselves are not in the list
pub fn crowded(doc: &Document, owner: Owner, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    let mut boxes = Vec::new();
    for &s in shapes {
        obstacle_into(doc, s, &mut boxes);
    }
    if boxes.is_empty() {
        return Vec::new();
    }
    let c = opts_for(doc, owner).clearance;
    let grown: Vec<[f64; 4]> = boxes.iter().map(|(_, b)| [b[0] - c, b[1] - c, b[2] + c, b[3] + c]).collect();
    doc.top_shapes(owner).filter(|&k| !shapes.contains(&k) && is_connector(doc, k) && path(doc, k).windows(2).any(|w| grown.iter().any(|b| route::hits_box(w[0], w[1], b)))).collect()
}

// whether a shape may keep connectors out at all: not ObjType 4, not a container, not permeable
// both ways
fn may_block(doc: &Document, s: ShapeRef) -> bool {
    let sheet = SheetRef::Shape(s);
    let perm = |c: &str| num(doc, sheet, c).is_some_and(|v| v != 0.0);
    let obj = num(doc, sheet, "ObjType").map_or(0, |v| v as i64);
    obj & 4 == 0 && !crate::structure::is_container(doc, s) && !(perm("ShapePermeableX") && perm("ShapePermeableY"))
}

// (JD21v) the straight pieces of the other dynamic connectors glued to the shapes this one is
// glued to: Visio's right-angle router lays each leg in the middle of the part of its band they
// leave free (route_styles::free; the counter-check visio-obst-1 [sprd]: connectors between other
// shapes did not count)
fn related_lines(doc: &Document, c: ShapeRef, targets: &[Target; 2]) -> Vec<[[f64; 2]; 2]> {
    let mut out = Vec::new();
    let Some(own) = doc.shape(c).map(|s| s.id()) else { return out };
    let ids: Vec<u32> = targets
        .iter()
        .filter_map(|t| match *t {
            Target::Shape(s) | Target::Point { shape: s, .. } => doc.shape(s).map(|x| x.id()),
            Target::None => None,
        })
        .collect();
    if ids.is_empty() {
        return out;
    }
    let mut seen = BTreeSet::new();
    for k in connects(doc, c.owner) {
        if k.from_sheet == own || !ids.contains(&k.to_sheet) || !seen.insert(k.from_sheet) {
            continue;
        }
        let Some(o) = doc.shape_by_id(c.owner, k.from_sheet) else { continue };
        if !is_connector(doc, o) {
            continue;
        }
        let way = path(doc, o);
        for s in way.windows(2) {
            let t = 1e-9 * (1.0 + s[0][0].abs().max(s[0][1].abs()));
            let (dx, dy) = ((s[0][0] - s[1][0]).abs(), (s[0][1] - s[1][1]).abs());
            if (dx <= t) != (dy <= t) {
                out.push([s[0], s[1]]);
            }
        }
    }
    out
}

// the clearance and the cost of a bend (1/2 inch), both on paper, in drawing units.  (JD21v) The
// clearance is half the page's AvenueSizeX/Y (Visio's default 7.5 mm where the page has none):
// Visio keeps that much from obstacles and lays the stubs of its styles so (route_styles.rs;
// AvenueSize 0.5 in gave 0.25 in, 0.1 in 0.05 in; LineToNode 0.4 in changed nothing, so it is
// not read here)
pub(crate) fn opts_for(doc: &Document, owner: Owner) -> Opts {
    let Owner::Page(p) = owner else { return Opts::default() };
    let sh = SheetRef::Page(p);
    let scale = match (num(doc, sh, "PageScale"), num(doc, sh, "DrawingScale")) {
        (Some(ps), Some(ds)) if ps > 0.0 && ds > 0.0 && (ds / ps).is_finite() => ds / ps,
        _ => 1.0,
    };
    let avenue = [num(doc, sh, "AvenueSizeX"), num(doc, sh, "AvenueSizeY")].into_iter().flatten().filter(|v| *v > 0.0 && v.is_finite()).fold(None::<f64>, |m, v| Some(m.map_or(v, |m| m.max(v))));
    let d = Opts::default();
    Opts { clearance: avenue.unwrap_or(7.5 / 25.4) * 0.5 * scale, bend: d.bend * scale }
}

fn run(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], force: bool) -> Result<(), EditError> {
    let mut cache: Vec<(Owner, Vec<(ShapeRef, [f64; 4])>)> = Vec::new();
    // (JD21, SS-19) which way connectors may cross each obstacle of an entry of the cache
    let mut passes: Vec<Vec<u8>> = Vec::new();
    // (JD21 jumps) the connectors laid, in order, with their ways before (jumps.rs)
    let mut laid = crate::jumps::Laid::default();
    for &c in shapes {
        if !is_one_d(tx.doc(), c) {
            continue;
        }
        let sheet = SheetRef::Shape(c);
        let end_cells = [names::BEGIN_X, names::BEGIN_Y, names::END_X, names::END_Y].map(|n| (sheet, CellAddr::single(n)));
        // static ends: the engine computes their PAR(PNT(..)) formulas
        eng.recalc_cells(tx, &end_cells)?;
        if is_connector(tx.doc(), c) {
            let i = match cache.iter().position(|(o, _)| *o == c.owner) {
                Some(i) => i,
                None => {
                    let list = obstacles(tx.doc(), c.owner);
                    passes.push(list.iter().map(|(s, _)| crate::route_styles::pass(tx.doc(), *s)).collect());
                    cache.push((c.owner, list));
                    cache.len() - 1
                }
            };
            let mut boxes: Vec<[f64; 4]> = cache[i].1.iter().filter(|(s, _)| *s != c).map(|(_, b)| *b).collect();
            let mut pass: Vec<u8> = cache[i].1.iter().zip(&passes[i]).filter(|((s, _), _)| *s != c).map(|(_, p)| *p).collect();
            // (JD21v) the shapes its own ends are glued to keep it out also where they are not
            // placeable (a file without ObjType; Visio makes them 1 when it glues)
            for t in glued(tx.doc(), c) {
                if let Target::Shape(s) | Target::Point { shape: s, .. } = t {
                    let doc = tx.doc();
                    if cache[i].1.iter().all(|(o, _)| *o != s) && !is_one_d(doc, s) && may_block(doc, s) {
                        if let Some(b) = page_box(doc, s) {
                            boxes.push(b);
                            pass.push(crate::route_styles::pass(doc, s));
                        }
                    }
                }
            }
            laid.note(tx.doc(), c);
            route_one(tx, eng, c, force, &boxes, &pass)?;
        } else {
            walk_line(tx, eng, c)?;
        }
    }
    // (JD21 jumps) their line jumps and those of the connectors they cross or crossed
    crate::jumps::lay(tx, eng, &laid)
}

// a 1-D shape with walking glue that is no connector: a dynamic end docks on the outline
// towards the other end
fn walk_line(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef) -> Result<(), EditError> {
    let doc = tx.doc();
    let targets = glued(doc, c);
    let Some((bp, ep)) = ends(doc, c) else { return Ok(()) };
    let pinv = parent_xform(doc, c).inverse().unwrap_or(Xform::IDENTITY);
    let mut set: Vec<(End, [f64; 2])> = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        if let Target::Shape(s) = *t {
            let other = if i == 0 { reference(doc, targets[1], ep) } else { reference(doc, targets[0], bp) };
            let m = centre(doc, s);
            let p = dock(&outline(doc, s), m, [other[0] - m[0], other[1] - m[1]], page_box(doc, s));
            set.push((if i == 0 { End::Begin } else { End::End }, pinv.apply(p)));
        }
    }
    if set.is_empty() {
        return Ok(());
    }
    let sheet = SheetRef::Shape(c);
    let mut cells = Vec::new();
    for (e, p) in set {
        let (cx, cy) = end_cells(e);
        tx.set_result(sheet, &CellAddr::single(cx), Value::Num(p[0]), None)?;
        tx.set_result(sheet, &CellAddr::single(cy), Value::Num(p[1]), None)?;
        cells.extend([(sheet, CellAddr::single(cx)), (sheet, CellAddr::single(cy))]);
    }
    eng.recalc_cells(tx, &cells)?;
    Ok(())
}

// the point a dynamic end aims at: the middle of its target, else the end itself
fn reference(doc: &Document, t: Target, at: [f64; 2]) -> [f64; 2] {
    match t {
        Target::Shape(s) => centre(doc, s),
        _ => at,
    }
}

// what a dynamic end lines up with: the other end's point, or the box of its dynamic target
#[derive(Copy, Clone)]
enum Facing {
    Point([f64; 2]),
    Box([f64; 4]),
}

fn facing(doc: &Document, t: Target, at: [f64; 2]) -> Facing {
    match t {
        Target::Shape(s) => page_box(doc, s).map_or(Facing::Point(at), Facing::Box),
        _ => Facing::Point(at),
    }
}

// where along a side of box b (side direction d) an end can face the other end straight: the
// other point's coordinate when it lies within the side, the middle of the overlap with the
// other box -> None: nowhere
fn lined_up(b: &[f64; 4], d: [f64; 2], other: Facing) -> Option<f64> {
    let k = if d[0] != 0.0 { 1 } else { 0 };
    let (lo, hi) = (b[k], b[k + 2]);
    match other {
        Facing::Point(p) => (p[k] > lo && p[k] < hi).then_some(p[k]),
        Facing::Box(o) => {
            let (l, h) = (lo.max(o[k]), hi.min(o[k + 2]));
            (l < h).then_some((l + h) * 0.5)
        }
    }
}

// the ports an end can have: one for a static or free end; for a dynamic end per side the
// middle and, where the other end lies within the side's reach, the point straight opposite it
// (as Visio docks when a straight connection is possible); the straight style: one, where the
// line to the other end leaves the outline
fn candidates(doc: &Document, t: Target, at: [f64; 2], toward: [f64; 2], other: Facing, style: Style) -> Vec<Port> {
    match t {
        Target::Point { shape, row } => {
            let dir = connection_points(doc, shape).iter().find(|p| p.row == row).and_then(|p| p.dir).map(|d| [-d[0], -d[1]]);
            alloc::vec![Port { at, dir, own: page_box(doc, shape) }]
        }
        Target::Shape(s) => {
            let own = page_box(doc, s);
            let segs = outline(doc, s);
            let m = centre(doc, s);
            if style == Style::Straight {
                let p = dock(&segs, m, [toward[0] - m[0], toward[1] - m[1]], own);
                return alloc::vec![Port { at: p, dir: None, own }];
            }
            let mut out = Vec::with_capacity(8);
            for d in SIDES {
                out.push(Port { at: dock(&segs, m, d, own), dir: Some(d), own });
                let k = if d[0] != 0.0 { 1 } else { 0 };
                if let Some(l) = own.and_then(|b| lined_up(&b, d, other)).filter(|l| (l - m[k]).abs() > 1e-9 * (1.0 + l.abs())) {
                    let mut o = m;
                    o[k] = l;
                    out.push(Port { at: dock(&segs, o, d, own), dir: Some(d), own });
                }
            }
            out
        }
        Target::None => alloc::vec![Port::free(at)],
    }
}

fn route_one(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, force: bool, boxes: &[[f64; 4]], pass: &[u8]) -> Result<(), EditError> {
    let doc = tx.doc();
    let sheet = SheetRef::Shape(c);
    let targets = glued(doc, c);
    let Some((bp, ep)) = ends(doc, c) else { return Ok(()) };
    let style = style(doc, c);
    let opts = opts_for(doc, c.owner);
    // (JD21, CN-04) ConFixedCode 1 and 2 keep the way (route_edit::kept; 3 .. 6 are what Visio
    // writes for the connectors it routes itself: free)
    let code = num(doc, sheet, "ConFixedCode").map_or(0, |v| v as i64);
    let fixed = !force && matches!(code, 1 | 2);
    let refs = [reference(doc, targets[0], bp), reference(doc, targets[1], ep)];
    let ca = candidates(doc, targets[0], bp, refs[1], facing(doc, targets[1], ep), style);
    let cb = candidates(doc, targets[1], ep, refs[0], facing(doc, targets[0], bp), style);
    // (JD21v, CN-05) the way of the connector's style as Visio lays it (route_styles::route; the
    // shapes of ends glued to whole shapes).  (JD15, SS-18) WalkPreference changes none of these
    // ways in Visio (the counter-check): it is kept, not read
    let shapes = [crate::route_styles::shape(doc, targets[0]), crate::route_styles::shape(doc, targets[1])];
    let lines = related_lines(doc, c, &targets);
    let lay = || crate::route_styles::route(crate::route_styles::of(doc, c), [shapes[0].as_ref(), shapes[1].as_ref()], &ca, &cb, boxes, pass, &lines, &opts);
    let (pts, router) = if style == Style::Straight {
        // (JD21, CN-04) a straight way with vertices of its own keeps them
        match fixed.then(|| crate::route_edit::kept_polyline(doc, c, &targets)).flatten() {
            Some(p) => (p, None),
            None => {
                let r = lay();
                (r.way, Some(r.router))
            }
        }
    } else {
        match fixed.then(|| crate::route_edit::kept(doc, c, code, &ca, &cb, boxes, pass, &opts)).flatten() {
            Some(p) => (p, None),
            None => {
                let r = lay();
                (r.way, Some(r.router))
            }
        }
    };
    write_route(tx, eng, c, &targets, &pts, style)?;
    // (JD21v) ConFixedCode 6 / 0 afterwards, as Visio writes it
    match router {
        Some(r) => crate::route_styles::mark(tx, c, r),
        None => Ok(()),
    }
}

// the Width or Height formula of a connector for a side of length d (None: leave it)
fn side_formula(current: &str, d: f64, standard: &'static str) -> Option<String> {
    let known = current == standard || current.starts_with("GUARD(0.19685039370079DL") || current.starts_with("GUARD(-0.19685039370079DL");
    if !known {
        return None;
    }
    let want = if d.abs() >= MIN_SIDE {
        String::from(standard)
    } else if d < 0.0 {
        format!("GUARD(-{MIN_SIDE}DL)")
    } else {
        format!("GUARD({MIN_SIDE}DL)")
    };
    (want != current).then_some(want)
}

// a value of a Geometry cell: a constant or a formula (NURBS(..), POLYLINE(..))
pub(crate) enum Cv {
    N(f64),
    F(String),
}

pub(crate) struct GeoRow {
    pub(crate) kind: Sym,
    pub(crate) kind_name: &'static str,
    pub(crate) cells: Vec<(Sym, Cv)>,
}

pub(crate) fn write_route(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, targets: &[Target; 2], pts: &[[f64; 2]], style: Style) -> Result<(), EditError> {
    let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else { return Ok(()) };
    let sheet = SheetRef::Shape(c);
    let pinv = parent_xform(tx.doc(), c).inverse().unwrap_or(Xform::IDENTITY);
    let (b, e) = (pinv.apply(first), pinv.apply(last));
    for (i, t) in targets.iter().enumerate() {
        if let Target::Shape(_) = t {
            let (p, (cx, cy)) = if i == 0 { (b, end_cells(End::Begin)) } else { (e, end_cells(End::End)) };
            tx.set_result(sheet, &CellAddr::single(cx), Value::Num(p[0]), None)?;
            tx.set_result(sheet, &CellAddr::single(cy), Value::Num(p[1]), None)?;
        }
    }
    let mut cells: Vec<(SheetRef, CellAddr)> =
        [names::BEGIN_X, names::BEGIN_Y, names::END_X, names::END_Y].iter().map(|n| (sheet, CellAddr::single(*n))).collect();
    for (cell, d, std) in [(names::WIDTH, e[0] - b[0], "GUARD(EndX-BeginX)"), (names::HEIGHT, e[1] - b[1], "GUARD(EndY-BeginY)")] {
        let a = CellAddr::single(cell);
        let cur = match tx.doc().resolve(sheet, &a).formula() {
            Formula::Expr(f) => String::from(tx.doc().str(f)),
            _ => String::new(),
        };
        if let Some(f) = side_formula(&cur, d, std) {
            tx.force_formula(sheet, &a, &f)?;
        }
        cells.push((sheet, a));
    }
    eng.recalc_cells(tx, &cells)?;
    // the way in local coordinates
    let doc = tx.doc();
    let li = doc.page_xform(c).inverse().unwrap_or(Xform::IDENTITY);
    let local: Vec<[f64; 2]> = pts.iter().map(|p| li.apply(*p)).collect();
    let (w, h) = (num_sym(doc, sheet, names::WIDTH).unwrap_or(0.0), num_sym(doc, sheet, names::HEIGHT).unwrap_or(0.0));
    let (rows, drawn) = if style == Style::Curved && local.len() >= 3 { curve_rows(&local, w, h) } else { (line_rows(&local), local.clone()) };
    write_geometry(tx, eng, c, &rows)?;
    // the text in the middle of the way
    let tp = tx.intern("TextPosition");
    let (csk, rk) = (SecKey::one(names::CONTROL), RowKey::Named(tp));
    if tx.doc().has_row(sheet, csk, rk) {
        let m = middle(&drawn);
        let (ax, ay) = (CellAddr::row(csk, rk, names::X), CellAddr::row(csk, rk, names::Y));
        tx.force_constant(sheet, &ax, Value::Num(m[0]), None)?;
        tx.force_constant(sheet, &ay, Value::Num(m[1]), None)?;
        eng.recalc_cells(tx, &[(sheet, ax), (sheet, ay)])?;
    }
    Ok(())
}

// the point halfway along a polyline
fn middle(pts: &[[f64; 2]]) -> [f64; 2] {
    let half = route::length(pts) * 0.5;
    let mut run = 0.0;
    for w in pts.windows(2) {
        let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
        let l = sqrt(dx * dx + dy * dy);
        if l > 0.0 && run + l >= half {
            let t = (half - run) / l;
            return [w[0][0] + dx * t, w[0][1] + dy * t];
        }
        run += l;
    }
    pts.first().copied().unwrap_or([0.0, 0.0])
}

fn xy(x: f64, y: f64) -> Vec<(Sym, Cv)> {
    alloc::vec![(names::X, Cv::N(x)), (names::Y, Cv::N(y))]
}

fn line_rows(local: &[[f64; 2]]) -> Vec<GeoRow> {
    local
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (kind, kind_name) = if i == 0 { (names::MOVE_TO, "MoveTo") } else { (names::LINE_TO, "LineTo") };
            GeoRow { kind, kind_name, cells: xy(p[0], p[1]) }
        })
        .collect()
}

// MoveTo and one NURBSTo with the corners as control points (relative to Width and Height
// where they are not zero, as Visio writes them) -> the rows and the curve sampled
fn curve_rows(local: &[[f64; 2]], w: f64, h: f64) -> (Vec<GeoRow>, Vec<[f64; 2]>) {
    let n = local.len();
    let (degree, knots) = route::spline_knots(n);
    let (rx, ry) = (w.abs() > 1e-9, h.abs() > 1e-9);
    let mut e = format!("NURBS({},{},{},{}", knots[n], degree, if rx { 0 } else { 1 }, if ry { 0 } else { 1 });
    for i in 1..n - 1 {
        let x = if rx { local[i][0] / w } else { local[i][0] };
        let y = if ry { local[i][1] / h } else { local[i][1] };
        e.push_str(&format!(",{x},{y},{},1", knots[i]));
    }
    e.push(')');
    let end = local[n - 1];
    let rows = alloc::vec![
        GeoRow { kind: names::MOVE_TO, kind_name: "MoveTo", cells: xy(local[0][0], local[0][1]) },
        GeoRow {
            kind: names::NURBS_TO,
            kind_name: "NURBSTo",
            cells: alloc::vec![
                (names::X, Cv::N(end[0])),
                (names::Y, Cv::N(end[1])),
                (names::A, Cv::N(knots[n - 1])),
                (names::B, Cv::N(1.0)),
                (names::C, Cv::N(knots[0])),
                (names::D, Cv::N(1.0)),
                (names::E, Cv::F(e)),
            ],
        },
    ];
    let curve = Nurbs { degree, points: local.to_vec(), weights: alloc::vec![1.0; n], knots };
    let drawn = if curve.valid() { (0..=64).map(|i| curve.at(i as f64 / 64.0)).collect() } else { local.to_vec() };
    (rows, drawn)
}

// make row `key` of the section have type `kind`: a row of its own is replaced, an inherited
// one gets the parent's row back -> false when the parent's row has another type
fn place_row(tx: &mut Transaction, sheet: SheetRef, sk: SecKey, key: u32, kind: Sym, kind_name: &str) -> Result<bool, EditError> {
    let rk = RowKey::Index(key);
    let doc = tx.doc();
    let eff = doc.rows(sheet, sk).into_iter().find(|r| r.key == rk);
    let local_del = doc.sheet(sheet).and_then(|s| s.section(sk)).and_then(|s| s.row(rk)).map(|r| r.del);
    let parent_kind = doc.parent_sheet(sheet, None).and_then(|p| doc.rows(p, sk).into_iter().find(|r| r.key == rk)).map(|r| r.kind);
    match eff {
        Some(r) if r.kind == Some(kind) => Ok(true),
        Some(_) => match parent_kind {
            None => {
                tx.delete_row(sheet, sk, rk)?;
                tx.add_row(sheet, sk, Some(rk), Some(kind_name))?;
                Ok(true)
            }
            Some(pk) if pk == Some(kind) => {
                tx.delete_row(sheet, sk, rk)?;
                tx.restore_row(sheet, sk, rk)?;
                Ok(true)
            }
            Some(_) => Ok(false),
        },
        None => match (local_del, parent_kind) {
            (Some(true), Some(pk)) if pk == Some(kind) => {
                tx.restore_row(sheet, sk, rk)?;
                Ok(true)
            }
            (Some(true), _) => Ok(false),
            _ => {
                tx.add_row(sheet, sk, Some(rk), Some(kind_name))?;
                Ok(true)
            }
        },
    }
}

// write the rows into the connector's first Geometry section (made when it has none); (JD21
// jumps) jumps.rs writes its line jumps through it
pub(crate) fn write_geometry(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, rows: &[GeoRow]) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(c);
    let sk = match tx.doc().geometry(sheet).first() {
        Some(k) => *k,
        None => {
            let k = tx.add_section(sheet, "Geometry")?;
            tx.set_constant(sheet, &CellAddr::section(k, names::NO_FILL), Value::Num(1.0), None)?;
            k
        }
    };
    let mut keys: Vec<u32> = Vec::with_capacity(rows.len());
    let mut fits = true;
    for (i, r) in rows.iter().enumerate() {
        if !place_row(tx, sheet, sk, i as u32 + 1, r.kind, r.kind_name)? {
            fits = false;
            break;
        }
        keys.push(i as u32 + 1);
    }
    let index = |k: RowKey| match k {
        RowKey::Index(i) => Some(i),
        RowKey::Named(_) => None,
    };
    if fits {
        // the rows past the new ones go
        let extra: Vec<RowKey> = tx.doc().rows(sheet, sk).into_iter().map(|r| r.key).filter(|k| index(*k).is_some_and(|i| i as usize > rows.len())).collect();
        for k in extra {
            tx.delete_row(sheet, sk, k)?;
        }
    } else {
        // a master's rows of other types: all go (Del marks), new rows follow them
        let doc = tx.doc();
        let own = doc.sheet(sheet).and_then(|s| s.section(sk)).map(|s| s.rows.iter().map(|r| r.key).collect::<Vec<_>>()).unwrap_or_default();
        let all: Vec<RowKey> = doc.rows(sheet, sk).into_iter().map(|r| r.key).collect();
        let max = all.iter().chain(own.iter()).filter_map(|k| index(*k)).max().unwrap_or(0);
        for k in all {
            tx.delete_row(sheet, sk, k)?;
        }
        keys.clear();
        for (i, r) in rows.iter().enumerate() {
            let key = max + 1 + i as u32;
            tx.add_row(sheet, sk, Some(RowKey::Index(key)), Some(r.kind_name))?;
            keys.push(key);
        }
    }
    let mut formulas = Vec::new();
    for (r, key) in rows.iter().zip(&keys) {
        for (cell, v) in &r.cells {
            let a = CellAddr::row(sk, RowKey::Index(*key), *cell);
            match v {
                Cv::N(x) => tx.force_constant(sheet, &a, Value::Num(*x), None)?,
                Cv::F(f) => {
                    // the value as text with its unit (U="NURBS", "POLYLINE"), then the formula
                    let unit = if f.starts_with("POLYLINE") { crate::units::Unit::Polyline } else { crate::units::Unit::Nurbs };
                    let v = Value::Text(tx.intern(f));
                    tx.force_constant(sheet, &a, v, Some(unit))?;
                    tx.force_formula(sheet, &a, f)?;
                    formulas.push((sheet, a));
                }
            }
        }
    }
    if !formulas.is_empty() {
        eng.recalc_cells(tx, &formulas)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "glue_tests.rs"]
mod tests;
