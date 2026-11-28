// tests -- unit tests of geom (cargo test -p jflib-geom --lib); the modelling tests run in apps/modeltool
//
// region: Boolean operations and the decomposition on many fixed cases (shared edges, corners, identical
// operands, holes on their outer contour, self-overlap, degenerate input, tiny and huge coordinates,
// cutters) and properties on random polygons: area(A + B) + area(A & B) = area(A) + area(B), area(A - B)
// + area(A & B) = area(A), the pieces of a decomposition cover exactly the union, scattered points on the
// right side, every mark lies on its input edge, no two output edges cross.

use alloc::vec;
use alloc::vec::Vec;

use crate::region::*;

type Pt = [f64; 2];

fn sq(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pt> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

fn rev(mut c: Vec<Pt>) -> Vec<Pt> {
    c.reverse();
    c
}

fn one(c: Vec<Pt>) -> Operand {
    Operand::closed(vec![c])
}

fn ngon(cx: f64, cy: f64, r: f64, n: usize, phase: f64) -> Vec<Pt> {
    (0..n)
        .map(|i| {
            let a = phase + core::f64::consts::TAU * i as f64 / n as f64;
            [cx + r * crate::math::cos(a), cy + r * crate::math::sin(a)]
        })
        .collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

// a deterministic generator (xorshift64*)
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }

    // sometimes on a coarse grid (shared edges, touching corners), sometimes anywhere
    fn coord(&mut self, coarse: bool) -> f64 {
        if coarse { (self.next() % 9) as f64 * 0.5 } else { self.range(0.0, 4.0) }
    }

    // a random simple polygon: a star around a centre, or a box
    fn polygon(&mut self) -> Vec<Pt> {
        let coarse = self.next() % 3 == 0;
        if self.next() % 2 == 0 {
            let (x0, x1) = (self.coord(coarse), self.coord(coarse));
            let (y0, y1) = (self.coord(coarse), self.coord(coarse));
            let (x0, x1) = (x0.min(x1), x0.max(x1) + 0.5);
            let (y0, y1) = (y0.min(y1), y0.max(y1) + 0.5);
            return sq(x0, y0, x1, y1);
        }
        let n = 3 + (self.next() % 12) as usize;
        let (cx, cy) = (self.range(1.0, 3.0), self.range(1.0, 3.0));
        let mut angles: Vec<f64> = (0..n).map(|_| self.range(0.0, core::f64::consts::TAU)).collect();
        angles.sort_by(|a, b| a.partial_cmp(b).unwrap());
        angles.dedup();
        angles.iter().map(|&a| {
            let r = self.range(0.3, 2.0);
            [cx + r * crate::math::cos(a), cy + r * crate::math::sin(a)]
        }).collect()
    }
}

// the winding number of p for closed polylines (ray to the right)
fn winding(contours: &[Vec<Pt>], p: Pt) -> i32 {
    let mut w = 0;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + 1) % n]);
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                if x > p[0] {
                    w += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
    }
    w
}

fn result_has(contours: &[Contour], p: Pt) -> bool {
    let cs: Vec<Vec<Pt>> = contours.iter().map(|c| c.points.clone()).collect();
    winding(&cs, p) != 0
}

// the distance of p to the nearest edge of the contours
fn near_edge(contours: &[Vec<Pt>], p: Pt) -> f64 {
    let mut best = f64::INFINITY;
    for c in contours {
        let n = c.len();
        for i in 0..n {
            let (a, b) = (c[i], c[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let (qx, qy) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
            best = best.min(crate::math::sqrt(qx * qx + qy * qy));
        }
    }
    best
}

fn seg_dist(p: Pt, a: Pt, b: Pt) -> f64 {
    near_edge(&[vec![a, b]], p)
}

// every output edge lies on the input edge its mark names (both ends within tol of it)
fn marks_ok(ops: &[Operand], out: &[Contour], tol: f64) -> bool {
    out.iter().all(|c| {
        let n = c.points.len();
        c.marks.len() == n
            && (0..n).all(|i| {
                let m = c.marks[i];
                let src = &ops[m.operand as usize].contours[m.contour as usize];
                let (a, b) = (src[m.edge as usize], src[(m.edge as usize + 1) % src.len()]);
                let (p, q) = (c.points[i], c.points[(i + 1) % n]);
                // the direction agrees with `reversed`
                let along = (q[0] - p[0]) * (b[0] - a[0]) + (q[1] - p[1]) * (b[1] - a[1]) > 0.0;
                seg_dist(p, a, b) <= tol && seg_dist(q, a, b) <= tol && along != m.reversed
            })
    })
}

// no two output edges cross properly (exact orientation on the f64 points)
fn no_crossings(out: &[Contour]) -> bool {
    let mut segs: Vec<(Pt, Pt)> = Vec::new();
    for c in out {
        let n = c.points.len();
        for i in 0..n {
            segs.push((c.points[i], c.points[(i + 1) % n]));
        }
    }
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            let ((a, b), (c, d)) = (segs[i], segs[j]);
            let o = crate::exact::orient2d;
            if o(a, b, c) * o(a, b, d) < 0 && o(c, d, a) * o(c, d, b) < 0 {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------- fixed cases

#[test]
fn squares_sharing_an_edge() {
    let ops = [one(sq(0.0, 0.0, 1.0, 1.0)), one(sq(1.0, 0.0, 2.0, 1.0))];
    let u = union(&ops, FillRule::NonZero).unwrap();
    assert_eq!(u.len(), 1);
    assert_eq!(area(&u), 2.0);
    assert!(u[0].points.contains(&[2.0, 1.0]) && u[0].points.contains(&[0.0, 0.0]));
    assert!(intersection(&ops, FillRule::NonZero).unwrap().is_empty());
    let d = difference(&ops, FillRule::NonZero).unwrap();
    assert_eq!(area(&d), 1.0);
    assert_eq!(area(&xor(&ops, FillRule::NonZero).unwrap()), 2.0);
    assert!(marks_ok(&ops, &u, 1e-12));
    // the pieces: two, each covered by one
    let p = decompose(&ops, FillRule::NonZero).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].cover.len() + p[1].cover.len(), 2);
}

#[test]
fn squares_corner_to_corner() {
    let ops = [one(sq(0.0, 0.0, 1.0, 1.0)), one(sq(1.0, 1.0, 2.0, 2.0))];
    let u = union(&ops, FillRule::NonZero).unwrap();
    // two contours touching in a point
    assert_eq!(u.len(), 2);
    assert_eq!(area(&u), 2.0);
    assert!(u.iter().all(|c| c.points.len() == 4 && c.area() == 1.0));
    assert!(intersection(&ops, FillRule::NonZero).unwrap().is_empty());
}

#[test]
fn identical_operands() {
    let a = sq(0.25, 0.5, 3.0, 2.0);
    let ops = [one(a.clone()), one(a.clone()), one(rev(a.clone()))];
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let u = union(&ops, rule).unwrap();
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].points.len(), 4);
        assert_eq!(area(&u), 2.75 * 1.5);
        // the marks name the first operand
        assert!(u[0].marks.iter().all(|m| m.operand == 0 && !m.reversed));
        // the input points come back bit for bit
        for p in &u[0].points {
            assert!(a.contains(p));
        }
        assert_eq!(area(&intersection(&ops, rule).unwrap()), 2.75 * 1.5);
        assert!(difference(&ops, rule).unwrap().is_empty());
        // three times the same: odd
        assert_eq!(area(&xor(&ops, rule).unwrap()), 2.75 * 1.5);
        assert!(xor(&ops[..2], rule).unwrap().is_empty());
    }
}

#[test]
fn hole_touching_the_outer_contour() {
    // a square with a hole along part of its left side and one touching a corner
    let outer = sq(0.0, 0.0, 4.0, 4.0);
    let hole1 = rev(sq(0.0, 1.0, 1.0, 3.0));
    let hole2 = rev(sq(3.0, 3.0, 4.0, 4.0));
    let a = Operand::closed(vec![outer, hole1, hole2]);
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let u = union(core::slice::from_ref(&a), rule).unwrap();
        assert_eq!(area(&u), 16.0 - 2.0 - 1.0, "{rule:?}");
        assert!(marks_ok(core::slice::from_ref(&a), &u, 1e-12));
    }
    // a hole drawn the same way round as its outer contour: only even-odd makes it one
    let b = Operand::closed(vec![sq(0.0, 0.0, 4.0, 4.0), sq(1.0, 1.0, 2.0, 2.0)]);
    assert_eq!(area(&union(core::slice::from_ref(&b), FillRule::EvenOdd).unwrap()), 15.0);
    assert_eq!(area(&union(core::slice::from_ref(&b), FillRule::NonZero).unwrap()), 16.0);
}

#[test]
fn hole_from_a_difference_and_islands() {
    let ops = [one(sq(0.0, 0.0, 10.0, 10.0)), one(sq(2.0, 2.0, 8.0, 8.0))];
    let d = difference(&ops, FillRule::NonZero).unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(d.iter().filter(|c| c.is_hole()).count(), 1);
    assert_eq!(area(&d), 64.0);
    // an island in the hole
    let ops2 = [ops[0].clone(), ops[1].clone(), one(sq(4.0, 4.0, 6.0, 6.0))];
    let x = xor(&ops2, FillRule::NonZero).unwrap();
    assert_eq!(area(&x), 100.0 - 36.0 + 4.0);
    assert!(result_has(&x, [5.0, 5.0]) && !result_has(&x, [3.0, 5.0]) && result_has(&x, [1.0, 1.0]));
    // the pieces of the three: ring, ring, middle; covers [0], [0, 1], [0, 1, 2]
    let p = decompose(&ops2, FillRule::NonZero).unwrap();
    let mut covers: Vec<Vec<u32>> = p.iter().map(|x| x.cover.clone()).collect();
    covers.sort();
    assert_eq!(covers, vec![vec![0], vec![0, 1], vec![0, 1, 2]]);
    let total: f64 = p.iter().map(|x| area(&x.contours)).sum();
    assert_eq!(total, 100.0);
    // each ring has its hole
    assert!(p.iter().filter(|x| x.cover.len() < 3).all(|x| x.contours.len() == 2));
}

#[test]
fn self_overlapping_operand() {
    let a = Operand::closed(vec![sq(0.0, 0.0, 2.0, 2.0), sq(1.0, 1.0, 3.0, 3.0)]);
    assert_eq!(area(&union(core::slice::from_ref(&a), FillRule::NonZero).unwrap()), 7.0);
    assert_eq!(area(&union(core::slice::from_ref(&a), FillRule::EvenOdd).unwrap()), 6.0);
    // a bow tie: a contour crossing itself
    let bow = one(vec![[0.0, 0.0], [2.0, 2.0], [2.0, 0.0], [0.0, 2.0]]);
    let u = union(core::slice::from_ref(&bow), FillRule::NonZero).unwrap();
    assert_eq!(u.len(), 2);
    assert!(close(area(&u), 2.0, 1e-12));
    assert!(u.iter().all(|c| c.area() > 0.0));
}

#[test]
fn degenerate_input() {
    // no area, repeated points, a contour of two points, empty operands
    let flat = one(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]);
    let dup = one(vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]]);
    let two = one(vec![[5.0, 5.0], [6.0, 6.0]]);
    let empty = Operand::closed(vec![]);
    assert!(union(core::slice::from_ref(&flat), FillRule::NonZero).unwrap().is_empty());
    let u = union(&[flat.clone(), dup.clone(), two.clone(), empty.clone()], FillRule::NonZero).unwrap();
    assert_eq!(u.len(), 1);
    assert_eq!(area(&u), 1.0);
    assert_eq!(u[0].points.len(), 4);
    // an empty operand makes an empty intersection, an empty first one an empty difference
    assert!(intersection(&[dup.clone(), empty.clone()], FillRule::NonZero).unwrap().is_empty());
    assert!(difference(&[empty.clone(), dup.clone()], FillRule::NonZero).unwrap().is_empty());
    assert!(union(&[], FillRule::NonZero).unwrap().is_empty());
    // a point only
    assert!(union(&[one(vec![[1.0, 1.0]; 3])], FillRule::NonZero).unwrap().is_empty());
    // not finite: refused
    let bad = one(vec![[0.0, 0.0], [f64::NAN, 1.0], [1.0, 1.0]]);
    assert!(union(core::slice::from_ref(&bad), FillRule::NonZero).is_err());
}

#[test]
fn tiny_huge_and_far_coordinates() {
    for (scale, off) in [(1e-12, 0.0), (1e12, 0.0), (1.0, 1e6), (1e-3, -5e3), (3e-150, 0.0), (1e150, 1e151)] {
        let s = |x0: f64, y0: f64, x1: f64, y1: f64| sq(off + x0 * scale, off + y0 * scale, off + x1 * scale, off + y1 * scale);
        let ops = [one(s(0.0, 0.0, 2.0, 2.0)), one(s(1.0, 1.0, 3.0, 3.0))];
        let u = union(&ops, FillRule::NonZero).unwrap();
        let i = intersection(&ops, FillRule::NonZero).unwrap();
        assert_eq!(u.len(), 1, "{scale} {off}");
        assert_eq!(u[0].points.len(), 8);
        let rel = |x: f64| x / scale / scale;
        assert!(close(rel(area(&u)), 7.0, 1e-6), "{scale} {off}: {}", rel(area(&u)));
        assert!(close(rel(area(&i)), 1.0, 1e-6));
        // the crossing points of the edges are on the grid's nearest step, the corners exact
        let corners = [ops[0].contours[0].clone(), ops[1].contours[0].clone()].concat();
        let exact = u[0].points.iter().filter(|p| corners.contains(p)).count();
        assert_eq!(exact, 6);
    }
}

#[test]
fn crossing_points_and_marks() {
    // a triangle over a square: the crossing points are not on the grid exactly
    let ops = [one(sq(0.0, 0.0, 3.0, 3.0)), one(vec![[1.0, -1.0], [2.5, 4.1], [-0.7, 1.3]])];
    for op in [BoolOp::Union, BoolOp::Intersection, BoolOp::Difference, BoolOp::Xor] {
        let r = boolean(&ops, op, FillRule::NonZero).unwrap();
        assert!(marks_ok(&ops, &r, 1e-8), "{op:?}");
        assert!(no_crossings(&r), "{op:?}");
    }
    let u = area(&union(&ops, FillRule::NonZero).unwrap());
    let i = area(&intersection(&ops, FillRule::NonZero).unwrap());
    let a = crate::poly::area(&ops[1].contours[0]).abs();
    // the crossing points move by at most half a step of the grid (5 / 2^31 here)
    assert!(close(u + i, 9.0 + a, 1e-7), "{}", u + i - 9.0 - a);
}

#[test]
fn cutters_split_pieces() {
    let sqr = one(sq(0.0, 0.0, 2.0, 2.0));
    // a line right through: two halves
    let cut = Operand::open(vec![vec![[-1.0, 1.0], [3.0, 1.0]]]);
    let p = decompose(&[sqr.clone(), cut.clone()], FillRule::NonZero).unwrap();
    assert_eq!(p.len(), 2);
    assert!(p.iter().all(|x| x.cover == vec![0] && area(&x.contours) == 2.0));
    // a line that ends inside: no split, the piece's boundary is the square
    let stub = Operand::open(vec![vec![[-1.0, 1.0], [1.0, 1.0]]]);
    let p = decompose(&[sqr.clone(), stub], FillRule::NonZero).unwrap();
    assert_eq!(p.len(), 1);
    assert_eq!(area(&p[0].contours), 4.0);
    // a polyline in a V and a diagonal: four pieces
    let v = Operand::open(vec![vec![[-0.5, 2.5], [1.0, -0.5], [2.5, 2.5]], vec![[-1.0, -1.0], [3.0, 3.0]]]);
    let p = decompose(&[sqr.clone(), v], FillRule::NonZero).unwrap();
    let total: f64 = p.iter().map(|x| area(&x.contours)).sum();
    assert!(close(total, 4.0, 1e-9));
    assert!(p.len() >= 4);
    // cutters do not take part in Boolean operations
    assert_eq!(area(&union(&[sqr, cut], FillRule::NonZero).unwrap()), 4.0);
}

#[test]
fn pieces_of_two_overlapping() {
    let ops = [one(sq(0.0, 0.0, 2.0, 2.0)), one(ngon(2.0, 1.0, 1.0, 16, 0.0))];
    let p = decompose(&ops, FillRule::NonZero).unwrap();
    let mut covers: Vec<Vec<u32>> = p.iter().map(|x| x.cover.clone()).collect();
    covers.sort();
    assert_eq!(covers, vec![vec![0], vec![0, 1], vec![1]]);
    let get = |c: &[u32]| p.iter().find(|x| x.cover == c).map(|x| area(&x.contours)).unwrap();
    let i = area(&intersection(&ops, FillRule::NonZero).unwrap());
    assert!(close(get(&[0, 1]), i, 1e-12));
    assert!(close(get(&[0]) + i, 4.0, 1e-12));
    for x in &p {
        assert!(marks_ok(&ops, &x.contours, 1e-8));
    }
}

#[test]
fn deterministic() {
    let mut r = Rng(7);
    let ops: Vec<Operand> = (0..5).map(|_| one(r.polygon())).collect();
    let a = decompose(&ops, FillRule::EvenOdd).unwrap();
    let b = decompose(&ops, FillRule::EvenOdd).unwrap();
    assert_eq!(a, b);
}

// ---------------------------------------------------------------- properties on random polygons

#[test]
fn random_area_identities_and_points() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let mut checked = 0;
    for round in 0..300 {
        let rule = if round % 2 == 0 { FillRule::NonZero } else { FillRule::EvenOdd };
        let n = 2 + round % 3;
        let ops: Vec<Operand> = (0..n).map(|_| one(r.polygon())).collect();
        let pair = &ops[..2];
        let u = union(pair, rule).unwrap();
        let i = intersection(pair, rule).unwrap();
        let d = difference(pair, rule).unwrap();
        let x = xor(pair, rule).unwrap();
        let a0 = area(&union(&ops[..1], rule).unwrap());
        let a1 = area(&union(&ops[1..2], rule).unwrap());
        let tol = 1e-7;
        assert!(close(area(&u) + area(&i), a0 + a1, tol), "round {round}");
        assert!(close(area(&d) + area(&i), a0, tol), "round {round}");
        assert!(close(area(&x), area(&u) - area(&i), tol), "round {round}");
        for res in [&u, &i, &d, &x] {
            assert!(no_crossings(res), "round {round}");
            assert!(marks_ok(pair, res, 1e-7), "round {round}");
        }
        // the decomposition of all n covers exactly their union; per operand its area
        let pieces = decompose(&ops, rule).unwrap();
        let un = area(&union(&ops, rule).unwrap());
        let total: f64 = pieces.iter().map(|p| area(&p.contours)).sum();
        assert!(close(total, un, tol), "round {round}: {total} {un}");
        for k in 0..n as u32 {
            let ak: f64 = pieces.iter().filter(|p| p.cover.contains(&k)).map(|p| area(&p.contours)).sum();
            let own = area(&union(&ops[k as usize..k as usize + 1], rule).unwrap());
            assert!(close(ak, own, tol), "round {round} operand {k}");
        }
        // scattered points away from every edge: on the right side of each result
        let inputs: Vec<Vec<Pt>> = ops.iter().map(|o| o.contours[0].clone()).collect();
        for _ in 0..30 {
            let p = [r.range(-0.5, 5.0), r.range(-0.5, 5.0)];
            if near_edge(&inputs, p) < 1e-6 {
                continue;
            }
            let ins = |k: usize| {
                let w = winding(&ops[k].contours, p);
                match rule {
                    FillRule::NonZero => w != 0,
                    FillRule::EvenOdd => w & 1 != 0,
                }
            };
            let (in0, in1) = (ins(0), ins(1));
            assert_eq!(result_has(&u, p), in0 || in1, "round {round} {p:?}");
            assert_eq!(result_has(&i, p), in0 && in1, "round {round} {p:?}");
            assert_eq!(result_has(&d, p), in0 && !in1, "round {round} {p:?}");
            assert_eq!(result_has(&x, p), in0 != in1, "round {round} {p:?}");
            let cover: Vec<u32> = (0..n).filter(|&k| ins(k)).map(|k| k as u32).collect();
            let hit: Vec<&Piece> = pieces.iter().filter(|q| result_has(&q.contours, p)).collect();
            if cover.is_empty() {
                assert!(hit.is_empty(), "round {round} {p:?}");
            } else {
                assert_eq!(hit.len(), 1, "round {round} {p:?}");
                assert_eq!(hit[0].cover, cover, "round {round} {p:?}");
            }
            checked += 1;
        }
    }
    assert!(checked > 5000);
}

#[test]
fn random_overlapping_circles() {
    // many edges in many directions, almost parallel ones, crossing points close together
    let mut r = Rng(42);
    for _ in 0..40 {
        let ops: Vec<Operand> = (0..4)
            .map(|_| one(ngon(r.range(0.0, 2.0), r.range(0.0, 2.0), r.range(0.5, 2.0), 32 + (r.next() % 64) as usize, r.range(0.0, 1.0))))
            .collect();
        let u = union(&ops, FillRule::NonZero).unwrap();
        let pieces = decompose(&ops, FillRule::NonZero).unwrap();
        let total: f64 = pieces.iter().map(|p| area(&p.contours)).sum();
        assert!(close(total, area(&u), 1e-7));
        assert!(no_crossings(&u));
    }
}
