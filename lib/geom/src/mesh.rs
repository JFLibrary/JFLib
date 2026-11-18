// mesh -- the indexed triangle mesh of the crate and the operations that keep its topology: moves,
// rotations, scales, mirrors, arrays, merging, turning the faces; bounds, area, volume; the fingerprint of
// its bits.  Normals and UV are in normals.rs, the checks in check.rs.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Aabb, Affine, Axis, Vec3, mat_vec};
use crate::{Error, MAX_TRIANGLES};

/// An indexed triangle mesh in metres: one normal and one UV per position, three indices per triangle,
/// counter-clockwise seen from outside (render3d's and glTF's convention), and the topology: each vertex
/// belongs to a point of the surface (`points`).  A hard edge or a UV seam is two vertices of one point, as a
/// GPU needs it; normals and the checks follow the points, so such a split does not open the mesh, while two
/// parts that only touch (the pieces of a wall) stay two closed parts.  [`Mesh::weld`] joins the points of
/// vertices at the same place (a half and its mirror image).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    /// metres
    pub positions: Vec<Vec3>,
    /// unit vectors, one per position (zero until a normal operation ran on a raw mesh)
    pub normals: Vec<Vec3>,
    /// texture coordinates, one per position; the box projection gives metres (normals.rs)
    pub uvs: Vec<[f64; 2]>,
    /// the point of each vertex, numbered 0 .. point count without gaps
    pub points: Vec<u32>,
    /// three per triangle
    pub indices: Vec<u32>,
}

impl Mesh {
    /// an empty mesh
    pub fn new() -> Mesh {
        Mesh::default()
    }

    /// a mesh of positions and triangles, every vertex its own point, normals and UV zero (a raw mesh: the
    /// generators share vertices where the surface is connected and give it to [`Mesh::smooth`] or
    /// [`Mesh::flat`]); Err for an index count that is no multiple of 3, an index out of range or a position
    /// that is not finite
    pub fn from_triangles(positions: Vec<Vec3>, indices: Vec<u32>) -> Result<Mesh, Error> {
        let n = positions.len();
        let m = Mesh { normals: vec![Vec3::ZERO; n], uvs: vec![[0.0; 2]; n], points: (0..n as u32).collect(), positions, indices };
        m.validate()?;
        Ok(m)
    }

    /// whether the arrays fit together: as many normals, UV and points as positions, points numbered without
    /// gaps, whole triangles, indices in range, every number finite, at most [`MAX_TRIANGLES`]
    pub fn validate(&self) -> Result<(), Error> {
        let n = self.positions.len();
        if self.normals.len() != n || self.uvs.len() != n || self.points.len() != n {
            return Err(Error::Invalid("Normalen, UV, Punkte und Positionen sind verschieden viele".into()));
        }
        let k = self.point_count();
        let mut seen = vec![false; k];
        for &p in &self.points {
            seen[p as usize] = true;
        }
        if seen.iter().any(|s| !s) {
            return Err(Error::Invalid("die Punkte sind nicht lückenlos nummeriert".into()));
        }
        if self.indices.len() % 3 != 0 {
            return Err(Error::Invalid("die Zahl der Indizes ist kein Vielfaches von 3".into()));
        }
        if self.indices.len() / 3 > MAX_TRIANGLES {
            return Err(Error::TooBig(alloc::format!("mehr als {} Dreiecke", MAX_TRIANGLES)));
        }
        if let Some(i) = self.indices.iter().find(|&&i| i as usize >= n) {
            return Err(Error::Invalid(alloc::format!("Index {} liegt außerhalb der {} Ecken", i, n)));
        }
        if !self.positions.iter().all(|p| p.is_finite()) || !self.normals.iter().all(|p| p.is_finite())
            || !self.uvs.iter().all(|u| u[0].is_finite() && u[1].is_finite())
        {
            return Err(Error::Invalid("eine Zahl ist nicht endlich".into()));
        }
        Ok(())
    }

    /// the number of vertices (positions)
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// the number of points (vertices joined by the topology)
    pub fn point_count(&self) -> usize {
        self.points.iter().max().map_or(0, |&p| p as usize + 1)
    }

    /// the number of triangles
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// the three indices of triangle `t`
    #[inline]
    pub fn triangle(&self, t: usize) -> [u32; 3] {
        [self.indices[3 * t], self.indices[3 * t + 1], self.indices[3 * t + 2]]
    }

    /// the three corners of triangle `t`
    #[inline]
    pub fn corners(&self, t: usize) -> [Vec3; 3] {
        let [a, b, c] = self.triangle(t);
        [self.positions[a as usize], self.positions[b as usize], self.positions[c as usize]]
    }

    /// the box around every position, None for a mesh without positions
    pub fn bounds(&self) -> Option<Aabb> {
        let mut it = self.positions.iter();
        let mut b = Aabb::point(*it.next()?);
        for &p in it {
            b.grow(p);
        }
        Some(b)
    }

    /// the surface area in square metres: the triangles' areas summed in order
    pub fn area(&self) -> f64 {
        let mut a = 0.0;
        for t in 0..self.triangle_count() {
            let [p, q, r] = self.corners(t);
            a += (q - p).cross(r - p).length() * 0.5;
        }
        a
    }

    /// the signed volume in cubic metres (divergence theorem, relative to the middle of the bounds for
    /// accuracy): positive for a closed mesh whose faces point outward, meaningless for an open one
    pub fn volume(&self) -> f64 {
        let Some(b) = self.bounds() else { return 0.0 };
        let o = b.center();
        let mut v = 0.0;
        for t in 0..self.triangle_count() {
            let [p, q, r] = self.corners(t);
            v += (p - o).dot((q - o).cross(r - o));
        }
        v / 6.0
    }

    /// The mesh mapped by `a`: positions moved; a map that keeps angles (rotation, mirror, uniform scale)
    /// turns the normals with it, any other (a scale that differs per axis) computes them again from the
    /// mapped faces within the same smoothing groups, so hard edges stay hard and smooth ones smooth; a map
    /// that mirrors (negative determinant) turns the triangles, so faces keep pointing outward.  UV stay
    /// (call [`Mesh::uv_box`] again for metres in the new place).
    pub fn transformed(&self, a: &Affine) -> Mesh {
        let similar = a.is_similarity();
        let nm = a.normal_matrix();
        let mut out = Mesh {
            positions: self.positions.iter().map(|&p| a.apply(p)).collect(),
            normals: if similar { self.normals.iter().map(|&n| mat_vec(&nm, n).normalize_or_zero().unsigned_zeros()).collect() } else { self.normals.clone() },
            uvs: self.uvs.clone(),
            points: self.points.clone(),
            indices: self.indices.clone(),
        };
        if a.determinant() < 0.0 {
            out.flip_in_place();
        }
        if !similar {
            out.renormal();
        }
        out
    }

    /// moved by (x, y, z) metres
    pub fn translated(&self, v: Vec3) -> Mesh {
        self.transformed(&Affine::translation(v))
    }

    /// turned by `deg` degrees about an axis through the origin (right-handed; exact at multiples of 90)
    pub fn rotated(&self, axis: Axis, deg: f64) -> Mesh {
        self.transformed(&Affine::rotation(axis, deg))
    }

    /// scaled along the axes about the origin (a negative factor mirrors; faces are turned to stay outward)
    pub fn scaled(&self, s: Vec3) -> Mesh {
        self.transformed(&Affine::scale(s))
    }

    /// mirrored at the plane through the origin across `axis` (x -> -x for Axis::X), faces turned so they
    /// point outward
    pub fn mirrored(&self, axis: Axis) -> Mesh {
        self.transformed(&Affine::mirror(axis))
    }

    /// the triangles turned the other way (normals negated): inside out
    pub fn flipped(&self) -> Mesh {
        let mut m = self.clone();
        m.flip_in_place();
        for n in m.normals.iter_mut() {
            *n = (-*n).unsigned_zeros();
        }
        m
    }

    /// turn every triangle's order (not the normals)
    fn flip_in_place(&mut self) {
        for t in self.indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }

    /// append `o` (its indices and points shifted: its points stay its own)
    pub fn append(&mut self, o: &Mesh) {
        let base = self.positions.len() as u32;
        let pbase = self.point_count() as u32;
        self.points.extend(o.points.iter().map(|&p| p + pbase));
        self.positions.extend_from_slice(&o.positions);
        self.normals.extend_from_slice(&o.normals);
        self.uvs.extend_from_slice(&o.uvs);
        self.indices.extend(o.indices.iter().map(|&i| i + base));
    }

    /// the meshes as one, in order (each keeps its own vertices and points: parts that touch stay apart in the
    /// topology; [`Mesh::weld`] joins them)
    pub fn merge(meshes: &[&Mesh]) -> Result<Mesh, Error> {
        let tris: usize = meshes.iter().map(|m| m.triangle_count()).sum();
        let verts: usize = meshes.iter().map(|m| m.vertex_count()).sum();
        if tris > MAX_TRIANGLES || verts > u32::MAX as usize / 2 {
            return Err(Error::TooBig(alloc::format!("zusammen {} Dreiecke, höchstens {}", tris, MAX_TRIANGLES)));
        }
        let mut out = Mesh::new();
        for m in meshes {
            out.append(m);
        }
        Ok(out)
    }

    /// `count` copies, copy i moved by i * `step` (copy 0 where the mesh is), as one mesh
    pub fn array(&self, count: usize, step: Vec3) -> Result<Mesh, Error> {
        check_copies(self, count)?;
        if !step.is_finite() {
            return Err(Error::Invalid("der Abstand ist nicht endlich".into()));
        }
        let copies: Vec<Mesh> = (0..count).map(|i| self.translated(step * i as f64)).collect();
        let refs: Vec<&Mesh> = copies.iter().collect();
        Mesh::merge(&refs)
    }

    /// `count` copies about the Y axis through the origin, copy i turned by i * `step_deg` degrees (counter-
    /// clockwise seen from above), as one mesh; 360 / count for a full circle
    pub fn radial(&self, count: usize, step_deg: f64) -> Result<Mesh, Error> {
        check_copies(self, count)?;
        if !(crate::math::abs(step_deg) <= crate::math::MAX_DEGREES) {
            return Err(Error::Invalid("der Winkel ist nicht endlich".into()));
        }
        let copies: Vec<Mesh> = (0..count).map(|i| self.rotated(Axis::Y, step_deg * i as f64)).collect();
        let refs: Vec<&Mesh> = copies.iter().collect();
        Mesh::merge(&refs)
    }

    /// the positions as f32 (render3d's MeshData, glTF)
    pub fn positions_f32(&self) -> Vec<[f32; 3]> {
        self.positions.iter().map(|p| p.to_f32()).collect()
    }

    /// the normals as f32
    pub fn normals_f32(&self) -> Vec<[f32; 3]> {
        self.normals.iter().map(|p| p.to_f32()).collect()
    }

    /// the UV as f32
    pub fn uvs_f32(&self) -> Vec<[f32; 2]> {
        self.uvs.iter().map(|u| [u[0] as f32, u[1] as f32]).collect()
    }

    /// FNV-1a (64 bits) over the bits of every position, normal, UV and index in order: equal for equal
    /// meshes on every target (the determinism checksums, cache checks)
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv::new();
        h.u(self.positions.len() as u64);
        for p in &self.positions {
            h.v(*p);
        }
        for n in &self.normals {
            h.v(*n);
        }
        for u in &self.uvs {
            h.f(u[0]);
            h.f(u[1]);
        }
        for &p in &self.points {
            h.u(p as u64);
        }
        h.u(self.indices.len() as u64);
        for &i in &self.indices {
            h.u(i as u64);
        }
        h.0
    }

    /// The points of vertices at the same place joined: at the bit-equal position (-0.0 = +0.0) for
    /// `tolerance` 0, else within `tolerance` metres on every axis (and through chains of such pairs).
    /// Positions stay; only the topology changes, so a half and its mirror image merged and welded are one
    /// closed surface.  Points are numbered again in the order of their first vertex.
    pub fn weld(&self, tolerance: f64) -> Mesh {
        let n = self.positions.len();
        let mut g = Groups::new(self.point_count());
        if tolerance > 0.0 {
            let mut order: Vec<u32> = (0..n as u32).collect();
            order.sort_unstable_by(|&a, &b| {
                let (p, q) = (self.positions[a as usize], self.positions[b as usize]);
                p.x.total_cmp(&q.x).then(a.cmp(&b))
            });
            for i in 0..n {
                let p = self.positions[order[i] as usize];
                for &o in &order[i + 1..] {
                    let q = self.positions[o as usize];
                    if q.x - p.x > tolerance {
                        break;
                    }
                    if crate::math::abs(q.y - p.y) <= tolerance && crate::math::abs(q.z - p.z) <= tolerance {
                        g.unite(self.points[order[i] as usize], self.points[o as usize]);
                    }
                }
            }
        } else {
            let keys: Vec<[u64; 3]> = self.positions.iter().map(|p| p.key()).collect();
            let (map, firsts) = dedupe(&keys);
            for v in 0..n {
                g.unite(self.points[v], self.points[firsts[map[v] as usize] as usize]);
            }
        }
        let roots: Vec<u32> = self.points.iter().map(|&p| g.find(p)).collect();
        let (points, _) = dedupe(&roots);
        Mesh { points, ..self.clone() }
    }
}

fn check_copies(m: &Mesh, count: usize) -> Result<(), Error> {
    if count == 0 {
        return Err(Error::Invalid("die Zahl der Kopien muss mindestens 1 sein".into()));
    }
    if count > crate::MAX_COPIES || count.saturating_mul(m.triangle_count()) > MAX_TRIANGLES {
        return Err(Error::TooBig(alloc::format!("{} Kopien von {} Dreiecken sind zu viele", count, m.triangle_count())));
    }
    Ok(())
}

/// FNV-1a over 64-bit words, the hash of [`Mesh::fingerprint`] (public for checksums over several meshes and
/// numbers)
#[derive(Clone, Copy, Debug)]
pub struct Fnv(pub u64);

impl Fnv {
    /// the offset basis
    pub fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }

    /// fold the 8 bytes of a word in, little-endian
    pub fn u(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    /// fold a number's bits in
    pub fn f(&mut self, v: f64) {
        self.u(v.to_bits());
    }

    /// fold a vector's bits in, x, y, z
    pub fn v(&mut self, v: Vec3) {
        self.f(v.x);
        self.f(v.y);
        self.f(v.z);
    }
}

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv::new()
    }
}

// ---------------------------------------------------------------- welding and deduplication

/// For keys of `n` corners: (the vertex of each corner, the first corner of each vertex): equal keys become
/// one vertex, numbered in the order of their first corner.  Sorting with the corner as the tie-break makes
/// it the same on every target.
pub(crate) fn dedupe<K: Ord + Copy>(keys: &[K]) -> (Vec<u32>, Vec<u32>) {
    let n = keys.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.sort_unstable_by(|&a, &b| keys[a as usize].cmp(&keys[b as usize]).then(a.cmp(&b)));
    let mut rep = vec![0u32; n];
    let mut i = 0;
    while i < n {
        let first = order[i];
        let k = keys[first as usize];
        let mut j = i;
        while j < n && keys[order[j] as usize] == k {
            rep[order[j] as usize] = first;
            j += 1;
        }
        i = j;
    }
    let mut vid = vec![u32::MAX; n];
    let mut firsts = Vec::new();
    for c in 0..n {
        if rep[c] as usize == c {
            vid[c] = firsts.len() as u32;
            firsts.push(c as u32);
        }
    }
    let map = (0..n).map(|c| vid[rep[c] as usize]).collect();
    (map, firsts)
}

/// a union-find over `n` items, the smaller root wins (so groups do not depend on the order of unions)
pub(crate) struct Groups {
    parent: Vec<u32>,
}

impl Groups {
    pub fn new(n: usize) -> Groups {
        Groups { parent: (0..n as u32).collect() }
    }

    pub fn find(&mut self, mut a: u32) -> u32 {
        while self.parent[a as usize] != a {
            let p = self.parent[a as usize];
            self.parent[a as usize] = self.parent[p as usize];
            a = p;
        }
        a
    }

    pub fn unite(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra < rb {
            self.parent[rb as usize] = ra;
        } else if rb < ra {
            self.parent[ra as usize] = rb;
        }
    }
}

/// an undirected edge of points with the triangle and corner it came from: (low point, high point,
/// triangle, corner of the edge's start, whether it runs low to high)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct EdgeUse {
    pub lo: u32,
    pub hi: u32,
    pub tri: u32,
    pub corner: u8,
    pub forward: bool,
}

/// every triangle's three edges over its points, sorted (equal edges next to each other); edges of
/// triangles with a repeated point are left out
pub(crate) fn edge_uses(m: &Mesh) -> Vec<EdgeUse> {
    let pid = &m.points;
    let mut e = Vec::with_capacity(m.indices.len());
    for t in 0..m.triangle_count() {
        let tri = m.triangle(t);
        let p = [pid[tri[0] as usize], pid[tri[1] as usize], pid[tri[2] as usize]];
        if p[0] == p[1] || p[1] == p[2] || p[0] == p[2] {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (p[k], p[(k + 1) % 3]);
            e.push(EdgeUse { lo: a.min(b), hi: a.max(b), tri: t as u32, corner: k as u8, forward: a < b });
        }
    }
    e.sort_unstable();
    e
}
