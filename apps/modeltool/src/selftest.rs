// selftest -- the checks of --check: lib/geom against the references of refs.rs and against what its operations
// must keep (volumes, bounds, closedness, orientation), the topology checks against meshes broken on purpose, the
// triangulation, the glTF round trip, the script host's messages and the examples' reports.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use geom::{Axis, Mesh, Vec3};

use crate::refs;
use crate::{examples, out, run};

/// the groups in the order --check runs them
pub const ALL: &[&str] = &["prims", "ops", "normals", "checks", "poly", "csg", "bevel", "gltf", "script", "watch", "examples"];

/// xorshift64*: the same numbers on every target
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// uniform in [lo, hi)
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * ((self.next_u64() >> 11) as f64 * (1.0 / 9_007_199_254_740_992.0))
    }

    /// uniform in lo ..= hi
    pub fn int(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next_u64() % (hi - lo + 1) as u64) as usize
    }
}

/// the counters of a group and its first failures
pub struct Ctx {
    pub group: &'static str,
    pub checks: u64,
    pub failed: u64,
    pub quick: bool,
    shown: u32,
}

impl Ctx {
    pub fn check(&mut self, ok: bool, msg: impl FnOnce() -> String) -> bool {
        self.checks += 1;
        if !ok {
            self.failed += 1;
            if self.shown < 12 {
                self.shown += 1;
                out(&format!("FAIL {}: {}\n", self.group, msg()));
            }
        }
        ok
    }

    /// a number of cases, a tenth with --quick
    pub fn n(&self, full: usize) -> usize {
        if self.quick { (full / 10).max(1) } else { full }
    }
}

pub fn run(names: &[&str], quick: bool) -> i32 {
    let (mut checks, mut failed) = (0u64, 0u64);
    for &name in names {
        let Some(&group) = ALL.iter().find(|g| **g == name) else {
            out(&format!("modeltool: unbekannte Gruppe {} (Gruppen: {})\n", name, ALL.join(" ")));
            return 2;
        };
        let mut c = Ctx { group, checks: 0, failed: 0, quick, shown: 0 };
        let t0 = plat::time_us();
        match group {
            "prims" => prims(&mut c),
            "ops" => ops(&mut c),
            "normals" => normals(&mut c),
            "checks" => topo(&mut c),
            "poly" => poly(&mut c),
            "csg" => crate::selftest_csg::csg(&mut c),
            "bevel" => crate::selftest_csg::bevel(&mut c),
            "gltf" => gltf_tests(&mut c),
            "script" => script(&mut c),
            "watch" => watch_tests(&mut c),
            _ => examples_tests(&mut c),
        }
        let ms = (plat::time_us() - t0) / 1000;
        out(&format!("{:<9} {:>8} Prüfungen, {} fehlgeschlagen ({} ms)\n", group, c.checks, c.failed, ms));
        checks += c.checks;
        failed += c.failed;
    }
    out(&format!("modeltool: {} von {} Prüfungen bestanden\n", checks - failed, checks));
    if failed == 0 { 0 } else { 1 }
}

// ---------------------------------------------------------------- helpers

fn rel(a: f64, b: f64, tol: f64) -> bool {
    geom::math::abs(a - b) <= tol * geom::math::abs(b).max(1e-300)
}

/// a good solid: closed, outward, no degenerate triangles, good normals, Euler characteristic `euler`, one
/// component; volume and area against the references
fn solid(c: &mut Ctx, what: &str, m: &Mesh, euler: i64, volume: f64, area: f64) {
    let r = m.check();
    c.check(m.validate().is_ok(), || format!("{what}: arrays do not fit"));
    c.check(r.closed(), || format!("{what}: not closed: open {} nonmanifold {} flipped {} pinched {}", r.open_edges, r.nonmanifold_edges, r.flipped_edges, r.nonmanifold_points));
    c.check(r.outward(), || format!("{what}: not outward (inverted {})", r.inverted_components));
    c.check(r.degenerate == 0, || format!("{what}: {} degenerate triangles", r.degenerate));
    c.check(r.bad_normals == 0, || format!("{what}: {} bad normals", r.bad_normals));
    c.check(r.components == 1 && r.euler == euler, || format!("{what}: {} components, euler {} (want {})", r.components, r.euler, euler));
    c.check(rel(r.volume, volume, 1e-11), || format!("{what}: volume {} want {}", r.volume, volume));
    c.check(rel(r.area, area, 1e-11), || format!("{what}: area {} want {}", r.area, area));
}

fn bounds_eq(c: &mut Ctx, what: &str, m: &Mesh, lo: Vec3, hi: Vec3) {
    let b = m.bounds().unwrap_or(geom::Aabb::point(Vec3::ZERO));
    c.check(b.min == lo && b.max == hi, || format!("{what}: bounds {:?} .. {:?} want {:?} .. {:?}", b.min, b.max, lo, hi));
}

fn bounds_near(c: &mut Ctx, what: &str, m: &Mesh, lo: Vec3, hi: Vec3, tol: f64) {
    let b = m.bounds().unwrap_or(geom::Aabb::point(Vec3::ZERO));
    let d = (b.min - lo).length().max((b.max - hi).length());
    c.check(d <= tol, || format!("{what}: bounds {:?} .. {:?} want {:?} .. {:?}", b.min, b.max, lo, hi));
}

/// a random star-shaped polygon about the origin: n points at increasing angles, random radii
fn star(rng: &mut Rng, n: usize, scale: f64) -> Vec<[f64; 2]> {
    (0..n)
        .map(|i| {
            let a = 2.0 * refs::PI * (i as f64 + rng.range(0.1, 0.9)) / n as f64;
            let r = scale * rng.range(0.3, 1.0);
            let (s, co) = refs::sincos(a);
            [r * co, r * s]
        })
        .collect()
}

/// a random profile for a lathe: from the axis at the bottom outward, up the outside at increasing heights,
/// back to the axis at the top (a simple polygon)
fn lathe_profile(rng: &mut Rng, k: usize) -> Vec<[f64; 2]> {
    let mut p = vec![[0.0, 0.0]];
    let mut y = 0.0;
    for _ in 0..k {
        p.push([rng.range(0.05, 1.0), y]);
        y += rng.range(0.02, 0.5);
    }
    p.push([0.0, y]);
    p
}

// ---------------------------------------------------------------- prims

fn prims(c: &mut Ctx) {
    let mut rng = Rng::new(7);
    for i in 0..c.n(300) {
        let (w, h, d) = (rng.range(0.01, 20.0), rng.range(0.01, 20.0), rng.range(0.01, 20.0));
        let m = geom::cuboid(w, h, d).unwrap();
        let (v, a) = refs::cuboid(w, h, d);
        solid(c, &format!("box {i}"), &m, 2, v, a);
        bounds_eq(c, "box", &m, Vec3::new(-w * 0.5, 0.0, -d * 0.5), Vec3::new(w * 0.5, h, d * 0.5));
        c.check(m.triangle_count() == 12 && m.vertex_count() == 24, || format!("box: {} triangles {} vertices", m.triangle_count(), m.vertex_count()));
    }
    for i in 0..c.n(200) {
        let (r, h, n) = (rng.range(0.01, 5.0), rng.range(0.01, 10.0), rng.int(3, 96));
        let m = geom::cylinder(r, h, n).unwrap();
        let p = refs::cylinder_profile(r, h);
        solid(c, &format!("cylinder {i} n {n}"), &m, 2, refs::lathe_volume(&p, n), refs::lathe_area(&p, n));
        if n % 4 == 0 {
            bounds_eq(c, "cylinder", &m, Vec3::new(-r, 0.0, -r), Vec3::new(r, h, r));
        }
        c.check(m.triangle_count() == 4 * n, || format!("cylinder: {} triangles", m.triangle_count()));
    }
    for i in 0..c.n(200) {
        let (r, h, n) = (rng.range(0.01, 5.0), rng.range(0.01, 10.0), rng.int(3, 96));
        let top = if i % 2 == 0 { 0.0 } else { rng.range(0.01, 5.0) };
        let m = geom::cone(r, h, n, top).unwrap();
        let p: Vec<[f64; 2]> = if top > 0.0 { vec![[0.0, 0.0], [r, 0.0], [top, h], [0.0, h]] } else { vec![[0.0, 0.0], [r, 0.0], [0.0, h]] };
        solid(c, &format!("cone {i} n {n}"), &m, 2, refs::lathe_volume(&p, n), refs::lathe_area(&p, n));
    }
    for i in 0..c.n(150) {
        let (r, n, st) = (rng.range(0.01, 5.0), rng.int(3, 80), rng.int(2, 40));
        let m = geom::sphere(r, n, st).unwrap();
        let p = refs::sphere_profile(r, st);
        solid(c, &format!("sphere {i} n {n} stacks {st}"), &m, 2, refs::lathe_volume(&p, n), refs::lathe_area(&p, n));
        c.check(m.bounds().unwrap().min.y == 0.0 && m.bounds().unwrap().max.y == 2.0 * r, || format!("sphere: y bounds {:?}", m.bounds()));
    }
    for i in 0..c.n(150) {
        let major = rng.range(0.1, 5.0);
        let minor = major * rng.range(0.05, 0.95);
        let (n, s) = (rng.int(3, 80), rng.int(3, 40));
        let m = geom::torus(major, minor, n, s).unwrap();
        let prof: Vec<[f64; 2]> = (0..s)
            .map(|k| {
                let (sn, cs) = geom::math::sin_cos_deg(360.0 * k as f64 / s as f64);
                [major + minor * cs, minor + minor * sn]
            })
            .collect();
        solid(c, &format!("torus {i}"), &m, 0, refs::torus_volume(major, minor, n, s), refs::lathe_area(&prof, n));
    }
    for i in 0..c.n(200) {
        let (w, h, d, n) = (rng.range(0.1, 5.0), rng.range(0.1, 5.0), rng.range(0.1, 8.0), rng.int(1, 40));
        let m = geom::stairs(w, h, d, n).unwrap();
        let (v, a) = refs::stairs(w, h, d, n);
        solid(c, &format!("stairs {i} n {n}"), &m, 2, v, a);
        bounds_near(c, "stairs", &m, Vec3::new(-w * 0.5, 0.0, -d * 0.5), Vec3::new(w * 0.5, h, d * 0.5), 1e-12 * (w + h + d));
    }
    for i in 0..c.n(200) {
        let w = rng.range(0.5, 6.0);
        let t = w * rng.range(0.05, 0.4);
        let r = w * 0.5 - t;
        let h = r + t + rng.range(0.0, 4.0);
        let d = rng.range(0.1, 2.0);
        let n = rng.int(2, 48);
        let m = geom::arch(w, h, d, t, n).unwrap();
        let (pa, pl) = refs::arch(w, h, t, n);
        solid(c, &format!("arch {i} n {n}"), &m, 2, pa * d, 2.0 * pa + pl * d);
        bounds_near(c, "arch", &m, Vec3::new(-w * 0.5, 0.0, -d * 0.5), Vec3::new(w * 0.5, h, d * 0.5), 1e-12 * (w + h + d));
    }
    // what is refused
    let bad: [(&str, Result<Mesh, geom::Error>); 8] = [
        ("box 0", geom::cuboid(0.0, 1.0, 1.0)),
        ("box nan", geom::cuboid(f64::NAN, 1.0, 1.0)),
        ("cylinder 2 segments", geom::cylinder(1.0, 1.0, 2)),
        ("sphere too many", geom::sphere(1.0, 5000, 10)),
        ("torus minor >= major", geom::torus(1.0, 1.0, 8, 8)),
        ("stairs 0 steps", geom::stairs(1.0, 1.0, 1.0, 0)),
        ("arch too low", geom::arch(2.0, 0.9, 1.0, 0.2, 8)),
        ("cone negative top", geom::cone(1.0, 1.0, 8, -1.0)),
    ];
    for (what, r) in bad {
        c.check(r.is_err(), || format!("{what}: not refused"));
    }
}

// ---------------------------------------------------------------- ops

fn ops(c: &mut Ctx) {
    let mut rng = Rng::new(11);
    // turns by multiples of 90 degrees are exact: the bounds permute without rounding
    let b = geom::cuboid(1.0, 2.0, 3.0).unwrap().translated(Vec3::new(0.25, 0.5, -0.75));
    for (axis, deg, lo, hi) in [
        (Axis::Y, 90.0, Vec3::new(-2.25, 0.5, -0.75), Vec3::new(0.75, 2.5, 0.25)),
        (Axis::X, -90.0, Vec3::new(-0.25, -2.25, -2.5), Vec3::new(0.75, 0.75, -0.5)),
        (Axis::Z, 180.0, Vec3::new(-0.75, -2.5, -2.25), Vec3::new(0.25, -0.5, 0.75)),
        (Axis::Y, 450.0, Vec3::new(-2.25, 0.5, -0.75), Vec3::new(0.75, 2.5, 0.25)),
    ] {
        let m = b.rotated(axis, deg);
        bounds_eq(c, &format!("rotate {:?} {}", axis, deg), &m, lo, hi);
        let r = m.check();
        c.check(r.ok() && r.volume == 6.0, || format!("rotate {:?} {}: ok {} volume {}", axis, deg, r.ok(), r.volume));
    }
    // any turn keeps volume and area, any scale multiplies the volume, a mirror keeps the faces outward
    for i in 0..c.n(200) {
        let base = match i % 4 {
            0 => geom::cuboid(rng.range(0.1, 3.0), rng.range(0.1, 3.0), rng.range(0.1, 3.0)).unwrap(),
            1 => geom::cylinder(rng.range(0.1, 2.0), rng.range(0.1, 3.0), rng.int(3, 40)).unwrap(),
            2 => geom::torus(1.0, rng.range(0.1, 0.9), rng.int(3, 30), rng.int(3, 20)).unwrap(),
            _ => geom::stairs(1.0, 1.0, 1.0, rng.int(1, 10)).unwrap(),
        };
        let r0 = base.check();
        c.check(r0.ok(), || format!("base {i}: {}", r0.json()));
        let axis = Vec3::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
        let turned = base.transformed(&geom::Affine::rotation_about(axis, rng.range(-720.0, 720.0)).unwrap());
        let r = turned.check();
        c.check(r.ok() && rel(r.volume, r0.volume, 1e-11) && rel(r.area, r0.area, 1e-11), || format!("turn {i}: ok {} volume {} / {}", r.ok(), r.volume, r0.volume));
        let s = Vec3::new(rng.range(0.2, 3.0), rng.range(0.2, 3.0), rng.range(0.2, 3.0));
        let flip = [1.0, -1.0][rng.int(0, 1)];
        let scaled = base.scaled(Vec3::new(s.x * flip, s.y, s.z));
        let r = scaled.check();
        // (a coarse smooth mesh stretched hard may get normals that lean away from a face even when they are
        // computed again: the check reports them, they are no error of the scale)
        let fine = r.closed() && r.outward() && r.degenerate == 0;
        c.check(fine && rel(r.volume, r0.volume * s.x * s.y * s.z, 1e-11), || format!("scale {i} {:?}: volume want {}: {}", s, r0.volume * s.x * s.y * s.z, r.json()));
        let uniform = base.scaled(Vec3::new(s.x * flip, s.x, s.x)).check();
        c.check(uniform.ok(), || format!("uniform scale {i}: {}", uniform.json()));
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let m = base.mirrored(axis);
            let r = m.check();
            c.check(r.ok() && rel(r.volume, r0.volume, 1e-12), || format!("mirror {i} {:?}: ok {} volume {}", axis, r.ok(), r.volume));
        }
        let f = base.flipped().check();
        c.check(f.inverted_components == 1 && !f.outward() && f.bad_normals == 0 && rel(f.volume, -r0.volume, 1e-12), || format!("flip {i}: inverted {} volume {}", f.inverted_components, f.volume));
        let n = rng.int(1, 6);
        let arr = base.array(n, Vec3::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0), rng.range(-3.0, 3.0))).unwrap().check();
        c.check(arr.closed() && arr.components == n && rel(arr.volume, r0.volume * n as f64, 1e-11), || format!("array {i}: components {} volume {}", arr.components, arr.volume));
        let rad = base.translated(Vec3::new(5.0, 0.0, 0.0)).radial(n, 360.0 / n as f64).unwrap().check();
        c.check(rad.closed() && rad.outward() && rad.components == n && rel(rad.volume, r0.volume * n as f64, 1e-11), || format!("radial {i}: components {} volume {}", rad.components, rad.volume));
    }
    // a radial array of four is exact: bounds symmetric
    let rad = geom::cuboid(1.0, 1.0, 1.0).unwrap().translated(Vec3::new(3.0, 0.0, 0.0)).radial(4, 90.0).unwrap();
    bounds_eq(c, "radial 4", &rad, Vec3::new(-3.5, 0.0, -3.5), Vec3::new(3.5, 1.0, 3.5));
    // touching parts stay apart: two boxes side by side, merged, are two closed parts
    let two = Mesh::merge(&[&geom::cuboid(1.0, 1.0, 1.0).unwrap(), &geom::cuboid(1.0, 1.0, 1.0).unwrap().translated(Vec3::new(1.0, 0.0, 0.0))]).unwrap();
    let r = two.check();
    c.check(r.ok() && r.components == 2 && r.nonmanifold_edges == 0 && r.volume == 2.0, || format!("touching boxes: ok {} components {} nonmanifold {}", r.ok(), r.components, r.nonmanifold_edges));
    // weld: six separate squares make a closed box only after welding
    let mut faces = Mesh::new();
    let cube = geom::cuboid(2.0, 2.0, 2.0).unwrap();
    for t in 0..cube.triangle_count() / 2 {
        let mut pos = Vec::new();
        for k in 0..6 {
            pos.push(cube.positions[cube.indices[6 * t + k] as usize]);
        }
        faces.append(&Mesh::from_triangles(pos, (0..6).collect()).unwrap().flat());
    }
    let open = faces.check();
    let welded = faces.weld(0.0).check();
    c.check(!open.closed() && open.open_edges == 36, || format!("unwelded faces: open edges {}", open.open_edges));
    c.check(welded.ok() && welded.points == 8 && welded.euler == 2 && welded.volume == 8.0, || format!("welded: ok {} points {} euler {}", welded.ok(), welded.points, welded.euler));
    // a half and its mirror image: one surface after weld (also with a tolerance)
    let half = geom::lathe(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], 8, 30.0).unwrap();
    let whole = Mesh::merge(&[&half.translated(Vec3::new(0.0, -1.0, 0.0)), &half.translated(Vec3::new(0.0, -1.0, 0.0)).mirrored(Axis::Y)]).unwrap();
    c.check(whole.check().components == 2, || "mirror pair not welded: two parts".into());
    // (the caps meet as two discs: welded, their triangles lie on each other; the welded surface has an edge of 4)
    let w = whole.weld(1e-9).check();
    c.check(w.components == 1, || format!("mirror pair welded: {} components", w.components));
    // extrude and prism of random star polygons
    for i in 0..c.n(300) {
        let (k, sc) = (rng.int(3, 40), rng.range(0.1, 5.0));
        let p = star(&mut rng, k, sc);
        let d = rng.range(0.01, 5.0);
        let a = refs::area2(&p) * 0.5;
        let per = refs::perimeter(&p);
        let m = geom::extrude(&p, d).unwrap();
        solid(c, &format!("extrude {i}"), &m, 2, a * d, 2.0 * a + per * d);
        let m = geom::prism(&p, d).unwrap();
        solid(c, &format!("prism {i}"), &m, 2, a * d, 2.0 * a + per * d);
        // the same profile clockwise gives the same solid
        let mut rev = p.clone();
        rev.reverse();
        c.check(geom::extrude(&rev, d).unwrap().check().ok(), || format!("extrude {i} clockwise"));
    }
    // sweeps: along a mitred L and a closed square the volume is area times the centre line (the profile's
    // centroid on the path); a closed loop that is not planar and a helix stay closed and outward
    for i in 0..c.n(100) {
        let p = geom::poly::rect(rng.range(0.05, 0.5), rng.range(0.05, 0.5));
        let a = refs::area2(&p) * 0.5;
        let (l1, l2) = (rng.range(1.0, 4.0), rng.range(1.0, 4.0));
        let path = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(l1, 0.0, 0.0), Vec3::new(l1, 0.0, -l2)];
        let m = geom::sweep(&p, &path, false, 0.0).unwrap();
        let r = m.check();
        c.check(r.ok() && rel(r.volume, a * (l1 + l2), 1e-11), || format!("sweep L {i}: ok {} volume {} want {}", r.ok(), r.volume, a * (l1 + l2)));
        let s = rng.range(1.0, 4.0);
        let sq = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(s, 0.0, 0.0), Vec3::new(s, s, 0.0), Vec3::new(0.0, s, 0.0)];
        let m = geom::sweep(&p, &sq, true, 0.0).unwrap();
        let r = m.check();
        c.check(r.ok() && r.euler == 0 && rel(r.volume, a * 4.0 * s, 1e-11), || format!("sweep square {i}: ok {} euler {} volume {} want {}", r.ok(), r.euler, r.volume, a * 4.0 * s));
        let saddle = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(s, 1.0, 0.0), Vec3::new(s, 0.0, s), Vec3::new(0.0, 1.0, s)];
        let r = geom::sweep(&p, &saddle, true, 30.0).unwrap().check();
        c.check(r.ok() && r.euler == 0, || format!("sweep saddle {i}: ok {} closed {} outward {} degenerate {}", r.ok(), r.closed(), r.outward(), r.degenerate));
        let helix: Vec<Vec3> = (0..=48)
            .map(|k| {
                let (sn, cs) = geom::math::sin_cos_deg(15.0 * k as f64);
                Vec3::new(2.0 * cs, 0.05 * k as f64, 2.0 * sn)
            })
            .collect();
        let circle = geom::poly::circle(0.1, 12);
        let r = geom::sweep(&circle, &helix, false, 40.0).unwrap().check();
        c.check(r.ok(), || format!("sweep helix {i}: closed {} outward {} degenerate {} bad normals {}", r.closed(), r.outward(), r.degenerate, r.bad_normals));
    }
    // lathe of random profiles against the discrete Pappus
    for i in 0..c.n(300) {
        let k = rng.int(1, 20);
        let p = lathe_profile(&mut rng, k);
        let n = rng.int(3, 64);
        let m = geom::lathe(&p, n, 30.0).unwrap();
        solid(c, &format!("lathe {i}"), &m, 2, refs::lathe_volume(&p, n), refs::lathe_area(&p, n));
    }
    // what is refused
    let bad: [(&str, bool); 6] = [
        ("extrude 2 points", geom::extrude(&[[0.0, 0.0], [1.0, 0.0]], 1.0).is_err()),
        ("extrude bow tie", geom::extrude(&[[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]], 1.0).is_err()),
        ("sweep path of one point", geom::sweep(&geom::poly::rect(1.0, 1.0), &[Vec3::ZERO, Vec3::ZERO], false, 0.0).is_err()),
        ("sweep turning back", geom::sweep(&geom::poly::rect(0.1, 0.1), &[Vec3::ZERO, Vec3::X, Vec3::ZERO], false, 0.0).is_err()),
        ("lathe negative radius", geom::lathe(&[[0.0, 0.0], [-1.0, 0.0], [0.0, 1.0]], 8, 30.0).is_err()),
        ("array of 0", geom::cuboid(1.0, 1.0, 1.0).unwrap().array(0, Vec3::X).is_err()),
    ];
    for (what, refused) in bad {
        c.check(refused, || format!("{what}: not refused"));
    }
}

// ---------------------------------------------------------------- normals

fn normals(c: &mut Ctx) {
    let mut rng = Rng::new(13);
    for i in 0..c.n(100) {
        // flat: every corner has its face's normal
        let m = geom::cuboid(rng.range(0.1, 2.0), rng.range(0.1, 2.0), rng.range(0.1, 2.0)).unwrap().rotated(Axis::Y, rng.range(0.0, 90.0)).flat();
        let mut ok = true;
        for t in 0..m.triangle_count() {
            let [p, q, r] = m.corners(t);
            let f = (q - p).cross(r - p).normalize_or_zero();
            for &v in &m.triangle(t) {
                ok &= m.normals[v as usize] == f.unsigned_zeros();
            }
        }
        c.check(ok, || format!("flat {i}: a corner without its face's normal"));
        // a box smoothed at 30 degrees stays flat
        let b = geom::cuboid(1.0, 2.0, 3.0).unwrap();
        c.check(b.smooth(30.0) == b.flat().uv_box(1.0) || b.smooth(30.0).vertex_count() == 24, || "box smooth(30) is not flat".into());
        // a sphere's normals point away from its centre
        let (r, n) = (rng.range(0.1, 3.0), rng.int(8, 64));
        let s = geom::sphere(r, n, n / 2).unwrap();
        let centre = Vec3::new(0.0, r, 0.0);
        let limit = geom::math::sin_cos_deg(360.0 / n as f64).1;
        let worst = s.positions.iter().zip(&s.normals).map(|(p, nn)| (*p - centre).normalize_or_zero().dot(*nn)).fold(2.0, f64::min);
        c.check(worst >= limit, || format!("sphere {i} n {n}: normal against radius cos {} < {}", worst, limit));
        let unit = s.normals.iter().all(|nn| geom::math::abs(nn.length() - 1.0) < 1e-12);
        c.check(unit, || format!("sphere {i}: normals not unit"));
        // a cylinder: caps exactly up and down, sides horizontal and radial
        let (cr, ch, cn) = (rng.range(0.1, 2.0), rng.range(0.1, 2.0), rng.int(12, 64));
        let cyl = geom::cylinder(cr, ch, cn).unwrap();
        let mut side_ok = true;
        let mut cap_ok = true;
        for t in 0..cyl.triangle_count() {
            let [p, q, rr] = cyl.corners(t);
            let f = (q - p).cross(rr - p).normalize_or_zero();
            for &v in &cyl.triangle(t) {
                let nn = cyl.normals[v as usize];
                if geom::math::abs(f.y) > 0.5 {
                    cap_ok &= nn == Vec3::new(0.0, f.y.signum(), 0.0);
                } else {
                    let p = cyl.positions[v as usize];
                    let radial = Vec3::new(p.x, 0.0, p.z).normalize_or_zero();
                    side_ok &= geom::math::abs(nn.y) < 1e-15 && radial.dot(nn) > 1.0 - 1e-12;
                }
            }
        }
        c.check(cap_ok, || format!("cylinder {i}: a cap normal is not exactly up or down"));
        c.check(side_ok, || format!("cylinder {i} n {cn}: a side normal is not horizontal and radial"));
        // below the threshold: an octagonal prism smoothed at 10 degrees is flat
        let oct = geom::cylinder(1.0, 1.0, 8).unwrap().smooth(10.0);
        c.check(oct.vertex_count() == geom::cylinder(1.0, 1.0, 8).unwrap().flat().vertex_count(), || "octagon smooth(10) is not flat".into());
    }
    // box UV: one unit per metre (and per scale), the projection per face
    let b = geom::cuboid(1.5, 2.5, 0.5).unwrap().translated(Vec3::new(0.3, 0.2, 0.1));
    for scale in [1.0, 0.5, 2.0] {
        let m = b.uv_box(scale);
        let mut ok = true;
        for t in 0..m.triangle_count() {
            let [p, q, r] = m.corners(t);
            let f = (q - p).cross(r - p).normalize_or_zero();
            for &v in &m.triangle(t) {
                let (pp, uv) = (m.positions[v as usize], m.uvs[v as usize]);
                let want = if f.y > 0.5 {
                    [pp.x, -pp.z]
                } else if f.y < -0.5 {
                    [pp.x, pp.z]
                } else if f.x > 0.5 {
                    [-pp.z, pp.y]
                } else if f.x < -0.5 {
                    [pp.z, pp.y]
                } else if f.z > 0.5 {
                    [pp.x, pp.y]
                } else {
                    [-pp.x, pp.y]
                };
                ok &= rel(uv[0], (want[0] + 0.0) / scale, 1e-15) || geom::math::abs(uv[0] - want[0] / scale) < 1e-15;
                ok &= rel(uv[1], (want[1] + 0.0) / scale, 1e-15) || geom::math::abs(uv[1] - want[1] / scale) < 1e-15;
            }
        }
        c.check(ok, || format!("uv_box {scale}: a corner off its projection"));
        c.check(m.check().ok(), || format!("uv_box {scale}: no longer closed"));
    }
}

// ---------------------------------------------------------------- topology checks

fn topo(c: &mut Ctx) {
    let cube = geom::cuboid(1.0, 1.0, 1.0).unwrap();
    let r = cube.check();
    c.check(r.ok() && r.points == 8 && r.edges == 18 && r.euler == 2, || format!("cube: points {} edges {} euler {}", r.points, r.edges, r.euler));
    // a hole: one triangle less
    let mut hole = cube.clone();
    hole.indices.truncate(hole.indices.len() - 3);
    let r = hole.check();
    c.check(!r.closed() && r.open_edges == 3 && r.inverted_components == 0, || format!("hole: open edges {}", r.open_edges));
    // where the first problem lies: on an edge of the hole (the missing triangle's edges lie at x = 0.5, the -X face
    // of the box is the last one: its corners are at x = -0.5); none for the good cube
    c.check(cube.check().problem_at.is_none() && r.problem_at.is_some_and(|p| p.x == -0.5), || format!("hole: problem at {:?}", r.problem_at));
    // one triangle turned
    let mut turned = cube.clone();
    turned.indices.swap(1, 2);
    let r = turned.check();
    c.check(!r.closed() && !r.outward() && r.flipped_edges == 3, || format!("turned: flipped edges {}", r.flipped_edges));
    // inside out
    let r = cube.flipped().check();
    c.check(r.closed() && !r.outward() && r.inverted_components == 1 && r.volume < 0.0, || format!("inside out: inverted {}", r.inverted_components));
    // a degenerate triangle: a sliver along an edge, and one with a repeated point
    let mut sliver = cube.clone();
    let base = sliver.positions.len() as u32;
    sliver.positions.extend_from_slice(&[Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0), Vec3::new(7.0, 1e-12, 0.0)]);
    sliver.normals.extend_from_slice(&[Vec3::Y; 3]);
    sliver.uvs.extend_from_slice(&[[0.0; 2]; 3]);
    let pb = sliver.point_count() as u32;
    sliver.points.extend_from_slice(&[pb, pb + 1, pb + 2]);
    sliver.indices.extend_from_slice(&[base, base + 1, base + 2, base, base, base + 1]);
    let r = sliver.check();
    c.check(r.degenerate == 2, || format!("sliver: degenerate {}", r.degenerate));
    // (its triangles are alone: their open edges come before the degenerate ones)
    c.check(r.problem_at.is_some_and(|p| p.x == 5.5 && p.z == 0.0), || format!("sliver: problem at {:?}", r.problem_at));
    // an edge of four triangles: two cubes welded along an edge
    let two = Mesh::merge(&[&cube, &cube.translated(Vec3::new(1.0, 1.0, 0.0))]).unwrap().weld(0.0);
    let r = two.check();
    c.check(!r.closed() && r.nonmanifold_edges == 1, || format!("edge of four: nonmanifold edges {}", r.nonmanifold_edges));
    // a pinched point: two cubes welded at a corner
    let pinch = Mesh::merge(&[&cube, &cube.translated(Vec3::new(1.0, 1.0, 1.0))]).unwrap().weld(0.0);
    let r = pinch.check();
    c.check(!r.closed() && r.nonmanifold_points == 1 && r.nonmanifold_edges == 0, || format!("pinched: points {} edges {}", r.nonmanifold_points, r.nonmanifold_edges));
    // bad normals: zero, and pointing in
    let mut bad = cube.clone();
    bad.normals[0] = Vec3::ZERO;
    let n0 = bad.indices.iter().filter(|&&i| i == 0).count();
    let r = bad.check();
    c.check(r.bad_normals == n0, || format!("zero normal: bad {} want {}", r.bad_normals, n0));
    let mut inward = cube.clone();
    for n in inward.normals.iter_mut() {
        *n = -*n;
    }
    let r = inward.check();
    c.check(r.bad_normals == 36, || format!("inward normals: bad {}", r.bad_normals));
    // Euler characteristic and components
    let t = geom::torus(2.0, 0.5, 24, 12).unwrap().check();
    c.check(t.ok() && t.euler == 0 && t.components == 1, || format!("torus: euler {}", t.euler));
    let s2 = Mesh::merge(&[&geom::sphere(1.0, 16, 8).unwrap(), &geom::sphere(1.0, 16, 8).unwrap().translated(Vec3::new(3.0, 0.0, 0.0))]).unwrap().check();
    c.check(s2.ok() && s2.euler == 4 && s2.components == 2, || format!("two spheres: euler {} components {}", s2.euler, s2.components));
    // the JSON of a report
    let j = cube.check().json();
    c.check(j.starts_with("{\"ok\": true, \"closed\": true") && j.contains("\"size\": [1, 1, 1]") && j.ends_with("\"bad_normals\": 0}"), || format!("json: {j}"));
    // empty
    let e = Mesh::new().check();
    c.check(!e.ok() && e.triangles == 0 && e.bounds.is_none(), || "empty mesh".into());
}

// ---------------------------------------------------------------- polygons

fn poly(c: &mut Ctx) {
    let mut rng = Rng::new(17);
    for i in 0..c.n(2000) {
        let n = rng.int(3, 80);
        let sc = rng.range(0.01, 100.0);
        let p = geom::poly::ccw(star(&mut rng, n, sc));
        let tris = match geom::poly::triangulate(&p) {
            Ok(t) => t,
            Err(e) => {
                c.check(false, || format!("star {i} ({n} points): {e}"));
                continue;
            }
        };
        c.check(tris.len() == n - 2, || format!("star {i}: {} triangles for {} points", tris.len(), n));
        let mut sum = 0.0;
        let mut positive = true;
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for t in &tris {
            let a = refs::area2(&[p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]]);
            positive &= a > 0.0;
            sum += a;
            for k in 0..3 {
                edges.push((t[k], t[(k + 1) % 3]));
            }
        }
        c.check(positive, || format!("star {i}: a triangle without area or turned"));
        c.check(rel(sum, refs::area2(&p), 1e-12), || format!("star {i}: area {} want {}", sum, refs::area2(&p)));
        // every polygon edge once forward, every inner edge once each way
        let mut ok = true;
        for &(a, b) in &edges {
            let boundary = (b as usize == (a as usize + 1) % n) || (a as usize == (b as usize + 1) % n);
            let fwd = edges.iter().filter(|&&e| e == (a, b)).count();
            let back = edges.iter().filter(|&&e| e == (b, a)).count();
            ok &= fwd == 1 && if boundary { back == 0 && b as usize == (a as usize + 1) % n } else { back == 1 };
        }
        c.check(ok, || format!("star {i}: the triangles do not tile the polygon"));
    }
    // points on straight runs stay on the boundary, no triangle without area
    let p = vec![[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0], [3.0, 1.0], [1.5, 1.0], [0.0, 1.0], [0.0, 0.5]];
    let t = geom::poly::triangulate(&p).unwrap();
    let ok = t.len() == 6 && t.iter().all(|t| refs::area2(&[p[t[0] as usize], p[t[1] as usize], p[t[2] as usize]]) > 0.0);
    c.check(ok, || format!("straight runs: {:?}", t));
    // what is refused: a bow tie, collinear points, too few points
    c.check(geom::poly::check_simple(&[[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 1.0]], "x").is_err(), || "bow tie not refused".into());
    c.check(geom::poly::clean(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], "x").is_err(), || "collinear not refused".into());
    c.check(geom::poly::clean(&[[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]], "x").is_err(), || "two points not refused".into());
    // the helpers: rect, circle exact at the axes, rounded rectangles of the right area
    c.check(refs::area2(&geom::poly::rect(2.0, 3.0)) == 12.0, || "rect".into());
    let circ = geom::poly::circle(2.0, 8);
    c.check(circ[0] == [2.0, 0.0] && circ[2] == [0.0, 2.0] && circ[4] == [-2.0, 0.0] && circ[6] == [0.0, -2.0], || format!("circle: {:?}", circ));
    for k in 1..8 {
        let r = 0.3;
        let rr = geom::poly::rounded_rect(2.0, 1.0, r, k);
        let want = 2.0 * 1.0 - 4.0 * r * r + 4.0 * (k as f64 * 0.5 * r * r * refs::sincos(refs::PI / (2.0 * k as f64)).0);
        c.check(rel(refs::area2(&rr) * 0.5, want, 1e-12) && rr.len() == 4 * (k + 1), || format!("rounded {k}: area {} want {}", refs::area2(&rr) * 0.5, want));
    }
}

// ---------------------------------------------------------------- glTF

fn gltf_tests(c: &mut Ctx) {
    for ex in examples::ALL {
        let mut loader = examples::loader;
        let r = run::run_script(&format!("{}.jfs", ex.name), ex.source, &[], &mut loader);
        if !c.check(r.ok(), || format!("{}: the script failed", ex.name)) {
            continue;
        }
        let doc = crate::export::document(&r.model);
        let glb = match gltf::write_document(&doc) {
            Ok(b) => b,
            Err(e) => {
                c.check(false, || format!("{}: write_document: {:?}", ex.name, e));
                continue;
            }
        };
        let back = match gltf::load(&glb, &mut |_| None, &gltf::Limits::DEFAULT) {
            Ok(d) => d,
            Err(e) => {
                c.check(false, || format!("{}: load: {:?}", ex.name, e));
                continue;
            }
        };
        c.check(back.meshes == doc.meshes, || format!("{}: meshes differ after the round trip", ex.name));
        c.check(back.materials == doc.materials, || format!("{}: materials differ: {:?} / {:?}", ex.name, back.materials.first(), doc.materials.first()));
        c.check(back.nodes == doc.nodes && back.scenes == doc.scenes && back.scene == Some(0), || format!("{}: nodes or scenes differ", ex.name));
        c.check(back.notes.is_empty(), || format!("{}: notes {:?}", ex.name, back.notes));
        c.check(back.instances(None).len() == r.model.parts.len(), || format!("{}: instances", ex.name));
    }
    // a node with a matrix and children, a material with every factor
    let mut doc = gltf::Document { version: "2.0".into(), ..Default::default() };
    let m = geom::cuboid(1.0, 1.0, 1.0).unwrap();
    doc.meshes.push(gltf::Mesh {
        name: "w".into(),
        primitives: vec![gltf::Primitive { positions: m.positions_f32(), normals: m.normals_f32(), uvs: m.uvs_f32(), indices: m.indices.clone(), material: Some(0), flat_normals: false, has_uvs: false }],
    });
    doc.materials.push(gltf::Material { name: "m".into(), base_color_factor: [0.1, 0.2, 0.3, 0.4], metallic_factor: 0.25, roughness_factor: 0.75, emissive_factor: [0.5, 0.0, 1.0], alpha_mode: gltf::AlphaMode::Mask, alpha_cutoff: 0.3, double_sided: true, ..Default::default() });
    let local = gltf::trs_matrix([1.5, -2.0, 0.25], [0.0, 0.38268343236508984, 0.0, 0.9238795325112867], [2.0, 1.0, 0.5]);
    doc.nodes.push(gltf::Node { name: "a".into(), mesh: None, children: vec![1], parent: None, local, world: local, light: None });
    doc.nodes.push(gltf::Node { name: "b".into(), mesh: Some(0), children: vec![], parent: Some(0), local: gltf::IDENTITY, world: local, light: None });
    doc.scenes.push(gltf::SceneDef { name: "s".into(), nodes: vec![0] });
    doc.scene = Some(0);
    let back = gltf::load(&gltf::write_document(&doc).unwrap(), &mut |_| None, &gltf::Limits::DEFAULT).unwrap();
    // without uv the reader fills zeros: compare all but them
    let (p0, p1) = (&doc.meshes[0].primitives[0], &back.meshes[0].primitives[0]);
    c.check(p0.positions == p1.positions && p0.normals == p1.normals && p0.indices == p1.indices && !p1.has_uvs, || "primitive without uv".into());
    c.check(back.materials == doc.materials, || format!("material: {:?}", back.materials));
    c.check(back.nodes == doc.nodes, || format!("nodes: {:?}", back.nodes));
    // what write_document refuses
    let mut tex = doc.clone();
    tex.materials[0].base_color_texture = Some(gltf::TextureRef { texture: 0, tex_coord: 0 });
    c.check(matches!(gltf::write_document(&tex), Err(gltf::Error::Unsupported(_))), || "a texture was not refused".into());
    let mut idx = doc.clone();
    idx.meshes[0].primitives[0].indices[0] = 9999;
    c.check(matches!(gltf::write_document(&idx), Err(gltf::Error::Invalid(_))), || "an index out of range was not refused".into());
    let mut nan = doc.clone();
    nan.meshes[0].primitives[0].positions[0][1] = f32::NAN;
    c.check(matches!(gltf::write_document(&nan), Err(gltf::Error::Invalid(_))), || "NaN was not refused".into());
}

// ---------------------------------------------------------------- the script host

/// run a script given as text; (run, problems as the console shows them)
fn script_run(src: &str, params: &[(&str, &str)]) -> (run::Run, String) {
    let params: Vec<(String, String)> = params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let mut loader = |p: &str| -> Result<String, String> {
        match p {
            "lib/teil.jfs" => Ok(String::from("fn wuerfel(a) { return geom.box(a, a, a) }\nlet _privat = 1\n")),
            "lib/kaputt.jfs" => Ok(String::from("let x = 1\nlet y = geom.box(1, 0, 1)\n")),
            "lib/regel.jfs" => Ok(String::from("fn regel() {\n  model.expect_size(nil, 1, nil)\n}\n")),
            "lib/warnt.jfs" => Ok(String::from("fn f() {\n  return geom.box(1, 1, 1).flatt()\n}\n")),
            _ => Err(String::from("die Datei gibt es nicht")),
        }
    };
    let r = run::run_script("test.jfs", src, &params, &mut loader);
    let sources = vec![(String::from("test.jfs"), String::from(src))];
    let text: String = r.problems.iter().map(|p| run::format_problem(p, &sources)).collect();
    (r, text)
}

/// whether the report of a run (without the model's measurements) has this text
fn json_has(r: &run::Run, s: &str) -> bool {
    crate::json::write(&run::report(r, None, None, None)).contains(s)
}

/// (S2) meshes and vectors as host objects, meshes freed during the run, the language services' warnings
fn host_objects(c: &mut Ctx) {
    // a mesh: its type, its text, the property tris, identity; nothing of it changes, the map's methods and fields
    // are not its, and a wrong member names what there is
    let src = "let m = geom.box(2, 1, 1)\nassert(type(m) == \"mesh\" && str(m) == \"mesh(12 Dreiecke)\" && m.tris == 12, \"mesh\")\nassert(m == m && m != m.move(0, 0, 0) && [m].contains(m), \"identity\")\nlet e = []\nfor f in [fn() { m.tris = 3 }, fn() { m.keys() }, fn() { m.tris() }, fn() { m.move }, fn() { m.mesh }] {\n  try { f() } catch x { e.push(x) }\n}\nprint(e.join(\"|\"))\nmodel.add(m)\n";
    let (r, text) = script_run(src, &[]);
    let got: Vec<&str> = r.output.trim_end().split('|').collect();
    let want = [
        "mesh.tris ist nur lesbar: ein mesh ändert sich nie; jede Methode gibt ein neues",
        "mesh hat keine Methode keys (es gibt: move, place, center, rotate_x, rotate_y, rotate_z, rotate, scale, mirror, flip, weld, array, radial, flat, smooth, bevel, bounds, area, volume, check)",
        "tris ist eine Eigenschaft von mesh, keine Methode – ohne Klammern: .tris",
        "move ist eine Methode von mesh, keine Eigenschaft – mit Klammern: move(..)",
        "mesh hat keine Eigenschaft mesh (es gibt: tris)",
    ];
    c.check(r.ok() && got == want, || format!("mesh object: {text} {:?}", got));
    // the language services saw the same members: warnings with their lines, before the run's
    let lines: Vec<(u32, u32)> = r.warnings.iter().map(|w| (w.line, w.col)).collect();
    c.check(lines == [(5, 20), (5, 41), (5, 60), (5, 79), (5, 96)], || format!("mesh members: {:?}", r.warnings));
    // a vec: made from three numbers or a list, x / y / z, + - between vectors, * / with a number, the minus, ==
    // by the numbers; its methods; vec.* on vecs and lists (a list of 2 stays a list); every point and vector of
    // the host takes it
    let src = concat!(
        "let v = vec(1, 2, 3)\n",
        "let w = vec([4, 5, 6])\n",
        "assert(type(v) == \"vec\" && v.x == 1 && v.y == 2 && v.z == 3 && str(v) == \"vec(1.0, 2.0, 3.0)\", \"vec\")\n",
        "assert(v + w == vec(5, 7, 9) && w - v == vec(3, 3, 3) && v * 2 == vec(2, 4, 6) && 2 * v == v * 2 && v / 2 == vec(0.5, 1, 1.5) && -v == vec(-1, -2, -3), \"operators\")\n",
        "assert(v == vec(1.0, 2.0, 3.0) && v != w && [w].contains(vec(4, 5, 6)) && vec(v) == v, \"equal\")\n",
        "assert(v.dot(w) == 32 && v.cross(w) == vec(-3, 6, -3) && vec(3, 4, 0).length() == 5 && vec(0, 0, 2).normalized() == vec(0, 0, 1) && v.distance(v + vec(0, 3, 4)) == 5 && v.lerp(w, 0.5) == vec(2.5, 3.5, 4.5) && v.dot([0, 1, 0]) == 2, \"methods\")\n",
        "assert(vec.add(v, [1, 1, 1]) == vec(2, 3, 4) && vec.sub([1, 2, 3], [1, 1, 1]) == vec(0, 1, 2) && type(vec.add([1, 2], [3, 4])) == \"list\" && vec.add([1, 2], [3, 4])[1] == 6 && vec.len(v - v) == 0 && vec.norm(w).length() == vec.len(vec.norm([4, 5, 6])), \"vec.*\")\n",
        "let m = geom.box(1, 1, 1)\n",
        "assert(m.move(v).bounds().min == m.move(1, 2, 3).bounds().min && m.move([1, 2, 3]).bounds().max == vec(1.5, 3, 3.5), \"move\")\n",
        "assert(m.bounds().size == vec(1, 1, 1) && m.bounds().center == vec(0, 0.5, 0), \"bounds\")\n",
        "let e = []\n",
        "for f in [fn() { v.x = 5 }, fn() { v[0] }, fn() { v * v }, fn() { v / 0 }, fn() { v + [1, 2, 3] }, fn() { vec(1, 2) }, fn() { vec(0, 0, 0).normalized() }, fn() { v.q }, fn() { m.move([1, 2]) }, fn() { m.move(1, 2) }, fn() { vec(1e300, 0, 0) * 1e10 }] {\n",
        "  try { f() } catch x { e.push(x) }\n",
        "}\n",
        "print(e.join(\"|\"))\n",
        "model.add(geom.sweep(profile.circle(0.1, 8), [vec(0, 0, 0), [1, 0, 0], vec(1, 1, 0)]).rotate(30, vec(0, 1, 0)).scale(vec(1, 2, 1)).array(2, vec(3, 0, 0)).place(v))\n",
        "model.expect_size(vec(1, 1, 1), 10)\n",
    );
    let (r, text) = script_run(src, &[]);
    let got: Vec<&str> = r.output.trim_end().split('|').collect();
    let want = [
        "vec.x ist nur lesbar: ein vec ändert sich nie; einen neuen macht vec(x, y, z)",
        "vec lässt sich nicht indizieren",
        "Der Operator * passt nicht zu vec und vec",
        "vec / int: das Ergebnis ist keine endliche Zahl (geteilt durch 0 oder zu groß)",
        "Der Operator + passt nicht zu vec und list",
        "vec: z muss eine Zahl sein, nicht nil",
        "normalized(): ein Nullvektor hat keine Richtung",
        "vec hat keine Eigenschaft q (es gibt: x, y, z)",
        "move(): die Verschiebung muss ein vec oder eine Liste aus 3 Zahlen sein, nicht list [1, 2]",
        "move(): die Verschiebung muss eine Zahl sein, nicht nil",
        "vec * float: das Ergebnis ist keine endliche Zahl (geteilt durch 0 oder zu groß)",
    ];
    c.check(r.ok() && got == want, || format!("vec: {text} {:?}", got));
    c.check(r.model.expects.first().is_some_and(|e| e.size == [1.0, 1.0, 1.0] && e.tolerance == 10.0), || "expect_size(vec)".into());
    // meshes the script no longer holds are freed during the run, but never a part's, and an id freed and used
    // again is another mesh; the triangle limit counts every mesh made (above: m.array in a loop)
    let src = "let keep = geom.box(1, 2, 3)\nfor i in 0..20000 { let t = geom.box(1, 1, 1) }\nassert(keep.bounds().size == vec(1, 2, 3), \"keep\")\nfor i in 0..200 { model.add(geom.box(1, 1, 1).move(i, 0, 0)) }\nfor i in 0..20000 { let t = geom.box(1, 1, 1) }\n";
    let (r, text) = script_run(src, &[]);
    let min_x = |i: usize| r.model.parts.get(i).and_then(|p| p.mesh.bounds()).map_or(f64::NAN, |b| b.min.x);
    c.check(r.ok() && r.meshes == 40_401 && r.meshes_peak < 10_000 && r.model.parts.len() == 200, || format!("drops: {text} made {} peak {}", r.meshes, r.meshes_peak));
    c.check(min_x(0) == -0.5 && min_x(199) == 198.5 && r.model.parts.iter().all(|p| p.mesh.triangle_count() == 12), || format!("drops: parts {} .. {}", min_x(0), min_x(199)));
    // the language services' warnings (ls::check with host::api): a member a mesh does not have, a name of a
    // namespace that is not there, a division of two ints, in the script and in a module; before the run's
    let src = "let m = geom.box(1, 1, 1)\nif false {\n  m.mov(1, 0, 0)\n  print(m.trs, vec.ad([1, 2, 3], [0, 0, 0]))\n}\nlet k = 7 / 2\nlet t = import(\"lib/warnt.jfs\")\nmodel.add(m.move(k, 0, 0))\nmodel.name(\"A\")\nmodel.name(\"B\")\n";
    let (r, text) = script_run(src, &[]);
    let got: Vec<String> = r.warnings.iter().map(|w| format!("{}:{}:{}: {}", w.file, w.line, w.col, w.msg)).collect();
    let want = [
        "test.jfs:3:5: mesh hat keine Methode mov – meinst du move?",
        "test.jfs:4:11: mesh hat keine Eigenschaft trs – meinst du tris?",
        "test.jfs:4:20: Unbekannter Name vec.ad – meinst du vec.add?",
        "test.jfs:6:9: Ganzzahlige Division: 7 / 2 schneidet ab (7 / 2 ist 3) – float(7) / 2 rechnet mit Komma, int(7 / 2) schneidet ausdrücklich ab",
        "lib/warnt.jfs:2:28: mesh hat keine Methode flatt – meinst du flat?",
        "test.jfs:10:7: model.name: \"B\" ersetzt den Namen \"A\" aus test.jfs:9",
    ];
    c.check(r.ok() && got == want, || format!("ls warnings: {text} {:#?}", got));
    c.check(json_has(&r, "{\"file\": \"lib/warnt.jfs\", \"line\": 2, \"col\": 28, \"message\": \"mesh hat keine Methode flatt"), || "ls warning in the report".into());
}

fn script(c: &mut Ctx) {
    // a good script: parts, materials, expectations met
    let (r, text) = script_run(
        "model.name(\"t\")\nlet a = geom.box(1, 2, 3)\nmodel.add(a, \"holz\")\nmodel.add(a.move(2, 0, 0).rotate_y(90), {color: \"#ff8000\"}, \"zwei\")\nmodel.expect_size(nil, 2, nil)\nmodel.budget(100)\n",
        &[],
    );
    c.check(r.ok(), || format!("good script: {text}"));
    let a = run::analyse(&r.model);
    c.check(a.ok() && r.model.parts.len() == 2 && r.model.materials.len() == 2 && r.model.parts[1].name == "zwei", || "good script: model".into());
    c.check(r.model.materials[1].color == [1.0, 128.0 / 255.0, 0.0], || format!("hex colour: {:?}", r.model.materials[1].color));
    // the messages name the file, line and column, show the line and point at it
    let cases: [(&str, &str, &str); 12] = [
        ("let a = geom.box(1, -2, 1)\nmodel.add(a)\n", "test.jfs:1:", "geom.box: die Höhe muss eine Zahl größer 0 sein, nicht -2"),
        ("let a = 1\nlet b = geom.box(\"x\", 1, 1)\n", "test.jfs:2:", "geom.box: w muss eine Zahl sein, nicht str \"x\""),
        ("let a = 1\n\nmodel.add(boks(1, 1, 1))\n", "test.jfs:3:", "Unbekannter Name boks"),
        ("let m = geom.box(1, 1, 1)\nmodel.add(m.spin(3))\n", "test.jfs:2:", "mesh hat keine Methode spin (es gibt: move, place, center,"),
        ("model.add(geom.box(1, 1, 1), \"plastik\")\n", "test.jfs:1:", "das Material \"plastik\" gibt es nicht"),
        ("let p = geom.extrude([[0, 0], [2, 2], [2, 0], [0, 1]], 1)\n", "test.jfs:1:", "schneiden sich"),
        ("let x = 1\n", "test.jfs: Fehler: Das Skript hat dem Modell nichts hinzugefügt", ""),
        ("let t = import(\"lib/fehlt.jfs\")\n", "test.jfs:1:", "lib/fehlt.jfs"),
        ("let t = import(\"lib/kaputt.jfs\")\n", "lib/kaputt.jfs:2:", "die Höhe muss eine Zahl größer 0 sein"),
        ("while true { }\n", "test.jfs:1:", "Rechenzeit aufgebraucht"),
        ("let m = geom.box(1, 1, 1)\nfor i in 0..100000 { m = m.array(2, 1, 0, 0) }\n", "test.jfs:2:", "Dreiecke"),
        ("model.add(geom.box(1, 1, 1).mirror(\"w\"))\n", "test.jfs:1:", "die Achse muss \"x\", \"y\" oder \"z\" sein"),
    ];
    for (src, at, msg) in cases {
        let (r, text) = script_run(src, &[]);
        c.check(!r.ok() && text.contains(at) && text.contains(msg), || format!("{:?}: wanted {at} and {msg}, got: {text}", src));
    }
    // the source line and a caret under the column
    let (_, text) = script_run("let a = 1\nlet b = geom.box(1, 1, 0)\n", &[]);
    c.check(text.contains("test.jfs:2:14:") && text.contains("  2 | let b = geom.box(1, 1, 0)\n") && text.contains("    |              ^\n"), || format!("caret: {text}"));
    // parameters: numbers, true / false and text
    let (r, text) = script_run("let a = geom.box(params.get(\"b\", 9), params.h, 1)\nif params.flag { model.add(a, params.mat) }\n", &[("b", "2"), ("h", "0.5"), ("flag", "true"), ("mat", "stein")]);
    c.check(r.ok() && r.model.parts.len() == 1 && r.model.parts[0].mesh.bounds().unwrap().size() == Vec3::new(2.0, 0.5, 1.0), || format!("params: {text}"));
    // a module and its private names
    let (r, text) = script_run("let t = import(\"lib/teil.jfs\")\nmodel.add(t.wuerfel(2))\nprint(t._privat)\n", &[]);
    c.check(r.ok() && r.imports.len() == 1 && r.output.trim() == "nil", || format!("import: {text} output {:?}", r.output));
    // expectations and budget that fail are findings, not errors; each names the line of its rule, and the report's
    // rules carry their file and line
    let (r, _) = script_run("model.add(geom.box(1, 1, 1))\nmodel.expect_size(1, 2, 1)\nmodel.budget(4)\n", &[]);
    let a = run::analyse(&r.model);
    let f = run::findings(&r.model, &a);
    c.check(r.ok() && !a.ok() && a.within_budget == Some(false) && f.len() == 2, || format!("findings: {:?}", f));
    c.check(
        f.len() == 2 && f[0].starts_with("test.jfs:2: Maße 1 x 2 x 1 m erwartet (Toleranz 0.001), sind 1 x 1 x 1 m") && f[1] == "test.jfs:3: über dem Budget: 12 Dreiecke, erlaubt 4",
        || format!("findings with lines: {:?}", f),
    );
    let rep = crate::json::write(&run::report(&r, Some(&a), None, None));
    c.check(
        rep.contains("{\"rule\": \"size\", \"file\": \"test.jfs\", \"line\": 2, \"expected\": [1, 2, 1]")
            && rep.contains("{\"rule\": \"budget\", \"file\": \"test.jfs\", \"line\": 3, \"expected\": 4, \"actual\": 12, \"ok\": false}"),
        || format!("rules in the report: {rep}"),
    );
    // a rule in a module: the module's path and line
    let (r, text) = script_run("let r = import(\"lib/regel.jfs\")\nmodel.add(geom.box(1, 2, 1))\nr.regel()\n", &[]);
    let f = run::findings(&r.model, &run::analyse(&r.model));
    c.check(r.ok() && f.first().is_some_and(|s| s.starts_with("lib/regel.jfs:2: Maße beliebig x 1 x beliebig m erwartet")), || format!("rule in a module: {text} {:?}", f));
    // warnings (cx.warn): the script goes on; each with the file, line and column of the call (the column of the
    // called name, as errors have it), on the console like an error, in the report; a warning before an error stays
    let src = "model.name(\"A\")\nmodel.add(geom.box(1, 1, 1, 9))\nmodel.name(\"B\")\nmodel.budget(100)\nmodel.budget(200)\nmodel.uv_scale(2)\nmodel.uv_scale(0.5)\nmodel.material(\"m\", {color: \"#ff0000\"})\nmodel.material(\"m\", {color: \"#00ff00\"})\nmodel.add(geom.sweep(profile.rect(0.1, 0.1), [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], 1))\nmodel.add(geom.box(1, 1, 1).flat(1), \"m\")\nmodel.name(\"B\")\nmodel.material(\"m\", {color: \"#00ff00\"})\n";
    let (r, text) = script_run(src, &[]);
    let lines: Vec<u32> = r.warnings.iter().map(|w| w.line).collect();
    c.check(r.ok() && lines == [2, 3, 5, 7, 9, 10, 11] && r.warnings.iter().all(|w| w.file == "test.jfs"), || format!("warnings: {text} {:?}", r.warnings));
    let msgs: Vec<&str> = r.warnings.iter().map(|w| w.msg.as_str()).collect();
    let want = [
        "geom.box: 4 Argumente, nimmt aber höchstens 3 (w, h, d); das letzte bleibt unbeachtet",
        "model.name: \"B\" ersetzt den Namen \"A\" aus test.jfs:1",
        "model.budget: 200 ersetzt das Budget 100 aus test.jfs:4",
        "model.uv_scale: 0.5 ersetzt 2 aus test.jfs:6; alle Teile bekommen den letzten Wert",
        "model.material: das Material \"m\" ist schon in test.jfs:8 anders festgelegt; ab hier gilt das neue, Teile davor behalten das alte",
        "geom.sweep: closed sollte true oder false sein, nicht int 1 (gilt als true)",
        "flat(): 1 Argument, nimmt aber keine; es bleibt unbeachtet",
    ];
    c.check(msgs == want, || format!("warning texts: {:?}", msgs));
    let sources = vec![(String::from("test.jfs"), String::from(src))];
    let console: String = run::format_warnings(&r, &sources).concat();
    c.check(
        console.contains("test.jfs:3:7: Warnung: model.name: \"B\" ersetzt den Namen \"A\" aus test.jfs:1\n  3 | model.name(\"B\")\n    |       ^\n")
            && console.contains("test.jfs:2:16: Warnung: geom.box: 4 Argumente"),
        || format!("warnings on the console: {console}"),
    );
    let rep = crate::json::write(&run::report(&r, Some(&run::analyse(&r.model)), None, None));
    c.check(
        rep.contains("\"warnings\": [\n    {\"file\": \"test.jfs\", \"line\": 2, \"col\": 16, \"message\": \"geom.box: 4 Argumente")
            && rep.contains("{\"file\": \"test.jfs\", \"line\": 3, \"col\": 7, \"message\": \"model.name: \\\"B\\\" ersetzt den Namen \\\"A\\\" aus test.jfs:1\"}")
            && !rep.contains("warnings_dropped"),
        || format!("warnings in the report: {rep}"),
    );
    let (r, text) = script_run("model.name(\"A\")\nmodel.name(\"B\")\nlet b = geom.box(1, 0, 1)\n", &[]);
    c.check(!r.ok() && r.warnings.len() == 1 && r.warnings[0].line == 2 && json_has(&r, "\"line\": 2, \"col\": 7"), || format!("warning before an error: {text} {:?}", r.warnings));
    // more than script::MAX_WARNINGS: the rest counted, the console shows the first ones and how many more
    let (r, text) = script_run("model.add(geom.box(1, 1, 1))\nfor i in 0..1100 { model.name(i) }\n", &[]);
    let console = run::format_warnings(&r, &[]);
    c.check(
        r.ok() && r.warnings.len() == script::MAX_WARNINGS && r.warnings_dropped == 99 && console.len() == run::WARNINGS_SHOWN + 1
            && console.last().is_some_and(|l| l == "  ... und 1089 weitere Warnungen (990 im Bericht; 99 nicht aufgehoben, höchstens 1000 je Lauf)\n")
            && json_has(&r, "\"warnings_dropped\": 99"),
        || format!("many warnings: {text} {} kept, {} dropped, {:?}", r.warnings.len(), r.warnings_dropped, console.last()),
    );
    // the description (host::api) is whole (no unknown type, no name twice); its names are English (decision G9.3),
    // the German ones are gone; --api is made from it; only merge and the Boolean operations take any number of
    // arguments, every other function and method reads at most its parameters (the warning about extra ones)
    let api = crate::host::api();
    c.check(api.validate().is_empty(), || format!("api: {:?}", api.validate()));
    let text = crate::host::api_text();
    for want in [
        "geom.box(w: float, h: float, d: float): mesh",
        "geom.cone(r: float, h: float, segments: int = 32, r_top: float = 0): mesh",
        "geom.sphere(r: float, segments: int = 32, rings: int = segments / 2): mesh",
        "geom.sweep(profile: list, path: list, closed: bool = false, smooth: float = 30): mesh",
        "geom.difference(m: mesh, ...cuts: mesh): mesh",
        "m.bevel(width: float, segments: int = 1, axis: str = nil): mesh",
        "m.radial(count: int, step: float = 360 / count): mesh",
        "m.tris: int, nur lesbar",
        "profile.arc(r: float, start: float, end: float, n: int = 16): list",
        "model.budget(triangles: int): nil",
        "model.expect_size(x: float, y: float, z: float, tolerance: float = 0.001): nil",
        "vec(x: float, y: float, z: float): vec",
        "v.x: float, nur lesbar",
        "v.normalized(): vec",
        "float * vec: vec",
        "-vec: vec",
        "Materialien: grau, weiss",
    ] {
        c.check(text.contains(want), || format!("--api lacks {want}"));
    }
    let german = ["segmente", "r_oben", "ringe", "seiten", "stufen", "staerke", "profil", "tiefe", "grundriss", "pfad", "geschlossen", "glatt", "grad", "toleranz", "anzahl", "schritt", "breite", "hoehe", "achse", "dreiecke", "meter", "von", "bis", "abzug", "abzug1"];
    let mut sigs: Vec<(String, &script::api::Sig)> = api.globals.iter().filter_map(|g| match &g.kind {
        script::api::GlobalKind::Func(s) => Some((g.name.clone(), s)),
        _ => None,
    }).collect();
    for t in &api.types {
        sigs.extend(t.methods.iter().map(|m| (format!("{}.{}", t.name, m.name), &m.sig)));
    }
    c.check(sigs.len() == 65, || format!("{} functions and methods", sigs.len()));
    for (name, sig) in &sigs {
        let params = sig.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
        let old: Vec<&&str> = german.iter().filter(|g| sig.params.iter().any(|p| p.name == **g)).collect();
        c.check(old.is_empty(), || format!("--api: {}({}) has German names {:?}", name, params, old));
        let variadic = matches!(name.as_str(), "geom.merge" | "geom.union" | "geom.difference" | "geom.intersection");
        c.check(crate::host::max_args(sig).is_none() == variadic, || format!("max_args({}) = {:?}", name, crate::host::max_args(sig)));
    }
    // the language services with it (an editor, G7o): after "m." a mesh's members, after "v." a vec's, after "vec."
    // the namespace's functions although vec is a function too; the signature of vec(..)
    let lcx = script::ls::Context::new(&api);
    let labels = |src: &str| -> Vec<String> { script::ls::complete(src, src.len() as u32, &lcx).items.into_iter().map(|i| i.label).collect() };
    let m = labels("let m = geom.box(1, 1, 1)\nm.");
    let v = labels("let v = vec(1, 2, 3) * 2\nv.");
    let ns = labels("vec.");
    let has = |l: &[String], w: &[&str]| w.iter().all(|x| l.iter().any(|y| y == x));
    c.check(has(&m, &["tris", "move", "bevel", "bounds"]) && !m.iter().any(|x| x == "keys"), || format!("complete m.: {:?}", m));
    c.check(has(&v, &["x", "z", "length", "normalized"]) && !v.iter().any(|x| x == "tris"), || format!("complete v.: {:?}", v));
    c.check(has(&ns, &["add", "cross", "lerp"]) && ns.len() == 9, || format!("complete vec.: {:?}", ns));
    let sig = script::ls::signature("let v = vec(1, ", 15, &lcx).map(|s| s.label);
    c.check(sig.as_deref() == Some("vec(x: float, y: float, z: float): vec"), || format!("signature vec(: {:?}", sig));
    // queries (since S2 the points of bounds are vecs, as is the result of vec.* in 3D)
    let (r, text) = script_run(
        "let m = geom.box(2, 1, 1)\nlet b = m.bounds()\nassert(b.size.x == 2 && b.max.y == 1, \"bounds\")\nassert(m.area() == 10 && m.volume() == 2, \"area\")\nassert(m.check().closed && m.tris == 12, \"check\")\nassert(vec.len([3, 4]) == 5 && vec.cross([1, 0, 0], [0, 1, 0]).z == 1, \"vec\")\nassert(profile.area(profile.rect(2, 3)) == 6, \"profile\")\nmodel.add(m.place(0, 1, 0).center().weld().smooth(45).flat())\n",
        &[],
    );
    c.check(r.ok(), || format!("queries: {text}"));
    host_objects(c);
    // Boolean operations and bevels from a script: a wall with a window (a ring: Euler 0), the result turned and
    // cut again (its cells go along), a rounded block
    let (r, text) = script_run(
        "let w = geom.difference(geom.box(4, 2.6, 0.3), geom.box(1, 1, 1).move(0, 1, 0))\nmodel.add(w, \"ziegel\", \"wand\")\nlet w2 = geom.difference(w.rotate_y(90).move(3, 0, 0), geom.box(0.5, 0.5, 1).move(3, 1.2, 1.2))\nmodel.add(w2)\nmodel.add(geom.box(1, 1, 1).bevel(0.1, 3).move(0, 3, 0))\nmodel.add(geom.union([geom.box(1, 1, 1), geom.cylinder(0.3, 2, 16)]).move(-3, 0, 0))\nmodel.add(geom.intersection(geom.box(1, 1, 1), geom.sphere(0.6, 12).move(0.3, 0, 0)).move(0, 0, 3))\n",
        &[],
    );
    if c.check(r.ok(), || format!("booleans: {text}")) {
        let a = run::analyse(&r.model);
        c.check(a.ok(), || format!("booleans: {:?}", run::findings(&r.model, &a)));
        let wall = &a.parts[0];
        c.check(wall.euler == 0 && rel(wall.volume, 4.0 * 2.6 * 0.3 - 0.3, 1e-12), || format!("wall: euler {} volume {}", wall.euler, wall.volume));
        let turned = &a.parts[1];
        c.check(turned.euler == -2 && rel(turned.volume, 4.0 * 2.6 * 0.3 - 0.3 - 0.3 * 0.5 * 1.0, 1e-12), || format!("turned wall: euler {} volume {}", turned.euler, turned.volume));
        c.check(rel(a.parts[2].volume, refs::bevel_box_volume(1.0, 1.0, 1.0, 0.1, 3), 1e-12), || format!("bevel: volume {}", a.parts[2].volume));
    }
    // their messages: not convex, two parts, nothing left, a bevel too wide, a wrong axis
    let bad: [(&str, &str); 6] = [
        ("model.add(geom.union(geom.stairs(1, 1, 1, 3), geom.box(1, 1, 1)))\n", "geom.union: Teil 1: das Mesh ist nicht konvex"),
        ("let b = geom.box(1, 1, 1)\nmodel.add(geom.difference(geom.merge(b, b.move(3, 0, 0)), b))\n", "geom.difference: Teil 1: das Mesh besteht aus 2 Teilen"),
        ("model.add(geom.intersection(geom.box(1, 1, 1), geom.box(1, 1, 1).move(5, 0, 0)))\n", "geom.intersection: das Ergebnis ist leer"),
        ("model.add(geom.difference(geom.box(1, 1, 1)))\n", "geom.difference: braucht mindestens 2 Meshes"),
        ("model.add(geom.box(1, 1, 1).bevel(0.6, 2))\n", "bevel(): die Breite 0.6 ist zu groß"),
        ("model.add(geom.box(1, 1, 1).bevel(0.1, 2, \"w\"))\n", "bevel(): axis muss"),
    ];
    for (src, msg) in bad {
        let (r, text) = script_run(src, &[]);
        c.check(!r.ok() && text.contains("test.jfs:") && text.contains(msg), || format!("{:?}: wanted {msg}, got: {text}", src));
    }
    // a finding about the surface says where it lies first and in which part: two boxes joined that touch only along
    // an edge (four faces meet there)
    let (r, text) = script_run("model.add(geom.box(1, 1, 1))\nmodel.add(geom.union(geom.box(1, 1, 1), geom.box(1, 1, 1).move(1, 1, 0)).move(3, 0, 0), \"grau\", \"kante\")\n", &[]);
    if c.check(r.ok(), || format!("edge contact: {text}")) {
        let a = run::analyse(&r.model);
        let f = run::findings(&r.model, &a);
        c.check(f.first().is_some_and(|s| s.contains("Kanten mit mehr als zwei Dreiecken, zuerst bei (3.5, 1, ") && s.ends_with("in Teil \"kante\"")), || format!("located finding: {:?}", f));
    }
    // imports only from the script's folder and below
    for (dir, path, ok) in [
        ("", "teil.jfs", true),
        ("", "lib/teil.jfs", true),
        ("", "../teil.jfs", false),
        ("", "/etc/passwd", false),
        ("", "C:/Windows/win.ini", false),
        ("apps/x/", "apps/x/lib/teil.jfs", true),
        ("apps/x/", "apps/y.jfs", false),
        ("apps/x/", "apps/xy/teil.jfs", false),
        ("apps/x/", "apps/x", false),
        ("C:/m/", "C:/m/a.jfs", true),
        ("C:/m/", "c:/m/a.jfs", false),
        ("C:/m/", "C:/n/a.jfs", false),
    ] {
        let r = run::import_allowed(dir, path);
        c.check(r.is_ok() == ok, || format!("import {path} from {dir:?}: {:?}", r));
    }
    c.check(run::script_dir("apps/x/haus.jfs") == "apps/x/" && run::script_dir("haus.jfs").is_empty() && run::script_dir("./a/../b/c.jfs") == "b/", || "script_dir".into());
    for (src, msg) in [
        ("let t = import(\"../geheim.jfs\")\n", "Modul ../geheim.jfs: liegt außerhalb des Ordners des Skripts"),
        ("let t = import(\"lib/../../geheim.jfs\")\n", "liegt außerhalb des Ordners des Skripts"),
        ("let t = import(\"/geheim.jfs\")\n", "liegt außerhalb des Ordners des Skripts"),
    ] {
        let (r, text) = script_run(src, &[]);
        c.check(!r.ok() && text.contains(msg), || format!("{:?}: wanted {msg}, got: {text}", src));
    }
    // a script in a folder imports relative to it, and not beside the folder
    let mut seen = Vec::new();
    let mut loader = |p: &str| -> Result<String, String> {
        seen.push(String::from(p));
        Ok(String::from("fn f() { return 1 }\n"))
    };
    let r = run::run_script("sub/test.jfs", "let a = import(\"teil.jfs\")\nlet b = import(\"../daneben.jfs\")\n", &[], &mut loader);
    c.check(r.problems.first().is_some_and(|p| p.message.contains("außerhalb")) && seen == ["sub/teil.jfs"], || format!("sub folder: {:?} loaded {:?}", r.problems.first().map(|p| &p.message), seen));
    // the cache key: the same for CRLF and LF, another for another parameter
    let k1 = run::cache_key("a\nb\n", &[], &[]);
    c.check(k1 == run::cache_key("a\r\nb\r\n", &[], &[]) && k1 != run::cache_key("a\nb\n", &[(String::from("x"), String::from("1"))], &[]) && k1.len() == 32, || "cache key".into());
}

// ---------------------------------------------------------------- the observation mode

/// The core of --watch with real files in a temporary folder: built when the script or a module changed, the old
/// model kept while the script has an error, a module that goes missing, an import outside the folder.  (The
/// browser has no files: nothing to check there.)
#[cfg(target_arch = "wasm32")]
fn watch_tests(_: &mut Ctx) {}

#[cfg(not(target_arch = "wasm32"))]
fn watch_tests(c: &mut Ctx) {
    let tmp = plat::temp_dir().unwrap_or_else(|| String::from(".")).replace('\\', "/");
    let base = format!("{}/modeltool-watch-{}", tmp.trim_end_matches('/'), plat::random_u64() % 1_000_000_000);
    if !c.check(plat::mkdir(&format!("{base}/sub")), || format!("mkdir {base}")) {
        return;
    }
    let write = |name: &str, text: &str| plat::write_file(&format!("{base}/{name}"), text.as_bytes());
    let a = format!("{base}/a.jfs");
    write("a.jfs", "model.name(\"A\")\nmodel.add(geom.box(1, 1, 1))\n");
    let mut w = crate::watch::Watched::new(&a, &[]);
    c.check(w.changed(), || "before the first build: changed".into());
    w.build();
    let st = &w.state;
    c.check(st.ok == Some(true) && st.title == "A" && st.summary == "1 Teile, 12 Dreiecke, 1 x 1 x 1 m" && st.builds == 1, || format!("first build: {:?} {} {}", st.ok, st.title, st.summary));
    c.check(!w.changed(), || "nothing changed, but changed".into());
    c.check(st.warnings.is_empty() && st.warning_count == 0, || format!("first build: warnings {:?}", st.warnings));
    // saved with another size, and a second name: a warning with its place and line
    write("a.jfs", "model.name(\"A\")\nmodel.add(geom.box(1, 2, 1))\nmodel.name(\"A2\")\n");
    c.check(w.changed(), || "saved: not changed".into());
    w.build();
    c.check(w.state.summary == "1 Teile, 12 Dreiecke, 1 x 2 x 1 m", || format!("second build: {}", w.state.summary));
    let warned = format!("{a}:3:7: Warnung: model.name: \"A2\" ersetzt den Namen \"A\" aus {a}:1");
    let st = &w.state;
    c.check(st.warning_count == 1 && st.warnings.first().is_some_and(|l| l[0] == warned && l[1] == "  3 | model.name(\"A2\")"), || format!("warning: {:?}", st.warnings));
    // an error: its place, the old model stays (the warnings are the new run's: none)
    write("a.jfs", "model.name(\"A\")\nmodel.add(geom.box(1, 0, 1))\n");
    w.build();
    let st = &w.state;
    let place = format!("{a}:2:");
    c.check(st.ok.is_none() && st.errors.first().is_some_and(|e| e[0].starts_with(&place) && e[0].contains("die Höhe")) && st.model.is_some() && st.warning_count == 0, || format!("error: {:?} {:?}", st.errors, st.warnings));
    c.check(!w.changed(), || "after the failed build: changed".into());
    // a module: watched too
    write("m.jfs", "fn teil() { return geom.box(2, 1, 1) }\n");
    write("a.jfs", "let m = import(\"m.jfs\")\nmodel.add(m.teil())\n");
    c.check(w.changed(), || "script with a module: not changed".into());
    w.build();
    c.check(w.state.ok == Some(true) && w.state.summary.ends_with("2 x 1 x 1 m") && w.files().iter().any(|f| f.ends_with("/m.jfs")), || format!("module: {:?} {} {:?}", w.state.errors, w.state.summary, w.files()));
    write("m.jfs", "fn teil() { return geom.box(3, 1, 1) }\n");
    c.check(w.changed(), || "module saved: not changed".into());
    w.build();
    c.check(w.state.summary.ends_with("3 x 1 x 1 m"), || format!("module saved: {}", w.state.summary));
    plat::delete(&format!("{base}/m.jfs"));
    c.check(w.changed(), || "module deleted: not changed".into());
    w.build();
    c.check(w.state.errors.first().is_some_and(|e| e[0].contains("m.jfs: die Datei gibt es nicht")), || format!("module deleted: {:?}", w.state.errors));
    // a script in a folder below may not import from the one above
    write("sub/b.jfs", "let a = import(\"../a.jfs\")\nmodel.add(geom.box(1, 1, 1))\n");
    let mut wb = crate::watch::Watched::new(&format!("{base}/sub/b.jfs"), &[]);
    wb.build();
    c.check(wb.state.errors.first().is_some_and(|e| e[0].contains("liegt außerhalb des Ordners des Skripts")), || format!("outside: {:?}", wb.state.errors));
    // a script that is gone
    let mut wg = crate::watch::Watched::new(&format!("{base}/fehlt.jfs"), &[]);
    wg.build();
    c.check(wg.state.errors.first().is_some_and(|e| e[0].contains("lässt sich nicht lesen")) && !wg.changed(), || format!("missing: {:?}", wg.state.errors));
    for f in ["a.jfs", "sub/b.jfs", "sub", ""] {
        plat::delete(&format!("{base}/{f}").trim_end_matches('/').to_string());
    }
}

// ---------------------------------------------------------------- examples

fn examples_tests(c: &mut Ctx) {
    for ex in examples::ALL {
        let mut loader = examples::loader;
        let file = format!("{}.jfs", ex.name);
        let r = run::run_script(&file, ex.source, &[], &mut loader);
        let sources = vec![(file.clone(), String::from(ex.source))];
        if !c.check(r.ok(), || format!("{}: {}", ex.name, r.problems.iter().map(|p| run::format_problem(p, &sources)).collect::<String>())) {
            continue;
        }
        let a = run::analyse(&r.model);
        c.check(a.ok(), || format!("{}: {:?}", ex.name, run::findings(&r.model, &a)));
        let now = run::stable_lines(&crate::json::write(&run::report(&r, Some(&a), None, None)));
        let want = run::stable_lines(ex.report);
        if !c.check(!want.is_empty(), || format!("{}: no report in examples/out (modeltool --examples apps/modeltool/examples/out)", ex.name)) {
            continue;
        }
        let first = now.iter().zip(&want).position(|(a, b)| a != b);
        c.check(now == want, || match first {
            Some(i) => format!("{}: report line {} is {:?}, examples/out has {:?}", ex.name, i + 1, now[i], want[i]),
            None => format!("{}: {} lines, examples/out has {}", ex.name, now.len(), want.len()),
        });
    }
}
