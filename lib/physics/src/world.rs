// world -- bodies (a shape, a placement, layers) in three dynamic AABB trees, and the queries over all of them
//
// A body starts in a tree of the bodies that stand still, with its exact box: meshes in one, the other
// shapes in another; it goes to the tree of the moving bodies when it is first moved (with a box 0.05 m
// larger, so small moves keep their leaf).  So the large box of a map never widens the nodes of the props
// and figures: rays and casts look at the map's mesh first, and its hit prunes the other trees
// (docs/PHYSICS.md, measurements).
//
// Results never depend on the shape of the tree: a first hit is the smallest (t, body, triangle), lists
// are sorted by body and triangle (rays by t first).  So a client that holds only some of the server's
// bodies gets the same answer from them, as long as the bodies have the same ids.

use crate::gjk::CAST_SKIN;
use crate::math::{Aabb, Iso, Vec3, max};
use crate::query::{self, CastHit, Contact, PointHit, RayHit};
use crate::shape::Shape;
use crate::tree::DynamicTree;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// a body of a [`World`]: an index and a generation (an id stays dead after remove, also when the index
/// is used again).  Ordered by index, then generation: the tie-break of the queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyId {
    index: u32,
    generation: u32,
}

impl BodyId {
    #[inline]
    pub fn index(self) -> u32 {
        self.index
    }

    #[inline]
    pub fn generation(self) -> u32 {
        self.generation
    }
}

/// a body: what [`World::add`] took, readable through [`World::body`]
#[derive(Clone, Debug)]
pub struct Body {
    pub shape: Shape,
    pub iso: Iso,
    /// the layers the body is in (bit mask); a query sees it when its filter's mask shares a bit
    pub layers: u32,
    /// free for the caller (an entity id)
    pub user: u64,
    leaf: u32,
    /// its tree: STILL_MESHES, STILL or MOVING
    tree: u8,
}

const STILL_MESHES: u8 = 0;
const STILL: u8 = 1;
const MOVING: u8 = 2;

impl Body {
    /// has the body been moved since it was added (it is in the tree of the moving bodies)
    pub fn is_moving(&self) -> bool {
        self.tree == MOVING
    }
}

/// which bodies a query sees
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    /// bodies with `layers & mask != 0`
    pub mask: u32,
    /// one body to leave out (the query's own, e.g. a figure's capsule)
    pub exclude: Option<BodyId>,
}

impl Filter {
    /// every body
    pub const ALL: Filter = Filter { mask: u32::MAX, exclude: None };

    /// the bodies in one of the layers of `mask`
    pub const fn mask(mask: u32) -> Filter {
        Filter { mask, exclude: None }
    }

    /// the same without body `id`
    pub const fn excluding(self, id: BodyId) -> Filter {
        Filter { mask: self.mask, exclude: Some(id) }
    }

    #[inline]
    fn sees(&self, id: BodyId, layers: u32) -> bool {
        layers & self.mask != 0 && self.exclude != Some(id)
    }
}

impl Default for Filter {
    fn default() -> Filter {
        Filter::ALL
    }
}

struct Slot {
    generation: u32,
    body: Option<Body>,
}

/// A set of bodies and the queries over them.  Bodies move with [`World::set_iso`]; nothing moves by itself
/// (no dynamics in this stage).
pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    /// the meshes that never moved, the other bodies that never moved, the moving bodies
    trees: [DynamicTree; 3],
    margin: f64,
    count: usize,
}

impl Default for World {
    fn default() -> World {
        World::new()
    }
}

/// (t, body, triangle) ordering of first hits
#[inline]
fn earlier(t: f64, id: BodyId, tri: u32, bt: f64, bid: BodyId, btri: u32) -> bool {
    t < bt || (t == bt && (id < bid || (id == bid && tri < btri)))
}

impl World {
    /// an empty world; a moving body's box in the tree is 0.05 m larger than the body on every side
    pub fn new() -> World {
        World::with_margin(0.05)
    }

    /// an empty world with this margin around a moving body's box (a body that has never moved has none)
    pub fn with_margin(margin: f64) -> World {
        World { slots: Vec::new(), free: Vec::new(), trees: [DynamicTree::new(), DynamicTree::new(), DynamicTree::new()], margin: max(margin, 0.0), count: 0 }
    }

    /// add a body in layer 1
    pub fn add(&mut self, shape: Shape, iso: Iso) -> BodyId {
        self.add_with_layers(shape, iso, 1)
    }

    /// add a body in `layers`
    pub fn add_with_layers(&mut self, shape: Shape, iso: Iso, layers: u32) -> BodyId {
        let aabb = shape.aabb(&iso);
        let index = match self.free.pop() {
            Some(i) => i,
            None => {
                self.slots.push(Slot { generation: 0, body: None });
                (self.slots.len() - 1) as u32
            }
        };
        let tree = if shape.is_convex() { STILL } else { STILL_MESHES };
        let leaf = self.trees[tree as usize].insert(aabb, index);
        let slot = &mut self.slots[index as usize];
        slot.body = Some(Body { shape, iso, layers, user: 0, leaf, tree });
        self.count += 1;
        BodyId { index, generation: slot.generation }
    }

    fn slot(&self, id: BodyId) -> Option<&Body> {
        let s = self.slots.get(id.index as usize)?;
        if s.generation != id.generation {
            return None;
        }
        s.body.as_ref()
    }

    fn slot_mut(&mut self, id: BodyId) -> Option<&mut Body> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.generation != id.generation {
            return None;
        }
        s.body.as_mut()
    }

    /// remove a body -> it, None when the id is dead
    pub fn remove(&mut self, id: BodyId) -> Option<Body> {
        let s = self.slots.get_mut(id.index as usize)?;
        if s.generation != id.generation || s.body.is_none() {
            return None;
        }
        let body = s.body.take()?;
        s.generation = s.generation.wrapping_add(1);
        self.trees[body.tree as usize].remove(body.leaf);
        self.free.push(id.index);
        self.count -= 1;
        Some(body)
    }

    /// the body, None when the id is dead
    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.slot(id)
    }

    /// move a body -> false when the id is dead.  The first move takes it to the tree of the moving bodies.
    pub fn set_iso(&mut self, id: BodyId, iso: Iso) -> bool {
        let margin = self.margin;
        let index = id.index;
        let Some(b) = self.slot_mut(id) else {
            return false;
        };
        b.iso = iso;
        let tight = b.shape.aabb(&iso);
        let (leaf, tree) = (b.leaf, b.tree);
        if tree != MOVING {
            self.trees[tree as usize].remove(leaf);
            let nl = self.trees[MOVING as usize].insert(tight.expanded(margin), index);
            if let Some(b) = self.slot_mut(id) {
                b.leaf = nl;
                b.tree = MOVING;
            }
        } else if !self.trees[MOVING as usize].aabb(leaf).contains(&tight) {
            self.trees[MOVING as usize].update(leaf, tight.expanded(margin));
        }
        true
    }

    pub fn set_layers(&mut self, id: BodyId, layers: u32) -> bool {
        match self.slot_mut(id) {
            Some(b) => {
                b.layers = layers;
                true
            }
            None => false,
        }
    }

    pub fn set_user(&mut self, id: BodyId, user: u64) -> bool {
        match self.slot_mut(id) {
            Some(b) => {
                b.user = user;
                true
            }
            None => false,
        }
    }

    /// the number of bodies
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// every body with its id, by index
    pub fn bodies(&self) -> impl Iterator<Item = (BodyId, &Body)> + '_ {
        self.slots.iter().enumerate().filter_map(|(i, s)| s.body.as_ref().map(|b| (BodyId { index: i as u32, generation: s.generation }, b)))
    }

    /// the broad phase (for tests and statistics): the trees of the meshes and of the other bodies that
    /// never moved, and the one of the moving bodies
    pub fn trees(&self) -> [&DynamicTree; 3] {
        [&self.trees[0], &self.trees[1], &self.trees[2]]
    }

    /// the leaves of the trees along a ray, the still meshes first; `f(item, tmax)` returns the new tmax
    fn ray_trees(&self, o: Vec3, d: Vec3, ext: Vec3, mut tmax: f64, mut f: impl FnMut(u32, f64) -> f64) {
        for tree in &self.trees {
            tmax = tree.ray(o, d, ext, tmax, &mut f);
        }
    }

    #[inline]
    fn id_of(&self, index: u32) -> BodyId {
        BodyId { index, generation: self.slots[index as usize].generation }
    }

    // ------------------------------------------------------------ queries

    /// the first hit of the ray origin + dir * t, 0 <= t <= max_t
    pub fn ray_cast(&self, origin: Vec3, dir: Vec3, max_t: f64, filter: &Filter) -> Option<(BodyId, RayHit)> {
        let mut best: Option<(BodyId, RayHit)> = None;
        self.ray_trees(origin, dir, Vec3::ZERO, max_t, |item, tmax| {
            let id = self.id_of(item);
            let Some(b) = self.slots[item as usize].body.as_ref() else {
                return tmax;
            };
            if !filter.sees(id, b.layers) {
                return tmax;
            }
            if let Some(h) = query::ray(&b.shape, &b.iso, origin, dir, tmax) {
                if best.as_ref().is_none_or(|(bid, bh)| earlier(h.t, id, h.triangle, bh.t, *bid, bh.triangle)) {
                    best = Some((id, h));
                    return h.t;
                }
            }
            tmax
        });
        best
    }

    /// every hit of the ray (one per convex body, one per triangle of a mesh), appended to `out` sorted by
    /// t, body, triangle
    pub fn ray_cast_all(&self, origin: Vec3, dir: Vec3, max_t: f64, filter: &Filter, out: &mut Vec<(BodyId, RayHit)>) {
        let start = out.len();
        let mut hits: Vec<RayHit> = Vec::new();
        self.ray_trees(origin, dir, Vec3::ZERO, max_t, |item, tmax| {
            let id = self.id_of(item);
            if let Some(b) = self.slots[item as usize].body.as_ref() {
                if filter.sees(id, b.layers) {
                    hits.clear();
                    query::ray_all(&b.shape, &b.iso, origin, dir, max_t, &mut hits);
                    out.extend(hits.iter().map(|h| (id, *h)));
                }
            }
            tmax
        });
        out[start..].sort_unstable_by(|a, b| {
            a.1.t.partial_cmp(&b.1.t).unwrap_or(Ordering::Equal).then(a.0.cmp(&b.0)).then(a.1.triangle.cmp(&b.1.triangle))
        });
    }

    /// the first body convex `shape`, moved from `iso` by `motion`, touches (see [`query::cast`])
    pub fn cast(&self, shape: &Shape, iso: &Iso, motion: Vec3, filter: &Filter) -> Option<(BodyId, CastHit)> {
        if !shape.is_convex() {
            return None;
        }
        let bx = shape.aabb(iso);
        let c = bx.center();
        let pad = CAST_SKIN + 1e-9 + 1e-12 * max(c.abs().max_elem(), bx.half_extents().max_elem());
        let ext = bx.half_extents() + Vec3::splat(pad);
        let mut best: Option<(BodyId, CastHit)> = None;
        self.ray_trees(c, motion, ext, 1.0, |item, tmax| {
            let id = self.id_of(item);
            let Some(b) = self.slots[item as usize].body.as_ref() else {
                return tmax;
            };
            if !filter.sees(id, b.layers) {
                return tmax;
            }
            if let Some(h) = query::cast(shape, iso, motion, &b.shape, &b.iso) {
                if best.as_ref().is_none_or(|(bid, bh)| earlier(h.fraction, id, h.triangle, bh.fraction, *bid, bh.triangle)) {
                    best = Some((id, h));
                    return h.fraction;
                }
            }
            tmax
        });
        best
    }

    /// the candidates of a box query, by body
    fn candidates(&self, bx: &Aabb, filter: &Filter) -> Vec<u32> {
        let mut c: Vec<u32> = Vec::new();
        for tree in &self.trees {
            tree.query(bx, |item| {
                if let Some(b) = self.slots[item as usize].body.as_ref() {
                    if filter.sees(self.id_of(item), b.layers) {
                        c.push(item);
                    }
                }
            });
        }
        c.sort_unstable();
        c
    }

    /// the bodies convex `shape` at `iso` touches or overlaps, appended to `out` by id
    pub fn overlap(&self, shape: &Shape, iso: &Iso, filter: &Filter, out: &mut Vec<BodyId>) {
        if !shape.is_convex() {
            return;
        }
        for item in self.candidates(&shape.aabb(iso), filter) {
            let b = self.slots[item as usize].body.as_ref();
            if let Some(b) = b {
                if query::intersects(shape, iso, &b.shape, &b.iso) {
                    out.push(self.id_of(item));
                }
            }
        }
    }

    /// the contacts of convex `shape` at `iso` with every body (every triangle) within `margin`, appended
    /// to `out` by body and triangle: what pushes a figure out of the level
    pub fn contacts(&self, shape: &Shape, iso: &Iso, margin: f64, filter: &Filter, out: &mut Vec<(BodyId, Contact)>) {
        if !shape.is_convex() {
            return;
        }
        let mut cs: Vec<Contact> = Vec::new();
        for item in self.candidates(&shape.aabb(iso).expanded(max(margin, 0.0)), filter) {
            if let Some(b) = self.slots[item as usize].body.as_ref() {
                cs.clear();
                query::contacts(shape, iso, &b.shape, &b.iso, margin, &mut cs);
                let id = self.id_of(item);
                out.extend(cs.iter().map(|c| (id, *c)));
            }
        }
    }

    /// the nearest point of any body to `point` within `max_dist` (equal distance: the lower body, triangle)
    pub fn closest_point(&self, point: Vec3, max_dist: f64, filter: &Filter) -> Option<(BodyId, PointHit)> {
        let bx = if max_dist.is_finite() {
            Aabb::new(point - Vec3::splat(max_dist), point + Vec3::splat(max_dist))
        } else {
            Aabb::new(Vec3::splat(f64::NEG_INFINITY), Vec3::splat(f64::INFINITY))
        };
        let mut best: Option<(BodyId, PointHit)> = None;
        let mut limit = max_dist;
        for item in self.candidates(&bx, filter) {
            if let Some(b) = self.slots[item as usize].body.as_ref() {
                if let Some(h) = query::closest_point(&b.shape, &b.iso, point, limit) {
                    let id = self.id_of(item);
                    if best.as_ref().is_none_or(|(bid, bh)| earlier(h.distance, id, h.triangle, bh.distance, *bid, bh.triangle)) {
                        best = Some((id, h));
                        limit = h.distance;
                    }
                }
            }
        }
        best
    }
}
