// structure::edge -- (JD21, ST-03) boundary members: shapes that sit on a container's edge (tabs on
// a panel), stay on it and go with it
//
//   REL_LEFT, REL_RIGHT, REL_TOP, REL_BOTTOM    the Relationships types 7 .. 10 (the member names
//                                       the container whose left / right / top / bottom edge it
//                                       sits on; learn.microsoft.com "Relationships Cell")
//   edge_of(doc, s, c) -> Option<Edge>  the edge of container c that member s sits on
//   boundary_members(doc, c)            the members of c on its edges
//   straddles(doc, c, s) -> Option<Edge>   the edge of c shape s lies across where it is now
//   targets(doc, owner, s, moving)      the containers whose edge s lies across (and that take it)
//   align(tx, eng, c) -> moved          every boundary member back onto its edge
//
// Visio (belegt: Visio team blog "Details on Container Behaviors", 2009): "To put a shape on the
// boundary of a container, simply drag it there"; boundary shapes stay members (types 1 and 4 as
// any member) "but they behave differently during resize actions": "it will move with that
// boundary"; the highlight shows along the one edge.  Angenommen (the Gegenprobe checks): a 2-D
// shape is on an edge when the edge's line runs through its box (not wholly inside nor outside)
// and its middle lies within the edge's length, the nearest edge wins at a corner; it then sits
// with its middle on the edge's line (dropped there it is put onto it), slides along the edge and
// stays within its ends; a plain member moved partly out stays a plain member (the container
// grows, JD10) -- only a shape that comes from outside or sits on an edge already takes one;
// boundary members do not count for the container's automatic size; only
// upright containers have boundary members; the container side names them as members (type 1)
// only, the member side has type 4 and its edge type.

use alloc::vec::Vec;

use super::{accepts, container, is_container, joins_by_position, rel, shape_box, upright, Edge, EPS};
use crate::document::{Document, Owner, ShapeRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;

/// the member names the container whose left edge it sits on
pub const REL_LEFT: u32 = 7;
/// ... right edge
pub const REL_RIGHT: u32 = 8;
/// ... top edge
pub const REL_TOP: u32 = 9;
/// ... bottom edge
pub const REL_BOTTOM: u32 = 10;

const EDGES: [(u32, Edge); 4] = [(REL_LEFT, Edge::Left), (REL_RIGHT, Edge::Right), (REL_TOP, Edge::Top), (REL_BOTTOM, Edge::Bottom)];

/// the type of an edge
pub fn rel_of(e: Edge) -> Option<u32> {
    EDGES.iter().find(|x| x.1 == e).map(|x| x.0)
}

/// the edge of container `c` that member `s` sits on (its Relationships, types 7 .. 10)
pub fn edge_of(doc: &Document, s: ShapeRef, c: ShapeRef) -> Option<Edge> {
    let r = rel::relations(doc, s);
    EDGES.iter().find(|(k, _)| r.get(*k).contains(&c)).map(|x| x.1)
}

/// the members of `c` that sit on one of its edges
pub fn boundary_members(doc: &Document, c: ShapeRef) -> Vec<(ShapeRef, Edge)> {
    super::members(doc, c).into_iter().filter_map(|m| edge_of(doc, m, c).map(|e| (m, e))).collect()
}

/// the edge of container `c` whose line runs through the box of `s` with the middle of `s` within
/// the edge's length (the nearest one at a corner); None: no edge, or `c` is turned
pub fn straddles(doc: &Document, c: ShapeRef, s: ShapeRef) -> Option<Edge> {
    if !is_container(doc, c) || !upright(doc, c) {
        return None;
    }
    let cb = shape_box(doc, c);
    let b = shape_box(doc, s);
    let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
    let across_x = |x: f64| b[0] < x - EPS && x + EPS < b[2];
    let across_y = |y: f64| b[1] < y - EPS && y + EPS < b[3];
    let in_y = mid[1] >= cb[1] - EPS && mid[1] <= cb[3] + EPS;
    let in_x = mid[0] >= cb[0] - EPS && mid[0] <= cb[2] + EPS;
    let mut best: Option<(f64, Edge)> = None;
    let mut take = |d: f64, e: Edge| {
        if best.is_none_or(|x| d < x.0) {
            best = Some((d, e));
        }
    };
    if in_y && across_x(cb[0]) {
        take((mid[0] - cb[0]).abs(), Edge::Left);
    }
    if in_y && across_x(cb[2]) {
        take((mid[0] - cb[2]).abs(), Edge::Right);
    }
    if in_x && across_y(cb[3]) {
        take((mid[1] - cb[3]).abs(), Edge::Top);
    }
    if in_x && across_y(cb[1]) {
        take((mid[1] - cb[1]).abs(), Edge::Bottom);
    }
    best.map(|x| x.1)
}

/// the containers on `owner` (not among `moving`) whose edge shape `s` lies across, that take
/// it and are not locked, with the edge
pub fn targets(doc: &Document, owner: Owner, s: ShapeRef, moving: &[ShapeRef]) -> Vec<(ShapeRef, Edge)> {
    if !joins_by_position(doc, s) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for c in doc.top_shapes(owner) {
        if c == s || moving.contains(&c) || !is_container(doc, c) || !accepts(doc, c, s) || container(doc, c).is_some_and(|i| i.locked) {
            continue;
        }
        if let Some(e) = straddles(doc, c, s) {
            out.push((c, e));
        }
    }
    out
}

/// write the edge member `s` sits on of container `c` (None: none)
pub(crate) fn set_edge(tx: &mut Transaction, s: ShapeRef, c: ShapeRef, e: Option<Edge>) -> Result<(), EditError> {
    let mut r = rel::relations(tx.doc(), s);
    let before = r.clone();
    for (k, _) in EDGES {
        r.get_mut(k).retain(|&x| x != c);
    }
    if let Some(k) = e.and_then(rel_of) {
        r.get_mut(k).push(c);
    }
    r.groups.retain(|g| !g.1.is_empty());
    if r == before {
        return Ok(());
    }
    rel::write(tx, s, &r)
}

/// every boundary member of `c` back onto its edge: its middle on the edge's line, within the
/// edge's length -> the members moved (with what they carry)
pub fn align(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef) -> Result<Vec<ShapeRef>, EditError> {
    let plan: Vec<(ShapeRef, [f64; 2])> = {
        let doc = tx.doc();
        if !upright(doc, c) {
            return Ok(Vec::new());
        }
        let cb = shape_box(doc, c);
        boundary_members(doc, c)
            .into_iter()
            .filter_map(|(m, e)| {
                let b = shape_box(doc, m);
                let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
                let clamp = |v: f64, a: f64, z: f64| if a <= z { v.clamp(a, z) } else { v };
                let to = match e {
                    Edge::Left => [cb[0], clamp(mid[1], cb[1], cb[3])],
                    Edge::Right => [cb[2], clamp(mid[1], cb[1], cb[3])],
                    Edge::Top => [clamp(mid[0], cb[0], cb[2]), cb[3]],
                    Edge::Bottom => [clamp(mid[0], cb[0], cb[2]), cb[1]],
                    Edge::None => return None,
                };
                let d = [to[0] - mid[0], to[1] - mid[1]];
                (d[0].abs() > EPS || d[1].abs() > EPS).then_some((m, d))
            })
            .collect()
    };
    let mut moved = Vec::new();
    for (m, d) in plan {
        let along = super::carried(tx.doc(), &[m]);
        crate::ops::move_shapes(tx, eng, &along, d)?;
        moved.extend(along);
    }
    Ok(moved)
}
