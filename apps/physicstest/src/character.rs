// character -- the character controller's tests (groups "character" and "charfuzz"), its part of the
// checksum and its measurement (docs/PHYSICS.md, "Der Character-Controller")
//
//   character  scenarios with exact expectations: jump apex and landing tick from the free-flight formula,
//              bunny hops keep the speed, a staircase (and one too high), the slope limit (where the capsule
//              stops at a steep ramp, sliding down one, standing on a walkable one), sliding along a wall at
//              many angles (the speed along the wall is the projection), V corners (rest at the apex,
//              touching both walls, no jitter), crouching under a low ceiling (stands up exactly where the
//              standing capsule fits), snapping down ramps and stairs, walking off a ledge, pushing out of
//              an overlap, the figure's own body in the world
//   charfuzz   random figures with random inputs on random sealed maps: two rooms with a wall between them,
//              hills, boxes, ramps, stairs, low ceilings, thin walls and convex bodies; velocities thrown up
//              to 1000 m/s: never inside anything, never in the other room or outside, never under the ground

use crate::scene::{self, MeshData};
use crate::tests::random_convex;
use crate::{Ctx, Rng, out};
use alloc::format;
use alloc::vec::Vec;
use physics::math::{self, max, min, sqrt};
use physics::{BodyId, CharacterController, CharacterInput, CharacterParams, CharacterState, Filter, Iso, Quat, Shape, Vec3, World};

/// the tick of the tests: 60 Hz
pub const DT: f64 = 1.0 / 60.0;
const ALL: Filter = Filter::ALL;

// ---------------------------------------------------------------- building maps

/// a box between two corners (outward winding as scene::cuboid)
pub fn aabox(m: &mut MeshData, lo: [f32; 3], hi: [f32; 3]) {
    let base = m.pos.len() as u32;
    for k in 0..8 {
        m.pos.push([
            if k & 1 != 0 { hi[0] } else { lo[0] },
            if k & 2 != 0 { hi[1] } else { lo[1] },
            if k & 4 != 0 { hi[2] } else { lo[2] },
        ]);
    }
    let faces: [[u32; 4]; 6] = [[0, 4, 6, 2], [1, 3, 7, 5], [0, 1, 5, 4], [2, 6, 7, 3], [0, 2, 3, 1], [4, 5, 7, 6]];
    for f in faces {
        m.idx.extend_from_slice(&[base + f[0], base + f[1], base + f[2], base + f[0], base + f[2], base + f[3]]);
    }
}

/// a prism over the quad `q` (x, z; one rotational order) from y0 to y1, every face with the same winding
pub fn prism(m: &mut MeshData, q: [[f64; 2]; 4], y0: f64, y1: f64) {
    let base = m.pos.len() as u32;
    for y in [y0, y1] {
        for p in q {
            m.pos.push([p[0] as f32, y as f32, p[1] as f32]);
        }
    }
    let mut quad = |a: u32, b: u32, c: u32, d: u32| m.idx.extend_from_slice(&[base + a, base + b, base + c, base + a, base + c, base + d]);
    quad(0, 3, 2, 1);
    quad(4, 5, 6, 7);
    for i in 0..4 {
        let j = (i + 1) % 4;
        quad(i, j, j + 4, i + 4);
    }
}

/// a solid ramp over x0..x0 + len (rising along +x by `rise`), z0..z1, on y0, turned by `turn` quarter turns
/// about (cx, cz) (exact in f32: coordinates swap and change sign)
#[allow(clippy::too_many_arguments)]
pub fn wedge(m: &mut MeshData, x0: f32, len: f32, z0: f32, z1: f32, y0: f32, rise: f32, turn: u32, c: [f32; 2]) {
    let base = m.pos.len() as u32;
    let x1 = x0 + len;
    let pts = [[x0, y0, z0], [x0, y0, z1], [x1, y0, z0], [x1, y0, z1], [x1, y0 + rise, z0], [x1, y0 + rise, z1]];
    for p in pts {
        let (mut dx, mut dz) = (p[0] - c[0], p[2] - c[1]);
        for _ in 0..turn % 4 {
            (dx, dz) = (-dz, dx);
        }
        m.pos.push([c[0] + dx, p[1], c[1] + dz]);
    }
    // slope, back, the two sides, the bottom; outward
    let tris: [[u32; 3]; 8] = [[0, 1, 5], [0, 5, 4], [2, 4, 5], [2, 5, 3], [0, 4, 2], [1, 3, 5], [0, 2, 3], [0, 3, 1]];
    for t in tris {
        m.idx.extend_from_slice(&[base + t[0], base + t[1], base + t[2]]);
    }
}

/// stairs along +x from x0: `n` blocks of `tread` with risers of `riser` on y0, z0..z1, then a landing of
/// `landing` at the top
#[allow(clippy::too_many_arguments)]
pub fn stairs(m: &mut MeshData, x0: f32, n: usize, riser: f32, tread: f32, y0: f32, z0: f32, z1: f32, landing: f32) {
    for i in 0..n {
        let a = x0 + i as f32 * tread;
        aabox(m, [a, y0, z0], [a + tread, y0 + (i + 1) as f32 * riser, z1]);
    }
    let a = x0 + n as f32 * tread;
    aabox(m, [a, y0, z0], [a + landing, y0 + n as f32 * riser, z1]);
}

fn world_of(m: &MeshData) -> World {
    let mut w = World::new();
    w.add(Shape::mesh(m.build()), Iso::IDENTITY);
    w
}

// ---------------------------------------------------------------- helpers

fn input(wish: Vec3, run: bool) -> CharacterInput {
    CharacterInput { wish, run, jump: false, crouch: false }
}

/// the deepest overlap of the figure with anything (0: none)
pub fn depth(ctrl: &CharacterController, w: &World, f: &Filter, st: &CharacterState) -> f64 {
    let mut cs = Vec::new();
    w.contacts(&ctrl.capsule(st.crouched), &ctrl.capsule_iso(st), 0.0, f, &mut cs);
    let mut d = 0.0f64;
    for (_, c) in &cs {
        d = max(d, c.depth);
    }
    d
}

/// something within `gap` of the figure
fn touching(ctrl: &CharacterController, w: &World, st: &CharacterState, gap: f64) -> bool {
    let mut cs = Vec::new();
    w.contacts(&ctrl.capsule(st.crouched), &ctrl.capsule_iso(st), gap, &ALL, &mut cs);
    !cs.is_empty()
}

/// steps without input until the figure stands still on the ground (at most 3 s)
fn settle(ctrl: &CharacterController, w: &World, feet: Vec3) -> CharacterState {
    let mut st = CharacterState::new(feet);
    for _ in 0..180 {
        let (s, _) = ctrl.step(&st, w, &ALL, &CharacterInput::default(), DT);
        st = s;
        if st.grounded && st.velocity == Vec3::ZERO {
            break;
        }
    }
    st
}

fn horiz(v: Vec3) -> f64 {
    sqrt(v.x * v.x + v.z * v.z)
}

// ---------------------------------------------------------------- scenarios

pub fn scenario_tests(c: &mut Ctx) {
    jump_tests(c);
    stair_tests(c);
    slope_tests(c);
    wall_tests(c);
    corner_tests(c);
    crouch_tests(c);
    snap_tests(c);
    misc_tests(c);
}

/// jump apex and landing from the free-flight formula; a drop; bunny hops keep the speed
fn jump_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    let w = world_of(&scene::flat(20, 1.0, 0.0));
    let j = ctrl.jump_speed();
    c.check(j == sqrt(2.0 * p.gravity * p.jump_height), || format!("jump speed {}", j));
    // on a vertex, an edge and a face of the grid
    for (case, (x, z)) in [(10.0, 10.0), (10.5, 7.0), (7.25, 8.6)].into_iter().enumerate() {
        let (mut st, _) = ctrl.step(&CharacterState::new(Vec3::new(x, p.skin, z)), &w, &ALL, &CharacterInput::default(), DT);
        c.check(st.grounded && (st.position.y - p.skin).abs() < 1e-12 && st.velocity == Vec3::ZERO, || format!("jump {}: not resting {:?}", case, st));
        let y0 = st.position.y;
        let (mut apex, mut landed, mut impact, mut air_ok) = (y0, 0usize, 0.0, true);
        for k in 1..300 {
            let (s, ev) = ctrl.step(&st, &w, &ALL, &CharacterInput { jump: k == 1, ..CharacterInput::default() }, DT);
            if k == 1 {
                c.check(ev.jumped, || format!("jump {}: no jump", case));
            }
            apex = max(apex, s.position.y);
            st = s;
            if ev.landed {
                landed = k;
                impact = ev.impact_speed;
                break;
            }
            air_ok &= !s.grounded && s.position.x == x && s.position.z == z;
        }
        // the free flight: y0 + j t - g t^2 / 2 at t = k dt; it lands in the first tick that ends within the
        // probe distance (3 skins) of the floor
        let free = |k: usize| {
            let t = k as f64 * DT;
            y0 + j * t - 0.5 * p.gravity * t * t
        };
        let (mut want_apex, mut want_land) = (y0, 0usize);
        for k in 1..300 {
            want_apex = max(want_apex, free(k));
            if free(k) <= 3.0 * p.skin {
                want_land = k;
                break;
            }
        }
        let tol = p.gravity * DT * DT / 8.0; // the highest tick lies within dt/2 of the true apex
        c.check(air_ok, || format!("jump {}: grounded or moved sideways in the air", case));
        c.check((apex - want_apex).abs() < 1e-9, || format!("jump {}: apex {} instead of {}", case, apex, want_apex));
        c.check(want_apex - y0 <= p.jump_height + 1e-12 && want_apex - y0 >= p.jump_height - tol - 1e-12, || format!("jump {}: apex height {}", case, want_apex - y0));
        c.check(landed == want_land, || format!("jump {}: landed in tick {} instead of {}", case, landed, want_land));
        let want_impact = p.gravity * (landed as f64 - 0.5) * DT - j;
        c.check((impact - want_impact).abs() < 1e-9, || format!("jump {}: impact {} instead of {}", case, impact, want_impact));
        c.check((st.position.y - p.skin).abs() < 1e-9 && st.velocity == Vec3::ZERO && st.grounded, || format!("jump {}: after landing {:?}", case, st));
    }
    // a drop from 3 m: the landing tick and speed
    let mut st = CharacterState::new(Vec3::new(5.0, 3.0, 5.0));
    let mut landed = 0;
    let mut impact = 0.0;
    for k in 1..200 {
        let (s, ev) = ctrl.step(&st, &w, &ALL, &CharacterInput::default(), DT);
        st = s;
        if ev.landed {
            landed = k;
            impact = ev.impact_speed;
            break;
        }
    }
    let mut want = 0;
    for k in 1..200 {
        let t = k as f64 * DT;
        if 3.0 - 0.5 * p.gravity * t * t <= 3.0 * p.skin {
            want = k;
            break;
        }
    }
    c.check(landed == want && (impact - p.gravity * (want as f64 - 0.5) * DT).abs() < 1e-9, || format!("drop: landed in tick {} at {} m/s, expected {}", landed, impact, want));
    c.check((st.position.y - p.skin).abs() < 1e-9 && st.grounded, || format!("drop: rests at {:?}", st.position));
    // bunny hops: jumping on every landing keeps the run speed exactly (no friction on those ticks)
    let w = world_of(&scene::flat(60, 1.0, 0.0));
    let mut st = settle(&ctrl, &w, Vec3::new(2.0, 0.1, 10.0));
    for _ in 0..60 {
        st = ctrl.step(&st, &w, &ALL, &input(Vec3::X, true), DT).0;
    }
    c.check((horiz(st.velocity) - p.run_speed).abs() < 1e-9, || format!("run speed {}", horiz(st.velocity)));
    let mut jumps = 0;
    let mut worst = 0.0f64;
    for _ in 0..150 {
        let (s, ev) = ctrl.step(&st, &w, &ALL, &CharacterInput { wish: Vec3::X, run: true, jump: true, crouch: false }, DT);
        jumps += ev.jumped as u32;
        worst = max(worst, (horiz(s.velocity) - p.run_speed).abs());
        st = s;
    }
    c.check(jumps >= 3 && worst < 1e-9, || format!("bunny hops: {} jumps, speed off by {}", jumps, worst));
}

/// a staircase at several speeds; risers just below and above the step height
fn stair_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    for (riser, tread, climbs) in [(0.25f32, 0.35f32, true), (0.15, 0.3, true), (0.375, 0.5, true), (0.45, 0.5, false), (0.6, 0.5, false)] {
        let n = 8;
        let x0 = 5.0f32;
        let mut m = scene::flat(24, 1.0, 0.0);
        stairs(&mut m, x0, n, riser, tread, 0.0, 2.0, 10.0, 6.0);
        let w = world_of(&m);
        let top = (n as f32 * riser) as f64;
        let top_x = (x0 + n as f32 * tread) as f64;
        // run, walk, crouch, a slow stick (1 m/s)
        for (speed_case, (amount, run, crouch)) in [(1.0, true, false), (1.0, false, false), (1.0, false, true), (0.2, false, false)].into_iter().enumerate() {
            let mut st = settle(&ctrl, &w, Vec3::new(2.0, 0.1, 6.0));
            let wish_speed = if crouch { p.crouch_speed } else if run { p.run_speed } else { p.walk_speed } * amount;
            let ticks = ((top_x + 1.5 - 2.0) / wish_speed / DT * 2.0) as usize + 60;
            let (mut air, mut inside, mut reached, mut max_x) = (0usize, 0.0f64, false, 0.0f64);
            for _ in 0..ticks {
                let inp = CharacterInput { wish: Vec3::new(amount, 0.0, 0.0), run, jump: false, crouch };
                let (s, _) = ctrl.step(&st, &w, &ALL, &inp, DT);
                st = s;
                air += !st.grounded as usize;
                inside = max(inside, depth(&ctrl, &w, &ALL, &st));
                max_x = max(max_x, st.position.x);
                if st.position.x > top_x + 1.5 {
                    reached = true;
                    break;
                }
            }
            if climbs {
                c.check(reached && air == 0 && (st.position.y - top - p.skin).abs() < 1e-9 && inside <= 1e-9, || {
                    format!("stairs {} x {} speed case {}: reached {} at {:?}, {} ticks in the air, overlap {}", riser, tread, speed_case, reached, st.position, air, inside)
                });
            } else {
                // stopped by the first riser, the capsule's side against it
                c.check(!reached && max_x <= x0 as f64 - p.radius + 1e-9 && (st.position.y - p.skin).abs() < 1e-9 && air == 0, || {
                    format!("stairs {} too high, speed case {}: got to x {} y {}", riser, speed_case, max_x, st.position.y)
                });
            }
        }
    }
}

/// walkable ramps are climbed; at steep ones the capsule stops where it touches the slope; on a steep one
/// it slides down, on a walkable one it stands still
fn slope_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    let (r, skin) = (p.radius, p.skin);
    for deg in [20.0f64, 30.0, 40.0, 44.0, 46.0, 50.0, 60.0, 75.0] {
        let x0 = 6.0f32;
        let len = 4.0f32;
        let rise = (len as f64 * math::sin(deg.to_radians_det()) / math::cos(deg.to_radians_det())) as f32;
        let mut m = scene::flat(24, 1.0, 0.0);
        wedge(&mut m, x0, len, 2.0, 10.0, 0.0, rise, 0, [0.0, 0.0]);
        aabox(&mut m, [x0 + len, 0.0, 2.0], [x0 + len + 5.0, rise, 10.0]);
        let w = world_of(&m);
        // the true angle of the f32 ramp
        let hyp = sqrt((len as f64) * (len as f64) + (rise as f64) * (rise as f64));
        let (sin_t, cos_t) = (rise as f64 / hyp, len as f64 / hyp);
        let walkable = cos_t >= ctrl.walkable_y();
        let mut st = settle(&ctrl, &w, Vec3::new(3.0, 0.1, 6.0));
        let (mut reached, mut max_x, mut max_y, mut air, mut inside) = (false, 0.0f64, 0.0f64, 0usize, 0.0f64);
        for _ in 0..240 {
            let (s, _) = ctrl.step(&st, &w, &ALL, &input(Vec3::X, true), DT);
            st = s;
            max_x = max(max_x, st.position.x);
            max_y = max(max_y, st.position.y);
            air += !st.grounded as usize;
            inside = max(inside, depth(&ctrl, &w, &ALL, &st));
            if st.position.x > (x0 + len) as f64 + 1.5 {
                reached = true;
                break;
            }
        }
        if walkable {
            c.check(reached && air == 0 && (st.position.y - rise as f64 - skin).abs() < 1e-9 && inside <= 1e-9, || {
                format!("ramp {} deg: reached {} at {:?}, {} ticks in the air", deg, reached, st.position, air)
            });
        } else {
            // the sphere of the capsule (centre r + skin above the floor) touches the slope plane
            let stop = x0 as f64 + ((skin + r) * cos_t - r) / sin_t;
            c.check(!reached && max_x <= stop + 1e-9 && max_x >= stop - 2.0 * skin && max_y <= skin + 1e-9 && air == 0 && inside <= 1e-9, || {
                format!("ramp {} deg: got to x {} (stop at {}), y {}, {} ticks in the air", deg, max_x, stop, max_y, air)
            });
        }
        // put on the ramp half way up, no input
        let mid = x0 as f64 + 0.5 * len as f64;
        let surface = (mid - x0 as f64) * sin_t / cos_t;
        let mut st = CharacterState::new(Vec3::new(mid, surface + 0.3, 6.0));
        let (mut on_slope, mut inside) = (0usize, 0.0f64);
        let mut last = st.position;
        let mut still = 0usize;
        for _ in 0..300 {
            let (s, _) = ctrl.step(&st, &w, &ALL, &CharacterInput::default(), DT);
            inside = max(inside, depth(&ctrl, &w, &ALL, &s));
            if s.grounded && s.position.x > x0 as f64 + 0.3 && s.position.x < (x0 + len) as f64 - 0.3 {
                on_slope += 1;
            }
            still = if s.position == last { still + 1 } else { 0 };
            last = s.position;
            st = s;
        }
        if walkable {
            c.check(still >= 200 && st.grounded && st.position.x > x0 as f64 && inside <= 1e-9, || format!("on the {} deg ramp: does not stand still ({:?})", deg, st.position));
        } else {
            // landing on the floor along the slope leaves a gap between 0 and the skin
            c.check(on_slope == 0 && st.grounded && st.position.x < x0 as f64 && st.position.y > 0.0 && st.position.y <= skin + 1e-9 && inside <= 1e-9, || {
                format!("on the {} deg ramp: {} ticks standing on it, ends at {:?}", deg, on_slope, st.position)
            });
        }
    }
}

trait Radians {
    fn to_radians_det(self) -> f64;
}

impl Radians for f64 {
    fn to_radians_det(self) -> f64 {
        self * (core::f64::consts::PI / 180.0)
    }
}

/// sliding along a wall at many angles: with neither friction nor acceleration the speed along the wall
/// is the projection of the speed into it; with the default feel the figure slides and never sticks
fn wall_tests(c: &mut Ctx) {
    let slide = CharacterController::new(CharacterParams { accel: 0.0, friction: 0.0, stop_speed: 0.0, air_control: 0.0, ..CharacterParams::default() });
    let feel = CharacterController::default();
    let p = *feel.params();
    for (k, yaw_deg) in [0.0f64, 37.0, 90.0, 211.0].into_iter().enumerate() {
        // the wall: a turned box (a body of the world) or a turned prism in the mesh; its face through
        // (12, 12) with the normal n
        let (sy, cy) = math::sincos(yaw_deg.to_radians_det());
        let n = Vec3::new(cy, 0.0, sy);
        let t = Vec3::new(-sy, 0.0, cy);
        let face = Vec3::new(12.0, 0.0, 12.0);
        let mut m = scene::flat(24, 1.0, 0.0);
        let mut w;
        if k % 2 == 0 {
            let q = |a: f64, b: f64| {
                let v = face + t * a - n * b;
                [v.x, v.z]
            };
            prism(&mut m, [q(-11.0, 0.0), q(11.0, 0.0), q(11.0, 1.0), q(-11.0, 1.0)], -0.5, 3.0);
            w = world_of(&m);
        } else {
            w = world_of(&m);
            let centre = face - n * 0.5 + Vec3::new(0.0, 1.0, 0.0);
            w.add(Shape::cuboid(Vec3::new(0.5, 2.0, 11.0)), Iso::new(centre, Quat::from_axis_angle(Vec3::Y, -yaw_deg.to_radians_det())));
        }
        let dist = |s: &CharacterState| (s.position - face).dot(n) - p.radius;
        for phi in [0.0f64, 5.0, 15.0, 30.0, 45.0, 60.0, 75.0, 85.0, 89.0] {
            let (sp, cp) = math::sincos(phi.to_radians_det());
            // the gap at the start: the wall is reached within 0.25 s also when moving nearly along it
            let gap0 = min(1.5, 0.25 * 6.0 * cp) + 0.01;
            let start = face + n * (p.radius + gap0) - t * 9.0 + Vec3::new(0.0, p.skin, 0.0);
            // the pure slide: 6 m/s into the wall
            let speed = 6.0;
            let mut st = CharacterState::new(start);
            st.velocity = (t * sp - n * cp) * speed;
            let (mut inside, mut hit) = (0.0f64, false);
            for _ in 0..60 {
                let (s, ev) = slide.step(&st, &w, &ALL, &CharacterInput::default(), DT);
                hit |= ev.wall_count() > 0;
                inside = max(inside, depth(&slide, &w, &ALL, &s));
                st = s;
            }
            let along = (st.position - start).dot(t);
            let want_v = t * (speed * sp);
            c.check(hit && (st.velocity - want_v).length() < 1e-9, || format!("wall {} phi {}: velocity {:?} instead of {:?}", yaw_deg, phi, st.velocity, want_v));
            c.check((along - speed * sp).abs() <= p.skin + 1e-9, || format!("wall {} phi {}: {} m along the wall instead of {}", yaw_deg, phi, along, speed * sp));
            c.check(dist(&st) >= -1e-9 && dist(&st) <= p.skin + 1e-9 && inside <= 1e-9, || format!("wall {} phi {}: gap {} overlap {}", yaw_deg, phi, dist(&st), inside));
            // the default feel, pushing into the wall at phi: it slides along, never into it
            let mut st = settle(&feel, &w, start);
            let wish = t * sp - n * cp;
            let mut inside = 0.0f64;
            let mut trail = Vec::new();
            for _ in 0..75 {
                let (s, _) = feel.step(&st, &w, &ALL, &input(wish, true), DT);
                inside = max(inside, depth(&feel, &w, &ALL, &s));
                trail.push(s.position);
                st = s;
            }
            let last = trail.len() - 1;
            // with input the velocity approaches the wish velocity; the wall takes its part into the wall
            let want_v = t * (p.run_speed * sp);
            let drift = (trail[last] - trail[last - 10]).dot(n).abs();
            let ok = if phi == 0.0 { (trail[last] - trail[last - 30]).length() < 1e-9 } else { (st.velocity - want_v).length() < 1e-9 && drift < 1e-9 };
            c.check(ok && inside <= 1e-9 && dist(&st) >= -1e-9 && dist(&st) <= p.skin + 1e-9, || {
                format!("wall {} phi {} with feel: velocity {:?} (want {:?}) drift {} gap {} overlap {}", yaw_deg, phi, st.velocity, want_v, drift, dist(&st), inside)
            });
        }
    }
}

/// V-shaped corners: the figure walking into the apex rests touching both walls, without jitter
fn corner_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    for (k, apex_deg) in [30.0f64, 60.0, 90.0, 120.0, 150.0].into_iter().enumerate() {
        let b = 0.7 + k as f64 * 1.3; // the bisector's heading
        let half = 0.5 * apex_deg.to_radians_det();
        let apex = Vec3::new(12.0, 0.0, 12.0);
        let dir = |a: f64| {
            let (s, co) = math::sincos(a);
            Vec3::new(co, 0.0, s)
        };
        let mut m = scene::flat(24, 1.0, 0.0);
        let mut normals = [Vec3::ZERO; 2];
        for (i, sgn) in [1.0f64, -1.0].into_iter().enumerate() {
            let u = dir(b + sgn * half);
            let out = dir(b + sgn * (half + core::f64::consts::FRAC_PI_2)); // away from the inside of the V
            normals[i] = -out;
            let q = |a: f64, o: f64| {
                let v = apex + u * a + out * o;
                [v.x, v.z]
            };
            prism(&mut m, [q(0.0, 0.0), q(8.0, 0.0), q(8.0, 1.0), q(-1.0, 1.0)], -0.5, 3.0);
        }
        let w = world_of(&m);
        let bis = dir(b);
        for off_deg in [0.0f64, 10.0, -10.0] {
            let mut st = settle(&ctrl, &w, apex + bis * 5.0 + Vec3::new(0.0, 0.2, 0.0));
            let wish = dir(b + core::f64::consts::PI + off_deg.to_radians_det());
            let mut trail = Vec::new();
            let mut inside = 0.0f64;
            for _ in 0..180 {
                let (s, _) = ctrl.step(&st, &w, &ALL, &input(wish, true), DT);
                inside = max(inside, depth(&ctrl, &w, &ALL, &s));
                trail.push(s.position);
                st = s;
            }
            let mut jitter = 0.0f64;
            for i in trail.len() - 60..trail.len() {
                jitter = max(jitter, (trail[i] - trail[i - 1]).length());
            }
            let gaps = [(st.position - apex).dot(normals[0]) - p.radius, (st.position - apex).dot(normals[1]) - p.radius];
            c.check(jitter < 1e-9 && inside <= 1e-9 && gaps.iter().all(|g| *g >= -1e-9 && *g <= p.skin + 1e-9), || {
                format!("corner {} deg, off {}: jitter {} gaps {:?} overlap {}", apex_deg, off_deg, jitter, gaps, inside)
            });
        }
    }
}

/// a low ceiling (1.5 m): standing figures stop at it, crouched ones pass, standing up waits for room
fn crouch_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    let (r, skin) = (p.radius, p.skin);
    let mut m = scene::flat(24, 1.0, 0.0);
    aabox(&mut m, [4.0, 1.5, 0.0], [6.0, 2.5, 12.0]);
    let w = world_of(&m);
    // the top sphere's centre of the standing figure is at skin + height - radius; at the slab's bottom
    // (1.5 m) the sphere has this radius
    let dy = 1.5 - (skin + p.height - r);
    let rim = sqrt(r * r - dy * dy);
    // standing: stopped by the slab's front edge
    let mut st = settle(&ctrl, &w, Vec3::new(1.0, 0.1, 6.0));
    let mut max_x = 0.0f64;
    for _ in 0..120 {
        st = ctrl.step(&st, &w, &ALL, &input(Vec3::X, true), DT).0;
        max_x = max(max_x, st.position.x);
    }
    c.check(max_x <= 4.0 - rim + 1e-9 && max_x >= 4.0 - rim - 2.0 * skin && !st.crouched, || format!("low ceiling: standing got to {} (edge at {})", max_x, 4.0 - rim));
    // crouched: through, and standing up where the standing capsule fits (blocked while its top sphere's
    // centre is under the slab or within a radius of its back edge)
    let mut st = settle(&ctrl, &w, Vec3::new(1.0, 0.1, 6.0));
    let (mut wrong, mut blocked_ticks, mut stood_at) = (0usize, 0usize, 0.0f64);
    let mut inside = 0.0f64;
    for _ in 0..240 {
        let x = st.position.x;
        let crouch = x < 5.0; // let go in the middle of the slab
        let (s, ev) = ctrl.step(&st, &w, &ALL, &CharacterInput { wish: Vec3::X, run: false, jump: false, crouch }, DT);
        inside = max(inside, depth(&ctrl, &w, &ALL, &s));
        if !crouch && st.crouched {
            let d = if x <= 6.0 { 0.0 } else { sqrt((x - 6.0) * (x - 6.0) + dy * dy) };
            let fits = d > r;
            if (d - r).abs() > 1e-6 && fits == s.crouched {
                wrong += 1;
            }
            blocked_ticks += ev.stand_blocked as usize;
            if !s.crouched {
                stood_at = x;
            }
        }
        st = s;
    }
    c.check(wrong == 0 && blocked_ticks > 10 && stood_at > 6.0 + rim - 0.1 && !st.crouched && st.position.x > 9.0 && inside <= 1e-9, || {
        format!("low ceiling: {} wrong stand-up decisions, {} blocked, stood at {}, ends {:?}", wrong, blocked_ticks, stood_at, st.position)
    });
    // crouching in the air pulls the legs up: a crouch jump reaches higher
    let w2 = world_of(&scene::flat(12, 1.0, 0.0));
    let mut tops = [0.0f64; 2];
    for (i, crouch) in [false, true].into_iter().enumerate() {
        let mut st = settle(&ctrl, &w2, Vec3::new(6.0, 0.1, 6.0));
        for k in 0..60 {
            let (s, _) = ctrl.step(&st, &w2, &ALL, &CharacterInput { wish: Vec3::ZERO, run: false, jump: k == 0, crouch: crouch && k > 5 }, DT);
            tops[i] = max(tops[i], s.position.y);
            st = s;
        }
    }
    c.check((tops[1] - tops[0] - (p.height - p.crouch_height)).abs() < 1e-9, || format!("crouch jump: feet {} and {}", tops[0], tops[1]));
}

/// down ramps and stairs the figure stays on the ground; off a high ledge it falls and lands
fn snap_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    for deg in [15.0f64, 30.0, 44.0] {
        let (x0, len) = (6.0f32, 4.0f32);
        let rise = (len as f64 * math::sin(deg.to_radians_det()) / math::cos(deg.to_radians_det())) as f32;
        let mut m = scene::flat(24, 1.0, 0.0);
        wedge(&mut m, x0, len, 2.0, 10.0, 0.0, rise, 0, [0.0, 0.0]);
        aabox(&mut m, [x0 + len, 0.0, 2.0], [x0 + len + 5.0, rise, 10.0]);
        let w = world_of(&m);
        for (amount, run) in [(1.0, true), (1.0, false), (0.3, false)] {
            let mut st = settle(&ctrl, &w, Vec3::new((x0 + len) as f64 + 3.0, rise as f64 + 0.1, 6.0));
            let (mut air, mut loose, mut snaps) = (0usize, 0usize, 0usize);
            for _ in 0..900 {
                let (s, ev) = ctrl.step(&st, &w, &ALL, &input(Vec3::new(-amount, 0.0, 0.0), run), DT);
                air += !s.grounded as usize;
                loose += !touching(&ctrl, &w, &s, p.skin + 1e-6) as usize;
                snaps += (ev.snapped_down > 0.0) as usize;
                st = s;
                if st.position.x < x0 as f64 - 2.0 {
                    break;
                }
            }
            // onto the floor along the slope: a gap between 0 and the skin
            c.check(air == 0 && loose == 0 && snaps > 0 && st.position.x < x0 as f64 - 2.0 && st.position.y > 0.0 && st.position.y <= p.skin + 1e-9, || {
                format!("down the {} deg ramp at {} {}: {} ticks in the air, {} not touching, {} snaps, ends {:?}", deg, amount, run, air, loose, snaps, st.position)
            });
        }
    }
    // down the stairs
    let n = 8;
    let mut m = scene::flat(24, 1.0, 0.0);
    stairs(&mut m, 5.0, n, 0.3, 0.4, 0.0, 2.0, 10.0, 6.0);
    let w = world_of(&m);
    for run in [true, false] {
        let mut st = settle(&ctrl, &w, Vec3::new(5.0 + n as f64 * 0.4 + 3.0, n as f64 * 0.3 + 0.1, 6.0));
        let (mut air, mut snaps) = (0usize, 0usize);
        for _ in 0..400 {
            let (s, ev) = ctrl.step(&st, &w, &ALL, &input(Vec3::new(-1.0, 0.0, 0.0), run), DT);
            air += !s.grounded as usize;
            snaps += (ev.snapped_down > 0.05) as usize;
            st = s;
            if st.position.x < 3.0 {
                break;
            }
        }
        c.check(air == 0 && snaps >= n - 1 && (st.position.y - p.skin).abs() < 1e-9, || format!("down the stairs (run {}): {} ticks in the air, {} snaps, ends {:?}", run, air, snaps, st.position));
    }
    // off a ledge higher than a step: leaves the ground once, lands once
    let mut m = scene::flat(24, 1.0, 0.0);
    aabox(&mut m, [2.0, 0.0, 2.0], [8.0, 1.0, 10.0]);
    let w = world_of(&m);
    let mut st = settle(&ctrl, &w, Vec3::new(5.0, 1.1, 6.0));
    let (mut left, mut landed) = (0, 0);
    for _ in 0..120 {
        let (s, ev) = ctrl.step(&st, &w, &ALL, &input(Vec3::X, false), DT);
        left += ev.left_ground as u32;
        landed += ev.landed as u32;
        st = s;
    }
    c.check(left == 1 && landed == 1 && st.grounded && st.position.y > 0.0 && st.position.y <= p.skin + 1e-9 && st.position.x > 8.0 + p.radius, || {
        format!("off the ledge: left {} landed {} at {:?}", left, landed, st.position)
    });
}

/// pushing out of an overlap; the figure's own body in the world changes nothing; a rotated query
fn misc_tests(c: &mut Ctx) {
    let ctrl = CharacterController::default();
    let p = *ctrl.params();
    let mut m = scene::flat(24, 1.0, 0.0);
    aabox(&mut m, [10.0, 0.0, 10.0], [14.0, 2.0, 14.0]);
    let w = world_of(&m);
    let mut rng = Rng::new(31);
    // teleported a little into the box or the floor: out after one step
    for case in 0..c.n(200) {
        let side = rng.below(3);
        let d = rng.range(0.001, 0.5 * p.radius);
        let feet = match side {
            0 => Vec3::new(10.0 - p.radius + d, rng.range(0.0, 0.5), rng.range(10.5, 13.5)),
            1 => Vec3::new(rng.range(10.5, 13.5), 2.0 - d, rng.range(10.5, 13.5)),
            _ => Vec3::new(rng.range(3.0, 8.0), -d, rng.range(3.0, 8.0)),
        };
        let st = CharacterState::new(feet);
        let before = depth(&ctrl, &w, &ALL, &st);
        let (s, ev) = ctrl.step(&st, &w, &ALL, &CharacterInput::default(), DT);
        let after = depth(&ctrl, &w, &ALL, &s);
        c.check(before > 1e-6 && after <= 1e-9 && ev.pushed_out > 0.0, || format!("push out {}: depth {} -> {} (pushed {})", case, before, after, ev.pushed_out));
    }
    // the same walk with the figure as a body of the world (left out by the filter): the same bits
    let mut w2 = world_of(&m);
    let mut a = settle(&ctrl, &w, Vec3::new(3.0, 0.1, 12.0));
    let mut b = a;
    let id = w2.add(ctrl.capsule(false), ctrl.capsule_iso(&b));
    let f2 = Filter::ALL.excluding(id);
    let mut same = true;
    for k in 0..240 {
        let inp = CharacterInput { wish: Vec3::new(1.0, 0.0, 0.1), run: true, jump: k % 50 == 7, crouch: (k / 60) % 2 == 1 };
        a = ctrl.step(&a, &w, &ALL, &inp, DT).0;
        b = ctrl.step(&b, &w2, &f2, &inp, DT).0;
        w2.set_iso(id, ctrl.capsule_iso(&b));
        same &= a == b;
    }
    c.check(same, || "a figure's own body changed its steps".into());
    // nothing changes for a dt that is not positive, a position that is not finite
    let st = settle(&ctrl, &w, Vec3::new(3.0, 0.1, 3.0));
    let busy = CharacterInput { wish: Vec3::X, run: true, jump: true, crouch: true };
    for dt in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        c.check(ctrl.step(&st, &w, &ALL, &busy, dt).0 == st, || format!("dt {} changed the state", dt));
    }
    let mut bad = st;
    bad.position.x = f64::NAN;
    let (s, _) = ctrl.step(&bad, &w, &ALL, &busy, DT);
    c.check(s.position.x.is_nan(), || "a NaN position moved".into());
    // a NaN velocity counts as 0, parameters that make no sense are limited
    let mut bad = st;
    bad.velocity = Vec3::new(f64::NAN, 1.0, 0.0);
    let (s, _) = ctrl.step(&bad, &w, &ALL, &CharacterInput::default(), DT);
    c.check(s.velocity == Vec3::ZERO && (s.position - st.position).length() < 1e-12, || format!("NaN velocity: {:?}", s));
    let odd = CharacterController::new(CharacterParams { radius: -1.0, height: f64::NAN, crouch_height: 99.0, skin: 0.0, max_slope_deg: 200.0, ..CharacterParams::default() });
    let q = odd.params();
    c.check(q.radius == 0.01 && q.height == 1.8 && q.crouch_height == 1.8 && q.skin == 1e-4 && q.max_slope_deg == 89.0, || format!("limited parameters {:?}", q));
}

// ---------------------------------------------------------------- fuzz

/// a random sealed map: the floor of hills over 24 x 24 m, walls around, a ceiling at 8 m, a wall across
/// at x = 12 (room A on the low side), random obstacles, convex bodies -> (world, the map's body, the
/// number of ground triangles, the dividing wall's low x)
pub fn fuzz_map(rng: &mut Rng) -> (World, BodyId, u32, f64) {
    let mut m = MeshData::new();
    scene::ground(&mut m, 24, 1.0, rng.range(0.0, 1.5) as f32, rng.next_u64() as u32);
    let ground_tris = m.triangles() as u32;
    aabox(&mut m, [-1.0, -1.0, -1.0], [0.0, 9.0, 25.0]);
    aabox(&mut m, [24.0, -1.0, -1.0], [25.0, 9.0, 25.0]);
    aabox(&mut m, [-1.0, -1.0, -1.0], [25.0, 9.0, 0.0]);
    aabox(&mut m, [-1.0, -1.0, 24.0], [25.0, 9.0, 25.0]);
    aabox(&mut m, [-1.0, 8.0, -1.0], [25.0, 9.0, 25.0]);
    let thick = rng.range(0.02, 0.3) as f32;
    aabox(&mut m, [12.0, -1.0, 0.0], [12.0 + thick, 9.0, 24.0]);
    let at = |rng: &mut Rng| [rng.range(1.0, 23.0) as f32, rng.range(1.0, 23.0) as f32];
    for _ in 0..12 + rng.below(12) {
        let [x, z] = at(rng);
        let base = rng.range(-0.2, 1.2) as f32;
        match rng.below(6) {
            0 => {
                let h = [rng.range(0.1, 1.5) as f32, rng.range(0.05, 1.5) as f32, rng.range(0.1, 1.5) as f32];
                aabox(&mut m, [x - h[0], base, z - h[2]], [x + h[0], base + 2.0 * h[1], z + h[2]]);
            }
            1 => {
                let len = rng.range(1.0, 5.0) as f32;
                let deg = rng.range(10.0, 70.0);
                let rise = (len as f64 * math::sin(deg.to_radians_det()) / math::cos(deg.to_radians_det())) as f32;
                wedge(&mut m, x, len, z, z + rng.range(1.0, 4.0) as f32, base, rise, rng.below(4) as u32, [x, z]);
            }
            2 => {
                let n = 3 + rng.below(6);
                stairs(&mut m, x, n, rng.range(0.1, 0.45) as f32, rng.range(0.25, 0.6) as f32, base, z, z + rng.range(1.0, 3.0) as f32, rng.range(0.5, 2.0) as f32);
            }
            3 => {
                // a low ceiling
                let y = base + rng.range(1.0, 1.9) as f32;
                aabox(&mut m, [x - 1.5, y, z - 1.5], [x + 1.5, y + 0.3, z + 1.5]);
            }
            4 => {
                // a thin wall
                let t = rng.range(0.01, 0.05) as f32;
                let l = rng.range(1.0, 6.0) as f32;
                if rng.below(2) == 0 {
                    aabox(&mut m, [x, base - 0.5, z], [x + t, base + 3.0, z + l]);
                } else {
                    aabox(&mut m, [x, base - 0.5, z], [x + l, base + 3.0, z + t]);
                }
            }
            _ => {
                // a slanted plate
                let (a, b) = (rng.range(0.5, 3.0), rng.range(0.5, 3.0));
                let y0 = base as f64 + rng.range(0.0, 1.0);
                let tilt = rng.range(-1.5, 1.5);
                let q = [[x as f64, z as f64], [x as f64 + a, z as f64], [x as f64 + a, z as f64 + b], [x as f64, z as f64 + b]];
                prism(&mut m, q, y0, y0 + 0.1 + tilt.abs());
            }
        }
    }
    let mut w = World::new();
    let map = w.add(Shape::mesh(m.build()), Iso::IDENTITY);
    for _ in 0..8 + rng.below(10) {
        let kind = rng.below(4);
        let scale = rng.range(0.3, 1.2);
        let s = random_convex(rng, kind, scale);
        let iso = Iso::new(Vec3::new(rng.range(1.0, 23.0), rng.range(0.0, 3.0), rng.range(1.0, 23.0)), rng.quat());
        w.add(s, iso);
    }
    (w, map, ground_tris, 12.0)
}

/// the parameter sets of the fuzz: the default feel, fast (1000 m/s), tiny, big, light, ice
pub fn fuzz_params(k: usize) -> CharacterParams {
    let d = CharacterParams::default();
    match k % 6 {
        0 => d,
        1 => CharacterParams { run_speed: 40.0, walk_speed: 20.0, accel: 30.0, air_control: 1.0, max_speed: 1000.0, max_fall_speed: 1000.0, ..d },
        2 => CharacterParams { radius: 0.1, height: 0.5, crouch_height: 0.3, step_height: 0.15, skin: 0.001, ..d },
        3 => CharacterParams { radius: 0.8, height: 3.0, crouch_height: 2.0, step_height: 0.8, skin: 0.02, max_slope_deg: 60.0, ..d },
        4 => CharacterParams { gravity: 2.0, jump_height: 4.0, air_control: 1.0, ..d },
        _ => CharacterParams { friction: 0.0, stop_speed: 0.0, max_slope_deg: 30.0, ..d },
    }
}

/// a free place for the figure in x lo..hi: the top surface under a random point, a little above
pub fn spawn(rng: &mut Rng, ctrl: &CharacterController, w: &World, f: &Filter, lo: f64, hi: f64) -> CharacterState {
    let r = ctrl.params().radius;
    for _ in 0..200 {
        let (x, z) = (rng.range(lo + r + 0.1, hi - r - 0.1), rng.range(r + 0.1, 24.0 - r - 0.1));
        let Some((_, h)) = w.ray_cast(Vec3::new(x, 7.9, z), Vec3::new(0.0, -1.0, 0.0), 10.0, f) else {
            continue;
        };
        let st = CharacterState::new(Vec3::new(x, h.point.y + 0.05, z));
        let mut ov = Vec::new();
        w.overlap(&ctrl.capsule(false), &ctrl.capsule_iso(&st), f, &mut ov);
        if ov.is_empty() && ctrl.capsule_iso(&st).pos.y + 0.5 * ctrl.height(false) < 7.95 {
            return st;
        }
    }
    CharacterState::new(Vec3::new(lo + 1.0, 7.0, 12.0))
}

/// random inputs: a heading, a stick amount, run, jump, crouch
pub fn random_input(rng: &mut Rng) -> CharacterInput {
    let (s, co) = math::sincos(rng.range(0.0, 6.283_185_307_179_586));
    let amount = if rng.below(6) == 0 { 0.0 } else { rng.range(0.2, 1.3) };
    CharacterInput { wish: Vec3::new(co, 0.0, s) * amount, run: rng.below(3) != 0, jump: rng.below(4) == 0, crouch: rng.below(6) == 0 }
}

pub fn fuzz_tests(c: &mut Ctx) {
    let mut rng = Rng::new(4711);
    let (mut steps, mut fastest, mut max_depth) = (0u64, 0.0f64, 0.0f64);
    let mut events = [0u64; 6]; // landed, jumped, stepped, snapped, walls, ceilings
    for map in 0..c.n(24) {
        let (w, map_id, ground_tris, divider) = fuzz_map(&mut rng);
        for fig in 0..6 {
            let ctrl = CharacterController::new(fuzz_params(fig + map));
            let throw = if (fig + map) % 6 == 1 { 1000.0 } else { 60.0 };
            let mut st = spawn(&mut rng, &ctrl, &w, &ALL, 0.0, divider);
            let mut inp = random_input(&mut rng);
            let mut fails = 0;
            for tick in 0..600 {
                if rng.below(30) == 0 {
                    inp = random_input(&mut rng);
                }
                if rng.below(150) == 0 {
                    st.velocity = rng.dir() * rng.range(0.0, throw);
                }
                let before = st;
                fastest = max(fastest, st.velocity.length());
                let (s, ev) = ctrl.step(&st, &w, &ALL, &inp, DT);
                steps += 1;
                events[0] += ev.landed as u64;
                events[1] += ev.jumped as u64;
                events[2] += (ev.stepped_up > 0.0) as u64;
                events[3] += (ev.snapped_down > 0.0) as u64;
                events[4] += ev.wall_count() as u64;
                events[5] += ev.hit_ceiling as u64;
                let (ok, d, in_room, under) = fuzz_ok(&ctrl, &w, map_id, ground_tris, divider, &s);
                max_depth = max(max_depth, d);
                if !c.check(ok, || {
                    format!("charfuzz map {} figure {} tick {}: depth {} in room {} under {} {:?} -> {:?} ({:?})", map, fig, tick, d, in_room, under, before, s, inp)
                }) {
                    fails += 1;
                    if fails > 2 {
                        break;
                    }
                }
                st = s;
            }
        }
    }
    // the checks see a figure moved without casts (the same throws, straight on): the counter-test
    let (w, map_id, ground_tris, divider) = fuzz_map(&mut rng);
    let ctrl = CharacterController::default();
    let mut st = spawn(&mut rng, &ctrl, &w, &ALL, 0.0, divider);
    let mut caught = 0;
    for _ in 0..c.n(6000) {
        if rng.below(30) == 0 {
            st.velocity = rng.dir() * rng.range(0.0, 60.0);
        }
        st.position += st.velocity * DT;
        if !fuzz_ok(&ctrl, &w, map_id, ground_tris, divider, &st).0 {
            caught += 1;
            st = spawn(&mut rng, &ctrl, &w, &ALL, 0.0, divider);
        }
    }
    c.check(caught > 10, || format!("the fuzz checks caught a figure moved without casts only {} times", caught));
    out(&format!(
        "  charfuzz: {} steps, fastest {:.0} m/s, deepest overlap {:e} m; {} landings, {} jumps, {} steps up, {} snaps, {} wall contacts, {} ceilings\n",
        steps, fastest, max_depth, events[0], events[1], events[2], events[3], events[4], events[5]
    ));
}

/// the checks of the fuzz: finite, inside nothing, in room A and the arena, not under the ground -> (all
/// right, the deepest overlap, in the room, under the ground)
fn fuzz_ok(ctrl: &CharacterController, w: &World, map_id: BodyId, ground_tris: u32, divider: f64, s: &CharacterState) -> (bool, f64, bool, bool) {
    let r = ctrl.params().radius;
    let d = depth(ctrl, w, &ALL, s);
    let cen = ctrl.capsule_iso(s).pos;
    let top = cen.y + 0.5 * ctrl.height(s.crouched);
    let in_room = cen.x + r <= divider + 1e-6 && cen.x - r >= -1e-6 && cen.z - r >= -1e-6 && cen.z + r <= 24.0 + 1e-6 && top <= 8.0 + 1e-6;
    // under the ground: a ray up from the centre meets the ground first (from below)
    let under = match w.ray_cast(cen, Vec3::Y, 20.0, &ALL) {
        Some((id, h)) => id == map_id && h.triangle < ground_tris,
        None => false,
    };
    (s.position.is_finite() && s.velocity.is_finite() && d <= 1e-6 && in_room && !under, d, in_room, under)
}

// ---------------------------------------------------------------- checksum

/// the controller's part of the determinism harness: twelve figures of four kinds as bodies of a world
/// (the map of the checksum with a fence, 40 bodies), random inputs and throws, 600 ticks; `u` takes
/// every value
pub fn checksum(mut u: impl FnMut(u64)) {
    let mut f = |v: f64| u(v.to_bits());
    let mut rng = Rng::new(2027);
    let mut data = scene::map(60, 40);
    // a fence around the map, so the figures stay on it
    aabox(&mut data, [-1.0, -1.0, -1.0], [0.0, 12.0, 61.0]);
    aabox(&mut data, [60.0, -1.0, -1.0], [61.0, 12.0, 61.0]);
    aabox(&mut data, [-1.0, -1.0, -1.0], [61.0, 12.0, 0.0]);
    aabox(&mut data, [-1.0, -1.0, 60.0], [61.0, 12.0, 61.0]);
    let mut w = World::new();
    w.add(Shape::mesh(data.build()), Iso::IDENTITY);
    for _ in 0..40 {
        let kind = rng.below(4);
        let s = random_convex(&mut rng, kind, 0.8);
        w.add(s, Iso::new(Vec3::new(rng.range(2.0, 58.0), rng.range(0.0, 4.0), rng.range(2.0, 58.0)), rng.quat()));
    }
    let ctrls: Vec<CharacterController> = (0..4).map(|k| CharacterController::new(fuzz_params(k))).collect();
    let mut figs: Vec<(usize, CharacterState, BodyId, CharacterInput)> = Vec::new();
    for i in 0..12 {
        let ctrl = &ctrls[i % 4];
        let feet = Vec3::new(8.0 + 3.7 * i as f64, 7.0, 10.0 + 2.9 * i as f64);
        let st = CharacterState::new(feet);
        let id = w.add(ctrl.capsule(false), ctrl.capsule_iso(&st));
        figs.push((i % 4, st, id, random_input(&mut rng)));
    }
    for tick in 0..600 {
        for fig in figs.iter_mut() {
            let ctrl = &ctrls[fig.0];
            if tick % 20 == 0 {
                fig.3 = random_input(&mut rng);
            }
            if tick % 97 == 13 {
                fig.1.velocity = rng.dir() * rng.range(0.0, 30.0);
            }
            let (s, ev) = ctrl.step(&fig.1, &w, &Filter::ALL.excluding(fig.2), &fig.3, DT);
            w.set_iso(fig.2, ctrl.capsule_iso(&s));
            fig.1 = s;
            f(s.position.x);
            f(s.position.y);
            f(s.position.z);
            f(s.velocity.x);
            f(s.velocity.y);
            f(s.velocity.z);
            f(s.ground_normal.x);
            f(s.ground_normal.y);
            f(s.ground_normal.z);
            f(ev.impact_speed);
            f(ev.stepped_up);
            f(ev.snapped_down);
            f(ev.pushed_out);
            f(((s.grounded as u64) | (s.crouched as u64) << 1 | (ev.landed as u64) << 2 | (ev.jumped as u64) << 3 | (ev.hit_ceiling as u64) << 4 | (ev.stand_blocked as u64) << 5) as f64);
            f(ev.casts as f64);
            for t in ev.walls() {
                f(t.normal.x);
                f(t.normal.y);
                f(t.normal.z);
                f(t.point.x);
                f(t.triangle as f64);
                f(t.body.index() as f64);
            }
        }
    }
}

// ---------------------------------------------------------------- measurement

/// controllers stepped per millisecond on the 50 000-triangle map: 32 figures (16 players and 16 bots) as
/// bodies of the world, random inputs, 600 ticks per run
pub fn bench(runs: usize) {
    let data = scene::map(155, 160);
    let mut w = World::new();
    w.add(Shape::mesh(data.build()), Iso::IDENTITY);
    let ctrl = CharacterController::default();
    let mut rng = Rng::new(99);
    let mut per_ms = Vec::new();
    let mut casts = 0u64;
    let mut steps = 0u64;
    for _ in 0..runs {
        let mut figs: Vec<(CharacterState, BodyId, CharacterInput)> = Vec::new();
        for _ in 0..32 {
            let (x, z) = (rng.range(5.0, 150.0), rng.range(5.0, 150.0));
            let y = match w.ray_cast(Vec3::new(x, 30.0, z), Vec3::new(0.0, -1.0, 0.0), 40.0, &ALL) {
                Some((_, h)) => h.point.y + 0.3,
                None => 3.0,
            };
            let st = CharacterState::new(Vec3::new(x, y, z));
            let id = w.add(ctrl.capsule(false), ctrl.capsule_iso(&st));
            figs.push((st, id, random_input(&mut rng)));
        }
        let (mut c, mut n) = (0u64, 0u64);
        let t0 = plat::time_us();
        for tick in 0..600 {
            for fig in figs.iter_mut() {
                if tick % 30 == 0 {
                    fig.2 = random_input(&mut rng);
                }
                let (s, ev) = ctrl.step(&fig.0, &w, &Filter::ALL.excluding(fig.1), &fig.2, DT);
                w.set_iso(fig.1, ctrl.capsule_iso(&s));
                fig.0 = s;
                c += ev.casts as u64;
                n += 1;
            }
        }
        let us = (plat::time_us() - t0) as f64;
        per_ms.push(n as f64 / (us / 1000.0));
        casts += c;
        steps += n;
        for fig in &figs {
            w.remove(fig.1);
        }
    }
    per_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let med = per_ms[per_ms.len() / 2];
    out(&format!(
        "controllers: {:.0} steps per ms (median of {:?}; 32 figures on {} triangles, {:.2} casts per step); 32 figures at 60 Hz: {:.2} % of a core\n",
        med,
        per_ms.iter().map(|v| (*v + 0.5) as i64).collect::<Vec<_>>(),
        data.triangles(),
        casts as f64 / steps.max(1) as f64,
        32.0 * 60.0 / (med * 1000.0) * 100.0
    ));
}
