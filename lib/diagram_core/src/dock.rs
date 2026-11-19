// dock -- 2-D shapes that snap onto connection points (JD13; SP-12, CN-04 "Andocken und
// Einrasten": equipment into racks, later doors into walls, valves into pipes)
//
//   snap(doc, owner, shapes, reach) -> Option<Snap>   the move that puts an outward point of
//                                    one of `shapes` on an inward point of another shape
//   dock(tx, eng, owner, shapes, reach) -> Option<Snap>   ... and do it (ops::move_shapes of the
//                                    shapes and what they carry; connectors follow in the app)
//   reach(doc, owner, paper)         a distance on paper in the page's drawing units
//
// Visio's point-to-point glue of 2-D shapes: a connection point of Type 1 ("outward") on the
// shape that moves meets one of Type 0 or 2 ("inward", "both") on another shape.  The nearest
// pair within `reach` (page units) wins.  A container that would take the shape (its
// categories, structure::accepts) and holds the middle of the shape's box is reached from
// anywhere inside it: a server dropped anywhere over a rack goes onto the nearest unit of its
// rail.  The shapes themselves, what they carry (members, callouts) and 1-D shapes are no
// targets; only top-level shapes of the owner dock and are docked on.
//
// Nothing here records glue: membership in the container (structure::settle after the move)
// keeps a device with its rack, in JFDiagrammer as in Visio.

use alloc::vec::Vec;

use crate::document::{Document, Owner, ShapeRef, SheetRef};
use crate::sheet::CellAddr;
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::geometry::sqrt;
use crate::glue::{self, ConnPoint};
use crate::ops;
use crate::structure;

/// a move that docks a shape
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Snap {
    /// the move (page units)
    pub d: [f64; 2],
    /// the shape whose outward point docks, and the IX of that point's row
    pub shape: ShapeRef,
    pub point: u32,
    /// the shape it docks on, and the IX of the inward point's row
    pub target: ShapeRef,
    pub target_point: u32,
}

const OUTWARD: u8 = 1;

fn inward(p: &ConnPoint) -> bool {
    p.kind != OUTWARD
}

/// the move that docks one of `shapes` (2-D shapes with an outward point) on the nearest inward
/// point of another top-level shape of `owner` within `reach`, or anywhere inside a container
/// that takes it; None: nothing to dock on
pub fn snap(doc: &Document, owner: Owner, shapes: &[ShapeRef], reach: f64) -> Option<Snap> {
    let moving = structure::carried(doc, shapes);
    let mut best: Option<(f64, Snap)> = None;
    for &s in shapes {
        if glue::is_one_d(doc, s) {
            continue;
        }
        let outs: Vec<ConnPoint> = glue::connection_points(doc, s).into_iter().filter(|p| p.kind == OUTWARD).collect();
        if outs.is_empty() {
            continue;
        }
        let b = structure::shape_box(doc, s);
        let mid = [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5];
        for t in doc.top_shapes(owner) {
            if moving.contains(&t) || glue::is_one_d(doc, t) {
                continue;
            }
            let ins: Vec<ConnPoint> = glue::connection_points(doc, t).into_iter().filter(inward).collect();
            if ins.is_empty() {
                continue;
            }
            let tb = structure::shape_box(doc, t);
            let inside = structure::is_container(doc, t) && structure::accepts(doc, t, s) && mid[0] >= tb[0] && mid[0] <= tb[2] && mid[1] >= tb[1] && mid[1] <= tb[3];
            for o in &outs {
                for i in &ins {
                    let d = [i.at[0] - o.at[0], i.at[1] - o.at[1]];
                    let dist = sqrt(d[0] * d[0] + d[1] * d[1]);
                    if !inside && dist > reach {
                        continue;
                    }
                    // a nearer pair wins; inside a taking container the pair counts as near
                    let score = if inside { dist - 1e9 } else { dist };
                    if best.as_ref().is_none_or(|(b, _)| score < *b) {
                        best = Some((score, Snap { d, shape: s, point: o.row, target: t, target_point: i.row }));
                    }
                }
            }
        }
    }
    best.map(|(_, s)| s)
}

/// `paper` inches on the paper of `owner`'s page in its drawing units (the page's scale; a
/// master or a page without scale: 1:1) -- the reach of a gesture that ends on the page
pub fn reach(doc: &Document, owner: Owner, paper: f64) -> f64 {
    let Owner::Page(p) = owner else { return paper };
    let sh = SheetRef::Page(p);
    let num = |n: &str| doc.sym(n).and_then(|c| doc.resolve(sh, &CellAddr::single(c)).num()).filter(|v| *v > 0.0 && v.is_finite());
    match (num("PageScale"), num("DrawingScale")) {
        (Some(ps), Some(ds)) if (ds / ps).is_finite() => paper * ds / ps,
        _ => paper,
    }
}

/// dock `shapes` (see snap) and move them with what they carry; None: nothing moved
pub fn dock(tx: &mut Transaction, eng: &mut Engine, owner: Owner, shapes: &[ShapeRef], reach: f64) -> Result<Option<Snap>, EditError> {
    let Some(s) = snap(tx.doc(), owner, shapes, reach) else { return Ok(None) };
    if s.d[0].abs() < 1e-9 && s.d[1].abs() < 1e-9 {
        return Ok(Some(s));
    }
    let along = structure::carried(tx.doc(), shapes);
    ops::move_shapes(tx, eng, &along, s.d)?;
    Ok(Some(s))
}
