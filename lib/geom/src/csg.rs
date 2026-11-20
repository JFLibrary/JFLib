// csg -- Boolean operations on convex solids, as brushes in a map editor (docs/GEOM.md, "Boolesche
// Operationen"): a Solid is a set of convex cells that do not overlap, each the intersection of half-spaces
// whose planes it shares with the other cells; union, difference and intersection split cells by planes and
// give such a set again, so results can be combined further.  to_mesh gives the boundary: the faces of the
// cells less where two cells touch, as one closed mesh.
//
// Exact where it decides: a point is where three planes meet, never rounded while the operation runs, and
// which side of a plane it lies on is the sign of a 4 x 4 determinant of plane coefficients (exact.rs: f64
// with an error bound, exact expansions near 0).  So splitting a cell, cutting a face and finding where
// faces of neighbouring cells meet always agree with each other, also for points exactly on a plane (the
// usual case: a window flush with a wall, boxes touching).  Only the output is rounded: every corner to the
// f64 nearest the exact point (equal points, equal bits), the faces triangulated with exact orientation
// tests on those coordinates.  The planes themselves come from the input meshes in f64; planes closer than
// WELD_RATIO of the extent count as one (also between the operands), so faces that are meant to lie in one
// plane do, and features smaller than that (where several planes almost meet in one point: the tip of a
// cone, the rings of a sphere) fall together into one point of the output.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use crate::exact::{Meet, Pl};
use crate::math::{Aabb, Affine, Axis, Vec3, abs};
use crate::mesh::{Groups, Mesh};
use crate::{Error, MAX_TRIANGLES};

/// Planes closer than this fraction of the extent count as one, output points closer than it fall together.
pub const WELD_RATIO: f64 = 1e-9;

/// the most cells a solid may have (a limit against runaway splitting)
pub const MAX_CELLS: usize = 20_000;

/// A plane n . x + w = 0, n a unit vector pointing out of the solid: inside is n . x + w < 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    /// the outward normal, unit length
    pub n: Vec3,
    /// the offset: n . x + w = 0 on the plane
    pub w: f64,
}

impl Plane {
    /// the signed distance of `p` (positive outside)
    pub fn eval(&self, p: Vec3) -> f64 {
        self.n.dot(p) + self.w
    }

    fn pl(&self) -> Pl {
        [self.n.x, self.n.y, self.n.z, self.w]
    }

    /// the same plane facing the other way
    pub fn flipped(&self) -> Plane {
        Plane { n: (-self.n).unsigned_zeros(), w: -self.w + 0.0 }
    }

    /// the plane mapped by `a` (normal by the normal matrix, kept pointing outward)
    pub fn transformed(&self, a: &Affine) -> Plane {
        let on = self.n * (-self.w);
        let p = a.apply(on);
        let nm = a.normal_matrix();
        let n = crate::math::mat_vec(&nm, self.n).normalize_or_zero().unsigned_zeros();
        Plane { n, w: -n.dot(p) + 0.0 }
    }
}

/// a plane of a cell: its index << 1, | 1 when the cell lies on its outside (the plane flipped)
type PRef = u32;

#[inline]
fn pid(r: PRef) -> usize {
    (r >> 1) as usize
}

#[inline]
fn flip(r: PRef) -> PRef {
    r ^ 1
}

/// A solid as convex cells that do not overlap (brushes): each cell is a list of planes whose inside it is.
/// Made from a convex mesh ([`Solid::from_convex`]) or by union, difference and intersection; [`Solid::to_mesh`]
/// gives its surface.
#[derive(Clone, Debug, PartialEq)]
pub struct Solid {
    planes: Vec<Plane>,
    cells: Vec<Vec<PRef>>,
    bounds: Aabb,
}

/// what a bevel made: the solid and how many planes were the original faces (the rest are the bevel's)
struct BevelPlanes {
    solid: Solid,
    original: usize,
    segments: usize,
    ctx: Ctx,
    poly: Poly,
}

// ---------------------------------------------------------------- the working set of an operation

/// a face of a cell: the support plane, the edge planes counter-clockwise seen from outside, and the corners
/// (v[j] where e[j] and e[j + 1] meet on s)
#[derive(Clone, Debug)]
struct Face {
    s: PRef,
    e: Vec<PRef>,
    v: Vec<u32>,
}

/// a convex cell with its faces
#[derive(Clone, Debug)]
struct Poly {
    faces: Vec<Face>,
}

impl Poly {
    fn supports(&self) -> Vec<PRef> {
        self.faces.iter().map(|f| f.s).collect()
    }
}

/// the planes of an operation (the solid's and six far planes of a box around everything, for the first
/// polygon on a plane) and the points where three of them meet
struct Ctx {
    pl: Vec<Pl>,
    nbox: usize,
    meets: Vec<Meet>,
    keys: Vec<[u32; 3]>,
    wsign: Vec<i32>,
    index: BTreeMap<[u32; 3], u32>,
    coords: Vec<Option<[f64; 3]>>,
    /// the sides of the points for the plane of the current split (valid where stamp is epoch)
    memo: Vec<i32>,
    stamp: Vec<u32>,
    epoch: u32,
}

fn bad(msg: &str) -> Error {
    Error::Invalid(alloc::format!("Boolesche Operation: {}", msg))
}

impl Ctx {
    fn new(planes: &[Plane], bounds: Aabb) -> Ctx {
        let mut pl: Vec<Pl> = planes.iter().map(|p| p.pl()).collect();
        let nbox = pl.len();
        // the box: a power of two beyond everything (exact planes)
        let reach = [bounds.min.x, bounds.min.y, bounds.min.z, bounds.max.x, bounds.max.y, bounds.max.z].iter().fold(1.0f64, |m, &v| m.max(abs(v)));
        let mut m = 1.0;
        while m < 4.0 * reach + 1.0 {
            m *= 2.0;
        }
        for axis in 0..3 {
            let mut n = [0.0; 4];
            n[axis] = 1.0;
            n[3] = -m;
            pl.push(n);
            let mut n = [0.0; 4];
            n[axis] = -1.0;
            n[3] = -m;
            pl.push(n);
        }
        Ctx { pl, nbox, meets: Vec::new(), keys: Vec::new(), wsign: Vec::new(), index: BTreeMap::new(), coords: Vec::new(), memo: Vec::new(), stamp: Vec::new(), epoch: 0 }
    }

    /// the reference of a box plane: axis 0..3, positive or negative side
    fn box_ref(&self, axis: usize, positive: bool) -> PRef {
        ((self.nbox + 2 * axis + if positive { 0 } else { 1 }) as u32) << 1
    }

    /// the point where planes a, b, c meet (Err when they do not meet in one point)
    fn vertex(&mut self, a: PRef, b: PRef, c: PRef) -> Result<u32, Error> {
        let mut k = [pid(a) as u32, pid(b) as u32, pid(c) as u32];
        k.sort_unstable();
        if let Some(&v) = self.index.get(&k) {
            return Ok(v);
        }
        if k[0] == k[1] || k[1] == k[2] {
            return Err(bad("eine Ecke aus zwei gleichen Ebenen"));
        }
        let (p, q, r) = (self.pl[k[0] as usize], self.pl[k[1] as usize], self.pl[k[2] as usize]);
        let mut m = Meet::new(&p, &q, &r);
        let w = m.w_sign(&p, &q, &r);
        if w == 0 {
            return Err(bad("drei Ebenen ohne gemeinsamen Punkt"));
        }
        let id = self.meets.len() as u32;
        self.meets.push(m);
        self.keys.push(k);
        self.wsign.push(w);
        self.coords.push(None);
        self.memo.push(0);
        self.stamp.push(0);
        self.index.insert(k, id);
        Ok(id)
    }

    /// the side of plane `r` at point `v`: -1 inside, 0 on it, 1 outside (exact)
    fn side(&mut self, r: PRef, v: u32) -> i32 {
        let k = self.keys[v as usize];
        let (p, q, t) = (self.pl[k[0] as usize], self.pl[k[1] as usize], self.pl[k[2] as usize]);
        let s = self.pl[pid(r)];
        let d = self.meets[v as usize].dot_sign(&s, &p, &q, &t) * self.wsign[v as usize];
        if r & 1 == 1 { -d } else { d }
    }

    /// the point's coordinates, correctly rounded
    fn coord(&mut self, v: u32) -> [f64; 3] {
        if let Some(c) = self.coords[v as usize] {
            return c;
        }
        let k = self.keys[v as usize];
        let (p, q, t) = (self.pl[k[0] as usize], self.pl[k[1] as usize], self.pl[k[2] as usize]);
        let c = self.meets[v as usize].point(&p, &q, &t);
        self.coords[v as usize] = Some(c);
        c
    }

    /// the plane's normal (flipped for a flipped reference)
    fn normal(&self, r: PRef) -> Vec3 {
        let p = self.pl[pid(r)];
        let n = Vec3::new(p[0], p[1], p[2]);
        if r & 1 == 1 { -n } else { n }
    }

    /// a polygon on plane s far larger than everything: the box planes across its dominant axis
    fn big_quad(&mut self, s: PRef) -> Result<Face, Error> {
        let n = self.normal(s);
        let a = if abs(n.x) >= abs(n.y) && abs(n.x) >= abs(n.z) {
            0
        } else if abs(n.y) >= abs(n.z) {
            1
        } else {
            2
        };
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        let pos = n[a] > 0.0;
        // counter-clockwise seen from +a in (u, v): bottom (v >= -M), right (u <= M), top, left
        let mut e = vec![self.box_ref(v, false), self.box_ref(u, true), self.box_ref(v, true), self.box_ref(u, false)];
        if !pos {
            e.reverse();
        }
        let mut vs = Vec::with_capacity(4);
        for j in 0..4 {
            vs.push(self.vertex(s, e[j], e[(j + 1) % 4])?);
        }
        Ok(Face { s, e, v: vs })
    }

    /// the face cut by plane p (its inside part); `signs` are p's sides at the face's corners.  None when
    /// nothing with an area is left
    fn clip_signs(&mut self, f: &Face, p: PRef, s: &[i32]) -> Result<Option<Face>, Error> {
        let k = f.e.len();
        if !s.iter().any(|&x| x < 0) {
            return Ok(None);
        }
        if !s.iter().any(|&x| x > 0) {
            return Ok(Some(f.clone()));
        }
        // edge j runs from corner j - 1 to corner j; it stays when one of its ends is inside
        let kept = |j: usize| s[(j + k - 1) % k] < 0 || s[j] < 0;
        let Some(last) = (0..k).find(|&j| kept(j) && (!kept((j + 1) % k) || s[j] > 0)) else {
            return Err(bad("ein Schnitt ohne Ende"));
        };
        let mut first = (last + 1) % k;
        while !kept(first) {
            first = (first + 1) % k;
        }
        let mut run = Vec::with_capacity(k + 1);
        let mut j = first;
        loop {
            run.push(j);
            if j == last {
                break;
            }
            j = (j + 1) % k;
            if !kept(j) {
                return Err(bad("ein Schnitt, der eine Fläche zweimal trifft"));
            }
        }
        let mut e: Vec<PRef> = run.iter().map(|&j| f.e[j]).collect();
        e.push(p);
        let mut v = Vec::with_capacity(e.len());
        for i in 0..run.len() - 1 {
            v.push(f.v[run[i]]);
        }
        v.push(self.vertex(f.s, f.e[last], p)?);
        v.push(self.vertex(f.s, p, f.e[first])?);
        Ok(Some(Face { s: f.s, e, v }))
    }

    fn clip(&mut self, f: &Face, p: PRef) -> Result<Option<Face>, Error> {
        let s: Vec<i32> = f.v.iter().map(|&v| self.side(p, v)).collect();
        self.clip_signs(f, p, &s)
    }

    /// The convex cell of the planes (None when it has no volume): the far box, split by each plane in turn,
    /// keeping the inside.
    fn polytope(&mut self, cell: &[PRef]) -> Result<Option<Poly>, Error> {
        let mut planes: Vec<PRef> = cell.to_vec();
        planes.sort_unstable();
        planes.dedup();
        if planes.windows(2).any(|w| w[0] >> 1 == w[1] >> 1) {
            return Ok(None); // a plane from both sides: flat
        }
        let mut faces = Vec::with_capacity(planes.len() + 6);
        for axis in 0..3 {
            for positive in [true, false] {
                let r = self.box_ref(axis, positive);
                faces.push(self.big_quad(r)?);
            }
        }
        let mut poly = Poly { faces };
        for &p in &planes {
            match self.split(poly, p, false)?.0 {
                Some(x) => poly = x,
                None => return Ok(None),
            }
        }
        let boxed = |r: PRef| pid(r) >= self.nbox && pid(r) < self.nbox + 6;
        if poly.faces.iter().any(|f| boxed(f.s) || f.e.iter().any(|&e| boxed(e))) {
            return Err(bad("ein Körper ohne Ende (zu wenige Ebenen)"));
        }
        Ok(Some(poly))
    }

    /// The cell split by plane p: (the part inside p, the part outside: None unless `outside`), each None when
    /// empty.  The faces are cut, the new face on p is a polygon on p cut by the planes of the faces that meet p.
    fn split(&mut self, poly: Poly, p: PRef, outside: bool) -> Result<(Option<Poly>, Option<Poly>), Error> {
        let mut signs: Vec<Vec<i32>> = Vec::with_capacity(poly.faces.len());
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.stamp.iter_mut().for_each(|x| *x = 0);
            self.epoch = 1;
        }
        let (mut neg, mut pos) = (false, false);
        for f in &poly.faces {
            let mut s = Vec::with_capacity(f.v.len());
            for &v in &f.v {
                let x = if self.stamp[v as usize] == self.epoch {
                    self.memo[v as usize]
                } else {
                    let x = self.side(p, v);
                    self.memo[v as usize] = x;
                    self.stamp[v as usize] = self.epoch;
                    x
                };
                neg |= x < 0;
                pos |= x > 0;
                s.push(x);
            }
            signs.push(s);
        }
        if !neg {
            return Ok((None, if outside { Some(poly) } else { None }));
        }
        if !pos {
            return Ok((Some(poly), None));
        }
        let (mut fin, mut fout, mut touch) = (Vec::new(), Vec::new(), Vec::new());
        for (f, s) in poly.faces.iter().zip(&signs) {
            let (lo, hi) = (s.iter().any(|&x| x < 0), s.iter().any(|&x| x > 0));
            if s.iter().any(|&x| x == 0) || (lo && hi) {
                touch.push(f.s);
            }
            if let Some(g) = self.clip_signs(f, p, s)? {
                fin.push(g);
            }
            if outside {
                let ns: Vec<i32> = s.iter().map(|&x| -x).collect();
                if let Some(g) = self.clip_signs(f, flip(p), &ns)? {
                    fout.push(g);
                }
            }
        }
        let mut cap = Some(self.big_quad(p)?);
        for &t in &touch {
            let Some(c) = cap else { break };
            cap = self.clip(&c, t)?;
        }
        let Some(cap) = cap else { return Err(bad("ein Schnitt ohne Fläche")) };
        let k = cap.e.len();
        let rev = Face {
            s: flip(p),
            e: cap.e.iter().rev().copied().collect(),
            v: (0..k).map(|j| cap.v[(2 * k - 2 - j) % k]).collect(),
        };
        fin.push(cap);
        if !outside {
            return Ok((Some(Poly { faces: fin }), None));
        }
        fout.push(rev);
        Ok((Some(Poly { faces: fin }), Some(Poly { faces: fout })))
    }

    /// poly less the convex cell of `planes`: the pieces outside each plane in turn
    fn subtract(&mut self, poly: Poly, planes: &[PRef]) -> Result<Vec<Poly>, Error> {
        let mut out = Vec::new();
        let mut rest = poly;
        for &p in planes {
            let (inside, outside) = self.split(rest, p, true)?;
            if let Some(o) = outside {
                out.push(o);
            }
            match inside {
                Some(i) => rest = i,
                None => return Ok(out),
            }
        }
        Ok(out)
    }

    /// the face less the face `o` on the same plane (from the other side): the pieces outside each of o's
    /// edges in turn
    fn face_minus(&mut self, f: Face, o: &Face) -> Result<Vec<Face>, Error> {
        let mut out = Vec::new();
        let mut rest = f;
        for &e in &o.e {
            if let Some(x) = self.clip(&rest, flip(e))? {
                out.push(x);
            }
            match self.clip(&rest, e)? {
                Some(x) => rest = x,
                None => return Ok(out),
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- solids

fn tolerance(b: &Aabb) -> f64 {
    let s = b.size();
    WELD_RATIO * s.x.max(s.y).max(s.z).max(1e-300)
}

/// whether planes p and q are one within `tol` over the ball (c, r): normals and offsets close
fn near(p: &Plane, q: &Plane, c: Vec3, r: f64, tol: f64) -> bool {
    let dn = p.n - q.n;
    (abs(dn.x) + abs(dn.y) + abs(dn.z)) * r + abs(p.eval(c) - q.eval(c)) <= tol
}

fn fmt3(v: Vec3) -> alloc::string::String {
    alloc::format!("({}, {}, {})", crate::fnum(v.x), crate::fnum(v.y), crate::fnum(v.z))
}

impl Solid {
    /// The solid of a convex closed mesh: one cell whose planes are the mesh's faces (triangles in one plane
    /// within [`WELD_RATIO`] of the extent are one face).  Err when the mesh is not closed, inside out, flat
    /// or not convex (a corner in front of a face by more than the tolerance).
    pub fn from_convex(m: &Mesh) -> Result<Solid, Error> {
        if m.triangle_count() == 0 {
            return Err(Error::Invalid("das Mesh ist leer".into()));
        }
        let r = m.check();
        if !r.closed() {
            return Err(Error::Invalid(alloc::format!(
                "das Mesh ist nicht geschlossen ({} offene Kanten, {} Kanten mit mehr als zwei Dreiecken): Boolesche Operationen brauchen geschlossene Körper",
                r.open_edges, r.nonmanifold_edges
            )));
        }
        if r.components != 1 {
            return Err(Error::Invalid(alloc::format!(
                "das Mesh besteht aus {} Teilen; eine Boolesche Operation nimmt je Mesh einen konvexen Körper (oder das Ergebnis einer Booleschen Operation): geom.union statt geom.merge",
                r.components
            )));
        }
        if !(r.volume > 0.0) {
            return Err(Error::Invalid("das Mesh steht innen nach außen (Volumen nicht positiv)".into()));
        }
        let bounds = m.bounds().unwrap_or(Aabb::point(Vec3::ZERO));
        let tol = tolerance(&bounds);
        let (c, rad) = (bounds.center(), bounds.size().length() * 0.5);
        let mut planes: Vec<Plane> = Vec::new();
        let mut areas: Vec<f64> = Vec::new();
        for t in 0..m.triangle_count() {
            let [a, b, cc] = m.corners(t);
            let cr = (b - a).cross(cc - a);
            let area2 = cr.length();
            if !(area2 > 0.0) {
                continue;
            }
            let n = (cr / area2).unsigned_zeros();
            // through a corner: a face across an axis gets its coordinate exactly
            let p = Plane { n, w: -n.dot(a) + 0.0 };
            match planes.iter().position(|q| near(q, &p, c, rad, tol)) {
                Some(g) => {
                    if area2 > areas[g] {
                        planes[g] = p;
                        areas[g] = area2;
                    }
                }
                None => {
                    planes.push(p);
                    areas.push(area2);
                }
            }
        }
        for &v in &m.positions {
            for p in &planes {
                let d = p.eval(v);
                if d > tol {
                    return Err(Error::Invalid(alloc::format!(
                        "das Mesh ist nicht konvex: die Ecke {} liegt {} m vor der Fläche mit der Normale {}; Boolesche Operationen nehmen konvexe Körper (Quader, Zylinder, Prismen konvexer Profile ...) oder Ergebnisse von geom.union, geom.difference, geom.intersection",
                        fmt3(v), crate::fnum(d), fmt3(p.n)
                    )));
                }
            }
        }
        let cell: Vec<PRef> = (0..planes.len() as u32).map(|i| i << 1).collect();
        let mut ctx = Ctx::new(&planes, bounds);
        let Some(poly) = ctx.polytope(&cell)? else {
            return Err(Error::Invalid("das Mesh ist flach (kein Volumen)".into()));
        };
        Ok(Solid { planes, cells: vec![poly.supports()], bounds }.pruned())
    }

    /// the number of cells (convex pieces)
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// the number of planes the cells use
    pub fn plane_count(&self) -> usize {
        self.planes.len()
    }

    /// whether it has no cell (nothing left)
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// the planes of each cell (each pointing out of its cell)
    pub fn cells(&self) -> Vec<Vec<Plane>> {
        self.cells.iter().map(|c| c.iter().map(|&r| if r & 1 == 1 { self.planes[pid(r)].flipped() } else { self.planes[pid(r)] }).collect()).collect()
    }

    /// the box around the operands it came from (at least the solid's own)
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// the solid mapped by `a` (planes and bounds; cells that shared a plane still do)
    pub fn transformed(&self, a: &Affine) -> Solid {
        let b = self.bounds;
        let mut nb: Option<Aabb> = None;
        for i in 0..8 {
            let p = Vec3::new(if i & 1 == 0 { b.min.x } else { b.max.x }, if i & 2 == 0 { b.min.y } else { b.max.y }, if i & 4 == 0 { b.min.z } else { b.max.z });
            let q = a.apply(p);
            nb = Some(match nb {
                Some(mut x) => {
                    x.grow(q);
                    x
                }
                None => Aabb::point(q),
            });
        }
        Solid { planes: self.planes.iter().map(|p| p.transformed(a)).collect(), cells: self.cells.clone(), bounds: nb.unwrap_or(b) }
    }

    /// only the planes the cells use, numbered in their old order
    fn pruned(self) -> Solid {
        let mut used = vec![false; self.planes.len()];
        for c in &self.cells {
            for &r in c {
                used[pid(r)] = true;
            }
        }
        let mut map = vec![0u32; self.planes.len()];
        let mut planes = Vec::new();
        for (i, p) in self.planes.iter().enumerate() {
            if used[i] {
                map[i] = planes.len() as u32;
                planes.push(*p);
            }
        }
        let cells = self.cells.iter().map(|c| c.iter().map(|&r| (map[pid(r)] << 1) | (r & 1)).collect()).collect();
        Solid { planes, cells, bounds: self.bounds }
    }

    /// The planes of both, `o`'s welded to this one's where they are one within the tolerance (same or
    /// opposite side): (planes, this one's cells, o's cells, the bounds of both).
    #[allow(clippy::type_complexity)]
    fn combine(&self, o: &Solid) -> (Vec<Plane>, Vec<Vec<PRef>>, Vec<Vec<PRef>>, Aabb) {
        let bounds = self.bounds.union(o.bounds);
        let tol = tolerance(&bounds);
        let (c, rad) = (bounds.center(), bounds.size().length() * 0.5);
        let mut planes = self.planes.clone();
        let mut map = Vec::with_capacity(o.planes.len());
        for q in &o.planes {
            let fq = q.flipped();
            let mut found = None;
            for (i, p) in planes.iter().enumerate() {
                if near(p, q, c, rad, tol) {
                    found = Some((i as u32) << 1);
                    break;
                }
                if near(p, &fq, c, rad, tol) {
                    found = Some(((i as u32) << 1) | 1);
                    break;
                }
            }
            map.push(match found {
                Some(r) => r,
                None => {
                    planes.push(*q);
                    ((planes.len() - 1) as u32) << 1
                }
            });
        }
        let ocells = o.cells.iter().map(|cell| cell.iter().map(|&r| map[pid(r)] ^ (r & 1)).collect()).collect();
        (planes, self.cells.clone(), ocells, bounds)
    }

    fn polys(ctx: &mut Ctx, cells: &[Vec<PRef>]) -> Result<Vec<Poly>, Error> {
        let mut out = Vec::with_capacity(cells.len());
        for c in cells {
            if let Some(p) = ctx.polytope(c)? {
                out.push(p);
            }
        }
        Ok(out)
    }

    fn from_polys(planes: Vec<Plane>, polys: &[Poly], bounds: Aabb) -> Result<Solid, Error> {
        if polys.len() > MAX_CELLS {
            return Err(Error::TooBig(alloc::format!("Boolesche Operation: mehr als {} konvexe Teile", MAX_CELLS)));
        }
        Ok(Solid { planes, cells: polys.iter().map(|p| p.supports()).collect(), bounds }.pruned())
    }

    /// whether a cell is flat after welding (a plane from both sides)
    fn flat(cell: &[PRef]) -> bool {
        cell.iter().any(|&r| cell.contains(&flip(r)))
    }

    /// Everything in this solid or in `o`.
    pub fn union(&self, o: &Solid) -> Result<Solid, Error> {
        let (planes, a, b, bounds) = self.combine(o);
        let mut ctx = Ctx::new(&planes, bounds);
        let mut cells = Self::polys(&mut ctx, &a)?;
        let mine: Vec<&Vec<PRef>> = a.iter().filter(|c| !Self::flat(c)).collect();
        for cb in &b {
            let Some(p) = ctx.polytope(cb)? else { continue };
            let mut pieces = vec![p];
            for ca in &mine {
                let mut next = Vec::new();
                for x in pieces {
                    next.extend(ctx.subtract(x, ca)?);
                }
                pieces = next;
                if pieces.is_empty() {
                    break;
                }
            }
            cells.extend(pieces);
            if cells.len() > MAX_CELLS {
                break;
            }
        }
        Self::from_polys(planes, &cells, bounds)
    }

    /// What is in this solid but not in `o`.
    pub fn difference(&self, o: &Solid) -> Result<Solid, Error> {
        let (planes, a, b, bounds) = self.combine(o);
        let mut ctx = Ctx::new(&planes, bounds);
        let mut cells = Self::polys(&mut ctx, &a)?;
        for cb in b.iter().filter(|c| !Self::flat(c)) {
            let mut next = Vec::new();
            for x in cells {
                next.extend(ctx.subtract(x, cb)?);
            }
            cells = next;
            if cells.len() > MAX_CELLS {
                break;
            }
        }
        Self::from_polys(planes, &cells, bounds)
    }

    /// What is in both.
    pub fn intersection(&self, o: &Solid) -> Result<Solid, Error> {
        let (planes, a, b, bounds) = self.combine(o);
        let mut ctx = Ctx::new(&planes, bounds);
        let mine = Self::polys(&mut ctx, &a)?;
        let mut cells = Vec::new();
        for cb in b.iter().filter(|c| !Self::flat(c)) {
            for x in &mine {
                let mut r = Some(x.clone());
                for &p in cb {
                    let Some(cur) = r else { break };
                    r = ctx.split(cur, p, false)?.0;
                }
                if let Some(r) = r {
                    cells.push(r);
                }
            }
        }
        Self::from_polys(planes, &cells, bounds)
    }

    /// The surface as one closed mesh with normals smooth up to `smooth_deg` degrees (0: flat) and box UV in
    /// metres.  Err for an empty solid.
    pub fn to_mesh(&self, smooth_deg: f64) -> Result<Mesh, Error> {
        let (m, _) = self.surface()?;
        let m = if smooth_deg <= 0.0 { m.flat() } else { m.smooth(smooth_deg) };
        Ok(m.uv_box(1.0))
    }

    /// The surface: a raw mesh (positions and triangles, each position its own point) and each triangle's plane.
    fn surface(&self) -> Result<(Mesh, Vec<PRef>), Error> {
        if self.cells.is_empty() {
            return Err(Error::Invalid("das Ergebnis ist leer (nichts übrig)".into()));
        }
        let mut ctx = Ctx::new(&self.planes, self.bounds);
        let polys = Self::polys(&mut ctx, &self.cells)?;
        surface_of(&mut ctx, polys, &self.bounds)
    }
}

/// the surface of cells (polytopes of ctx): a raw mesh (positions and triangles, each position its own point)
/// and each triangle's plane
fn surface_of(ctx: &mut Ctx, polys: Vec<Poly>, bounds: &Aabb) -> Result<(Mesh, Vec<PRef>), Error> {
    // the faces by plane, from the front (+) and from behind (-)
    let mut groups: BTreeMap<usize, (Vec<Face>, Vec<Face>)> = BTreeMap::new();
    for p in polys {
        for f in p.faces {
            let g = groups.entry(pid(f.s)).or_default();
            if f.s & 1 == 0 { g.0.push(f) } else { g.1.push(f) }
        }
    }
    // where two cells touch, their faces lie on one plane from both sides: what is left of each side
    let mut faces: Vec<Face> = Vec::new();
    for (_, (front, back)) in groups {
        for (mine, other) in [(&front, &back), (&back, &front)] {
            let mut cur: Vec<Face> = mine.clone();
            for o in other {
                let mut next = Vec::new();
                for f in cur {
                    next.extend(ctx.face_minus(f, o)?);
                }
                cur = next;
            }
            faces.extend(cur);
        }
    }
    // the points: the corners, rounded; equal exact points round to equal bits
    let mut point_of: BTreeMap<[u64; 3], u32> = BTreeMap::new();
    let mut pts: Vec<[f64; 3]> = Vec::new();
    let mut rep: Vec<u32> = Vec::new(); // a corner (meet) at each point
    let mut fpts: Vec<Vec<u32>> = Vec::with_capacity(faces.len());
    for f in &faces {
        let mut v = Vec::with_capacity(f.v.len());
        for &c in &f.v {
            let x = ctx.coord(c);
            let key = [(x[0] + 0.0).to_bits(), (x[1] + 0.0).to_bits(), (x[2] + 0.0).to_bits()];
            let id = *point_of.entry(key).or_insert_with(|| {
                pts.push(x);
                rep.push(c);
                (pts.len() - 1) as u32
            });
            v.push(id);
        }
        fpts.push(v);
    }
    // points in the middle of an edge (a face of a neighbour ends there): into the edge, in order
    let mut by_x: Vec<u32> = (0..pts.len() as u32).collect();
    by_x.sort_unstable_by(|&a, &b| pts[a as usize][0].total_cmp(&pts[b as usize][0]).then(a.cmp(&b)));
    let xs: Vec<f64> = by_x.iter().map(|&i| pts[i as usize][0]).collect();
    // each ring entry: the point and the face's edges it lies on (a corner two, a point inserted into an
    // edge one); in exact geometry every face is convex, its corners turn strictly, the inserted points lie
    // on its edges, so three entries lie on one line exactly when they share an edge
    let mut loops: Vec<Vec<(u32, [u32; 2])>> = Vec::with_capacity(faces.len());
    for (f, fp) in faces.iter().zip(&fpts) {
        let k = f.e.len();
        let mut seq = Vec::with_capacity(k);
        for j in 0..k {
            let (a, b) = (fp[(j + k - 1) % k], fp[j]);
            let (pa, pb) = (pts[a as usize], pts[b as usize]);
            let lo = [pa[0].min(pb[0]), pa[1].min(pb[1]), pa[2].min(pb[2])];
            let hi = [pa[0].max(pb[0]), pa[1].max(pb[1]), pa[2].max(pb[2])];
            let start = xs.partition_point(|&x| x < lo[0]);
            let mut on: Vec<u32> = Vec::new();
            for &q in by_x[start..].iter().take_while(|&&q| pts[q as usize][0] <= hi[0]) {
                if q == a || q == b {
                    continue;
                }
                let pq = pts[q as usize];
                if pq[1] < lo[1] || pq[1] > hi[1] || pq[2] < lo[2] || pq[2] > hi[2] {
                    continue;
                }
                let c = rep[q as usize];
                if ctx.side(f.s, c) == 0 && ctx.side(f.e[j], c) == 0 && ctx.side(f.e[(j + k - 1) % k], c) < 0 && ctx.side(f.e[(j + 1) % k], c) < 0 {
                    on.push(q);
                }
            }
            if !on.is_empty() {
                // along the edge: by the coordinate that changes most, from a to b (rounding keeps the order)
                let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
                let ax = (0..3).max_by(|&i, &j| abs(d[i]).total_cmp(&abs(d[j])).then(j.cmp(&i))).unwrap_or(0);
                on.sort_unstable_by(|&p, &q| {
                    let (u, w) = (pts[p as usize][ax], pts[q as usize][ax]);
                    if d[ax] >= 0.0 { u.total_cmp(&w) } else { w.total_cmp(&u) }
                });
                seq.extend(on.iter().map(|&q| (q, [j as u32, u32::MAX])));
            }
            seq.push((b, [j as u32, ((j + 1) % k) as u32]));
        }
        loops.push(seq);
    }
    // points closer than the tolerance fall together (where planes almost meet in one point)
    let tol = tolerance(bounds);
    let mut g = Groups::new(pts.len());
    for i in 0..by_x.len() {
        let p = pts[by_x[i] as usize];
        for &q in &by_x[i + 1..] {
            let pq = pts[q as usize];
            if pq[0] - p[0] > tol {
                break;
            }
            if abs(pq[1] - p[1]) <= tol && abs(pq[2] - p[2]) <= tol {
                g.unite(by_x[i], q);
            }
        }
    }
    // triangles: each face cut into ears by the exact geometry alone, then the points that fell together
    // joined (a triangle with a point twice goes)
    let mut tris: Vec<([u32; 3], PRef)> = Vec::new();
    for (f, seq) in faces.iter().zip(&loops) {
        let Some(ears) = ear_cut(seq) else { return Err(bad("eine Fläche ließ sich nicht in Dreiecke zerlegen")) };
        for t in ears {
            let q = [g.find(seq[t[0]].0), g.find(seq[t[1]].0), g.find(seq[t[2]].0)];
            if q[0] != q[1] && q[1] != q[2] && q[0] != q[2] {
                tris.push((q, f.s));
            }
        }
    }
    // two triangles on the same three points facing apart (a fold that fell together): both go
    let key = |t: &[u32; 3]| {
        let mut k = *t;
        k.sort_unstable();
        k
    };
    let mut seen: BTreeMap<[u32; 3], Vec<usize>> = BTreeMap::new();
    for (i, (t, _)) in tris.iter().enumerate() {
        seen.entry(key(t)).or_default().push(i);
    }
    let mut gone = vec![false; tris.len()];
    for (_, list) in seen {
        if list.len() == 2 {
            let (a, b) = (tris[list[0]].0, tris[list[1]].0);
            // the same cyclic order is the same facing
            let same = (0..3).any(|r| a == [b[r], b[(r + 1) % 3], b[(r + 2) % 3]]);
            if !same {
                gone[list[0]] = true;
                gone[list[1]] = true;
            }
        }
    }
    flip_slivers(&mut tris, &mut gone, &pts);
    let mut used: Vec<u32> = vec![u32::MAX; pts.len()];
    let mut positions: Vec<Vec3> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut tri_planes: Vec<PRef> = Vec::new();
    for (i, (t, s)) in tris.iter().enumerate() {
        if gone[i] {
            continue;
        }
        for &q in t {
            if used[q as usize] == u32::MAX {
                used[q as usize] = positions.len() as u32;
                let p = pts[q as usize];
                positions.push(Vec3::new(p[0], p[1], p[2]).unsigned_zeros());
            }
            indices.push(used[q as usize]);
        }
        tri_planes.push(*s);
    }
    if indices.len() / 3 > MAX_TRIANGLES {
        return Err(Error::TooBig(alloc::format!("Boolesche Operation: mehr als {} Dreiecke", MAX_TRIANGLES)));
    }
    if indices.is_empty() {
        return Err(Error::Invalid("das Ergebnis ist leer (nichts übrig)".into()));
    }
    Ok((Mesh::from_triangles(positions, indices)?, tri_planes))
}

/// Slivers left where points fell together (a point within rounding of a long edge, but farther than the
/// tolerance from its ends): the long edge is flipped with the neighbour across it, which splits that
/// neighbour at the point instead.  The surface stays where it was (the sliver had no area), the mesh stays
/// closed.  A few rounds, as long as something changes.
fn flip_slivers(tris: &mut [([u32; 3], PRef)], gone: &mut [bool], pts: &[[f64; 3]]) {
    let at = |q: u32| Vec3::new(pts[q as usize][0], pts[q as usize][1], pts[q as usize][2]);
    for _ in 0..8 {
        let mut edge: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        for (i, (t, _)) in tris.iter().enumerate() {
            if !gone[i] {
                for k in 0..3 {
                    edge.insert((t[k], t[(k + 1) % 3]), i);
                }
            }
        }
        let mut changed = false;
        for i in 0..tris.len() {
            if gone[i] {
                continue;
            }
            let t = tris[i].0;
            let p = [at(t[0]), at(t[1]), at(t[2])];
            let len2 = [(p[1] - p[0]).length_sq(), (p[2] - p[1]).length_sq(), (p[0] - p[2]).length_sq()];
            let k = if len2[0] >= len2[1] && len2[0] >= len2[2] {
                0
            } else if len2[1] >= len2[2] {
                1
            } else {
                2
            };
            let twice = (p[1] - p[0]).cross(p[2] - p[0]).length();
            if twice > crate::DEGENERATE_RATIO * len2[k] {
                continue;
            }
            // the long edge u -> v, the point w opposite; the neighbour runs v -> u to its point x
            let (u, v, w) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            let Some(&j) = edge.get(&(v, u)) else { continue };
            if j == i || gone[j] {
                continue;
            }
            let nb = tris[j].0;
            let Some(r) = (0..3).find(|&r| nb[r] == v && nb[(r + 1) % 3] == u) else { continue };
            let x = nb[(r + 2) % 3];
            if x == w || edge.contains_key(&(w, x)) || edge.contains_key(&(x, w)) {
                continue;
            }
            let s = tris[j].1;
            for (a, b) in [(u, v), (v, w), (w, u), (v, u), (u, x), (x, v)] {
                edge.remove(&(a, b));
            }
            tris[i] = ([w, u, x], s);
            tris[j] = ([x, v, w], s);
            for (a, b, n) in [(w, u, i), (u, x, i), (x, w, i), (x, v, j), (v, w, j), (w, x, j)] {
                edge.insert((a, b), n);
            }
            changed = true;
        }
        if !changed {
            break;
        }
    }
}

/// Triangles of a face's ring by ear cutting on the exact geometry alone, no coordinates: the face is convex,
/// so three entries lie on one line exactly when they share an edge of the face, and a diagonal passes
/// through another entry exactly when the three share an edge.  An ear (prev, tip, next) is not on one line
/// and no other entry lies on prev-next.  Indices into the ring; None when no ear is left.
fn ear_cut(ring: &[(u32, [u32; 2])]) -> Option<Vec<[usize; 3]>> {
    let n = ring.len();
    if n < 3 {
        return Some(Vec::new());
    }
    let on = |i: usize, e: u32| e != u32::MAX && (ring[i].1[0] == e || ring[i].1[1] == e);
    let common = |a: usize, b: usize| -> [u32; 2] {
        let mut out = [u32::MAX; 2];
        let mut k = 0;
        for &e in &ring[a].1 {
            if e != u32::MAX && on(b, e) {
                out[k] = e;
                k += 1;
            }
        }
        out
    };
    let line = |a: usize, b: usize, c: usize| common(a, b).iter().any(|&e| on(c, e));
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut start = 0usize;
    while idx.len() > 3 {
        let m = idx.len();
        let mut cut = None;
        for s in 0..m {
            let i = (start + s) % m;
            let (p, t, q) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            if line(p, t, q) {
                continue;
            }
            let shared = common(p, q);
            let blocked = shared.iter().any(|&e| e != u32::MAX && idx.iter().any(|&x| x != p && x != t && x != q && on(x, e)));
            if !blocked {
                cut = Some(i);
                break;
            }
        }
        let i = cut?;
        let m = idx.len();
        out.push([idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]]);
        idx.remove(i);
        start = if i == 0 { 0 } else { i - 1 };
    }
    if !line(idx[0], idx[1], idx[2]) {
        out.push([idx[0], idx[1], idx[2]]);
    }
    Some(out)
}

// ---------------------------------------------------------------- bevel

/// the point where three planes meet in f64 (Cramer), None when they hardly do
fn meet3(p: [&Plane; 3], rhs: [f64; 3]) -> Option<Vec3> {
    let (a, b, c) = (p[0].n, p[1].n, p[2].n);
    let det = a.dot(b.cross(c));
    if !(abs(det) > 1e-12) {
        return None;
    }
    let x = (b.cross(c) * rhs[0] + c.cross(a) * rhs[1] + a.cross(b) * rhs[2]) / det;
    Some(x)
}

impl Solid {
    /// The planes of a bevel of this convex solid (one cell): its faces, along each edge `segments` planes
    /// through points of a circular arc of radius `radius` (whose axis lies `radius` inside both faces), at
    /// each corner of three faces the triangles of a spherical patch of that radius.  `only`: just the edges
    /// along that axis (within a degree), no corners.
    fn bevel_planes(&self, radius: f64, segments: usize, only: Option<Axis>) -> Result<BevelPlanes, Error> {
        if self.cells.len() != 1 {
            return Err(Error::Invalid(alloc::format!(
                "abschrägen geht nur bei einem konvexen Körper, dieser hat {} Teile (vor der Booleschen Operation abschrägen)",
                self.cells.len()
            )));
        }
        let mut ctx = Ctx::new(&self.planes, self.bounds);
        let Some(poly) = ctx.polytope(&self.cells[0])? else { return Err(Error::Invalid("der Körper ist leer".into())) };
        let plane = |r: PRef| -> Plane { if r & 1 == 1 { self.planes[pid(r)].flipped() } else { self.planes[pid(r)] } };
        let faces: Vec<PRef> = poly.supports();
        let originals: Vec<Plane> = faces.iter().map(|&r| plane(r)).collect();
        let k = segments;
        let mut planes: Vec<Plane> = originals.clone();
        let centre = self.bounds.center();
        let fidx = |r: PRef| faces.iter().position(|&x| x == r);
        // edges: each pair of faces that share one, once
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (i, f) in poly.faces.iter().enumerate() {
            for &e in &f.e {
                if let Some(j) = fidx(e) {
                    if i < j && !edges.contains(&(i, j)) {
                        edges.push((i, j));
                    }
                }
            }
        }
        let axis = only.map(|a| match a {
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        });
        // the segments count per quarter turn: an edge turning by phi degrees gets round(k phi / 90), at least one
        let seg_of = |a: &Plane, b: &Plane| -> usize { ((k as f64 * crate::math::angle_deg(a.n, b.n) / 90.0 + 0.5) as usize).clamp(1, 4 * k) };
        let arc = |a: Vec3, b: Vec3, i: usize, n: usize| -> Vec3 { (a * (n - i) as f64 + b * i as f64).normalize_or_zero() };
        let mut bevelled: Vec<(usize, usize, usize)> = Vec::new();
        for &(i, j) in &edges {
            let (p1, p2) = (&originals[i], &originals[j]);
            let d = p1.n.cross(p2.n);
            let Some(dir) = d.try_normalize() else { continue };
            if crate::math::angle_deg(p1.n, p2.n) < 1.0 {
                continue;
            }
            if let Some(ax) = axis {
                if abs(dir.dot(ax)) < 0.99985 {
                    continue;
                }
            }
            let dp = Plane { n: dir, w: -dir.dot(centre) };
            let Some(q) = meet3([p1, p2, &dp], [-p1.w - radius, -p2.w - radius, -dp.w]) else { continue };
            let ke = seg_of(p1, p2);
            for s in 0..ke {
                let (a, b) = (arc(p1.n, p2.n, s, ke), arc(p1.n, p2.n, s + 1, ke));
                let m = (a + b).normalize_or_zero().unsigned_zeros();
                let x = q + a * radius;
                planes.push(Plane { n: m, w: -m.dot(x) + 0.0 });
            }
            bevelled.push((i, j, ke));
        }
        // the bevel must fit: the faces of bevelled edges moved inward by the radius still enclose a volume
        let inner: Vec<Plane> = originals
            .iter()
            .enumerate()
            .map(|(i, p)| Plane { n: p.n, w: p.w + if bevelled.iter().any(|&(a, b, _)| a == i || b == i) { radius } else { 0.0 } })
            .collect();
        let inner_refs: Vec<PRef> = (0..inner.len() as u32).map(|i| i << 1).collect();
        let mut ictx = Ctx::new(&inner, self.bounds);
        if ictx.polytope(&inner_refs)?.is_none() {
            return Err(Error::Param(alloc::format!("die Breite {} ist zu groß für diesen Körper (die Rundungen würden sich überschneiden)", crate::fnum(radius))));
        }
        // corners where exactly three faces meet, every edge of them bevelled
        if only.is_none() {
            let mut at: BTreeMap<[u64; 3], Vec<usize>> = BTreeMap::new();
            for (i, f) in poly.faces.iter().enumerate() {
                for &v in &f.v {
                    let c = ctx.coord(v);
                    let key = [(c[0] + 0.0).to_bits(), (c[1] + 0.0).to_bits(), (c[2] + 0.0).to_bits()];
                    let l = at.entry(key).or_default();
                    if !l.contains(&i) {
                        l.push(i);
                    }
                }
            }
            for (_, fs) in at {
                if fs.len() != 3 {
                    continue;
                }
                let seg = |a: usize, b: usize| bevelled.iter().find(|e| (e.0, e.1) == (a.min(b), a.max(b))).map(|e| e.2);
                let (Some(k01), Some(k12), Some(k02)) = (seg(fs[0], fs[1]), seg(fs[1], fs[2]), seg(fs[0], fs[2])) else { continue };
                let (p1, p2, p3) = (&originals[fs[0]], &originals[fs[1]], &originals[fs[2]]);
                let Some(c) = meet3([p1, p2, p3], [-p1.w - radius, -p2.w - radius, -p3.w - radius]) else { continue };
                let mut tri = |a: Vec3, b: Vec3, cc: Vec3| {
                    let (x, y, z) = (c + a * radius, c + b * radius, c + cc * radius);
                    let Some(mut n) = (y - x).cross(z - x).try_normalize() else { return };
                    if n.dot((x + y + z) / 3.0 - c) < 0.0 {
                        n = -n;
                    }
                    let n = n.unsigned_zeros();
                    planes.push(Plane { n, w: -n.dot(x) + 0.0 });
                };
                if k01 == k12 && k12 == k02 {
                    // the grid normalize(i n1 + j n2 + (k - i - j) n3), its k^2 triangles
                    let kk = k01;
                    let dir = |i: usize, j: usize| -> Vec3 { (p1.n * i as f64 + p2.n * j as f64 + p3.n * (kk - i - j) as f64).normalize_or_zero() };
                    for i in 0..kk {
                        for j in 0..kk - i {
                            tri(dir(i, j), dir(i + 1, j), dir(i, j + 1));
                            if i + j + 2 <= kk {
                                tri(dir(i + 1, j), dir(i + 1, j + 1), dir(i, j + 1));
                            }
                        }
                    }
                } else {
                    // rows from the face opposite the edge with the fewest segments: row i of R has
                    // ceil(bottom i / R) segments, matching the two edges from the apex (R) and the bottom edge
                    let (apex, nb, nc, bottom, rows) = if k12 <= k01 && k12 <= k02 {
                        (p1.n, p2.n, p3.n, k12, k01.max(k02))
                    } else if k02 <= k01 {
                        (p2.n, p1.n, p3.n, k02, k01.max(k12))
                    } else {
                        (p3.n, p1.n, p2.n, k01, k02.max(k12))
                    };
                    let row = |i: usize| -> Vec<Vec3> {
                        if i == 0 {
                            return alloc::vec![apex];
                        }
                        let si = (bottom * i).div_ceil(rows).max(1);
                        (0..=si)
                            .map(|j| {
                                let side = nb * ((si - j) as f64 / si as f64) + nc * (j as f64 / si as f64);
                                (apex * (rows - i) as f64 + side * i as f64).normalize_or_zero()
                            })
                            .collect()
                    };
                    let mut prev = row(0);
                    for i in 1..=rows {
                        let cur = row(i);
                        let (la, lb) = (prev.len() - 1, cur.len() - 1);
                        let (mut a, mut b) = (0, 0);
                        while a < la || b < lb {
                            if a < la && (b == lb || (a + 1) * lb <= (b + 1) * la) {
                                tri(prev[a], prev[a + 1], cur[b]);
                                a += 1;
                            } else {
                                tri(prev[a], cur[b], cur[b + 1]);
                                b += 1;
                            }
                        }
                        prev = cur;
                    }
                }
            }
        }
        let cell: Vec<PRef> = (0..planes.len() as u32).map(|i| i << 1).collect();
        let original = originals.len();
        let mut bctx = Ctx::new(&planes, self.bounds);
        let Some(p) = bctx.polytope(&cell)? else { return Err(Error::Invalid("das Abschrägen ließ nichts übrig".into())) };
        let solid = Solid { planes, cells: vec![p.supports()], bounds: self.bounds };
        Ok(BevelPlanes { solid, original, segments, ctx: bctx, poly: p })
    }

    /// The solid with its edges and corners bevelled, and its surface ([`bevel`]): `segments` 1 chamfers (a
    /// flat face along each edge, a triangle at each corner of three faces), more round them with that many
    /// faces per quarter turn (an edge turning by another angle gets proportionally many, at least one), the
    /// faces cut back by `radius` at right angles.  `only`: just the edges along that axis, the corners stay.
    /// Only for one convex cell.
    pub fn bevel(&self, radius: f64, segments: usize, only: Option<Axis>) -> Result<(Solid, Mesh), Error> {
        check_bevel(radius, segments)?;
        let b = self.bevel_planes(radius, segments, only)?;
        let solid = b.solid.clone().pruned();
        Ok((solid, bevel_mesh(b)?))
    }
}

fn check_bevel(radius: f64, segments: usize) -> Result<(), Error> {
    if !(radius > 0.0 && radius.is_finite()) {
        return Err(Error::Param(alloc::format!("die Breite muss eine Zahl größer 0 sein, nicht {}", radius)));
    }
    if !(1..=64).contains(&segments) {
        return Err(Error::Param(alloc::format!("die Zahl der Segmente muss zwischen 1 und 64 liegen, nicht {}", segments)));
    }
    Ok(())
}

// ---------------------------------------------------------------- on meshes

fn solid_of(m: &Mesh, what: &str) -> Result<Solid, Error> {
    Solid::from_convex(m).map_err(|e| Error::Invalid(alloc::format!("{}: {}", what, e.message())))
}

/// The union of convex closed meshes as one mesh, normals smooth up to 30 degrees.
pub fn union(meshes: &[&Mesh]) -> Result<Mesh, Error> {
    let Some((first, rest)) = meshes.split_first() else { return Err(Error::Param("braucht mindestens ein Mesh".into())) };
    let mut s = solid_of(first, "Mesh 1")?;
    for (i, m) in rest.iter().enumerate() {
        s = s.union(&solid_of(m, &alloc::format!("Mesh {}", i + 2))?)?;
    }
    s.to_mesh(crate::DEFAULT_SMOOTH_DEG)
}

/// The convex closed mesh `a` less each of `minus`, as one mesh, normals smooth up to 30 degrees.
pub fn difference(a: &Mesh, minus: &[&Mesh]) -> Result<Mesh, Error> {
    let mut s = solid_of(a, "Mesh 1")?;
    for (i, m) in minus.iter().enumerate() {
        s = s.difference(&solid_of(m, &alloc::format!("Mesh {}", i + 2))?)?;
    }
    s.to_mesh(crate::DEFAULT_SMOOTH_DEG)
}

/// The intersection of convex closed meshes as one mesh, normals smooth up to 30 degrees.
pub fn intersection(meshes: &[&Mesh]) -> Result<Mesh, Error> {
    let Some((first, rest)) = meshes.split_first() else { return Err(Error::Param("braucht mindestens ein Mesh".into())) };
    let mut s = solid_of(first, "Mesh 1")?;
    for (i, m) in rest.iter().enumerate() {
        s = s.intersection(&solid_of(m, &alloc::format!("Mesh {}", i + 2))?)?;
    }
    s.to_mesh(crate::DEFAULT_SMOOTH_DEG)
}

/// A convex closed mesh with its edges and corners bevelled ([`Solid::bevel`]); the flat faces keep flat
/// normals, the rounded ones get normals across the arcs (where an arc meets a face, the face's normal: no
/// seam, no shading across the face).
pub fn bevel(m: &Mesh, radius: f64, segments: usize, only: Option<Axis>) -> Result<Mesh, Error> {
    check_bevel(radius, segments)?;
    let s = solid_of(m, "abschrägen")?;
    let b = s.bevel_planes(radius, segments, only)?;
    bevel_mesh(b)
}

/// The surface of a bevel with its normals: a corner of a face of the original solid gets the face's normal.
/// A chamfer (one segment) is flat.  A rounded bevel's corner gets the normal of the original face it lies on
/// where the arc runs into that face (exactly one such face, the face's normal within 50 degrees of the bevel
/// face's: tangent there), else the angle-weighted normal of the bevel faces around its point.
fn bevel_mesh(b: BevelPlanes) -> Result<Mesh, Error> {
    let BevelPlanes { solid, original, segments, mut ctx, poly } = b;
    let s = &solid;
    let (raw, tri_planes) = surface_of(&mut ctx, vec![poly], &s.bounds)?;
    let plane = |r: PRef| -> Plane { if r & 1 == 1 { s.planes[pid(r)].flipped() } else { s.planes[pid(r)] } };
    let is_original = |r: PRef| pid(r) < original;
    let tol = tolerance(&s.bounds) * 10.0;
    let np = raw.positions.len();
    // the angle-weighted normals of the bevel faces around each point
    let mut acc = vec![Vec3::ZERO; np];
    for t in 0..raw.triangle_count() {
        if is_original(tri_planes[t]) {
            continue;
        }
        let tri = raw.triangle(t);
        let p = raw.corners(t);
        let n = plane(tri_planes[t]).n;
        for c in 0..3 {
            let (a, bb) = (p[(c + 1) % 3] - p[c], p[(c + 2) % 3] - p[c]);
            let w = crate::math::atan2(a.cross(bb).length(), a.dot(bb));
            acc[tri[c] as usize] += n * w;
        }
    }
    // cos 50 degrees
    let tangent = 0.642_787_609_686_539_3;
    let mut normal = Vec::with_capacity(raw.indices.len());
    for t in 0..raw.triangle_count() {
        let r = tri_planes[t];
        let tri = raw.triangle(t);
        let own = plane(r).n;
        for &v in &tri {
            let n = if is_original(r) || segments == 1 {
                own
            } else {
                let mut face = None;
                let mut count = 0;
                for (j, q) in s.planes.iter().enumerate().take(original) {
                    if abs(q.eval(raw.positions[v as usize])) <= tol && q.n.dot(own) >= tangent {
                        face = Some(j);
                        count += 1;
                    }
                }
                match (count, face) {
                    (1, Some(j)) => s.planes[j].n,
                    _ => acc[v as usize].try_normalize().unwrap_or(own),
                }
            };
            normal.push(n);
        }
    }
    let uv = vec![[0.0; 2]; raw.indices.len()];
    Ok(crate::normals::from_corners(&raw, &normal, &uv).uv_box(1.0))
}
