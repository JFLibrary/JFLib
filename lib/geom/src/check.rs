// check -- what a model must be, measured: closed and manifold (every edge between exactly two triangles,
// one fan around every point), consistently wound with the faces outward (positive volume per closed
// part), normals that agree with the faces, no degenerate triangles; bounds, area, volume, counts.  The
// topology is the mesh's points (Mesh::points), so hard edges and UV seams do not open a mesh and parts
// that only touch are not counted as non-manifold.  The report is plain data and JSON for tools
// (apps/modeltool).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::math::{Aabb, Vec3};
use crate::mesh::{Groups, Mesh, edge_uses};

/// A triangle counts as degenerate when two corners are one point or its height above the longest edge is
/// below this fraction of that edge (collinear corners, slivers no renderer shows)
pub const DEGENERATE_RATIO: f64 = 1e-9;

/// A stored normal counts as wrong when its length is off 1 by more than this, or when it points away from
/// its triangle's face (a dot product of 0 or less)
pub const NORMAL_TOLERANCE: f64 = 1e-6;

/// The checks of a mesh (see [`Mesh::check`]).  Counts of problems are 0 for a good mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// vertices of the mesh
    pub vertices: usize,
    /// triangles of the mesh
    pub triangles: usize,
    /// points of the topology (`Mesh::points`)
    pub points: usize,
    /// distinct edges between points
    pub edges: usize,
    /// the bounds in metres (None for an empty mesh)
    pub bounds: Option<Aabb>,
    /// square metres
    pub area: f64,
    /// cubic metres, signed (outward faces: positive); meaningful when closed
    pub volume: f64,
    /// parts connected across edges
    pub components: usize,
    /// V - E + F summed over the components (2 per closed part without a hole, 0 for a torus)
    pub euler: i64,
    /// edges with one triangle: the mesh has a hole there
    pub open_edges: usize,
    /// edges with more than two triangles
    pub nonmanifold_edges: usize,
    /// edges whose two triangles run the same way: one of them is turned
    pub flipped_edges: usize,
    /// points whose triangles form more than one fan (two parts touching at a point)
    pub nonmanifold_points: usize,
    /// closed components with a negative volume: inside out
    pub inverted_components: usize,
    /// triangles without area (see [`DEGENERATE_RATIO`])
    pub degenerate: usize,
    /// vertex normals that are not unit length or point away from their face (see [`NORMAL_TOLERANCE`])
    pub bad_normals: usize,
}

impl Report {
    /// closed and manifold: no open, non-manifold or flipped edges, no pinched points
    pub fn closed(&self) -> bool {
        self.open_edges == 0 && self.nonmanifold_edges == 0 && self.flipped_edges == 0 && self.nonmanifold_points == 0
    }

    /// faces point outward: consistently wound (no flipped edges) and no closed part inside out
    pub fn outward(&self) -> bool {
        self.flipped_edges == 0 && self.inverted_components == 0
    }

    /// every check passed: closed, outward, no degenerate triangles, good normals
    pub fn ok(&self) -> bool {
        self.closed() && self.outward() && self.degenerate == 0 && self.bad_normals == 0 && self.triangles > 0
    }

    /// width, height, depth in metres (0 for an empty mesh)
    pub fn size(&self) -> Vec3 {
        self.bounds.map_or(Vec3::ZERO, |b| b.size())
    }

    /// the report as one JSON object, keys in a fixed order, numbers rounded to 1e-9 (metres: a nanometre)
    pub fn json(&self) -> String {
        let mut s = String::new();
        let b = self.bounds.unwrap_or(Aabb::point(Vec3::ZERO));
        let v3 = |v: Vec3| alloc::format!("[{}, {}, {}]", fnum(v.x), fnum(v.y), fnum(v.z));
        s.push('{');
        s.push_str(&alloc::format!(
            "\"ok\": {}, \"closed\": {}, \"outward\": {}, \"vertices\": {}, \"triangles\": {}, \"points\": {}, \"edges\": {}, ",
            self.ok(), self.closed(), self.outward(), self.vertices, self.triangles, self.points, self.edges
        ));
        s.push_str(&alloc::format!(
            "\"min\": {}, \"max\": {}, \"size\": {}, \"area\": {}, \"volume\": {}, \"components\": {}, \"euler\": {}, ",
            v3(b.min), v3(b.max), v3(self.size()), fnum(self.area), fnum(self.volume), self.components, self.euler
        ));
        s.push_str(&alloc::format!(
            "\"open_edges\": {}, \"nonmanifold_edges\": {}, \"flipped_edges\": {}, \"nonmanifold_points\": {}, \"inverted_components\": {}, \"degenerate\": {}, \"bad_normals\": {}}}",
            self.open_edges, self.nonmanifold_edges, self.flipped_edges, self.nonmanifold_points, self.inverted_components, self.degenerate, self.bad_normals
        ));
        s
    }
}

/// a number for JSON: rounded to 9 decimals, without trailing zeros, -0 as 0, NaN and infinities as null
pub fn fnum(v: f64) -> String {
    if !v.is_finite() {
        return String::from("null");
    }
    let mut s = alloc::format!("{:.9}", v);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = String::from("0");
    }
    s
}

impl Mesh {
    /// Measure the mesh: counts, bounds, area, volume and the topology checks of [`Report`].
    pub fn check(&self) -> Report {
        let tris = self.triangle_count();
        let pid = &self.points;
        let points = self.point_count();
        // degenerate triangles: a repeated point or (almost) no height
        let mut degenerate = 0;
        for t in 0..tris {
            let tri = self.triangle(t);
            let p = [pid[tri[0] as usize], pid[tri[1] as usize], pid[tri[2] as usize]];
            if p[0] == p[1] || p[1] == p[2] || p[0] == p[2] {
                degenerate += 1;
                continue;
            }
            let [a, b, c] = self.corners(t);
            let e2 = (b - a).length_sq().max((c - b).length_sq()).max((a - c).length_sq());
            let twice = (b - a).cross(c - a).length();
            // height = twice / longest edge; degenerate when height < ratio * longest edge
            if !(twice > DEGENERATE_RATIO * e2) {
                degenerate += 1;
            }
        }
        // edges
        let uses = edge_uses(self);
        let mut comp = Groups::new(tris);
        let mut corner_fans = Groups::new(3 * tris);
        let (mut edges, mut open, mut nonmanifold, mut flipped) = (0, 0, 0, 0);
        let mut i = 0;
        while i < uses.len() {
            let mut j = i + 1;
            while j < uses.len() && uses[j].lo == uses[i].lo && uses[j].hi == uses[i].hi {
                j += 1;
            }
            edges += 1;
            match j - i {
                1 => open += 1,
                2 => {
                    let (e, f) = (uses[i], uses[i + 1]);
                    if e.forward == f.forward {
                        flipped += 1;
                    } else {
                        let (t, k) = (3 * e.tri, e.corner as u32);
                        let (u, l) = (3 * f.tri, f.corner as u32);
                        corner_fans.unite(t + k, u + (l + 1) % 3);
                        corner_fans.unite(t + (k + 1) % 3, u + l);
                    }
                    comp.unite(e.tri, f.tri);
                }
                _ => {
                    nonmanifold += 1;
                    for u in &uses[i..j] {
                        comp.unite(uses[i].tri, u.tri);
                    }
                }
            }
            i = j;
        }
        // pinched points: more than one fan around a point (only where its edges are all manifold)
        let mut fans: Vec<(u32, u32)> = Vec::with_capacity(3 * tris);
        for t in 0..tris {
            let tri = self.triangle(t);
            for k in 0..3 {
                fans.push((pid[tri[k] as usize], corner_fans.find((3 * t + k) as u32)));
            }
        }
        fans.sort_unstable();
        fans.dedup();
        let mut fan_count = vec![0u32; points];
        for &(p, _) in &fans {
            fan_count[p as usize] += 1;
        }
        let nonmanifold_points = if open == 0 && nonmanifold == 0 && flipped == 0 { fan_count.iter().filter(|&&c| c > 1).count() } else { 0 };
        // components: points, edges, triangles and signed volume each
        let mut root_of = vec![0u32; tris];
        for (t, r) in root_of.iter_mut().enumerate() {
            *r = comp.find(t as u32);
        }
        let mut roots: Vec<u32> = root_of.clone();
        roots.sort_unstable();
        roots.dedup();
        let index_of = |r: u32| roots.binary_search(&r).unwrap_or(0);
        let nc = roots.len();
        let mut c_tris = vec![0i64; nc];
        let mut c_edges = vec![0i64; nc];
        let mut c_vol = vec![0.0f64; nc];
        let mut c_open = vec![false; nc];
        let mut c_points: Vec<(usize, u32)> = Vec::with_capacity(3 * tris);
        let origin = self.bounds().map_or(Vec3::ZERO, |b| b.center());
        for t in 0..tris {
            let c = index_of(root_of[t]);
            c_tris[c] += 1;
            let [a, b, cc] = self.corners(t);
            c_vol[c] += (a - origin).dot((b - origin).cross(cc - origin));
            for &v in &self.triangle(t) {
                c_points.push((c, pid[v as usize]));
            }
        }
        c_points.sort_unstable();
        c_points.dedup();
        let mut c_npoints = vec![0i64; nc];
        for &(c, _) in &c_points {
            c_npoints[c] += 1;
        }
        let mut i = 0;
        while i < uses.len() {
            let mut j = i + 1;
            while j < uses.len() && uses[j].lo == uses[i].lo && uses[j].hi == uses[i].hi {
                j += 1;
            }
            let c = index_of(root_of[uses[i].tri as usize]);
            c_edges[c] += 1;
            if j - i != 2 || uses[i].forward == uses[i + 1].forward {
                c_open[c] = true;
            }
            i = j;
        }
        let mut inverted = 0;
        let mut euler = 0i64;
        for c in 0..nc {
            euler += c_npoints[c] - c_edges[c] + c_tris[c];
            if !c_open[c] && c_vol[c] < 0.0 {
                inverted += 1;
            }
        }
        // stored normals against the faces
        let fnorm = crate::normals::face_normals(self);
        let mut bad_normals = 0;
        for c in 0..self.indices.len() {
            let n = self.normals[self.indices[c] as usize];
            let len = n.length();
            let wrong_len = !(crate::math::abs(len - 1.0) <= NORMAL_TOLERANCE);
            let against = fnorm[c / 3].is_some_and(|f| !(f.dot(n) > 0.0));
            if wrong_len || against {
                bad_normals += 1;
            }
        }
        Report {
            vertices: self.vertex_count(),
            triangles: tris,
            points,
            edges,
            bounds: self.bounds(),
            area: self.area(),
            volume: self.volume(),
            components: nc,
            euler,
            open_edges: open,
            nonmanifold_edges: nonmanifold,
            flipped_edges: flipped,
            nonmanifold_points,
            inverted_components: inverted,
            degenerate,
            bad_normals,
        }
    }
}
