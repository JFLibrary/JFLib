// tests -- the groups of --check (docs/PHYSICS.md, "Tests")

use crate::refs::{self, Poly};
use crate::scene;
use crate::{Ctx, Rng, out};
use rt::fx as rtfx;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use physics::math::{self, sqrt};
use physics::{Aabb, BodyId, CAST_SKIN, ConvexHull, Filter, HullError, Iso, MeshError, MeshOptions, Quat, Shape, TriMesh, Vec3, World, query};

pub const ALL: &[&str] = &["math", "hull", "ray", "distance", "cast", "tunnel", "seams", "bvh", "world", "fuzz", "edge"];

pub fn run(names: &[&str], quick: bool) -> i32 {
    let (mut checks, mut failed) = (0u64, 0u64);
    for &name in names {
        let Some(&group) = ALL.iter().find(|g| **g == name) else {
            out(&format!("physicstest: unknown group {} (groups: {})\n", name, ALL.join(" ")));
            return 2;
        };
        let mut c = Ctx::new(group, quick);
        let t0 = plat::time_us();
        match group {
            "math" => math_tests(&mut c),
            "hull" => hull_tests(&mut c),
            "ray" => ray_tests(&mut c),
            "distance" => distance_tests(&mut c),
            "cast" => cast_tests(&mut c),
            "tunnel" => tunnel_tests(&mut c),
            "seams" => seam_tests(&mut c),
            "bvh" => bvh_tests(&mut c),
            "world" => world_tests(&mut c),
            "fuzz" => fuzz_tests(&mut c),
            _ => edge_tests(&mut c),
        }
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<9} {:>8} checks, {} failed ({} ms; casts that fell back so far: {})\n", group, c.checks, c.failed, ms, physics::cast_fallbacks()));
        checks += c.checks;
        failed += c.failed;
    }
    out(&format!("physicstest: {} von {} Prüfungen bestanden\n", checks - failed, checks));
    if failed == 0 { 0 } else { 1 }
}

// ---------------------------------------------------------------- helpers

fn bits(v: Vec3) -> [u64; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

fn unit(v: Vec3) -> bool {
    (v.length() - 1.0).abs() < 1e-9
}

fn fin(v: Vec3) -> bool {
    v.is_finite()
}

/// a random convex shape: 0 sphere, 1 capsule, 2 box, 3 hull
pub fn random_convex(rng: &mut Rng, kind: usize, scale: f64) -> Shape {
    match kind {
        0 => Shape::sphere(rng.range(0.05, 1.0) * scale),
        1 => Shape::capsule(rng.range(0.05, 1.0) * scale, rng.range(0.05, 0.6) * scale),
        2 => Shape::cuboid(Vec3::new(rng.range(0.05, 1.0), rng.range(0.05, 1.0), rng.range(0.05, 1.0)) * scale),
        _ => {
            let n = 5 + rng.below(8);
            let pts: Vec<Vec3> = (0..n).map(|_| rng.vec(-1.0, 1.0) * scale).collect();
            match Shape::hull_from_points(&pts) {
                Ok(s) => s,
                Err(_) => Shape::cuboid(Vec3::splat(0.5 * scale)),
            }
        }
    }
}

fn random_iso(rng: &mut Rng, spread: f64) -> Iso {
    Iso::new(rng.vec(-spread, spread), if rng.below(4) == 0 { Quat::IDENTITY } else { rng.quat() })
}

/// a mesh of one triangle (every edge open, so active: no normal is changed)
fn tri_mesh(a: Vec3, b: Vec3, c: Vec3) -> Shape {
    let f = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
    Shape::mesh(TriMesh::new(&[f(a), f(b), f(c)], &[0, 1, 2]).expect("triangle"))
}

/// the corners of a one-triangle mesh (as stored, f32 exact)
fn tri_of(s: &Shape) -> [Vec3; 3] {
    match s {
        Shape::Mesh(m) => m.triangles().next().map(|(_, t)| t).expect("a triangle"),
        _ => panic!("no mesh"),
    }
}

fn random_tri(rng: &mut Rng, spread: f64) -> Shape {
    loop {
        let c = rng.vec(-spread, spread);
        let (a, b, d) = (c + rng.vec(-1.5, 1.5), c + rng.vec(-1.5, 1.5), c + rng.vec(-1.5, 1.5));
        if (b - a).cross(d - a).length() > 0.05 {
            return tri_mesh(a, b, d);
        }
    }
}

fn poly_of(s: &Shape, iso: &Iso) -> Poly {
    match s {
        Shape::Mesh(_) => {
            let t = tri_of(s);
            Poly::triangle(iso.transform_point(t[0]), iso.transform_point(t[1]), iso.transform_point(t[2]))
        }
        _ => Poly::of(s, iso),
    }
}

// ---------------------------------------------------------------- math

fn math_tests(c: &mut Ctx) {
    let mut rng = Rng::new(1);
    // the square root: lib/physics' portable one against rt's correctly rounded one (the processor's on
    // x86-64 and ARM64, rt's integer root elsewhere), bit for bit
    let check_sqrt = |c: &mut Ctx, x: f64| {
        let a = math::sqrt_portable(x);
        let b = rtfx::sqrtd(x);
        let s = math::sqrt(x);
        let same = (a.is_nan() && b.is_nan() && s.is_nan()) || (a.to_bits() == b.to_bits() && s.to_bits() == b.to_bits());
        c.check(same, || format!("sqrt({:e} = {:016x}): portable {:016x}, rt {:016x}, sqrt {:016x}", x, x.to_bits(), a.to_bits(), b.to_bits(), s.to_bits()));
    };
    let specials = [
        0.0,
        -0.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
        -1.0,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::from_bits(0x000f_ffff_ffff_ffff),
        f64::MAX,
        1.0,
        2.0,
        4.0,
        0.25,
        3.0,
        1e-300,
        1e300,
    ];
    for x in specials {
        check_sqrt(c, x);
    }
    for _ in 0..c.n(200_000) {
        let b = rng.next_u64() & 0x7fff_ffff_ffff_ffff;
        let x = f64::from_bits(b);
        if x.is_finite() {
            check_sqrt(c, x);
        }
        let y = rng.range(0.0, 1e4);
        check_sqrt(c, y);
    }
    // near the midpoints: x = the double nearest to ((2q + 1) / 2)^2 and its neighbours
    for _ in 0..c.n(100_000) {
        let q = (1u64 << 52) | (rng.next_u64() & ((1u64 << 52) - 1));
        let m = 2 * q as u128 + 1;
        let sq = m * m;
        let lz = sq.leading_zeros();
        let shift = 128 - lz - 53;
        let mut mant = (sq >> shift) as u64;
        if (sq >> (shift - 1)) & 1 != 0 {
            mant += 1;
        }
        if mant >= 1u64 << 53 {
            continue;
        }
        let e = shift as i32 - 2 + (rng.below(200) as i32 - 100) * 2;
        let x = mant as f64 * pow2(e);
        for k in [-2i64, -1, 0, 1, 2] {
            let xb = (x.to_bits() as i64 + k) as u64;
            check_sqrt(c, f64::from_bits(xb));
        }
    }
    // sin, cos, atan2 (their bits are the checksum's; here only sanity)
    let (s, co) = math::sincos(core::f64::consts::PI / 6.0);
    c.check((s - 0.5).abs() < 1e-15 && (co - 0.866_025_403_784_438_6).abs() < 1e-15, || format!("sincos(pi/6) = {} {}", s, co));
    for _ in 0..c.n(5000) {
        let x = rng.range(-100.0, 100.0);
        let (s, co) = math::sincos(x);
        c.check((s * s + co * co - 1.0).abs() < 1e-15 && s == math::sin(x) && co == math::cos(x), || format!("sincos({})", x));
        let a = math::atan2(s, co);
        let back = math::sincos(a);
        c.check((back.0 - s).abs() < 1e-14 && (back.1 - co).abs() < 1e-14, || format!("atan2 of sincos({})", x));
    }
    // rotations
    for _ in 0..c.n(20_000) {
        let q = rng.quat();
        let v = rng.vec(-10.0, 10.0);
        let m = q.to_mat3();
        let a = q.rotate(v);
        let b = m.mul_vec(v);
        c.check((a - b).length() < 1e-13, || format!("rotate {:?} {:?}: {:?} {:?}", q, v, a, b));
        c.check((q.inverse_rotate(a) - v).length() < 1e-13, || "inverse_rotate".into());
        c.check((a.length() - v.length()).abs() < 1e-13, || "length kept".into());
        let i1 = random_iso(&mut rng, 100.0);
        let i2 = random_iso(&mut rng, 100.0);
        let back = i1.inv_mul(&i1.mul(&i2));
        c.check((back.pos - i2.pos).length() < 1e-11 && (back.rot.w - i2.rot.w).abs() + (back.rot.x - i2.rot.x).abs() < 1e-13, || "inv_mul(mul)".into());
        let p = rng.vec(-50.0, 50.0);
        c.check((i1.inverse_transform_point(i1.transform_point(p)) - p).length() < 1e-11, || "transform back".into());
        let (f, t) = (rng.dir(), rng.dir());
        let r = Quat::from_rotation_arc(f, t);
        c.check((r.rotate(f) - t).length() < 1e-12, || format!("from_rotation_arc {:?} -> {:?}", f, t));
        let axis = rng.dir();
        let ang = rng.range(-3.0, 3.0);
        let qa = Quat::from_axis_angle(axis, ang);
        c.check((qa.rotate(axis) - axis).length() < 1e-13, || "axis kept".into());
    }
}

fn pow2(e: i32) -> f64 {
    let mut x = 1.0f64;
    if e >= 0 {
        for _ in 0..e {
            x *= 2.0;
        }
    } else {
        for _ in 0..-e {
            x *= 0.5;
        }
    }
    x
}

// ---------------------------------------------------------------- hull

fn hull_tests(c: &mut Ctx) {
    let mut rng = Rng::new(2);
    for case in 0..c.n(1500) {
        let kind = case % 6;
        let n = 4 + rng.below(if kind == 5 { 40 } else { 200 });
        let mut pts: Vec<Vec3> = Vec::with_capacity(n);
        for _ in 0..n {
            let p = match kind {
                0 => rng.vec(-1.0, 1.0),
                1 => rng.dir() * 2.0,
                2 => {
                    // on a box's surface: coplanar faces
                    let mut p = rng.vec(-1.0, 1.0);
                    let a = rng.below(3);
                    let s = if rng.below(2) == 0 { -1.0 } else { 1.0 };
                    match a {
                        0 => p.x = s,
                        1 => p.y = s,
                        _ => p.z = s,
                    }
                    p
                }
                3 => Vec3::new(rng.range(-1.0, 1.0), rng.range(-1e-3, 1e-3), rng.range(-1.0, 1.0)),
                4 => rng.vec(-1.0, 1.0) * 1000.0 + Vec3::new(1e5, -2e4, 3e3),
                _ => {
                    // few positions, many duplicates
                    let k = rng.below(6);
                    Vec3::new((k.min(3) as f64) - 1.5, ((k * 7) % 3) as f64 - 1.0, ((k * 5) % 4) as f64 - 2.0)
                }
            };
            pts.push(p);
        }
        if kind == 5 {
            pts.extend_from_slice(&[Vec3::new(-2.0, -2.0, -2.0), Vec3::new(2.0, -2.0, -2.0), Vec3::new(0.0, 2.0, -2.0), Vec3::new(0.0, 0.0, 2.0)]);
        }
        let h = match ConvexHull::new(&pts) {
            Ok(h) => h,
            Err(e) => {
                c.check(false, || format!("case {} kind {}: {:?}", case, kind, e));
                continue;
            }
        };
        let tol = h.tolerance() * 4.0 + 1e-12;
        let mut worst = f64::NEG_INFINITY;
        for p in &pts {
            for &(nrm, d) in h.planes() {
                worst = worst.max(nrm.dot(*p) - d);
            }
        }
        c.check(worst <= tol, || format!("case {} kind {}: a point {} outside (tol {})", case, kind, worst, tol));
        c.check(h.points().iter().all(|v| pts.contains(v)), || format!("case {}: a vertex that is no input point", case));
        // closed: every edge in exactly two faces, once each way; V - E + F = 2
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for f in h.faces() {
            for k in 0..3 {
                edges.push((f[k], f[(k + 1) % 3]));
            }
        }
        let mut sorted = edges.clone();
        sorted.sort_unstable();
        let dup = sorted.windows(2).any(|w| w[0] == w[1]);
        let twins = edges.iter().all(|&(a, b)| sorted.binary_search(&(b, a)).is_ok());
        let (v, e, f) = (h.points().len() as i64, edges.len() as i64 / 2, h.faces().len() as i64);
        c.check(!dup && twins && v - e + f == 2, || format!("case {} kind {}: not a closed polyhedron (V {} E {} F {})", case, kind, v, e, f));
        // the centre lies inside every face
        let cen = h.points().iter().fold(Vec3::ZERO, |s, p| s + *p) / h.points().len() as f64;
        c.check(h.planes().iter().all(|&(nrm, d)| nrm.dot(cen) < d), || format!("case {}: a face points inwards", case));
        // the support point against the input points
        for _ in 0..8 {
            let d = rng.dir();
            let best = pts.iter().fold(f64::NEG_INFINITY, |m, p| m.max(p.dot(d)));
            let s = h.support(d).dot(d);
            c.check(s >= best - tol * 4.0, || format!("case {}: support {} below {}", case, s, best));
        }
    }
    // a box from its corners and points on its faces: the volume
    for _ in 0..c.n(200) {
        let hs = rng.vec(0.1, 3.0);
        let mut pts = Vec::new();
        for k in 0..8 {
            pts.push(Vec3::new(if k & 1 != 0 { hs.x } else { -hs.x }, if k & 2 != 0 { hs.y } else { -hs.y }, if k & 4 != 0 { hs.z } else { -hs.z }));
        }
        for _ in 0..30 {
            let mut p = rng.vec(-1.0, 1.0).mul_elem(hs);
            p.y = if rng.below(2) == 0 { hs.y } else { -hs.y };
            pts.push(p);
        }
        let h = ConvexHull::new(&pts).expect("box hull");
        let vol = 8.0 * hs.x * hs.y * hs.z;
        c.check((h.volume() - vol).abs() < 1e-12 * vol.max(1.0) && h.points().len() == 8, || format!("box hull volume {} vs {}, {} vertices", h.volume(), vol, h.points().len()));
    }
    // refused input
    let flat: Vec<Vec3> = (0..20).map(|i| Vec3::new(i as f64, (i * i % 7) as f64, 0.0)).collect();
    c.check(ConvexHull::new(&flat).err() == Some(HullError::Flat), || "coplanar points".into());
    c.check(ConvexHull::new(&flat[..3]).err() == Some(HullError::Count), || "three points".into());
    let mut nan = flat.clone();
    nan[3].z = f64::NAN;
    c.check(ConvexHull::new(&nan).err() == Some(HullError::NotFinite), || "NaN".into());
    let line: Vec<Vec3> = (0..10).map(|i| Vec3::new(i as f64, 2.0 * i as f64, 0.5 * i as f64)).collect();
    c.check(ConvexHull::new(&line).err() == Some(HullError::Flat), || "collinear points".into());
}

// ---------------------------------------------------------------- ray

fn ray_tests(c: &mut Ctx) {
    let mut rng = Rng::new(3);
    // convex shapes against the bisection of the convex distance along the ray
    for case in 0..c.n(6000) {
        let kind = case % 4;
        let s = random_convex(&mut rng, kind, 1.0);
        let iso = random_iso(&mut rng, 2.0);
        let o = rng.vec(-5.0, 5.0);
        let aim = iso.pos + rng.vec(-1.0, 1.0);
        let d = if rng.below(5) == 0 { rng.dir() * rng.range(0.1, 3.0) } else { (aim - o) * rng.range(0.2, 1.5) };
        let tmax = rng.range(0.5, 4.0);
        let p = Poly::of(&s, &iso);
        let f = |t: f64| refs::point_dist(o + d * (t * tmax), &p);
        let hit = query::ray(&s, &iso, o, d, tmax);
        let (root, fmin) = refs::first_root(&f, -1e-9);
        match hit {
            Some(h) => {
                let at = refs::point_dist(o + d * h.t, &p);
                let ok_surface = if h.t == 0.0 { at <= 1e-9 } else { at.abs() <= 1e-9 * (1.0 + d.length() * tmax) };
                c.check(ok_surface, || format!("case {} kind {}: hit t {} not on the surface ({})", case, kind, h.t, at));
                let late = match root {
                    Some(r) => h.t <= r * tmax + 1e-9,
                    None => true,
                };
                c.check(late, || format!("case {} kind {}: hit t {} after the entry {:?}", case, kind, h.t, root.map(|r| r * tmax)));
                c.check(unit(h.normal) && h.normal.dot(d) <= 1e-12, || format!("case {} kind {}: normal {:?}", case, kind, h.normal));
                c.check(h.t >= 0.0 && h.t <= tmax, || "t outside".into());
            }
            None => {
                c.check(fmin >= -1e-9, || format!("case {} kind {}: missed (the ray enters by {})", case, kind, -fmin));
            }
        }
        // the sphere also against the geometric formula
        if let Shape::Sphere { radius } = s {
            let l = d.length();
            let dn = d / l;
            let oc = iso.pos - o;
            let tca = oc.dot(dn);
            let d2 = oc.length_sq() - tca * tca;
            if oc.length_sq() > radius * radius && tca > 0.0 && d2 < radius * radius * (1.0 - 1e-9) {
                let t = (tca - sqrt(radius * radius - d2)) / l;
                if t < tmax * (1.0 - 1e-9) {
                    let ok = hit.is_some_and(|h| (h.t - t).abs() <= 1e-12 * (1.0 + t));
                    c.check(ok, || format!("sphere: {:?} vs {}", hit.map(|h| h.t), t));
                }
            }
        }
    }
    // triangles: watertight against Moeller-Trumbore away from the edges
    for case in 0..c.n(20_000) {
        let m = random_tri(&mut rng, 1.0);
        let t = tri_of(&m);
        let o = rng.vec(-4.0, 4.0);
        let target = t[0] * 0.3 + t[1] * 0.3 + t[2] * 0.4 + rng.vec(-0.8, 0.8);
        let d = target - o;
        let hit = query::ray(&m, &Iso::IDENTITY, o, d, 2.0);
        if let Some((tt, u, v)) = refs::ray_tri_mt(o, d, t[0], t[1], t[2]) {
            let w = 1.0 - u - v;
            if u > 1e-7 && v > 1e-7 && w > 1e-7 && tt > 1e-9 && tt < 2.0 - 1e-9 {
                let ok = hit.is_some_and(|h| (h.t - tt).abs() <= 1e-12 * (1.0 + tt));
                c.check(ok, || format!("case {}: triangle hit {:?} vs {}", case, hit.map(|h| h.t), tt));
            }
        } else {
            // a clear miss: the point of the plane is well outside
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            let den = n.dot(d);
            if den.abs() > 1e-6 {
                let tt = n.dot(t[0] - o) / den;
                let q = o + d * tt;
                let cp = refs::closest_on_triangle(q, t[0], t[1], t[2]);
                if (cp - q).length() > 1e-6 {
                    c.check(hit.is_none(), || format!("case {}: hit outside the triangle", case));
                }
            }
        }
        if let Some(h) = hit {
            c.check(unit(h.normal) && h.normal.dot(d) < 0.0, || "triangle normal".into());
        }
    }
    // watertight: rays at the vertices and edges of a grid with alternating diagonals, from above and below
    let grid = scene::flat(8, 1.0, 0.0).build();
    let g = Shape::mesh(grid);
    let mut misses = 0;
    let mut total = 0;
    for i in 0..=16 {
        for j in 0..=16 {
            let (x, z) = (i as f64 * 0.5, j as f64 * 0.5);
            for (dy, slant) in [(-1.0, 0.0), (1.0, 0.0), (-1.0, 0.3), (-1.0, -0.7), (1.0, 0.45)] {
                let d = Vec3::new(slant, dy, slant * 0.5);
                let o = Vec3::new(x, 0.0, z) - d * 3.0;
                // the outer border has no neighbour: a slanted ray exactly on it may round either way
                let border = x == 0.0 || x == 8.0 || z == 0.0 || z == 8.0;
                if border && slant != 0.0 {
                    continue;
                }
                total += 1;
                if query::ray(&g, &Iso::IDENTITY, o, d, 10.0).is_none() {
                    misses += 1;
                    if misses <= 3 {
                        let b = query::brute::ray(&g, &Iso::IDENTITY, o, d, 10.0);
                        out(&format!("  grid miss: o {:?} d {:?}, without the tree {:?}\n", o, d, b));
                    }
                }
            }
        }
    }
    for _ in 0..c.n(20_000) {
        let (x, z) = (rng.range(0.0, 8.0), rng.range(0.0, 8.0));
        let d = Vec3::new(rng.range(-0.5, 0.5), -1.0, rng.range(-0.5, 0.5));
        total += 1;
        if query::ray(&g, &Iso::IDENTITY, Vec3::new(x, 0.0, z) - d * 2.0, d, 10.0).is_none() {
            misses += 1;
            if misses <= 6 {
                let b = query::brute::ray(&g, &Iso::IDENTITY, Vec3::new(x, 0.0, z) - d * 2.0, d, 10.0);
                out(&format!("  grid miss: at {} {} d {:?}, without the tree {:?}\n", x, z, d, b));
            }
        }
    }
    c.check(misses == 0, || format!("{} of {} rays slipped through the grid", misses, total));
    // a closed box mesh: rays from inside always hit a wall
    let mut bm = scene::MeshData::new();
    scene::cuboid(&mut bm, [0.0, 0.0, 0.0], [1.0, 0.5, 0.75]);
    let bs = Shape::mesh(bm.build());
    let mut escaped = 0;
    for k in 0..c.n(20_000) {
        let o = if k % 3 == 0 { Vec3::ZERO } else { rng.vec(-0.4, 0.4) };
        // directions at the box's corners and edges, and random ones
        let d = if k % 4 == 0 { Vec3::new(1.0, 0.5, 0.75) * if k % 8 == 0 { 1.0 } else { -1.0 } } else { rng.dir() };
        if query::ray(&bs, &Iso::IDENTITY, o, d, 10.0).is_none() {
            escaped += 1;
        }
    }
    c.check(escaped == 0, || format!("{} rays escaped a closed box", escaped));
}

// ---------------------------------------------------------------- distance

fn distance_tests(c: &mut Ctx) {
    let mut rng = Rng::new(4);
    let mut max_err_sep = 0.0f64;
    let mut max_err_pen = 0.0f64;
    for case in 0..c.n(8000) {
        let ka = rng.below(4);
        let kb = rng.below(5);
        let a = random_convex(&mut rng, ka, 1.0);
        let b = if kb == 4 { random_tri(&mut rng, 0.5) } else { random_convex(&mut rng, kb, 1.0) };
        let ia = random_iso(&mut rng, 1.5);
        let ib = if kb == 4 { Iso::IDENTITY } else { random_iso(&mut rng, 1.5) };
        let pa = Poly::of(&a, &ia);
        let pb = poly_of(&b, &ib);
        let r = refs::signed_dist(&pa, &pb);
        let Some(s) = query::separation(&a, &ia, &b, &ib, f64::INFINITY) else {
            c.check(false, || format!("case {}: no separation", case));
            continue;
        };
        let err = (s.distance - r).abs();
        if r > 1e-9 {
            max_err_sep = max_err_sep.max(err);
            c.check(err <= 1e-9, || format!("case {} {}/{}: distance {} vs reference {}", case, ka, kb, s.distance, r));
            c.check(((s.point_a - s.point_b).length() - s.distance).abs() <= 1e-8 && (s.point_a - s.point_b - s.normal * s.distance).length() <= 1e-8, || format!("case {}: witness points", case));
            c.check(refs::point_dist(s.point_b, &pb).abs() <= 1e-8 && refs::point_dist(s.point_a, &pa).abs() <= 1e-8, || format!("case {}: witness not on the surfaces", case));
        } else if r < -1e-9 {
            max_err_pen = max_err_pen.max(err);
            c.check(err <= 1e-7 * (1.0 + r.abs()), || format!("case {} {}/{}: depth {} vs reference {}", case, ka, kb, -s.distance, -r));
            // moving a by normal * (depth + a little) separates them
            let moved = pa.moved(s.normal * (-s.distance + 1e-6));
            c.check(refs::signed_dist(&moved, &pb) >= -1e-7, || format!("case {} {}/{}: the normal does not separate", case, ka, kb));
        }
        c.check(unit(s.normal), || format!("case {}: normal {:?}", case, s.normal));
        if r.abs() > 1e-9 {
            c.check(query::intersects(&a, &ia, &b, &ib) == (r <= 0.0), || format!("case {} {}/{}: intersects vs {}", case, ka, kb, r));
            let mut cs = Vec::new();
            query::contacts(&a, &ia, &b, &ib, 0.0, &mut cs);
            c.check(cs.len() == (r <= 0.0) as usize, || format!("case {}: {} contacts at {}", case, cs.len(), r));
            if let Some(ct) = cs.first() {
                c.check((ct.depth + r).abs() <= 1e-7 * (1.0 + r.abs()) && unit(ct.normal), || format!("case {}: contact depth {} vs {}", case, ct.depth, -r));
            }
        }
    }
    out(&format!("  distance: largest error separated {:.3e} m, penetrating {:.3e} m\n", max_err_sep, max_err_pen));
    // analytic pairs
    for _ in 0..c.n(5000) {
        let (r1, r2) = (rng.range(0.1, 1.0), rng.range(0.1, 1.0));
        let (c1, c2) = (rng.vec(-2.0, 2.0), rng.vec(-2.0, 2.0));
        let s = query::separation(&Shape::sphere(r1), &Iso::at(c1), &Shape::sphere(r2), &Iso::at(c2), f64::INFINITY).unwrap();
        let want = (c1 - c2).length() - r1 - r2;
        c.check((s.distance - want).abs() <= 1e-12 * (1.0 + want.abs()), || format!("sphere-sphere {} vs {}", s.distance, want));
        // capsule-capsule by the closest points of their segments
        let (ia, ib) = (random_iso(&mut rng, 1.0), random_iso(&mut rng, 1.0));
        let (h1, h2) = (rng.range(0.1, 1.0), rng.range(0.1, 1.0));
        let (sa, sb) = (Shape::capsule(h1, r1), Shape::capsule(h2, r2));
        let (p1, q1) = (ia.transform_point(Vec3::new(0.0, -h1, 0.0)), ia.transform_point(Vec3::new(0.0, h1, 0.0)));
        let (p2, q2) = (ib.transform_point(Vec3::new(0.0, -h2, 0.0)), ib.transform_point(Vec3::new(0.0, h2, 0.0)));
        let (x, y) = refs::seg_seg(p1, q1, p2, q2);
        let want = (x - y).length() - r1 - r2;
        let s = query::separation(&sa, &ia, &sb, &ib, f64::INFINITY).unwrap();
        if (x - y).length() > 1e-6 {
            c.check((s.distance - want).abs() <= 1e-9, || format!("capsule-capsule {} vs {}", s.distance, want));
        }
        // sphere-box by clamping
        let hb = rng.vec(0.1, 1.0);
        let ibx = random_iso(&mut rng, 1.0);
        let cl = ibx.inverse_transform_point(c1);
        let q = Vec3::new(cl.x.clamp(-hb.x, hb.x), cl.y.clamp(-hb.y, hb.y), cl.z.clamp(-hb.z, hb.z));
        if q != cl {
            let want = (cl - q).length() - r1;
            let s = query::separation(&Shape::sphere(r1), &Iso::at(c1), &Shape::cuboid(hb), &ibx, f64::INFINITY).unwrap();
            c.check((s.distance - want).abs() <= 1e-9, || format!("sphere-box {} vs {}", s.distance, want));
        }
    }
    // the nearest point
    for case in 0..c.n(8000) {
        let kb = rng.below(5);
        let b = if kb == 4 { random_tri(&mut rng, 0.5) } else { random_convex(&mut rng, kb, 1.0) };
        let ib = if kb == 4 { Iso::IDENTITY } else { random_iso(&mut rng, 1.0) };
        let pb = poly_of(&b, &ib);
        let p = rng.vec(-3.0, 3.0);
        let h = query::closest_point(&b, &ib, p, f64::INFINITY).unwrap();
        let r = refs::point_dist(p, &pb);
        if r > 1e-9 {
            c.check(!h.inside && (h.distance - r).abs() <= 1e-9 && ((h.point - p).length() - r).abs() <= 1e-9, || format!("case {} kind {}: closest {} vs {}", case, kb, h.distance, r));
        } else if r < -1e-9 {
            c.check(h.inside && h.distance == 0.0 && h.point == p, || format!("case {} kind {}: inside not seen ({})", case, kb, r));
        }
    }
}

// ---------------------------------------------------------------- cast

/// one cast against the reference: (checks passed, a message)
fn check_cast(c: &mut Ctx, label: &str, a: &Shape, ia: &Iso, r: Vec3, b: &Shape, ib: &Iso) -> Option<f64> {
    let pa = Poly::of(a, ia);
    let pb = poly_of(b, ib);
    let radius = pa.radius + pb.radius;
    let target = (CAST_SKIN - radius).max(0.0);
    let f = |t: f64| refs::signed_dist(&pa.moved(r * t), &pb);
    let tol = 1e-8;
    let hit = query::cast(a, ia, r, b, ib);
    let f0 = f(0.0);
    if f0 <= tol {
        // touching or overlapping at the start: a hit at 0 exactly when the motion goes deeper
        let step = 1e-4 / r.length().max(1e-9);
        let deeper = f(step.min(1.0)) < f0 - 1e-10;
        let leaving = f(step.min(1.0)) > f0 + 1e-10;
        if deeper {
            c.check(hit.is_some_and(|h| h.fraction == 0.0), || format!("{}: start overlap {} going deeper, got {:?}", label, f0, hit.map(|h| h.fraction)));
        } else if leaving {
            c.check(hit.is_none() || hit.is_some_and(|h| h.fraction > 0.0), || format!("{}: start overlap {} leaving, got a hit at 0", label, f0));
        }
        return hit.map(|h| h.fraction);
    }
    let (root, fmin) = refs::first_root(&f, -tol);
    match hit {
        Some(h) => {
            let gap = f(h.fraction);
            c.check(gap <= target + tol && gap >= -tol, || format!("{}: hit at {} with gap {} (target {})", label, h.fraction, gap, target));
            if let Some(rt) = root {
                c.check(h.fraction <= rt + 1e-9, || format!("{}: hit at {} after the contact at {}", label, h.fraction, rt));
            }
            c.check(unit(h.normal) && h.normal.dot(r) <= 1e-9 * r.length() && (0.0..=1.0).contains(&h.fraction), || format!("{}: normal {:?} motion {:?}", label, h.normal, r));
            c.check(refs::point_dist(h.point, &pb).abs() <= 1e-7, || format!("{}: point not on the target ({})", label, refs::point_dist(h.point, &pb)));
        }
        None => {
            c.check(fmin >= -tol, || format!("{}: missed, the shapes overlap by {}", label, -fmin));
        }
    }
    hit.map(|h| h.fraction)
}

fn cast_tests(c: &mut Ctx) {
    let mut rng = Rng::new(5);
    let mut hits = 0;
    for case in 0..c.n(4000) {
        let ka = rng.below(4);
        let kb = rng.below(5);
        let a = random_convex(&mut rng, ka, 0.7);
        let b = if kb == 4 { random_tri(&mut rng, 0.5) } else { random_convex(&mut rng, kb, 0.7) };
        let ia = random_iso(&mut rng, 3.0);
        let ib = if kb == 4 { Iso::IDENTITY } else { random_iso(&mut rng, 1.0) };
        let aim = ib.pos + rng.vec(-1.0, 1.0) - ia.pos;
        let r = if rng.below(6) == 0 { rng.dir() * rng.range(0.0, 3.0) } else { aim * rng.range(0.3, 2.0) };
        if check_cast(c, &format!("case {} {}/{}", case, ka, kb), &a, &ia, r, &b, &ib).is_some() {
            hits += 1;
        }
    }
    out(&format!("  cast: {} of {} random casts hit\n", hits, c.n(4000)));
    // sphere against sphere: the exact time of a ray against the sum of the radii
    for _ in 0..c.n(5000) {
        let (r1, r2) = (rng.range(0.05, 1.0), rng.range(0.05, 1.0));
        let p = rng.vec(-5.0, 5.0);
        let q = rng.vec(-1.0, 1.0);
        let m = (q - p + rng.vec(-0.5, 0.5)) * rng.range(0.5, 2.0);
        let h = query::cast(&Shape::sphere(r1), &Iso::at(p), m, &Shape::sphere(r2), &Iso::at(q));
        let rs = r1 + r2;
        let l = m.length();
        let dn = m / l;
        let oc = q - p;
        let tca = oc.dot(dn);
        let d2 = oc.length_sq() - tca * tca;
        if oc.length() > rs && tca > 0.0 && d2 < rs * rs * (1.0 - 1e-6) {
            let t = (tca - sqrt(rs * rs - d2)) / l;
            if t < 1.0 - 1e-9 {
                // at the cast's time the gap is at most 1e-9 m, and the time is not after the exact one
                let ok = h.is_some_and(|h| {
                    let gap = (p + m * h.fraction - q).length() - rs;
                    (-1e-12..=1e-9 + 1e-12).contains(&gap) && h.fraction <= t + 1e-12
                });
                c.check(ok, || format!("sphere-sphere cast {:?} vs {}", h.map(|h| h.fraction), t));
            } else if t > 1.0 + 1e-9 {
                c.check(h.is_none(), || "sphere-sphere cast beyond the motion".into());
            }
        }
    }
}

// ---------------------------------------------------------------- tunnel

fn tunnel_tests(c: &mut Ctx) {
    let mut rng = Rng::new(6);
    // a small sphere at up to 10 km per step through a triangle: its centre's path crosses the triangle
    let mut worst_speed = 0.0f64;
    for case in 0..c.n(20_000) {
        let m = random_tri(&mut rng, 0.0);
        let t = tri_of(&m);
        let n = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        let w = [rng.f(), rng.f(), rng.f()];
        let s = w[0] + w[1] + w[2];
        let x = (t[0] * w[0] + t[1] * w[1] + t[2] * w[2]) / s;
        let speed = 10f64.powf_det(rng.range(-1.0, 4.0));
        let dir = (n + rng.vec(-0.7, 0.7)).normalize_or_zero();
        let before = rng.range(0.05, 0.95);
        let start = x - dir * (speed * before);
        let motion = dir * speed;
        let r = rng.range(0.001, 0.05);
        if !refs::seg_crosses_tri(start, start + motion, t[0], t[1], t[2]) {
            continue;
        }
        let cross = refs::ray_tri_mt(start, motion, t[0], t[1], t[2]).map(|v| v.0).unwrap_or(1.0);
        let h = query::cast(&Shape::sphere(r), &Iso::at(start), motion, &m, &Iso::IDENTITY);
        let ok = h.is_some_and(|h| h.fraction <= cross + 1e-12);
        if ok {
            worst_speed = worst_speed.max(speed);
        }
        c.check(ok, || format!("case {}: sphere {} at {} m per step went through ({:?}, crossing at {})", case, r, speed, h.map(|h| h.fraction), cross));
    }
    // a capsule and a box through a thin box (1 mm)
    let wall = Shape::cuboid(Vec3::new(2.0, 2.0, 0.0005));
    for case in 0..c.n(10_000) {
        let a = if case % 2 == 0 { Shape::capsule(0.3, 0.02) } else { Shape::cuboid(Vec3::new(0.01, 0.02, 0.005)) };
        let start = Vec3::new(rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), -rng.range(0.1, 5000.0));
        let end = Vec3::new(rng.range(-1.5, 1.5), rng.range(-1.5, 1.5), rng.range(0.1, 5000.0));
        let iso = Iso::new(start, rng.quat());
        let h = query::cast(&a, &iso, end - start, &wall, &Iso::IDENTITY);
        let cross = -start.z / (end.z - start.z);
        c.check(h.is_some_and(|h| h.fraction <= cross), || format!("case {}: through the thin wall ({:?}, crossing {})", case, h.map(|h| h.fraction), cross));
    }
    // straight down through the map from above at every speed
    let map = scene::map(40, 20);
    let ms = Shape::mesh(map.build());
    for case in 0..c.n(5000) {
        let p = Vec3::new(rng.range(0.5, 39.5), 20.0, rng.range(0.5, 39.5));
        let fall = rng.range(25.0, 1e4);
        let s = if case % 2 == 0 { Shape::sphere(rng.range(0.01, 0.5)) } else { Shape::capsule(rng.range(0.1, 0.6), rng.range(0.05, 0.4)) };
        let h = query::cast(&s, &Iso::at(p), Vec3::new(0.0, -fall, 0.0), &ms, &Iso::IDENTITY);
        c.check(h.is_some(), || format!("case {}: fell through the map at {:?} ({} m)", case, p, fall));
    }
    out(&format!("  tunnel: fastest sphere caught {:.0} m per step\n", worst_speed));
}

trait PowDet {
    fn powf_det(self, e: f64) -> f64;
}

impl PowDet for f64 {
    /// 10^e from rt's portable exp2 (the same on every target)
    fn powf_det(self, e: f64) -> f64 {
        let _ = self;
        #[cfg(not(target_arch = "wasm32"))]
        {
            rt::portable::math::exp2(e * core::f64::consts::LOG2_10)
        }
        #[cfg(target_arch = "wasm32")]
        {
            rt::x87::exp2(e * core::f64::consts::LOG2_10)
        }
    }
}

// ---------------------------------------------------------------- seams

fn seam_tests(c: &mut Ctx) {
    let mut rng = Rng::new(7);
    let grid = scene::flat(16, 1.0, 0.0);
    let on = Shape::mesh(grid.build());
    let off = Shape::mesh(TriMesh::with_options(&grid.pos, &grid.idx, &MeshOptions { active_edge_cos: 2.0, ..MeshOptions::default() }).unwrap());
    // sliding over the flat grid at, just above and just below touching height: nothing may block
    let mut blocked = 0usize;
    let mut ghosts_off = 0usize;
    let mut total = 0usize;
    for case in 0..c.n(6000) {
        let delta = [1e-3, 1e-7, 0.0, -1e-7, -1e-5][case % 5];
        let (shape, lift) = match case % 3 {
            0 => (Shape::sphere(0.5), 0.5),
            1 => (Shape::capsule(0.5, 0.3), 0.8),
            _ => (Shape::cuboid(Vec3::new(0.4, 0.3, 0.4)), 0.3),
        };
        let p = Vec3::new(rng.range(3.0, 13.0), lift + delta, rng.range(3.0, 13.0));
        let ang = rng.range(0.0, 6.283);
        let (s, co) = math::sincos(ang);
        let m = Vec3::new(co, 0.0, s) * rng.range(0.5, 2.5);
        let rot = if case % 3 == 2 { Quat::from_axis_angle(Vec3::Y, rng.range(0.0, 1.5)) } else { Quat::IDENTITY };
        let iso = Iso::new(p, rot);
        total += 1;
        if let Some(h) = query::cast(&shape, &iso, m, &on, &Iso::IDENTITY) {
            blocked += 1;
            if blocked <= 3 {
                out(&format!("  seam hit: case {} delta {:e} fraction {} normal {:?} tri {}\n", case, delta, h.fraction, h.normal, h.triangle));
            }
        }
        if delta < 0.0 && case % 3 == 0 && query::cast(&shape, &iso, m, &off, &Iso::IDENTITY).is_some() {
            ghosts_off += 1;
        }
    }
    c.check(blocked == 0, || format!("{} of {} slides over a flat grid were blocked", blocked, total));
    // the same with the active edges off shows the ghost collisions the test is about
    c.check(ghosts_off > 0, || "no ghost collision without active edges: the seam test sees nothing".into());
    out(&format!("  seams: {} blocked slides of {} (without active edges: {} ghost hits)\n", blocked, total, ghosts_off));
    // straight down onto vertices, edges and faces: a hit at the surface with the face's normal
    for i in 0..=32 {
        for j in 0..=32 {
            let (x, z) = (i as f64 * 0.5, j as f64 * 0.5);
            for (k, shape) in [Shape::sphere(0.4), Shape::capsule(0.3, 0.2), Shape::cuboid(Vec3::new(0.2, 0.1, 0.3))].iter().enumerate() {
                let lift = [0.4, 0.5, 0.1][k];
                let p = Vec3::new(x, 3.0, z);
                let h = query::cast(shape, &Iso::at(p), Vec3::new(0.0, -5.0, 0.0), &on, &Iso::IDENTITY);
                // inside the grid the triangle's normal exactly; on its open border an edge's normal
                let border = x == 0.0 || x == 16.0 || z == 0.0 || z == 16.0;
                let ok = h.is_some_and(|h| {
                    let y = 3.0 - 5.0 * h.fraction - lift;
                    (-1e-9..=CAST_SKIN + 1e-8).contains(&y) && (h.normal - Vec3::Y).length() < if border { 1e-6 } else { 1e-12 }
                });
                c.check(ok, || format!("drop at ({}, {}) shape {}: {:?}", x, z, k, h));
            }
        }
    }
    // a sphere dropped on hilly ground: contacts at rest point upwards and are shallow
    let hills = scene::MeshData::new();
    let mut hills = hills;
    scene::ground(&mut hills, 24, 1.0, 2.0, 3);
    let hs = Shape::mesh(hills.build());
    for case in 0..c.n(3000) {
        let r = rng.range(0.1, 0.8);
        let p = Vec3::new(rng.range(2.0, 22.0), 6.0, rng.range(2.0, 22.0));
        let Some(h) = query::cast(&Shape::sphere(r), &Iso::at(p), Vec3::new(0.0, -10.0, 0.0), &hs, &Iso::IDENTITY) else {
            c.check(false, || format!("case {}: missed the hills", case));
            continue;
        };
        let rest = Iso::at(p + Vec3::new(0.0, -10.0 * h.fraction, 0.0));
        let mut cs = Vec::new();
        query::contacts(&Shape::sphere(r), &rest, &hs, &Iso::IDENTITY, 1e-6, &mut cs);
        c.check(!cs.is_empty() && cs.iter().all(|ct| ct.normal.y > 0.0 && ct.depth <= 1e-8 && unit(ct.normal)), || format!("case {}: contacts at rest {:?}", case, cs));
    }
    // collide and slide over the map: a sphere and a capsule never end up inside it
    let map = scene::map(30, 25);
    let ms = Shape::mesh(map.build());
    for walker in 0..c.n(40) {
        let shape = if walker % 2 == 0 { Shape::sphere(0.35) } else { Shape::capsule(0.4, 0.3) };
        let mut pos = Vec3::new(rng.range(4.0, 26.0), 8.0, rng.range(4.0, 26.0));
        let ang = rng.range(0.0, 6.283);
        let mut vel = Vec3::new(math::cos(ang) * 0.15, -0.2, math::sin(ang) * 0.15);
        for step in 0..c.n(1500).max(150) {
            let (np, nv) = slide(&shape, pos, vel, &ms);
            pos = np;
            vel = nv;
            if step % 37 == 0 {
                vel = Vec3::new(math::cos(ang + step as f64) * 0.15, vel.y, math::sin(ang + step as f64) * 0.15);
            }
            let s = query::separation(&shape, &Iso::at(pos), &ms, &Iso::IDENTITY, 1.0);
            let ok = s.is_none_or(|s| s.distance >= -1e-6);
            if !c.check(ok, || format!("walker {} step {}: inside the map by {:?} at {:?}", walker, step, s.map(|s| -s.distance), pos)) {
                break;
            }
            if pos.x < 1.0 || pos.x > 29.0 || pos.z < 1.0 || pos.z > 29.0 {
                pos = Vec3::new(15.0, 8.0, 15.0);
            }
        }
    }
}

/// one step of collide and slide (a stand-in for the controller of part 2): up to four casts, stop 1 mm
/// before each contact, take the rest of the motion along the surface
pub fn slide(shape: &Shape, mut pos: Vec3, vel: Vec3, map: &Shape) -> (Vec3, Vec3) {
    let skin = 1e-3;
    let mut left = vel;
    let mut v = vel;
    for _ in 0..4 {
        let l = left.length();
        if l < 1e-9 {
            break;
        }
        match query::cast(shape, &Iso::at(pos), left, map, &Iso::IDENTITY) {
            None => {
                pos += left;
                break;
            }
            Some(h) => {
                let go = (h.fraction - skin / l).max(0.0);
                pos += left * go;
                let rest = left * (1.0 - go);
                left = rest - h.normal * rest.dot(h.normal);
                v = v - h.normal * v.dot(h.normal).min(0.0);
                if h.depth > 0.0 {
                    pos += h.normal * (h.depth + skin);
                }
            }
        }
    }
    (pos, Vec3::new(v.x, (v.y - 0.01).max(-0.5), v.z))
}

// ---------------------------------------------------------------- bvh

fn bvh_tests(c: &mut Ctx) {
    let mut rng = Rng::new(8);
    let data = scene::map(36, 30);
    let mesh = data.build();
    out(&format!("  bvh: {} triangles, {} nodes, depth {}\n", mesh.stored_triangle_count(), mesh.nodes().len(), mesh.depth()));
    // the tree: every triangle in one leaf, boxes hold their children and triangles
    let nodes = mesh.nodes();
    let tris: Vec<(u32, [Vec3; 3])> = mesh.triangles().collect();
    let mut seen = vec![0u32; tris.len()];
    let mut stack = vec![0usize];
    let mut bad = 0;
    while let Some(i) = stack.pop() {
        let n = nodes[i];
        let bx = Aabb::new(Vec3::from(n.min), Vec3::from(n.max));
        if n.is_leaf() {
            c.check(n.count <= 4, || format!("leaf of {}", n.count));
            for t in n.first..n.first + n.count {
                seen[t as usize] += 1;
                for p in tris[t as usize].1 {
                    if !bx.contains_point(p) {
                        bad += 1;
                    }
                }
            }
        } else {
            for ch in [n.first as usize, n.first as usize + 1] {
                let cn = nodes[ch];
                if !bx.contains(&Aabb::new(Vec3::from(cn.min), Vec3::from(cn.max))) {
                    bad += 1;
                }
                stack.push(ch);
            }
        }
    }
    c.check(bad == 0 && seen.iter().all(|&s| s == 1), || format!("tree: {} boxes too small, triangles seen {:?}", bad, seen.iter().filter(|&&s| s != 1).count()));
    let ms = Shape::mesh(mesh);
    let place = Iso::new(Vec3::new(-3.0, 0.5, 7.0), Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), 0.3));
    let size = 36.0;
    let rand_pt = |rng: &mut Rng| place.transform_point(Vec3::new(rng.range(-2.0, size + 2.0), rng.range(-1.0, 5.0), rng.range(-2.0, size + 2.0)));
    // rays: the tree and the loop over every triangle, bit for bit
    for case in 0..c.n(8000) {
        let o = rand_pt(&mut rng);
        let d = if case % 2 == 0 { Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 0.2), rng.range(-1.0, 1.0)) * 30.0 } else { rand_pt(&mut rng) - o };
        let a = query::ray(&ms, &place, o, d, 1.0);
        let b = query::brute::ray(&ms, &place, o, d, 1.0);
        c.check(same_ray(&a, &b), || format!("case {}: ray {:?} vs brute {:?}", case, a, b));
        if case % 8 == 0 {
            let (mut la, mut lb) = (Vec::new(), Vec::new());
            query::ray_all(&ms, &place, o, d, 1.0, &mut la);
            query::brute::ray_all(&ms, &place, o, d, 1.0, &mut lb);
            c.check(la.len() == lb.len() && la.iter().zip(&lb).all(|(x, y)| same_ray(&Some(*x), &Some(*y))), || format!("case {}: ray_all {} vs {}", case, la.len(), lb.len()));
        }
    }
    // casts, contacts, the nearest point, separation, overlap
    for case in 0..c.n(1000) {
        let s = random_convex(&mut rng, case % 4, 0.6);
        let iso = Iso::new(rand_pt(&mut rng), rng.quat());
        let m = Vec3::new(rng.range(-3.0, 3.0), rng.range(-4.0, 1.0), rng.range(-3.0, 3.0));
        let a = query::cast(&s, &iso, m, &ms, &place);
        let b = query::brute::cast(&s, &iso, m, &ms, &place);
        c.check(same_cast(&a, &b), || format!("case {}: cast {:?} vs brute {:?}", case, a, b));
        let (mut ca, mut cb) = (Vec::new(), Vec::new());
        query::contacts(&s, &iso, &ms, &place, 0.05, &mut ca);
        query::brute::contacts(&s, &iso, &ms, &place, 0.05, &mut cb);
        c.check(ca.len() == cb.len() && ca.iter().zip(&cb).all(|(x, y)| x.triangle == y.triangle && x.depth.to_bits() == y.depth.to_bits() && bits(x.normal) == bits(y.normal)), || format!("case {}: contacts {} vs {}", case, ca.len(), cb.len()));
        c.check(query::intersects(&s, &iso, &ms, &place) == query::brute::intersects(&s, &iso, &ms, &place), || format!("case {}: intersects", case));
        let sa = query::separation(&s, &iso, &ms, &place, 2.0);
        let sb = query::brute::separation(&s, &iso, &ms, &place, 2.0);
        c.check(sa.map(|x| (x.distance.to_bits(), x.triangle)) == sb.map(|x| (x.distance.to_bits(), x.triangle)), || format!("case {}: separation {:?} vs {:?}", case, sa, sb));
        let p = rand_pt(&mut rng);
        let pa = query::closest_point(&ms, &place, p, 3.0);
        let pb = query::brute::closest_point(&ms, &place, p, 3.0);
        c.check(pa.map(|x| (x.distance.to_bits(), x.triangle, bits(x.point))) == pb.map(|x| (x.distance.to_bits(), x.triangle, bits(x.point))), || format!("case {}: closest {:?} vs {:?}", case, pa, pb));
    }
}

fn same_ray(a: &Option<query::RayHit>, b: &Option<query::RayHit>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x.t.to_bits() == y.t.to_bits() && x.triangle == y.triangle && bits(x.normal) == bits(y.normal) && bits(x.point) == bits(y.point),
        _ => false,
    }
}

fn same_cast(a: &Option<query::CastHit>, b: &Option<query::CastHit>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => {
            x.fraction.to_bits() == y.fraction.to_bits() && x.triangle == y.triangle && bits(x.normal) == bits(y.normal) && bits(x.point) == bits(y.point) && x.depth.to_bits() == y.depth.to_bits()
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- world

fn world_tests(c: &mut Ctx) {
    let mut rng = Rng::new(9);
    let map = Arc::new(scene::map(32, 20).build());
    let mut w = World::new();
    let ground = w.add_with_layers(Shape::Mesh(map.clone()), Iso::IDENTITY, 1);
    let mut ids: Vec<BodyId> = Vec::new();
    let spawn = |rng: &mut Rng, w: &mut World| {
        let k = rng.below(4);
        let s = random_convex(rng, k, 0.5);
        let iso = Iso::new(Vec3::new(rng.range(0.0, 32.0), rng.range(0.0, 6.0), rng.range(0.0, 32.0)), rng.quat());
        let layers = 1u32 << rng.below(3);
        w.add_with_layers(s, iso, layers)
    };
    for _ in 0..c.n(400).max(60) {
        ids.push(spawn(&mut rng, &mut w));
    }
    let ts = w.trees();
    c.check(ts[0].validate().is_ok() && ts[1].validate().is_ok() && ts[0].len() == 1 && ts[2].is_empty(), || "trees after adding".into());
    for round in 0..c.n(60).max(8) {
        // move, remove and add
        for _ in 0..30 {
            let i = rng.below(ids.len());
            let iso = Iso::new(Vec3::new(rng.range(0.0, 32.0), rng.range(0.0, 6.0), rng.range(0.0, 32.0)), rng.quat());
            c.check(w.set_iso(ids[i], iso), || "set_iso".into());
        }
        for _ in 0..5 {
            let i = rng.below(ids.len());
            let id = ids.swap_remove(i);
            c.check(w.remove(id).is_some() && w.body(id).is_none() && !w.set_iso(id, Iso::IDENTITY), || "remove".into());
            ids.push(spawn(&mut rng, &mut w));
        }
        let ts = w.trees();
        let v = ts[0].validate().and(ts[1].validate()).and(ts[2].validate());
        c.check(v.is_ok() && w.len() == ids.len() + 1 && ts[0].len() + ts[1].len() + ts[2].len() == w.len(), || format!("round {}: trees {:?}, {} bodies", round, v, w.len()));
        // every query against the loop over the bodies
        for q in 0..40 {
            let filter = match q % 4 {
                0 => Filter::ALL,
                1 => Filter::mask(1 << rng.below(3)),
                2 => Filter::ALL.excluding(ground),
                _ => Filter::mask(0b110).excluding(ids[rng.below(ids.len())]),
            };
            let o = Vec3::new(rng.range(-2.0, 34.0), rng.range(-1.0, 8.0), rng.range(-2.0, 34.0));
            let d = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 0.3), rng.range(-1.0, 1.0)) * rng.range(1.0, 40.0);
            let a = w.ray_cast(o, d, 1.0, &filter);
            let b = brute_ray(&w, o, d, &filter);
            c.check(a.map(|x| (x.0, x.1.t.to_bits(), x.1.triangle)) == b.map(|x| (x.0, x.1.t.to_bits(), x.1.triangle)), || format!("round {} ray {:?} vs {:?}", round, a, b));
            let mut la = Vec::new();
            w.ray_cast_all(o, d, 1.0, &filter, &mut la);
            let mut lb: Vec<(BodyId, query::RayHit)> = Vec::new();
            for (id, body) in w.bodies() {
                if body.layers & filter.mask != 0 && filter.exclude != Some(id) {
                    let mut hs = Vec::new();
                    query::ray_all(&body.shape, &body.iso, o, d, 1.0, &mut hs);
                    lb.extend(hs.into_iter().map(|h| (id, h)));
                }
            }
            lb.sort_by(|x, y| x.1.t.partial_cmp(&y.1.t).unwrap().then(x.0.cmp(&y.0)).then(x.1.triangle.cmp(&y.1.triangle)));
            c.check(la.len() == lb.len() && la.iter().zip(&lb).all(|(x, y)| x.0 == y.0 && x.1.t.to_bits() == y.1.t.to_bits() && x.1.triangle == y.1.triangle), || format!("round {} ray_all {} vs {}", round, la.len(), lb.len()));
            let s = random_convex(&mut rng, q % 4, 0.4);
            let iso = Iso::new(o, rng.quat());
            let m = d * 0.2;
            let ca = w.cast(&s, &iso, m, &filter);
            let mut cb: Option<(BodyId, query::CastHit)> = None;
            for (id, body) in w.bodies() {
                if body.layers & filter.mask != 0 && filter.exclude != Some(id) {
                    if let Some(h) = query::cast(&s, &iso, m, &body.shape, &body.iso) {
                        let better = cb.is_none_or(|(bid, bh)| h.fraction < bh.fraction || (h.fraction == bh.fraction && (id < bid || (id == bid && h.triangle < bh.triangle))));
                        if better {
                            cb = Some((id, h));
                        }
                    }
                }
            }
            c.check(ca.map(|x| (x.0, x.1.fraction.to_bits(), x.1.triangle)) == cb.map(|x| (x.0, x.1.fraction.to_bits(), x.1.triangle)), || format!("round {} cast {:?} vs {:?}", round, ca, cb));
            let mut oa = Vec::new();
            w.overlap(&s, &iso, &filter, &mut oa);
            let ob: Vec<BodyId> = w.bodies().filter(|(id, b)| b.layers & filter.mask != 0 && filter.exclude != Some(*id) && query::intersects(&s, &iso, &b.shape, &b.iso)).map(|(id, _)| id).collect();
            c.check(oa == ob, || format!("round {} overlap {:?} vs {:?}", round, oa, ob));
            let mut ka = Vec::new();
            w.contacts(&s, &iso, 0.1, &filter, &mut ka);
            let mut kb: Vec<(BodyId, query::Contact)> = Vec::new();
            for (id, b) in w.bodies() {
                if b.layers & filter.mask != 0 && filter.exclude != Some(id) {
                    let mut cs = Vec::new();
                    query::contacts(&s, &iso, &b.shape, &b.iso, 0.1, &mut cs);
                    kb.extend(cs.into_iter().map(|x| (id, x)));
                }
            }
            c.check(ka.len() == kb.len() && ka.iter().zip(&kb).all(|(x, y)| x.0 == y.0 && x.1.triangle == y.1.triangle && x.1.depth.to_bits() == y.1.depth.to_bits()), || format!("round {} contacts {} vs {}", round, ka.len(), kb.len()));
            let pa = w.closest_point(o, 3.0, &filter);
            let mut pb: Option<(BodyId, query::PointHit)> = None;
            for (id, b) in w.bodies() {
                if b.layers & filter.mask != 0 && filter.exclude != Some(id) {
                    if let Some(h) = query::closest_point(&b.shape, &b.iso, o, 3.0) {
                        if pb.is_none_or(|(bid, bh)| h.distance < bh.distance || (h.distance == bh.distance && (id < bid || (id == bid && h.triangle < bh.triangle)))) {
                            pb = Some((id, h));
                        }
                    }
                }
            }
            c.check(pa.map(|x| (x.0, x.1.distance.to_bits())) == pb.map(|x| (x.0, x.1.distance.to_bits())), || format!("round {} closest {:?} vs {:?}", round, pa, pb));
        }
    }
    // ids: a dead id stays dead when its index comes back
    let id = ids[0];
    let _ = w.remove(id);
    let again = w.add(Shape::sphere(1.0), Iso::IDENTITY);
    c.check(again.index() == id.index() && again != id && w.body(id).is_none() && w.body(again).is_some(), || "generations".into());
}

fn brute_ray(w: &World, o: Vec3, d: Vec3, filter: &Filter) -> Option<(BodyId, query::RayHit)> {
    let mut best: Option<(BodyId, query::RayHit)> = None;
    for (id, b) in w.bodies() {
        if b.layers & filter.mask == 0 || filter.exclude == Some(id) {
            continue;
        }
        if let Some(h) = query::ray(&b.shape, &b.iso, o, d, 1.0) {
            if best.is_none_or(|(bid, bh)| h.t < bh.t || (h.t == bh.t && (id < bid || (id == bid && h.triangle < bh.triangle)))) {
                best = Some((id, h));
            }
        }
    }
    best
}

// ---------------------------------------------------------------- fuzz

fn fuzz_tests(c: &mut Ctx) {
    let mut rng = Rng::new(10);
    let mut asym = 0.0f64;
    for case in 0..c.n(40_000) {
        let scale = 10f64.powf_det(rng.range(-2.5, 2.5));
        let far = if case % 5 == 0 { rng.vec(-1e5, 1e5) } else { Vec3::ZERO };
        let ka = rng.below(4);
        let kb = rng.below(5);
        let a = random_convex(&mut rng, ka, scale);
        let b = if kb == 4 {
            let t = rng.vec(-1.0, 1.0) * scale;
            tri_mesh(t, t + rng.vec(-1.0, 1.0) * scale, t + rng.vec(-1.0, 1.0) * scale)
        } else {
            random_convex(&mut rng, kb, scale)
        };
        let ia = Iso::new(far + rng.vec(-2.0, 2.0) * scale, rng.quat());
        let ib = Iso::new(far + rng.vec(-2.0, 2.0) * scale, rng.quat());
        let m = rng.vec(-4.0, 4.0) * scale;
        let label = || format!("case {} {}/{} scale {:e}", case, ka, kb, scale);
        if let Some(h) = query::cast(&a, &ia, m, &b, &ib) {
            c.check((0.0..=1.0).contains(&h.fraction) && unit(h.normal) && fin(h.point) && h.depth >= 0.0 && h.depth.is_finite(), || format!("{}: cast {:?}", label(), h));
        }
        let s = query::separation(&a, &ia, &b, &ib, f64::INFINITY);
        match s {
            Some(s) => {
                c.check(s.distance.is_finite() && unit(s.normal) && fin(s.point_a) && fin(s.point_b), || format!("{}: separation {:?}", label(), s));
                let i = query::intersects(&a, &ia, &b, &ib);
                let band = 1e-9 * (scale + far.length());
                if s.distance.abs() > band {
                    c.check(i == (s.distance <= 0.0), || format!("{}: intersects {} at distance {}", label(), i, s.distance));
                }
                if kb < 4 {
                    if let Some(t) = query::separation(&b, &ib, &a, &ia, f64::INFINITY) {
                        let e = (t.distance - s.distance).abs() / (scale + far.length() * 1e-7);
                        asym = asym.max(e);
                        c.check(e <= 1e-6, || format!("{}: separation not symmetric: {} vs {}", label(), s.distance, t.distance));
                    }
                }
            }
            None => {
                c.check(false, || format!("{}: no separation", label()));
            }
        }
        let p = ia.pos + rng.vec(-3.0, 3.0) * scale;
        if let Some(h) = query::closest_point(&b, &ib, p, f64::INFINITY) {
            c.check(h.distance >= 0.0 && fin(h.point), || format!("{}: closest {:?}", label(), h));
        }
        if let Some(h) = query::ray(&b, &ib, p, m, 1.0) {
            c.check(h.t >= 0.0 && h.t <= 1.0 && unit(h.normal), || format!("{}: ray {:?}", label(), h));
        }
    }
    out(&format!("  fuzz: largest asymmetry of separation {:.3e} (relative to the size)\n", asym));
}

// ---------------------------------------------------------------- edge cases

fn edge_tests(c: &mut Ctx) {
    let mut rng = Rng::new(11);
    // meshes: refused input, nothing to hit
    c.check(TriMesh::new(&[[0.0; 3]; 3], &[0, 1]).err() == Some(MeshError::IndexCount), || "index count".into());
    c.check(TriMesh::new(&[[0.0; 3]; 3], &[0, 1, 3]).err() == Some(MeshError::IndexRange), || "index range".into());
    c.check(TriMesh::new(&[[0.0, f32::NAN, 0.0]; 3], &[0, 1, 2]).err() == Some(MeshError::NotFinite), || "NaN".into());
    let empty = Shape::mesh(TriMesh::new(&[], &[]).unwrap());
    let flat = Shape::mesh(TriMesh::new(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], &[0, 1, 2, 0, 0, 1]).unwrap());
    for m in [&empty, &flat] {
        c.check(query::ray(m, &Iso::IDENTITY, Vec3::new(0.5, 1.0, 0.0), Vec3::new(0.0, -2.0, 0.0), 1.0).is_none(), || "ray on an empty mesh".into());
        c.check(query::cast(&Shape::sphere(0.5), &Iso::at(Vec3::new(0.0, 2.0, 0.0)), Vec3::new(0.0, -4.0, 0.0), m, &Iso::IDENTITY).is_none(), || "cast on an empty mesh".into());
        c.check(query::closest_point(m, &Iso::IDENTITY, Vec3::ZERO, f64::INFINITY).is_none(), || "closest on an empty mesh".into());
    }
    if let Shape::Mesh(m) = &flat {
        c.check(m.triangle_count() == 2 && m.stored_triangle_count() == 0, || "degenerate triangles left out".into());
    }
    // a mesh is no query shape
    let tri = tri_mesh(Vec3::ZERO, Vec3::X, Vec3::Z);
    c.check(query::cast(&tri, &Iso::IDENTITY, Vec3::Y, &Shape::sphere(1.0), &Iso::IDENTITY).is_none(), || "a mesh as query shape".into());
    // no motion: touching or apart gives no hit; overlapping neither (not going deeper)
    let s = Shape::sphere(1.0);
    for gap in [0.5, 0.0, -0.3] {
        let h = query::cast(&s, &Iso::at(Vec3::new(2.0 + gap, 0.0, 0.0)), Vec3::ZERO, &s, &Iso::IDENTITY);
        c.check(h.is_none(), || format!("no motion at gap {}: {:?}", gap, h));
    }
    // touching and moving away or along: free; moving in: a hit at 0
    let touch = Iso::at(Vec3::new(2.0, 0.0, 0.0));
    c.check(query::cast(&s, &touch, Vec3::new(1.0, 0.0, 0.0), &s, &Iso::IDENTITY).is_none(), || "leaving".into());
    c.check(query::cast(&s, &touch, Vec3::new(0.0, 1.0, 0.0), &s, &Iso::IDENTITY).is_none(), || "along".into());
    c.check(query::cast(&s, &touch, Vec3::new(-1.0, 0.0, 0.0), &s, &Iso::IDENTITY).is_some_and(|h| h.fraction == 0.0), || "entering".into());
    // a ray without direction
    c.check(query::ray(&s, &Iso::at(Vec3::new(5.0, 0.0, 0.0)), Vec3::ZERO, Vec3::ZERO, 1.0).is_none(), || "zero ray outside".into());
    c.check(query::ray(&s, &Iso::IDENTITY, Vec3::ZERO, Vec3::ZERO, 1.0).is_some_and(|h| h.t == 0.0), || "zero ray inside".into());
    // far from the origin: the same scene shifted by 10^6 m gives the same answers to 1e-8 m, except where
    // a shape passes an edge within the cast's tolerance (a grazing contact: hit in one frame, a miss or a
    // later hit in the other; the 1e-10 m of rounding in the relative placement decide it)
    let map = Arc::new(scene::map(16, 10).build());
    let ms = Shape::Mesh(map.clone());
    let shift = Vec3::new(1.0e6, -2.0e3, -1.0e6);
    let mut grazing = 0usize;
    let n_far = c.n(3000);
    for case in 0..n_far {
        let shp = random_convex(&mut rng, case % 4, 0.5);
        let p = Vec3::new(rng.range(1.0, 15.0), rng.range(2.0, 6.0), rng.range(1.0, 15.0));
        let rot = rng.quat();
        let m = Vec3::new(rng.range(-2.0, 2.0), rng.range(-6.0, 0.0), rng.range(-2.0, 2.0));
        let a = query::cast(&shp, &Iso::new(p, rot), m, &ms, &Iso::IDENTITY);
        let b = query::cast(&shp, &Iso::new(p + shift, rot), m, &ms, &Iso::at(shift));
        let same = match (a, b) {
            (None, None) => true,
            // overlapping at the start: the depth and the normal (the deepest point of a face-to-face overlap
            // is any point of a patch)
            (Some(x), Some(y)) if x.fraction == 0.0 && y.fraction == 0.0 => (x.depth - y.depth).abs() <= 1e-8 && (x.normal - y.normal).length() <= 1e-8,
            (Some(x), Some(y)) => (x.fraction - y.fraction).abs() * m.length() <= 1e-8 && (x.point + shift - y.point).length() <= 1e-8,
            _ => false,
        };
        if same {
            c.check(true, String::new);
            continue;
        }
        // a grazing contact: at the earlier of the two times the shape is within the tolerance of the map
        let first = match (a, b) {
            (Some(x), Some(y)) => x.fraction.min(y.fraction),
            (Some(x), None) | (None, Some(x)) => x.fraction,
            _ => 0.0,
        };
        let gap = query::separation(&shp, &Iso::new(p + m * first, rot), &ms, &Iso::IDENTITY, 1.0).map(|s| s.distance);
        let graze = gap.is_some_and(|g| g.abs() <= CAST_SKIN + 1e-8);
        grazing += graze as usize;
        c.check(graze, || format!("case {}: far away {:?} vs {:?} (gap {:?} at {})", case, a, b, gap, first));
    }
    c.check(grazing * 100 <= n_far, || format!("{} of {} casts differ far away (grazing)", grazing, n_far));
    out(&format!("  edge: {} of {} casts at 10^6 m differ by a grazing contact\n", grazing, n_far));
    // a motion of a thousand kilometres still meets a box
    let h = query::cast(&Shape::sphere(0.1), &Iso::at(Vec3::new(-5e5, 0.0, 0.0)), Vec3::new(1e6, 0.0, 0.0), &Shape::cuboid(Vec3::splat(0.5)), &Iso::IDENTITY);
    c.check(h.is_some_and(|h| ((h.fraction * 1e6 - 5e5) + 0.6).abs() < 1e-6), || format!("1000 km: {:?}", h));
    // contacts list per triangle, sorted
    let grid = Shape::mesh(scene::flat(4, 1.0, 0.0).build());
    let mut cs = Vec::new();
    query::contacts(&Shape::sphere(0.6), &Iso::at(Vec3::new(1.0, 0.5, 1.0)), &grid, &Iso::IDENTITY, 0.0, &mut cs);
    c.check(cs.len() >= 6 && cs.windows(2).all(|w| w[0].triangle < w[1].triangle) && cs.iter().all(|ct| (ct.normal - Vec3::Y).length() < 1e-12 && (ct.depth - 0.1).abs() < 1e-12), || format!("contacts at a vertex: {:?}", cs));
}
