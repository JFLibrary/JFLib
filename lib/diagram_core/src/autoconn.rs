// autoconn -- (JD21) connectors that look after the shapes they join: a 2-D shape dropped on a
// connector is put into the flow (connector splitting, CN-08) and the shapes after it make
// room, a deleted shape takes its connectors along and the flow through it heals (CN-09),
// placeable shapes move away from a dropped one (PlowCode, PR-09)
//
//   page_splits(doc, page)            PageShapeSplit of the page (missing: on, Visio's default)
//   splits(doc, shape)                ShapeSplit: a top-level 2-D shape that splits connectors
//   splittable(doc, connector)        a dynamic connector with ShapeSplittable
//   can_insert(doc, owner, shape)     splits and has no connector glued to it yet
//   split_target(doc, owner, box, skip) -> Option<connector>   the connector a shape with this
//                                     box (page) splits when it is dropped there
//   split(tx, eng, connector, shape) -> new connector   Page.SplitConnector: the connector ends
//                                     at the shape, a copy of it (without its text) runs from
//                                     the shape to where it ended
//   insert(tx, eng, owner, shape, connector, gap) -> Inserted   split, the shape lined up with
//                                     the one before it, the shapes after it moved on as far as
//                                     the room needs
//   downstream(doc, owner, shape, stop) -> shapes   what follows a shape through connectors
//   plow(tx, eng, owner, shapes, gap) -> moved   placeable shapes under the dropped ones move
//                                     away (PlowCode / ShapePlowCode)
//   heal(tx, eng, owner, deleted) -> Healed   before shapes are deleted: their connectors to
//                                     delete as well, the flow through a shape joined again
//   spacing(doc, page) -> [x, y]      the page's spacing between shapes (AvenueSizeX/Y)
//
// Every point is on the page in drawing units (inches, y up).  Nothing runs on its own: the
// app calls these when the user drops, pastes, moves or deletes (PLAN.md, "Fallen").
//
// Visio's rules -- belegt (Microsoft Learn: the ShapeSplit, ShapeSplittable, PageShapeSplit,
// PlowCode and ShapePlowCode cells; Page.SplitConnector, ApplicationSettings.
// ConnectorSplittingEnabled and DeleteConnectorsEnabled, Shape.DeleteEx / VisDeleteFlags; the
// Visio blog "Inserting and Deleting Shapes in Visio 2010"):
// * Splitting needs all of: the app's option (Enable connector splitting), the page
//   (PageShapeSplit 1, the default), the shape (ShapeSplit 1; a 2-D shape or a placeable 1-D
//   one -- only 2-D here) and the connector (ShapeSplittable 1, a routable 1-D connector).
// * SplitConnector "duplicates the connector (except for text), positions the shape
//   automatically, and glues the shape to the two resulting connectors"; it returns the copy.
// * A drop on a connector moves the shapes downstream of the insertion "just enough to make
//   room"; a downstream shape beside the flow does not move; the new shape is aligned with the
//   upstream shape and spaced from it.
// * PlowCode 0 / 1: placeable shapes do not / do move away when a placeable shape is dropped
//   near them (Page Setup › "Move other shapes away on drop"); ShapePlowCode 0 as the page
//   says, 1 never, 2 always.
// * Deleting a shape deletes the connectors glued to it (the app's option Delete connectors
//   when deleting shapes) except connectors with text; the option "also determines whether two
//   connectors reconnect when a shape to which they are both connected is deleted"; with more
//   than one connector in and out the reconnection is ambiguous and nothing heals; the other
//   shapes do not move to close the gap.
//
// Angenommen (the user's check in Visio may correct it):
// * The connector a dropped shape splits is a splittable connector on the page whose way
//   crosses the shape's box while both its ends lie outside the box and the box overlaps none of
//   the shapes it is glued to; among several the one whose way passes nearest the box's middle.
//   A shape that already has a connector glued to it splits nothing (it is in a flow already).
// * The original connector keeps its Begin and its text and ends at the shape; the copy starts
//   at the shape and takes over the old End's glue (a connection point stays that point, a
//   whole shape stays dynamic, a free end stays where it was).  Both glue to the dropped shape
//   as a whole (dynamic glue).
// * The flow's axis is the main direction from the shape before to the shape after (else from
//   Begin to End).  The dropped shape's middle goes on the middle line of the shape before when
//   their bands across the axis overlap, and at least the page's spacing (AvenueSizeX/Y) after
//   it; when the shape after is then nearer than the spacing, it and the shapes downstream of it
//   (through connectors Begin -> End) that do not lie before the dropped shape move on along
//   the axis by what is missing.  Fixed shapes (ShapeFixedCode &H7, LockMoveX/Y), containers
//   and shapes with ShapePlowCode 1 stay, and so does what lies only behind them.  This always
//   comes with a split; PlowCode adds plowing for every drop.
// * Plowing: a placeable top-level 2-D shape (no container, no member of one, no callout, not
//   ObjType 4, not fixed) whose box overlaps a dropped shape's box moves the shortest way out of
//   it to the page's spacing, and in turn pushes what it then overlaps (each shape moves once).
// * Healing counts dynamic connectors only (plain lines lose their glue as before); a connector
//   whose both ends are on the deleted shape is deleted; with exactly one connector in (End on
//   the shape) and one out (Begin on it) the one coming in survives (the one going out when
//   only that one has text; both with text: no healing) and takes the other one's far end; a
//   heal that would join a shape to itself does not happen.  Healing runs through chains of
//   deleted shapes.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::glue::{self, End, Target};
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;
use crate::{field, layout, ops, route, structure};

#[cfg(test)]
#[path = "autoconn_tests.rs"]
mod tests;

fn num(doc: &Document, sheet: SheetRef, cell: &str) -> Option<f64> {
    let c = doc.sym(cell)?;
    doc.resolve(sheet, &CellAddr::single(c)).num().filter(|v| v.is_finite())
}

// the shape an end is glued to
fn shape_of(t: Target) -> Option<ShapeRef> {
    match t {
        Target::Point { shape, .. } | Target::Shape(shape) => Some(shape),
        Target::None => None,
    }
}

// the top-level shape a shape belongs to
fn top_of(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut cur = s;
    for _ in 0..1024 {
        match doc.shape(cur).and_then(|x| x.parent()) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

fn centre(b: [f64; 4]) -> [f64; 2] {
    [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5]
}

// two boxes share an area
fn overlap(a: [f64; 4], b: [f64; 4]) -> bool {
    let e = 1e-9;
    a[0] < b[2] - e && b[0] < a[2] - e && a[1] < b[3] - e && b[1] < a[3] - e
}

fn inside(b: [f64; 4], p: [f64; 2]) -> bool {
    p[0] > b[0] && p[0] < b[2] && p[1] > b[1] && p[1] < b[3]
}

// the distance from p to the segment a-b
fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    crate::geometry::sqrt(qx * qx + qy * qy)
}

/// whether shapes split connectors on the page: PageShapeSplit (missing: yes, Visio's default)
pub fn page_splits(doc: &Document, page: PageRef) -> bool {
    num(doc, SheetRef::Page(page), "PageShapeSplit").is_none_or(|v| v != 0.0)
}

/// whether a shape splits connectors it is dropped on: a top-level 2-D shape (no guide, no
/// container, no callout) with ShapeSplit 1
pub fn splits(doc: &Document, s: ShapeRef) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    if sh.is_deleted() || sh.parent().is_some() || sh.header().kind == ShapeKind::Guide || glue::is_one_d(doc, s) {
        return false;
    }
    if structure::is_container(doc, s) || structure::kind(doc, s) == Some(structure::Kind::Callout) {
        return false;
    }
    num(doc, SheetRef::Shape(s), "ShapeSplit").is_some_and(|v| v != 0.0)
}

/// whether a connector can be split: a top-level dynamic connector with ShapeSplittable 1
pub fn splittable(doc: &Document, c: ShapeRef) -> bool {
    doc.shape(c).is_some_and(|x| x.parent().is_none() && !x.is_deleted())
        && glue::is_connector(doc, c)
        && num(doc, SheetRef::Shape(c), "ShapeSplittable").is_some_and(|v| v != 0.0)
}

/// whether a shape may be put into a connector: it splits, and no connector is glued to it yet
pub fn can_insert(doc: &Document, owner: Owner, s: ShapeRef) -> bool {
    splits(doc, s) && glue::dependents(doc, owner, &[s]).is_empty()
}

/// the connector a shape with box `b` (page) splits when dropped there: a splittable connector
/// whose way crosses the box, both ends outside it, the box over none of the shapes it is glued
/// to, nearest the box's middle; connectors glued to a shape of `skip` (the dropped shape and
/// what moves with it) do not count
pub fn split_target(doc: &Document, owner: Owner, b: [f64; 4], skip: &[ShapeRef]) -> Option<ShapeRef> {
    if !b.iter().all(|v| v.is_finite()) || b[2] <= b[0] || b[3] <= b[1] {
        return None;
    }
    let mid = centre(b);
    let mut best: Option<(f64, ShapeRef)> = None;
    'next: for c in doc.top_shapes(owner) {
        if skip.contains(&c) || !splittable(doc, c) {
            continue;
        }
        for t in glue::glued(doc, c) {
            if let Some(s) = shape_of(t) {
                let s = top_of(doc, s);
                if skip.contains(&s) || overlap(b, structure::shape_box(doc, s)) {
                    continue 'next;
                }
            }
        }
        let way = glue::path(doc, c);
        let (Some(&p), Some(&q)) = (way.first(), way.last()) else { continue };
        if way.len() < 2 || inside(b, p) || inside(b, q) {
            continue;
        }
        if !way.windows(2).any(|w| route::hits_box(w[0], w[1], &b)) {
            continue;
        }
        let d = way.windows(2).map(|w| seg_dist(mid, w[0], w[1])).fold(f64::INFINITY, f64::min);
        if best.is_none_or(|x| d < x.0) {
            best = Some((d, c));
        }
    }
    best.map(|x| x.1)
}

// whether a shape has text of its own or from its master (fields as they show)
fn has_text(doc: &Document, s: ShapeRef) -> bool {
    !field::plain(doc, s).trim().is_empty()
}

/// split connector `c` with shape `s` (Visio's Page.SplitConnector): a copy of the connector
/// without its text is made, `c` ends at `s`, the copy runs from `s` to where `c` ended (with
/// that end's glue); both glue to `s` as a whole and route -> the copy.  Invalid: `c` is no
/// splittable connector, `s` is 1-D or elsewhere
pub fn split(tx: &mut Transaction, eng: &mut Engine, c: ShapeRef, s: ShapeRef) -> Result<ShapeRef, EditError> {
    let owner = c.owner;
    if s.owner != owner || s == c || !splittable(tx.doc(), c) || tx.doc().shape(s).is_none() || glue::is_one_d(tx.doc(), s) {
        return Err(EditError::Invalid);
    }
    let (g, (_, ep), mid) = {
        let doc = tx.doc();
        (glue::glued(doc, c), glue::ends(doc, c).ok_or(EditError::Invalid)?, ops::frame(doc, s).centre())
    };
    // the copy: every cell and the format; glue to shapes outside the copy is let go
    let clip = ops::copy(tx.doc(), &[c]);
    let made = ops::paste(tx, eng, owner, &clip, [0.0, 0.0])?;
    let n = *made.first().ok_or(EditError::NoShape)?;
    if has_text(tx.doc(), n) {
        tx.set_text(n, None)?;
        eng.recalc(tx, &[SheetRef::Shape(n)])?;
    }
    glue::glue(tx, eng, c, End::End, mid, Target::Shape(s))?;
    glue::glue(tx, eng, n, End::Begin, mid, Target::Shape(s))?;
    glue::glue(tx, eng, n, End::End, ep, g[1])?;
    glue::update(tx, eng, &[c, n])?;
    Ok(n)
}

/// what insert did
#[derive(Clone, Debug, PartialEq)]
pub struct Inserted {
    /// the new connector (from the shape to the old End)
    pub connector: ShapeRef,
    /// the shapes moved: the inserted one when it was lined up, those after it
    pub moved: Vec<ShapeRef>,
}

// the flow's axis: index 0 (x) or 1 (y) and its sign, from a to b
fn axis(a: [f64; 2], b: [f64; 2]) -> (usize, f64) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    if dx.abs() >= dy.abs() {
        (0, if dx < 0.0 { -1.0 } else { 1.0 })
    } else {
        (1, if dy < 0.0 { -1.0 } else { 1.0 })
    }
}

// a box's start and end along the axis (k, sign)
fn span(b: [f64; 4], k: usize, sign: f64) -> (f64, f64) {
    if sign > 0.0 { (b[k], b[k + 2]) } else { (-b[k + 2], -b[k]) }
}

// whether a shape may be moved by a drop (not fixed, not ShapePlowCode 1, no container)
fn movable(doc: &Document, s: ShapeRef) -> bool {
    !layout::fixed(doc, s) && num(doc, SheetRef::Shape(s), "ShapePlowCode") != Some(1.0) && !structure::is_container(doc, s)
}

// the top-level shapes right after x: the End shapes of the connectors whose Begin is on x
fn successors(doc: &Document, owner: Owner, x: ShapeRef) -> Vec<ShapeRef> {
    let mut out = Vec::new();
    for c in glue::dependents(doc, owner, &[x]) {
        if !glue::is_one_d(doc, c) {
            continue;
        }
        let g = glue::glued(doc, c);
        let (Some(b), Some(e)) = (shape_of(g[0]), shape_of(g[1])) else { continue };
        if top_of(doc, b) == x {
            let e = top_of(doc, e);
            if e != x && !out.contains(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// the shapes downstream of `from` through connectors (Begin on one, End on the next),
/// top-level, in the order found (breadth first); `stop` are never entered
pub fn downstream(doc: &Document, owner: Owner, from: ShapeRef, stop: &[ShapeRef]) -> Vec<ShapeRef> {
    let mut out: Vec<ShapeRef> = Vec::new();
    let mut seen: BTreeSet<ShapeRef> = stop.iter().copied().collect();
    seen.insert(from);
    let mut queue = alloc::vec![from];
    let mut i = 0;
    while i < queue.len() {
        let x = queue[i];
        i += 1;
        for e in successors(doc, owner, x) {
            if seen.insert(e) {
                out.push(e);
                queue.push(e);
            }
        }
    }
    out
}

/// put shape `s` into connector `c` (a drop): split, `s` lined up with the shape before it and
/// spaced from it, the shapes after it moved on along the flow where the room is short (`gap`:
/// the page's spacing in x and y) -> the new connector and what moved; connectors follow
pub fn insert(tx: &mut Transaction, eng: &mut Engine, owner: Owner, s: ShapeRef, c: ShapeRef, gap: [f64; 2]) -> Result<Inserted, EditError> {
    let (up, down, k, sign) = {
        let doc = tx.doc();
        let g = glue::glued(doc, c);
        let (bp, ep) = glue::ends(doc, c).ok_or(EditError::Invalid)?;
        let up = shape_of(g[0]).map(|x| top_of(doc, x)).filter(|&x| x != s);
        let down = shape_of(g[1]).map(|x| top_of(doc, x)).filter(|&x| x != s);
        let a = up.map_or(bp, |u| centre(structure::shape_box(doc, u)));
        let z = down.map_or(ep, |d| centre(structure::shape_box(doc, d)));
        let (k, sign) = axis(a, z);
        (up, down, k, sign)
    };
    let n = split(tx, eng, c, s)?;
    let mut moved: Vec<ShapeRef> = Vec::new();
    let j = 1 - k;
    // the dropped shape: on the middle line of the one before it, the spacing after it
    if let Some(u) = up {
        let (ub, sb) = {
            let doc = tx.doc();
            (structure::shape_box(doc, u), structure::shape_box(doc, s))
        };
        let mut d = [0.0f64; 2];
        if ub[j] < sb[j + 2] && sb[j] < ub[j + 2] {
            d[j] = centre(ub)[j] - centre(sb)[j];
        }
        let (_, u_end) = span(ub, k, sign);
        let (s_start, _) = span(sb, k, sign);
        if s_start - u_end < gap[k] {
            d[k] = (gap[k] - (s_start - u_end)) * sign;
        }
        if (d[0].abs() > 1e-9 || d[1].abs() > 1e-9) && movable(tx.doc(), s) {
            let along = structure::carried(tx.doc(), &[s]);
            ops::move_shapes(tx, eng, &along, d)?;
            moved.push(s);
        }
    }
    // the shapes after it: on by what the room is short
    if let Some(dn) = down {
        let set: Vec<ShapeRef> = {
            let doc = tx.doc();
            let (s_start, s_end) = span(structure::shape_box(doc, s), k, sign);
            let (d_start, _) = span(structure::shape_box(doc, dn), k, sign);
            let short = gap[k] - (d_start - s_end);
            let mut set: Vec<ShapeRef> = Vec::new();
            if short > 1e-9 && movable(doc, dn) {
                // breadth first from the shape after; a shape that cannot move, or lies before
                // the inserted one (beside the flow), stays and is not gone through
                let mut seen: BTreeSet<ShapeRef> = BTreeSet::from([s, dn]);
                seen.extend(up);
                set.push(dn);
                let mut i = 0;
                while i < set.len() {
                    let x = set[i];
                    i += 1;
                    for e in successors(doc, owner, x) {
                        if !seen.insert(e) || !movable(doc, e) {
                            continue;
                        }
                        let (e_start, _) = span(structure::shape_box(doc, e), k, sign);
                        if e_start + 1e-9 < s_start {
                            continue;
                        }
                        set.push(e);
                    }
                }
            }
            set
        };
        if !set.is_empty() {
            let short = {
                let doc = tx.doc();
                let (_, s_end) = span(structure::shape_box(doc, s), k, sign);
                let (d_start, _) = span(structure::shape_box(doc, dn), k, sign);
                gap[k] - (d_start - s_end)
            };
            let mut d = [0.0f64; 2];
            d[k] = short * sign;
            let along = structure::carried(tx.doc(), &set);
            ops::move_shapes(tx, eng, &along, d)?;
            moved.extend(set);
        }
    }
    // the connectors of what moved, the two halves
    let mut deps = glue::dependents(tx.doc(), owner, &moved);
    for x in [c, n] {
        if !deps.contains(&x) {
            deps.push(x);
        }
    }
    glue::update(tx, eng, &deps)?;
    Ok(Inserted { connector: n, moved })
}

/// the page's spacing between shapes in drawing units: AvenueSizeX/Y (Seite einrichten ›
/// Layout und Routing; Visio's metric default 7.5 mm), through the drawing scale
pub fn spacing(doc: &Document, page: PageRef) -> [f64; 2] {
    let s = SheetRef::Page(page);
    let get = |n: &str| num(doc, s, n).filter(|v| *v > 0.0);
    let scale = match (get("PageScale"), get("DrawingScale")) {
        (Some(ps), Some(ds)) => ds / ps,
        _ => 1.0,
    };
    [get("AvenueSizeX").unwrap_or(7.5 / 25.4) * scale, get("AvenueSizeY").unwrap_or(7.5 / 25.4) * scale]
}

// a shape a drop may push: placeable, top-level 2-D, no container or member of one, no callout
fn pushable(doc: &Document, s: ShapeRef, page_plows: bool) -> bool {
    let Some(sh) = doc.shape(s) else { return false };
    if sh.is_deleted() || sh.parent().is_some() || sh.header().kind == ShapeKind::Guide || glue::is_one_d(doc, s) {
        return false;
    }
    if structure::is_container(doc, s) || !structure::containers_of(doc, s).is_empty() || structure::kind(doc, s) == Some(structure::Kind::Callout) {
        return false;
    }
    let sheet = SheetRef::Shape(s);
    if num(doc, sheet, "ObjType").is_some_and(|v| (v as i64) & 4 != 0) || layout::fixed(doc, s) {
        return false;
    }
    match num(doc, sheet, "ShapePlowCode").map(|v| v as i64) {
        Some(1) => false,
        Some(2) => true,
        _ => page_plows,
    }
}

/// whether a drop on the page may push shapes at all: PlowCode 1, or a shape on it with
/// ShapePlowCode 2
pub fn plows(doc: &Document, page: PageRef) -> bool {
    num(doc, SheetRef::Page(page), "PlowCode") == Some(1.0) || doc.top_shapes(Owner::Page(page)).any(|s| num(doc, SheetRef::Shape(s), "ShapePlowCode") == Some(2.0))
}

/// the dropped `shapes` push the placeable shapes they cover out of the way (PlowCode of the
/// page, ShapePlowCode of each shape): the shortest way out to the page's spacing `gap`, and
/// what those then cover in turn; each shape moves once -> the moved shapes (connectors follow)
pub fn plow(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], gap: [f64; 2]) -> Result<Vec<ShapeRef>, EditError> {
    let page_plows = match owner {
        Owner::Page(p) => num(tx.doc(), SheetRef::Page(p), "PlowCode") == Some(1.0),
        Owner::Master(_) => false,
    };
    let mut still: BTreeSet<ShapeRef> = structure::carried(tx.doc(), shapes).into_iter().collect();
    let mut queue: Vec<ShapeRef> = shapes.to_vec();
    let mut moved: Vec<ShapeRef> = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        let p = queue[i];
        i += 1;
        let pb = structure::shape_box(tx.doc(), p);
        let hits: Vec<ShapeRef> = {
            let doc = tx.doc();
            doc.top_shapes(owner).filter(|x| !still.contains(x) && pushable(doc, *x, page_plows) && overlap(pb, structure::shape_box(doc, *x))).collect()
        };
        for x in hits {
            let xb = structure::shape_box(tx.doc(), x);
            // the four ways out, the shortest wins (right, down, left, up on ties)
            let ways = [[pb[2] + gap[0] - xb[0], 0.0], [0.0, (pb[1] - gap[1]) - xb[3]], [(pb[0] - gap[0]) - xb[2], 0.0], [0.0, pb[3] + gap[1] - xb[1]]];
            let len = |d: &[f64; 2]| d[0].abs() + d[1].abs();
            let Some(d) = ways.iter().copied().filter(|d| len(d) > 1e-12).min_by(|a, b| len(a).total_cmp(&len(b))) else { continue };
            let along = structure::carried(tx.doc(), &[x]);
            ops::move_shapes(tx, eng, &along, d)?;
            still.insert(x);
            moved.push(x);
            queue.push(x);
        }
    }
    if !moved.is_empty() {
        let deps = glue::dependents(tx.doc(), owner, &moved);
        glue::update(tx, eng, &deps)?;
    }
    Ok(moved)
}

/// what heal decided
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Healed {
    /// connectors to delete with the shapes
    pub gone: Vec<ShapeRef>,
    /// connectors that now join the shapes before and after a deleted one: route them once the
    /// shapes are gone
    pub joined: Vec<ShapeRef>,
}

// whether shape s is one of `dead` or inside one
fn doomed(doc: &Document, s: ShapeRef, dead: &BTreeSet<ShapeRef>) -> bool {
    let mut cur = Some(s);
    let mut n = 0;
    while let (Some(r), true) = (cur, n < 1024) {
        if dead.contains(&r) {
            return true;
        }
        cur = doc.shape(r).and_then(|x| x.parent());
        n += 1;
    }
    false
}

/// before `deleted` are deleted (Visio's "Delete connectors when deleting shapes"): the dynamic
/// connectors glued to a deleted 2-D shape go with it unless they have text; where exactly one
/// comes in (End) and one goes out (Begin), one of them survives and joins the shapes before and
/// after (the flow heals).  Glues the survivors anew; the caller deletes `gone` with the shapes,
/// detaches what stays and routes `joined` after the deletion
pub fn heal(tx: &mut Transaction, eng: &mut Engine, owner: Owner, deleted: &[ShapeRef]) -> Result<Healed, EditError> {
    let dead: BTreeSet<ShapeRef> = deleted.iter().copied().collect();
    let mut out = Healed::default();
    for &x in deleted {
        if tx.doc().shape(x).is_none() || glue::is_one_d(tx.doc(), x) {
            continue;
        }
        let me: BTreeSet<ShapeRef> = BTreeSet::from([x]);
        // the dynamic connectors glued to x (or inside it) that stay so far: Begin on x, End on x
        let list = |doc: &Document, gone: &[ShapeRef]| -> Vec<(ShapeRef, bool, bool)> {
            let at = |t: Target| shape_of(t).is_some_and(|s| doomed(doc, s, &me));
            glue::dependents(doc, owner, &[x])
                .into_iter()
                .filter(|&c| !gone.contains(&c) && !doomed(doc, c, &dead) && glue::is_connector(doc, c))
                .map(|c| {
                    let g = glue::glued(doc, c);
                    (c, at(g[0]), at(g[1]))
                })
                .filter(|&(_, b, e)| b || e)
                .collect()
        };
        let conns = list(tx.doc(), &out.gone);
        let ins: Vec<ShapeRef> = conns.iter().filter(|c| c.2 && !c.1).map(|c| c.0).collect();
        let outs: Vec<ShapeRef> = conns.iter().filter(|c| c.1 && !c.2).map(|c| c.0).collect();
        if ins.len() == 1 && outs.len() == 1 {
            let (a, b) = (ins[0], outs[0]);
            let doc = tx.doc();
            let (ga, gb) = (glue::glued(doc, a), glue::glued(doc, b));
            let (ea, eb) = (glue::ends(doc, a), glue::ends(doc, b));
            let (ta, tb) = (has_text(doc, a), has_text(doc, b));
            let far = (shape_of(ga[0]).map(|s| top_of(doc, s)), shape_of(gb[1]).map(|s| top_of(doc, s)));
            let loops = matches!(far, (Some(p), Some(q)) if p == q);
            if let (false, Some(ea), Some(eb), false) = (loops, ea, eb, ta && tb) {
                if tb && !ta {
                    // the one going out has the text: it starts where the one coming in did
                    glue::glue(tx, eng, b, End::Begin, ea.0, ga[0])?;
                    out.gone.push(a);
                    if !out.joined.contains(&b) {
                        out.joined.push(b);
                    }
                } else {
                    glue::glue(tx, eng, a, End::End, eb.1, gb[1])?;
                    out.gone.push(b);
                    if !out.joined.contains(&a) {
                        out.joined.push(a);
                    }
                }
            }
        }
        // what is still glued to x goes, unless it has text
        for (c, _, _) in list(tx.doc(), &out.gone) {
            if !has_text(tx.doc(), c) {
                out.gone.push(c);
            }
        }
    }
    out.joined.retain(|c| !out.gone.contains(c));
    Ok(out)
}
