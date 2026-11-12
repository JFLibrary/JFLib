// fill -- a fill's rings on the CPU (decision 26; docs/RENDER2D.md "Flächen"): cleaned, nested (even-odd: a ring inside
// an odd number of others is a hole), oriented (the inside on the left in drawing coordinates: outer rings
// counter-clockwise, holes clockwise), and each outer ring with its holes cut into triangles in f64.
//
//   prepare(rings) -> Prepared   the rings as they go up (their points), the triangles as indices into them
//
// The triangulation is ear clipping after earcut (Mapbox, ISC; written anew here): holes bridged into their outer ring
// left to right (Eberly's bridge), ears found through a z-order hash for larger rings, collinear and repeated points
// filtered, and where no ear is left, small self-intersections cured locally and as a last resort the polygon split by
// a valid diagonal.  It only takes the rings' points, never new ones, so every triangle's corner is a ring's point (the
// shader finds a corner's two boundary edges through it).  Robust against collinear and repeated points and rings
// that touch; self-intersecting rings come out approximately (decision 26).  The numbers are relative to the fill's
// first point (small differences, exact orientation tests where the points allow).

use alloc::vec;
use alloc::vec::Vec;

/// the rings ready for the records: their points (relative to `origin`), oriented; triangles as (ring, point) pairs; per
/// triangle and per edge the edges near it (near: numbered ring after ring, nearest first)
#[derive(Clone, PartialEq)]
pub struct Prepared {
    pub origin: [f64; 2],
    pub rings: Vec<Vec<[f64; 2]>>,
    pub tris: Vec<[(u32, u32); 3]>,
    pub tri_near: Vec<Vec<u32>>,
    pub edge_near: Vec<Vec<u32>>,
}

/// the most edges a triangle's record names (the fragment shader computes its pixels from them)
pub const TRI_NEAR: usize = 20;
/// the most edges besides its two neighbours an edge's record names
pub const EDGE_NEAR: usize = 4;

fn seg_dist2(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let ll = dx * dx + dy * dy;
    let t = if ll > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / ll).clamp(0.0, 1.0) } else { 0.0 };
    let (ex, ey) = (p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
    ex * ex + ey * ey
}

fn cross(o: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// the squared distance of two segments (0 where they cross)
fn segs_dist2(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> f64 {
    let (d1, d2, d3, d4) = (cross(a, b, c), cross(a, b, d), cross(c, d, a), cross(c, d, b));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return 0.0;
    }
    seg_dist2(a, c, d).min(seg_dist2(b, c, d)).min(seg_dist2(c, a, b)).min(seg_dist2(d, a, b))
}

impl Prepared {
    /// the edges within d (drawing units) of each triangle and of each edge (other than itself and its neighbours),
    /// nearest first, at most TRI_NEAR and EDGE_NEAR (beyond: the nearest ones; the shader is then approximate there)
    pub fn near(&mut self, d: f64) {
        let mut edges: Vec<([f64; 2], [f64; 2], u32, u32)> = Vec::new();
        let mut base = Vec::with_capacity(self.rings.len());
        for r in &self.rings {
            base.push(edges.len() as u32);
            let n = r.len();
            let b = edges.len() as u32;
            for k in 0..n {
                edges.push((r[k], r[(k + 1) % n], b + ((k + n - 1) % n) as u32, b + ((k + 1) % n) as u32));
            }
        }
        // a grid of the edges' boxes grown by d
        let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for e in &edges {
            for p in [e.0, e.1] {
                bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
            }
        }
        let side = (bb[2] - bb[0]).max(bb[3] - bb[1]).max(1e-300);
        let cells = (crate::math::sqrt(edges.len() as f64) as usize).clamp(1, 256);
        let cs = (side / cells as f64).max(d);
        let nx = (((bb[2] - bb[0]) / cs) as usize + 1).min(4096);
        let ny = (((bb[3] - bb[1]) / cs) as usize + 1).min(4096);
        let cell = |x: f64, y: f64| -> (usize, usize) { ((((x - bb[0]) / cs).max(0.0) as usize).min(nx - 1), (((y - bb[1]) / cs).max(0.0) as usize).min(ny - 1)) };
        let mut grid: Vec<Vec<u32>> = vec![Vec::new(); nx * ny];
        for (k, e) in edges.iter().enumerate() {
            let (x0, y0) = cell(e.0[0].min(e.1[0]) - d, e.0[1].min(e.1[1]) - d);
            let (x1, y1) = cell(e.0[0].max(e.1[0]) + d, e.0[1].max(e.1[1]) + d);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    grid[y * nx + x].push(k as u32);
                }
            }
        }
        let d2 = d * d;
        let query = |lo: [f64; 2], hi: [f64; 2], out: &mut Vec<u32>| {
            out.clear();
            let (x0, y0) = cell(lo[0], lo[1]);
            let (x1, y1) = cell(hi[0], hi[1]);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    out.extend_from_slice(&grid[y * nx + x]);
                }
            }
            out.sort_unstable();
            out.dedup();
        };
        let mut cand = Vec::new();
        self.tri_near.clear();
        for t in &self.tris {
            let p = |(r, k): (u32, u32)| self.rings[r as usize][k as usize];
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            query([a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])], [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])], &mut cand);
            let area = cross(a, b, c);
            let mut list: Vec<(f64, u32)> = Vec::new();
            for &k in &cand {
                let e = &edges[k as usize];
                // inside the triangle: 0; else the nearest of its sides
                let inside = |q: [f64; 2]| {
                    let (s0, s1, s2) = (cross(a, b, q), cross(b, c, q), cross(c, a, q));
                    if area >= 0.0 { s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0 } else { s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0 }
                };
                let dd = if inside(e.0) || inside(e.1) { 0.0 } else { segs_dist2(e.0, e.1, a, b).min(segs_dist2(e.0, e.1, b, c)).min(segs_dist2(e.0, e.1, c, a)) };
                if dd <= d2 {
                    list.push((dd, k));
                }
            }
            list.sort_by(|x, y| x.partial_cmp(y).unwrap_or(core::cmp::Ordering::Equal));
            list.truncate(TRI_NEAR);
            self.tri_near.push(list.iter().map(|x| x.1).collect());
        }
        self.edge_near.clear();
        for (k, e) in edges.iter().enumerate() {
            query([e.0[0].min(e.1[0]) - d, e.0[1].min(e.1[1]) - d], [e.0[0].max(e.1[0]) + d, e.0[1].max(e.1[1]) + d], &mut cand);
            let mut list: Vec<(f64, u32)> = Vec::new();
            for &j in &cand {
                if j == k as u32 || j == e.2 || j == e.3 {
                    continue;
                }
                let o = &edges[j as usize];
                let dd = segs_dist2(e.0, e.1, o.0, o.1);
                if dd <= d2 {
                    list.push((dd, j));
                }
            }
            list.sort_by(|x, y| x.partial_cmp(y).unwrap_or(core::cmp::Ordering::Equal));
            list.truncate(EDGE_NEAR);
            self.edge_near.push(list.iter().map(|x| x.1).collect());
        }
    }
}

fn signed_area(p: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    let n = p.len();
    for i in 0..n {
        let (a, b) = (p[i], p[(i + 1) % n]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * s
}

fn bbox(p: &[[f64; 2]]) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for q in p {
        b[0] = b[0].min(q[0]);
        b[1] = b[1].min(q[1]);
        b[2] = b[2].max(q[0]);
        b[3] = b[3].max(q[1]);
    }
    b
}

/// where p lies against ring r: Some(true) inside, Some(false) outside, None on its boundary (within eps)
fn classify(r: &[[f64; 2]], p: [f64; 2], eps: f64) -> Option<bool> {
    let n = r.len();
    let mut inside = false;
    for i in 0..n {
        let (a, b) = (r[i], r[(i + 1) % n]);
        // on the edge?
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let ll = dx * dx + dy * dy;
        let t = if ll > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / ll).clamp(0.0, 1.0) } else { 0.0 };
        let (ex, ey) = (p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
        if ex * ex + ey * ey <= eps * eps {
            return None;
        }
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * dx / dy;
            if x > p[0] {
                inside = !inside;
            }
        }
    }
    Some(inside)
}

/// ring a inside ring b (a point of a off b's boundary decides; a ring that lies on b all along is not inside it)
fn inside_of(a: &[[f64; 2]], ba: &[f64; 4], b: &[[f64; 2]], bb: &[f64; 4], eps: f64) -> bool {
    if ba[0] < bb[0] - eps || ba[1] < bb[1] - eps || ba[2] > bb[2] + eps || ba[3] > bb[3] + eps {
        return false;
    }
    for &p in a {
        if let Some(v) = classify(b, p, eps) {
            return v;
        }
    }
    // (all of a on b: the midpoints of a's edges)
    for i in 0..a.len() {
        let (p, q) = (a[i], a[(i + 1) % a.len()]);
        if let Some(v) = classify(b, [0.5 * (p[0] + q[0]), 0.5 * (p[1] + q[1])], eps) {
            return v;
        }
    }
    false
}

/// a diamond angle of a direction: monotone in its angle, in [0, 4)
fn pseudo(dx: f64, dy: f64) -> f64 {
    let s = dx.abs() + dy.abs();
    if s == 0.0 {
        return 0.0;
    }
    let (x, y) = (dx / s, dy / s);
    if y >= 0.0 {
        if x >= 0.0 { y } else { 2.0 - y }
    } else if x < 0.0 {
        2.0 - y
    } else {
        4.0 + y
    }
}

/// rings that cross themselves or each other, overlap or touch along an edge, as the boundary of their even-odd region:
/// every edge split where another crosses or touches it, edges that come twice dropped (both their sides alike), each
/// left oriented with the odd side on its left, and the cycles traced along the faces on their left (so they cross no
/// more; they may touch at points).  None: no ring crosses or touches another one or itself (the rings as they are)
fn untangle(rings: &[Vec<[f64; 2]>], size: f64) -> Option<Vec<Vec<[f64; 2]>>> {
    let mut edges: Vec<([f64; 2], [f64; 2])> = Vec::new();
    for r in rings {
        let n = r.len();
        for k in 0..n {
            edges.push((r[k], r[(k + 1) % n]));
        }
    }
    let ne = edges.len();
    let eps = size * 1e-12;
    // the splits of each edge: (t, point)
    let mut splits: Vec<Vec<(f64, [f64; 2])>> = vec![Vec::new(); ne];
    let mut found = false;
    // (a grid of the edges' boxes for many edges)
    let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for e in &edges {
        for p in [e.0, e.1] {
            bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
        }
    }
    let cells = (crate::math::sqrt(ne as f64) as usize).clamp(1, 128);
    let cs = ((bb[2] - bb[0]).max(bb[3] - bb[1]) / cells as f64).max(1e-300);
    let cell = |x: f64| (((x / cs).max(0.0)) as usize).min(cells - 1);
    let mut grid: Vec<Vec<u32>> = vec![Vec::new(); cells * cells];
    for (k, e) in edges.iter().enumerate() {
        let (x0, x1) = (cell(e.0[0].min(e.1[0]) - bb[0] - eps), cell(e.0[0].max(e.1[0]) - bb[0] + eps));
        let (y0, y1) = (cell(e.0[1].min(e.1[1]) - bb[1] - eps), cell(e.0[1].max(e.1[1]) - bb[1] + eps));
        for y in y0..=y1 {
            for x in x0..=x1 {
                grid[y * cells + x].push(k as u32);
            }
        }
    }
    let mut seen = alloc::collections::BTreeSet::new();
    for g in &grid {
        for (ai, &i) in g.iter().enumerate() {
            for &j in &g[ai + 1..] {
                let (i, j) = (i.min(j) as usize, i.max(j) as usize);
                if !seen.insert((i, j)) {
                    continue;
                }
                let (p, p2) = edges[i];
                let (q, q2) = edges[j];
                let r = [p2[0] - p[0], p2[1] - p[1]];
                let s = [q2[0] - q[0], q2[1] - q[1]];
                let rr = r[0] * r[0] + r[1] * r[1];
                let ss = s[0] * s[0] + s[1] * s[1];
                if rr == 0.0 || ss == 0.0 {
                    continue;
                }
                let qp = [q[0] - p[0], q[1] - p[1]];
                let den = r[0] * s[1] - r[1] * s[0];
                let near = |t: f64, l2: f64| t * t * l2 <= eps * eps;
                if den.abs() > 1e-12 * crate::math::sqrt(rr * ss) {
                    let t = (qp[0] * s[1] - qp[1] * s[0]) / den;
                    let u = (qp[0] * r[1] - qp[1] * r[0]) / den;
                    if t < 0.0 && !near(t, rr) || t > 1.0 && !near(t - 1.0, rr) || u < 0.0 && !near(u, ss) || u > 1.0 && !near(u - 1.0, ss) {
                        continue;
                    }
                    // (at an end of the other edge: that end itself)
                    let (te, ue) = (near(t, rr) || near(t - 1.0, rr), near(u, ss) || near(u - 1.0, ss));
                    if te && ue {
                        // (ends meet: the rings' own vertices, nothing to split)
                        continue;
                    }
                    let x = if ue { if near(u, ss) { q } else { q2 } } else if te { if near(t, rr) { p } else { p2 } } else { [p[0] + t * r[0], p[1] + t * r[1]] };
                    if !te {
                        splits[i].push((t, x));
                    }
                    if !ue {
                        splits[j].push((u, x));
                    }
                    found = true;
                } else if (qp[0] * r[1] - qp[1] * r[0]).abs() <= eps * crate::math::sqrt(rr) {
                    // collinear: each splits at the other's ends inside it
                    let mut any = false;
                    for (a, b, l2, e) in [(p, r, rr, i), (q, s, ss, j)] {
                        let (o, o2) = if e == i { (q, q2) } else { (p, p2) };
                        for x in [o, o2] {
                            let t = ((x[0] - a[0]) * b[0] + (x[1] - a[1]) * b[1]) / l2;
                            if t > 0.0 && t < 1.0 && !near(t, l2) && !near(t - 1.0, l2) {
                                splits[e].push((t, x));
                                any = true;
                            }
                        }
                    }
                    let t0 = (qp[0] * r[0] + qp[1] * r[1]) / rr;
                    let t1 = ((q2[0] - p[0]) * r[0] + (q2[1] - p[1]) * r[1]) / rr;
                    // (overlapping, or the same edge twice)
                    found |= any || (t0.min(t1) < 1.0 && t0.max(t1) > 0.0);
                }
            }
        }
    }
    if !found {
        return None;
    }
    // the pieces, their ends merged within eps
    let key = |p: [f64; 2]| ((p[0] / (eps * 4.0)).round_f(), (p[1] / (eps * 4.0)).round_f());
    let mut ids: alloc::collections::BTreeMap<(i64, i64), u32> = alloc::collections::BTreeMap::new();
    let mut pts: Vec<[f64; 2]> = Vec::new();
    let mut id = |p: [f64; 2], pts: &mut Vec<[f64; 2]>| -> u32 {
        *ids.entry(key(p)).or_insert_with(|| {
            pts.push(p);
            pts.len() as u32 - 1
        })
    };
    // undirected pieces, counted (an even count: both sides alike, dropped)
    let mut count: alloc::collections::BTreeMap<(u32, u32), u32> = alloc::collections::BTreeMap::new();
    for (k, e) in edges.iter().enumerate() {
        let sp = &mut splits[k];
        sp.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        let mut a = id(e.0, &mut pts);
        for &(_, x) in sp.iter() {
            let b = id(x, &mut pts);
            if a != b {
                *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                a = b;
            }
        }
        let b = id(e.1, &mut pts);
        if a != b {
            *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    let und: Vec<(u32, u32)> = count.iter().filter(|e| *e.1 % 2 == 1).map(|e| *e.0).collect();
    // each piece with the odd side on its left: the parity of a point just left of its middle (a ray to +x)
    let parity = |x: f64, y: f64| -> bool {
        let mut c = false;
        for &(a, b) in &und {
            let (pa, pb) = (pts[a as usize], pts[b as usize]);
            if (pa[1] > y) != (pb[1] > y) {
                let xi = pa[0] + (y - pa[1]) * (pb[0] - pa[0]) / (pb[1] - pa[1]);
                if xi > x {
                    c = !c;
                }
            }
        }
        c
    };
    let mut out_edges: Vec<Vec<(u32, f64)>> = vec![Vec::new(); pts.len()];
    for &(a, b) in &und {
        let (pa, pb) = (pts[a as usize], pts[b as usize]);
        let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
        let l = crate::math::sqrt(dx * dx + dy * dy);
        let off = (l * 1e-6).max(eps * 64.0);
        let (mx, my) = (0.5 * (pa[0] + pb[0]), 0.5 * (pa[1] + pb[1]));
        let (lx, ly) = (mx - dy / l * off, my + dx / l * off);
        let (rx, ry) = (mx + dy / l * off, my - dx / l * off);
        let (pl, pr) = (parity(lx, ly), parity(rx, ry));
        if pl == pr {
            continue;
        }
        let (u, v) = if pl { (a, b) } else { (b, a) };
        let (pu, pv) = (pts[u as usize], pts[v as usize]);
        out_edges[u as usize].push((v, pseudo(pv[0] - pu[0], pv[1] - pu[1])));
    }
    // the cycles: at each vertex the outgoing piece first met turning clockwise from the way back
    let mut used: Vec<Vec<bool>> = out_edges.iter().map(|v| vec![false; v.len()]).collect();
    let mut cycles = Vec::new();
    for s in 0..pts.len() {
        for si in 0..out_edges[s].len() {
            if used[s][si] {
                continue;
            }
            let mut cyc = Vec::new();
            let (mut u, mut ui) = (s, si);
            for _ in 0..4 * und.len() + 4 {
                if used[u][ui] {
                    break;
                }
                used[u][ui] = true;
                cyc.push(pts[u]);
                let v = out_edges[u][ui].0 as usize;
                let back = pseudo(pts[u][0] - pts[v][0], pts[u][1] - pts[v][1]);
                // (the largest angle below the way back, else the largest)
                let mut best: Option<(usize, f64)> = None;
                for (k, &(w, a)) in out_edges[v].iter().enumerate() {
                    if w as usize == u && out_edges[v].len() > 1 {
                        continue;
                    }
                    let rel = if a < back { back - a } else { back - a + 4.0 };
                    if best.is_none_or(|b| rel < b.1) {
                        best = Some((k, rel));
                    }
                }
                let Some((k, _)) = best else { break };
                u = v;
                ui = k;
            }
            if cyc.len() >= 3 {
                cycles.push(cyc);
            }
        }
    }
    Some(cycles)
}

trait RoundF {
    fn round_f(self) -> i64;
}

impl RoundF for f64 {
    fn round_f(self) -> i64 {
        let t = (self + 0.5) as i64;
        if (t as f64) > self + 0.5 { t - 1 } else { t }
    }
}

/// the rings (absolute f64 points, implicitly closed): cleaned, nested, oriented, triangulated
pub fn prepare(input: &[Vec<[f64; 2]>]) -> Prepared {
    let origin = input.iter().find_map(|r| r.first().copied()).unwrap_or([0.0, 0.0]);
    let mut rings: Vec<Vec<[f64; 2]>> = Vec::new();
    for r in input {
        let mut v: Vec<[f64; 2]> = Vec::with_capacity(r.len());
        for p in r {
            let q = [p[0] - origin[0], p[1] - origin[1]];
            if v.last() != Some(&q) {
                v.push(q);
            }
        }
        while v.len() >= 2 && v[0] == v[v.len() - 1] {
            v.pop();
        }
        // (fewer than three points or no area: nothing to fill, no edge)
        if v.len() >= 3 && signed_area(&v) != 0.0 {
            rings.push(v);
        }
    }
    let boxes: Vec<[f64; 4]> = rings.iter().map(|r| bbox(r)).collect();
    let size = boxes.iter().fold(0.0f64, |m, b| m.max(b[2] - b[0]).max(b[3] - b[1]));
    // (crossing, overlapping or touching along an edge: the even-odd region's boundary instead)
    if let Some(c) = untangle(&rings, size) {
        rings = c.into_iter().filter(|r| r.len() >= 3 && signed_area(r) != 0.0).collect();
    }
    let n = rings.len();
    let boxes: Vec<[f64; 4]> = rings.iter().map(|r| bbox(r)).collect();
    let eps = size * 1e-12;
    // contained[a] = the rings a lies inside
    let mut depth = vec![0u32; n];
    let mut parent = vec![usize::MAX; n];
    let mut containers: Vec<Vec<usize>> = vec![Vec::new(); n];
    for a in 0..n {
        for b in 0..n {
            if a != b && inside_of(&rings[a], &boxes[a], &rings[b], &boxes[b], eps) {
                containers[a].push(b);
            }
        }
        depth[a] = containers[a].len() as u32;
    }
    for a in 0..n {
        // the innermost container: the deepest
        let mut best: Option<usize> = None;
        for &b in &containers[a] {
            if best.is_none_or(|x| depth[b] > depth[x]) {
                best = Some(b);
            }
        }
        if let Some(b) = best {
            parent[a] = b;
        }
    }
    // orientation: even depth counter-clockwise, odd clockwise (the inside on the left)
    for (a, r) in rings.iter_mut().enumerate() {
        let ccw = signed_area(r) > 0.0;
        if ccw != (depth[a] % 2 == 0) {
            r.reverse();
        }
    }
    let mut tris = Vec::new();
    for o in 0..n {
        if depth[o] % 2 != 0 {
            continue;
        }
        let holes: Vec<usize> = (0..n).filter(|&h| depth[h] == depth[o] + 1 && parent[h] == o).collect();
        // the points of the outer ring and its holes, one after the other
        let mut data: Vec<[f64; 2]> = rings[o].clone();
        let mut owner: Vec<(u32, u32)> = (0..rings[o].len()).map(|k| (o as u32, k as u32)).collect();
        let mut starts = Vec::new();
        for &hh in &holes {
            starts.push(data.len());
            data.extend_from_slice(&rings[hh]);
            owner.extend((0..rings[hh].len()).map(|k| (hh as u32, k as u32)));
        }
        let mut out = Vec::new();
        earcut(&data, &starts, &mut out);
        for t in out.chunks_exact(3) {
            tris.push([owner[t[0]], owner[t[1]], owner[t[2]]]);
        }
    }
    Prepared { origin, rings, tris, tri_near: Vec::new(), edge_near: Vec::new() }
}

// ---------------------------------------------------------------- earcut

const NIL: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Node {
    i: usize,
    x: f64,
    y: f64,
    prev: usize,
    next: usize,
    z: u32,
    prev_z: usize,
    next_z: usize,
    steiner: bool,
}

struct Ear {
    n: Vec<Node>,
    min_x: f64,
    min_y: f64,
    inv: f64,
}

impl Ear {
    fn insert(&mut self, i: usize, x: f64, y: f64, last: usize) -> usize {
        let k = self.n.len();
        self.n.push(Node { i, x, y, prev: k, next: k, z: 0, prev_z: NIL, next_z: NIL, steiner: false });
        if last != NIL {
            let ln = self.n[last].next;
            self.n[k].next = ln;
            self.n[k].prev = last;
            self.n[ln].prev = k;
            self.n[last].next = k;
        }
        k
    }

    fn remove(&mut self, p: usize) {
        let (pp, pn) = (self.n[p].prev, self.n[p].next);
        self.n[pn].prev = pp;
        self.n[pp].next = pn;
        let (zp, zn) = (self.n[p].prev_z, self.n[p].next_z);
        if zp != NIL {
            self.n[zp].next_z = zn;
        }
        if zn != NIL {
            self.n[zn].prev_z = zp;
        }
    }

    fn area(&self, p: usize, q: usize, r: usize) -> f64 {
        let (p, q, r) = (&self.n[p], &self.n[q], &self.n[r]);
        (q.y - p.y) * (r.x - q.x) - (q.x - p.x) * (r.y - q.y)
    }

    fn equals(&self, a: usize, b: usize) -> bool {
        self.n[a].x == self.n[b].x && self.n[a].y == self.n[b].y
    }

    fn linked_list(&mut self, data: &[[f64; 2]], start: usize, end: usize, clockwise: bool) -> usize {
        let mut sum = 0.0;
        let mut j = end - 1;
        for i in start..end {
            sum += (data[j][0] - data[i][0]) * (data[i][1] + data[j][1]);
            j = i;
        }
        let mut last = NIL;
        if clockwise == (sum > 0.0) {
            for (i, d) in data.iter().enumerate().take(end).skip(start) {
                last = self.insert(i, d[0], d[1], last);
            }
        } else {
            for i in (start..end).rev() {
                last = self.insert(i, data[i][0], data[i][1], last);
            }
        }
        if last != NIL && self.equals(last, self.n[last].next) {
            let nx = self.n[last].next;
            self.remove(last);
            last = nx;
        }
        last
    }

    fn filter(&mut self, start: usize, end: usize) -> usize {
        if start == NIL {
            return start;
        }
        let mut end = if end == NIL { start } else { end };
        let mut p = start;
        loop {
            let mut again = false;
            let pn = self.n[p].next;
            let pp = self.n[p].prev;
            if !self.n[p].steiner && (self.equals(p, pn) || self.area(pp, p, pn) == 0.0) {
                self.remove(p);
                p = pp;
                end = pp;
                if p == self.n[p].next {
                    break;
                }
                again = true;
            } else {
                p = pn;
            }
            if !(again || p != end) {
                break;
            }
        }
        end
    }

    fn z_order(&self, x: f64, y: f64) -> u32 {
        let mut x = ((x - self.min_x) * self.inv) as u32;
        let mut y = ((y - self.min_y) * self.inv) as u32;
        x = (x | (x << 8)) & 0x00FF_00FF;
        x = (x | (x << 4)) & 0x0F0F_0F0F;
        x = (x | (x << 2)) & 0x3333_3333;
        x = (x | (x << 1)) & 0x5555_5555;
        y = (y | (y << 8)) & 0x00FF_00FF;
        y = (y | (y << 4)) & 0x0F0F_0F0F;
        y = (y | (y << 2)) & 0x3333_3333;
        y = (y | (y << 1)) & 0x5555_5555;
        x | (y << 1)
    }

    fn index_curve(&mut self, start: usize) {
        let mut p = start;
        loop {
            if self.n[p].z == 0 {
                self.n[p].z = self.z_order(self.n[p].x, self.n[p].y);
            }
            self.n[p].prev_z = self.n[p].prev;
            self.n[p].next_z = self.n[p].next;
            p = self.n[p].next;
            if p == start {
                break;
            }
        }
        let pz = self.n[p].prev_z;
        self.n[pz].next_z = NIL;
        self.n[p].prev_z = NIL;
        self.sort_linked(p);
    }

    fn sort_linked(&mut self, list: usize) {
        let mut list = list;
        let mut in_size = 1;
        loop {
            let mut p = list;
            list = NIL;
            let mut tail = NIL;
            let mut merges = 0;
            while p != NIL {
                merges += 1;
                let mut q = p;
                let mut p_size = 0;
                for _ in 0..in_size {
                    p_size += 1;
                    q = self.n[q].next_z;
                    if q == NIL {
                        break;
                    }
                }
                let mut q_size = in_size;
                while p_size > 0 || (q_size > 0 && q != NIL) {
                    let e;
                    if p_size != 0 && (q_size == 0 || q == NIL || self.n[p].z <= self.n[q].z) {
                        e = p;
                        p = self.n[p].next_z;
                        p_size -= 1;
                    } else {
                        e = q;
                        q = self.n[q].next_z;
                        q_size -= 1;
                    }
                    if tail != NIL {
                        self.n[tail].next_z = e;
                    } else {
                        list = e;
                    }
                    self.n[e].prev_z = tail;
                    tail = e;
                }
                p = q;
            }
            if tail != NIL {
                self.n[tail].next_z = NIL;
            }
            in_size *= 2;
            if merges <= 1 {
                break;
            }
        }
    }

    fn point_in_triangle(ax: f64, ay: f64, bx: f64, by: f64, cx: f64, cy: f64, px: f64, py: f64) -> bool {
        (cx - px) * (ay - py) >= (ax - px) * (cy - py) && (ax - px) * (by - py) >= (bx - px) * (ay - py) && (bx - px) * (cy - py) >= (cx - px) * (by - py)
    }

    fn is_ear(&self, ear: usize) -> bool {
        let (a, b, c) = (self.n[ear].prev, ear, self.n[ear].next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let (ax, ay, bx, by, cx, cy) = (self.n[a].x, self.n[a].y, self.n[b].x, self.n[b].y, self.n[c].x, self.n[c].y);
        let (x0, x1) = (ax.min(bx).min(cx), ax.max(bx).max(cx));
        let (y0, y1) = (ay.min(by).min(cy), ay.max(by).max(cy));
        let mut p = self.n[c].next;
        while p != a {
            let n = &self.n[p];
            if n.x >= x0 && n.x <= x1 && n.y >= y0 && n.y <= y1 && Self::point_in_triangle(ax, ay, bx, by, cx, cy, n.x, n.y) && self.area(n.prev, p, n.next) >= 0.0 {
                return false;
            }
            p = n.next;
        }
        true
    }

    fn blocks(&self, p: usize, a: usize, c: usize, t: (f64, f64, f64, f64, f64, f64), bb: (f64, f64, f64, f64)) -> bool {
        let n = &self.n[p];
        n.x >= bb.0 && n.x <= bb.1 && n.y >= bb.2 && n.y <= bb.3 && p != a && p != c && Self::point_in_triangle(t.0, t.1, t.2, t.3, t.4, t.5, n.x, n.y) && self.area(n.prev, p, n.next) >= 0.0
    }

    fn is_ear_hashed(&self, ear: usize) -> bool {
        let (a, b, c) = (self.n[ear].prev, ear, self.n[ear].next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }
        let t = (self.n[a].x, self.n[a].y, self.n[b].x, self.n[b].y, self.n[c].x, self.n[c].y);
        let bb = (t.0.min(t.2).min(t.4), t.0.max(t.2).max(t.4), t.1.min(t.3).min(t.5), t.1.max(t.3).max(t.5));
        let min_z = self.z_order(bb.0, bb.2);
        let max_z = self.z_order(bb.1, bb.3);
        let mut p = self.n[ear].prev_z;
        let mut n = self.n[ear].next_z;
        while p != NIL && self.n[p].z >= min_z && n != NIL && self.n[n].z <= max_z {
            if self.blocks(p, a, c, t, bb) {
                return false;
            }
            p = self.n[p].prev_z;
            if self.blocks(n, a, c, t, bb) {
                return false;
            }
            n = self.n[n].next_z;
        }
        while p != NIL && self.n[p].z >= min_z {
            if self.blocks(p, a, c, t, bb) {
                return false;
            }
            p = self.n[p].prev_z;
        }
        while n != NIL && self.n[n].z <= max_z {
            if self.blocks(n, a, c, t, bb) {
                return false;
            }
            n = self.n[n].next_z;
        }
        true
    }

    fn sign(v: f64) -> i32 {
        if v > 0.0 {
            1
        } else if v < 0.0 {
            -1
        } else {
            0
        }
    }

    fn on_segment(&self, p: usize, q: usize, r: usize) -> bool {
        let (p, q, r) = (&self.n[p], &self.n[q], &self.n[r]);
        q.x <= p.x.max(r.x) && q.x >= p.x.min(r.x) && q.y <= p.y.max(r.y) && q.y >= p.y.min(r.y)
    }

    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let o1 = Self::sign(self.area(p1, q1, p2));
        let o2 = Self::sign(self.area(p1, q1, q2));
        let o3 = Self::sign(self.area(p2, q2, p1));
        let o4 = Self::sign(self.area(p2, q2, q1));
        (o1 != o2 && o3 != o4) || (o1 == 0 && self.on_segment(p1, p2, q1)) || (o2 == 0 && self.on_segment(p1, q2, q1)) || (o3 == 0 && self.on_segment(p2, p1, q2)) || (o4 == 0 && self.on_segment(p2, q1, q2))
    }

    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        loop {
            let pn = self.n[p].next;
            if self.n[p].i != self.n[a].i && self.n[pn].i != self.n[a].i && self.n[p].i != self.n[b].i && self.n[pn].i != self.n[b].i && self.intersects(p, pn, a, b) {
                return true;
            }
            p = pn;
            if p == a {
                return false;
            }
        }
    }

    fn locally_inside(&self, a: usize, b: usize) -> bool {
        let (ap, an) = (self.n[a].prev, self.n[a].next);
        if self.area(ap, a, an) < 0.0 {
            self.area(a, b, an) >= 0.0 && self.area(a, ap, b) >= 0.0
        } else {
            self.area(a, b, ap) < 0.0 || self.area(a, an, b) < 0.0
        }
    }

    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        let mut inside = false;
        let (px, py) = (0.5 * (self.n[a].x + self.n[b].x), 0.5 * (self.n[a].y + self.n[b].y));
        loop {
            let (n, m) = (&self.n[p], &self.n[self.n[p].next]);
            if (n.y > py) != (m.y > py) && m.y != n.y && px < (m.x - n.x) * (py - n.y) / (m.y - n.y) + n.x {
                inside = !inside;
            }
            p = n.next;
            if p == a {
                return inside;
            }
        }
    }

    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        let (an, ap) = (self.n[a].next, self.n[a].prev);
        let (bn, bp) = (self.n[b].next, self.n[b].prev);
        self.n[an].i != self.n[b].i
            && self.n[ap].i != self.n[b].i
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b) && self.locally_inside(b, a) && self.middle_inside(a, b) && (self.area(ap, a, bp) != 0.0 || self.area(a, bp, b) != 0.0))
                || (self.equals(a, b) && self.area(ap, a, an) > 0.0 && self.area(bp, b, bn) > 0.0))
    }

    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let a2 = self.n.len();
        let na = self.n[a];
        self.n.push(Node { prev: NIL, next: NIL, z: 0, prev_z: NIL, next_z: NIL, steiner: false, ..na });
        let b2 = self.n.len();
        let nb = self.n[b];
        self.n.push(Node { prev: NIL, next: NIL, z: 0, prev_z: NIL, next_z: NIL, steiner: false, ..nb });
        let an = self.n[a].next;
        let bp = self.n[b].prev;
        self.n[a].next = b;
        self.n[b].prev = a;
        self.n[a2].next = an;
        self.n[an].prev = a2;
        self.n[b2].next = a2;
        self.n[a2].prev = b2;
        self.n[bp].next = b2;
        self.n[b2].prev = bp;
        b2
    }

    fn cure_local_intersections(&mut self, start: usize, out: &mut Vec<usize>) -> usize {
        let mut start = start;
        let mut p = start;
        loop {
            let a = self.n[p].prev;
            let pn = self.n[p].next;
            let b = self.n[pn].next;
            if !self.equals(a, b) && self.intersects(a, p, pn, b) && self.locally_inside(a, b) && self.locally_inside(b, a) {
                out.extend_from_slice(&[self.n[a].i, self.n[p].i, self.n[b].i]);
                self.remove(p);
                self.remove(pn);
                p = b;
                start = b;
            }
            p = self.n[p].next;
            if p == start {
                break;
            }
        }
        self.filter(p, NIL)
    }

    fn split_earcut(&mut self, start: usize, out: &mut Vec<usize>, guard: u32) {
        let mut a = start;
        loop {
            let mut b = self.n[self.n[a].next].next;
            while b != self.n[a].prev {
                if self.n[a].i != self.n[b].i && self.is_valid_diagonal(a, b) {
                    let mut c = self.split_polygon(a, b);
                    let an = self.n[a].next;
                    let a2 = self.filter(a, an);
                    let cn = self.n[c].next;
                    c = self.filter(c, cn);
                    self.linked(a2, out, 0, guard + 1);
                    self.linked(c, out, 0, guard + 1);
                    return;
                }
                b = self.n[b].next;
            }
            a = self.n[a].next;
            if a == start {
                return;
            }
        }
    }

    fn linked(&mut self, ear: usize, out: &mut Vec<usize>, pass: u32, guard: u32) {
        if ear == NIL || guard > 64 {
            return;
        }
        if pass == 0 && self.inv != 0.0 {
            self.index_curve(ear);
        }
        let mut ear = ear;
        let mut stop = ear;
        while self.n[ear].prev != self.n[ear].next {
            let prev = self.n[ear].prev;
            let next = self.n[ear].next;
            let ok = if self.inv != 0.0 { self.is_ear_hashed(ear) } else { self.is_ear(ear) };
            if ok {
                out.extend_from_slice(&[self.n[prev].i, self.n[ear].i, self.n[next].i]);
                self.remove(ear);
                ear = self.n[next].next;
                stop = ear;
                continue;
            }
            ear = next;
            if ear == stop {
                if pass == 0 {
                    let f = self.filter(ear, NIL);
                    self.linked(f, out, 1, guard);
                } else if pass == 1 {
                    let f = self.filter(ear, NIL);
                    let e = self.cure_local_intersections(f, out);
                    self.linked(e, out, 2, guard);
                } else if pass == 2 {
                    self.split_earcut(ear, out, guard);
                }
                break;
            }
        }
    }

    fn leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut l = start;
        loop {
            if self.n[p].x < self.n[l].x || (self.n[p].x == self.n[l].x && self.n[p].y < self.n[l].y) {
                l = p;
            }
            p = self.n[p].next;
            if p == start {
                return l;
            }
        }
    }

    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.n[m].prev, m, self.n[p].prev) < 0.0 && self.area(self.n[p].next, m, self.n[m].next) < 0.0
    }

    fn find_hole_bridge(&self, hole: usize, outer: usize) -> usize {
        let mut p = outer;
        let (hx, hy) = (self.n[hole].x, self.n[hole].y);
        let mut qx = f64::NEG_INFINITY;
        let mut m = NIL;
        loop {
            let (n, nn) = (&self.n[p], &self.n[self.n[p].next]);
            if hy <= n.y && hy >= nn.y && nn.y != n.y {
                let x = n.x + (hy - n.y) * (nn.x - n.x) / (nn.y - n.y);
                if x <= hx && x > qx {
                    qx = x;
                    m = if n.x < nn.x { p } else { n.next };
                    if x == hx {
                        return m;
                    }
                }
            }
            p = n.next;
            if p == outer {
                break;
            }
        }
        if m == NIL {
            return NIL;
        }
        let stop = m;
        let (mx, my) = (self.n[m].x, self.n[m].y);
        let mut tan_min = f64::INFINITY;
        p = m;
        loop {
            let (px, py) = (self.n[p].x, self.n[p].y);
            if hx >= px && px >= mx && hx != px && Self::point_in_triangle(if hy < my { hx } else { qx }, hy, mx, my, if hy < my { qx } else { hx }, hy, px, py) {
                let tan = (hy - py).abs() / (hx - px);
                if self.locally_inside(p, hole) && (tan < tan_min || (tan == tan_min && (px > self.n[m].x || (px == self.n[m].x && self.sector_contains_sector(m, p))))) {
                    m = p;
                    tan_min = tan;
                }
            }
            p = self.n[p].next;
            if p == stop {
                break;
            }
        }
        m
    }

    fn eliminate_holes(&mut self, data: &[[f64; 2]], starts: &[usize], outer: usize) -> usize {
        let mut queue = Vec::new();
        for (k, &s) in starts.iter().enumerate() {
            let e = if k + 1 < starts.len() { starts[k + 1] } else { data.len() };
            let list = self.linked_list(data, s, e, false);
            if list == NIL {
                continue;
            }
            if list == self.n[list].next {
                self.n[list].steiner = true;
            }
            queue.push(self.leftmost(list));
        }
        queue.sort_by(|&a, &b| self.n[a].x.partial_cmp(&self.n[b].x).unwrap_or(core::cmp::Ordering::Equal));
        let mut outer = outer;
        for h in queue {
            let bridge = self.find_hole_bridge(h, outer);
            if bridge == NIL {
                continue;
            }
            let rev = self.split_polygon(bridge, h);
            let rn = self.n[rev].next;
            self.filter(rev, rn);
            let bn = self.n[bridge].next;
            outer = self.filter(bridge, bn);
        }
        outer
    }
}

/// triangles of the polygon `data` (the outer ring, then the holes from `starts` on) as indices into it
pub fn earcut(data: &[[f64; 2]], starts: &[usize], out: &mut Vec<usize>) {
    let outer_len = starts.first().copied().unwrap_or(data.len());
    let mut e = Ear { n: Vec::with_capacity(data.len() * 3 / 2 + 8), min_x: 0.0, min_y: 0.0, inv: 0.0 };
    let mut outer = e.linked_list(data, 0, outer_len, true);
    if outer == NIL || e.n[outer].next == e.n[outer].prev {
        return;
    }
    if !starts.is_empty() {
        outer = e.eliminate_holes(data, starts, outer);
    }
    if data.len() > 80 {
        let b = bbox(&data[..outer_len]);
        e.min_x = b[0];
        e.min_y = b[1];
        let s = (b[2] - b[0]).max(b[3] - b[1]);
        e.inv = if s != 0.0 { 32767.0 / s } else { 0.0 };
    }
    e.linked(outer, out, 0, 0);
}

/// (tests) the triangles' area against the rings' even-odd area, both in units squared
pub fn areas(p: &Prepared) -> (f64, f64) {
    let pt = |(r, k): (u32, u32)| p.rings[r as usize][k as usize];
    let mut t = 0.0;
    for tri in &p.tris {
        let (a, b, c) = (pt(tri[0]), pt(tri[1]), pt(tri[2]));
        t += 0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs();
    }
    // (oriented: holes count negative)
    let r: f64 = p.rings.iter().map(|r| signed_area(r)).sum();
    (t, r)
}
