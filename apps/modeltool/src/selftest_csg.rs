// selftest_csg -- the checks of the Boolean operations and the bevel (groups csg and bevel): results against
// independent references (refs.rs: overlaps of boxes, ray parity and the planes of the inputs for points
// sampled around them, the Minkowski sum of a bevelled box), the topology every result must have (closed,
// faces outward, nothing degenerate, the Euler characteristic of its holes), what must be refused.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use geom::{Axis, Mesh, Solid, Vec3};

use crate::refs;
use crate::selftest::{Ctx, Rng};

fn rel(a: f64, b: f64, tol: f64) -> bool {
    geom::math::abs(a - b) <= tol * geom::math::abs(b).max(1e-300)
}

/// closed, outward, nothing degenerate, normals good; `parts` components and Euler characteristic `euler`
/// (None: not checked)
fn good(c: &mut Ctx, what: &str, m: &Mesh, parts: Option<usize>, euler: Option<i64>) -> bool {
    let r = m.check();
    let ok = c.check(r.ok(), || format!("{what}: {}", r.json()));
    let ok = ok && c.check(parts.is_none_or(|p| p == r.components), || format!("{what}: {} components, want {:?}", r.components, parts));
    ok && c.check(euler.is_none_or(|e| e == r.euler), || format!("{what}: euler {} want {:?}", r.euler, euler))
}

/// a box between two corners
fn block(lo: Vec3, hi: Vec3) -> Mesh {
    let s = hi - lo;
    geom::cuboid(s.x, s.y, s.z).unwrap().translated(Vec3::new((lo.x + hi.x) * 0.5, lo.y, (lo.z + hi.z) * 0.5))
}

/// points around both inputs: the result must agree with `want(in a, in b)` (left out near a face)
fn sample(c: &mut Ctx, rng: &mut Rng, what: &str, a: &Mesh, b: &Mesh, result: Option<&Mesh>, want: fn(bool, bool) -> bool, n: usize) {
    let bb = a.bounds().unwrap().union(b.bounds().unwrap());
    let margin = 1e-7 * bb.size().length();
    let (mut tested, mut wrong) = (0, 0);
    for _ in 0..n {
        let p = Vec3::new(rng.range(bb.min.x, bb.max.x), rng.range(bb.min.y, bb.max.y), rng.range(bb.min.z, bb.max.z));
        let (Some(ia), Some(ib)) = (refs::inside_convex(a, p, margin), refs::inside_convex(b, p, margin)) else { continue };
        let expect = want(ia, ib);
        let got = match result {
            Some(m) => refs::inside_mesh(m, p),
            None => Some(false),
        };
        let Some(got) = got else { continue };
        tested += 1;
        if got != expect {
            wrong += 1;
        }
    }
    c.check(wrong == 0, || format!("{what}: {wrong} of {tested} sampled points on the wrong side"));
}

fn union_of(a: bool, b: bool) -> bool {
    a || b
}
fn minus(a: bool, b: bool) -> bool {
    a && !b
}
fn both(a: bool, b: bool) -> bool {
    a && b
}

/// the error text of a result, "" when it is Ok
fn err_text(r: &Result<Mesh, geom::Error>) -> String {
    match r {
        Ok(_) => String::new(),
        Err(e) => String::from(e.message()),
    }
}

pub fn csg(c: &mut Ctx) {
    let mut rng = Rng::new(31);
    // axis-aligned boxes: overlapping, touching at a face, flush, apart, one inside the other
    for i in 0..c.n(300) {
        let mut la = Vec3::new(rng.range(-2.0, 0.0), rng.range(-2.0, 0.0), rng.range(-2.0, 0.0));
        let mut ha = la + Vec3::new(rng.range(0.1, 2.5), rng.range(0.1, 2.5), rng.range(0.1, 2.5));
        let mut lb = Vec3::new(rng.range(-2.5, 1.0), rng.range(-2.5, 1.0), rng.range(-2.5, 1.0));
        let mut hb = lb + Vec3::new(rng.range(0.1, 2.5), rng.range(0.1, 2.5), rng.range(0.1, 2.5));
        // a third of them with faces on A's planes: touching, flush, inside; on a grid of 1/64 m, so the box
        // generator's corners are exact
        if i % 3 == 0 {
            let g = |rng: &mut Rng, lo: i64, hi: i64| (lo + (rng.next_u64() % (hi - lo + 1) as u64) as i64) as f64 / 64.0;
            la = Vec3::new(g(&mut rng, -128, 0), g(&mut rng, -128, 0), g(&mut rng, -128, 0));
            ha = la + Vec3::new(g(&mut rng, 6, 160), g(&mut rng, 6, 160), g(&mut rng, 6, 160));
            lb = Vec3::new(g(&mut rng, -160, 64), g(&mut rng, -160, 64), g(&mut rng, -160, 64));
            hb = lb + Vec3::new(g(&mut rng, 6, 160), g(&mut rng, 6, 160), g(&mut rng, 6, 160));
            let axis = rng.int(0, 2);
            let s = hb - lb;
            let pick = rng.int(0, 3);
            let (lo, hi) = (la[axis], ha[axis]);
            let start = match pick {
                0 => hi,              // touching beyond A's high face
                1 => lo - s[axis],    // touching below A's low face
                2 => lo,              // flush with the low face
                _ => hi - s[axis],    // flush with the high face
            };
            match axis {
                0 => {
                    lb.x = start;
                    hb.x = start + s.x;
                }
                1 => {
                    lb.y = start;
                    hb.y = start + s.y;
                }
                _ => {
                    lb.z = start;
                    hb.z = start + s.z;
                }
            }
        }
        let (a, b) = (block(la, ha), block(lb, hb));
        // the meshes' own bounds (the box corners as the box generator rounds them)
        let (ba, bb) = (a.bounds().unwrap(), b.bounds().unwrap());
        let mut o = [refs::overlap(ba.min.x, ba.max.x, bb.min.x, bb.max.x), refs::overlap(ba.min.y, ba.max.y, bb.min.y, bb.max.y), refs::overlap(ba.min.z, ba.max.z, bb.min.z, bb.max.z)];
        // faces closer than the tolerance are one plane (geom::WELD_RATIO): such an overlap or gap is a contact,
        // and then the bounds are only equal within it
        let weld = geom::WELD_RATIO * ba.union(bb).size().x.max(ba.union(bb).size().y).max(ba.union(bb).size().z);
        let snapped = o.iter().any(|&x| x != 0.0 && geom::math::abs(x) <= weld);
        for x in o.iter_mut() {
            if geom::math::abs(*x) <= weld {
                *x = 0.0;
            }
        }
        let same_box = |p: geom::Aabb, q: geom::Aabb| if snapped { (p.min - q.min).length().max((p.max - q.max).length()) <= 4.0 * weld } else { p == q };
        let (va, vb) = (a.volume(), b.volume());
        let vi = if o.iter().all(|&x| x > 0.0) { o[0] * o[1] * o[2] } else { 0.0 };
        let zeros = o.iter().filter(|&&x| x == 0.0).count();
        let apart = o.iter().any(|&x| x < 0.0);
        let tol = 1e-11;
        // union: one part when they overlap or touch at a face, two when apart; an edge or corner contact is no
        // manifold (left out)
        let u = geom::union(&[&a, &b]);
        if apart || zeros <= 1 {
            let parts = if apart { 2 } else { 1 };
            if c.check(u.is_ok(), || format!("box union {i}: {}", err_text(&u))) {
                let u = u.unwrap();
                if good(c, &format!("box union {i}"), &u, Some(parts), Some(2 * parts as i64)) {
                    c.check(rel(u.volume(), va + vb - vi, tol), || format!("box union {i}: volume {} want {}", u.volume(), va + vb - vi));
                    let ub = u.bounds().unwrap();
                    c.check(same_box(ub, ba.union(bb)), || format!("box union {i}: bounds {:?} want {:?}", ub, ba.union(bb)));
                }
            }
        }
        // difference: empty when B holds A
        let d = geom::difference(&a, &[&b]);
        if rel(va - vi, 0.0, 0.0) || va - vi <= va * 1e-12 {
            c.check(d.is_err(), || format!("box difference {i}: A inside B, but not empty"));
        } else if c.check(d.is_ok(), || format!("box difference {i}: {}", err_text(&d))) {
            let d = d.unwrap();
            if good(c, &format!("box difference {i}"), &d, None, None) {
                c.check(rel(d.volume(), va - vi, tol), || format!("box difference {i}: volume {} want {}", d.volume(), va - vi));
            }
        }
        // intersection: the overlap box, empty without one
        let x = geom::intersection(&[&a, &b]);
        if vi > 0.0 {
            if c.check(x.is_ok(), || format!("box intersection {i}: {}", err_text(&x))) {
                let x = x.unwrap();
                if good(c, &format!("box intersection {i}"), &x, Some(1), Some(2)) {
                    c.check(rel(x.volume(), vi, tol), || format!("box intersection {i}: volume {} want {}", x.volume(), vi));
                    let want = geom::Aabb { min: ba.min.max(bb.min), max: ba.max.min(bb.max) };
                    c.check(same_box(x.bounds().unwrap(), want), || format!("box intersection {i}: bounds {:?} want {:?}", x.bounds(), want));
                }
            }
        } else {
            c.check(x.is_err(), || format!("box intersection {i}: no overlap, but not empty"));
        }
    }
    // turned boxes, cylinders, cones, spheres and extruded profiles against each other: the identities of the
    // volumes and the side of sampled points
    for i in 0..c.n(120) {
        let shape = |rng: &mut Rng, k: usize| -> Mesh {
            let m = match k % 5 {
                0 => geom::cuboid(rng.range(0.3, 2.0), rng.range(0.3, 2.0), rng.range(0.3, 2.0)).unwrap(),
                1 => geom::cylinder(rng.range(0.2, 1.0), rng.range(0.3, 2.0), rng.int(3, 40)).unwrap(),
                2 => geom::cone(rng.range(0.2, 1.0), rng.range(0.3, 2.0), rng.int(3, 24), if rng.int(0, 1) == 0 { 0.0 } else { rng.range(0.05, 0.5) }).unwrap(),
                3 => geom::sphere(rng.range(0.3, 1.0), rng.int(4, 20), rng.int(2, 10)).unwrap(),
                _ => geom::extrude(&geom::poly::circle(rng.range(0.3, 1.0), rng.int(3, 9)), rng.range(0.2, 1.5)).unwrap(),
            };
            let axis = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
            let turn = geom::Affine::rotation_about(axis, rng.range(0.0, 360.0)).unwrap_or(geom::Affine::IDENTITY);
            m.transformed(&turn).translated(Vec3::new(rng.range(-0.6, 0.6), rng.range(-0.6, 0.6), rng.range(-0.6, 0.6)))
        };
        let (ka, kb) = (rng.int(0, 4), rng.int(0, 4));
        let (a, b) = (shape(&mut rng, ka), shape(&mut rng, kb));
        let what = format!("shapes {i} ({ka} {kb})");
        let (sa, sb) = match (Solid::from_convex(&a), Solid::from_convex(&b)) {
            (Ok(x), Ok(y)) => (x, y),
            (x, y) => {
                c.check(false, || format!("{what}: from_convex {:?} {:?}", x.err(), y.err()));
                continue;
            }
        };
        let (va, vb) = (a.volume(), b.volume());
        let mesh = |s: Result<Solid, geom::Error>| s.and_then(|s| s.to_mesh(30.0));
        let (u, d, x) = (mesh(sa.union(&sb)), mesh(sa.difference(&sb)), mesh(sa.intersection(&sb)));
        let vol = |m: &Result<Mesh, geom::Error>| m.as_ref().map_or(0.0, |m| m.volume());
        let (vu, vd, vx) = (vol(&u), vol(&d), vol(&x));
        c.check(rel(vu + vx, va + vb, 1e-9), || format!("{what}: union {vu} + intersection {vx} want {}", va + vb));
        c.check(rel(vd + vx, va, 1e-9), || format!("{what}: difference {vd} + intersection {vx} want {va}"));
        for (name, r, want) in [("union", &u, union_of as fn(bool, bool) -> bool), ("difference", &d, minus), ("intersection", &x, both)] {
            if let Ok(m) = r {
                good(c, &format!("{what} {name}"), m, None, None);
            }
            sample(c, &mut rng, &format!("{what} {name}"), &a, &b, r.as_ref().ok(), want, 40);
        }
    }
    // holes: a prism through a slab is a ring (Euler 0), its volume the slab less the polygon times the thickness
    for i in 0..c.n(80) {
        let (w, h, t) = (rng.range(1.0, 4.0), rng.range(1.0, 4.0), rng.range(0.05, 0.6));
        let n = rng.int(3, 48);
        let r = rng.range(0.05, 0.24) * w.min(h);
        let (cx, cy) = (rng.range(-w * 0.5 + 1.2 * r, w * 0.5 - 1.2 * r), rng.range(1.2 * r, h - 1.2 * r));
        let wall = geom::cuboid(w, h, t).unwrap();
        let hole = geom::cylinder(r, 3.0 * t, n).unwrap().rotated(Axis::X, 90.0).translated(Vec3::new(cx, cy, -1.5 * t));
        let polygon = n as f64 * 0.5 * r * r * refs::sincos(2.0 * refs::PI / n as f64).0;
        match geom::difference(&wall, &[&hole]) {
            Ok(m) => {
                if good(c, &format!("hole {i} n {n}"), &m, Some(1), Some(0)) {
                    c.check(rel(m.volume(), w * h * t - polygon * t, 1e-11), || format!("hole {i}: volume {} want {}", m.volume(), w * h * t - polygon * t));
                }
            }
            Err(e) => {
                c.check(false, || format!("hole {i}: {e}"));
            }
        }
    }
    // a wall with windows and a door, one after the other: genus k, Euler 2 - 2k
    for i in 0..c.n(30) {
        let (l, h, t) = (rng.range(4.0, 9.0), rng.range(2.4, 3.2), rng.range(0.15, 0.4));
        let wall = geom::cuboid(l, h, t).unwrap();
        let mut s = Solid::from_convex(&wall).unwrap();
        let k = rng.int(1, 4);
        let slot = l / k as f64;
        let mut cut = 0.0;
        let mut holes = 0;
        for j in 0..k {
            let bw = slot * rng.range(0.3, 0.7);
            let x0 = -l * 0.5 + slot * j as f64 + (slot - bw) * 0.5;
            let door = j == 0 && i % 2 == 0;
            let (y0, bh) = if door { (0.0, h * 0.75) } else { (h * 0.3, h * 0.45) };
            let opening = block(Vec3::new(x0, y0 - if door { 0.1 } else { 0.0 }, -t), Vec3::new(x0 + bw, y0 + bh, t));
            s = s.difference(&Solid::from_convex(&opening).unwrap()).unwrap();
            cut += bw * bh * t;
            if !door {
                holes += 1;
            }
        }
        match s.to_mesh(30.0) {
            Ok(m) => {
                if good(c, &format!("wall {i} ({k} openings)"), &m, Some(1), Some(2 - 2 * holes)) {
                    c.check(rel(m.volume(), l * h * t - cut, 1e-11), || format!("wall {i}: volume {} want {}", m.volume(), l * h * t - cut));
                }
            }
            Err(e) => {
                c.check(false, || format!("wall {i}: {e}"));
            }
        }
    }
    // a solid moved and turned keeps its cells: (A - B) turned, less C, equals A, B and C turned first
    for i in 0..c.n(20) {
        let a = geom::cuboid(2.0, 1.0, 1.5).unwrap();
        let b = geom::cylinder(0.3, 2.0, rng.int(5, 24)).unwrap().translated(Vec3::new(rng.range(-0.5, 0.5), -0.5, rng.range(-0.3, 0.3)));
        let cc = geom::cuboid(0.6, 0.6, 3.0).unwrap().translated(Vec3::new(rng.range(-1.0, 1.0), 0.2, -1.5));
        let turn = geom::Affine::rotation_about(Vec3::new(0.2, 1.0, 0.1), rng.range(0.0, 360.0)).unwrap().then_after(&geom::Affine::translation(Vec3::new(0.3, 0.1, -0.2)));
        let first = Solid::from_convex(&a).unwrap().difference(&Solid::from_convex(&b).unwrap()).unwrap();
        let x = first.transformed(&turn).difference(&Solid::from_convex(&cc).unwrap()).and_then(|s| s.to_mesh(30.0));
        let y = Solid::from_convex(&a.transformed(&turn)).unwrap().difference(&Solid::from_convex(&b.transformed(&turn)).unwrap()).unwrap().difference(&Solid::from_convex(&cc).unwrap()).and_then(|s| s.to_mesh(30.0));
        match (x, y) {
            (Ok(x), Ok(y)) => {
                good(c, &format!("turned {i}"), &x, None, None);
                c.check(rel(x.volume(), y.volume(), 1e-10), || format!("turned {i}: volume {} want {}", x.volume(), y.volume()));
            }
            (x, y) => {
                c.check(false, || format!("turned {i}: {:?} {:?}", x.err(), y.err()));
            }
        }
    }
    // the same solid with itself; a box split into touching halves and joined again
    let a = geom::cuboid(1.5, 1.0, 0.75).unwrap();
    let u = geom::union(&[&a, &a]).unwrap();
    c.check(u.volume() == a.volume() && u.bounds() == a.bounds() && u.check().ok(), || format!("A + A: volume {}", u.volume()));
    c.check(geom::difference(&a, &[&a]).is_err(), || "A - A is not empty".into());
    let x = geom::intersection(&[&a, &a]).unwrap();
    c.check(x.volume() == a.volume() && x.check().ok(), || format!("A * A: volume {}", x.volume()));
    let halves = geom::union(&[&block(Vec3::new(-0.75, 0.0, -0.375), Vec3::new(0.0, 1.0, 0.375)), &block(Vec3::new(0.0, 0.0, -0.375), Vec3::new(0.75, 1.0, 0.375))]).unwrap();
    let hr = halves.check();
    c.check(hr.ok() && hr.components == 1 && halves.volume() == a.volume(), || format!("halves: {}", hr.json()));
    // a union of many: a row of overlapping crates is one part
    let crates: Vec<Mesh> = (0..6).map(|k| geom::cuboid(1.0, 1.0, 1.0).unwrap().rotated(Axis::Y, 15.0 * k as f64).translated(Vec3::new(0.7 * k as f64, 0.0, 0.0))).collect();
    let refs_: Vec<&Mesh> = crates.iter().collect();
    match geom::union(&refs_) {
        Ok(m) => {
            good(c, "row of crates", &m, Some(1), Some(2));
        }
        Err(e) => {
            c.check(false, || format!("row of crates: {e}"));
        }
    }
    // what is refused: not convex, not closed, two parts, flat
    let stairs = geom::stairs(1.0, 1.0, 1.0, 4).unwrap();
    let e = err_text(&geom::union(&[&a, &stairs]));
    c.check(e.contains("nicht konvex"), || format!("stairs: {e}"));
    let mut open = a.clone();
    open.indices.truncate(open.indices.len() - 3);
    let e = err_text(&geom::union(&[&open, &a]));
    c.check(e.contains("nicht geschlossen"), || format!("open: {e}"));
    let two = Mesh::merge(&[&a, &a.translated(Vec3::new(5.0, 0.0, 0.0))]).unwrap();
    let e = err_text(&geom::difference(&two, &[&a]));
    c.check(e.contains("2 Teilen"), || format!("two parts: {e}"));
    let e = err_text(&geom::intersection(&[&a, &a.translated(Vec3::new(9.0, 0.0, 0.0))]));
    c.check(e.contains("leer"), || format!("disjoint intersection: {e}"));
}

pub fn bevel(c: &mut Ctx) {
    let mut rng = Rng::new(37);
    // boxes: the volume of the Minkowski sum, bounds exact, the flat faces flat and their area
    for i in 0..c.n(120) {
        let (w, h, d) = (rng.range(0.2, 3.0), rng.range(0.2, 3.0), rng.range(0.2, 3.0));
        let r = rng.range(0.02, 0.45) * w.min(h).min(d);
        let k = rng.int(1, 6);
        let b = geom::cuboid(w, h, d).unwrap().translated(Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)));
        let what = format!("bevel box {i} k {k}");
        let m = match geom::bevel(&b, r, k, None) {
            Ok(m) => m,
            Err(e) => {
                c.check(false, || format!("{what}: {e}"));
                continue;
            }
        };
        if !good(c, &what, &m, Some(1), Some(2)) {
            continue;
        }
        c.check(rel(m.volume(), refs::bevel_box_volume(w, h, d, r, k), 1e-11), || format!("{what}: volume {} want {}", m.volume(), refs::bevel_box_volume(w, h, d, r, k)));
        c.check(m.bounds() == b.bounds(), || format!("{what}: bounds {:?} want {:?}", m.bounds(), b.bounds()));
        // the triangles whose three normals are +X: the +X face, (h - 2r) x (d - 2r)
        let mut flat = 0.0;
        let mut seam = true;
        for t in 0..m.triangle_count() {
            let tri = m.triangle(t);
            if tri.iter().all(|&v| m.normals[v as usize] == Vec3::X) {
                let [p, q, s] = m.corners(t);
                flat += (q - p).cross(s - p).length() * 0.5;
            }
        }
        // no seam: at every point of the +X face the bevel's triangles carry the face's normal too
        let xmax = b.bounds().unwrap().max.x;
        for (v, p) in m.positions.iter().enumerate() {
            if p.x == xmax && m.normals[v] != Vec3::X {
                seam = false;
            }
        }
        c.check(rel(flat, (h - 2.0 * r) * (d - 2.0 * r), 1e-11), || format!("{what}: flat +X area {} want {}", flat, (h - 2.0 * r) * (d - 2.0 * r)));
        // (a chamfer is flat: its faces keep their own normals)
        c.check(seam || k == 1, || format!("{what}: a corner on the +X face with another normal"));
    }
    // only the vertical edges: the cross-section is a rectangle with rounded corners, the top and bottom flat
    for i in 0..c.n(40) {
        let (w, h, d) = (rng.range(0.2, 3.0), rng.range(0.2, 3.0), rng.range(0.2, 3.0));
        let r = rng.range(0.02, 0.45) * w.min(d);
        let k = rng.int(1, 8);
        let m = geom::bevel(&geom::cuboid(w, h, d).unwrap(), r, k, Some(Axis::Y)).unwrap();
        let (a, e) = (w - 2.0 * r, d - 2.0 * r);
        let want = h * (a * e + 2.0 * r * (a + e) + refs::bevel_outline_area(r, k));
        if good(c, &format!("bevel y {i}"), &m, Some(1), Some(2)) {
            c.check(rel(m.volume(), want, 1e-11), || format!("bevel y {i}: volume {} want {}", m.volume(), want));
            let top = m.normals.iter().filter(|n| **n == Vec3::Y).count();
            c.check(top >= 4, || format!("bevel y {i}: the top is not flat"));
        }
    }
    // cylinders, frustums and turned boxes: good solids between the original and the shrunk one
    for i in 0..c.n(60) {
        let base = match i % 3 {
            0 => geom::cylinder(rng.range(0.3, 1.0), rng.range(0.3, 2.0), rng.int(5, 32)).unwrap(),
            1 => geom::cone(rng.range(0.5, 1.0), rng.range(0.5, 2.0), rng.int(5, 24), rng.range(0.2, 0.45)).unwrap(),
            _ => geom::cuboid(rng.range(0.5, 2.0), rng.range(0.5, 2.0), rng.range(0.5, 2.0)).unwrap().transformed(&geom::Affine::rotation_about(Vec3::new(0.3, 1.0, -0.4), rng.range(0.0, 360.0)).unwrap()),
        };
        let r = rng.range(0.01, 0.08);
        let k = rng.int(1, 5);
        match geom::bevel(&base, r, k, None) {
            Ok(m) => {
                if good(c, &format!("bevel shape {i}"), &m, Some(1), Some(2)) {
                    c.check(m.volume() < base.volume() && m.volume() > base.volume() * 0.5, || format!("bevel shape {i}: volume {} of {}", m.volume(), base.volume()));
                }
            }
            Err(e) => {
                c.check(false, || format!("bevel shape {i}: {e}"));
            }
        }
    }
    // a bevelled box is a convex input of a Boolean operation: a rounded block with a hole
    let rb = geom::bevel(&geom::cuboid(1.0, 0.6, 1.0).unwrap(), 0.1, 4, None).unwrap();
    let hole = geom::cylinder(0.15, 2.0, 24).unwrap().translated(Vec3::new(0.0, -0.5, 0.0));
    match geom::difference(&rb, &[&hole]) {
        Ok(m) => {
            good(c, "rounded block with a hole", &m, Some(1), Some(0));
        }
        Err(e) => {
            c.check(false, || format!("rounded block with a hole: {e}"));
        }
    }
    // what is refused
    let b = geom::cuboid(1.0, 1.0, 1.0).unwrap();
    for (what, r) in [
        ("too wide", geom::bevel(&b, 0.6, 2, None)),
        ("zero", geom::bevel(&b, 0.0, 2, None)),
        ("no segments", geom::bevel(&b, 0.1, 0, None)),
        ("not convex", geom::bevel(&geom::stairs(1.0, 1.0, 1.0, 3).unwrap(), 0.05, 1, None)),
    ] {
        c.check(r.is_err(), || format!("bevel {what}: not refused"));
    }
}
