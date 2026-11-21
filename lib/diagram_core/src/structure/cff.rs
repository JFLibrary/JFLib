// structure::cff -- the cross-functional flowchart (ST-05): lanes and phases in a pool
//
//   find(doc, s) -> Option<Cff>        the flowchart a shape belongs to (the pool itself, one of
//                                      its lists, a lane, a phase, a shape in a lane)
//   pools(doc, owner)                  every flowchart of a page
//   layout(tx, eng, &cff)              lanes stacked, phases side by side across every lane,
//                                      the lists and the pool around them -> what moved
//   lanes(doc, &cff) / phases(doc, &cff)
//   create(tx, eng, &src, page, top_left, horizontal, lanes, phases, lang) -> Cff
//                                      (JD13) a new flowchart from the masters of stencil `src`
//                                      (JFDiagrammer's "cff"): pool, both lists, the lanes and
//                                      phases, laid out from `top_left`; create_with takes a
//                                      lane or phase already on the page along (a dropped one)
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
use crate::document::PageRef;
use crate::instance;

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

/// the hidden masters of a flowchart (NameU, as in JFDiagrammer's stencil "cff")
pub const M_POOL: &str = "CFF Container";
pub const M_LANE_LIST: &str = "Swimlane List";
pub const M_PHASE_LIST: &str = "Phase List";

/// the master of a lane: horizontal (lanes one above the other) or vertical
pub fn lane_master(horizontal: bool) -> &'static str {
    if horizontal { "Swimlane" } else { "Swimlane (vertical)" }
}

/// the master of a phase (a separator)
pub fn phase_master(horizontal: bool) -> &'static str {
    if horizontal { "Separator" } else { "Separator (vertical)" }
}

// an instance of master `name_u` of `src` with its pin at `at` (computed, its subshapes too)
fn put(tx: &mut Transaction, eng: &mut Engine, src: &Document, name_u: &str, page: PageRef, at: [f64; 2], lang: usize) -> Result<ShapeRef, EditError> {
    let m = src
        .masters()
        .find(|&m| src.master(m).and_then(|x| x.header.name_u).is_some_and(|n| src.str(n).eq_ignore_ascii_case(name_u)))
        .ok_or(EditError::NoMaster)?;
    let m = instance::import_master(tx, src, m)?;
    let s = instance::place(tx, page, m, at[0], at[1], lang)?;
    let mut sheets = Vec::new();
    let mut stack = alloc::vec![s];
    while let Some(c) = stack.pop() {
        sheets.push(SheetRef::Shape(c));
        if let Some(sh) = tx.doc().shape(c) {
            stack.extend(sh.children());
        }
    }
    eng.recalc(tx, &sheets)?;
    Ok(s)
}

/// a new flowchart from the masters of stencil `src` on `page`: the pool, the lane list, the
/// phase list, `lanes` lanes (at least one) and `phases` phases, laid out with the pool's top
/// left corner at `top_left`; names and texts in language `lang` (instance::LANGS).  Connectors
/// near it are not rerouted (the caller's glue::update)
pub fn create(tx: &mut Transaction, eng: &mut Engine, src: &Document, page: PageRef, top_left: [f64; 2], horizontal: bool, lanes: usize, phases: usize, lang: usize) -> Result<Cff, EditError> {
    create_with(tx, eng, src, page, top_left, horizontal, None, None, lanes, phases, lang)
}

/// create with a lane or a phase already on the page (one dropped outside any flowchart) as the
/// first lane or phase; the others are new
#[allow(clippy::too_many_arguments)]
pub fn create_with(
    tx: &mut Transaction,
    eng: &mut Engine,
    src: &Document,
    page: PageRef,
    top_left: [f64; 2],
    horizontal: bool,
    lane: Option<ShapeRef>,
    phase: Option<ShapeRef>,
    lanes: usize,
    phases: usize,
    lang: usize,
) -> Result<Cff, EditError> {
    let at = top_left;
    let pool = put(tx, eng, src, M_POOL, page, at, lang)?;
    let ll = put(tx, eng, src, M_LANE_LIST, page, at, lang)?;
    let pl = put(tx, eng, src, M_PHASE_LIST, page, at, lang)?;
    // the directions of the lists: lanes across the flow, phases along it
    for (l, d) in [(ll, if horizontal { 2.0 } else { 0.0 }), (pl, if horizontal { 0.0 } else { 2.0 })] {
        tx.set_user(SheetRef::Shape(l), "msvSDListDirection", Value::Num(d), None)?;
    }
    let mut ls: Vec<ShapeRef> = lane.into_iter().collect();
    while ls.len() < lanes.max(1) {
        ls.push(put(tx, eng, src, lane_master(horizontal), page, at, lang)?);
    }
    let mut ps: Vec<ShapeRef> = phase.into_iter().collect();
    while ps.len() < phases {
        ps.push(put(tx, eng, src, phase_master(horizontal), page, at, lang)?);
    }
    // the pool's top left corner at `at`
    let pb = shape_box(tx.doc(), pool);
    ops::move_shapes(tx, eng, &[pool], [at[0] - pb[0], at[1] - pb[3]])?;
    // the frame behind everything: pool, lists, phases, lanes
    for &s in ls.iter().chain(ps.iter()).chain([pl, ll, pool].iter()) {
        ops::reorder(tx, &[s], ops::Z::Back)?;
    }
    add_members(tx, eng, pool, &[ll, pl], None)?;
    add_members(tx, eng, ll, &ls, None)?;
    if !ps.is_empty() {
        add_members(tx, eng, pl, &ps, None)?;
    }
    let f = of_pool(tx.doc(), pool).ok_or(EditError::Invalid)?;
    layout(tx, eng, &f)?;
    Ok(f)
}
