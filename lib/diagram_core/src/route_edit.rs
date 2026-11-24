// route_edit -- (JD21, CN-04) the way of a dynamic connector by hand: the handles of a selected
// connector (a midpoint on every piece, a vertex at every bend), a piece moved across (right
// angles stay right angles; next to an end a jog keeps the end where it is), a bend moved, added
// and removed; the way kept afterwards when its shapes move
//
//   way(doc, c) -> Option<Way>        the corners of the drawn way on the page and whether it is
//                                     right-angled (None: no dynamic connector, curved, LockVtxEdit)
//   handles(doc, c) -> Vec<Handle>    the midpoints and vertices to show (page coordinates)
//   move_piece(&pts, i, to, right, stub, snap) / move_bend(&pts, j, ..) / add_bend(&pts, at)
//   / remove_bend(&pts, j, right, stub) -> new points        pure, the ends stay
//   edit(tx, eng, c, Edit, snap) -> changed   one of them on the connector: the way written,
//                                     ConFixedCode 1 (2 stays)
//   set_way(tx, eng, c, &pts)         a way written as given (ends must be the connector's)
//   reset(tx, eng, &connectors)       Verbinder zurücksetzen: ConFixedCode 1 -> 0, routed anew
//   kept(..) / kept_polyline(..)      (crate, glue::route_one) a kept way after its shapes moved
//
// Visio's rules -- belegt (Visio 2000 user's guide, chapter 6, "Working with routable connectors";
// support.microsoft.com "Edit connector lines, arrows, or points"; Microsoft Learn ConFixedCode):
// routable connectors show midpoints and vertices besides their endpoints; their paths are edited
// "by moving, adding, or deleting endpoints, midpoints, and vertices"; "to reroute a connector,
// select it, and then drag a midpoint to a new location"; vertices are added with Ctrl + click and
// deleted with Del after a click on them (the pencil tool, as for other shapes); ConFixedCode 1 is
// "reroute as needed (manual reroute)", 2 "never reroute", 3 "reroute on crossover", 4 .. 6
// "internal".  The local corpus: the connectors on the pages have 0 (from the style), 3, 4, 5 or 6
// (Visio writes them when it routes), never 1 or 2: they reroute freely here as before.
//
// Angenommen (the user's check in Visio may correct it):
// * A right-angle way: a midpoint moves its piece across only; where the piece meets the end, a
//   short stub (the routing clearance, at most a third of the piece) stays at the end and a jog
//   carries the moved piece (two new bends); a piece next to an end's stub cannot be dragged past
//   it.  A piece that comes within `snap` of the line of a parallel piece two further on, or of an
//   end, snaps to it, and the bends between them go (that is how bends are removed by dragging).
//   A vertex moves the two pieces at it.  Ctrl + click adds a vertex on the piece (a right-angle
//   way: the piece is split; drag the vertex or a half to make a step).  Del removes a vertex: on a
//   right-angle way with its neighbour, the jog between them flattened by moving the piece after
//   (or before) them onto the line of the piece before (after) -- not where that piece ends at an
//   end of the connector.
// * A straight way (straight, network, centre to centre): dragging a midpoint makes a new vertex
//   there; vertices move freely and are removed with Del.  Curved connectors have no handles.
// * A way edited by hand gets ConFixedCode 1 (Visio's "manual reroute"; 2 stays).  Kept ways (1,
//   2) keep their corners when their shapes move: the first and last pieces follow the ends, a
//   dynamic end stays on the side the way leaves by.  1 is laid anew when the kept way now runs
//   through a shape or an end piece would run back into its shape; 2 never.  Verbinder
//   zurücksetzen gives ConFixedCode 1 back as 0 and routes anew.

use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, Style, Target};
use crate::names;
use crate::route::{self, Opts, Port};
use crate::sheet::CellAddr;

/// the drawn way of a connector by hand
#[derive(Clone, PartialEq, Debug)]
pub struct Way {
    /// its corners on the page, Begin first
    pub pts: Vec<[f64; 2]>,
    /// right angles (a right-angle style and every piece upright or level); else a polyline
    pub right: bool,
}

/// a handle of a connector's way
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Handle {
    /// where it is (page)
    pub at: [f64; 2],
    /// a vertex (true) or a piece's midpoint (false)
    pub bend: bool,
    /// the vertex's index into Way::pts, or the piece's (the piece from pts[index])
    pub index: usize,
    /// a midpoint: half the piece, from the handle to its end (page); a vertex: 0, 0
    pub half: [f64; 2],
    /// a midpoint that moves anywhere (a straight way: a new vertex), not only across
    pub free: bool,
}

/// one change by hand
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Edit {
    /// the piece from pts[i] moves (across, or a new vertex on a straight way) to `to`
    Piece(usize, [f64; 2]),
    /// vertex pts[j] moves to `to`
    Bend(usize, [f64; 2]),
    /// a vertex on the piece nearest to the point
    Add([f64; 2]),
    /// vertex pts[j] goes
    Remove(usize),
}

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

fn tol(a: [f64; 2], b: [f64; 2]) -> f64 {
    1e-9 * (1.0 + a[0].abs().max(a[1].abs()) + b[0].abs().max(b[1].abs()))
}

fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    let t = tol(a, b);
    (a[0] - b[0]).abs() <= t && (a[1] - b[1]).abs() <= t
}

/// the axis a piece runs along: 0 level, 1 upright (None: slanted or no length)
fn along(p: [f64; 2], q: [f64; 2]) -> Option<usize> {
    let t = tol(p, q);
    let (dx, dy) = ((q[0] - p[0]).abs(), (q[1] - p[1]).abs());
    if dx <= t && dy <= t {
        None
    } else if dy <= t {
        Some(0)
    } else if dx <= t {
        Some(1)
    } else {
        None
    }
}

/// whether every piece is level or upright (pieces without length count as either)
pub fn orthogonal(pts: &[[f64; 2]]) -> bool {
    pts.windows(2).all(|w| same(w[0], w[1]) || along(w[0], w[1]).is_some())
}

// the corners of the drawn way: the first Geometry section of MoveTo and LineTo rows only
fn corners(doc: &Document, c: ShapeRef) -> Option<Vec<[f64; 2]>> {
    let sheet = SheetRef::Shape(c);
    let sk = *doc.geometry(sheet).first()?;
    let rows = doc.rows(sheet, sk);
    if rows.is_empty() || rows.iter().any(|r| r.kind != Some(names::MOVE_TO) && r.kind != Some(names::LINE_TO)) {
        return None;
    }
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(rows.len());
    for p in glue::path(doc, c) {
        if pts.last().is_none_or(|&l| !same(l, p)) {
            pts.push(p);
        }
    }
    (pts.len() >= 2).then_some(pts)
}

/// the way of a connector that can be edited by hand: a dynamic connector, not curved, not
/// locked against vertex edits (LockVtxEdit)
pub fn way(doc: &Document, c: ShapeRef) -> Option<Way> {
    if doc.shape(c).is_none() || !glue::is_connector(doc, c) {
        return None;
    }
    let style = glue::style(doc, c);
    if style == Style::Curved || num(doc, SheetRef::Shape(c), "LockVtxEdit").is_some_and(|v| v != 0.0) {
        return None;
    }
    let pts = corners(doc, c)?;
    let right = style == Style::RightAngle && orthogonal(&pts);
    Some(Way { pts, right })
}

/// the handles a selected connector shows: a vertex at every bend, a midpoint on every piece
pub fn handles(doc: &Document, c: ShapeRef) -> Vec<Handle> {
    let Some(w) = way(doc, c) else { return Vec::new() };
    let n = w.pts.len();
    let mut out = Vec::with_capacity(2 * n);
    for (i, s) in w.pts.windows(2).enumerate() {
        let half = [(s[1][0] - s[0][0]) * 0.5, (s[1][1] - s[0][1]) * 0.5];
        out.push(Handle { at: [s[0][0] + half[0], s[0][1] + half[1]], bend: false, index: i, half, free: !w.right });
    }
    for j in 1..n.saturating_sub(1) {
        out.push(Handle { at: w.pts[j], bend: true, index: j, half: [0.0, 0.0], free: false });
    }
    out
}

// ---------------------------------------------------------------- the edits (pure)

// piece i moved across to the coordinate v (the axis across it), not simplified: a joint whose
// other piece runs across follows; at an end a stub stays and a jog carries the piece, at a
// piece on the same line a jog too.  Next to an end's piece the piece stops short of the end
fn shift(pts: &[[f64; 2]], i: usize, v: f64, stub: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    if i + 1 >= n {
        return pts.to_vec();
    }
    let (a, b) = (pts[i], pts[i + 1]);
    let Some(ax) = along(a, b) else { return pts.to_vec() };
    let k = 1 - ax;
    let len = (b[ax] - a[ax]).abs();
    let s = stub.min(len / 3.0).max(0.0);
    let sgn = if b[ax] >= a[ax] { 1.0 } else { -1.0 };
    let a_moves = i > 0 && along(pts[i - 1], a) == Some(k);
    let b_moves = i + 2 < n && along(b, pts[i + 2]) == Some(k);
    // an end's piece keeps its direction and a stub's length
    let mut v = v;
    for (moves, joint, end) in [(a_moves && i == 1, a, pts[0]), (b_moves && i + 2 == n - 1, b, pts[n - 1])] {
        if !moves {
            continue;
        }
        let orig = joint[k] - end[k];
        let keep = stub.min(orig.abs());
        if orig > 0.0 {
            v = v.max(end[k] + keep);
        } else if orig < 0.0 {
            v = v.min(end[k] - keep);
        }
    }
    let moved = |mut p: [f64; 2]| {
        p[k] = v;
        p
    };
    let mut out = Vec::with_capacity(n + 4);
    out.extend_from_slice(&pts[..i]);
    if a_moves {
        out.push(moved(a));
    } else if i == 0 {
        let mut st = a;
        st[ax] += sgn * s;
        out.extend([a, st, moved(st)]);
    } else {
        out.extend([a, moved(a)]);
    }
    if b_moves {
        out.push(moved(b));
    } else if i + 2 == n {
        let mut st = b;
        st[ax] -= sgn * s;
        out.extend([moved(st), st, b]);
    } else {
        out.extend([moved(b), b]);
    }
    out.extend_from_slice(&pts[i + 2..]);
    out
}

// v snapped to the line of a parallel piece two further on either side or to an end's (within snap)
fn snapped(pts: &[[f64; 2]], i: usize, v: f64, snap: f64) -> f64 {
    let n = pts.len();
    let Some(ax) = along(pts[i], pts[i + 1]) else { return v };
    let k = 1 - ax;
    let mut lines = alloc::vec![pts[0][k], pts[n - 1][k]];
    if i >= 2 && along(pts[i - 2], pts[i - 1]) == Some(ax) {
        lines.push(pts[i - 1][k]);
    }
    if i + 3 < n && along(pts[i + 2], pts[i + 3]) == Some(ax) {
        lines.push(pts[i + 2][k]);
    }
    let best = lines.into_iter().min_by(|a, b| (a - v).abs().total_cmp(&(b - v).abs()));
    match best {
        Some(l) if (l - v).abs() <= snap.max(0.0) => l,
        _ => v,
    }
}

/// the way with piece i (from pts[i]) dragged to `to`: a right-angle way moves the piece across
/// (snapped within `snap`, jogs at the ends with stubs of `stub`), a straight one gets a vertex
/// at `to`
pub fn move_piece(pts: &[[f64; 2]], i: usize, to: [f64; 2], right: bool, stub: f64, snap: f64) -> Vec<[f64; 2]> {
    if i + 1 >= pts.len() {
        return pts.to_vec();
    }
    if !right {
        let mut out = pts.to_vec();
        out.insert(i + 1, to);
        return out;
    }
    let Some(ax) = along(pts[i], pts[i + 1]) else { return pts.to_vec() };
    let v = snapped(pts, i, to[1 - ax], snap);
    route::simplify(shift(pts, i, v, stub))
}

/// the way with vertex pts[j] dragged to `to`: on a right-angle way the two pieces at it move
/// across (at a vertex that only splits a piece: the piece after it), else the vertex moves
pub fn move_bend(pts: &[[f64; 2]], j: usize, to: [f64; 2], right: bool, stub: f64, snap: f64) -> Vec<[f64; 2]> {
    let n = pts.len();
    if j == 0 || j + 1 >= n {
        return pts.to_vec();
    }
    if !right {
        let mut out = pts.to_vec();
        out[j] = to;
        return out;
    }
    match (along(pts[j - 1], pts[j]), along(pts[j], pts[j + 1])) {
        (Some(x), Some(y)) if x != y => {
            // the piece after the vertex first (nothing before it changes), then the one before
            let q = shift(pts, j, snapped(pts, j, to[x], snap), stub);
            let q2 = shift(&q, j - 1, snapped(&q, j - 1, to[y], snap), stub);
            route::simplify(q2)
        }
        (_, Some(y)) => route::simplify(shift(pts, j, snapped(pts, j, to[1 - y], snap), stub)),
        _ => pts.to_vec(),
    }
}

// the nearest point on segment p-q to x and its distance
fn nearest(p: [f64; 2], q: [f64; 2], x: [f64; 2]) -> ([f64; 2], f64) {
    let d = [q[0] - p[0], q[1] - p[1]];
    let l2 = d[0] * d[0] + d[1] * d[1];
    let t = if l2 > 0.0 { (((x[0] - p[0]) * d[0] + (x[1] - p[1]) * d[1]) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let n = [p[0] + d[0] * t, p[1] + d[1] * t];
    let (dx, dy) = (x[0] - n[0], x[1] - n[1]);
    (n, crate::geometry::sqrt(dx * dx + dy * dy))
}

/// the way with a new vertex on the piece nearest to `at` (where `at` falls onto it; a right-angle
/// way: the piece is split there) -> None: on a vertex or an end already
pub fn add_bend(pts: &[[f64; 2]], at: [f64; 2]) -> Option<Vec<[f64; 2]>> {
    let (i, (p, _)) = pts.windows(2).map(|w| nearest(w[0], w[1], at)).enumerate().min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))?;
    let len = route::length(&pts[i..i + 2]);
    let eps = 1e-6 * (1.0 + len);
    let from = |q: [f64; 2]| ((q[0] - p[0]).abs() + (q[1] - p[1]).abs()) > eps;
    if !from(pts[i]) || !from(pts[i + 1]) {
        return None;
    }
    let mut out = pts.to_vec();
    out.insert(i + 1, p);
    Some(out)
}

/// the way without vertex pts[j] (see the file header) -> None: it cannot go
pub fn remove_bend(pts: &[[f64; 2]], j: usize, right: bool, stub: f64) -> Option<Vec<[f64; 2]>> {
    let n = pts.len();
    if j == 0 || j + 1 >= n {
        return None;
    }
    let (x, y) = (along(pts[j - 1], pts[j]), along(pts[j], pts[j + 1]));
    if !right || x.is_none() || y.is_none() || x == y {
        let mut out = pts.to_vec();
        out.remove(j);
        return Some(if right { route::simplify(out) } else { out });
    }
    let before = route::bends(pts);
    let fewer = |q: Vec<[f64; 2]>| {
        let q = route::simplify(q);
        (route::bends(&q) + 2 <= before && q.len() >= 2).then_some(q)
    };
    // the jog j, j + 1: the piece after it onto the line of the piece before it
    if j + 2 < n - 1 {
        if let Some(ax) = along(pts[j + 1], pts[j + 2]) {
            if let Some(q) = fewer(shift(pts, j + 1, pts[j][1 - ax], stub)) {
                return Some(q);
            }
        }
    }
    // the jog j - 1, j: the piece before it onto the line of the piece after it
    if j >= 2 {
        if let Some(ax) = along(pts[j - 2], pts[j - 1]) {
            if let Some(q) = fewer(shift(pts, j - 2, pts[j][1 - ax], stub)) {
                return Some(q);
            }
        }
    }
    None
}

// ---------------------------------------------------------------- on the connector

/// write a way into a connector as given (Begin first; the ends must be where the connector's
/// are) and mark it as the user's (ConFixedCode 1, unless it is 1 or 2 already)
pub fn set_way(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, pts: &[[f64; 2]]) -> Result<(), EditError> {
    let doc = tx.doc();
    if doc.shape(c).is_none() {
        return Err(EditError::NoShape);
    }
    if !glue::is_connector(doc, c) || pts.len() < 2 {
        return Err(EditError::Invalid);
    }
    let style = glue::style(doc, c);
    if style == Style::Curved {
        return Err(EditError::Invalid);
    }
    let targets = glue::glued(doc, c);
    let sheet = SheetRef::Shape(c);
    let code = num(doc, sheet, "ConFixedCode").unwrap_or(0.0);
    glue::write_route(tx, eng, c, &targets, pts, style)?;
    if code != 1.0 && code != 2.0 {
        let a = tx.single("ConFixedCode");
        tx.set_constant(sheet, &a, Value::Num(1.0), None)?;
    }
    Ok(())
}

/// apply one change by hand to a connector's way (`snap`: how near a piece snaps to a line, in
/// drawing units) -> whether the way changed
pub fn edit(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, e: Edit, snap: f64) -> Result<bool, EditError> {
    let doc = tx.doc();
    let Some(w) = way(doc, c) else { return Err(EditError::Invalid) };
    let stub = glue::opts_for(doc, c.owner).clearance;
    let n = w.pts.len();
    let new = match e {
        Edit::Piece(i, to) if i + 1 < n => Some(move_piece(&w.pts, i, to, w.right, stub, snap)),
        Edit::Bend(j, to) if j > 0 && j + 1 < n => Some(move_bend(&w.pts, j, to, w.right, stub, snap)),
        Edit::Add(at) => add_bend(&w.pts, at),
        Edit::Remove(j) => remove_bend(&w.pts, j, w.right, stub),
        _ => None,
    };
    match new {
        Some(p) if p != w.pts && p.len() >= 2 => {
            set_way(tx, eng, c, &p)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Verbinder zurücksetzen: a way kept because it was edited by hand (ConFixedCode 1) is free
/// again (0); the connectors are routed anew with few bends
pub fn reset(tx: &mut Transaction, eng: &mut Engine, connectors: &[ShapeRef]) -> Result<(), EditError> {
    for &c in connectors {
        let sheet = SheetRef::Shape(c);
        if glue::is_connector(tx.doc(), c) && num(tx.doc(), sheet, "ConFixedCode") == Some(1.0) {
            let a = tx.single("ConFixedCode");
            tx.set_constant(sheet, &a, Value::Num(0.0), None)?;
        }
    }
    glue::reset(tx, eng, connectors)
}

// ---------------------------------------------------------------- kept ways (glue::route_one)

// the axis direction from p to q
fn heading(p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    if dx.abs() >= dy.abs() {
        [dx.signum(), 0.0]
    } else {
        [0.0, dy.signum()]
    }
}

// the port an end keeps: its only one, else the one on the side the way leaves by that lies
// nearest to where the end is now
fn pick(ports: &[Port], at: [f64; 2], h: [f64; 2]) -> Option<usize> {
    if ports.len() == 1 {
        return Some(0);
    }
    let d2 = |p: &Port| {
        let (dx, dy) = (p.at[0] - at[0], p.at[1] - at[1]);
        dx * dx + dy * dy
    };
    (0..ports.len()).filter(|&k| ports[k].dir.is_some_and(|d| d[0] * h[0] + d[1] * h[1] > 0.9)).min_by(|&a, &b| d2(&ports[a]).total_cmp(&d2(&ports[b])))
}

/// a kept right-angle way after its shapes moved (ConFixedCode `code` 1 or 2): the corners stay,
/// the first and last pieces follow the ends -> None: lay it anew (see the file header)
#[allow(clippy::too_many_arguments)]
pub(crate) fn kept(doc: &Document, c: ShapeRef, code: i64, ca: &[Port], cb: &[Port], boxes: &[[f64; 4]], pass: &[u8], opts: &Opts) -> Option<Vec<[f64; 2]>> {
    let cur = corners(doc, c)?;
    let n = cur.len();
    if n < 3 || !orthogonal(&cur) {
        return None;
    }
    let i = pick(ca, cur[0], heading(cur[0], cur[1]))?;
    let j = pick(cb, cur[n - 1], heading(cur[n - 1], cur[n - 2]))?;
    let (b, e) = (ca[i].at, cb[j].at);
    let level = |p: [f64; 2], q: [f64; 2]| (p[1] - q[1]).abs() <= (p[0] - q[0]).abs();
    let q = if n == 3 {
        let corner = if level(cur[0], cur[1]) { [e[0], b[1]] } else { [b[0], e[1]] };
        alloc::vec![b, corner, e]
    } else {
        let mut q = cur.clone();
        let (h0, h1) = (level(cur[0], cur[1]), level(cur[n - 1], cur[n - 2]));
        q[0] = b;
        if h0 { q[1][1] = b[1] } else { q[1][0] = b[0] }
        q[n - 1] = e;
        if h1 { q[n - 2][1] = e[1] } else { q[n - 2][0] = e[0] }
        q
    };
    let q = route::simplify(q);
    if code == 2 || q.len() < 2 {
        return Some(q);
    }
    // laid anew when the kept way runs through a shape (also the glued ones, past the end pieces)
    let own: Vec<[f64; 4]> = [ca[i].own, cb[j].own].into_iter().flatten().collect();
    let m = q.len();
    let through = q.windows(2).enumerate().any(|(k, s)| {
        boxes.iter().enumerate().any(|(o, bx)| route::hits_pass(s[0], s[1], bx, pass.get(o).copied().unwrap_or(0)))
            || (k > 0 && k + 2 < m && own.iter().any(|bx| route::hits_box(s[0], s[1], bx)))
    });
    if through {
        return None;
    }
    // an end piece that runs back into its shape: laid anew too
    let out = |p: &Port, from: [f64; 2], to: [f64; 2]| p.dir.is_none_or(|d| (to[0] - from[0]) * d[0] + (to[1] - from[1]) * d[1] > opts.clearance * 0.5);
    if !(out(&ca[i], q[0], q[1]) && out(&cb[j], q[m - 1], q[m - 2])) {
        return None;
    }
    Some(q)
}

/// a kept straight way with vertices of its own after its shapes moved: the vertices stay, a
/// dynamic end docks towards its neighbouring vertex, the others are where the connector's ends
/// are -> None: a plain straight line (fewer than three points)
pub(crate) fn kept_polyline(doc: &Document, c: ShapeRef, targets: &[Target; 2]) -> Option<Vec<[f64; 2]>> {
    let mut q = corners(doc, c)?;
    let n = q.len();
    if n < 3 {
        return None;
    }
    let (bp, ep) = glue::ends(doc, c)?;
    for (k, (t, at, next)) in [(targets[0], bp, q[1]), (targets[1], ep, q[n - 2])].into_iter().enumerate() {
        let p = match t {
            Target::Shape(s) => {
                let m = glue::centre(doc, s);
                glue::dock(&glue::outline(doc, s), m, [next[0] - m[0], next[1] - m[1]], glue::page_box(doc, s))
            }
            _ => at,
        };
        q[if k == 0 { 0 } else { n - 1 }] = p;
    }
    Some(q)
}

#[cfg(test)]
#[path = "route_edit_tests.rs"]
mod tests;
