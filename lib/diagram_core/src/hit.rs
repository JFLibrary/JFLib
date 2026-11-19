// hit -- hit tests on a page's display list: the shape under a point, shapes in a rectangle (JD7)
//
//   hit::at(&drawing, p, tol) -> Option<Hit>   the topmost shape drawn under p
//   hit::all_at(&drawing, p, tol) -> Vec<Hit>  every shape drawn under p, topmost first, each
//                                    once (the subselection in groups goes down this list)
//   hit::in_rect(&drawing, r, whole) -> Vec<ShapeRef>   the top-level shapes wholly inside r
//                                    (whole) or touching it, in z-order (FEATURES ZF-06)
//   hit::top_boxes(&drawing) -> Vec<(ShapeRef, [f64; 4])>   each top-level shape's box, in
//                                    z-order (alignment guides, selection frames)
//   hit::item_at(&item, p, tol) / item_box(&item)   one item: under p?  its exact box
//   Hit { shape, top, item }         the shape drawn (a subshape too), its top-level shape
//                                    (what a click selects), the item's index
//
// Everything works on render::Drawing: paper inches, y up, the origin at the page's bottom
// left corner (the app maps its pixels there: 96 DIP per inch, y down); `tol` is in paper
// inches.  Only the page's own shapes count: the items of its background pages (their shapes
// belong to another page) and shadows are never hit.  Under a point means: inside a fill
// (even-odd; curves cut into straight pieces within a hundredth of the tolerance, at least
// 1e-4 in), within half its width plus tol of a line (a hairline: tol), inside a text's runs
// (each from the descent to the ascent, grown by tol), inside a picture's rectangle where the
// shape's box keeps it.  Line ends count with their line, a text background as text.
// A box (item_box) is the geometry's without the line width, a text's runs, a picture's
// visible part; a top-level shape's box is that of its geometry and line ends, of its texts
// where it has no geometry.  The drawing's grid (Drawing::query) finds the candidates: a test
// looks at the items near the point or rectangle, not at the whole page.

use alloc::vec::Vec;

use crate::document::{Owner, ShapeRef};
use crate::geometry::{Path, Seg, Sub};
use crate::render::{bbox_of, union, Drawing, ImageItem, Item, ItemKind, Role, TextItem};

/// a shape under a point
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// the shape the item draws (a subshape of a group too)
    pub shape: ShapeRef,
    /// its top-level shape: what a click selects
    pub top: ShapeRef,
    /// the item's index in the drawing
    pub item: u32,
}

/// whether an item belongs to the page itself and is not a shadow
fn own(d: &Drawing, it: &Item) -> bool {
    it.role != Role::Shadow && d.page.is_some_and(|p| it.top.owner == Owner::Page(p))
}

/// the topmost shape drawn under p
pub fn at(d: &Drawing, p: [f64; 2], tol: f64) -> Option<Hit> {
    let tol = if tol.is_finite() { tol.max(0.0) } else { 0.0 };
    let mut cand = Vec::new();
    d.query([p[0] - tol, p[1] - tol, p[0] + tol, p[1] + tol], &mut cand);
    cand.iter().rev().find_map(|&i| {
        let it = &d.items[i as usize];
        (own(d, it) && item_at(it, p, tol)).then_some(Hit { shape: it.shape, top: it.top, item: i })
    })
}

/// every shape drawn under p, the topmost first, each shape once (its topmost item)
pub fn all_at(d: &Drawing, p: [f64; 2], tol: f64) -> Vec<Hit> {
    let tol = if tol.is_finite() { tol.max(0.0) } else { 0.0 };
    let mut cand = Vec::new();
    d.query([p[0] - tol, p[1] - tol, p[0] + tol, p[1] + tol], &mut cand);
    let mut out: Vec<Hit> = Vec::new();
    for &i in cand.iter().rev() {
        let it = &d.items[i as usize];
        if own(d, it) && !out.iter().any(|h| h.shape == it.shape) && item_at(it, p, tol) {
            out.push(Hit { shape: it.shape, top: it.top, item: i });
        }
    }
    out
}

/// the top-level shapes of the page wholly inside r (x0, y0, x1, y1; whole) or touching it
/// (not whole), in z-order; a shape that draws nothing is never inside
pub fn in_rect(d: &Drawing, r: [f64; 4], whole: bool) -> Vec<ShapeRef> {
    let r = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
    if !r.iter().all(|v| v.is_finite()) {
        return Vec::new();
    }
    let mut cand = Vec::new();
    d.query(r, &mut cand);
    // the top-level shapes of the candidates, ascending (z-order)
    let mut tops: Vec<usize> = cand.iter().filter(|&&i| own(d, &d.items[i as usize])).filter_map(|&i| d.top_of_item(i as usize)).collect();
    tops.dedup();
    let mut out = Vec::new();
    for k in tops {
        let range = d.top_range(k);
        let items = || d.items[range.clone()].iter().filter(|it| own(d, it));
        let hit = if whole {
            items().next().is_some() && items().all(|it| inside_rect(item_box(it), r))
        } else {
            items().any(|it| touches(it, r))
        };
        if hit {
            out.push(d.tops[k].shape);
        }
    }
    out
}

/// the box of each top-level shape of the page that draws something, in z-order: its
/// geometry's and line ends' (without the line width), its texts' where it has no geometry
pub fn top_boxes(d: &Drawing) -> Vec<(ShapeRef, [f64; 4])> {
    let mut out = Vec::new();
    for (shape, range) in d.tops() {
        let items = &d.items[range];
        let of = |want: fn(Role) -> bool| items.iter().filter(|it| own(d, it) && want(it.role)).map(item_box).filter(|b| b.iter().all(|v| v.is_finite())).reduce(union);
        if let Some(b) = of(|r| matches!(r, Role::Shape | Role::Arrow)).or_else(|| of(|r| r == Role::Text)) {
            out.push((shape, b));
        }
    }
    out
}

/// whether one item is drawn under p (see the module comment)
pub fn item_at(it: &Item, p: [f64; 2], tol: f64) -> bool {
    let eps = flat_eps(tol);
    match &it.kind {
        ItemKind::Fill { path, .. } => inside(path, p, eps),
        ItemKind::Stroke { path, stroke } => near(path, p, stroke.width.max(0.0) * 0.5 + tol, eps),
        ItemKind::Text(t) => text_quads(t, tol).iter().any(|q| in_poly(q, p)),
        ItemKind::Image(img) => image_at(img, p),
    }
}

/// an item's exact box: the geometry's without the line width, a text's runs, a picture's
/// visible part
pub fn item_box(it: &Item) -> [f64; 4] {
    match &it.kind {
        ItemKind::Fill { path, .. } | ItemKind::Stroke { path, .. } => {
            let mut pts = Vec::new();
            for s in &path.subs {
                flatten(s, 1e-4, &mut pts);
            }
            if pts.is_empty() { it.bbox } else { bbox_of(&pts) }
        }
        ItemKind::Text(t) => {
            let corners: Vec<[f64; 2]> = text_quads(t, 0.0).into_iter().flatten().collect();
            if corners.is_empty() { it.bbox } else { bbox_of(&corners) }
        }
        ItemKind::Image(_) => it.bbox,
    }
}

// ---- geometry

/// how close the straight pieces of a curve stay to it for a tolerance
fn flat_eps(tol: f64) -> f64 {
    (tol * 0.01).max(1e-4)
}

/// a sub-path's points with its curves cut into pieces within eps of the curve (the bound
/// 3/4 max|p0 - 2 p1 + p2| / n^2 of a cubic's second difference)
fn flatten(s: &Sub, eps: f64, out: &mut Vec<[f64; 2]>) {
    out.push(s.start);
    let mut prev = s.start;
    for seg in &s.segs {
        match *seg {
            Seg::Line(q) => out.push(q),
            Seg::Cubic(a, b, q) => {
                let dd = |u: [f64; 2], v: [f64; 2], w: [f64; 2]| {
                    let (x, y) = (u[0] - 2.0 * v[0] + w[0], u[1] - 2.0 * v[1] + w[1]);
                    crate::geometry::sqrt(x * x + y * y)
                };
                let m = dd(prev, a, b).max(dd(a, b, q));
                let n = crate::geometry::ceil(crate::geometry::sqrt(0.75 * m / eps));
                let n = if n.is_finite() { (n as usize).clamp(1, 256) } else { 1 };
                for i in 1..=n {
                    out.push(crate::geometry::cubic_at(prev, a, b, q, i as f64 / n as f64));
                }
            }
        }
        prev = seg.end();
    }
}

/// even-odd: p inside the path (every sub-path closed)
fn inside(path: &Path, p: [f64; 2], eps: f64) -> bool {
    let mut odd = false;
    let mut pts = Vec::new();
    for s in &path.subs {
        if s.segs.is_empty() {
            continue;
        }
        pts.clear();
        flatten(s, eps, &mut pts);
        odd ^= crossings(&pts, p);
    }
    odd
}

/// whether a ray from p to the right crosses the closed polygon an odd number of times
fn crossings(pts: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut odd = false;
    let n = pts.len();
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                odd = !odd;
            }
        }
    }
    odd
}

fn in_poly(q: &[[f64; 2]], p: [f64; 2]) -> bool {
    crossings(q, p)
}

/// p within r of the path's lines
fn near(path: &Path, p: [f64; 2], r: f64, eps: f64) -> bool {
    let r2 = r * r;
    let mut pts = Vec::new();
    for s in &path.subs {
        pts.clear();
        flatten(s, eps, &mut pts);
        if s.closed && pts.len() > 2 && pts.first() != pts.last() {
            pts.push(s.start);
        }
        if pts.len() == 1 && dist2_seg(p, pts[0], pts[0]) <= r2 {
            return true;
        }
        if pts.windows(2).any(|w| dist2_seg(p, w[0], w[1]) <= r2) {
            return true;
        }
    }
    false
}

/// the squared distance of p from the segment a b
fn dist2_seg(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (x, y) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    x * x + y * y
}

/// each run of a text as the quad on the page its box covers (from the descent to the
/// ascent, grown by tol)
fn text_quads(t: &TextItem, tol: f64) -> Vec<[[f64; 2]; 4]> {
    t.runs
        .iter()
        .map(|r| {
            let b = [r.at[0] - tol, r.at[1] - r.size * 0.25 - tol, r.at[0] + r.width + tol, r.at[1] + r.size * 0.9 + tol];
            [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]].map(|q| t.xform.apply(q))
        })
        .collect()
}

/// p on a picture: inside its rectangle and inside the shape's box that cuts it
fn image_at(img: &ImageItem, p: [f64; 2]) -> bool {
    let Some(inv) = img.xform.inverse() else { return false };
    let q = inv.apply(p);
    (0.0..=1.0).contains(&q[0]) && (0.0..=1.0).contains(&q[1]) && in_poly(&img.clip, p)
}

fn inside_rect(b: [f64; 4], r: [f64; 4]) -> bool {
    b[0] >= r[0] && b[1] >= r[1] && b[2] <= r[2] && b[3] <= r[3]
}

/// whether an item touches the rectangle r: a line within half its width, a fill, text or
/// picture over any part of it
fn touches(it: &Item, r: [f64; 4]) -> bool {
    let b = it.bbox;
    if b[0] > r[2] || b[2] < r[0] || b[1] > r[3] || b[3] < r[1] {
        return false;
    }
    let centre = [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5];
    let eps = 1e-4;
    match &it.kind {
        ItemKind::Stroke { path, stroke } => {
            let g = stroke.width.max(0.0) * 0.5;
            let rr = [r[0] - g, r[1] - g, r[2] + g, r[3] + g];
            let mut pts = Vec::new();
            path.subs.iter().any(|s| {
                pts.clear();
                flatten(s, eps, &mut pts);
                if s.closed && pts.len() > 2 && pts.first() != pts.last() {
                    pts.push(s.start);
                }
                (pts.len() == 1 && inside_rect([pts[0][0], pts[0][1], pts[0][0], pts[0][1]], rr)) || pts.windows(2).any(|w| seg_meets_rect(w[0], w[1], rr))
            })
        }
        ItemKind::Fill { path, .. } => {
            let mut pts = Vec::new();
            let edge = path.subs.iter().any(|s| {
                pts.clear();
                flatten(s, eps, &mut pts);
                let n = pts.len();
                (0..n).any(|i| seg_meets_rect(pts[i], pts[(i + 1) % n], r))
            });
            edge || inside(path, centre, eps)
        }
        ItemKind::Text(t) => text_quads(t, 0.0).iter().any(|q| poly_meets_rect(q, r)),
        ItemKind::Image(img) => poly_meets_rect(&img.clip, r),
    }
}

fn poly_meets_rect(q: &[[f64; 2]], r: [f64; 4]) -> bool {
    let n = q.len();
    (0..n).any(|i| seg_meets_rect(q[i], q[(i + 1) % n], r)) || in_poly(q, [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5])
}

/// whether the segment a b meets the rectangle r (Liang-Barsky)
fn seg_meets_rect(a: [f64; 2], b: [f64; 2], r: [f64; 4]) -> bool {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a[0] - r[0]), (dx, r[2] - a[0]), (-dy, a[1] - r[1]), (dy, r[3] - a[1])] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "hit_tests.rs"]
pub(crate) mod tests;
