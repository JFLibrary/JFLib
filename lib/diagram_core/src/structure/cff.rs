// structure::cff -- the cross-functional flowchart (ST-05): lanes and phases in a pool
//
//   find(doc, s) -> Option<Cff>        the flowchart a shape belongs to (the pool itself, one of
//                                      its lists, a lane, a phase, a shape in a lane)
//   pools(doc, owner)                  every flowchart of a page
//   layout(tx, eng, &cff)              lanes stacked, phases side by side across every lane,
//                                      the lists and the pool around them -> what moved
//   lanes(doc, &cff) / phases(doc, &cff)
//
// Visio's structure, kept so that Visio recognises it (research in jd10-strukturen.md):
//
//   CFF Container   Container, categories "CFF Container", takes only "CFF List"; its heading
//                   is the title bar
//    ├ Swimlane List  List of the lanes (items: categories "Swimlane"), direction 2 (top to
//    │                bottom) for horizontal lanes, 0 for vertical ones
//    └ Phase List     List of the phases (items: "Phase"), the other direction
//   Swimlane        Container with its heading on the left (horizontal) or on top (vertical)
//   Separator       Container with its heading on top (horizontal) or on the left; it spans
//                   every lane, a shape in it is its member as well as its lane's
//
// Horizontal (lanes one above the other):
//
//   +--------------------------------------------+  title bar (the pool's heading, Ht)
//   |       | phase 1 heading  | phase 2 heading |  phase row (the phases' headings, Hp)
//   | lane1 |                  |                 |
//   +-------+------------------+-----------------+
//   | lane2 |                  |                 |
//   +--------------------------------------------+
//    Wl (the lanes' headings)
//
// The lanes keep their heights and share one width: the lane headings plus the phases (the last
// phase takes what is left), or the widest lane without phases.  The phases keep their widths
// and span the phase row and every lane.  Vertical flowcharts are the same turned: lanes side by
// side, the phases one above the other.  Lanes and phases carry their shapes; a shape in both
// moves by the larger of the two moves on each axis (once, not twice).

use alloc::vec::Vec;

use super::*;

/// a cross-functional flowchart
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Cff {
    pub pool: ShapeRef,
    /// the list of the lanes and of the phases
    pub lane_list: Option<ShapeRef>,
    pub phase_list: Option<ShapeRef>,
    /// lanes one above the other (else side by side)
    pub horizontal: bool,
}

/// the categories of the parts
pub const CAT_POOL: &str = "CFF Container";
pub const CAT_LIST: &str = "CFF List";
pub const CAT_LANE: &str = "Swimlane";
pub const CAT_PHASE: &str = "Phase";

/// whether a shape is a flowchart's pool
pub fn is_pool(doc: &Document, s: ShapeRef) -> bool {
    is_container(doc, s) && has_category(doc, s, CAT_POOL)
}

/// the flowchart of a pool
pub fn of_pool(doc: &Document, pool: ShapeRef) -> Option<Cff> {
    if !is_pool(doc, pool) {
        return None;
    }
    let mut lane_list = None;
    let mut phase_list = None;
    for m in members(doc, pool) {
        if kind(doc, m) != Some(Kind::List) {
            continue;
        }
        let req = split_list(&user_text(doc, m, "msvSDListRequiredCategories").unwrap_or_default());
        if req.iter().any(|c| c.eq_ignore_ascii_case(CAT_LANE)) {
            lane_list = Some(m);
        } else if req.iter().any(|c| c.eq_ignore_ascii_case(CAT_PHASE)) {
            phase_list = Some(m);
        }
    }
    let horizontal = lane_list.and_then(|l| list(doc, l)).is_none_or(|i| !i.direction.horizontal());
    Some(Cff { pool, lane_list, phase_list, horizontal })
}

/// the flowchart a shape belongs to: the pool, a list of it, a lane, a phase, a shape inside
pub fn find(doc: &Document, s: ShapeRef) -> Option<Cff> {
    let mut seen = BTreeSet::new();
    let mut stack = alloc::vec![s];
    while let Some(c) = stack.pop() {
        if !seen.insert(c) || seen.len() > 64 {
            continue;
        }
        if let Some(f) = of_pool(doc, c) {
            return Some(f);
        }
        stack.extend(containers_of(doc, c));
    }
    None
}

/// every flowchart on a page or master
pub fn pools(doc: &Document, owner: Owner) -> Vec<Cff> {
    doc.top_shapes(owner).filter_map(|s| of_pool(doc, s)).collect()
}

/// the lanes in order
pub fn lanes(doc: &Document, f: &Cff) -> Vec<ShapeRef> {
    f.lane_list.map(|l| items(doc, l)).unwrap_or_default()
}

/// the phases in order
pub fn phases(doc: &Document, f: &Cff) -> Vec<ShapeRef> {
    f.phase_list.map(|l| items(doc, l)).unwrap_or_default()
}

// a container's heading depth (0 without one)
fn head(doc: &Document, s: ShapeRef) -> f64 {
    container(doc, s).map_or(0.0, |c| if c.heading == Edge::None { 0.0 } else { c.heading_size })
}

/// lay the flowchart out from its pool's top left corner -> the shapes that moved
pub fn layout(tx: &mut Transaction, eng: &mut Engine, f: &Cff) -> Result<Vec<ShapeRef>, EditError> {
    layout_with(tx, eng, f, None)
}

/// layout with the lanes' length given (a pool or lane resized by hand), else the longest
/// lane's or what the phases need
pub fn layout_with(tx: &mut Transaction, eng: &mut Engine, f: &Cff, want: Option<f64>) -> Result<Vec<ShapeRef>, EditError> {
    let (pool_box, ht, ls, ps) = {
        let doc = tx.doc();
        if !upright(doc, f.pool) {
            return Ok(Vec::new());
        }
        (shape_box(doc, f.pool), head(doc, f.pool), lanes(doc, f), phases(doc, f))
    };
    let mut moved: Vec<ShapeRef> = Vec::new();
    let (x0, y1) = (pool_box[0], pool_box[3]);
    let doc = tx.doc();
    let lane_boxes: Vec<[f64; 4]> = ls.iter().map(|&s| shape_box(doc, s)).collect();
    let phase_boxes: Vec<[f64; 4]> = ps.iter().map(|&s| shape_box(doc, s)).collect();
    // the heading of the lanes (on the side across their direction) and of the phases
    let lane_head = ls.first().map_or(0.0, |&s| head(doc, s));
    let phase_head = ps.iter().map(|&s| head(doc, s)).fold(0.0f64, f64::max);
    // what each part gets: (shape, from, to)
    let mut plan: Vec<(ShapeRef, [f64; 4], [f64; 4])> = Vec::new();
    let mut lists: Vec<(ShapeRef, [f64; 4])> = Vec::new();
    let pool_to;
    if f.horizontal {
        // lane length: the lane headings and the phases, or the widest lane
        let widest = lane_boxes.iter().map(|b| b[2] - b[0]).fold(0.0f64, f64::max);
        let phases_w: f64 = phase_boxes.iter().map(|b| b[2] - b[0]).sum();
        let last_w = phase_boxes.last().map_or(0.0, |b| b[2] - b[0]);
        let w = match want {
            Some(v) if ps.is_empty() => v.max(lane_head + EPS),
            Some(v) => v.max(lane_head + phases_w - last_w + EPS),
            None if ps.is_empty() => widest.max(EPS),
            None => widest.max(lane_head + phases_w),
        };
        let top = y1 - ht;
        let lanes_top = top - if ps.is_empty() { 0.0 } else { phase_head };
        let mut y = lanes_top;
        for (k, &s) in ls.iter().enumerate() {
            let h = lane_boxes[k][3] - lane_boxes[k][1];
            plan.push((s, lane_boxes[k], [x0, y - h, x0 + w, y]));
            y -= h;
        }
        let bottom = if ls.is_empty() { lanes_top - 1.0 } else { y };
        let mut x = x0 + lane_head;
        for (k, &s) in ps.iter().enumerate() {
            let pw = if k + 1 == ps.len() { (x0 + w - x).max(EPS) } else { phase_boxes[k][2] - phase_boxes[k][0] };
            plan.push((s, phase_boxes[k], [x, bottom, x + pw, top]));
            x += pw;
        }
        if let Some(l) = f.lane_list {
            lists.push((l, [x0, bottom, x0 + w, lanes_top]));
        }
        if let Some(l) = f.phase_list {
            lists.push((l, [x0 + lane_head, bottom, x0 + w, top]));
        }
        pool_to = [x0, bottom, x0 + w, y1];
    } else {
        // lanes side by side, phases one above the other
        let tallest = lane_boxes.iter().map(|b| b[3] - b[1]).fold(0.0f64, f64::max);
        let phases_h: f64 = phase_boxes.iter().map(|b| b[3] - b[1]).sum();
        let top = y1 - ht;
        let last_h = phase_boxes.last().map_or(0.0, |b| b[3] - b[1]);
        let h = match want {
            Some(v) if ps.is_empty() => v.max(lane_head + EPS),
            Some(v) => v.max(lane_head + phases_h - last_h + EPS),
            None if ps.is_empty() => tallest.max(EPS),
            None => tallest.max(lane_head + phases_h),
        };
        let lanes_left = x0 + if ps.is_empty() { 0.0 } else { phase_head };
        let mut x = lanes_left;
        for (k, &s) in ls.iter().enumerate() {
            let lw = lane_boxes[k][2] - lane_boxes[k][0];
            plan.push((s, lane_boxes[k], [x, top - h, x + lw, top]));
            x += lw;
        }
        let right = if ls.is_empty() { lanes_left + 1.0 } else { x };
        let mut y = top - lane_head;
        for (k, &s) in ps.iter().enumerate() {
            let ph = if k + 1 == ps.len() { (y - (top - h)).max(EPS) } else { phase_boxes[k][3] - phase_boxes[k][1] };
            plan.push((s, phase_boxes[k], [x0, y - ph, right, y]));
            y -= ph;
        }
        if let Some(l) = f.lane_list {
            lists.push((l, [lanes_left, top - h, right, top]));
        }
        if let Some(l) = f.phase_list {
            lists.push((l, [x0, top - h, right, top - lane_head]));
        }
        pool_to = [x0, top - h, right, y1];
    }
    // the shapes in the lanes and phases: how far each moves (its top left corner's move; the
    // larger of a lane's and a phase's on each axis)
    let mut carry: alloc::collections::BTreeMap<ShapeRef, [f64; 2]> = alloc::collections::BTreeMap::new();
    for &(s, from, to) in &plan {
        let d = [to[0] - from[0], to[3] - from[3]];
        if d == [0.0, 0.0] {
            continue;
        }
        let doc = tx.doc();
        for m in members(doc, s).into_iter().flat_map(|m| carried(doc, &[m])) {
            if m == s || plan.iter().any(|p| p.0 == m) {
                continue;
            }
            let e = carry.entry(m).or_insert([0.0, 0.0]);
            for k in 0..2 {
                if d[k].abs() > e[k].abs() {
                    e[k] = d[k];
                }
            }
        }
    }
    for (m, d) in carry {
        ops::move_shapes(tx, eng, &[m], d)?;
        moved.push(m);
    }
    for (s, from, to) in plan {
        if (0..4).all(|i| (to[i] - from[i]).abs() < EPS) {
            continue;
        }
        ops::set_frame(tx, eng, s, &Frame::from_box(to))?;
        moved.push(s);
    }
    for (l, to) in lists {
        let from = shape_box(tx.doc(), l);
        if (0..4).any(|i| (to[i] - from[i]).abs() > EPS) {
            ops::set_frame(tx, eng, l, &Frame::from_box(to))?;
            moved.push(l);
        }
    }
    if (0..4).any(|i| (pool_to[i] - pool_box[i]).abs() > EPS) {
        ops::set_frame(tx, eng, f.pool, &Frame::from_box(pool_to))?;
        moved.push(f.pool);
    }
    moved.sort();
    moved.dedup();
    Ok(moved)
}
