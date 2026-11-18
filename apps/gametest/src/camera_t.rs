// camera_t -- the group "camera": the basis against Quat::from_yaw_pitch, the limits, walking, the
// follow camera's distance, zoom and walls (a physics World), free flight; closed forms as references

use alloc::format;

use game::camera::{self, PITCH_LIMIT, basis, wrap_angle, yaw_pitch_of};
use game::{FirstPerson, Follow, FreeFly, Quat, Vec3, View};
use physics::{Filter, Iso, Shape, World as PhysicsWorld};

use crate::{Ctx, Rng};

pub fn run(c: &mut Ctx) {
    bases(c);
    first_person(c);
    follow(c);
    free_fly(c);
}

fn near(a: Vec3, b: Vec3, tol: f64) -> bool {
    (a - b).length() <= tol
}

fn bases(c: &mut Ctx) {
    let mut r = Rng::new(3);
    let (mut bad, mut worst) = (0, 0.0f64);
    for _ in 0..c.n(20_000) {
        let (y, p) = (r.range(-10.0, 10.0), r.range(-1.55, 1.55));
        let (f, rt, u) = basis(y, p);
        let q = Quat::from_yaw_pitch(y, p);
        let e = (f - q.rotate(Vec3::new(0.0, 0.0, -1.0))).length().max((rt - q.rotate(Vec3::X)).length()).max((u - q.rotate(Vec3::Y)).length());
        worst = worst.max(e);
        let ortho = f.dot(rt).abs() < 1e-15 && f.dot(u).abs() < 1e-15 && rt.dot(u).abs() < 1e-15;
        let unit = [f, rt, u].iter().all(|v| (v.length() - 1.0).abs() < 1e-15);
        let handed = near(f.cross(u), rt, 1e-15);
        if e > 1e-15 || !ortho || !unit || !handed {
            bad += 1;
        }
        // the angles of the direction back (the yaw wrapped)
        if let Some((y2, p2)) = yaw_pitch_of(f) {
            if (wrap_angle(y2 - y)).abs() > 1e-12 || (p2 - p).abs() > 1e-12 {
                bad += 1;
            }
        }
    }
    c.check(bad == 0, || format!("{bad} bases off, worst against the quaternion {worst:e}"));
    let (f, rt, u) = basis(0.0, 0.0);
    c.check(f == Vec3::new(0.0, 0.0, -1.0) && rt == Vec3::X && u == Vec3::Y, || "yaw 0, pitch 0: -Z, +X, +Y".into());
    let (f, _, _) = basis(core::f64::consts::FRAC_PI_2, 0.0);
    c.check(near(f, Vec3::new(-1.0, 0.0, 0.0), 1e-15), || format!("yaw +90 degrees looks along -X (to the left): {f:?}"));
    let pi = core::f64::consts::PI;
    c.check(wrap_angle(pi) == pi && wrap_angle(-pi) == pi && (wrap_angle(3.0 * pi).abs() - pi).abs() < 1e-12 && wrap_angle(0.25) == 0.25, || "wrap_angle".into());
    c.check(wrap_angle(f64::NAN) == 0.0 && (wrap_angle(1e6) - (1e6 - 159_155.0 * core::f64::consts::TAU)).abs() < 1e-6, || "wrap_angle of NaN and large values".into());
    let v = View::look_at(Vec3::new(0.0, 2.0, 5.0), Vec3::ZERO, Vec3::Y).expect("view");
    c.check(near(v.forward, Vec3::new(0.0, -2.0, -5.0).normalize_or_zero(), 1e-15) && v.forward.dot(v.up).abs() < 1e-15 && v.right().y.abs() < 1e-15,
            || "View::look_at".into());
    c.check(View::look_at(Vec3::ZERO, Vec3::ZERO, Vec3::Y).is_none() && View::look_at(Vec3::ZERO, Vec3::Y, Vec3::Y).is_none(), || "look_at refuses".into());
    c.check(near(camera::walk_dir(0.0, 1.0, 0.0), Vec3::new(0.0, 0.0, -1.0), 0.0), || "walk_dir".into());
}

fn first_person(c: &mut Ctx) {
    let mut fp = FirstPerson::new(Vec3::new(0.0, 1.7, 0.0));
    // the mouse to the right turns right (towards +X), down looks down
    fp.look(0.1, 0.0);
    c.check(fp.forward().x > 0.0 && fp.yaw == -0.1, || format!("right: {:?}", fp.forward()));
    fp.look(0.0, 0.2);
    c.check(fp.forward().y < 0.0 && fp.pitch == -0.2, || "down".into());
    // the limit holds, the yaw wraps
    fp.look(0.0, 10.0);
    c.check(fp.pitch == -PITCH_LIMIT, || format!("pitch held at -89 degrees: {}", fp.pitch));
    fp.look(0.0, -100.0);
    c.check(fp.pitch == PITCH_LIMIT, || "and at +89".into());
    for _ in 0..1000 {
        fp.look(0.37, 0.0);
    }
    c.check(fp.yaw > -core::f64::consts::PI && fp.yaw <= core::f64::consts::PI, || format!("yaw kept in (-pi, pi]: {}", fp.yaw));
    fp.look(f64::NAN, 1.0);
    c.check(fp.yaw.is_finite() && fp.pitch == PITCH_LIMIT, || "NaN ignored".into());
    let v = fp.view();
    c.check(v.eye == fp.eye && near(v.forward, fp.forward(), 0.0) && (v.forward.length() - 1.0).abs() < 1e-15, || "view".into());
    c.check(near(fp.rotation().rotate(Vec3::new(0.0, 0.0, -1.0)), fp.forward(), 1e-15), || "rotation".into());
    // walking: along the yaw on the ground, at most 1
    let mut w = FirstPerson::new(Vec3::ZERO);
    w.pitch = 0.8;
    c.check(near(w.walk_dir(1.0, 0.0), Vec3::new(0.0, 0.0, -1.0), 1e-15) && near(w.walk_dir(0.0, 1.0), Vec3::X, 1e-15), || "walk at yaw 0".into());
    let d = w.walk_dir(1.0, 1.0);
    c.check((d.length() - 1.0).abs() < 1e-15 && d.y == 0.0, || format!("diagonal at most 1: {}", d.length()));
    c.check(near(w.walk_dir(0.5, 0.0), Vec3::new(0.0, 0.0, -0.5), 1e-15), || "half".into());
    w.yaw = core::f64::consts::FRAC_PI_2;
    c.check(near(w.walk_dir(1.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), 1e-15), || "walk at yaw 90 degrees".into());
}

fn follow(c: &mut Ctx) {
    let target = Vec3::new(10.0, 1.5, -4.0);
    let mut f = Follow::new(target, 6.0);
    let mut r = Rng::new(5);
    let mut bad = 0;
    for _ in 0..c.n(2000) {
        f.look(r.range(-0.5, 0.5), r.range(-0.5, 0.5));
        let v = f.view();
        let toward = (target - v.eye).normalize_or_zero();
        if ((v.eye - target).length() - 6.0).abs() > 1e-12 || !near(toward, v.forward, 1e-12) || f.pitch < f.pitch_min || f.pitch > f.pitch_max {
            bad += 1;
        }
    }
    c.check(bad == 0, || format!("{bad} orbits off"));
    c.check(near(f.eye(), f.view().eye, 0.0), || "eye".into());
    // zoom: 10 % a notch, kept in min ..= max
    let mut z = Follow::new(target, 10.0);
    z.zoom(1.0);
    c.check((z.distance - 9.0).abs() < 1e-12, || format!("a notch nearer: {}", z.distance));
    z.zoom(-1.0);
    c.check((z.distance - 10.0).abs() < 1e-12, || "a notch farther".into());
    z.zoom(1000.0);
    c.check(z.distance == z.min_distance, || "min".into());
    z.zoom(-1000.0);
    c.check(z.distance == z.max_distance, || "max".into());
    c.check(Follow::new(target, 1000.0).distance == 50.0 && Follow::new(target, 0.0).distance == 0.5, || "new keeps the distance".into());
    // a wall between the target and the eye: the eye comes in front of it
    let mut world = PhysicsWorld::new();
    let figure = world.add(Shape::capsule(0.5, 0.3), Iso::at(Vec3::new(0.0, 1.0, 0.0)));
    world.add(Shape::cuboid(Vec3::new(5.0, 5.0, 0.1)), Iso::at(Vec3::new(0.0, 1.0, 3.0)));
    let mut cam = Follow::new(Vec3::new(0.0, 1.5, 0.0), 6.0);
    cam.pitch = 0.0; // the eye behind the target along +Z (forward -Z)
    let filter = Filter::ALL.excluding(figure);
    let (v, d) = cam.view_in(&world, &filter, 0.2);
    // the wall's near face at z 2.9, a sphere of 0.2: the eye at most at z 2.7
    c.check(d < 6.0 && v.eye.z <= 2.7 + 1e-9 && v.eye.z > 2.6, || format!("pulled in: distance {d}, eye z {}", v.eye.z));
    let mut probe = alloc::vec::Vec::new();
    world.overlap(&Shape::sphere(0.2), &Iso::at(v.eye), &filter, &mut probe);
    c.check(probe.is_empty(), || "the eye's sphere touches nothing".into());
    cam.yaw = core::f64::consts::PI; // the other way: nothing in the way
    let (v, d) = cam.view_in(&world, &filter, 0.2);
    c.check(d == 6.0 && near(v.eye, cam.eye(), 0.0), || format!("free: {d}"));
}

fn free_fly(c: &mut Ctx) {
    let mut f = FreeFly::new(Vec3::ZERO);
    f.look(0.0, -0.5); // up
    let fwd = f.view().forward;
    f.fly(1.0, 0.0, 0.0, 1.0);
    c.check(near(f.eye, fwd * 10.0, 1e-12) && f.eye.y > 0.0, || format!("forward along the view: {:?}", f.eye));
    let mut g = FreeFly::new(Vec3::ZERO);
    g.fly(1.0, 1.0, 1.0, 0.5);
    c.check(((g.eye.length()) - 5.0).abs() < 1e-12, || format!("all three at most speed: {}", g.eye.length()));
    let mut h = FreeFly::new(Vec3::ZERO);
    h.fly(0.0, 0.0, 1.0, 2.0);
    c.check(h.eye == Vec3::new(0.0, 20.0, 0.0), || "up along +Y".into());
    h.fly(f64::NAN, 0.0, 0.0, 1.0);
    c.check(h.eye == Vec3::new(0.0, 20.0, 0.0), || "NaN ignored".into());
}
