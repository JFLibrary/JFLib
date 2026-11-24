// route -- the ways of connectors on the page: right angles around obstacles, straight, curved
// (CN-04, CN-05)
//
//   Port { at, dir, own }              an end of a route: the point; the direction the route
//                                      leaves it in (outward, any length; None: any); the box
//                                      of the shape the end is glued to (None: a free end)
//   Opts { clearance, bend }           the distance kept to obstacles; what a bend costs, as a
//                                      length (fewer bends against a longer way)
//   orthogonal(a, b, &obstacles, &opts) -> points   a right-angle way from a.at to b.at
//   best(&a_ports, &b_ports, &obstacles, &opts) -> (i, j, points)   the cheapest way between
//                                      any two of the ports (a dynamic end offers one or two
//                                      per side)
//   best_through(&a, &b, &obstacles, &pass, &opts)   (JD21, SS-19) the same; pass[k] says
//                                      which way the way may cross obstacle k (PASS_X across it
//                                      horizontally: ShapePermeableX; PASS_Y vertically)
//   hits_pass(p, q, &b, pass)          hits_box, but a horizontal (vertical) segment through a
//                                      box it may cross does not count
//   straight(a, b) -> points           the line between the two ends
//   cost(&points, &opts) / bends(&points) / length(&points)
//   spline_knots(n) -> (degree, knots) a clamped uniform B-spline over n control points: the
//                                      curved style takes the right-angle way's corners as the
//                                      control polygon of one NURBS (glue.rs writes the row)
//   hits_box(p, q, &b)                 whether a segment runs through a box's inside
//
// Everything is in page coordinates (drawing units, y up); boxes are [x0, y0, x1, y1].
//
// How orthogonal finds its way (deterministic, no state between calls):
// 1. Obstacles grow by the clearance.  A box that holds one of the two ends (a container, the
//    group around a glued subshape) is no obstacle; the boxes of the glued shapes (Port::own)
//    are, unless one holds the other end inside it.
//    Glued shapes closer than twice the clearance keep half their distance instead.
// 2. Each end leaves its own box: with a direction along it (snapped to the nearer axis) up to
//    the grown box, without one in each of the four axis directions; a free end starts where
//    it is, in any direction.  A direction that points into the own shape counts as none.
//    best() offers several ports for an end: all their exits start (or end) one search.
// 3. A sparse grid: the x and y lines of the grown boxes, of the exit points and the lines
//    halfway between the two ends' exits (so a Z bends in the middle, as Visio's do).  Grid
//    points inside a grown box and grid edges through one are blocked; the lines of a grown box
//    itself stay open, so a way keeps exactly the clearance.  (JD21) A box the way may cross
//    horizontally (vertically) blocks only its vertical (horizontal) edges and no grid point, so
//    a way runs straight through it but cannot turn inside it.
// 4. A* over (grid point, direction of travel) with the cost length + bend * bends; the
//    estimate is the Manhattan distance plus the fewest bends any way to a goal needs (from
//    the direction of travel and the one the goal wants), so the way found is the cheapest.
//    Arriving at the far end against its direction costs a bend, turning back two.
// 5. Only the obstacles near the two ends take part at first (a window around them); when the
//    way crosses one left out, the window grows to hold the way and the search runs again (the
//    third round takes all).  A way that cannot be found (an end inside an obstacle) comes from
//    the glued boxes alone, else from a plain L.
// 6. Points on one line merge: the result has the two ends and the corners only; the middle
//    piece of a Z moves halfway between the pieces around it where that stays clear.

use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// an end of a route
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Port {
    /// where the route ends (page coordinates)
    pub at: [f64; 2],
    /// the direction it leaves the end in (outward; None: any direction)
    pub dir: Option<[f64; 2]>,
    /// the box of the shape the end is glued to: the way does not cross it (None: free)
    pub own: Option<[f64; 4]>,
}

impl Port {
    /// a free end: any direction, no shape
    pub fn free(at: [f64; 2]) -> Port {
        Port { at, dir: None, own: None }
    }
}

/// what a route keeps to
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Opts {
    /// the distance to obstacles and the length of the first and last piece (drawing units)
    pub clearance: f64,
    /// the cost of a bend, as a length
    pub bend: f64,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts { clearance: 0.125, bend: 0.5 }
    }
}

// the four directions of travel: +x, -x, +y, -y (d ^ 1 is the opposite)
const DIRS: [[f64; 2]; 4] = [[1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0]];
const NONE: u32 = u32::MAX;

fn opposite(d: usize) -> usize {
    d ^ 1
}

/// the axis direction nearest to a vector (None: no direction)
fn axis(v: [f64; 2]) -> Option<usize> {
    let (ax, ay) = (v[0].abs(), v[1].abs());
    if !(ax.is_finite() && ay.is_finite()) || ax.max(ay) <= 1e-12 {
        None
    } else if ax >= ay {
        Some(if v[0] > 0.0 { 0 } else { 1 })
    } else {
        Some(if v[1] > 0.0 { 2 } else { 3 })
    }
}

fn norm(b: &[f64; 4]) -> [f64; 4] {
    [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])]
}

fn grow(b: &[f64; 4], c: f64) -> [f64; 4] {
    [b[0] - c, b[1] - c, b[2] + c, b[3] + c]
}

// whether the box holds the point (its edge included, with a little tolerance)
fn holds(b: &[f64; 4], p: [f64; 2]) -> bool {
    let t = 1e-9 * (1.0 + b[2].abs().max(b[3].abs()));
    p[0] >= b[0] - t && p[0] <= b[2] + t && p[1] >= b[1] - t && p[1] <= b[3] + t
}

// whether the point lies inside the box, off its edge
fn inside(b: &[f64; 4], p: [f64; 2]) -> bool {
    let t = 1e-9 * (1.0 + b[2].abs().max(b[3].abs()));
    p[0] > b[0] + t && p[0] < b[2] - t && p[1] > b[1] + t && p[1] < b[3] - t
}

fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// whether the segment p-q runs through the inside of box b (touching its edge is not)
pub fn hits_box(p: [f64; 2], q: [f64; 2], b: &[f64; 4]) -> bool {
    let b = norm(b);
    let tol = 1e-9 * (1.0 + b[2].abs().max(b[3].abs()));
    let (x0, y0, x1, y1) = (b[0] + tol, b[1] + tol, b[2] - tol, b[3] - tol);
    if x0 >= x1 || y0 >= y1 {
        return false;
    }
    // Liang-Barsky clipping against the slightly shrunk box: an overlap of positive length
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (pp, qq) in [(-dx, p[0] - x0), (dx, x1 - p[0]), (-dy, p[1] - y0), (dy, y1 - p[1])] {
        if pp == 0.0 {
            if qq <= 0.0 {
                return false;
            }
        } else {
            let r = qq / pp;
            if pp < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    if t0 < t1 {
        // a point strictly between the entry and exit lies inside
        let t = (t0 + t1) * 0.5;
        let m = [p[0] + dx * t, p[1] + dy * t];
        m[0] > x0 && m[0] < x1 && m[1] > y0 && m[1] < y1
    } else {
        false
    }
}

/// (JD21, SS-19) a way may cross the obstacle horizontally (ShapePermeableX)
pub const PASS_X: u8 = 1;
/// (JD21, SS-19) a way may cross the obstacle vertically (ShapePermeableY)
pub const PASS_Y: u8 = 2;

/// (JD21) whether the segment p-q runs through the inside of box b where it may not: a
/// horizontal segment through a box with PASS_X, a vertical one through a box with PASS_Y do not
pub fn hits_pass(p: [f64; 2], q: [f64; 2], b: &[f64; 4], pass: u8) -> bool {
    if pass != 0 {
        let t = 1e-9 * (1.0 + p[0].abs().max(p[1].abs()) + q[0].abs().max(q[1].abs()));
        let (flat, upright) = ((p[1] - q[1]).abs() <= t, (p[0] - q[0]).abs() <= t);
        if (flat && pass & PASS_X != 0) || (upright && pass & PASS_Y != 0) {
            return false;
        }
    }
    hits_box(p, q, b)
}

/// the length of a polyline
pub fn length(points: &[[f64; 2]]) -> f64 {
    points
        .windows(2)
        .map(|w| {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            crate::geometry::sqrt(dx * dx + dy * dy)
        })
        .sum()
}

/// the number of bends of a polyline (corners whose two pieces are not on one line)
pub fn bends(points: &[[f64; 2]]) -> usize {
    points.windows(3).filter(|w| !collinear(w[0], w[1], w[2])).count()
}

/// length + bends * opts.bend
pub fn cost(points: &[[f64; 2]], opts: &Opts) -> f64 {
    length(points) + bends(points) as f64 * opts.bend
}

fn collinear(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
    let scale = (b[0] - a[0]).abs() + (b[1] - a[1]).abs() + (c[0] - b[0]).abs() + (c[1] - b[1]).abs();
    let dot = (b[0] - a[0]) * (c[0] - b[0]) + (b[1] - a[1]) * (c[1] - b[1]);
    cross.abs() <= 1e-9 * (1.0 + scale * scale) && dot >= 0.0
}

// three points on one line, in any order
fn on_line(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
    let scale = (b[0] - a[0]).abs() + (b[1] - a[1]).abs() + (c[0] - b[0]).abs() + (c[1] - b[1]).abs();
    cross.abs() <= 1e-9 * (1.0 + scale * scale)
}

fn same(a: [f64; 2], b: [f64; 2]) -> bool {
    let t = 1e-9 * (1.0 + a[0].abs().max(a[1].abs()));
    (a[0] - b[0]).abs() <= t && (a[1] - b[1]).abs() <= t
}

// drop repeated points and the middle ones of three on a line (also where the way turns back
// on its line: no spikes)
pub(crate) fn simplify(points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().is_some_and(|&l| same(l, p)) {
            continue;
        }
        while out.len() >= 2 && on_line(out[out.len() - 2], out[out.len() - 1], p) {
            out.pop();
        }
        if out.last().is_some_and(|&l| same(l, p)) {
            continue;
        }
        out.push(p);
    }
    if out.len() == 1 {
        out.push(out[0]);
    }
    out
}

/// the straight route: the two ends
pub fn straight(a: &Port, b: &Port) -> Vec<[f64; 2]> {
    alloc::vec![a.at, b.at]
}

/// a clamped uniform B-spline over n control points (n >= 2): its degree (at most 3) and its
/// n + degree + 1 knots from 0 to 1
pub fn spline_knots(n: usize) -> (usize, Vec<f64>) {
    let n = n.max(2);
    let degree = (n - 1).min(3);
    let inner = n - degree - 1;
    let mut knots = Vec::with_capacity(n + degree + 1);
    knots.extend(core::iter::repeat(0.0).take(degree + 1));
    for i in 1..=inner {
        knots.push(i as f64 / (inner + 1) as f64);
    }
    knots.extend(core::iter::repeat(1.0).take(degree + 1));
    (degree, knots)
}

// ---------------------------------------------------------------- right angles

// where a way leaves an end: the grid point, the direction it leaves in (None: any), the
// length from the end to it, the port it belongs to
#[derive(Copy, Clone, Debug)]
struct Exit {
    at: [f64; 2],
    dir: Option<usize>,
    cost: f64,
    port: usize,
}

// from p along axis direction d to the edge of box b (p inside or on it)
fn to_edge(p: [f64; 2], d: usize, b: &[f64; 4]) -> [f64; 2] {
    match d {
        0 => [b[2], p[1]],
        1 => [b[0], p[1]],
        2 => [p[0], b[3]],
        _ => [p[0], b[1]],
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).abs() + (a[1] - b[1]).abs()
}

// the exits of port number `port`; `own` is its box grown by the clearance (None: none here)
fn exits(p: &Port, port: usize, own: Option<[f64; 4]>, c: f64, out: &mut Vec<Exit>) {
    let d = p.dir.and_then(axis);
    let own = own.filter(|g| holds(g, p.at));
    let edge = |d: usize, g: &[f64; 4]| {
        let e = to_edge(p.at, d, g);
        Exit { at: e, dir: Some(d), cost: dist(p.at, e), port }
    };
    match (d, own) {
        (Some(d), Some(g)) => {
            // a direction into the shape (towards its middle) is no direction
            let mid = [(g[0] + g[2]) * 0.5, (g[1] + g[3]) * 0.5];
            let v = DIRS[d];
            let size = (g[2] - g[0]).abs() + (g[3] - g[1]).abs();
            if (p.at[0] - mid[0]) * v[0] + (p.at[1] - mid[1]) * v[1] >= -1e-9 * (1.0 + size) {
                out.push(edge(d, &g));
            } else {
                out.extend((0..4).map(|d| edge(d, &g)));
            }
        }
        (Some(d), None) => {
            let v = DIRS[d];
            out.push(Exit { at: [p.at[0] + v[0] * c, p.at[1] + v[1] * c], dir: Some(d), cost: c, port });
        }
        (None, Some(g)) => out.extend((0..4).map(|d| edge(d, &g))),
        (None, None) => out.push(Exit { at: p.at, dir: None, cost: 0.0, port }),
    }
}

// a heap entry: the smallest estimate first, then the lowest state (deterministic)
#[derive(Copy, Clone)]
struct Item {
    f: f64,
    g: f64,
    st: u32,
}

impl PartialEq for Item {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    // the heap pops the greatest: the smallest estimate, then the longest way so far (it is
    // nearer the goal), then the lowest state
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f).then_with(|| self.g.total_cmp(&o.g)).then_with(|| o.st.cmp(&self.st))
    }
}

// the fewest bends from p, travelling in direction d, to e, arriving there in direction `a`
// (None: any) -- a lower bound of the bends of any way (obstacles only add to them); the
// search may also turn at e itself, into the last piece (one bend, turning back two)
pub(crate) fn min_bends(d: usize, p: [f64; 2], e: [f64; 2], a: Option<usize>) -> u32 {
    match a {
        None => arrive(d, p, e, None),
        Some(a) => (0..4)
            .map(|h| {
                let turn = if h == a { 0 } else if h == opposite(a) { 2 } else { 1 };
                arrive(d, p, e, Some(h)) + turn
            })
            .min()
            .unwrap_or(0),
    }
}

// the fewest direction changes from p (travelling in d, turning at p allowed) to arrive at e
// travelling in a (None: any)
fn arrive(d: usize, p: [f64; 2], e: [f64; 2], a: Option<usize>) -> u32 {
    let t = 1e-9 * (1.0 + p[0].abs().max(p[1].abs()) + e[0].abs().max(e[1].abs()));
    let v = DIRS[d];
    let w = [e[0] - p[0], e[1] - p[1]];
    let fwd = w[0] * v[0] + w[1] * v[1];
    let side = (w[0] * v[1] - w[1] * v[0]).abs() > t;
    let (ahead, behind) = (fwd > t, fwd < -t);
    match a {
        None if !side => {
            if behind { 3 } else { 0 }
        }
        None => {
            if behind { 2 } else { 1 }
        }
        Some(a) if a == d => {
            if !side && !behind {
                0
            } else if side && ahead {
                2
            } else {
                4
            }
        }
        Some(a) if a == opposite(d) => {
            if side { 2 } else { 4 }
        }
        Some(a) => {
            let u = DIRS[a];
            if w[0] * u[0] + w[1] * u[1] > t && !behind { 1 } else { 3 }
        }
    }
}

// sorted coordinates without near repeats
fn coords(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(f64::total_cmp);
    let mut out: Vec<f64> = Vec::with_capacity(v.len());
    for x in v {
        if out.last().is_none_or(|&l| x - l > 1e-9 * (1.0 + x.abs())) {
            out.push(x);
        }
    }
    out
}

// the index of the coordinate nearest to x
fn index_of(v: &[f64], x: f64) -> usize {
    let i = v.partition_point(|&c| c < x);
    if i == 0 {
        0
    } else if i >= v.len() {
        v.len() - 1
    } else if (v[i] - x).abs() < (x - v[i - 1]).abs() {
        i
    } else {
        i - 1
    }
}

/// the most grid points one search uses before the obstacles are thinned to the window
const MAX_NODES: usize = 1 << 20;

// one search over the grid of these grown obstacles, from any start to any goal -> the ports
// of the cheapest way and the way (the start port's point first, the goal port's last)
type Found = (usize, usize, Vec<[f64; 2]>);

// `pass[k]`: how the way may cross grown[k] (missing: not at all)
#[allow(clippy::too_many_arguments)]
fn search(a: &[Port], b: &[Port], starts: &[Exit], goals: &[Exit], grown: &[[f64; 4]], pass: &[u8], opts: &Opts, informed: bool) -> Option<Found> {
    let mut xs = Vec::with_capacity(grown.len() * 2 + 8);
    let mut ys = Vec::with_capacity(grown.len() * 2 + 8);
    for g in grown {
        xs.extend([g[0], g[2]]);
        ys.extend([g[1], g[3]]);
    }
    for e in starts.iter().chain(goals) {
        xs.push(e.at[0]);
        ys.push(e.at[1]);
    }
    for s in starts {
        for g in goals {
            xs.push((s.at[0] + g.at[0]) * 0.5);
            ys.push((s.at[1] + g.at[1]) * 0.5);
        }
    }
    let xs = coords(xs);
    let ys = coords(ys);
    let (w, h) = (xs.len(), ys.len());
    if w == 0 || h == 0 || w.saturating_mul(h) > MAX_NODES {
        return None;
    }
    let n = w * h;
    let mut inside = alloc::vec![false; n];
    let mut hblk = alloc::vec![false; n];
    let mut vblk = alloc::vec![false; n];
    for (k, g) in grown.iter().enumerate() {
        let (li, hi) = (index_of(&xs, g[0]), index_of(&xs, g[2]));
        let (lj, hj) = (index_of(&ys, g[1]), index_of(&ys, g[3]));
        // (JD21) a box the way may cross: no point blocked, only the edges across the way it may run
        let p = pass.get(k).copied().unwrap_or(0);
        for j in lj..=hj {
            let row = j * w;
            if j > lj && j < hj {
                if p & PASS_X == 0 {
                    for i in li..hi {
                        hblk[row + i] = true;
                    }
                }
                if p == 0 {
                    for i in li + 1..hi {
                        inside[row + i] = true;
                    }
                }
            }
            if j < hj && p & PASS_Y == 0 {
                for i in li + 1..hi {
                    vblk[row + i] = true;
                }
            }
        }
    }
    let node = |p: [f64; 2]| index_of(&ys, p[1]) * w + index_of(&xs, p[0]);
    let goal_nodes: Vec<(usize, Exit)> = goals.iter().map(|e| (node(e.at), *e)).collect();
    for &(m, _) in &goal_nodes {
        inside[m] = false;
    }
    let start_nodes: Vec<(usize, Exit)> = starts.iter().map(|e| (node(e.at), *e)).collect();
    for &(m, _) in &start_nodes {
        inside[m] = false;
    }
    let bend = opts.bend.max(0.0);
    // the estimate: the Manhattan distance and the fewest bends to the cheapest goal
    let est = |m: usize, d: usize| {
        if !informed {
            return 0.0;
        }
        let p = [xs[m % w], ys[m / w]];
        goal_nodes
            .iter()
            .map(|(_, e)| dist(p, e.at) + e.cost + bend * min_bends(d, p, e.at, e.dir.map(opposite)) as f64)
            .fold(f64::INFINITY, f64::min)
    };
    let extra = |e: &Exit, d: usize| match e.dir {
        None => e.cost,
        Some(gd) if d == opposite(gd) => e.cost,
        Some(gd) if d == gd => e.cost + 2.0 * bend,
        Some(_) => e.cost + bend,
    };
    let mut gs = alloc::vec![f64::INFINITY; n * 4];
    let mut parent = alloc::vec![NONE; n * 4];
    let mut heap = BinaryHeap::new();
    for &(m, e) in &start_nodes {
        let dirs: &[usize] = match e.dir {
            Some(d) => &[d][..],
            None => &[0, 1, 2, 3][..],
        };
        for &d in dirs {
            let st = m * 4 + d;
            if e.cost < gs[st] {
                gs[st] = e.cost;
                parent[st] = NONE;
                heap.push(Item { f: e.cost + est(m, d), g: e.cost, st: st as u32 });
            }
        }
    }
    let mut best = (f64::INFINITY, NONE, 0usize);
    while let Some(it) = heap.pop() {
        if it.f >= best.0 {
            break;
        }
        let st = it.st as usize;
        if it.g > gs[st] {
            continue;
        }
        let (m, d) = (st / 4, st % 4);
        for (gm, e) in &goal_nodes {
            if *gm == m {
                let total = it.g + extra(e, d);
                if total < best.0 {
                    best = (total, it.st, e.port);
                }
            }
        }
        let (i, j) = (m % w, m / w);
        for nd in 0..4 {
            if nd == opposite(d) {
                continue;
            }
            let next = match nd {
                0 if i + 1 < w && !hblk[m] => m + 1,
                1 if i > 0 && !hblk[m - 1] => m - 1,
                2 if j + 1 < h && !vblk[m] => m + w,
                3 if j > 0 && !vblk[m - w] => m - w,
                _ => continue,
            };
            if inside[next] {
                continue;
            }
            let len = (xs[next % w] - xs[i]).abs() + (ys[next / w] - ys[j]).abs();
            let ng = it.g + len + if nd != d { bend } else { 0.0 };
            let ns = next * 4 + nd;
            if ng < gs[ns] {
                gs[ns] = ng;
                parent[ns] = it.st;
                heap.push(Item { f: ng + est(next, nd), g: ng, st: ns as u32 });
            }
        }
    }
    if best.1 == NONE {
        return None;
    }
    let mut chain = Vec::new();
    let (mut st, mut root) = (best.1, best.1);
    while st != NONE {
        let m = st as usize / 4;
        chain.push([xs[m % w], ys[m / w]]);
        root = st;
        st = parent[st as usize];
    }
    chain.reverse();
    // the start the way began at: its grid point and direction (any direction for a free one)
    let (rm, rd) = (root as usize / 4, root as usize % 4);
    let pa = start_nodes.iter().find(|(m, e)| *m == rm && e.dir.is_none_or(|d| d == rd)).map_or(0, |(_, e)| e.port);
    let pb = best.2;
    let mut pts = Vec::with_capacity(chain.len() + 2);
    pts.push(a[pa].at);
    pts.extend(chain);
    pts.push(b[pb].at);
    Some((pa, pb, simplify(pts)))
}

// an L (or a straight line) from exit to exit, the first piece along the start's direction
fn plain(a: &[Port], b: &[Port], s: &Exit, g: &Exit) -> Found {
    let horizontal = s.dir.is_none_or(|d| d < 2);
    let corner = if horizontal { [g.at[0], s.at[1]] } else { [s.at[0], g.at[1]] };
    (s.port, g.port, simplify(alloc::vec![a[s.port].at, s.at, corner, g.at, b[g.port].at]))
}

// a Z bends in the middle, as Visio's do: the middle piece of three whose outer pieces run the
// same way moves halfway between their far ends, when the pieces stay clear of the grown
// obstacles and of the glued shapes (the middle piece also of their clearance)
fn centre(mut pts: Vec<[f64; 2]>, others: &[[f64; 4]], pass: &[u8], own: &[[f64; 4]], c: f64) -> Vec<[f64; 2]> {
    let hit = |s: [f64; 2], e: [f64; 2], k: usize, o: &[f64; 4]| hits_pass(s, e, o, pass.get(k).copied().unwrap_or(0));
    let n = pts.len();
    if n < 4 {
        return pts;
    }
    let own_grown: Vec<[f64; 4]> = own.iter().map(|o| grow(o, c)).collect();
    for i in 1..n - 2 {
        let (p0, p1, p2, p3) = (pts[i - 1], pts[i], pts[i + 1], pts[i + 2]);
        let t = 1e-9 * (1.0 + p1[0].abs().max(p1[1].abs()));
        // k: the axis the middle piece slides along
        let k = if (p1[0] - p2[0]).abs() <= t && (p1[1] - p2[1]).abs() > t {
            0
        } else if (p1[1] - p2[1]).abs() <= t && (p1[0] - p2[0]).abs() > t {
            1
        } else {
            continue;
        };
        let (a0, a1, a3) = (p0[k], p1[k], p3[k]);
        let across = (p0[1 - k] - p1[1 - k]).abs() <= t && (p2[1 - k] - p3[1 - k]).abs() <= t;
        if !across || !((a0 < a1 && a1 < a3) || (a0 > a1 && a1 > a3)) {
            continue;
        }
        let mid = (a0 + a3) * 0.5;
        if (mid - a1).abs() <= t {
            continue;
        }
        let (mut q1, mut q2) = (p1, p2);
        q1[k] = mid;
        q2[k] = mid;
        // the outer pieces may run inside the own box of the end they start at (an end docked
        // on an outline inside its box)
        let free = |s: [f64; 2], e: [f64; 2], end: [f64; 2]| {
            others.iter().enumerate().all(|(k, o)| !hit(s, e, k, o)) && own.iter().filter(|o| !holds(o, end)).all(|o| !hits_box(s, e, o))
        };
        let clear = free(p0, q1, p0)
            && free(q2, p3, p3)
            && others.iter().enumerate().all(|(k, o)| !hit(q1, q2, k, o))
            && own_grown.iter().all(|o| !hits_box(q1, q2, o));
        if clear {
            pts[i] = q1;
            pts[i + 1] = q2;
        }
    }
    pts
}

/// a right-angle route from a.at to b.at around the obstacles (boxes in page coordinates):
/// the two ends and the corners
pub fn orthogonal(a: Port, b: Port, obstacles: &[[f64; 4]], opts: &Opts) -> Vec<[f64; 2]> {
    route_with(&[a], &[b], obstacles, &[], opts, true).2
}

/// the cheapest right-angle route from any of the ports `a` to any of the ports `b` (one
/// search: a dynamic end offers a port per side) -> the indices of the two ports and the
/// route; both lists must not be empty
pub fn best(a: &[Port], b: &[Port], obstacles: &[[f64; 4]], opts: &Opts) -> (usize, usize, Vec<[f64; 2]>) {
    route_with(a, b, obstacles, &[], opts, true)
}

/// (JD21, SS-19) best() among obstacles the way may cross: `pass[k]` (PASS_X, PASS_Y; missing:
/// 0) for obstacles[k]
pub fn best_through(a: &[Port], b: &[Port], obstacles: &[[f64; 4]], pass: &[u8], opts: &Opts) -> (usize, usize, Vec<[f64; 2]>) {
    route_with(a, b, obstacles, pass, opts, true)
}

/// (tests) the same search without the estimate (Dijkstra): the cost must not differ
#[cfg(test)]
pub(crate) fn orthogonal_exact(a: Port, b: Port, obstacles: &[[f64; 4]], opts: &Opts) -> Vec<[f64; 2]> {
    route_with(&[a], &[b], obstacles, &[], opts, false).2
}

fn route_with(a: &[Port], b: &[Port], obstacles: &[[f64; 4]], pass: &[u8], opts: &Opts, informed: bool) -> Found {
    if a.is_empty() || b.is_empty() {
        return (0, 0, Vec::new());
    }
    let c = if opts.clearance.is_finite() { opts.clearance.max(0.0) } else { 0.0 };
    // the own boxes: obstacles unless one holds an end of the other side inside (not on its
    // edge: both ends on one shape)
    let own_of = |ps: &[Port], others: &[Port]| -> Vec<Option<[f64; 4]>> {
        ps.iter().map(|p| p.own.map(|o| norm(&o)).filter(|o| !others.iter().any(|q| inside(o, q.at)))).collect()
    };
    let (own_a, own_b) = (own_of(a, b), own_of(b, a));
    let mut own: Vec<[f64; 4]> = Vec::new();
    for o in own_a.iter().chain(&own_b).flatten() {
        if !own.contains(o) {
            own.push(*o);
        }
    }
    // glued shapes closer than twice the clearance keep half their distance (Visio connects
    // close shapes straight), overlapping ones none
    let mut co = c;
    for (i, x) in own.iter().enumerate() {
        for y in &own[i + 1..] {
            let gap = (y[0] - x[2]).max(x[0] - y[2]).max(y[1] - x[3]).max(x[1] - y[3]);
            co = co.min((gap * 0.5).max(0.0));
        }
    }
    let mut starts = Vec::new();
    for (i, p) in a.iter().enumerate() {
        exits(p, i, own_a[i].map(|o| grow(&o, co)), c, &mut starts);
    }
    let mut goals = Vec::new();
    for (i, p) in b.iter().enumerate() {
        exits(p, i, own_b[i].map(|o| grow(&o, co)), c, &mut goals);
    }
    // boxes that hold an end are not obstacles (containers, the group of a glued subshape)
    let ends: Vec<[f64; 2]> = a.iter().chain(b).map(|p| p.at).collect();
    let list: Vec<([f64; 4], u8)> = obstacles
        .iter()
        .enumerate()
        .map(|(k, o)| (norm(o), pass.get(k).copied().unwrap_or(0)))
        .filter(|(o, _)| o.iter().all(|v| v.is_finite()) && !ends.iter().any(|e| holds(o, *e)))
        .collect();
    let fixed: Vec<[f64; 4]> = own.iter().map(|o| grow(o, co)).collect();
    // the window: the ends with their boxes, grown by a margin
    let mut win = [ends[0][0], ends[0][1], ends[0][0], ends[0][1]];
    for e in &ends {
        win = union(win, [e[0], e[1], e[0], e[1]]);
    }
    for o in fixed.iter() {
        win = union(win, *o);
    }
    let span = (win[2] - win[0]).max(win[3] - win[1]);
    let margin = (6.0 * c).max(0.3 * span);
    win = grow(&win, margin);
    for round in 0..3 {
        let all = round == 2;
        let mut grown = fixed.clone();
        let mut gpass = alloc::vec![0u8; fixed.len()];
        let mut left_out: Vec<([f64; 4], u8)> = Vec::new();
        for &(o, p) in &list {
            if all || overlaps(&o, &win) {
                grown.push(grow(&o, c));
                gpass.push(p);
            } else {
                left_out.push((o, p));
            }
        }
        let Some((pa, pb, path)) = search(a, b, &starts, &goals, &grown, &gpass, opts, informed) else { break };
        let crossed = path.windows(2).any(|s| left_out.iter().any(|(o, p)| hits_pass(s[0], s[1], o, *p)));
        if !crossed {
            return (pa, pb, centre(path, &grown[fixed.len()..], &gpass[fixed.len()..], &own, co));
        }
        let mut bb = win;
        for p in &path {
            bb = union(bb, [p[0], p[1], p[0], p[1]]);
        }
        win = grow(&bb, margin);
    }
    // no way among the obstacles: around the glued shapes alone, else a plain L
    if let Some((pa, pb, p)) = search(a, b, &starts, &goals, &fixed, &[], opts, informed) {
        return (pa, pb, centre(p, &[], &[], &own, co));
    }
    plain(a, b, &starts[0], &goals[0])
}
