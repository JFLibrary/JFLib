// bench -- the numbers of docs/PHYSICS.md: a map of about 50 000 triangles, the tree's build, rays and
// capsule casts per second, bytes per triangle; each the median of `runs`

use crate::scene;
use crate::{Rng, out};
use alloc::format;
use alloc::vec::Vec;
use physics::{Filter, Iso, Shape, Vec3, World, query};

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

pub fn run(runs: usize) {
    let data = scene::map(155, 160);
    out(&format!("bench: map of {} triangles, {} vertices, {} runs\n", data.triangles(), data.pos.len(), runs));
    let mut build_ms = Vec::new();
    let mut mesh = None;
    for _ in 0..runs {
        let t0 = plat::time_us();
        let m = data.build();
        build_ms.push((plat::time_us() - t0) as f64 / 1000.0);
        mesh = Some(m);
    }
    let mesh = mesh.expect("built");
    let bytes = mesh.memory_bytes();
    out(&format!(
        "build: {:.1} ms (median; runs {:?}), {} nodes, depth {}\n",
        median(&mut build_ms.clone()),
        build_ms.iter().map(|v| (v * 10.0).round_det() / 10.0).collect::<Vec<_>>(),
        mesh.nodes().len(),
        mesh.depth()
    ));
    out(&format!("memory: {} bytes, {:.1} bytes per triangle\n", bytes, bytes as f64 / data.triangles() as f64));
    let size = 155.0;
    let ms = Shape::mesh(mesh);

    // rays: from above the ground, down and at a slant, 40 m long
    let n_rays = 200_000;
    let mut rng = Rng::new(77);
    let rays: Vec<(Vec3, Vec3)> = (0..n_rays)
        .map(|_| {
            let o = Vec3::new(rng.range(0.0, size), rng.range(2.0, 8.0), rng.range(0.0, size));
            let d = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 0.1), rng.range(-1.0, 1.0)).normalize_or_zero();
            (o, d)
        })
        .collect();
    let mut per_s = Vec::new();
    let mut hits = 0;
    for _ in 0..runs {
        hits = 0;
        let t0 = plat::time_us();
        for (o, d) in &rays {
            if query::ray(&ms, &Iso::IDENTITY, *o, *d, 40.0).is_some() {
                hits += 1;
            }
        }
        let s = (plat::time_us() - t0) as f64 / 1e6;
        per_s.push(n_rays as f64 / s);
    }
    out(&format!("rays: {:.0} per second (median; {} of {} hit)\n", median(&mut per_s), hits, n_rays));

    // capsule casts: a figure-sized capsule just above the ground, moved 0.5 .. 3 m sideways and a little down
    let cap = Shape::capsule(0.5, 0.3);
    let n_casts = 40_000;
    let mut casts: Vec<(Vec3, Vec3)> = Vec::with_capacity(n_casts);
    while casts.len() < n_casts {
        let x = rng.range(2.0, size - 2.0);
        let z = rng.range(2.0, size - 2.0);
        let Some(g) = query::ray(&ms, &Iso::IDENTITY, Vec3::new(x, 20.0, z), Vec3::new(0.0, -1.0, 0.0), 40.0) else {
            continue;
        };
        let p = g.point + Vec3::new(0.0, 0.8 + 0.01, 0.0);
        let ang = rng.range(0.0, 6.283);
        let l = rng.range(0.5, 3.0);
        let m = Vec3::new(physics::math::cos(ang) * l, -0.2, physics::math::sin(ang) * l);
        casts.push((p, m));
    }
    let mut per_s = Vec::new();
    for _ in 0..runs {
        hits = 0;
        let t0 = plat::time_us();
        for (p, m) in &casts {
            if query::cast(&cap, &Iso::at(*p), *m, &ms, &Iso::IDENTITY).is_some() {
                hits += 1;
            }
        }
        let s = (plat::time_us() - t0) as f64 / 1e6;
        per_s.push(n_casts as f64 / s);
    }
    out(&format!("capsule casts: {:.0} per second (median; {} of {} hit)\n", median(&mut per_s), hits, n_casts));

    // rays through a world that holds only the map: the cost of the world's own tree and bookkeeping
    let mut w = World::new();
    w.add(ms.clone(), Iso::IDENTITY);
    let mut per_s = Vec::new();
    for _ in 0..runs {
        hits = 0;
        let t0 = plat::time_us();
        for (o, d) in &rays {
            if w.ray_cast(*o, *d * 40.0, 1.0, &Filter::ALL).is_some() {
                hits += 1;
            }
        }
        let s = (plat::time_us() - t0) as f64 / 1e6;
        per_s.push(n_rays as f64 / s);
    }
    out(&format!("world rays (only the map): {:.0} per second (median; {} hit)\n", median(&mut per_s), hits));

    // the same casts through a world with 500 more bodies around
    for k in 0..500 {
        let s = crate::tests::random_convex(&mut rng, k % 4, 0.5);
        w.add(s, Iso::new(Vec3::new(rng.range(0.0, size), rng.range(0.0, 4.0), rng.range(0.0, size)), rng.quat()));
    }
    let mut per_s = Vec::new();
    for _ in 0..runs {
        hits = 0;
        let t0 = plat::time_us();
        for (p, m) in &casts {
            if w.cast(&cap, &Iso::at(*p), *m, &Filter::ALL).is_some() {
                hits += 1;
            }
        }
        let s = (plat::time_us() - t0) as f64 / 1e6;
        per_s.push(n_casts as f64 / s);
    }
    out(&format!("world capsule casts (map + 500 bodies): {:.0} per second (median; {} hit)\n", median(&mut per_s), hits));
    let mut per_s = Vec::new();
    for _ in 0..runs {
        hits = 0;
        let t0 = plat::time_us();
        for (o, d) in &rays {
            if w.ray_cast(*o, *d * 40.0, 1.0, &Filter::ALL).is_some() {
                hits += 1;
            }
        }
        let s = (plat::time_us() - t0) as f64 / 1e6;
        per_s.push(n_rays as f64 / s);
    }
    out(&format!("world rays: {:.0} per second (median; {} hit)\n", median(&mut per_s), hits));
    let mut leaves = 0u64;
    let t0 = plat::time_us();
    for (o, d) in &rays {
        w.trees()[1].ray(*o, *d * 40.0, Vec3::ZERO, 1.0, |_, t| {
            leaves += 1;
            t
        });
    }
    let us = (plat::time_us() - t0) as f64 / n_rays as f64;
    out(&format!("world tree of the 500 still bodies: height {}, {:.2} leaves per ray without pruning, {:.3} us per ray for the tree alone\n", w.trees()[1].height(), leaves as f64 / n_rays as f64, us));
}

trait RoundDet {
    fn round_det(self) -> f64;
}

impl RoundDet for f64 {
    fn round_det(self) -> f64 {
        (self + 0.5) as i64 as f64
    }
}
