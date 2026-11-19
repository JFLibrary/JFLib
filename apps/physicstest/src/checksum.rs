// checksum -- the determinism harness: a fixed set of scenarios, every result folded bit for bit into one
// FNV-1a hash.  The same number on Windows, Linux, macOS, the browser and Android means every query gave
// the same bits there (client prediction against the server, G7e).

use crate::scene;
use crate::tests::{random_convex, slide};
use crate::Rng;
use alloc::sync::Arc;
use alloc::vec::Vec;
use physics::math;
use physics::{ConvexHull, Filter, Iso, Shape, Vec3, World, query};

struct Hash {
    h: u64,
    n: u64,
}

impl Hash {
    fn u(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.h = (self.h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.n += 1;
    }

    fn f(&mut self, v: f64) {
        self.u(v.to_bits());
    }

    fn v(&mut self, v: Vec3) {
        self.f(v.x);
        self.f(v.y);
        self.f(v.z);
    }
}

/// -> (the checksum, the number of values in it)
pub fn run() -> (u64, u64) {
    let mut h = Hash { h: 0xcbf2_9ce4_8422_2325, n: 0 };
    let mut rng = Rng::new(2026);

    // the arithmetic underneath
    for _ in 0..20_000 {
        let x = f64::from_bits(rng.next_u64() & 0x7fef_ffff_ffff_ffff);
        h.f(math::sqrt(x));
        h.f(math::sqrt_portable(x));
        let a = rng.range(-50.0, 50.0);
        let (s, c) = math::sincos(a);
        h.f(s);
        h.f(c);
        h.f(math::atan2(rng.range(-5.0, 5.0), rng.range(-5.0, 5.0)));
        let q = rng.quat();
        h.v(q.rotate(rng.vec(-10.0, 10.0)));
    }

    // hulls
    for k in 0..20 {
        let n = 8 + k * 9;
        let pts: Vec<Vec3> = (0..n).map(|_| if k % 2 == 0 { rng.vec(-1.0, 1.0) } else { rng.dir() * 1.5 }).collect();
        let hull = ConvexHull::new(&pts).expect("hull");
        h.u(hull.points().len() as u64);
        for p in hull.points() {
            h.v(*p);
        }
        for (n, d) in hull.planes() {
            h.v(*n);
            h.f(*d);
        }
    }

    // the map's tree and edges
    let data = scene::map(60, 40);
    let map = Arc::new(data.build());
    for n in map.nodes() {
        for k in 0..3 {
            h.u(n.min[k].to_bits() as u64);
            h.u(n.max[k].to_bits() as u64);
        }
        h.u(n.first as u64);
        h.u(n.count as u64);
    }
    for id in (0..data.triangles() as u32).step_by(7) {
        h.u(map.edge_flags(id).map(|f| f as u64).unwrap_or(999));
    }

    // a world: the map and 300 bodies, queries between moves
    let mut w = World::new();
    w.add(Shape::Mesh(map.clone()), Iso::IDENTITY);
    let mut ids = Vec::new();
    for _ in 0..300 {
        let k = rng.below(4);
        let s = random_convex(&mut rng, k, 0.6);
        let iso = Iso::new(Vec3::new(rng.range(0.0, 60.0), rng.range(0.0, 8.0), rng.range(0.0, 60.0)), rng.quat());
        ids.push(w.add_with_layers(s, iso, 1 | (1 << rng.below(3))));
    }
    for round in 0..10 {
        for _ in 0..40 {
            let i = rng.below(ids.len());
            let iso = Iso::new(Vec3::new(rng.range(0.0, 60.0), rng.range(0.0, 8.0), rng.range(0.0, 60.0)), rng.quat());
            w.set_iso(ids[i], iso);
        }
        if round % 3 == 2 {
            let i = rng.below(ids.len());
            w.remove(ids.swap_remove(i));
            let k = rng.below(4);
        let s = random_convex(&mut rng, k, 0.6);
            ids.push(w.add(s, Iso::at(Vec3::new(rng.range(0.0, 60.0), 3.0, rng.range(0.0, 60.0)))));
        }
        let filter = if round % 2 == 0 { Filter::ALL } else { Filter::mask(0b100) };
        for q in 0..300 {
            let o = Vec3::new(rng.range(-2.0, 62.0), rng.range(-1.0, 10.0), rng.range(-2.0, 62.0));
            let d = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 0.3), rng.range(-1.0, 1.0)) * rng.range(1.0, 50.0);
            match w.ray_cast(o, d, 1.0, &filter) {
                Some((id, r)) => {
                    h.u(id.index() as u64);
                    h.f(r.t);
                    h.v(r.normal);
                    h.u(r.triangle as u64);
                }
                None => h.u(u64::MAX),
            }
            if q % 10 == 0 {
                let mut all = Vec::new();
                w.ray_cast_all(o, d, 1.0, &filter, &mut all);
                h.u(all.len() as u64);
                for (id, r) in &all {
                    h.u(id.index() as u64);
                    h.f(r.t);
                }
            }
            if q % 2 == 0 {
                let s = random_convex(&mut rng, q % 4, 0.5);
                let iso = Iso::new(o, rng.quat());
                match w.cast(&s, &iso, d * 0.2, &filter) {
                    Some((id, c)) => {
                        h.u(id.index() as u64);
                        h.f(c.fraction);
                        h.v(c.normal);
                        h.v(c.point);
                        h.f(c.depth);
                        h.u(c.triangle as u64);
                    }
                    None => h.u(u64::MAX),
                }
                if q % 6 == 0 {
                    let mut ov = Vec::new();
                    w.overlap(&s, &iso, &filter, &mut ov);
                    h.u(ov.len() as u64);
                    let mut cs = Vec::new();
                    w.contacts(&s, &iso, 0.05, &filter, &mut cs);
                    for (id, c) in &cs {
                        h.u(id.index() as u64);
                        h.f(c.depth);
                        h.v(c.normal);
                        h.v(c.point);
                    }
                }
            }
            if q % 3 == 0 {
                if let Some((id, p)) = w.closest_point(o, 4.0, &filter) {
                    h.u(id.index() as u64);
                    h.f(p.distance);
                    h.v(p.point);
                }
            }
        }
    }

    // pairs: distances and penetrations of every kind
    for k in 0..3000 {
        let a = random_convex(&mut rng, k % 4, 1.0);
        let b = random_convex(&mut rng, (k / 4) % 4, 1.0);
        let ia = Iso::new(rng.vec(-1.5, 1.5), rng.quat());
        let ib = Iso::new(rng.vec(-1.5, 1.5), rng.quat());
        if let Some(s) = query::separation(&a, &ia, &b, &ib, f64::INFINITY) {
            h.f(s.distance);
            h.v(s.normal);
            h.v(s.point_a);
            h.v(s.point_b);
        }
        if let Some(c) = query::cast(&a, &ia, rng.vec(-3.0, 3.0), &b, &ib) {
            h.f(c.fraction);
            h.v(c.normal);
        }
    }

    // collide and slide over the map
    let ms = Shape::Mesh(map);
    for walker in 0..6 {
        let shape = if walker % 2 == 0 { Shape::sphere(0.35) } else { Shape::capsule(0.4, 0.3) };
        let mut pos = Vec3::new(10.0 + walker as f64 * 7.0, 8.0, 20.0 + walker as f64 * 3.0);
        let mut vel = Vec3::new(0.12, -0.2, 0.05 * walker as f64);
        for step in 0..600 {
            let (p, v) = slide(&shape, pos, vel, &ms);
            pos = p;
            vel = v;
            if step % 50 == 0 {
                let (s, c) = math::sincos(step as f64 * 0.37 + walker as f64);
                vel = Vec3::new(c * 0.15, vel.y, s * 0.15);
            }
            h.v(pos);
        }
    }
    (h.h, h.n)
}

/// the character controller's checksum (character.rs): its own number, so the one above stays what it was
/// -> (the checksum, the number of values in it)
pub fn controller() -> (u64, u64) {
    let mut h = Hash { h: 0xcbf2_9ce4_8422_2325, n: 0 };
    crate::character::checksum(|v| h.u(v));
    (h.h, h.n)
}
