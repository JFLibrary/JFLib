// normals -- vertex normals and texture coordinates: flat (each triangle its own normal), smooth with an
// angle threshold (the triangles around a point share a normal across every edge flatter than the
// threshold, split at sharper ones), and UV by box projection in metres.  Each rebuilds the vertices from
// the triangles' corners: corners with the same position, normal and UV become one vertex, in the order of
// their first corner, so the result is the same on every target.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Vec3, atan2, sin_cos_deg};
use crate::mesh::{Groups, Mesh, dedupe, edge_uses};

/// the normal a corner of a triangle without area gets (it has none of its own); the checks count such
/// triangles as degenerate
const FALLBACK: Vec3 = Vec3::Y;

/// the unit normal of every triangle (counter-clockwise from outside), None for one without area
pub(crate) fn face_normals(m: &Mesh) -> Vec<Option<Vec3>> {
    (0..m.triangle_count())
        .map(|t| {
            let [p, q, r] = m.corners(t);
            (q - p).cross(r - p).try_normalize()
        })
        .collect()
}

/// the corners (point, normal, uv) of every triangle as an indexed mesh: corners of one point with the same
/// normal and UV become one vertex; points numbered again in the order of their first corner
fn from_corners(m: &Mesh, normal: &[Vec3], uv: &[[f64; 2]]) -> Mesh {
    let n = m.indices.len();
    let mut keys: Vec<[u64; 6]> = Vec::with_capacity(n);
    for c in 0..n {
        let q = normal[c].key();
        let pt = m.points[m.indices[c] as usize] as u64;
        keys.push([pt, q[0], q[1], q[2], (uv[c][0] + 0.0).to_bits(), (uv[c][1] + 0.0).to_bits()]);
    }
    let (map, firsts) = dedupe(&keys);
    let old_points: Vec<u32> = firsts.iter().map(|&c| m.points[m.indices[c as usize] as usize]).collect();
    let (points, _) = dedupe(&old_points);
    Mesh {
        positions: firsts.iter().map(|&c| m.positions[m.indices[c as usize] as usize].unsigned_zeros()).collect(),
        normals: firsts.iter().map(|&c| normal[c as usize].unsigned_zeros()).collect(),
        uvs: firsts.iter().map(|&c| [uv[c as usize][0] + 0.0, uv[c as usize][1] + 0.0]).collect(),
        points,
        indices: map,
    }
}

/// the UV of every corner as the vertices have them
fn corner_uvs(m: &Mesh) -> Vec<[f64; 2]> {
    m.indices.iter().map(|&i| m.uvs[i as usize]).collect()
}

impl Mesh {
    /// Compute the normals again from the faces, keeping the smoothing groups: the vertices of one point
    /// with the same normal now share the angle-weighted normal of all their triangles (after a scale that
    /// differs per axis the old normals lean; a hard edge stays two groups, a UV seam inside a smooth area
    /// stays one).
    pub(crate) fn renormal(&mut self) {
        let keys: Vec<[u64; 4]> = (0..self.positions.len())
            .map(|v| {
                let n = self.normals[v].key();
                [self.points[v] as u64, n[0], n[1], n[2]]
            })
            .collect();
        let (group, _) = dedupe(&keys);
        let fnorm = face_normals(self);
        let mut sum = vec![Vec3::ZERO; self.positions.len()];
        for t in 0..self.triangle_count() {
            let Some(n) = fnorm[t] else { continue };
            let p = self.corners(t);
            for k in 0..3 {
                let (a, b) = (p[(k + 1) % 3] - p[k], p[(k + 2) % 3] - p[k]);
                let w = atan2(a.cross(b).length(), a.dot(b));
                sum[group[self.indices[3 * t + k] as usize] as usize] += n * w;
            }
        }
        for v in 0..self.positions.len() {
            self.normals[v] = sum[group[v] as usize].try_normalize().unwrap_or(FALLBACK).unsigned_zeros();
        }
    }

    /// Flat normals: every corner gets the normal of its triangle (hard edges everywhere).
    pub fn flat(&self) -> Mesh {
        let fnorm = face_normals(self);
        let normal: Vec<Vec3> = (0..self.indices.len()).map(|c| fnorm[c / 3].unwrap_or(FALLBACK)).collect();
        from_corners(self, &normal, &corner_uvs(self))
    }

    /// Smooth normals with an angle threshold in degrees: around each point the triangles that meet across
    /// an edge whose faces differ by at most `max_angle_deg` share one normal (their normals weighted by
    /// the corner angles), split at sharper edges, open edges and edges of more than two triangles.  Edges
    /// are those of the points (`Mesh::points`), so a hard edge split before can become smooth again; parts
    /// that only touch are never smoothed together.  0 gives flat normals except across exactly flat edges,
    /// 180 smooths every manifold edge.
    pub fn smooth(&self, max_angle_deg: f64) -> Mesh {
        let tris = self.triangle_count();
        let fnorm = face_normals(self);
        let cos_max = if max_angle_deg >= 180.0 { -2.0 } else { sin_cos_deg(max_angle_deg.max(0.0)).1 };
        let mut groups = Groups::new(3 * tris);
        let edges = edge_uses(self);
        let mut i = 0;
        while i < edges.len() {
            let mut j = i + 1;
            while j < edges.len() && edges[j].lo == edges[i].lo && edges[j].hi == edges[i].hi {
                j += 1;
            }
            // exactly two triangles in opposite directions, both with a normal, flat enough
            if j - i == 2 && edges[i].forward != edges[i + 1].forward {
                let (e, f) = (edges[i], edges[i + 1]);
                if let (Some(a), Some(b)) = (fnorm[e.tri as usize], fnorm[f.tri as usize]) {
                    if a.dot(b) >= cos_max {
                        // e runs from its corner k to k + 1; f runs the other way, from its corner l (at e's
                        // end) to l + 1 (at e's start)
                        let (t, k) = (3 * e.tri, e.corner as u32);
                        let (u, l) = (3 * f.tri, f.corner as u32);
                        groups.unite(t + k, u + (l + 1) % 3);
                        groups.unite(t + (k + 1) % 3, u + l);
                    }
                }
            }
            i = j;
        }
        // the normal of a group: its corners' face normals weighted by the corner angle
        let mut sum = vec![Vec3::ZERO; 3 * tris];
        for t in 0..tris {
            let Some(n) = fnorm[t] else { continue };
            let p = self.corners(t);
            for k in 0..3 {
                let (a, b) = (p[(k + 1) % 3] - p[k], p[(k + 2) % 3] - p[k]);
                let w = atan2(a.cross(b).length(), a.dot(b));
                let r = groups.find((3 * t + k) as u32) as usize;
                sum[r] += n * w;
            }
        }
        let normal: Vec<Vec3> = (0..3 * tris)
            .map(|c| {
                let r = groups.find(c as u32) as usize;
                sum[r].try_normalize().or(fnorm[c / 3]).unwrap_or(FALLBACK)
            })
            .collect();
        from_corners(self, &normal, &corner_uvs(self))
    }

    /// UV by box projection in world scale: each triangle is projected along the axis its normal points
    /// most along (ties: Y, then X), so one UV unit is `metres_per_unit` metres on every face, without
    /// mirrored textures: +X faces (u, v) = (-z, y), -X (z, y), +Y (x, -z), -Y (x, z), +Z (x, y), -Z (-x, y).
    /// Normals stay; vertices split where the projection changes.
    pub fn uv_box(&self, metres_per_unit: f64) -> Mesh {
        let s = if metres_per_unit > 0.0 && metres_per_unit.is_finite() { 1.0 / metres_per_unit } else { 1.0 };
        let fnorm = face_normals(self);
        let n = self.indices.len();
        let mut uv = Vec::with_capacity(n);
        for c in 0..n {
            let f = fnorm[c / 3].unwrap_or(FALLBACK);
            let p = self.positions[self.indices[c] as usize];
            let (ax, ay, az) = (crate::math::abs(f.x), crate::math::abs(f.y), crate::math::abs(f.z));
            let (u, v) = if ay >= ax && ay >= az {
                if f.y > 0.0 { (p.x, -p.z) } else { (p.x, p.z) }
            } else if ax >= az {
                if f.x > 0.0 { (-p.z, p.y) } else { (p.z, p.y) }
            } else if f.z > 0.0 {
                (p.x, p.y)
            } else {
                (-p.x, p.y)
            };
            uv.push([u * s, v * s]);
        }
        let normal: Vec<Vec3> = self.indices.iter().map(|&i| self.normals[i as usize]).collect();
        from_corners(self, &normal, &uv)
    }
}
