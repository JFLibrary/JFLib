// dynamics -- the rigid bodies of lib/physics against what they must do: mass properties against closed forms
// and integration by slices, free motion against the exact sums of the integrator, contacts against the
// laws of friction and restitution, stacks that stand and fall asleep, joints against pendulum periods and
// their limits, sweeps against thin walls, events, and random scenes that must not explode.
//
// Groups: mass motion contact stack joint ccd events dynfuzz; the third checksum (checksum) and the measurement
// (bench) run on `showcase`, a scene with a bit of everything.

use crate::scene;
use crate::tests::random_convex;
use crate::{Ctx, Rng};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use physics::math::{self, sqrt};
use physics::{
    BodyId, BodyKind, ContactEvent, ConvexHull, Iso, JointError, JointId, JointKind, MassProps, Mat3, Motor, Quat, RigidBody, Shape, SimParams, Simulation, Spring,
    Vec3,
};

const DT: f64 = 1.0 / 60.0;
const PI: f64 = core::f64::consts::PI;

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-300)
}

fn mat_rel(a: &Mat3, b: &Mat3) -> f64 {
    if !a.is_finite() || !b.is_finite() {
        return f64::INFINITY;
    }
    let mut m = 0.0f64;
    let s = b.trace().abs().max(1e-300);
    for r in 0..3 {
        for c in 0..3 {
            m = m.max((a.at(r, c) - b.at(r, c)).abs() / s);
        }
    }
    m
}

/// a static floor: a box whose top is at y = 0
fn floor(sim: &mut Simulation, half: f64) -> BodyId {
    sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(half, 1.0, half)), Iso::at(Vec3::new(0.0, -1.0, 0.0)))).expect("floor")
}

fn run(sim: &mut Simulation, steps: usize) {
    for _ in 0..steps {
        sim.step(DT);
    }
}

fn no_gravity() -> SimParams {
    SimParams { gravity: Vec3::ZERO, ..SimParams::default() }
}

// ---------------------------------------------------------------- mass

/// mass, centre and inertia of a solid of revolution about Y with radius r(y) on [y0, y1], by Simpson's rule
/// over n slices (discs: mass pi r^2 dy, Iyy pi r^4 / 2 dy, Ixx pi r^4 / 4 dy + y^2 dm)
fn revolution(r: impl Fn(f64) -> f64, y0: f64, y1: f64, n: usize, density: f64) -> (f64, f64, f64) {
    let h = (y1 - y0) / n as f64;
    let (mut m, mut iy, mut ix) = (0.0, 0.0, 0.0);
    for k in 0..=n {
        let y = y0 + h * k as f64;
        let w = if k == 0 || k == n { 1.0 } else if k % 2 == 1 { 4.0 } else { 2.0 };
        let rr = r(y);
        let a = PI * rr * rr;
        m += w * a;
        iy += w * a * rr * rr * 0.5;
        ix += w * (a * rr * rr * 0.25 + a * y * y);
    }
    let s = density * h / 3.0;
    (m * s, ix * s, iy * s)
}

/// the inertia of a tetrahedron about its centroid by Tonon's formulas (J. Math. Stat. 1(1), 2004), coordinates
/// relative to the centroid
fn tonon(p: [Vec3; 4], density: f64) -> (f64, Vec3, Mat3) {
    let c = (p[0] + p[1] + p[2] + p[3]) * 0.25;
    let q: Vec<Vec3> = p.iter().map(|v| *v - c).collect();
    let det = (q[1] - q[0]).dot((q[2] - q[0]).cross(q[3] - q[0])).abs();
    let mass = density * det / 6.0;
    let sq = |f: &dyn Fn(Vec3) -> f64| {
        let mut s = 0.0;
        for i in 0..4 {
            for j in i..4 {
                s += f(q[i]) * f(q[j]);
            }
        }
        s
    };
    let cross = |f: &dyn Fn(Vec3) -> f64, g: &dyn Fn(Vec3) -> f64| {
        let mut s = 0.0;
        for i in 0..4 {
            for j in 0..4 {
                s += f(q[i]) * g(q[j]) * if i == j { 2.0 } else { 1.0 };
            }
        }
        s
    };
    let (x, y, z) = (&|v: Vec3| v.x, &|v: Vec3| v.y, &|v: Vec3| v.z);
    let k = density * det;
    let a = k * (sq(y) + sq(z)) / 60.0;
    let b = k * (sq(x) + sq(z)) / 60.0;
    let cc = k * (sq(x) + sq(y)) / 60.0;
    let a2 = k * cross(y, z) / 120.0; // the product of inertia yz
    let b2 = k * cross(x, z) / 120.0; // xz
    let c2 = k * cross(x, y) / 120.0; // xy
    let i = Mat3::from_cols(Vec3::new(a, -c2, -b2), Vec3::new(-c2, b, -a2), Vec3::new(-b2, -a2, cc));
    (mass, c, i)
}

pub fn mass_tests(c: &mut Ctx) {
    let rho = 750.0;
    // sphere and box in closed form
    let m = Shape::sphere(0.4).mass_properties(rho).expect("sphere");
    let ms = rho * 4.0 / 3.0 * PI * 0.064;
    c.check(rel(m.mass, ms) < 1e-14 && mat_rel(&m.inertia, &Mat3::diagonal(Vec3::splat(0.4 * ms * 0.16))) < 1e-14, || format!("sphere {:?}", m));
    let h = Vec3::new(0.5, 1.0, 1.5);
    let m = Shape::cuboid(h).mass_properties(rho).expect("box");
    let mb = rho * 8.0 * 0.75;
    let ib = Mat3::diagonal(Vec3::new(1.0 + 2.25, 0.25 + 2.25, 0.25 + 1.0) * (mb / 3.0));
    c.check(rel(m.mass, mb) < 1e-14 && mat_rel(&m.inertia, &ib) < 1e-14, || format!("box {:?}", m));
    // capsules against slices
    for &(hh, r) in &[(0.5, 0.3), (0.05, 0.6), (2.0, 0.1), (1e-3, 1.0)] {
        let m = Shape::capsule(hh, r).mass_properties(rho).expect("capsule");
        let rad = |y: f64| {
            let e = if y.abs() <= hh { 0.0 } else { y.abs() - hh };
            sqrt((r * r - e * e).max(0.0))
        };
        // the slices end exactly at the caps' poles; the cap ends are steep, so many slices
        let (rm, rix, riy) = revolution(rad, -hh - r, hh + r, 400_000, rho);
        let ok = rel(m.mass, rm) < 1e-6 && rel(m.inertia.at(0, 0), rix) < 1e-6 && rel(m.inertia.at(1, 1), riy) < 1e-6 && rel(m.inertia.at(2, 2), rix) < 1e-6;
        c.check(ok, || format!("capsule {} {}: {:?} slices {} {} {}", hh, r, m, rm, rix, riy));
    }
    // a hull of a box's corners is that box; turned and moved, its properties turn and move with it
    let mut rng = Rng::new(31);
    for _ in 0..c.n(200) {
        let h = Vec3::new(rng.range(0.05, 2.0), rng.range(0.05, 2.0), rng.range(0.05, 2.0));
        let q = rng.quat();
        let t = rng.vec(-50.0, 50.0);
        let iso = Iso::new(t, q);
        let pts: Vec<Vec3> = (0..8)
            .map(|k| iso.transform_point(Vec3::new(if k & 1 != 0 { h.x } else { -h.x }, if k & 2 != 0 { h.y } else { -h.y }, if k & 4 != 0 { h.z } else { -h.z })))
            .collect();
        let hm = Shape::hull_from_points(&pts).expect("hull").mass_properties(rho).expect("hull mass");
        let bm = Shape::cuboid(h).mass_properties(rho).expect("box").transformed(&iso);
        let ok = rel(hm.mass, bm.mass) < 1e-12 && (hm.center - bm.center).length() < 1e-11 * (1.0 + t.length()) && mat_rel(&hm.inertia, &bm.inertia) < 1e-9;
        c.check(ok, || format!("hull of a box {:?}: {:?} against {:?}", h, hm, bm));
        c.check(hm.is_valid(), || format!("hull mass not valid {:?}", hm));
    }
    // tetrahedra against Tonon's formulas
    for _ in 0..c.n(500) {
        let p = [rng.vec(-1.0, 1.0), rng.vec(-1.0, 1.0), rng.vec(-1.0, 1.0), rng.vec(-1.0, 1.0)];
        let vol = (p[1] - p[0]).dot((p[2] - p[0]).cross(p[3] - p[0])).abs() / 6.0;
        if vol < 1e-3 {
            continue;
        }
        let off = rng.vec(-100.0, 100.0);
        let pts: Vec<Vec3> = p.iter().map(|v| *v + off).collect();
        // four points with a volume: the hull must take all of them
        let Ok(hull) = ConvexHull::new(&pts) else {
            c.check(false, || format!("the hull refused the tetrahedron {:?}", pts));
            continue;
        };
        if !c.check(hull.points().len() == 4, || format!("the hull of the tetrahedron {:?} has {} corners", pts, hull.points().len())) {
            continue;
        }
        let m = Shape::hull(hull).mass_properties(rho).expect("tetra");
        let (tm, tc, ti) = tonon([pts[0], pts[1], pts[2], pts[3]], rho);
        let ok = rel(m.mass, tm) < 1e-9 && (m.center - tc).length() < 1e-10 * 100.0 && mat_rel(&m.inertia, &ti) < 1e-8;
        c.check(ok, || format!("tetrahedron {:?}: {:?} against {} {:?} {:?}", p, m, tm, tc, ti));
    }
    // a fine polyhedron of a sphere is nearly a sphere (within its faceting)
    let mut pts = Vec::new();
    for i in 0..40 {
        for j in 0..80 {
            let th = PI * (i as f64 + 0.5) / 40.0;
            let ph = 2.0 * PI * j as f64 / 80.0;
            let (st, ct) = math::sincos(th);
            let (sp, cp) = math::sincos(ph);
            pts.push(Vec3::new(st * cp, ct, st * sp) * 0.7);
        }
    }
    let m = Shape::hull_from_points(&pts).expect("sphere hull").mass_properties(rho).expect("mass");
    let ms = rho * 4.0 / 3.0 * PI * 0.343;
    c.check(rel(m.mass, ms) < 0.01 && rel(m.inertia.at(0, 0), 0.4 * ms * 0.49) < 0.02 && m.center.length() < 1e-9, || format!("polyhedral sphere {:?} ({})", m, ms));
    // the parallel axis theorem and what is refused
    let m = Shape::cuboid(Vec3::new(0.3, 0.2, 0.1)).mass_properties(rho).expect("box");
    let about = m.inertia_about(Vec3::new(1.0, 0.0, 0.0));
    c.check(rel(about.at(1, 1), m.inertia.at(1, 1) + m.mass) < 1e-14 && rel(about.at(0, 0), m.inertia.at(0, 0)) < 1e-14, || format!("parallel axes {:?}", about));
    c.check(Shape::sphere(1.0).mass_properties(0.0).is_none() && Shape::sphere(1.0).mass_properties(f64::NAN).is_none(), || "a density of 0 or NaN gives a mass".into());
    let mesh = scene::flat(2, 1.0, 0.0).build();
    c.check(Shape::mesh(mesh).mass_properties(rho).is_none(), || "a mesh gave a mass".into());
    c.check(!MassProps::new(1.0, Vec3::ZERO, Mat3::diagonal(Vec3::new(1.0, 1.0, 3.0))).is_valid(), || "a tensor against the triangle inequality is valid".into());
    c.check(MassProps::point(2.0).is_valid(), || "a point mass is not valid".into());
    // a tensor whose diagonal looks fine but whose principal moments do not (-1, 3, 1)
    let bad = Mat3::from_cols(Vec3::new(1.0, 2.0, 0.0), Vec3::new(2.0, 1.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
    c.check(!MassProps::new(1.0, Vec3::ZERO, bad).is_valid(), || "a tensor with a negative principal moment is valid".into());
    let mut sim = Simulation::new();
    c.check(sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::IDENTITY).with_mass(MassProps::new(1.0, Vec3::ZERO, bad))).err() == Some(physics::BodyError::Mass), || "a body with a negative principal moment was accepted".into());
    // a turned box's tensor is valid in any frame
    let turned = Shape::cuboid(Vec3::new(0.1, 0.5, 2.0)).mass_properties(rho).expect("box").transformed(&Iso::new(Vec3::ZERO, Quat::from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize_or_zero(), 0.9)));
    c.check(turned.is_valid(), || format!("a turned box is not valid: {:?}", turned));
    // of any size: one near the top of the range is valid (its products would overflow), the bad one near the
    // bottom is not (they would underflow)
    let p2 = |k: i64| f64::from_bits(((k + 1023) as u64) << 52);
    let huge = Mat3::from_cols(Vec3::new(2.0, 1.0, 0.0), Vec3::new(1.0, 2.0, 0.0), Vec3::new(0.0, 0.0, 2.0)).scale(p2(600));
    c.check(MassProps::new(1.0, Vec3::ZERO, huge).is_valid(), || "a valid tensor of 2^600 is not valid".into());
    c.check(!MassProps::new(1.0, Vec3::ZERO, bad.scale(p2(-600))).is_valid(), || "the tensor with a negative principal moment is valid at 2^-600".into());
    // the moments 1 + 1e-5, 1 - 1e-5 and 0 break the triangle inequality by 1e-5 of their sum, beyond the 1e-9
    // allowed, in any frame
    let near = Mat3::from_cols(Vec3::new(1.0, 1e-5, 0.0), Vec3::new(1e-5, 1.0, 0.0), Vec3::ZERO);
    c.check(!MassProps::new(1.0, Vec3::ZERO, near).is_valid(), || "a tensor 1e-5 off the triangle inequality is valid".into());
    // a rod's and a plate's tensor (tr(I)/2 - I has an eigenvalue 0) turned anyhow stay valid, of any size;
    // the plate's moment over the sum of the others by 1e-7 does not
    let mut rng = Rng::new(43);
    let (mut lost, mut kept) = (0usize, 0usize);
    for _ in 0..100 {
        let q = rng.quat();
        for (s, diag) in [(1.0, Vec3::new(1.0, 0.0, 1.0)), (1.0, Vec3::new(1.0, 2.0, 3.0)), (p2(-900), Vec3::new(0.0, 3.0, 3.0)), (p2(900), Vec3::new(2.0, 1.0, 3.0))] {
            lost += usize::from(!MassProps::new(1.0, Vec3::ZERO, Mat3::diagonal(diag * s).rotated(q)).is_valid());
        }
        kept += usize::from(MassProps::new(1.0, Vec3::ZERO, Mat3::diagonal(Vec3::new(1.0, 2.0, 3.0 * (1.0 + 1e-7))).rotated(q)).is_valid());
    }
    c.check(lost == 0 && kept == 0, || format!("turned rods and plates: {} of 400 not valid, {} of 100 just off valid", lost, kept));
    c.check(Shape::cuboid(Vec3::new(-1.0, -1.0, 1.0)).mass_properties(rho).is_none() && Shape::sphere(-0.5).mass_properties(rho).is_none() && Shape::capsule(0.5, -0.1).mass_properties(rho).is_none(), || "negative sizes gave a mass".into());
    c.check(Shape::capsule(0.0, 0.3).mass_properties(rho).is_some(), || "a capsule of half height 0 (a ball) gave no mass".into());
    let tiny = Mat3::diagonal(Vec3::splat(1e-105)).inverse();
    c.check(tiny.is_some_and(|m| rel(m.at(0, 0), 1e105) < 1e-12 && m.is_finite()), || format!("the inverse of a tiny diagonal: {:?}", tiny));
    // entries of very different sizes: diag(2^-300, 2^-300, 2^300), and an ordinary matrix with its first
    // two rows times 2^-600 (a determinant of 2^-1200 with one scale for all)
    let skew = Mat3::diagonal(Vec3::new(p2(-300), p2(-300), p2(300))).inverse();
    c.check(skew.is_some_and(|m| m.at(0, 0) == p2(300) && m.at(1, 1) == p2(300) && m.at(2, 2) == p2(-300) && m.at(0, 1) == 0.0), || format!("the inverse of diag(2^-300, 2^-300, 2^300): {:?}", skew));
    let plain = Mat3::from_cols(Vec3::new(2.0, 1.0, 0.5), Vec3::new(1.0, 3.0, 1.0), Vec3::new(0.5, 1.0, 4.0));
    let want = plain.inverse().expect("plain").mul(&Mat3::diagonal(Vec3::new(p2(600), p2(600), 1.0)));
    let got = Mat3::diagonal(Vec3::new(p2(-600), p2(-600), 1.0)).mul(&plain).inverse();
    c.check(got.is_some_and(|m| (0..3).all(|r| (0..3).all(|k| rel(m.at(r, k), want.at(r, k)) < 1e-12))), || format!("the inverse of rows 2^-600 apart: {:?}", got));
}

// ---------------------------------------------------------------- motion

pub fn motion_tests(c: &mut Ctx) {
    let p = SimParams::default();
    let n = p.substeps as f64;
    let h = DT / n;
    // free fall: v_k = k g h, y_k = y0 + g h^2 k (k + 1) / 2 after k substeps
    let mut sim = Simulation::new();
    let v0 = Vec3::new(3.0, 5.0, -2.0);
    let b = sim.add(RigidBody::dynamic(Shape::sphere(0.25), Iso::at(Vec3::new(1.0, 100.0, 2.0))).with_velocity(v0).sleepless()).expect("ball");
    for step in 1..=c.n(600) {
        sim.step(DT);
        if step % 50 == 0 || step == 1 {
            let k = step as f64 * n;
            let g = p.gravity.y;
            let s = sim.state(b).expect("state");
            let want_y = 100.0 + v0.y * h * k + g * h * h * k * (k + 1.0) * 0.5;
            let want_x = 1.0 + v0.x * h * k;
            let ok = (s.center.y - want_y).abs() < 1e-9 * (1.0 + want_y.abs()) && (s.center.x - want_x).abs() < 1e-11 * (1.0 + want_x.abs()) && (s.linear_velocity.y - (v0.y + g * h * k)).abs() < 1e-10;
            c.check(ok, || format!("free fall step {}: {:?} want y {} x {}", step, s, want_y, want_x));
        }
    }
    // a constant spin is exact: the angle is the rate times the time
    let mut sim = Simulation::with_params(no_gravity());
    let axis = Vec3::new(1.0, 2.0, -0.5).normalize_or_zero();
    let rate = 7.3;
    let b = sim.add(RigidBody::dynamic(Shape::sphere(0.5), Iso::IDENTITY).with_angular_velocity(axis * rate).with_damping(0.0, 0.0).sleepless()).expect("spin");
    run(&mut sim, 120);
    let q = sim.iso(b).expect("iso").rot;
    let want = Quat::from_axis_angle(axis, rate * 2.0);
    c.check(q.dot(want).abs() > 1.0 - 1e-12, || format!("spin {:?} want {:?}", q, want));
    // damping: v (1 / (1 + h c))^k
    let mut sim = Simulation::with_params(no_gravity());
    let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::IDENTITY).with_velocity(Vec3::new(10.0, 0.0, 0.0)).with_damping(0.7, 0.0).sleepless()).expect("damped");
    run(&mut sim, 60);
    let want = (0..240).fold(10.0, |v, _| v * (1.0 / (1.0 + h * 0.7)));
    let v = sim.velocity(b).expect("v").0.x;
    c.check(rel(v, want) < 1e-12, || format!("damping {} want {}", v, want));
    // forces, impulses, torques
    let mut sim = Simulation::with_params(no_gravity());
    let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 1.0)), Iso::IDENTITY).with_damping(0.0, 0.0)).expect("box");
    let mp = sim.mass(b).expect("mass");
    sim.apply_force(b, Vec3::new(0.0, 0.0, 120.0), Vec3::ZERO);
    sim.step(DT);
    let (v, w) = sim.velocity(b).expect("v");
    c.check(rel(v.z, 120.0 / mp.mass * DT) < 1e-12 && w.length() == 0.0, || format!("force: {:?} {:?}", v, w));
    sim.step(DT);
    c.check(rel(sim.velocity(b).expect("v").0.z, 120.0 / mp.mass * DT) < 1e-12, || "a force acted for more than its step".into());
    let mut sim = Simulation::with_params(no_gravity());
    let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 1.0)), Iso::IDENTITY).with_damping(0.0, 0.0)).expect("box");
    sim.apply_impulse(b, Vec3::new(0.0, 6.0, 0.0), Vec3::new(0.5, 0.0, 0.0));
    let (v, w) = sim.velocity(b).expect("v");
    let iz = mp.inertia.at(2, 2);
    c.check(rel(v.y, 6.0 / mp.mass) < 1e-14 && rel(w.z, 3.0 / iz) < 1e-12, || format!("impulse: {:?} {:?}", v, w));
    // a kinematic body reaches its target exactly and pushes nothing back
    let mut sim = Simulation::new();
    let k = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::splat(0.5)), Iso::IDENTITY)).expect("kinematic");
    let target = Iso::new(Vec3::new(0.3, 0.1, -0.2), Quat::from_axis_angle(Vec3::Y, 0.2));
    sim.move_kinematic(k, target, DT);
    sim.step(DT);
    let at = sim.iso(k).expect("iso");
    c.check(at.pos == target.pos && at.rot.dot(target.rot).abs() > 1.0 - 1e-15, || format!("kinematic at {:?} want {:?}", at, target));
    let (v, w) = sim.velocity(k).expect("v");
    c.check((v - Vec3::new(0.3, 0.1, -0.2) / DT).length() < 1e-9 && (w.y - 0.2 / DT).abs() < 1e-9, || format!("kinematic velocity {:?} {:?}", v, w));
    // a kinematic body that never turns stays so; a move over an infinite time is refused
    let mut sim = Simulation::new();
    let k = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::splat(0.5)), Iso::IDENTITY).fixed_rotation()).expect("kinematic");
    sim.set_velocity(k, Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 3.0, 0.0));
    sim.step(DT);
    sim.move_kinematic(k, Iso::new(Vec3::new(1.0, 0.0, 0.0), Quat::from_axis_angle(Vec3::Y, 1.0)), DT);
    sim.step(DT);
    let at = sim.iso(k).expect("iso");
    c.check(at.rot.is_identity() && at.pos == Vec3::new(1.0, 0.0, 0.0), || format!("a kinematic body with fixed rotation turned or missed: {:?}", at));
    c.check(!sim.move_kinematic(k, Iso::IDENTITY, f64::INFINITY), || "a move over an infinite time was taken".into());
    // gravity scale 0 floats
    let mut sim = Simulation::new();
    let b = sim.add(RigidBody::dynamic(Shape::sphere(0.5), Iso::at(Vec3::new(0.0, 5.0, 0.0))).with_gravity_scale(0.0)).expect("float");
    run(&mut sim, 60);
    c.check(sim.iso(b).expect("iso").pos.y == 5.0, || "gravity scale 0 fell".into());
    // a tumbling box keeps its angular momentum with gyroscopic forces and flips (Dzhanibekov), without them
    // its angular velocity stays
    for gyro in [true, false] {
        let mut sim = Simulation::with_params(no_gravity());
        let mut desc = RigidBody::dynamic(Shape::cuboid(Vec3::new(0.1, 0.3, 0.6)), Iso::IDENTITY).with_angular_velocity(Vec3::new(0.01, 6.0, 0.01)).with_damping(0.0, 0.0).sleepless();
        if gyro {
            desc = desc.gyroscopic();
        }
        let b = sim.add(desc).expect("tumbler");
        let mp = sim.mass(b).expect("mass");
        let momentum = |sim: &Simulation| {
            let s = sim.state(b).expect("state");
            mp.inertia.rotated(s.iso.rot).mul_vec(s.angular_velocity)
        };
        let l0 = momentum(&sim);
        let mut flipped = false;
        let mut worst = 0.0f64;
        for _ in 0..c.n(1200) {
            sim.step(DT);
            let s = sim.state(b).expect("state");
            if s.iso.rot.rotate(Vec3::Y).dot(Vec3::Y) < -0.9 {
                flipped = true;
            }
            worst = worst.max((momentum(&sim) - l0).length() / l0.length());
        }
        if gyro {
            c.check(worst < 0.02, || format!("gyroscopic: angular momentum off by {}", worst));
            c.check(flipped || c.quick, || "gyroscopic: the box did not flip".into());
        } else {
            let w = sim.velocity(b).expect("v").1;
            c.check((w - Vec3::new(0.01, 6.0, 0.01)).length() < 1e-12, || format!("without gyroscopic forces the spin changed: {:?}", w));
        }
    }
}

// ---------------------------------------------------------------- contact

pub fn contact_tests(c: &mut Ctx) {
    // a ball comes to rest on the floor, just on it, and falls asleep
    for shape in [Shape::sphere(0.3), Shape::cuboid(Vec3::new(0.4, 0.3, 0.2)), Shape::capsule(0.3, 0.2)] {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let b = sim.add(RigidBody::dynamic(shape.clone(), Iso::at(Vec3::new(0.0, 2.0, 0.0)))).expect("body");
        let mut asleep_at = None;
        for step in 0..600 {
            sim.step(DT);
            if asleep_at.is_none() && !sim.is_awake(b) {
                asleep_at = Some(step);
            }
        }
        let s = sim.state(b).expect("state");
        let bottom = shape.aabb(&s.iso).min.y;
        c.check(bottom > -0.006 && bottom < 0.002, || format!("{:?} rests at {} ({:?})", shape, bottom, s));
        c.check(asleep_at.is_some_and(|t| t < 300), || format!("{:?} asleep at step {:?}", shape, asleep_at));
    }
    // restitution: a bounce comes back to e^2 of the height
    for &(e, h0, tol) in &[(0.5, 2.0, 0.05), (0.8, 3.0, 0.05), (1.0, 2.0, 0.04)] {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let r = 0.2;
        let b = sim.add(RigidBody::dynamic(Shape::sphere(r), Iso::at(Vec3::new(0.0, h0 + r, 0.0))).with_restitution(e)).expect("ball");
        // down to the floor and up again
        let mut low = f64::INFINITY;
        let mut went_up = false;
        let mut top = f64::NEG_INFINITY;
        for _ in 0..240 {
            sim.step(DT);
            let s = sim.state(b).expect("state");
            low = low.min(s.center.y);
            if s.linear_velocity.y > 0.0 {
                went_up = true;
            }
            if went_up {
                if s.linear_velocity.y < 0.0 {
                    break;
                }
                top = top.max(s.center.y);
            }
        }
        let rise = top - r;
        c.check(rel(rise, e * e * h0) < tol, || format!("restitution {}: up to {} want {} (lowest {})", e, rise, e * e * h0, low));
    }
    // friction on a slope of 20 degrees: friction 0.6 holds a box (tan 20 = 0.364), 0.2 lets it slide with
    // g (sin - mu cos)
    let th = 20.0f64.to_radians();
    let (st, ct) = math::sincos(th);
    for &mu in &[0.6, 0.2, 0.0] {
        let mut sim = Simulation::new();
        let slope = Quat::from_axis_angle(Vec3::Z, th);
        let down = slope.rotate(Vec3::new(-1.0, 0.0, 0.0));
        let normal = slope.rotate(Vec3::Y);
        sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(30.0, 1.0, 5.0)), Iso::new(normal * -1.0, slope)).with_friction(mu)).expect("slope");
        let start = normal * 0.25 + down * -20.0;
        let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 0.5)), Iso::new(start, slope)).with_friction(mu)).expect("box");
        run(&mut sim, 30); // settle
        let p0 = sim.state(b).expect("state").center;
        let t = 1.5;
        run(&mut sim, (t / DT) as usize);
        let d = (sim.state(b).expect("state").center - p0).dot(down);
        if mu > st / ct {
            c.check(d.abs() < 0.005, || format!("friction {} slid {} m", mu, d));
        } else {
            let v0 = 0.0f64.max(0.0);
            let a = 9.81 * (st - mu * ct);
            // it was already sliding for the settle steps: d = v0 t + a t^2 / 2 with v0 = a * 0.5 s
            let vs = a * 30.0 * DT;
            let want = (vs + v0) * t + 0.5 * a * t * t;
            c.check(rel(d, want) < 0.03, || format!("friction {} slid {} m want {}", mu, d, want));
        }
    }
    // a ball rolls down the slope with 5/7 g sin (no slipping), without angular damping
    {
        let mut sim = Simulation::new();
        let slope = Quat::from_axis_angle(Vec3::Z, th);
        let down = slope.rotate(Vec3::new(-1.0, 0.0, 0.0));
        let normal = slope.rotate(Vec3::Y);
        sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(30.0, 1.0, 5.0)), Iso::new(normal * -1.0, slope)).with_friction(1.0)).expect("slope");
        let r = 0.3;
        let b = sim.add(RigidBody::dynamic(Shape::sphere(r), Iso::at(normal * r + down * -20.0)).with_friction(1.0).with_damping(0.0, 0.0)).expect("ball");
        run(&mut sim, 30);
        let (p0, v0) = (sim.state(b).expect("s").center, sim.state(b).expect("s").linear_velocity.dot(down));
        let t = 1.5;
        run(&mut sim, (t / DT) as usize);
        let d = (sim.state(b).expect("state").center - p0).dot(down);
        let a = 5.0 / 7.0 * 9.81 * st;
        let want = v0 * t + 0.5 * a * t * t;
        c.check(rel(d, want) < 0.02, || format!("rolling ball went {} want {}", d, want));
    }
    // rolling resistance stops a rolling ball: its torque c m g r brakes it like a slope of c (for a ball the
    // deceleration is 5/7 c g); without it the ball rolls on
    for &cr in &[0.0, 0.05] {
        let mut sim = Simulation::new();
        floor(&mut sim, 200.0);
        let r = 0.25;
        let b = sim.add(RigidBody::dynamic(Shape::sphere(r), Iso::at(Vec3::new(0.0, r, 0.0))).with_velocity(Vec3::new(2.0, 0.0, 0.0)).with_angular_velocity(Vec3::new(0.0, 0.0, -2.0 / r)).with_damping(0.0, 0.0).with_rolling_resistance(cr)).expect("ball");
        run(&mut sim, 60);
        let v = sim.velocity(b).expect("v").0.x;
        if cr == 0.0 {
            c.check((v - 2.0).abs() < 5e-3, || format!("a ball without rolling resistance slowed to {}", v));
        } else {
            let want = 2.0 - 5.0 / 7.0 * cr * 9.81 * 1.0;
            c.check((v - want).abs() < 0.03, || format!("rolling resistance: {} m/s want {}", v, want));
        }
    }
    // two balls meet head-on in space: elastic they swap their velocities, plastic they move on together;
    // momentum stays
    for &e in &[1.0, 0.0] {
        let mut sim = Simulation::with_params(no_gravity());
        let a = sim.add(RigidBody::dynamic(Shape::sphere(0.5), Iso::at(Vec3::new(-2.0, 0.0, 0.0))).with_velocity(Vec3::new(3.0, 0.0, 0.0)).with_restitution(e)).expect("a");
        let b = sim.add(RigidBody::dynamic(Shape::sphere(0.5), Iso::at(Vec3::new(2.0, 0.0, 0.0))).with_velocity(Vec3::new(-1.0, 0.0, 0.0)).with_restitution(e)).expect("b");
        let m = sim.mass(a).expect("m").mass;
        run(&mut sim, 120);
        let (va, vb) = (sim.velocity(a).expect("v").0, sim.velocity(b).expect("v").0);
        let p = (va + vb) * m;
        c.check((p.x - 2.0 * m).abs() < 1e-9 * m && p.y.abs() < 1e-9 * m, || format!("momentum {:?}", p));
        if e == 1.0 {
            c.check((va.x + 1.0).abs() < 0.03 && (vb.x - 3.0).abs() < 0.03, || format!("elastic: {:?} {:?}", va, vb));
        } else {
            c.check((va.x - 1.0).abs() < 0.01 && (vb.x - 1.0).abs() < 0.01, || format!("plastic: {:?} {:?}", va, vb));
        }
    }
    // a box on a flat mesh rests with one manifold of four points across the seams
    {
        let mut sim = Simulation::new();
        let grid = scene::flat(8, 1.0, 0.0).build();
        sim.add(RigidBody::fixed(Shape::mesh(grid), Iso::at(Vec3::new(-4.0, 0.0, -4.0)))).expect("grid");
        let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.7, 0.3, 0.45)), Iso::new(Vec3::new(0.13, 1.0, 0.27), Quat::from_axis_angle(Vec3::Y, 0.4)))).expect("box");
        run(&mut sim, 240);
        let s = sim.state(b).expect("state");
        let ms: Vec<_> = sim.manifolds().filter(|m| m.a == b || m.b == b).collect();
        let bottom = Shape::cuboid(Vec3::new(0.7, 0.3, 0.45)).aabb(&s.iso).min.y;
        c.check(ms.len() == 1 && ms[0].count == 4, || format!("box on a mesh: {} manifolds {:?}", ms.len(), ms.iter().map(|m| m.count).collect::<Vec<_>>()));
        c.check(bottom > -0.006 && bottom < 0.002 && !s.awake, || format!("box on a mesh rests at {} awake {}", bottom, s.awake));
    }
    // a ball in a valley of two faces 2 degrees apart: one manifold (within the merge), every point on the line
    // through its centre along the normal, so it is pushed through its centre also by the face of the other
    // normal
    {
        let mut sim = Simulation::new();
        let (sn, cs) = math::sincos(1.0f64.to_radians());
        let t = sn / cs;
        let h = (5.0 * t) as f32;
        let pos = [[-5.0, h, 5.0], [0.0, 0.0, 5.0], [0.0, 0.0, -5.0], [-5.0, h, -5.0], [5.0, h, 5.0], [5.0, h, -5.0]];
        let valley = physics::TriMesh::new(&pos, &[0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2]).expect("valley");
        sim.add(RigidBody::fixed(Shape::mesh(valley), Iso::IDENTITY)).expect("valley");
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::at(Vec3::new(0.0, sqrt(1.0 + t * t), 0.0)))).expect("ball");
        run(&mut sim, 30);
        let center = sim.state(ball).expect("state").center;
        sim.step(DT);
        let ms: Vec<_> = sim.manifolds().filter(|m| m.a == ball).collect();
        let off = ms.iter().flat_map(|m| m.points[..m.count].iter().map(move |p| (p.point - center).cross(m.normal).length())).fold(0.0, f64::max);
        let counts: Vec<usize> = ms.iter().map(|m| m.count).collect();
        c.check(counts == [2] && off < 1e-9, || format!("a ball in a valley: points {:?}, up to {} m off the line through its centre", counts, off));
    }
    // a kinematic platform carries a box (friction) and lifts it
    {
        let mut sim = Simulation::new();
        floor(&mut sim, 30.0);
        let plat = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::new(2.0, 0.1, 2.0)), Iso::at(Vec3::new(0.0, 0.5, 0.0)))).expect("platform");
        let b = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.3)), Iso::at(Vec3::new(0.0, 0.9, 0.0)))).expect("box");
        run(&mut sim, 30);
        sim.set_velocity(plat, Vec3::new(1.0, 0.5, 0.0), Vec3::ZERO);
        run(&mut sim, 120);
        let (pv, bv) = (sim.velocity(plat).expect("v").0, sim.velocity(b).expect("v").0);
        let (pp, bp) = (sim.state(plat).expect("s").center, sim.state(b).expect("s").center);
        c.check((bv - pv).length() < 0.02, || format!("carried box {:?} platform {:?}", bv, pv));
        c.check((bp.y - pp.y - 0.4).abs() < 0.01 && (bp.x - pp.x).abs() < 0.05, || format!("box at {:?} on the platform at {:?}", bp, pp));
    }
    // a heavy box on a light one: the mass ratio 100 stands
    {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let low = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 0.5)), Iso::at(Vec3::new(0.0, 0.25, 0.0))).with_density(100.0)).expect("light");
        let high = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 0.5)), Iso::at(Vec3::new(0.0, 0.75, 0.0))).with_density(10_000.0)).expect("heavy");
        run(&mut sim, 300);
        let (l, hh) = (sim.state(low).expect("s").center, sim.state(high).expect("s").center);
        // soft contacts give: the heavy box presses into the light one by about 100 x g / (2 pi 30 Hz)^2 (2.8 cm,
        // docs/PHYSICS.md); it must stay on it, upright and still, and the light box on the floor
        crate::out(&format!("  contact: at 1:100 the heavy box presses {:.1} mm into the light one, that one {:.1} mm into the floor\n", (0.75 - hh.y + l.y - 0.25) * 1000.0, (0.25 - l.y) * 1000.0));
        c.check((l.y - 0.25).abs() < 0.015 && hh.y > 0.69 && hh.y < 0.751 && hh.x.abs() < 0.01 && hh.z.abs() < 0.01, || format!("mass ratio 100: {:?} {:?}", l, hh));
        c.check(!sim.is_awake(low) && !sim.is_awake(high), || "mass ratio 100 did not fall asleep".into());
        // 1:10 sits within millimetres
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let low = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 0.5)), Iso::at(Vec3::new(0.0, 0.25, 0.0))).with_density(300.0)).expect("light");
        let high = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 0.25, 0.5)), Iso::at(Vec3::new(0.0, 0.75, 0.0))).with_density(3000.0)).expect("heavy");
        run(&mut sim, 300);
        let (l, hh) = (sim.state(low).expect("s").center, sim.state(high).expect("s").center);
        c.check((l.y - 0.25).abs() < 0.003 && (hh.y - 0.75).abs() < 0.008 && hh.x.abs() < 0.002, || format!("mass ratio 10: {:?} {:?}", l, hh));
    }
}

// ---------------------------------------------------------------- stack

pub fn stack_tests(c: &mut Ctx) {
    // a tower of boxes stands and falls asleep
    for &n in &[5usize, 12] {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let mut ids = Vec::new();
        for k in 0..n {
            ids.push(sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(0.0, 0.5 + k as f64, 0.0)))).expect("box"));
        }
        let mut steps = 0;
        while steps < 1200 && ids.iter().any(|b| sim.is_awake(*b)) {
            sim.step(DT);
            steps += 1;
        }
        let top = sim.state(ids[n - 1]).expect("top").center;
        crate::out(&format!("  stack: a tower of {} sags {:.1} mm, its top {:.1} mm aside, asleep after {} steps\n", n, (n as f64 - 0.5 - top.y) * 1000.0, sqrt(top.x * top.x + top.z * top.z) * 1000.0, steps));
        c.check((top.y - (n as f64 - 0.5)).abs() < 0.05 && (top.x * top.x + top.z * top.z) < 0.01 * 0.01, || format!("tower of {}: top at {:?}", n, top));
        c.check(steps < 600, || format!("tower of {} asleep after {} steps", n, steps));
    }
    // a pyramid of boxes
    {
        let rows = if c.quick { 6 } else { 12 };
        let mut sim = Simulation::new();
        floor(&mut sim, 40.0);
        let mut ids = Vec::new();
        for row in 0..rows {
            for k in 0..rows - row {
                let x = (k as f64 - (rows - row - 1) as f64 * 0.5) * 1.02;
                ids.push(sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(x, 0.5 + row as f64, 0.0)))).expect("box"));
            }
        }
        let mut steps = 0;
        while steps < 1500 && ids.iter().any(|b| sim.is_awake(*b)) {
            sim.step(DT);
            steps += 1;
        }
        let top = sim.state(*ids.last().expect("top")).expect("top").center;
        c.check((top.y - (rows as f64 - 0.5)).abs() < 0.1 && top.x.abs() < 0.05, || format!("pyramid of {} rows: top at {:?}", rows, top));
        c.check(steps < 1200, || format!("pyramid asleep after {} steps", steps));
    }
    // a heap of every shape in a pit on a mesh: nothing leaves it or sinks into the ground, it comes to rest (with
    // rolling resistance: without it a ball rolls on for a minute, slowed by its angular damping only)
    {
        let mut rng = Rng::new(77);
        let mut sim = Simulation::new();
        let ground = scene::flat(10, 1.0, 0.0).build();
        sim.add(RigidBody::fixed(Shape::mesh(ground), Iso::at(Vec3::new(-5.0, 0.0, -5.0)))).expect("ground");
        for (x, z, hx, hz) in [(-3.1, 0.0, 0.1, 3.2), (3.1, 0.0, 0.1, 3.2), (0.0, -3.1, 3.2, 0.1), (0.0, 3.1, 3.2, 0.1)] {
            sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(hx, 2.0, hz)), Iso::at(Vec3::new(x, 2.0, z)))).expect("wall");
        }
        let mut ids = Vec::new();
        for k in 0..c.n(120) {
            let s = random_convex(&mut rng, k % 4, 0.35);
            let at = Iso::new(Vec3::new(rng.range(-2.4, 2.4), 1.0 + k as f64 * 0.25, rng.range(-2.4, 2.4)), rng.quat());
            ids.push((sim.add(RigidBody::dynamic(s.clone(), at).with_rolling_resistance(0.05)).expect("body"), s));
        }
        let mut steps = 0;
        while steps < 3600 && ids.iter().any(|(b, _)| sim.is_awake(*b)) {
            sim.step(DT);
            steps += 1;
        }
        for (b, s) in &ids {
            let st = sim.state(*b).expect("state");
            let bx = s.aabb(&st.iso);
            let inside = bx.min.y > -0.02 && bx.min.x > -3.05 && bx.max.x < 3.05 && bx.min.z > -3.05 && bx.max.z < 3.05;
            c.check(inside && st.center.is_finite(), || format!("heap: {:?} at {:?}", s, bx));
        }
        crate::out(&format!("  stack: a heap of {} bodies on a mesh asleep after {} steps\n", ids.len(), steps));
        c.check(steps < 3600, || format!("heap not asleep after {} steps ({} awake)", steps, ids.iter().filter(|(b, _)| sim.is_awake(*b)).count()));
    }
}

// ---------------------------------------------------------------- joint

/// a static body that collides with nothing: something to hang joints on
fn anchor(sim: &mut Simulation, at: Vec3) -> BodyId {
    sim.add(RigidBody::fixed(Shape::sphere(0.01), Iso::at(at)).with_layers(0, 0)).expect("anchor")
}

/// the distance between a joint's two anchors and its swing (angle between the frames' X axes) and twist
fn joint_error(sim: &Simulation, j: JointId) -> (f64, f64, f64) {
    let jt = *sim.joint(j).expect("joint");
    let (ia, ib) = (sim.iso(jt.a).expect("a"), sim.iso(jt.b).expect("b"));
    let (fa, fb) = (ia.mul(&jt.frame_a), ib.mul(&jt.frame_b));
    let gap = (fb.pos - fa.pos).length();
    let (xa, xb) = (fa.rot.rotate(Vec3::X), fb.rot.rotate(Vec3::X));
    let swing = math::atan2(xa.cross(xb).length(), xa.dot(xb));
    let mut r = fa.rot.conjugate().mul(fb.rot);
    if r.w < 0.0 {
        r = Quat::new(-r.x, -r.y, -r.z, -r.w);
    }
    // the twist: the rotation about X left after the swing is taken out
    let tw = 2.0 * math::atan2(r.x, r.w);
    (gap, swing, tw)
}

/// the time between the first and the last of the down-crossings of x through 0, divided by their number
/// minus one: the period of an oscillation
fn period(xs: &[f64], dt: f64) -> f64 {
    let mut cross = Vec::new();
    for k in 1..xs.len() {
        if xs[k - 1] > 0.0 && xs[k] <= 0.0 {
            // linear interpolation between the samples
            let f = xs[k - 1] / (xs[k - 1] - xs[k]);
            cross.push((k - 1) as f64 + f);
        }
    }
    if cross.len() < 2 {
        return f64::NAN;
    }
    (cross[cross.len() - 1] - cross[0]) / (cross.len() - 1) as f64 * dt
}

/// a figure of ten bodies: pelvis, chest, head, upper and lower arms and legs; ball joints with cones and
/// twists at the neck, shoulders and hips, hinges with limits at the elbows and knees -> the joints with
/// their limits (cone or hinge range, twist range)
pub fn ragdoll(sim: &mut Simulation, at: Iso) -> Vec<(JointId, Option<(f64, f64)>, Option<f64>, Option<(f64, f64)>)> {
    let place = |p: Vec3, r: Quat| Iso::new(at.transform_point(p), at.rot.mul(r));
    let side = Quat::from_axis_angle(Vec3::Z, PI * 0.5); // a capsule lying along X
    let pelvis = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.16, 0.1, 0.1)), place(Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY))).expect("pelvis");
    let chest = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.18, 0.2, 0.11)), place(Vec3::new(0.0, 1.32, 0.0), Quat::IDENTITY))).expect("chest");
    let head = sim.add(RigidBody::dynamic(Shape::sphere(0.11), place(Vec3::new(0.0, 1.66, 0.0), Quat::IDENTITY))).expect("head");
    let mut joints = Vec::new();
    let up = Quat::from_axis_angle(Vec3::Z, PI * 0.5); // the joint's X axis up
    let ball = |sim: &mut Simulation, a: BodyId, b: BodyId, p: Vec3, q: Quat, cone: f64, tw: (f64, f64)| {
        let kind = JointKind::Ball { cone: Some(cone), twist: Some(tw) };
        let j = sim.add_joint(a, b, place(p, q), kind).expect("ball joint");
        (j, None, Some(cone), Some(tw))
    };
    joints.push(ball(sim, pelvis, chest, Vec3::new(0.0, 1.1, 0.0), up, 0.5, (-0.4, 0.4)));
    joints.push(ball(sim, chest, head, Vec3::new(0.0, 1.54, 0.0), up, 0.7, (-0.8, 0.8)));
    for s in [-1.0, 1.0] {
        // the arms hang along X from the shoulders, the legs down from the hips
        let upper = sim.add(RigidBody::dynamic(Shape::capsule(0.12, 0.05), place(Vec3::new(s * 0.36, 1.46, 0.0), side))).expect("upper arm");
        let lower = sim.add(RigidBody::dynamic(Shape::capsule(0.11, 0.045), place(Vec3::new(s * 0.64, 1.46, 0.0), side))).expect("lower arm");
        let arm = if s > 0.0 { Quat::IDENTITY } else { Quat::from_axis_angle(Vec3::Y, PI) };
        joints.push(ball(sim, chest, upper, Vec3::new(s * 0.2, 1.46, 0.0), arm, 1.2, (-0.6, 0.6)));
        let elbow = sim.add_joint(upper, lower, place(Vec3::new(s * 0.5, 1.46, 0.0), Quat::from_axis_angle(Vec3::Z, PI * 0.5)), JointKind::Hinge { limits: Some((0.0, 2.4)), motor: None, spring: None }).expect("elbow");
        joints.push((elbow, Some((0.0, 2.4)), None, None));
        let thigh = sim.add(RigidBody::dynamic(Shape::capsule(0.16, 0.07), place(Vec3::new(s * 0.1, 0.7, 0.0), Quat::IDENTITY))).expect("thigh");
        let shin = sim.add(RigidBody::dynamic(Shape::capsule(0.16, 0.055), place(Vec3::new(s * 0.1, 0.24, 0.0), Quat::IDENTITY))).expect("shin");
        let down = Quat::from_axis_angle(Vec3::Z, -PI * 0.5);
        joints.push(ball(sim, pelvis, thigh, Vec3::new(s * 0.1, 0.93, 0.0), down, 0.9, (-0.3, 0.3)));
        let knee = sim.add_joint(thigh, shin, place(Vec3::new(s * 0.1, 0.47, 0.0), Quat::IDENTITY), JointKind::Hinge { limits: Some((-2.4, 0.0)), motor: None, spring: None }).expect("knee");
        joints.push((knee, Some((-2.4, 0.0)), None, None));
    }
    joints
}

pub fn joint_tests(c: &mut Ctx) {
    // the times these simulate are part of what they check: --quick only drops ragdolls
    let g = 9.81;
    // pendulums: the period of a ball on a ball joint against 2 pi sqrt(I / (m g L)) and its series in the
    // amplitude; the joint carries m g at rest
    for &(l, a0) in &[(1.0, 5.0f64.to_radians()), (2.5, 60.0f64.to_radians())] {
        let mut sim = Simulation::new();
        let top = anchor(&mut sim, Vec3::new(0.0, 10.0, 0.0));
        let r = 0.05;
        let (s, cc) = math::sincos(a0);
        let bob = sim.add(RigidBody::dynamic(Shape::sphere(r), Iso::at(Vec3::new(l * s, 10.0 - l * cc, 0.0))).with_damping(0.0, 0.0).sleepless()).expect("bob");
        let j = sim.add_joint(top, bob, Iso::at(Vec3::new(0.0, 10.0, 0.0)), JointKind::BALL).expect("joint");
        let mut xs = Vec::new();
        for _ in 0..1200 {
            sim.step(DT);
            xs.push(sim.state(bob).expect("s").center.x);
        }
        let t = period(&xs, DT);
        let ip = l * l + 0.4 * r * r;
        let t0 = 2.0 * PI * sqrt(ip / (g * l));
        let series = 1.0 + a0 * a0 / 16.0 + 11.0 * a0 * a0 * a0 * a0 / 3072.0 + 173.0 * (a0 * a0) * (a0 * a0) * (a0 * a0) / 737_280.0;
        c.check(rel(t, t0 * series) < 0.005, || format!("pendulum L {} amplitude {}: period {} want {}", l, a0, t, t0 * series));
        let (gap, _, _) = joint_error(&sim, j);
        c.check(gap < 0.002, || format!("pendulum L {}: the anchors {} m apart", l, gap));
    }
    {
        let mut sim = Simulation::new();
        let top = anchor(&mut sim, Vec3::new(0.0, 5.0, 0.0));
        let bob = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.2)), Iso::at(Vec3::new(0.0, 4.0, 0.0)))).expect("bob");
        let j = sim.add_joint(top, bob, Iso::at(Vec3::new(0.0, 5.0, 0.0)), JointKind::BALL).expect("joint");
        run(&mut sim, 120);
        let m = sim.mass(bob).expect("m").mass;
        let (f, _) = sim.joint_force(j).expect("force");
        let (fy, want) = (f.y, m * g);
        c.check((fy - want).abs() < 0.01 * m * g, || format!("the joint pulls b up: force on b {:?}, want y {}", f, want));
    }
    // a chain of 20 links released horizontally: the links stay together while it swings and at rest
    {
        let mut sim = Simulation::new();
        let top = anchor(&mut sim, Vec3::new(0.0, 20.0, 0.0));
        let side = Quat::from_axis_angle(Vec3::Z, PI * 0.5);
        let mut prev = top;
        let mut js = Vec::new();
        for k in 0..20 {
            let x = 0.25 + 0.5 * k as f64;
            let link = sim.add(RigidBody::dynamic(Shape::capsule(0.2, 0.05), Iso::new(Vec3::new(x, 20.0, 0.0), side)).with_layers(2, 1)).expect("link");
            js.push(sim.add_joint(prev, link, Iso::at(Vec3::new(x - 0.25, 20.0, 0.0)), JointKind::BALL).expect("link joint"));
            prev = link;
        }
        let mut worst = 0.0f64;
        for _ in 0..240 {
            sim.step(DT);
            for j in &js {
                worst = worst.max(joint_error(&sim, *j).0);
            }
        }
        c.check(worst < 0.03, || format!("chain: the links came {} m apart while swinging", worst));
        run(&mut sim, 900);
        let rest = js.iter().map(|j| joint_error(&sim, *j).0).fold(0.0, f64::max);
        // soft joints: the top one carries 20 links (docs/PHYSICS.md)
        crate::out(&format!("  joint: a chain of 20 links: {:.1} mm apart while swinging, {:.2} mm at rest\n", worst * 1000.0, rest * 1000.0));
        c.check(rest < 0.006, || format!("chain at rest: links {} m apart", rest));
    }
    // a hinged door stays within its limits however it is pushed
    {
        let mut sim = Simulation::new();
        let post = anchor(&mut sim, Vec3::new(0.0, 1.0, 0.0));
        let door = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 1.0, 0.05)), Iso::at(Vec3::new(0.5, 1.0, 0.0))).sleepless()).expect("door");
        let lo = -0.2;
        let hi = 1.5;
        let up = Quat::from_axis_angle(Vec3::Z, PI * 0.5);
        let j = sim.add_joint(post, door, Iso::new(Vec3::new(0.0, 1.0, 0.0), up), JointKind::Hinge { limits: Some((lo, hi)), motor: None, spring: None }).expect("hinge");
        let mut rng = Rng::new(5);
        let (mut amin, mut amax, mut worst_gap, mut worst_axis) = (f64::INFINITY, f64::NEG_INFINITY, 0.0f64, 0.0f64);
        for step in 0..1200 {
            if step % 20 == 0 {
                let p = sim.state(door).expect("s").center + Vec3::new(0.0, 0.0, 0.0);
                sim.apply_impulse(door, Vec3::new(0.0, 0.0, rng.range(-400.0, 400.0)), p + Vec3::new(0.4, 0.0, 0.0));
            }
            sim.step(DT);
            let a = sim.joint_position(j).expect("angle");
            amin = amin.min(a);
            amax = amax.max(a);
            let (gap, swing, _) = joint_error(&sim, j);
            worst_gap = worst_gap.max(gap);
            worst_axis = worst_axis.max(swing);
        }
        let tol = 2.0f64.to_radians();
        c.check(amin > lo - tol && amax < hi + tol, || format!("door angle within {}..{}, limits {}..{}", amin, amax, lo, hi));
        c.check(amax > hi - 0.1 && amin < lo + 0.1, || format!("the pushes did not reach the limits: {}..{}", amin, amax));
        c.check(worst_gap < 0.01 && worst_axis < 1.0f64.to_radians(), || format!("door hinge: anchors {} m, axis {} rad off", worst_gap, worst_axis));
    }
    // a motor turns a wheel up to its speed; a weak one barely
    for &(torque, ok_speed) in &[(500.0, true), (0.5, false)] {
        let mut sim = Simulation::with_params(no_gravity());
        let hub = anchor(&mut sim, Vec3::ZERO);
        let wheel = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.1, 0.5, 0.5)), Iso::IDENTITY).with_damping(0.0, 0.0).sleepless()).expect("wheel");
        let j = sim.add_joint(hub, wheel, Iso::IDENTITY, JointKind::Hinge { limits: None, motor: Some(Motor { speed: 3.0, max_force: torque }), spring: None }).expect("hinge");
        run(&mut sim, 60);
        let w = sim.velocity(wheel).expect("v").1;
        if ok_speed {
            c.check((w.x - 3.0).abs() < 0.01 && w.y.abs() + w.z.abs() < 1e-6, || format!("motor speed {:?}", w));
        } else {
            let i = sim.mass(wheel).expect("m").inertia.at(0, 0);
            let want = torque / i * 1.0;
            c.check((w.x - want).abs() < 0.02 * want, || format!("weak motor: {} rad/s want {}", w.x, want));
        }
        let _ = j;
    }
    // a hinge spring of 2 Hz oscillates at 2 Hz; a slider spring of 1.5 Hz at 1.5 Hz; a distance spring too
    {
        let mut sim = Simulation::with_params(no_gravity());
        let hub = anchor(&mut sim, Vec3::ZERO);
        // the frames are aligned when the joint is made, so the angle starts at 0 and the spring pulls to -0.3
        let bar = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.1, 0.1, 0.8)), Iso::IDENTITY).with_damping(0.0, 0.0).sleepless()).expect("bar");
        let j = sim.add_joint(hub, bar, Iso::IDENTITY, JointKind::Hinge { limits: None, motor: None, spring: Some(Spring { target: -0.3, hertz: 2.0, damping_ratio: 0.0 }) }).expect("hinge");
        let mut xs = Vec::new();
        for _ in 0..600 {
            sim.step(DT);
            xs.push(sim.joint_position(j).expect("angle") + 0.3);
        }
        let t = period(&xs, DT);
        c.check(rel(t, 0.5) < 0.03, || format!("hinge spring period {} want 0.5", t));
        let mut sim = Simulation::with_params(no_gravity());
        let rail = anchor(&mut sim, Vec3::ZERO);
        let car = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.2)), Iso::IDENTITY).with_damping(0.0, 0.0).sleepless()).expect("car");
        let j = sim.add_joint(rail, car, Iso::IDENTITY, JointKind::Slider { limits: None, motor: None, spring: Some(Spring { target: -0.3, hertz: 1.5, damping_ratio: 0.0 }) }).expect("slider");
        let mut xs = Vec::new();
        for _ in 0..600 {
            sim.step(DT);
            xs.push(sim.joint_position(j).expect("x") + 0.3);
        }
        let t = period(&xs, DT);
        c.check(rel(t, 1.0 / 1.5) < 0.03, || format!("slider spring period {} want {}", t, 1.0 / 1.5));
        let mut sim = Simulation::with_params(no_gravity());
        let a = anchor(&mut sim, Vec3::ZERO);
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(Vec3::new(1.4, 0.0, 0.0))).with_damping(0.0, 0.0).sleepless()).expect("ball");
        let j = sim.add_distance_joint(a, ball, Vec3::ZERO, Vec3::new(1.4, 0.0, 0.0), JointKind::Distance { min: 0.5, max: 2.0, spring: Some(Spring { target: 1.0, hertz: 1.0, damping_ratio: 0.0 }) }).expect("spring");
        let mut xs = Vec::new();
        for _ in 0..600 {
            sim.step(DT);
            xs.push(sim.joint_position(j).expect("length") - 1.0);
        }
        let t = period(&xs, DT);
        c.check(rel(t, 1.0) < 0.03, || format!("distance spring period {} want 1", t));
    }
    // a weld holds two bodies as one while they tumble onto the floor; a welded beam hardly sags
    {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let a = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.3, 0.2, 0.2)), Iso::new(Vec3::new(0.0, 3.0, 0.0), Quat::from_axis_angle(Vec3::new(1.0, 1.0, 0.0).normalize_or_zero(), 0.7))).with_angular_velocity(Vec3::new(2.0, 5.0, -1.0))).expect("a");
        let ia = sim.iso(a).expect("iso");
        let b = sim.add(RigidBody::dynamic(Shape::sphere(0.25), Iso::new(ia.transform_point(Vec3::new(0.55, 0.0, 0.0)), ia.rot)).with_density(3000.0)).expect("b");
        let j = sim.add_joint(a, b, Iso::new(ia.transform_point(Vec3::new(0.3, 0.0, 0.0)), ia.rot), JointKind::Fixed).expect("weld");
        let (mut worst_gap, mut worst_swing, mut worst_twist) = (0.0f64, 0.0f64, 0.0f64);
        for _ in 0..300 {
            sim.step(DT);
            let (gap, swing, twist) = joint_error(&sim, j);
            worst_gap = worst_gap.max(gap);
            worst_swing = worst_swing.max(swing);
            worst_twist = worst_twist.max(twist.abs());
        }
        c.check(worst_gap < 0.01 && worst_swing < 1.5f64.to_radians() && worst_twist < 1.5f64.to_radians(), || format!("weld: {} m, {} rad, {} rad", worst_gap, worst_swing, worst_twist));
        let mut sim = Simulation::new();
        let wall = anchor(&mut sim, Vec3::new(0.0, 2.0, 0.0));
        let beam = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(1.0, 0.05, 0.05)), Iso::at(Vec3::new(1.0, 2.0, 0.0)))).expect("beam");
        sim.add_joint(wall, beam, Iso::at(Vec3::new(0.0, 2.0, 0.0)), JointKind::Fixed).expect("weld");
        run(&mut sim, 180);
        let tip = sim.iso(beam).expect("iso").transform_point(Vec3::new(1.0, 0.0, 0.0));
        c.check((tip.y - 2.0).abs() < 0.01 && (tip.x - 2.0).abs() < 0.01, || format!("beam tip at {:?}", tip));
    }
    // a slider down a slope stops at its limit, stays on its axis, does not turn; its motor drives it
    {
        let mut sim = Simulation::new();
        let rail = anchor(&mut sim, Vec3::new(0.0, 5.0, 0.0));
        let axis = Vec3::new(1.0, -0.4, 0.2).normalize_or_zero();
        let q = Quat::from_rotation_arc(Vec3::X, axis);
        let car = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.3, 0.2, 0.2)), Iso::at(Vec3::new(0.0, 5.0, 0.0))).sleepless()).expect("car");
        let j = sim.add_joint(rail, car, Iso::new(Vec3::new(0.0, 5.0, 0.0), q), JointKind::Slider { limits: Some((-1.0, 1.5)), motor: None, spring: None }).expect("slider");
        let (mut xmax, mut worst_off, mut worst_rot) = (f64::NEG_INFINITY, 0.0f64, 0.0f64);
        for _ in 0..240 {
            sim.step(DT);
            let x = sim.joint_position(j).expect("x");
            xmax = xmax.max(x);
            let s = sim.iso(car).expect("iso");
            let d = s.pos - Vec3::new(0.0, 5.0, 0.0);
            worst_off = worst_off.max((d - axis * d.dot(axis)).length());
            worst_rot = worst_rot.max(2.0 * math::atan2(Vec3::new(s.rot.x, s.rot.y, s.rot.z).length(), s.rot.w.abs()));
        }
        c.check(xmax < 1.5 + 0.01 && sim.joint_position(j).expect("x") > 1.5 - 0.01, || format!("slider ends at {} (max {}) want 1.5", sim.joint_position(j).expect("x"), xmax));
        c.check(worst_off < 0.003 && worst_rot < 0.5f64.to_radians(), || format!("slider: {} m off the axis, turned {} rad", worst_off, worst_rot));
        sim.set_joint(j, JointKind::Slider { limits: Some((-1.0, 1.5)), motor: Some(Motor { speed: -0.5, max_force: 5000.0 }), spring: None }, false);
        run(&mut sim, 60);
        let (v, _) = sim.velocity(car).expect("v");
        c.check((v.dot(axis) + 0.5).abs() < 0.01, || format!("slider motor: {} m/s along the axis want -0.5", v.dot(axis)));
    }
    // a rope never gets longer than its length; a rod keeps its length
    for &(kind, lo, hi) in &[(JointKind::rope(2.0), 0.0, 2.0), (JointKind::rod(1.5), 1.5, 1.5)] {
        let mut sim = Simulation::new();
        let top = anchor(&mut sim, Vec3::new(0.0, 10.0, 0.0));
        let start = if hi > lo { Vec3::new(0.3, 10.0, 0.0) } else { Vec3::new(1.5, 10.0, 0.0) };
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(start)).with_velocity(Vec3::new(0.0, 0.0, 3.0))).expect("ball");
        let j = sim.add_distance_joint(top, ball, Vec3::new(0.0, 10.0, 0.0), start, kind).expect("rope");
        let (mut lmin, mut lmax) = (f64::INFINITY, f64::NEG_INFINITY);
        for _ in 0..300 {
            sim.step(DT);
            let len = sim.joint_position(j).expect("length");
            lmin = lmin.min(len);
            lmax = lmax.max(len);
        }
        c.check(lmax < hi + 0.02 && (hi > lo || lmin > lo - 0.02), || format!("distance {}..{}: length {}..{}", lo, hi, lmin, lmax));
    }
    // a ball joint with a cone and a twist range: shaken, it stays within both
    {
        let mut sim = Simulation::new();
        let top = anchor(&mut sim, Vec3::new(0.0, 5.0, 0.0));
        let down = Quat::from_axis_angle(Vec3::Z, -PI * 0.5);
        let limb = sim.add(RigidBody::dynamic(Shape::capsule(0.3, 0.06), Iso::at(Vec3::new(0.0, 4.6, 0.0))).sleepless()).expect("limb");
        let cone = 0.5;
        let tw = (-0.35, 0.35);
        let j = sim.add_joint(top, limb, Iso::new(Vec3::new(0.0, 5.0, 0.0), down), JointKind::Ball { cone: Some(cone), twist: Some(tw) }).expect("ball");
        let mut rng = Rng::new(9);
        let (mut swing, mut tmin, mut tmax, mut gap) = (0.0f64, f64::INFINITY, f64::NEG_INFINITY, 0.0f64);
        for step in 0..1200 {
            if step % 15 == 0 {
                sim.apply_angular_impulse(limb, rng.vec(-1.0, 1.0) * 3.0);
                sim.apply_impulse(limb, rng.vec(-1.0, 1.0) * 10.0, sim.state(limb).expect("s").center);
            }
            sim.step(DT);
            let (g2, s, t) = joint_error(&sim, j);
            swing = swing.max(s);
            tmin = tmin.min(t);
            tmax = tmax.max(t);
            gap = gap.max(g2);
        }
        let tol = 3.0f64.to_radians();
        c.check(swing < cone + tol && swing > cone - 0.1, || format!("cone {}: swing up to {}", cone, swing));
        c.check(tmin > tw.0 - tol && tmax < tw.1 + tol && tmax > tw.1 - 0.15 && tmin < tw.0 + 0.15, || format!("twist {:?}: {}..{}", tw, tmin, tmax));
        c.check(gap < 0.01, || format!("ball joint anchors {} m apart", gap));
    }
    // ragdolls dropped on a floor of boxes: while they hit it the soft joints give a little (a limb is
    // pressed past its limit by the ground, which is solved last), once they lie still every joint holds
    // within millimetres and 2 degrees, nothing in the floor
    {
        let mut rng = Rng::new(12);
        let mut sim = Simulation::new();
        let ground = scene::flat(12, 1.0, 0.0).build();
        sim.add(RigidBody::fixed(Shape::mesh(ground), Iso::at(Vec3::new(-6.0, 0.0, -6.0)))).expect("ground");
        for k in 0..6 {
            sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(0.4, 0.3, 0.4)), Iso::new(Vec3::new(rng.range(-3.0, 3.0), 0.3, rng.range(-3.0, 3.0)), Quat::from_axis_angle(Vec3::Y, k as f64)))).expect("box");
        }
        let mut all = Vec::new();
        for _ in 0..c.n(8) {
            let at = Iso::new(Vec3::new(rng.range(-3.0, 3.0), rng.range(1.0, 4.0), rng.range(-3.0, 3.0)), rng.quat());
            all.extend(ragdoll(&mut sim, at));
        }
        let measure = |sim: &Simulation| {
            let mut worst = (0.0f64, 0.0f64);
            for (j, hinge, cone, tw) in &all {
                let (gap, swing, twist) = joint_error(sim, *j);
                worst.0 = worst.0.max(gap);
                let over = match (hinge, cone, tw) {
                    (Some((lo, hi)), _, _) => {
                        let a = sim.joint_position(*j).expect("angle");
                        (lo - a).max(a - hi).max(0.0)
                    }
                    (None, Some(cmax), Some((lo, hi))) => (swing - cmax).max(lo - twist).max(twist - hi).max(0.0),
                    _ => 0.0,
                };
                worst.1 = worst.1.max(over);
            }
            worst
        };
        let mut hit = (0.0f64, 0.0f64);
        for _ in 0..600 {
            sim.step(DT);
            let w = measure(&sim);
            hit = (hit.0.max(w.0), hit.1.max(w.1));
        }
        let rest = measure(&sim);
        crate::out(&format!(
            "  joint: ragdolls: falling up to {:.1} mm apart and {:.1} degrees past a limit, at rest {:.2} mm and {:.2} degrees\n",
            hit.0 * 1000.0,
            hit.1.to_degrees(),
            rest.0 * 1000.0,
            rest.1.to_degrees()
        ));
        c.check(hit.0 < 0.05 && hit.1 < 12.0f64.to_radians(), || format!("ragdolls while falling: anchors up to {} m apart, a limit exceeded by {} rad", hit.0, hit.1));
        c.check(rest.0 < 0.005 && rest.1 < 2.0f64.to_radians(), || format!("ragdolls at rest: anchors up to {} m apart, a limit exceeded by {} rad", rest.0, rest.1));
        let low = sim.world().bodies().filter(|(_, b)| b.shape.is_convex()).map(|(_, b)| b.shape.aabb(&b.iso).min.y).fold(f64::INFINITY, f64::min);
        c.check(low > -0.01, || format!("ragdolls: a body {} m in the floor", -low));
    }
    // joints made far outside their limits come back to them without a jolt
    {
        let mut sim = Simulation::new();
        let post = anchor(&mut sim, Vec3::new(0.0, 3.0, 0.0));
        let door = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 1.0, 0.05)), Iso::at(Vec3::new(0.5, 3.0, 0.0))).sleepless()).expect("door");
        let up = Quat::from_axis_angle(Vec3::Z, PI * 0.5);
        let h = sim.add_joint(post, door, Iso::new(Vec3::new(0.0, 3.0, 0.0), up), JointKind::Hinge { limits: Some((1.5, 2.5)), motor: None, spring: None }).expect("hinge");
        let rail = anchor(&mut sim, Vec3::new(5.0, 3.0, 0.0));
        let car = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.2)), Iso::at(Vec3::new(5.0, 3.0, 0.0))).sleepless()).expect("car");
        let s = sim.add_joint(rail, car, Iso::at(Vec3::new(5.0, 3.0, 0.0)), JointKind::Slider { limits: Some((2.0, 3.0)), motor: None, spring: None }).expect("slider");
        let mut fastest = 0.0f64;
        for _ in 0..180 {
            sim.step(DT);
            fastest = fastest.max(sim.velocity(car).expect("v").0.length()).max(sim.velocity_at(door, sim.iso(door).expect("iso").transform_point(Vec3::new(0.5, 0.0, 0.0))).expect("v").length());
        }
        let (a, x) = (sim.joint_position(h).expect("angle"), sim.joint_position(s).expect("x"));
        c.check(a > 1.5 - 0.03 && a < 2.5 + 0.03 && x > 2.0 - 0.01 && x < 3.0 + 0.01, || format!("joints made outside their limits: angle {} position {}", a, x));
        c.check(fastest < 30.0, || format!("joints made outside their limits jolted their bodies to {} m/s", fastest));
    }
    // a rod made with both anchors in one point pushes them apart to its length
    {
        let mut sim = Simulation::with_params(no_gravity());
        let hub = anchor(&mut sim, Vec3::ZERO);
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(Vec3::new(0.0, 0.0, 0.0))).sleepless()).expect("ball");
        let j = sim.add_joint(hub, ball, Iso::IDENTITY, JointKind::rod(1.0)).expect("rod");
        run(&mut sim, 120);
        let len = sim.joint_position(j).expect("length");
        c.check((len - 1.0).abs() < 0.01, || format!("a rod from one point: length {} want 1", len));
    }
    // a rope that held a load and then has its anchors put together pulls nothing: the load falls freely
    {
        let mut sim = Simulation::new();
        let hub = anchor(&mut sim, Vec3::ZERO);
        let at = Vec3::new(0.0, -1.0, 0.0);
        let load = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(at)).with_damping(0.0, 0.0).sleepless()).expect("load");
        sim.add_joint(hub, load, Iso::at(at), JointKind::Distance { min: 0.0, max: 1.0, spring: None }).expect("rope");
        run(&mut sim, 120);
        let hung = sim.state(load).expect("load").center.y;
        sim.set_iso(load, Iso::at(at));
        sim.set_velocity(load, Vec3::ZERO, Vec3::ZERO);
        sim.step(DT);
        let (v, fall) = (sim.velocity(load).expect("v").0, SimParams::default().gravity * DT);
        c.check((hung + 2.0).abs() < 0.01 && (v - fall).length() < 1e-9, || format!("a slack rope with its anchors together: hung at {}, then {:?} after a step, free fall {:?}", hung, v, fall));
    }
    // a slider whose motor is taken away while it still drives the car coasts instead of being driven on by the
    // motor's old impulse
    {
        let mut sim = Simulation::with_params(no_gravity());
        let rail = anchor(&mut sim, Vec3::ZERO);
        let car = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.2)), Iso::IDENTITY).with_damping(0.0, 0.0).sleepless()).expect("car");
        let kind = |m: Option<Motor>| JointKind::Slider { limits: None, motor: m, spring: None };
        let j = sim.add_joint(rail, car, Iso::IDENTITY, kind(Some(Motor { speed: 2.0, max_force: 100.0 }))).expect("slider");
        sim.step(DT);
        let v0 = sim.velocity(car).expect("v").0.x;
        sim.set_joint(j, kind(None), false);
        run(&mut sim, 60);
        let v1 = sim.velocity(car).expect("v").0.x;
        c.check(v0 > 0.0 && v0 < 2.0 && (v1 - v0).abs() < 1e-6, || format!("a slider without its motor: {} m/s (still below 2) then {} m/s", v0, v1));
    }
    // a body welded to a slow kinematic carrier stays awake and goes along
    {
        let mut sim = Simulation::new();
        let carrier = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::new(1.0, 0.1, 1.0)), Iso::at(Vec3::new(0.0, 3.0, 0.0))).with_velocity(Vec3::new(0.01, 0.0, 0.0))).expect("carrier");
        let load = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.2)), Iso::at(Vec3::new(0.0, 2.5, 0.0)))).expect("load");
        sim.add_joint(carrier, load, Iso::at(Vec3::new(0.0, 2.8, 0.0)), JointKind::Fixed).expect("weld");
        run(&mut sim, 300);
        let (pc, pl) = (sim.state(carrier).expect("c").center, sim.state(load).expect("l").center);
        c.check(sim.is_awake(load) && ((pl.x - pc.x).abs() < 0.002), || format!("the welded load: awake {} at {:?}, the carrier at {:?}", sim.is_awake(load), pl, pc));
    }
    // a cone whose axes start against each other turns b back into it
    {
        let mut sim = Simulation::with_params(no_gravity());
        let top = anchor(&mut sim, Vec3::ZERO);
        let limb = sim.add(RigidBody::dynamic(Shape::capsule(0.3, 0.05), Iso::IDENTITY).sleepless()).expect("limb");
        let joint = physics::Joint {
            a: top,
            b: limb,
            frame_a: Iso::IDENTITY,
            frame_b: Iso::new(Vec3::ZERO, Quat::from_axis_angle(Vec3::Z, PI)),
            kind: JointKind::Ball { cone: Some(0.5), twist: None },
            collide_connected: false,
            user: 0,
        };
        let j = sim.add_joint_local(joint).expect("ball");
        run(&mut sim, 240);
        let (_, swing, _) = joint_error(&sim, j);
        c.check(swing < 0.5 + 3.0f64.to_radians(), || format!("antiparallel axes: swing {} after 4 s, cone 0.5", swing));
    }
    // what is refused
    {
        let mut sim = Simulation::new();
        let a = anchor(&mut sim, Vec3::ZERO);
        let b = anchor(&mut sim, Vec3::X);
        let d = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::IDENTITY)).expect("d");
        c.check(sim.add_joint(a, b, Iso::IDENTITY, JointKind::BALL) == Err(JointError::NothingMoves), || "a joint between two static bodies".into());
        c.check(sim.add_joint(d, d, Iso::IDENTITY, JointKind::BALL) == Err(JointError::SameBody), || "a joint of a body with itself".into());
        c.check(sim.add_joint(a, d, Iso::IDENTITY, JointKind::Hinge { limits: Some((1.0, 0.0)), motor: None, spring: None }) == Err(JointError::Range), || "limits the wrong way round".into());
        c.check(sim.add_joint(a, d, Iso::IDENTITY, JointKind::Distance { min: f64::NAN, max: 1.0, spring: None }) == Err(JointError::NotFinite), || "a NaN length".into());
        c.check(sim.add_joint(a, d, Iso::IDENTITY, JointKind::Slider { limits: Some((f64::INFINITY, f64::INFINITY)), motor: None, spring: None }) == Err(JointError::Range), || "limits no position meets".into());
        c.check(sim.add_joint(a, d, Iso::new(Vec3::ZERO, Quat::new(f64::INFINITY, 0.0, 0.0, 1.0)), JointKind::BALL) == Err(JointError::NotFinite), || "a frame with an infinite rotation".into());
        c.check(sim.add_joint(a, d, Iso::IDENTITY, JointKind::Hinge { limits: None, motor: None, spring: Some(Spring { target: 0.0, hertz: 1e200, damping_ratio: 1.0 }) }).is_ok(), || "a very stiff spring was refused".into());
        let j = sim.add_joint(a, d, Iso::IDENTITY, JointKind::BALL).expect("joint");
        c.check(sim.remove(d) && sim.joint(j).is_none(), || "removing a body left its joint".into());
    }
}

// ---------------------------------------------------------------- ccd

pub fn ccd_tests(c: &mut Ctx) {
    // two balls 1.5 m apart that each move 1 m towards the other in a step find each other
    {
        let mut sim = Simulation::with_params(no_gravity());
        let a = sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::at(Vec3::new(-1.75, 0.0, 0.0))).with_velocity(Vec3::new(60.0, 0.0, 0.0))).expect("a");
        let b = sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::at(Vec3::new(1.75, 0.0, 0.0))).with_velocity(Vec3::new(-60.0, 0.0, 0.0))).expect("b");
        let mut closest = f64::INFINITY;
        for _ in 0..4 {
            sim.step(DT);
            closest = closest.min((sim.state(b).expect("b").center - sim.state(a).expect("a").center).length());
        }
        c.check(closest > 2.0 - 0.01, || format!("two balls racing at each other came {} m close (radii 1 + 1)", closest));
    }
    // a bullet across the path of a moving body: swept with the motion relative to it, it stops where they meet
    {
        let mut sim = Simulation::with_params(no_gravity());
        let shot = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(Vec3::new(-5.0, 0.0, 0.0))).with_velocity(Vec3::new(600.0, 0.0, 0.0)).bullet()).expect("shot");
        sim.add(RigidBody::dynamic(Shape::sphere(0.5), Iso::at(Vec3::new(0.0, -5.0, 0.0))).with_velocity(Vec3::new(0.0, 600.0, 0.0))).expect("target");
        sim.step(DT);
        let x = sim.state(shot).expect("shot").center.x;
        c.check(x < 0.0 && x > -1.0, || format!("the bullet across a moving body ended at x {} (they meet at about -0.42)", x));
    }
    // a bullet at a sleeping box beyond the reach of its contacts stops at it, and the box wakes and moves
    {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let target = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.3)), Iso::at(Vec3::new(0.0, 0.3, 0.0)))).expect("box");
        run(&mut sim, 120);
        let asleep = !sim.is_awake(target);
        let shot = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(Vec3::new(-10.0, 0.3, 0.0))).with_velocity(Vec3::new(1200.0, 0.0, 0.0)).bullet()).expect("shot");
        sim.step(DT);
        let x = sim.state(shot).expect("shot").center.x;
        run(&mut sim, 10);
        let moved = sim.state(target).expect("box").center.x;
        c.check(asleep && x < -0.39 && x > -1.0 && moved > 0.01, || format!("a bullet at a sleeping box: asleep {}, the shot at x {} (the box from -0.3), the box then at {}", asleep, x, moved));
    }
    // a bullet and a kinematic box racing at each other: the box carries it on, the two do not pass
    {
        let mut sim = Simulation::with_params(no_gravity());
        let shot = sim.add(RigidBody::dynamic(Shape::sphere(0.1), Iso::at(Vec3::new(-5.0, 0.0, 0.0))).with_velocity(Vec3::new(600.0, 0.0, 0.0)).bullet()).expect("shot");
        let wall = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(5.0, 0.0, 0.0))).with_velocity(Vec3::new(-600.0, 0.0, 0.0))).expect("wall");
        let mut ahead = f64::INFINITY;
        for _ in 0..3 {
            sim.step(DT);
            ahead = ahead.min(sim.state(wall).expect("wall").center.x - sim.state(shot).expect("shot").center.x);
        }
        c.check(ahead > 0.6 - 0.01, || format!("a bullet and a kinematic box racing at each other: the box {} m ahead of it (0.6 touching)", ahead));
    }
    // a pellet at 500 m/s does not pass a 1 mm wall, a box or a mesh; without the sweep it does
    for (mesh, continuous) in [(false, true), (true, true), (false, false)] {
        let mut sim = Simulation::with_params(SimParams { continuous, ..no_gravity() });
        if mesh {
            let f = |x: f64, y: f64, z: f64| [x as f32, y as f32, z as f32];
            let wall = physics::TriMesh::new(&[f(10.0, -5.0, -5.0), f(10.0, 5.0, -5.0), f(10.0, 5.0, 5.0), f(10.0, -5.0, 5.0)], &[0, 1, 2, 0, 2, 3]).expect("wall");
            sim.add(RigidBody::fixed(Shape::mesh(wall), Iso::IDENTITY)).expect("wall");
        } else {
            sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(0.0005, 5.0, 5.0)), Iso::at(Vec3::new(10.0, 0.0, 0.0)))).expect("wall");
        }
        let pellet = sim.add(RigidBody::dynamic(Shape::sphere(0.01), Iso::at(Vec3::new(0.0, 0.3, -0.2))).with_velocity(Vec3::new(500.0, 3.0, 1.0))).expect("pellet");
        run(&mut sim, 12);
        let x = sim.state(pellet).expect("s").center.x;
        if continuous {
            c.check(x < 10.0, || format!("pellet through the wall (mesh {}): x {}", mesh, x));
        } else {
            c.check(x > 10.0, || format!("without the sweep the pellet should pass: x {}", x));
        }
    }
    // a bullet hits a light box (a body that moves) and pushes it; a bullet that is not one passes
    for bullet in [true, false] {
        let mut sim = Simulation::with_params(no_gravity());
        let target = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.05, 0.5, 0.5)), Iso::at(Vec3::new(5.0, 0.0, 0.0))).with_density(100.0)).expect("target");
        let mut d = RigidBody::dynamic(Shape::sphere(0.01), Iso::IDENTITY).with_velocity(Vec3::new(400.0, 0.0, 0.0)).with_density(10_000.0);
        if bullet {
            d = d.bullet();
        }
        let shot = sim.add(d).expect("shot");
        run(&mut sim, 6);
        let (vt, xs) = (sim.velocity(target).expect("v").0.x, sim.state(shot).expect("s").center.x);
        if bullet {
            c.check(vt > 0.1 && xs < 5.1, || format!("bullet: the target moves {} m/s, the shot at {}", vt, xs));
        } else {
            c.check(vt == 0.0 && xs > 5.1, || format!("not a bullet: target {} m/s, shot at {}", vt, xs));
        }
    }
    // things thrown at the ground at up to 150 m/s, spinning: no centre passes through it (a corner may
    // turn into it for a step, faster than any contact can follow), and once they lie still they lie on it
    {
        let mut rng = Rng::new(21);
        let mut sim = Simulation::new();
        let ground = scene::flat(20, 1.0, 0.0).build();
        sim.add(RigidBody::fixed(Shape::mesh(ground), Iso::at(Vec3::new(-10.0, 0.0, -10.0)))).expect("ground");
        let mut ids = Vec::new();
        for k in 0..c.n(200) {
            let s = random_convex(&mut rng, k % 4, 0.3);
            let v = Vec3::new(rng.range(-30.0, 30.0), -rng.range(50.0, 150.0), rng.range(-30.0, 30.0));
            let at = Iso::new(Vec3::new(rng.range(-5.0, 5.0), rng.range(3.0, 20.0), rng.range(-5.0, 5.0)), rng.quat());
            ids.push((sim.add(RigidBody::dynamic(s.clone(), at).with_velocity(v).with_angular_velocity(rng.vec(-30.0, 30.0))).expect("body"), s));
        }
        let mut worst = f64::INFINITY;
        for _ in 0..120 {
            sim.step(DT);
            for (b, _) in &ids {
                let st = sim.state(*b).expect("s");
                // inside the ground's square
                if st.center.x.abs() < 9.5 && st.center.z.abs() < 9.5 {
                    worst = worst.min(st.center.y);
                }
            }
        }
        c.check(worst > 0.0, || format!("thrown at the ground: a centre went {} m into it", -worst));
        run(&mut sim, 240);
        let mut low = 0.0f64;
        for (b, s) in &ids {
            let st = sim.state(*b).expect("s");
            if st.center.x.abs() < 9.5 && st.center.z.abs() < 9.5 {
                low = low.min(s.aabb(&st.iso).min.y);
            }
        }
        c.check(low > -0.02, || format!("thrown at the ground: after 4 s one lies {} m in it", -low));
    }
}

// ---------------------------------------------------------------- events

pub fn event_tests(c: &mut Ctx) {
    // a static sensor teleported off a sleeping body ends the overlap, back onto it begins it again
    {
        let mut sim = Simulation::new();
        floor(&mut sim, 20.0);
        let crate_box = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.3)), Iso::at(Vec3::new(0.0, 0.3, 0.0)))).expect("box");
        let zone = sim.add(RigidBody::fixed(Shape::cuboid(Vec3::splat(1.0)), Iso::at(Vec3::new(0.0, 0.5, 0.0))).sensor()).expect("sensor");
        run(&mut sim, 120);
        let pair = |e: &ContactEvent| matches!(e, ContactEvent::SensorEnd { sensor, other } | ContactEvent::SensorBegin { sensor, other } if *sensor == zone && *other == crate_box);
        c.check(!sim.is_awake(crate_box), || "the box in the sensor did not fall asleep".into());
        sim.set_iso(zone, Iso::at(Vec3::new(10.0, 0.5, 0.0)));
        sim.step(DT);
        let ended = sim.events().iter().filter(|e| pair(e) && matches!(e, ContactEvent::SensorEnd { .. })).count();
        sim.set_iso(zone, Iso::at(Vec3::new(0.0, 0.5, 0.0)));
        sim.step(DT);
        let began = sim.events().iter().filter(|e| pair(e) && matches!(e, ContactEvent::SensorBegin { .. })).count();
        c.check(ended == 1 && began == 1, || format!("a teleported sensor: {} ends, {} begins", ended, began));
    }
    // a floor that no longer collides with a sleeping body: its contact ends
    {
        let mut sim = Simulation::new();
        let fl = floor(&mut sim, 20.0);
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.3), Iso::at(Vec3::new(0.0, 0.3, 0.0)))).expect("ball");
        run(&mut sim, 120);
        sim.set_layers(fl, 1, 0);
        sim.step(DT);
        let ended = sim.events().iter().any(|e| matches!(e, ContactEvent::End { .. }));
        let left = sim.manifolds().filter(|m| m.a == ball || m.b == ball).count();
        c.check(ended && left == 0, || format!("a floor relayered: end {} manifolds {}", ended, left));
    }
    // one Begin at the impact with its speed, nothing while resting, End when lifted off
    {
        let mut sim = Simulation::new();
        let fl = floor(&mut sim, 20.0);
        let r = 0.2;
        let h = 1.25;
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(r), Iso::at(Vec3::new(0.0, h + r, 0.0)))).expect("ball");
        let mut begins = Vec::new();
        let mut ends = 0;
        for _ in 0..180 {
            sim.step(DT);
            for e in sim.events() {
                match e {
                    ContactEvent::Begin { a, b, speed, normal, .. } => begins.push((*a, *b, *speed, *normal)),
                    ContactEvent::End { .. } => ends += 1,
                    _ => {}
                }
            }
        }
        let want = sqrt(2.0 * 9.81 * h);
        c.check(begins.len() == 1 && ends == 0, || format!("events while falling and resting: {:?}, {} ends", begins, ends));
        if let Some((a, b, speed, n)) = begins.first() {
            c.check(rel(*speed, want) < 0.05, || format!("impact speed {} want {}", speed, want));
            c.check((*a == ball && *b == fl && n.y < -0.99) || (*a == fl && *b == ball && n.y > 0.99), || format!("begin between {:?} and {:?}, normal {:?}", a, b, n));
        }
        sim.set_iso(ball, Iso::at(Vec3::new(0.0, 3.0, 0.0)));
        sim.step(DT);
        let n_end = sim.events().iter().filter(|e| matches!(e, ContactEvent::End { .. })).count();
        c.check(n_end == 1, || format!("lifting off gave {:?}", sim.events()));
        // removing a body that touches another ends the contact
        run(&mut sim, 90);
        sim.remove(ball);
        sim.step(DT);
        c.check(sim.events().iter().any(|e| matches!(e, ContactEvent::End { .. })), || "removing a resting body gave no End".into());
    }
    // a sensor: entered and left, the body passes unhindered; a kinematic body in a sensor too
    {
        let mut sim = Simulation::new();
        let sensor = sim.add(RigidBody::fixed(Shape::cuboid(Vec3::splat(1.0)), Iso::at(Vec3::new(0.0, 5.0, 0.0))).sensor()).expect("sensor");
        let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.2), Iso::at(Vec3::new(0.0, 10.0, 0.0))).with_damping(0.0, 0.0)).expect("ball");
        let (mut entered, mut left) = (0, 0);
        for _ in 0..90 {
            sim.step(DT);
            for e in sim.events() {
                match e {
                    ContactEvent::SensorBegin { sensor: s, other } if *s == sensor && *other == ball => entered += 1,
                    ContactEvent::SensorEnd { sensor: s, other } if *s == sensor && *other == ball => left += 1,
                    _ => {}
                }
            }
        }
        c.check(entered == 1 && left == 1, || format!("sensor: entered {} left {}", entered, left));
        let k = 90.0 * 4.0;
        let hh = DT / 4.0;
        let want = -9.81 * hh * k;
        c.check((sim.velocity(ball).expect("v").0.y - want).abs() < 1e-9, || "the sensor slowed the ball".into());
        let door = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::splat(0.2)), Iso::at(Vec3::new(-3.0, 5.0, 0.0))).with_velocity(Vec3::new(6.0, 0.0, 0.0))).expect("kinematic");
        let mut seen = Vec::new();
        for _ in 0..90 {
            sim.step(DT);
            for e in sim.events() {
                match e {
                    ContactEvent::SensorBegin { sensor: s, other } if *s == sensor && *other == door => seen.push(true),
                    ContactEvent::SensorEnd { sensor: s, other } if *s == sensor && *other == door => seen.push(false),
                    _ => {}
                }
            }
        }
        c.check(seen == [true, false], || format!("a kinematic body through the sensor: {:?} (begin true, end false)", seen));
    }
}

// ---------------------------------------------------------------- dynfuzz

/// the kinetic and potential energy of the dynamic bodies (potential against y = 0)
fn energy(sim: &Simulation) -> f64 {
    let mut e = 0.0;
    for (id, _) in sim.world().bodies() {
        let s = sim.state(id).expect("state");
        if s.kind != BodyKind::Dynamic {
            continue;
        }
        let mp = sim.mass(id).expect("mass");
        let iw = mp.inertia.rotated(s.iso.rot);
        e += 0.5 * mp.mass * s.linear_velocity.length_sq() + 0.5 * s.angular_velocity.dot(iw.mul_vec(s.angular_velocity)) + mp.mass * 9.81 * s.center.y;
    }
    e
}

/// a random scene: a floor (a box or a mesh), bodies of every shape and density with random velocities, joints
/// of every kind between some of them, their limits around where they start -> also the power its motors can
/// give (W)
fn random_scene(rng: &mut Rng, mesh: bool, n: usize) -> (Simulation, Vec<(BodyId, Shape)>, f64) {
    let mut sim = Simulation::new();
    if mesh {
        let ground = scene::flat(16, 1.0, 0.0).build();
        sim.add(RigidBody::fixed(Shape::mesh(ground), Iso::at(Vec3::new(-8.0, 0.0, -8.0)))).expect("ground");
    } else {
        floor(&mut sim, 8.0);
    }
    let mut ids = Vec::new();
    for k in 0..n {
        let scale = rng.range(0.2, 0.8);
        let s = random_convex(rng, k % 4, scale);
        // where nothing is yet (an overlap at the start is pushed apart, which gives energy)
        let mut at = Iso::IDENTITY;
        for _ in 0..50 {
            at = Iso::new(Vec3::new(rng.range(-5.0, 5.0), rng.range(1.0, 8.0), rng.range(-5.0, 5.0)), rng.quat());
            let mut hits = Vec::new();
            sim.world().overlap(&s, &at, &physics::Filter::ALL, &mut hits);
            if hits.is_empty() {
                break;
            }
        }
        let d = RigidBody::dynamic(s.clone(), at)
            .with_density(rng.range(50.0, 20_000.0))
            .with_velocity(rng.vec(-15.0, 15.0))
            .with_angular_velocity(rng.vec(-20.0, 20.0))
            .with_friction(rng.range(0.0, 1.2))
            .with_restitution(if rng.below(3) == 0 { rng.range(0.0, 1.0) } else { 0.0 });
        ids.push((sim.add(d).expect("body"), s));
    }
    let mut power = 0.0;
    for _ in 0..n / 4 {
        let a = ids[rng.below(ids.len())].0;
        let b = ids[rng.below(ids.len())].0;
        if a == b {
            continue;
        }
        let pa = sim.state(a).expect("a").center;
        let pb = sim.state(b).expect("b").center;
        let frame = Iso::new((pa + pb) * 0.5, rng.quat());
        let lim = |rng: &mut Rng| (-rng.range(0.0, 2.0), rng.range(0.0, 2.0));
        let kind = match rng.below(5) {
            0 => JointKind::Ball { cone: if rng.below(2) == 0 { Some(rng.range(0.1, 1.5)) } else { None }, twist: if rng.below(2) == 0 { Some(lim(rng)) } else { None } },
            1 => JointKind::Hinge { limits: if rng.below(2) == 0 { Some(lim(rng)) } else { None }, motor: if rng.below(3) == 0 { Some(Motor { speed: rng.range(-5.0, 5.0), max_force: rng.range(0.0, 500.0) }) } else { None }, spring: None },
            2 => JointKind::Fixed,
            3 => JointKind::Slider { limits: Some(lim(rng)), motor: None, spring: None },
            _ => {
                let d = (pb - pa).length();
                JointKind::Distance { min: d * 0.5, max: d * 1.2, spring: None }
            }
        };
        if let JointKind::Hinge { motor: Some(m), .. } = kind {
            power += m.max_force * m.speed.abs();
        }
        let _ = if let JointKind::Distance { .. } = kind { sim.add_distance_joint(a, b, pa, pb, kind) } else { sim.add_joint(a, b, frame, kind) };
    }
    (sim, ids, power)
}

pub fn fuzz_tests(c: &mut Ctx) {
    let mut rng = Rng::new(2027);
    for scene_k in 0..c.n(24) {
        let mesh = scene_k % 2 == 1;
        let (mut sim, ids, power) = random_scene(&mut rng, mesh, 50);
        let e0 = energy(&sim);
        let mut e_max = e0;
        let mut bad = None;
        // a body that went over the edge may be pulled under the floor by a joint
        let mut left = vec![false; ids.len()];
        for step in 0..360 {
            sim.step(DT);
            e_max = e_max.max(energy(&sim));
            for (k, (b, _)) in ids.iter().enumerate() {
                let st = sim.state(*b).expect("state");
                let c0 = st.center;
                // back above the floor inside its square, it counts again
                if c0.x.abs() > 7.5 || c0.z.abs() > 7.5 {
                    left[k] = true;
                } else if c0.y >= 0.0 {
                    left[k] = false;
                }
                if !(c0.is_finite() && st.linear_velocity.is_finite() && st.angular_velocity.is_finite() && st.iso.rot.is_finite()) {
                    bad = Some((step, *b, "not finite"));
                } else if !left[k] && c0.y < 0.0 {
                    bad = Some((step, *b, "its centre in or under the floor"));
                }
            }
            if bad.is_some() {
                break;
            }
        }
        c.check(bad.is_none(), || format!("scene {} (mesh {}): {:?}", scene_k, mesh, bad));
        // nothing gains energy but through the motors (and a little through the softness of the joints)
        let motors = power * 360.0 * DT;
        c.check(e_max <= e0 + e0.abs() * 0.02 + motors + 1.0, || format!("scene {}: energy {} up from {} (motors up to {})", scene_k, e_max, e0, motors));
    }
    // the same scene twice gives the same bits; a client that builds it in the same order agrees
    let run_bits = |seed: u64| {
        let mut rng = Rng::new(seed);
        let (mut sim, ids, _) = random_scene(&mut rng, true, 40);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for _ in 0..240 {
            sim.step(DT);
            for (b, _) in &ids {
                let s = sim.state(*b).expect("s");
                for v in [s.center.x, s.center.y, s.center.z, s.iso.rot.x, s.iso.rot.w, s.linear_velocity.y] {
                    for byte in v.to_bits().to_le_bytes() {
                        h = (h ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
            }
        }
        h
    };
    let (h1, h2) = (run_bits(99), run_bits(99));
    c.check(h1 == h2, || format!("the same scene gave {:x} and {:x}", h1, h2));
    // a clone is a snapshot: it steps on to the same bits (rollback)
    {
        let mut rng = Rng::new(7);
        let (mut sim, ids, _) = random_scene(&mut rng, true, 40);
        run(&mut sim, 100);
        let mut copy = sim.clone();
        run(&mut sim, 150);
        run(&mut copy, 150);
        let same = ids.iter().all(|(b, _)| {
            let (x, y) = (sim.state(*b).expect("s"), copy.state(*b).expect("s"));
            x.center.x.to_bits() == y.center.x.to_bits() && x.iso.rot.w.to_bits() == y.iso.rot.w.to_bits() && x.angular_velocity.z.to_bits() == y.angular_velocity.z.to_bits() && x.awake == y.awake
        });
        c.check(same && sim.events() == copy.events(), || "a clone of the simulation went another way".into());
    }
    // bodies added and removed while it runs
    {
        let mut rng = Rng::new(3);
        let (mut sim, mut ids, _) = random_scene(&mut rng, false, 30);
        for step in 0..300 {
            if step % 7 == 0 && !ids.is_empty() {
                let k = rng.below(ids.len());
                let (b, _) = ids.swap_remove(k);
                c.check(sim.remove(b), || "remove failed".into());
                c.check(!sim.remove(b) && sim.state(b).is_none(), || "a removed id is alive".into());
            }
            if step % 5 == 0 {
                let s = random_convex(&mut rng, step % 4, 0.4);
                let id = sim.add(RigidBody::dynamic(s.clone(), Iso::at(Vec3::new(rng.range(-4.0, 4.0), 6.0, rng.range(-4.0, 4.0))))).expect("add");
                ids.push((id, s));
            }
            sim.step(DT);
        }
        // (thrown at up to 15 m/s, some fly off the floor)
        c.check(ids.iter().all(|(b, _)| sim.state(*b).is_some_and(|s| s.center.is_finite())), || "after adding and removing: a body lost".into());
        c.check(sim.add(RigidBody::dynamic(Shape::mesh(scene::flat(1, 1.0, 0.0).build()), Iso::IDENTITY)).err() == Some(physics::BodyError::DynamicMesh), || "a dynamic mesh was accepted".into());
        c.check(sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::at(Vec3::new(f64::NAN, 0.0, 0.0)))).err() == Some(physics::BodyError::NotFinite), || "a NaN position was accepted".into());
        c.check(sim.add(RigidBody::dynamic(Shape::sphere(1.0), Iso::IDENTITY).with_density(-1.0)).err() == Some(physics::BodyError::Mass), || "a negative density was accepted".into());
    }
}

// ---------------------------------------------------------------- checksum and measurement

/// the scene of the dynamics checksum and of `--bench dynamics`: the test map, a heap of every shape with
/// rolling resistance, a pyramid, ragdolls, a chain, a motor, a slider, a rope, a kinematic platform, a sensor
pub fn showcase(rng: &mut Rng, heap: usize, pyramid: usize, dolls: usize) -> (Simulation, Vec<BodyId>, Vec<JointId>, BodyId) {
    let mut sim = Simulation::new();
    let map = scene::map(40, 30).build();
    sim.add(RigidBody::fixed(Shape::mesh(map), Iso::at(Vec3::new(-20.0, 0.0, -20.0)))).expect("map");
    let mut bodies = Vec::new();
    let mut joints = Vec::new();
    for k in 0..heap {
        let scale = rng.range(0.15, 0.5);
        let s = random_convex(rng, k % 4, scale);
        let at = Iso::new(Vec3::new(rng.range(-12.0, 12.0), 4.0 + rng.range(0.0, 12.0), rng.range(-12.0, 12.0)), rng.quat());
        let d = RigidBody::dynamic(s, at).with_density(rng.range(200.0, 3000.0)).with_velocity(rng.vec(-3.0, 3.0)).with_rolling_resistance(0.03);
        bodies.push(sim.add(d).expect("heap"));
    }
    // a pyramid on a platform of its own above the map
    sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(8.0, 0.5, 3.0)), Iso::at(Vec3::new(0.0, 6.5, 30.0)))).expect("plinth");
    for row in 0..pyramid {
        for k in 0..pyramid - row {
            let x = (k as f64 - (pyramid - row - 1) as f64 * 0.5) * 0.81;
            bodies.push(sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.4)), Iso::at(Vec3::new(x, 7.4 + row as f64 * 0.8, 30.0)))).expect("box"));
        }
    }
    for d in 0..dolls {
        let at = Iso::new(Vec3::new(-8.0 + d as f64 * 3.0, 9.0 + rng.range(0.0, 3.0), -8.0), rng.quat());
        joints.extend(ragdoll(&mut sim, at).into_iter().map(|j| j.0));
    }
    // a chain from a crane
    let top = anchor(&mut sim, Vec3::new(12.0, 14.0, 12.0));
    let side = Quat::from_axis_angle(Vec3::Z, PI * 0.5);
    let mut prev = top;
    for k in 0..12 {
        let x = 12.25 + 0.5 * k as f64;
        let link = sim.add(RigidBody::dynamic(Shape::capsule(0.2, 0.05), Iso::new(Vec3::new(x, 14.0, 12.0), side))).expect("link");
        joints.push(sim.add_joint(prev, link, Iso::at(Vec3::new(x - 0.25, 14.0, 12.0)), JointKind::BALL).expect("chain"));
        bodies.push(link);
        prev = link;
    }
    // a wheel driven by a motor, a car on a slider, a ball on a rope
    let hub = anchor(&mut sim, Vec3::new(-12.0, 8.0, 12.0));
    let wheel = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.1, 1.0, 1.0)), Iso::at(Vec3::new(-12.0, 8.0, 12.0)))).expect("wheel");
    joints.push(sim.add_joint(hub, wheel, Iso::at(Vec3::new(-12.0, 8.0, 12.0)), JointKind::Hinge { limits: None, motor: Some(Motor { speed: 2.0, max_force: 2000.0 }), spring: None }).expect("motor"));
    let rail = anchor(&mut sim, Vec3::new(-6.0, 8.0, 12.0));
    let car = sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.3)), Iso::at(Vec3::new(-6.0, 8.0, 12.0)))).expect("car");
    joints.push(sim.add_joint(rail, car, Iso::new(Vec3::new(-6.0, 8.0, 12.0), Quat::from_axis_angle(Vec3::Z, -0.5)), JointKind::Slider { limits: Some((-2.0, 2.0)), motor: None, spring: Some(Spring { target: 0.0, hertz: 0.5, damping_ratio: 0.1 }) }).expect("slider"));
    let hook = anchor(&mut sim, Vec3::new(0.0, 12.0, 12.0));
    let ball = sim.add(RigidBody::dynamic(Shape::sphere(0.3), Iso::at(Vec3::new(2.0, 12.0, 12.0))).with_velocity(Vec3::new(0.0, 0.0, 4.0))).expect("ball");
    joints.push(sim.add_distance_joint(hook, ball, Vec3::new(0.0, 12.0, 12.0), Vec3::new(2.0, 12.0, 12.0), JointKind::rope(2.5)).expect("rope"));
    bodies.extend([wheel, car, ball]);
    // a platform that the step drives and a trigger
    let platform = sim.add(RigidBody::kinematic(Shape::cuboid(Vec3::new(1.5, 0.2, 1.5)), Iso::at(Vec3::new(6.0, 5.0, -10.0)))).expect("platform");
    for k in 0..4 {
        bodies.push(sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.25)), Iso::at(Vec3::new(5.4 + 0.6 * k as f64, 5.5, -10.0)))).expect("cargo"));
    }
    sim.add(RigidBody::fixed(Shape::cuboid(Vec3::new(4.0, 2.0, 4.0)), Iso::at(Vec3::new(0.0, 3.0, 0.0))).sensor()).expect("trigger");
    (sim, bodies, joints, platform)
}

/// the dynamics checksum: every body's pose and velocities, the events and the joints of the showcase over
/// 600 steps -> (hash, values)
pub fn checksum(mut put: impl FnMut(u64)) {
    let mut rng = Rng::new(4711);
    let (mut sim, _, joints, platform) = showcase(&mut rng, 80, 6, 2);
    let f = |put: &mut dyn FnMut(u64), v: f64| put(v.to_bits());
    for step in 0..600 {
        let t = step as f64 * DT;
        let (s, cc) = math::sincos(t * 0.8);
        sim.move_kinematic(platform, Iso::new(Vec3::new(6.0 + 3.0 * s, 5.0 + 1.5 * (1.0 - cc), -10.0), Quat::from_axis_angle(Vec3::Y, t * 0.3)), DT);
        sim.step(DT);
        let ids: Vec<BodyId> = sim.world().bodies().map(|(id, _)| id).collect();
        for id in ids {
            let st = sim.state(id).expect("state");
            for v in [st.center.x, st.center.y, st.center.z, st.iso.pos.x, st.iso.pos.y, st.iso.pos.z, st.iso.rot.x, st.iso.rot.y, st.iso.rot.z, st.iso.rot.w] {
                f(&mut put, v);
            }
            for v in [st.linear_velocity, st.angular_velocity] {
                f(&mut put, v.x);
                f(&mut put, v.y);
                f(&mut put, v.z);
            }
            put(st.awake as u64);
        }
        for e in sim.events() {
            match e {
                ContactEvent::Begin { a, b, point, normal, speed } => {
                    put(1 | (a.index() as u64) << 8 | (b.index() as u64) << 32);
                    for v in [point.x, point.y, point.z, normal.x, normal.y, normal.z, *speed] {
                        f(&mut put, v);
                    }
                }
                ContactEvent::End { a, b } => put(2 | (a.index() as u64) << 8 | (b.index() as u64) << 32),
                ContactEvent::SensorBegin { sensor, other } => put(3 | (sensor.index() as u64) << 8 | (other.index() as u64) << 32),
                ContactEvent::SensorEnd { sensor, other } => put(4 | (sensor.index() as u64) << 8 | (other.index() as u64) << 32),
            }
        }
        if step % 10 == 0 {
            for j in &joints {
                f(&mut put, sim.joint_position(*j).expect("joint"));
                let (force, torque) = sim.joint_force(*j).expect("force");
                for v in [force.x, force.y, force.z, torque.x, torque.y, torque.z] {
                    f(&mut put, v);
                }
            }
            let st = sim.stats();
            for v in [st.awake, st.manifolds, st.points, st.pairs, st.sleeping_islands, st.swept] {
                put(v as u64);
            }
        }
    }
}

/// `--bench dynamics`: steps per second of the showcase at three sizes, and of a pyramid alone
pub fn bench(runs: usize) {
    use crate::out;
    let median = |mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        v[v.len() / 2]
    };
    for &(heap, pyramid, dolls) in &[(100, 8, 4), (300, 12, 8), (1000, 16, 16)] {
        let mut times = Vec::new();
        let mut last = physics::SimStats::default();
        let mut bodies = 0;
        for _ in 0..runs {
            let mut rng = Rng::new(4711);
            let (mut sim, _, _, _) = showcase(&mut rng, heap, pyramid, dolls);
            bodies = sim.len();
            // the first second: everything falls and collides (the hard part), then the rest of 5 s
            let t0 = plat::time_us();
            for _ in 0..60 {
                sim.step(DT);
            }
            let t1 = plat::time_us();
            for _ in 0..240 {
                sim.step(DT);
            }
            let t2 = plat::time_us();
            times.push(((t1 - t0) as f64 / 60.0, (t2 - t1) as f64 / 240.0));
            last = sim.stats();
        }
        let first = median(times.iter().map(|t| t.0).collect());
        let rest = median(times.iter().map(|t| t.1).collect());
        out(&format!(
            "RESULT dynamics bodies={} first_second_ms_per_step={:.3} next_4s_ms_per_step={:.3} awake_at_5s={} manifolds={} points={} joints={}\n",
            bodies, first / 1000.0, rest / 1000.0, last.awake, last.manifolds, last.points, last.joints
        ));
    }
    for &rows in &[10usize, 20] {
        let mut times = Vec::new();
        let mut boxes = 0;
        for _ in 0..runs {
            let mut sim = Simulation::new();
            floor(&mut sim, 40.0);
            for row in 0..rows {
                for k in 0..rows - row {
                    let x = (k as f64 - (rows - row - 1) as f64 * 0.5) * 1.02;
                    sim.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(x, 0.5 + row as f64, 0.0))).sleepless()).expect("box");
                }
            }
            boxes = sim.len() - 1;
            let t0 = plat::time_us();
            for _ in 0..120 {
                sim.step(DT);
            }
            times.push((plat::time_us() - t0) as f64 / 120.0);
        }
        out(&format!("RESULT dynamics pyramid boxes={} ms_per_step={:.3}\n", boxes, median(times) / 1000.0));
    }
}
