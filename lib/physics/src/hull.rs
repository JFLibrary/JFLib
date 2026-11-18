// hull -- a convex hull from points (quickhull in 3D) and its support function
//
// Quickhull: a first tetrahedron from extreme points, every other point in the outside set of the face it
// lies farthest above; then, as long as a face has an outside set, its farthest point (the eye) removes the
// faces it sees (a connected region from that face), the horizon around them gets new faces to the eye,
// and the points of the removed faces go to the new ones (or are inside now).  A point within `eps` of a
// face's plane counts as on it, so the hull can leave out points that lie that close to it.  Everything in
// a fixed order (faces and points by index, ties to the lower index): the same hull on every target.

use crate::math::{Aabb, Vec3, max};
use alloc::vec;
use alloc::vec::Vec;

/// at most this many input points for [`ConvexHull::new`]
pub const MAX_HULL_POINTS: usize = 1 << 16;

/// why [`ConvexHull::new`] refused the points
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HullError {
    /// fewer than 4 points, or more than [`MAX_HULL_POINTS`]
    Count,
    /// a coordinate is NaN or infinite
    NotFinite,
    /// all points lie on a plane (or a line or a point), within the tolerance: no volume
    Flat,
}

/// A convex polyhedron: the hull of the points it was made from.  Vertices, triangles (counter-clockwise
/// seen from outside) and their planes, in its own coordinates.
#[derive(Clone, Debug)]
pub struct ConvexHull {
    points: Vec<Vec3>,
    faces: Vec<[u32; 3]>,
    /// per face the outward unit normal and its distance from the origin (n . p = d on the face)
    planes: Vec<(Vec3, f64)>,
    /// vertex adjacency (CSR): the neighbours of vertex i are adj[adj_start[i]..adj_start[i + 1]]
    adj_start: Vec<u32>,
    adj: Vec<u32>,
    aabb: Aabb,
    /// the tolerance the hull was built with: no input point lies farther than this outside it
    eps: f64,
}

struct Face {
    v: [u32; 3],
    n: Vec3,
    d: f64,
    adj: [u32; 3], // the face across edge i (v[i] -> v[i + 1])
    outside: Vec<u32>,
    alive: bool,
}

const NONE: u32 = u32::MAX;

fn plane(p: &[Vec3], v: [u32; 3]) -> (Vec3, f64) {
    let (a, b, c) = (p[v[0] as usize], p[v[1] as usize], p[v[2] as usize]);
    let n = (b - a).cross(c - a).normalize_or_zero();
    // the distance from the centroid: the same for all three vertex orders
    (n, n.dot((a + b + c) * (1.0 / 3.0)))
}

impl ConvexHull {
    /// the convex hull of `points` (at least 4, not all on a plane)
    pub fn new(points: &[Vec3]) -> Result<ConvexHull, HullError> {
        if points.len() < 4 || points.len() > MAX_HULL_POINTS {
            return Err(HullError::Count);
        }
        if points.iter().any(|p| !p.is_finite()) {
            return Err(HullError::NotFinite);
        }
        let mut bounds = Aabb::EMPTY;
        let mut sum_abs = 0.0f64;
        for p in points {
            bounds = bounds.add_point(*p);
        }
        let ext = bounds.max - bounds.min;
        sum_abs += max(crate::math::abs(bounds.min.x), crate::math::abs(bounds.max.x));
        sum_abs += max(crate::math::abs(bounds.min.y), crate::math::abs(bounds.max.y));
        sum_abs += max(crate::math::abs(bounds.min.z), crate::math::abs(bounds.max.z));
        // within this of a plane is on it: a little above the rounding of the plane tests, and a fixed
        // fraction of the size so that nearly coplanar points do not make slivers
        let eps = 3.0 * f64::EPSILON * sum_abs + 1e-10 * ext.max_elem();
        let p = points;

        // the first tetrahedron: the farthest pair of the six extreme points, the point farthest from
        // their line, the point farthest from that plane
        let mut ext_idx = [0usize; 6];
        for (i, q) in p.iter().enumerate() {
            for a in 0..3 {
                if q[a] < p[ext_idx[a * 2]][a] {
                    ext_idx[a * 2] = i;
                }
                if q[a] > p[ext_idx[a * 2 + 1]][a] {
                    ext_idx[a * 2 + 1] = i;
                }
            }
        }
        let (mut i0, mut i1, mut best) = (0usize, 0usize, -1.0f64);
        for a in 0..6 {
            for b in a + 1..6 {
                let d = (p[ext_idx[a]] - p[ext_idx[b]]).length_sq();
                if d > best {
                    best = d;
                    i0 = ext_idx[a];
                    i1 = ext_idx[b];
                }
            }
        }
        if best <= eps * eps {
            return Err(HullError::Flat);
        }
        let dir = (p[i1] - p[i0]).normalize_or_zero();
        let (mut i2, mut best) = (0usize, -1.0f64);
        for (i, q) in p.iter().enumerate() {
            let v = *q - p[i0];
            let d = (v - dir * v.dot(dir)).length_sq();
            if d > best {
                best = d;
                i2 = i;
            }
        }
        if best <= eps * eps {
            return Err(HullError::Flat);
        }
        let n = (p[i1] - p[i0]).cross(p[i2] - p[i0]).normalize_or_zero();
        let (mut i3, mut best) = (0usize, -1.0f64);
        for (i, q) in p.iter().enumerate() {
            let d = crate::math::abs(n.dot(*q - p[i0]));
            if d > best {
                best = d;
                i3 = i;
            }
        }
        if best <= eps {
            return Err(HullError::Flat);
        }
        // orient: the fourth point below the first face
        let (a, b, c, d) = if n.dot(p[i3] - p[i0]) < 0.0 {
            (i0 as u32, i1 as u32, i2 as u32, i3 as u32)
        } else {
            (i0 as u32, i2 as u32, i1 as u32, i3 as u32)
        };
        let mut faces: Vec<Face> = Vec::new();
        // faces abc, adb, bdc, cda: each edge shared once in each direction
        let tet = [[a, b, c], [a, d, b], [b, d, c], [c, d, a]];
        for v in tet {
            let (n, dd) = plane(p, v);
            faces.push(Face { v, n, d: dd, adj: [NONE; 3], outside: Vec::new(), alive: true });
        }
        link_all(&mut faces);

        // the outside sets
        let base = [i0, i1, i2, i3];
        for (i, q) in p.iter().enumerate() {
            if base.contains(&i) {
                continue;
            }
            assign(&mut faces, &[0, 1, 2, 3], i as u32, *q, eps);
        }

        let mut visible: Vec<u32> = Vec::new();
        let mut horizon: Vec<(u32, u32, u32)> = Vec::new(); // (from, to, the face beyond)
        let mut stack: Vec<u32> = Vec::new();
        let mut mark: Vec<u32> = vec![0; faces.len()];
        let mut stamp = 0u32;
        let mut cursor = 0usize;
        loop {
            // the first face (from the cursor on) with an outside set
            while cursor < faces.len() && (!faces[cursor].alive || faces[cursor].outside.is_empty()) {
                cursor += 1;
            }
            if cursor == faces.len() {
                break;
            }
            let f0 = cursor;
            let (mut eye, mut far) = (NONE, -1.0f64);
            for &q in &faces[f0].outside {
                let d = faces[f0].n.dot(p[q as usize]) - faces[f0].d;
                if d > far || (d == far && q < eye) {
                    far = d;
                    eye = q;
                }
            }
            let e = p[eye as usize];
            // the faces the eye sees: a connected region from f0
            stamp += 1;
            mark.resize(faces.len(), 0);
            visible.clear();
            stack.clear();
            stack.push(f0 as u32);
            mark[f0] = stamp;
            while let Some(f) = stack.pop() {
                visible.push(f);
                for k in 0..3 {
                    let g = faces[f as usize].adj[k];
                    if mark[g as usize] == stamp {
                        continue;
                    }
                    let fg = &faces[g as usize];
                    if fg.n.dot(e) - fg.d > eps {
                        mark[g as usize] = stamp;
                        stack.push(g);
                    }
                }
            }
            // the horizon: edges of visible faces whose neighbour is not visible
            horizon.clear();
            for &f in &visible {
                let ff = &faces[f as usize];
                for k in 0..3 {
                    let g = ff.adj[k];
                    if mark[g as usize] != stamp {
                        horizon.push((ff.v[k], ff.v[(k + 1) % 3], g));
                    }
                }
            }
            // a simple cycle: every vertex starts exactly one edge; else this point is dropped (rounding)
            let simple = horizon.len() >= 3
                && horizon.iter().enumerate().all(|(i, h)| horizon.iter().skip(i + 1).all(|o| o.0 != h.0));
            if !simple {
                let f = &mut faces[f0];
                f.outside.retain(|&q| q != eye);
                continue;
            }
            // new faces (from, to, eye)
            let first = faces.len() as u32;
            for &(u, w, g) in &horizon {
                let v = [u, w, eye];
                let (n, dd) = plane(p, v);
                let nf = faces.len() as u32;
                faces.push(Face { v, n, d: dd, adj: [g, NONE, NONE], outside: Vec::new(), alive: true });
                // the old face's edge w -> u now borders the new face
                let gf = &mut faces[g as usize];
                for k in 0..3 {
                    if gf.v[k] == w && gf.v[(k + 1) % 3] == u {
                        gf.adj[k] = nf;
                    }
                }
            }
            let count = horizon.len() as u32;
            for i in 0..count {
                let (u, w, _) = horizon[i as usize];
                // edge w -> eye borders the new face that starts at w; edge eye -> u the one that ends at u
                let next = (0..count).find(|&j| horizon[j as usize].0 == w).unwrap_or(i);
                let prev = (0..count).find(|&j| horizon[j as usize].1 == u).unwrap_or(i);
                let f = &mut faces[(first + i) as usize];
                f.adj[1] = first + next;
                f.adj[2] = first + prev;
            }
            // the points of the removed faces go to the new ones
            let mut orphans: Vec<u32> = Vec::new();
            for &f in &visible {
                let ff = &mut faces[f as usize];
                ff.alive = false;
                orphans.append(&mut ff.outside);
            }
            orphans.sort_unstable();
            let news: Vec<u32> = (first..faces.len() as u32).collect();
            for q in orphans {
                if q != eye {
                    assign(&mut faces, &news, q, p[q as usize], eps);
                }
            }
            // points only ever go to new faces (at the end), so the faces before the cursor stay empty
        }

        // the result: alive faces, their vertices renumbered in the order of the input points
        let mut used = vec![NONE; p.len()];
        let mut alive: Vec<[u32; 3]> = Vec::new();
        for f in faces.iter().filter(|f| f.alive) {
            alive.push(f.v);
            for &v in &f.v {
                used[v as usize] = 0;
            }
        }
        let mut points: Vec<Vec3> = Vec::new();
        for (i, u) in used.iter_mut().enumerate() {
            if *u == 0 {
                *u = points.len() as u32;
                points.push(p[i]);
            }
        }
        let faces: Vec<[u32; 3]> = alive.iter().map(|v| [used[v[0] as usize], used[v[1] as usize], used[v[2] as usize]]).collect();
        let planes: Vec<(Vec3, f64)> = faces.iter().map(|v| plane(&points, *v)).collect();
        // vertex adjacency from the edges of the faces
        let mut pairs: Vec<(u32, u32)> = Vec::with_capacity(faces.len() * 3);
        for v in &faces {
            for k in 0..3 {
                pairs.push((v[k], v[(k + 1) % 3]));
            }
        }
        pairs.sort_unstable();
        pairs.dedup();
        let mut adj_start = vec![0u32; points.len() + 1];
        for &(a, _) in &pairs {
            adj_start[a as usize + 1] += 1;
        }
        for i in 0..points.len() {
            adj_start[i + 1] += adj_start[i];
        }
        let adj: Vec<u32> = pairs.iter().map(|&(_, b)| b).collect();
        let mut aabb = Aabb::EMPTY;
        for q in &points {
            aabb = aabb.add_point(*q);
        }
        Ok(ConvexHull { points, faces, planes, adj_start, adj, aabb, eps })
    }

    /// the vertices (a subset of the input points, in their order)
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    /// the triangles, as indices into [`ConvexHull::points`], counter-clockwise seen from outside
    pub fn faces(&self) -> &[[u32; 3]] {
        &self.faces
    }

    /// per face the outward unit normal n and d, with n . p = d on the face
    pub fn planes(&self) -> &[(Vec3, f64)] {
        &self.planes
    }

    /// the box around the hull in its own coordinates
    pub fn aabb(&self) -> Aabb {
        self.aabb
    }

    /// the tolerance of the construction: an input point lies at most this far outside the hull
    pub fn tolerance(&self) -> f64 {
        self.eps
    }

    /// a vertex farthest in direction `d`: the first of equals among up to 32 vertices, beyond that by hill
    /// climbing over the vertex adjacency (exact for a convex polyhedron, always the same path)
    pub fn support(&self, d: Vec3) -> Vec3 {
        self.points[self.support_index(d) as usize]
    }

    pub(crate) fn support_index(&self, d: Vec3) -> u32 {
        let n = self.points.len();
        if n <= 32 {
            let (mut best, mut bi) = (f64::NEG_INFINITY, 0u32);
            for (i, q) in self.points.iter().enumerate() {
                let v = q.dot(d);
                if v > best {
                    best = v;
                    bi = i as u32;
                }
            }
            return bi;
        }
        let mut cur = 0u32;
        let mut best = self.points[0].dot(d);
        loop {
            let mut moved = false;
            let (s, e) = (self.adj_start[cur as usize] as usize, self.adj_start[cur as usize + 1] as usize);
            for &nb in &self.adj[s..e] {
                let v = self.points[nb as usize].dot(d);
                if v > best {
                    best = v;
                    cur = nb;
                    moved = true;
                }
            }
            if !moved {
                return cur;
            }
        }
    }

    /// the volume (for tests and mass properties later)
    pub fn volume(&self) -> f64 {
        let c = self.aabb.center();
        let mut v = 0.0;
        for f in &self.faces {
            let (a, b, d) = (self.points[f[0] as usize] - c, self.points[f[1] as usize] - c, self.points[f[2] as usize] - c);
            v += a.dot(b.cross(d));
        }
        v / 6.0
    }
}

/// put point `q` into the outside set of the face (among `faces`) it lies farthest above, if any
fn assign(all: &mut [Face], faces: &[u32], q: u32, pt: Vec3, eps: f64) {
    let (mut best, mut bf) = (eps, NONE);
    for &f in faces {
        let ff = &all[f as usize];
        let d = ff.n.dot(pt) - ff.d;
        if d > best {
            best = d;
            bf = f;
        }
    }
    if bf != NONE {
        all[bf as usize].outside.push(q);
    }
}

/// the adjacency of a closed set of faces by their shared edges (the first tetrahedron)
fn link_all(faces: &mut [Face]) {
    for i in 0..faces.len() {
        for k in 0..3 {
            let (u, w) = (faces[i].v[k], faces[i].v[(k + 1) % 3]);
            for j in 0..faces.len() {
                if j == i {
                    continue;
                }
                for l in 0..3 {
                    if faces[j].v[l] == w && faces[j].v[(l + 1) % 3] == u {
                        faces[i].adj[k] = j as u32;
                    }
                }
            }
        }
    }
}
