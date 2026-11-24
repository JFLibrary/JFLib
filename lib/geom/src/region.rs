// region -- Boolean operations on plane regions (docs/GEOM.md, "Ebene Regionen"): sets given as contours
// (outer contours and holes, nested to any depth, several operands, filled by a rule: non-zero or
// even-odd), union, intersection, difference, exclusive or, and the decomposition of their overlay into
// its faces with the operands that cover each one (Visio's "Fragment", the polygons of a map made from
// overlapping areas).  Open polylines go in as cutters: they split the faces of a decomposition.
//
//   boolean(operands, op, rule) -> Vec<Contour>      the result's boundary: outer contours
//                                                     counter-clockwise, holes clockwise
//   decompose(operands, rule) -> Vec<Piece>           every covered face with its operands
//   union / intersection / difference / xor           shorthands; area(contours)
//   Operand { contours, open }, Contour { points, marks }, Mark { operand, contour, edge, reversed }
//
// Every output edge carries a Mark: the input edge it lies on (operand, contour, the edge from point i to
// point i + 1, and whether it runs against it).  A caller that flattened curves into polylines
// (diagram_core: the arcs of a shape) finds its curves again by these marks.  Consecutive output edges
// with the same mark are one edge (the vertex between them is gone).
//
// How it decides (snap rounding to an integer grid, exact integer arithmetic):
// * The input is put on a grid: the box around all points is 2^30 steps across its longer side (the
//   coordinate minus the box's corner times a power of two, rounded to the nearest step), so every point
//   becomes a pair of integers below 2^30.  Points closer than a step (10^-9 of the extent) are one.
// * Snap rounding (Hobby; Guibas and Marimont): every endpoint and every point where two segments cross
//   makes a "hot pixel", the unit square around the grid point it rounds to; every segment is bent
//   through the centre of each hot pixel it passes through, in order.  The result has no crossings: its
//   segments meet at their ends or lie on each other.  Done again on its own output until nothing changes
//   (iterated snap rounding, Halperin and Packer), which also splits a segment where a vertex lies on it.
//   All tests are signs of determinants and comparisons of fractions in i128: exact, the same answer on
//   every target, and no case is special (touching in a point or along an edge, identical operands, a
//   hole on its outer contour, an operand overlapping itself).  Only crossing points move, by at most
//   half a step; input points come back with their own f64 bits.
// * The overlay: equal segments become one edge that keeps, per operand, how much the winding number
//   grows from its right to its left (+1 for an edge of a counter-clockwise contour).  Edges that change
//   nothing (an operand's edge drawn there and back) are left out.  Around each vertex the edges are
//   sorted by angle (exactly: half-plane, then cross product) and the faces traced as cycles; a cycle with
//   positive area is a face, the one with area <= 0 of each connected part is its outside.  Which face
//   holds a part is found by a ray down from just left of its lowest-leftmost vertex: the first edge it
//   meets, compared as exact fractions.  Winding numbers spread from the face around a part across its
//   edges.
// * A face is inside an operand when its winding number is not 0 (non-zero) or odd (even-odd).  The result
//   of an operation is its boundary: the edges with a different answer left and right, traced so that each
//   contour turns as sharply as it can where two pieces touch (two squares corner to corner stay two
//   contours).
//
// Deterministic like the rest of geom: integers and + - * on f64, sorting with unique keys, no unordered
// containers, no threads.  Limits against runaway input: MAX_EDGES input edges, MAX_HOT crossings.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use crate::Error;

/// a grid point
type P = [i64; 2];

/// The input's box is 2^GRID_BITS steps across its longer side (a step is 10^-9 of the extent).
pub const GRID_BITS: i32 = 30;

/// the most input edges (all operands together)
pub const MAX_EDGES: usize = 1 << 20;

/// the most points where edges cross
pub const MAX_HOT: usize = 1 << 22;

// rounds of snap rounding before giving up (two in practice: one that changes, one that confirms)
const MAX_ROUNDS: usize = 24;

/// how contours fill: where the winding number is not 0, or where it is odd
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// the Boolean operations
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BoolOp {
    /// in any operand
    Union,
    /// in every operand
    Intersection,
    /// in the first and in none of the others
    Difference,
    /// in an odd number of operands
    Xor,
}

/// An operand: closed contours (the last point joins the first; outer contours and holes in any order and
/// direction, the fill rule decides) or, `open`, polylines that only cut (decompose).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Operand {
    pub contours: Vec<Vec<[f64; 2]>>,
    pub open: bool,
}

impl Operand {
    /// an area from closed contours
    pub fn closed(contours: Vec<Vec<[f64; 2]>>) -> Operand {
        Operand { contours, open: false }
    }

    /// cutters from open polylines
    pub fn open(polylines: Vec<Vec<[f64; 2]>>) -> Operand {
        Operand { contours: polylines, open: true }
    }
}

/// Where an output edge comes from: the input edge from point `edge` to point `edge + 1` (the closing edge
/// of a closed contour has the index of its last point) of contour `contour` of operand `operand`;
/// `reversed`: the output runs against it.  Where input edges lie on each other, the smallest of them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mark {
    pub operand: u32,
    pub contour: u32,
    pub edge: u32,
    pub reversed: bool,
}

/// An output contour: closed, `marks[i]` the mark of the edge from `points[i]` to the next point.
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    pub points: Vec<[f64; 2]>,
    pub marks: Vec<Mark>,
}

impl Contour {
    /// the signed area (shoelace about the first point, so that far coordinates do not cancel): positive
    /// for an outer contour, negative for a hole
    pub fn area(&self) -> f64 {
        let Some(&o) = self.points.first() else { return 0.0 };
        let mut a = 0.0;
        for w in self.points.windows(2) {
            let (p, q) = ([w[0][0] - o[0], w[0][1] - o[1]], [w[1][0] - o[0], w[1][1] - o[1]]);
            a += p[0] * q[1] - q[0] * p[1];
        }
        a * 0.5
    }

    /// whether it runs clockwise (a hole)
    pub fn is_hole(&self) -> bool {
        self.area() < 0.0
    }
}

/// A face of a decomposition: the operands that cover it (indices, rising) and its boundary (an outer
/// contour, counter-clockwise, and its holes, clockwise).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub cover: Vec<u32>,
    pub contours: Vec<Contour>,
}

/// the sum of the signed areas of contours (the area of a result)
pub fn area(contours: &[Contour]) -> f64 {
    contours.iter().fold(0.0, |s, c| s + c.area())
}

/// union of the closed operands
pub fn union(operands: &[Operand], rule: FillRule) -> Result<Vec<Contour>, Error> {
    boolean(operands, BoolOp::Union, rule)
}

/// intersection of the closed operands
pub fn intersection(operands: &[Operand], rule: FillRule) -> Result<Vec<Contour>, Error> {
    boolean(operands, BoolOp::Intersection, rule)
}

/// the first closed operand without the others
pub fn difference(operands: &[Operand], rule: FillRule) -> Result<Vec<Contour>, Error> {
    boolean(operands, BoolOp::Difference, rule)
}

/// what an odd number of closed operands cover
pub fn xor(operands: &[Operand], rule: FillRule) -> Result<Vec<Contour>, Error> {
    boolean(operands, BoolOp::Xor, rule)
}

/// The boundary of a Boolean operation on the closed operands (open ones do not take part): outer contours
/// counter-clockwise, holes clockwise, none crossing another (they may touch in a point).  Empty when
/// nothing is left.
pub fn boolean(operands: &[Operand], op: BoolOp, rule: FillRule) -> Result<Vec<Contour>, Error> {
    let total = operands.iter().filter(|o| !o.open).count();
    let Some(ov) = Overlay::build(operands, false)? else { return Ok(Vec::new()) };
    let w = ov.windings();
    let m = ov.slots;
    // the first closed operand's slot (Difference): every closed operand has a slot, in order
    let inside = |c: usize| -> bool {
        let ws = &w[c * m..c * m + m];
        let ins = |k: usize| fill(ws[k], rule);
        match op {
            BoolOp::Union => (0..m).any(ins),
            BoolOp::Intersection => m > 0 && m == total && (0..m).all(ins),
            BoolOp::Difference => m > 0 && ins(0) && !(1..m).any(ins),
            BoolOp::Xor => (0..m).filter(|&k| ins(k)).count() % 2 == 1,
        }
    };
    let cin: Vec<bool> = (0..ov.cycles).map(|c| inside(ov.face_of(c))).collect();
    // the output half-edges: the result on their left only
    let take: Vec<bool> = (0..ov.half()).map(|h| cin[ov.cycle[h] as usize] && !cin[ov.cycle[h ^ 1] as usize]).collect();
    Ok(ov.trace(&take))
}

/// The faces of the overlay that at least one closed operand covers, each with the operands covering it
/// (by `rule`) and its boundary.  Neighbouring faces with the same cover are one piece unless a cutter (an
/// open operand) runs between them.  Pieces in a fixed order (the order their edges were found).
pub fn decompose(operands: &[Operand], rule: FillRule) -> Result<Vec<Piece>, Error> {
    let Some(ov) = Overlay::build(operands, true)? else { return Ok(Vec::new()) };
    let w = ov.windings();
    let m = ov.slots;
    let covers: Vec<Vec<u32>> = (0..ov.cycles)
        .map(|c| {
            let f = ov.face_of(c);
            (0..m).filter(|&k| fill(w[f * m + k], rule)).map(|k| ov.slot_operand[k]).collect()
        })
        .collect();
    // the face of a cycle: itself when it has area, the face around its part for an outer cycle
    let face: Vec<Option<u32>> = (0..ov.cycles)
        .map(|c| {
            let f = ov.face_of(c);
            (ov.area[f] > 0).then_some(f as u32)
        })
        .collect();
    // faces join across edges that are no cutter and keep the cover
    let mut uf: Vec<u32> = (0..ov.cycles as u32).collect();
    for e in 0..ov.edges() {
        if ov.cutter[e] {
            continue;
        }
        let (a, b) = (ov.cycle[2 * e] as usize, ov.cycle[2 * e + 1] as usize);
        if let (Some(fa), Some(fb)) = (face[a], face[b]) {
            if covers[a] == covers[b] {
                union_find(&mut uf, fa, fb);
            }
        }
    }
    let pieces: Vec<Option<u32>> = (0..ov.cycles)
        .map(|c| match face[c] {
            Some(f) if !covers[c].is_empty() => Some(find(&mut uf, f)),
            _ => None,
        })
        .collect();
    // each piece's boundary: half-edges with it on the left and something else on the right
    let mut order: Vec<u32> = Vec::new();
    let mut seen: BTreeMap<u32, ()> = BTreeMap::new();
    for h in 0..ov.half() {
        if let Some(p) = pieces[ov.cycle[h] as usize] {
            if pieces[ov.cycle[h ^ 1] as usize] != Some(p) && seen.insert(p, ()).is_none() {
                order.push(p);
            }
        }
    }
    let mut out = Vec::with_capacity(order.len());
    for &p in &order {
        let take: Vec<bool> = (0..ov.half())
            .map(|h| pieces[ov.cycle[h] as usize] == Some(p) && pieces[ov.cycle[h ^ 1] as usize] != Some(p))
            .collect();
        let rep = (0..ov.cycles).find(|&c| pieces[c] == Some(p)).unwrap_or(0);
        out.push(Piece { cover: covers[rep].clone(), contours: ov.trace(&take) });
    }
    Ok(out)
}

fn fill(w: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w & 1 != 0,
    }
}

fn find(uf: &mut [u32], mut x: u32) -> u32 {
    while uf[x as usize] != x {
        let p = uf[x as usize];
        uf[x as usize] = uf[p as usize];
        x = p;
    }
    x
}

// the smaller root wins (deterministic)
fn union_find(uf: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(uf, a), find(uf, b));
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        uf[hi as usize] = lo;
    }
}

// ---------------------------------------------------------------- the grid

fn pow2(k: i32) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

// x * 2^k without overflow of the factor (|k| up to about 2000)
fn scale2(x: f64, k: i32) -> f64 {
    let h = k / 2;
    x * pow2(h) * pow2(k - h)
}

// the nearest integer, ties to even (|v| < 2^52)
fn round_even(v: f64) -> i64 {
    let f = crate::math::floor(v);
    let i = f as i64;
    let r = v - f;
    if r > 0.5 || (r == 0.5 && i & 1 == 1) { i + 1 } else { i }
}

struct Grid {
    o: [f64; 2],
    k: i32,
}

impl Grid {
    // the grid for points in the box lo..hi (None: no extent)
    fn new(lo: [f64; 2], hi: [f64; 2]) -> Option<Grid> {
        let ext = crate::math::max(hi[0] - lo[0], hi[1] - lo[1]);
        if !(ext > 1e-290 && ext.is_finite()) {
            return None;
        }
        // 2^exp <= ext < 2^(exp + 1), so ext * 2^k < 2^GRID_BITS
        let exp = ((ext.to_bits() >> 52) & 0x7ff) as i32 - 1023;
        Some(Grid { o: lo, k: GRID_BITS - 1 - exp })
    }

    fn to_int(&self, p: [f64; 2]) -> P {
        [round_even(scale2(p[0] - self.o[0], self.k)), round_even(scale2(p[1] - self.o[1], self.k))]
    }

    fn to_f64(&self, q: P) -> [f64; 2] {
        [self.o[0] + scale2(q[0] as f64, -self.k) + 0.0, self.o[1] + scale2(q[1] as f64, -self.k) + 0.0]
    }
}

// ---------------------------------------------------------------- exact predicates

#[inline]
fn cross(a: P, b: P, c: P) -> i128 {
    (b[0] - a[0]) as i128 * (c[1] - a[1]) as i128 - (b[1] - a[1]) as i128 * (c[0] - a[0]) as i128
}

#[inline]
fn sgn(x: i128) -> i32 {
    (x > 0) as i32 - (x < 0) as i32
}

// the grid point nearest to where segments a-b and c-d cross properly (None: they do not)
fn crossing(a: P, b: P, c: P, d: P) -> Option<P> {
    let (o1, o2) = (sgn(cross(a, b, c)), sgn(cross(a, b, d)));
    if o1 * o2 >= 0 {
        return None;
    }
    let (o3, o4) = (sgn(cross(c, d, a)), sgn(cross(c, d, b)));
    if o3 * o4 >= 0 {
        return None;
    }
    let r = [(b[0] - a[0]) as i128, (b[1] - a[1]) as i128];
    let s = [(d[0] - c[0]) as i128, (d[1] - c[1]) as i128];
    let mut den = r[0] * s[1] - r[1] * s[0];
    let mut tn = (c[0] - a[0]) as i128 * s[1] - (c[1] - a[1]) as i128 * s[0];
    if den < 0 {
        den = -den;
        tn = -tn;
    }
    // a + r * tn / den, rounded half up (the pixel of x is [h - 1/2, h + 1/2)): floor((2n + den) / 2den)
    let at = |a0: i64, r0: i128| -> i64 {
        let n = a0 as i128 * den + r0 * tn;
        (2 * n + den).div_euclid(2 * den) as i64
    };
    Some([at(a[0], r[0]), at(a[1], r[1])])
}

// a fraction n / d with d > 0, and whether the bound it is part of is open
#[derive(Copy, Clone, Debug)]
struct Bound {
    n: i128,
    d: i128,
    open: bool,
}

fn cmp_frac(a: &Bound, b: &Bound) -> core::cmp::Ordering {
    (a.n * b.d).cmp(&(b.n * a.d))
}

// Where the closed segment a-b enters the half-open pixel of h ([h - 1/2, h + 1/2) in x and y): the
// parameter as a fraction and whether the entry is open (None: it does not meet it).  In doubled
// coordinates relative to h the pixel is [-1, 1) x [-1, 1).
fn pixel_entry(a: P, b: P, h: P) -> Option<Bound> {
    use core::cmp::Ordering::*;
    let p = [2 * (a[0] - h[0]) as i128, 2 * (a[1] - h[1]) as i128];
    let q = [2 * (b[0] - h[0]) as i128, 2 * (b[1] - h[1]) as i128];
    let mut lo = Bound { n: 0, d: 1, open: false };
    let mut hi = Bound { n: 1, d: 1, open: false };
    for k in 0..2 {
        let d = q[k] - p[k];
        let (l, u) = if d == 0 {
            if !(-1 <= p[k] && p[k] < 1) {
                return None;
            }
            continue;
        } else if d > 0 {
            (Bound { n: -1 - p[k], d, open: false }, Bound { n: 1 - p[k], d, open: true })
        } else {
            (Bound { n: p[k] - 1, d: -d, open: true }, Bound { n: 1 + p[k], d: -d, open: false })
        };
        match cmp_frac(&l, &lo) {
            Greater => lo = l,
            Equal => lo.open |= l.open,
            Less => {}
        }
        match cmp_frac(&u, &hi) {
            Less => hi = u,
            Equal => hi.open |= u.open,
            Greater => {}
        }
    }
    match cmp_frac(&lo, &hi) {
        Less => Some(lo),
        Equal if !lo.open && !hi.open => Some(lo),
        _ => None,
    }
}

// ---------------------------------------------------------------- snap rounding

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Seg {
    a: P,
    b: P,
    // the input edge (index into Overlay::srcs)
    src: u32,
}

// the pairs of segments whose boxes overlap, by a sweep over x
fn box_pairs(segs: &[Seg], mut f: impl FnMut(usize, usize) -> Result<(), Error>) -> Result<(), Error> {
    let mut idx: Vec<u32> = (0..segs.len() as u32).collect();
    let minx = |s: &Seg| s.a[0].min(s.b[0]);
    idx.sort_by_key(|&i| (minx(&segs[i as usize]), i));
    for (n, &i) in idx.iter().enumerate() {
        let s = &segs[i as usize];
        let (x1, y0, y1) = (s.a[0].max(s.b[0]), s.a[1].min(s.b[1]), s.a[1].max(s.b[1]));
        for &j in &idx[n + 1..] {
            let t = &segs[j as usize];
            if minx(t) > x1 {
                break;
            }
            if t.a[1].max(t.b[1]) < y0 || t.a[1].min(t.b[1]) > y1 {
                continue;
            }
            f(i as usize, j as usize)?;
        }
    }
    Ok(())
}

// one round: the hot pixels (ends and crossings), every segment bent through those it passes
fn snap_round(segs: &[Seg]) -> Result<Vec<Seg>, Error> {
    let mut hot: Vec<P> = Vec::with_capacity(segs.len() * 2);
    for s in segs {
        hot.push(s.a);
        hot.push(s.b);
    }
    let mut crossings = 0usize;
    box_pairs(segs, |i, j| {
        if let Some(p) = crossing(segs[i].a, segs[i].b, segs[j].a, segs[j].b) {
            crossings += 1;
            if crossings > MAX_HOT {
                return Err(Error::TooBig(alloc::format!("Flächen: mehr als {} Schnittpunkte", MAX_HOT)));
            }
            hot.push(p);
        }
        Ok(())
    })?;
    hot.sort_unstable();
    hot.dedup();
    let mut out = Vec::with_capacity(segs.len());
    let mut hits: Vec<(Bound, P)> = Vec::new();
    for s in segs {
        let (x0, x1) = (s.a[0].min(s.b[0]), s.a[0].max(s.b[0]));
        let (y0, y1) = (s.a[1].min(s.b[1]), s.a[1].max(s.b[1]));
        hits.clear();
        let start = hot.partition_point(|h| h[0] < x0);
        for &h in &hot[start..] {
            if h[0] > x1 {
                break;
            }
            if h[1] < y0 || h[1] > y1 {
                continue;
            }
            if let Some(t) = pixel_entry(s.a, s.b, h) {
                hits.push((t, h));
            }
        }
        hits.sort_by(|x, y| cmp_frac(&x.0, &y.0).then((x.0.open as u8).cmp(&(y.0.open as u8))).then(x.1.cmp(&y.1)));
        let mut prev = s.a;
        for &(_, h) in &hits {
            if h != prev {
                out.push(Seg { a: prev, b: h, src: s.src });
                prev = h;
            }
        }
        if prev != s.b {
            out.push(Seg { a: prev, b: s.b, src: s.src });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- the overlay

// an input edge
#[derive(Copy, Clone, Debug)]
struct Src {
    operand: u32,
    contour: u32,
    edge: u32,
}

struct Overlay {
    verts: Vec<P>,
    // back to f64: the input points by their grid point, else the grid's
    back: BTreeMap<P, [f64; 2]>,
    grid: Grid,
    srcs: Vec<Src>,
    // per edge: its ends (vertex indices, the point of eu below the point of ev), the winding deltas
    // (slot, delta) from dstart[e], the representative source and whether it runs eu -> ev, whether it is
    // only a cutter
    eu: Vec<u32>,
    ev: Vec<u32>,
    dstart: Vec<u32>,
    dlist: Vec<(u32, i32)>,
    rep: Vec<(u32, bool)>,
    cutter: Vec<bool>,
    // the closed operands, numbered as winding slots
    slots: usize,
    slot_operand: Vec<u32>,
    // half-edges: 2e is eu -> ev, 2e + 1 is ev -> eu; around each vertex counter-clockwise
    around: Vec<u32>,
    astart: Vec<u32>,
    apos: Vec<u32>,
    next: Vec<u32>,
    cycle: Vec<u32>,
    cycles: usize,
    // per cycle: twice its area, its first half-edge, its part; per part its outer cycle and container
    area: Vec<i128>,
    cstart: Vec<u32>,
    comp_of_cycle: Vec<u32>,
    outer_of: Vec<u32>,
    container: Vec<Option<u32>>,
    comp_order: Vec<u32>,
}

impl Overlay {
    fn half(&self) -> usize {
        self.eu.len() * 2
    }

    fn edges(&self) -> usize {
        self.eu.len()
    }

    fn origin(&self, h: usize) -> u32 {
        if h & 1 == 0 { self.eu[h >> 1] } else { self.ev[h >> 1] }
    }

    fn dest(&self, h: usize) -> u32 {
        self.origin(h ^ 1)
    }

    // the cycle whose winding numbers hold for cycle c: an outer cycle takes those of its container
    fn face_of(&self, c: usize) -> usize {
        let mut c = c;
        for _ in 0..=self.cycles {
            if self.area[c] > 0 {
                return c;
            }
            match self.container[self.comp_of_cycle[c] as usize] {
                Some(f) => c = f as usize,
                None => return c,
            }
        }
        c
    }

    // `cutters`: a decomposition (open operands split faces); else they are left out
    fn build(operands: &[Operand], cutters: bool) -> Result<Option<Overlay>, Error> {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        let mut n_edges = 0usize;
        for op in operands.iter().filter(|o| cutters || !o.open) {
            for c in &op.contours {
                for p in c {
                    if !(p[0].is_finite() && p[1].is_finite()) {
                        return Err(Error::Profile("Flächen: eine Koordinate ist nicht endlich".into()));
                    }
                    for k in 0..2 {
                        lo[k] = crate::math::min(lo[k], p[k]);
                        hi[k] = crate::math::max(hi[k], p[k]);
                    }
                }
                n_edges += c.len();
            }
        }
        if n_edges > MAX_EDGES {
            return Err(Error::TooBig(alloc::format!("Flächen: mehr als {} Kanten", MAX_EDGES)));
        }
        if n_edges == 0 {
            return Ok(None);
        }
        let Some(grid) = Grid::new(lo, hi) else { return Ok(None) };
        let mut back: BTreeMap<P, [f64; 2]> = BTreeMap::new();
        let mut srcs: Vec<Src> = Vec::new();
        let mut segs: Vec<Seg> = Vec::new();
        let mut slot_of: Vec<Option<u32>> = vec![None; operands.len()];
        let mut slot_operand: Vec<u32> = Vec::new();
        for (oi, op) in operands.iter().enumerate() {
            if !op.open {
                slot_of[oi] = Some(slot_operand.len() as u32);
                slot_operand.push(oi as u32);
            } else if !cutters {
                continue;
            }
            for (ci, c) in op.contours.iter().enumerate() {
                let n = c.len();
                let q: Vec<P> = c.iter().map(|&p| grid.to_int(p)).collect();
                for (i, &p) in c.iter().enumerate() {
                    back.entry(q[i]).or_insert([p[0] + 0.0, p[1] + 0.0]);
                }
                let m = if op.open { n.saturating_sub(1) } else if n >= 2 { n } else { 0 };
                for i in 0..m {
                    let (a, b) = (q[i], q[(i + 1) % n]);
                    if a != b {
                        segs.push(Seg { a, b, src: srcs.len() as u32 });
                    }
                    srcs.push(Src { operand: oi as u32, contour: ci as u32, edge: i as u32 });
                }
            }
        }
        // snap rounding until nothing changes
        let mut rounds = 0;
        loop {
            let next = snap_round(&segs)?;
            if next == segs {
                break;
            }
            segs = next;
            rounds += 1;
            if rounds > MAX_ROUNDS {
                return Err(Error::Invalid("Flächen: das Runden auf das Raster kommt nicht zur Ruhe".into()));
            }
        }
        // equal segments are one edge
        let mut frags: Vec<(P, P, u32, bool)> =
            segs.iter().map(|s| if s.a < s.b { (s.a, s.b, s.src, true) } else { (s.b, s.a, s.src, false) }).collect();
        frags.sort_unstable();
        let mut ends: Vec<(P, P)> = Vec::new();
        let mut dstart = vec![0u32];
        let mut dlist: Vec<(u32, i32)> = Vec::new();
        let mut rep: Vec<(u32, bool)> = Vec::new();
        let mut cutter: Vec<bool> = Vec::new();
        let mut deltas: BTreeMap<u32, i32> = BTreeMap::new();
        let mut i = 0;
        while i < frags.len() {
            let (u, v) = (frags[i].0, frags[i].1);
            let mut j = i;
            deltas.clear();
            let mut cut = false;
            // the smallest source (operand, contour, edge), and its direction
            let mut best: Option<((u32, u32, u32), u32, bool)> = None;
            while j < frags.len() && frags[j].0 == u && frags[j].1 == v {
                let (_, _, si, fwd) = frags[j];
                let s = srcs[si as usize];
                match slot_of[s.operand as usize] {
                    Some(slot) => *deltas.entry(slot).or_insert(0) += if fwd { 1 } else { -1 },
                    None => cut = true,
                }
                let key = (s.operand, s.contour, s.edge);
                if best.is_none_or(|b| key < b.0) {
                    best = Some((key, si, fwd));
                }
                j += 1;
            }
            let any = deltas.values().any(|&d| d != 0);
            if any || cut {
                ends.push((u, v));
                for (&k, &d) in &deltas {
                    if d != 0 {
                        dlist.push((k, d));
                    }
                }
                dstart.push(dlist.len() as u32);
                let (_, si, fwd) = best.unwrap_or(((0, 0, 0), frags[i].2, frags[i].3));
                rep.push((si, fwd));
                cutter.push(cut && !any);
            }
            i = j;
        }
        if ends.is_empty() {
            return Ok(None);
        }
        let mut verts: Vec<P> = ends.iter().flat_map(|&(u, v)| [u, v]).collect();
        verts.sort_unstable();
        verts.dedup();
        let vix = |p: P| verts.binary_search(&p).unwrap_or(0) as u32;
        let eu: Vec<u32> = ends.iter().map(|e| vix(e.0)).collect();
        let ev: Vec<u32> = ends.iter().map(|e| vix(e.1)).collect();
        let mut ov = Overlay {
            verts,
            back,
            grid,
            srcs,
            eu,
            ev,
            dstart,
            dlist,
            rep,
            cutter,
            slots: slot_operand.len(),
            slot_operand,
            around: Vec::new(),
            astart: Vec::new(),
            apos: Vec::new(),
            next: Vec::new(),
            cycle: Vec::new(),
            cycles: 0,
            area: Vec::new(),
            cstart: Vec::new(),
            comp_of_cycle: Vec::new(),
            outer_of: Vec::new(),
            container: Vec::new(),
            comp_order: Vec::new(),
        };
        ov.link();
        ov.faces();
        ov.nest();
        Ok(Some(ov))
    }

    // the half-edges around each vertex, counter-clockwise, and next: the half-edge after h on the
    // boundary of the face left of h (the first one clockwise from h's twin)
    fn link(&mut self) {
        let nv = self.verts.len();
        let nh = self.half();
        let mut count = vec![0u32; nv + 1];
        for h in 0..nh {
            count[self.origin(h) as usize + 1] += 1;
        }
        for v in 0..nv {
            count[v + 1] += count[v];
        }
        let mut at = count.clone();
        let mut around = vec![0u32; nh];
        for h in 0..nh {
            let o = self.origin(h) as usize;
            around[at[o] as usize] = h as u32;
            at[o] += 1;
        }
        let dir = |h: u32| -> P {
            let (a, b) = (self.verts[self.origin(h as usize) as usize], self.verts[self.dest(h as usize) as usize]);
            [b[0] - a[0], b[1] - a[1]]
        };
        // directions from angle 0 (inclusive) to 180 (exclusive) first
        let upper = |d: P| d[1] > 0 || (d[1] == 0 && d[0] > 0);
        for v in 0..nv {
            around[count[v] as usize..count[v + 1] as usize].sort_by(|&x, &y| {
                let (dx, dy) = (dir(x), dir(y));
                let (ux, uy) = (upper(dx), upper(dy));
                if ux != uy {
                    return uy.cmp(&ux);
                }
                let c = dx[0] as i128 * dy[1] as i128 - dx[1] as i128 * dy[0] as i128;
                0.cmp(&c).then(x.cmp(&y))
            });
        }
        let mut apos = vec![0u32; nh];
        for v in 0..nv {
            for k in count[v]..count[v + 1] {
                apos[around[k as usize] as usize] = k;
            }
        }
        let mut next = vec![0u32; nh];
        for (h, nx) in next.iter_mut().enumerate() {
            let t = h ^ 1;
            let v = self.origin(t) as usize;
            let (s, e) = (count[v], count[v + 1]);
            let k = apos[t];
            *nx = around[if k == s { e - 1 } else { k - 1 } as usize];
        }
        self.around = around;
        self.astart = count;
        self.apos = apos;
        self.next = next;
    }

    // the cycles and twice their areas
    fn faces(&mut self) {
        let nh = self.half();
        let mut cycle = vec![u32::MAX; nh];
        let mut area = Vec::new();
        let mut cstart = Vec::new();
        for h0 in 0..nh {
            if cycle[h0] != u32::MAX {
                continue;
            }
            let c = area.len() as u32;
            let mut a: i128 = 0;
            let mut h = h0;
            loop {
                cycle[h] = c;
                let (p, q) = (self.verts[self.origin(h) as usize], self.verts[self.dest(h) as usize]);
                a += p[0] as i128 * q[1] as i128 - q[0] as i128 * p[1] as i128;
                h = self.next[h] as usize;
                if h == h0 {
                    break;
                }
            }
            area.push(a);
            cstart.push(h0 as u32);
        }
        self.cycles = area.len();
        self.cycle = cycle;
        self.area = area;
        self.cstart = cstart;
    }

    // the connected parts, their outer cycles and the face that holds each
    fn nest(&mut self) {
        let nv = self.verts.len();
        let mut uf: Vec<u32> = (0..nv as u32).collect();
        for e in 0..self.edges() {
            union_find(&mut uf, self.eu[e], self.ev[e]);
        }
        // parts by root: the smallest vertex of a part, which is its lowest-leftmost one (verts are
        // sorted by x, then y)
        let mut comp_of_vert = vec![u32::MAX; nv];
        let mut roots: Vec<u32> = Vec::new();
        for v in 0..nv {
            let r = find(&mut uf, v as u32) as usize;
            if comp_of_vert[r] == u32::MAX {
                comp_of_vert[r] = roots.len() as u32;
                roots.push(r as u32);
            }
            comp_of_vert[v] = comp_of_vert[r];
        }
        let nc = roots.len();
        let comp_of_edge: Vec<u32> = (0..self.edges()).map(|e| comp_of_vert[self.eu[e] as usize]).collect();
        let comp_of_cycle: Vec<u32> = (0..self.cycles).map(|c| comp_of_edge[(self.cstart[c] >> 1) as usize]).collect();
        // the outer cycle of a part has area <= 0
        let mut outer_of = vec![u32::MAX; nc];
        for c in 0..self.cycles {
            let k = comp_of_cycle[c] as usize;
            if self.area[c] <= 0 && outer_of[k] == u32::MAX {
                outer_of[k] = c as u32;
            }
        }
        // the container: a ray down from just left of the root, the first edge of another part it meets
        let mut container: Vec<Option<u32>> = vec![None; nc];
        for k in 0..nc {
            let v = self.verts[roots[k] as usize];
            // y where the edge meets x = v.x, as n / d, and the edge
            let mut best: Option<(i128, i128, usize)> = None;
            for e in 0..self.edges() {
                if comp_of_edge[e] as usize == k {
                    continue;
                }
                let (a, b) = (self.verts[self.eu[e] as usize], self.verts[self.ev[e] as usize]);
                // a is left of b (or below it on a vertical edge): it must span x = v.x - eps
                if !(a[0] < v[0] && v[0] <= b[0]) {
                    continue;
                }
                let d = (b[0] - a[0]) as i128;
                let n = a[1] as i128 * d + (v[0] - a[0]) as i128 * (b[1] - a[1]) as i128;
                if n >= v[1] as i128 * d {
                    continue;
                }
                let better = match best {
                    None => true,
                    Some((bn, bd, be)) => match (n * bd).cmp(&(bn * d)) {
                        core::cmp::Ordering::Greater => true,
                        core::cmp::Ordering::Less => false,
                        core::cmp::Ordering::Equal => {
                            // they meet at x = v.x: just left of it the one with the smaller slope is higher
                            let (pa, pb) = (self.verts[self.eu[be] as usize], self.verts[self.ev[be] as usize]);
                            ((b[1] - a[1]) as i128) * ((pb[0] - pa[0]) as i128) < ((pb[1] - pa[1]) as i128) * d
                        }
                    },
                };
                if better {
                    best = Some((n, d, e));
                }
            }
            // the half-edge running right has v's side on its left
            container[k] = best.map(|(_, _, e)| self.cycle[2 * e]);
        }
        let mut order: Vec<u32> = (0..nc as u32).collect();
        order.sort_by_key(|&k| (self.verts[roots[k as usize] as usize], k));
        self.comp_of_cycle = comp_of_cycle;
        self.outer_of = outer_of;
        self.container = container;
        self.comp_order = order;
    }

    // the winding numbers per cycle and closed operand (cycle * slots + slot); a part's outer cycle gets
    // those of the face around it (parts in the order of their leftmost points: that face is known then)
    fn windings(&self) -> Vec<i32> {
        let m = self.slots;
        let mut w = vec![0i32; self.cycles * m];
        let mut done = vec![false; self.cycles];
        let mut queue: Vec<u32> = Vec::new();
        for &k in &self.comp_order {
            let oc = self.outer_of[k as usize];
            if oc == u32::MAX {
                continue;
            }
            if let Some(f) = self.container[k as usize] {
                let f = self.face_of(f as usize);
                for s in 0..m {
                    w[oc as usize * m + s] = w[f * m + s];
                }
            }
            done[oc as usize] = true;
            queue.clear();
            queue.push(oc);
            while let Some(c) = queue.pop() {
                let h0 = self.cstart[c as usize] as usize;
                let mut h = h0;
                loop {
                    let c2 = self.cycle[h ^ 1] as usize;
                    if !done[c2] {
                        done[c2] = true;
                        // the right of h is c2: w(right) = w(left) - delta(h)
                        let e = h >> 1;
                        let sign = if h & 1 == 0 { 1 } else { -1 };
                        for s in 0..m {
                            w[c2 * m + s] = w[c as usize * m + s];
                        }
                        for &(s, d) in &self.dlist[self.dstart[e] as usize..self.dstart[e + 1] as usize] {
                            w[c2 * m + s as usize] -= sign * d;
                        }
                        queue.push(c2 as u32);
                    }
                    h = self.next[h] as usize;
                    if h == h0 {
                        break;
                    }
                }
            }
        }
        w
    }

    // the contours of the half-edges in `take` (each with what it bounds on its left): at each vertex the
    // first taken half-edge clockwise from the one it came along
    fn trace(&self, take: &[bool]) -> Vec<Contour> {
        let nh = self.half();
        let mut used = vec![false; nh];
        let mut out = Vec::new();
        let mut hs: Vec<u32> = Vec::new();
        for h0 in 0..nh {
            if !take[h0] || used[h0] {
                continue;
            }
            hs.clear();
            let mut h = h0;
            loop {
                used[h] = true;
                hs.push(h as u32);
                let t = h ^ 1;
                let v = self.origin(t) as usize;
                let (s, e) = (self.astart[v], self.astart[v + 1]);
                let mut k = self.apos[t];
                let mut found = None;
                for _ in s..e {
                    k = if k == s { e - 1 } else { k - 1 };
                    let c = self.around[k as usize] as usize;
                    if take[c] && (!used[c] || c == h0) {
                        found = Some(c);
                        break;
                    }
                }
                match found {
                    Some(n) if n != h0 => h = n,
                    _ => break,
                }
            }
            out.push(self.contour(&hs));
        }
        out
    }

    fn mark(&self, h: usize) -> Mark {
        let (si, fwd) = self.rep[h >> 1];
        let s = self.srcs[si as usize];
        // the half-edge runs eu -> ev for h even; the source ran eu -> ev when fwd
        let along = (h & 1 == 0) == fwd;
        Mark { operand: s.operand, contour: s.contour, edge: s.edge, reversed: !along }
    }

    fn contour(&self, hs: &[u32]) -> Contour {
        let n = hs.len();
        let marks: Vec<Mark> = hs.iter().map(|&h| self.mark(h as usize)).collect();
        // start where the mark changes, so that a run of one mark becomes one edge
        let start = (0..n).find(|&i| marks[i] != marks[(i + n - 1) % n]).unwrap_or(0);
        let mut points = Vec::with_capacity(n);
        let mut ms = Vec::with_capacity(n);
        for k in 0..n {
            let i = (start + k) % n;
            if k > 0 && marks[i] == marks[(i + n - 1) % n] {
                continue;
            }
            let p = self.verts[self.origin(hs[i] as usize) as usize];
            points.push(self.back.get(&p).copied().unwrap_or_else(|| self.grid.to_f64(p)));
            ms.push(marks[i]);
        }
        Contour { points, marks: ms }
    }
}
