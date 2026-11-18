// mesh -- a static triangle mesh: vertices in f32 (as render3d's meshes), a fixed BVH, the active edges,
// and the queries against it in its own frame
//
// Seams: every triangle is tested as the closed triangle it is, so a shape sweeping across a shared edge
// meets the edge of one triangle or the other, there is no gap.  The opposite trouble -- a shape sliding over
// a flat or concave edge between two triangles hits the edge itself with a normal that leans backwards
// (a "ghost" collision) -- is handled by the active edges: an edge is active when the surface bends
// outwards there by more than the threshold angle (or the edge is open or shared by more than two
// triangles).  A contact on an inactive edge (or on a vertex whose two edges of this triangle are inactive)
// gets the triangle's normal, and a cast ignores it unless the shape moves into that face.  As in Jolt's
// "active edges"; the side matters, an edge that is concave from the front is convex from the back.

use crate::bvh::{self, BvhNode, STACK};
use crate::convex::{Core, Placed};
use crate::gjk::{self, Cast, Sep};
use crate::math::{Aabb, Vec3, inverse_dir, max, ray_box};
use crate::ray;
use alloc::vec;
use alloc::vec::Vec;

/// at most this many triangles in one mesh
pub const MAX_TRIANGLES: usize = 1 << 28;

/// cos 5 degrees: two triangles whose normals differ by less than this meet at a flat edge
pub const ACTIVE_EDGE_COS: f64 = 0.996_194_698_091_745_5;

/// a barycentric weight up to this counts as 0: the point is on the edge or vertex
const BARY_EPS: f64 = 1e-9;

/// a cast counts as moving into a surface when normal . motion < -APPROACH * |motion|
pub(crate) const APPROACH: f64 = 1e-12;

/// why [`TriMesh::new`] refused the data
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshError {
    /// the index count is not a multiple of 3, or there are more than [`MAX_TRIANGLES`] triangles
    IndexCount,
    /// an index is not below the vertex count
    IndexRange,
    /// a coordinate is NaN or infinite
    NotFinite,
}

/// how a [`TriMesh`] is built
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshOptions {
    /// an edge between two triangles whose normals have a cosine of at least this is flat (inactive); above
    /// 1 every edge is active (no handling of ghost collisions)
    pub active_edge_cos: f64,
    /// at most so many triangles in a leaf of the tree
    pub max_leaf: usize,
}

impl Default for MeshOptions {
    fn default() -> MeshOptions {
        MeshOptions { active_edge_cos: ACTIVE_EDGE_COS, max_leaf: 4 }
    }
}

/// A static triangle mesh with its tree.  Triangles have two sides.  Triangles without area (all three
/// corners on a line) are left out: they cannot be hit and their edges belong to their neighbours.
#[derive(Clone, Debug)]
pub struct TriMesh {
    vertices: Vec<[f32; 3]>,
    /// the triangles in the order of the tree's leaves
    tris: Vec<[u32; 3]>,
    /// per stored triangle its index in the caller's index list (triangle i = indices[3 i .. 3 i + 3])
    ids: Vec<u32>,
    /// per stored triangle: bit e = edge e (v[e] -> v[e + 1]) is active seen from the front (the side of
    /// (b - a) x (c - a)), bit 3 + e = seen from the back
    flags: Vec<u8>,
    nodes: Vec<BvhNode>,
    aabb: Aabb,
    input_triangles: u32,
}

/// a hit on a mesh in its own frame
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalHit {
    pub t: f64,
    pub point: Vec3,
    pub normal: Vec3,
    pub tri: u32,
    pub depth: f64,
}

#[inline]
fn v3(p: &[f32; 3]) -> Vec3 {
    Vec3::new(p[0] as f64, p[1] as f64, p[2] as f64)
}

/// the distance between two boxes (0 when they touch or overlap)
fn box_gap(a: &Aabb, b: &Aabb) -> f64 {
    let dx = max(0.0, max(a.min.x - b.max.x, b.min.x - a.max.x));
    let dy = max(0.0, max(a.min.y - b.max.y, b.min.y - a.max.y));
    let dz = max(0.0, max(a.min.z - b.max.z, b.min.z - a.max.z));
    crate::math::sqrt(dx * dx + dy * dy + dz * dz)
}

impl TriMesh {
    /// a mesh from positions and a triangle list (three indices per triangle), with [`MeshOptions`]'
    /// defaults
    pub fn new(positions: &[[f32; 3]], indices: &[u32]) -> Result<TriMesh, MeshError> {
        TriMesh::with_options(positions, indices, &MeshOptions::default())
    }

    /// a mesh as [`TriMesh::new`] makes it, with the threshold of the active edges and the leaf size of `opt`
    pub fn with_options(positions: &[[f32; 3]], indices: &[u32], opt: &MeshOptions) -> Result<TriMesh, MeshError> {
        if indices.len() % 3 != 0 || indices.len() / 3 > MAX_TRIANGLES {
            return Err(MeshError::IndexCount);
        }
        if positions.iter().any(|p| !(p[0].is_finite() && p[1].is_finite() && p[2].is_finite())) {
            return Err(MeshError::NotFinite);
        }
        if indices.iter().any(|&i| i as usize >= positions.len()) {
            return Err(MeshError::IndexRange);
        }
        let n_in = indices.len() / 3;
        let mut kept: Vec<[u32; 3]> = Vec::with_capacity(n_in);
        let mut kept_ids: Vec<u32> = Vec::with_capacity(n_in);
        let mut boxes: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(n_in);
        for t in 0..n_in {
            let v = [indices[3 * t], indices[3 * t + 1], indices[3 * t + 2]];
            let (a, b, c) = (v3(&positions[v[0] as usize]), v3(&positions[v[1] as usize]), v3(&positions[v[2] as usize]));
            if (b - a).cross(c - a) == Vec3::ZERO {
                continue;
            }
            let mut lo = positions[v[0] as usize];
            let mut hi = lo;
            for &i in &v[1..] {
                let p = positions[i as usize];
                for k in 0..3 {
                    if p[k] < lo[k] {
                        lo[k] = p[k];
                    }
                    if p[k] > hi[k] {
                        hi[k] = p[k];
                    }
                }
            }
            kept.push(v);
            kept_ids.push(t as u32);
            boxes.push((lo, hi));
        }
        let (nodes, order) = bvh::build(&boxes, opt.max_leaf.clamp(1, 16));
        let tris: Vec<[u32; 3]> = order.iter().map(|&k| kept[k as usize]).collect();
        let ids: Vec<u32> = order.iter().map(|&k| kept_ids[k as usize]).collect();
        let aabb = match nodes.first() {
            Some(n) => n.aabb(),
            None => Aabb::EMPTY,
        };
        let mut m = TriMesh { vertices: positions.to_vec(), tris, ids, flags: Vec::new(), nodes, aabb, input_triangles: n_in as u32 };
        m.flags = m.active_edges(opt.active_edge_cos);
        Ok(m)
    }

    /// the edge flags (see `flags`)
    fn active_edges(&self, cos_flat: f64) -> Vec<u8> {
        let nv = self.vertices.len();
        // weld the vertices by position (-0 = +0): the canonical id is the first index of a position
        let key = |i: u32| {
            let p = self.vertices[i as usize];
            ((p[0] + 0.0).to_bits(), (p[1] + 0.0).to_bits(), (p[2] + 0.0).to_bits())
        };
        let mut by_pos: Vec<u32> = (0..nv as u32).collect();
        by_pos.sort_unstable_by(|&a, &b| key(a).cmp(&key(b)).then(a.cmp(&b)));
        let mut canon = vec![0u32; nv];
        let mut k = 0;
        while k < by_pos.len() {
            let first = by_pos[k];
            let kk = key(first);
            let mut j = k;
            while j < by_pos.len() && key(by_pos[j]) == kk {
                canon[by_pos[j] as usize] = first;
                j += 1;
            }
            k = j;
        }
        // the edges, each with its triangle and its place in it, sorted by the welded pair
        let mut edges: Vec<(u32, u32, u32, u8)> = Vec::with_capacity(self.tris.len() * 3);
        for (t, v) in self.tris.iter().enumerate() {
            for e in 0..3 {
                let (p, q) = (canon[v[e] as usize], canon[v[(e + 1) % 3] as usize]);
                edges.push((p.min(q), p.max(q), t as u32, e as u8));
            }
        }
        edges.sort_unstable();
        let mut flags = vec![0u8; self.tris.len()];
        let mut i = 0;
        while i < edges.len() {
            let mut j = i + 1;
            while j < edges.len() && edges[j].0 == edges[i].0 && edges[j].1 == edges[i].1 {
                j += 1;
            }
            if j - i == 2 {
                let (t1, e1) = (edges[i].2 as usize, edges[i].3 as usize);
                let (t2, e2) = (edges[i + 1].2 as usize, edges[i + 1].3 as usize);
                let f1 = self.edge_sides(t1, e1, t2, e2, &canon, cos_flat);
                let f2 = self.edge_sides(t2, e2, t1, e1, &canon, cos_flat);
                flags[t1] |= f1;
                flags[t2] |= f2;
            } else {
                // open or shared by more than two: active on both sides
                for &(_, _, t, e) in &edges[i..j] {
                    flags[t as usize] |= (1 << e) | (8 << e);
                }
            }
            i = j;
        }
        flags
    }

    /// the flag bits of edge e1 of triangle t1 against its neighbour t2 (edge e2)
    fn edge_sides(&self, t1: usize, e1: usize, t2: usize, e2: usize, canon: &[u32], cos_flat: f64) -> u8 {
        let both = (1u8 << e1) | (8u8 << e1);
        if cos_flat > 1.0 {
            return both; // every edge active: no handling of ghost collisions
        }
        let (v1, v2) = (self.tris[t1], self.tris[t2]);
        // a consistent winding runs the shared edge the other way round in the neighbour
        if canon[v1[e1] as usize] != canon[v2[(e2 + 1) % 3] as usize] {
            return both;
        }
        let [a, b, c] = self.tri(t1);
        let [p, q, r] = self.tri(t2);
        let (Some(n1), Some(n2)) = ((b - a).cross(c - a).try_normalize(), (q - p).cross(r - p).try_normalize()) else {
            return both;
        };
        if n1.dot(n2) >= cos_flat {
            return 0; // flat: inactive on both sides
        }
        let opposite = self.tri(t2)[(e2 + 2) % 3];
        let edge_start = self.tri(t1)[e1];
        let h = n1.dot(opposite - edge_start);
        if h < 0.0 {
            1u8 << e1 // convex seen from the front
        } else {
            8u8 << e1 // concave from the front: convex from the back
        }
    }

    /// stored triangle t's corners in the mesh's frame
    #[inline]
    pub(crate) fn tri(&self, t: usize) -> [Vec3; 3] {
        let v = self.tris[t];
        [v3(&self.vertices[v[0] as usize]), v3(&self.vertices[v[1] as usize]), v3(&self.vertices[v[2] as usize])]
    }

    /// the number of triangles in the caller's list (with the ones left out for having no area)
    pub fn triangle_count(&self) -> usize {
        self.input_triangles as usize
    }

    /// the number of triangles in the tree
    pub fn stored_triangle_count(&self) -> usize {
        self.tris.len()
    }

    /// the positions as given, in the mesh's frame
    pub fn vertices(&self) -> &[[f32; 3]] {
        &self.vertices
    }

    /// the box around the mesh in its own frame
    pub fn aabb(&self) -> Aabb {
        self.aabb
    }

    /// the tree's nodes (the root first)
    pub fn nodes(&self) -> &[BvhNode] {
        &self.nodes
    }

    /// the depth of the tree
    pub fn depth(&self) -> u32 {
        bvh::depth(&self.nodes)
    }

    /// the stored triangles in tree order: their caller's index and corners (for brute-force references)
    pub fn triangles(&self) -> impl Iterator<Item = (u32, [Vec3; 3])> + '_ {
        (0..self.tris.len()).map(|t| (self.ids[t], self.tri(t)))
    }

    /// the active-edge bits of the triangle with the caller's index `id` (bit e: edge e from the front,
    /// bit 3 + e: from the back), None when it was left out
    pub fn edge_flags(&self, id: u32) -> Option<u8> {
        self.ids.iter().position(|&i| i == id).map(|t| self.flags[t])
    }

    /// the bytes the mesh holds (vertices, triangles, ids, flags, nodes)
    pub fn memory_bytes(&self) -> usize {
        self.vertices.capacity() * 12
            + self.tris.capacity() * 12
            + self.ids.capacity() * 4
            + self.flags.capacity()
            + self.nodes.capacity() * core::mem::size_of::<BvhNode>()
            + core::mem::size_of::<TriMesh>()
    }

    // ------------------------------------------------------------ the active edges at work

    #[inline]
    fn active(&self, t: usize, e: usize, front: bool) -> bool {
        self.flags[t] & (if front { 1u8 } else { 8u8 }) << e != 0
    }

    /// the normal of a contact at `p` on stored triangle t, `n` from the triangle towards the shape: the
    /// triangle's own normal unless the contact is on an active edge or vertex
    pub(crate) fn fix_normal(&self, t: usize, tri: &[Vec3; 3], p: Vec3, n: Vec3) -> Vec3 {
        let Some(face) = (tri[1] - tri[0]).cross(tri[2] - tri[0]).try_normalize() else {
            return n;
        };
        let front = face.dot(n) >= 0.0;
        let fnorm = if front { face } else { -face };
        let bc = gjk::tri_bary(tri[0], tri[1], tri[2], p);
        let zero = [bc[0] <= BARY_EPS, bc[1] <= BARY_EPS, bc[2] <= BARY_EPS];
        match zero.iter().filter(|z| **z).count() {
            0 => fnorm,
            1 => {
                let i = zero.iter().position(|z| *z).unwrap_or(0);
                if self.active(t, (i + 1) % 3, front) { n } else { fnorm }
            }
            2 => {
                let j = zero.iter().position(|z| !*z).unwrap_or(0);
                if self.active(t, j, front) || self.active(t, (j + 2) % 3, front) { n } else { fnorm }
            }
            _ => n,
        }
    }

    // ------------------------------------------------------------ traversals

    /// visit the leaves whose box, grown by `ext`, the ray o + t d enters before `tmax`, nearest first;
    /// `leaf(first, count, tmax)` returns the new tmax.  A box entered exactly at tmax is still visited (ties
    /// between hits are broken by the triangle's index).
    fn walk_ray(&self, brute: bool, o: Vec3, d: Vec3, ext: Vec3, mut tmax: f64, mut leaf: impl FnMut(usize, usize, f64) -> f64) {
        if self.nodes.is_empty() {
            return;
        }
        if brute {
            leaf(0, self.tris.len(), tmax);
            return;
        }
        let inv = inverse_dir(d);
        let enter = |n: &BvhNode, tmax: f64| ray_box(o, inv, Vec3::from(n.min) - ext, Vec3::from(n.max) + ext, tmax);
        let Some(t0) = enter(&self.nodes[0], tmax) else {
            return;
        };
        let mut stack = [(0u32, 0.0f64); STACK];
        stack[0] = (0, t0);
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let (ni, te) = stack[sp];
            if te > tmax {
                continue;
            }
            let n = &self.nodes[ni as usize];
            if n.count > 0 {
                tmax = leaf(n.first as usize, n.count as usize, tmax);
                continue;
            }
            let (l, r) = (n.first, n.first + 1);
            let tl = enter(&self.nodes[l as usize], tmax);
            let tr = enter(&self.nodes[r as usize], tmax);
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    // the nearer one on top
                    let ((fi, ft), (ni2, nt)) = if b < a { ((l, a), (r, b)) } else { ((r, b), (l, a)) };
                    stack[sp] = (fi, ft);
                    stack[sp + 1] = (ni2, nt);
                    sp += 2;
                }
                (Some(a), None) => {
                    stack[sp] = (l, a);
                    sp += 1;
                }
                (None, Some(b)) => {
                    stack[sp] = (r, b);
                    sp += 1;
                }
                (None, None) => {}
            }
        }
    }

    /// visit the leaves whose box touches `bx`
    fn walk_box(&self, brute: bool, bx: &Aabb, mut leaf: impl FnMut(usize, usize)) {
        if self.nodes.is_empty() {
            return;
        }
        if brute {
            leaf(0, self.tris.len());
            return;
        }
        let mut stack = [0u32; STACK];
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let n = &self.nodes[stack[sp] as usize];
            if !n.aabb().intersects(bx) {
                continue;
            }
            if n.count > 0 {
                leaf(n.first as usize, n.count as usize);
            } else {
                stack[sp] = n.first + 1;
                stack[sp + 1] = n.first;
                sp += 2;
            }
        }
    }

    /// visit the leaves nearer than `best` to `bx` (box distance), nearer subtrees first; `leaf` returns
    /// the new best distance
    fn walk_near(&self, brute: bool, bx: &Aabb, mut best: f64, mut leaf: impl FnMut(usize, usize, f64) -> f64) {
        if self.nodes.is_empty() {
            return;
        }
        if brute {
            leaf(0, self.tris.len(), best);
            return;
        }
        let mut stack = [(0u32, 0.0f64); STACK];
        stack[0] = (0, box_gap(&self.nodes[0].aabb(), bx));
        let mut sp = 1;
        while sp > 0 {
            sp -= 1;
            let (ni, gap) = stack[sp];
            if gap > best {
                continue;
            }
            let n = &self.nodes[ni as usize];
            if n.count > 0 {
                best = leaf(n.first as usize, n.count as usize, best);
                continue;
            }
            let (l, r) = (n.first, n.first + 1);
            let gl = box_gap(&self.nodes[l as usize].aabb(), bx);
            let gr = box_gap(&self.nodes[r as usize].aabb(), bx);
            if gr < gl {
                stack[sp] = (l, gl);
                stack[sp + 1] = (r, gr);
            } else {
                stack[sp] = (r, gr);
                stack[sp + 1] = (l, gl);
            }
            sp += 2;
        }
    }

    // ------------------------------------------------------------ queries in the mesh's frame

    /// the first triangle the ray o + t d hits (t <= tmax; equal t: the lower caller's index)
    pub(crate) fn ray(&self, brute: bool, o: Vec3, d: Vec3, tmax: f64) -> Option<LocalHit> {
        let mut best: Option<LocalHit> = None;
        self.walk_ray(brute, o, d, Vec3::ZERO, tmax, |first, count, tmax| {
            let mut tm = tmax;
            for t in first..first + count {
                let [a, b, c] = self.tri(t);
                if let Some((th, _)) = ray::triangle(o, d, a, b, c, tm) {
                    let id = self.ids[t];
                    if best.is_none_or(|h| th < h.t || (th == h.t && id < h.tri)) {
                        best = Some(LocalHit { t: th, point: o + d * th, normal: ray::tri_normal_against(a, b, c, d), tri: id, depth: 0.0 });
                        tm = th;
                    }
                }
            }
            tm
        });
        best
    }

    /// every triangle the ray hits (t <= tmax), in no particular order
    pub(crate) fn ray_all(&self, brute: bool, o: Vec3, d: Vec3, tmax: f64, out: &mut Vec<LocalHit>) {
        self.walk_ray(brute, o, d, Vec3::ZERO, tmax, |first, count, tmax| {
            for t in first..first + count {
                let [a, b, c] = self.tri(t);
                if let Some((th, _)) = ray::triangle(o, d, a, b, c, tmax) {
                    out.push(LocalHit { t: th, point: o + d * th, normal: ray::tri_normal_against(a, b, c, d), tri: self.ids[t], depth: 0.0 });
                }
            }
            tmax
        });
    }

    /// the cast of convex `a` (in the mesh's frame) by r against one stored triangle -> (lambda, normal,
    /// point on the triangle, depth) when it moves into it
    pub(crate) fn cast_triangle(&self, a: &Placed, r: Vec3, t: usize) -> Option<(f64, Vec3, Vec3, f64)> {
        let tri = self.tri(t);
        let b = Placed::new(Core::Triangle(tri), 0.0, None, Vec3::ZERO);
        let rl = r.length();
        match gjk::cast(a, &b, r) {
            Cast::Miss => None,
            Cast::Start => {
                let s = gjk::separation(a, &b, a.center() - b.center());
                let n = self.fix_normal(t, &tri, s.pb, s.normal);
                if n.dot(r) < -APPROACH * rl { Some((0.0, n, s.pb, max(0.0, -s.dist))) } else { None }
            }
            Cast::Hit { lambda, normal, pb } => {
                let n = self.fix_normal(t, &tri, pb, normal);
                if n.dot(r) < -APPROACH * rl { Some((lambda, n, pb, 0.0)) } else { None }
            }
        }
    }

    /// the first triangle convex `a` (in the mesh's frame) meets when it moves by r
    pub(crate) fn cast(&self, brute: bool, a: &Placed, r: Vec3) -> Option<LocalHit> {
        let bx = a.aabb();
        let c = bx.center();
        let pad = gjk::CAST_SKIN + 1e-9 + 1e-12 * max(c.abs().max_elem(), bx.half_extents().max_elem());
        let ext = bx.half_extents() + Vec3::splat(pad);
        let mut best: Option<LocalHit> = None;
        self.walk_ray(brute, c, r, ext, 1.0, |first, count, tmax| {
            let mut tm = tmax;
            for t in first..first + count {
                if let Some((lambda, n, p, depth)) = self.cast_triangle(a, r, t) {
                    let id = self.ids[t];
                    if best.is_none_or(|h| lambda < h.t || (lambda == h.t && id < h.tri)) {
                        best = Some(LocalHit { t: lambda, point: p, normal: n, tri: id, depth });
                        tm = lambda;
                    }
                }
            }
            tm
        });
        best
    }

    /// the contact of convex `a` with one stored triangle if their gap is at most `margin`: (normal from the
    /// triangle, gap along it (negative: depth), point on the triangle)
    pub(crate) fn contact_triangle(&self, a: &Placed, t: usize, margin: f64) -> Option<(Vec3, f64, Vec3)> {
        let tri = self.tri(t);
        let b = Placed::new(Core::Triangle(tri), 0.0, None, Vec3::ZERO);
        let s = gjk::separation(a, &b, a.center() - b.center());
        if s.dist > margin {
            return None;
        }
        // on an inactive edge the triangle's normal, with the true distance (the depth over the plane would
        // count the bend of a slightly convex flat edge as penetration)
        let n = self.fix_normal(t, &tri, s.pb, s.normal);
        Some((n, s.dist, s.pb))
    }

    /// every triangle within `margin` of convex `a` (in the mesh's frame)
    pub(crate) fn contacts(&self, brute: bool, a: &Placed, margin: f64, out: &mut Vec<LocalHit>) {
        let bx = a.aabb().expanded(max(margin, 0.0));
        self.walk_box(brute, &bx, |first, count| {
            for t in first..first + count {
                if let Some((n, gap, p)) = self.contact_triangle(a, t, margin) {
                    out.push(LocalHit { t: gap, point: p, normal: n, tri: self.ids[t], depth: max(0.0, -gap) });
                }
            }
        });
    }

    /// does convex `a` touch or overlap a triangle
    pub(crate) fn intersects(&self, brute: bool, a: &Placed) -> bool {
        let bx = a.aabb();
        let mut hit = false;
        self.walk_box(brute, &bx, |first, count| {
            if hit {
                return;
            }
            for t in first..first + count {
                let b = Placed::new(Core::Triangle(self.tri(t)), 0.0, None, Vec3::ZERO);
                let g = gjk::distance(a, &b);
                if g.overlap || g.dist <= a.margin {
                    hit = true;
                    return;
                }
            }
        });
        hit
    }

    /// the triangle nearest to convex `a` within `max_dist` (equal distance: the lower caller's index)
    pub(crate) fn separation(&self, brute: bool, a: &Placed, max_dist: f64) -> Option<(Sep, u32)> {
        let bx = a.aabb();
        let mut best: Option<(Sep, u32)> = None;
        self.walk_near(brute, &bx, max_dist, |first, count, best_d| {
            let mut bd = best_d;
            for t in first..first + count {
                let tri = self.tri(t);
                let b = Placed::new(Core::Triangle(tri), 0.0, None, Vec3::ZERO);
                let s = gjk::separation(a, &b, a.center() - b.center());
                let id = self.ids[t];
                if s.dist <= bd && best.as_ref().is_none_or(|(o, oi)| s.dist < o.dist || (s.dist == o.dist && id < *oi)) {
                    best = Some((s, id));
                    bd = max(s.dist, 0.0);
                }
            }
            bd
        });
        best
    }

    /// the point of the mesh nearest to p within max_dist: (point, distance, caller's triangle index)
    pub(crate) fn closest_point(&self, brute: bool, p: Vec3, max_dist: f64) -> Option<(Vec3, f64, u32)> {
        let bx = Aabb::new(p, p);
        let mut best: Option<(Vec3, f64, u32)> = None;
        self.walk_near(brute, &bx, max_dist, |first, count, best_d| {
            let mut bd = best_d;
            for t in first..first + count {
                let [a, b, c] = self.tri(t);
                let w = gjk::tri_bary(a, b, c, p);
                let q = a * w[0] + b * w[1] + c * w[2];
                let d = (q - p).length();
                let id = self.ids[t];
                if d <= bd && best.is_none_or(|(_, od, oi)| d < od || (d == od && id < oi)) {
                    best = Some((q, d, id));
                    bd = d;
                }
            }
            bd
        });
        best
    }
}
