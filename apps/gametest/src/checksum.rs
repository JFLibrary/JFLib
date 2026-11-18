// checksum -- the determinism harness: a fixed run of lib/game folded into one number (FNV-1a over the
// bits of every result).  Entities come and go and move under systems in their order, poses settle and
// interpolate, an Input gets the same events every run (keys, buttons, motion, the lock and its end),
// three cameras follow it, a figure casts against a physics World.  The same number on every target
// means the core computes the same bits everywhere (the server and its clients, G7e).

use alloc::vec::Vec;

use game::input::MOUSE_LEFT;
use game::{ActionKind, ActionMap, Bind, FirstPerson, Follow, FreeFly, Game, Input, Pose, Quat, Source, Step, TickInput, Transform, Vec3, World};
use physics::{Filter, Iso, Shape, World as PhysicsWorld};
use plat::{KC_A, KC_D, KC_S, KC_SPACE, KC_W, PointerLock};

use crate::Rng;

struct Sum {
    h: u64,
    n: u64,
}

impl Sum {
    fn u(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.h ^= b as u64;
            self.h = self.h.wrapping_mul(0x0000_0100_0000_01b3);
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
    fn t(&mut self, t: &Transform) {
        self.v(t.pos);
        self.f(t.rot.x);
        self.f(t.rot.y);
        self.f(t.rot.z);
        self.f(t.rot.w);
    }
}

#[derive(Clone, Copy)]
struct Vel(Vec3);

#[derive(Clone, Copy)]
struct Spin(f64);

struct Res {
    input: Input,
    tick: TickInput,
    ids: [game::ActionId; 4],
    fp: FirstPerson,
    phys: PhysicsWorld,
    player: Vec3,
    rng: Rng,
}

/// gravity and the floor at y 0 for the bodies with a velocity
fn sys_fall(w: &mut World, _: &mut Res, s: &Step) {
    let (poses, vels) = w.pair_mut::<Pose, Vel>();
    for (e, mut v) in vels.iter_mut() {
        if let Some(mut p) = poses.get_mut(e) {
            v.0.y -= 9.81 * s.dt;
            let mut pos = p.now.pos + v.0 * s.dt;
            if pos.y < 0.0 {
                pos.y = -pos.y;
                v.0.y = -v.0.y * 0.8;
            }
            p.now.pos = pos;
        }
    }
}

fn sys_spin(w: &mut World, _: &mut Res, s: &Step) {
    let (poses, spins) = w.pair_mut::<Pose, Spin>();
    for (e, sp) in spins.iter() {
        if let Some(mut p) = poses.get_mut(e) {
            p.now.rot = p.now.rot.mul(Quat::from_axis_angle(Vec3::Y, sp.0 * s.dt)).normalize();
        }
    }
}

/// some die, some are born (the order of iteration changes)
fn sys_churn(w: &mut World, r: &mut Res, _: &Step) {
    let live: Vec<_> = w.column::<Vel>().map_or(Vec::new(), |c| c.entities().to_vec());
    for _ in 0..3 {
        if !live.is_empty() {
            let e = live[r.rng.below(live.len())];
            w.despawn(e);
        }
        let p = Vec3::new(r.rng.range(-20.0, 20.0), r.rng.range(0.0, 10.0), r.rng.range(-20.0, 20.0));
        let v = Vec3::new(r.rng.range(-2.0, 2.0), r.rng.range(-1.0, 3.0), r.rng.range(-2.0, 2.0));
        let e = w.spawn_with((Pose::at(p), Vel(v)));
        if r.rng.below(3) == 0 {
            w.insert(e, Spin(r.rng.range(-3.0, 3.0)));
        }
    }
}

/// the player walks by the tick's input, a capsule cast keeps it above the ground
fn sys_player(_: &mut World, r: &mut Res, s: &Step) {
    let (fwd, side) = (r.tick.value(r.ids[0]), r.tick.value(r.ids[1]));
    let dir = r.fp.walk_dir(fwd, side);
    let motion = dir * (4.5 * s.dt) + Vec3::new(0.0, -0.05, 0.0);
    let cap = Shape::capsule(0.5, 0.3);
    match r.phys.cast(&cap, &Iso::at(r.player), motion, &Filter::ALL) {
        Some((_, hit)) => r.player += motion * hit.fraction.max(0.0),
        None => r.player += motion,
    }
}

pub fn run() -> (u64, u64) {
    let mut sum = Sum { h: 0xcbf2_9ce4_8422_2325, n: 0 };
    let mut map = ActionMap::new();
    let ids = [
        map.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]),
        map.add("strafe", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0)]),
        map.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]),
        map.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY)]),
    ];
    let fire = map.add("fire", ActionKind::Button, &[Bind::new(Source::Mouse(MOUSE_LEFT)), Bind::key(KC_SPACE)]);
    let mut input = Input::new(map);
    // the same factor everywhere: the checksum compares targets, not their mice
    input.set_mouse(game::MouseSettings { factor: Some(1.0), ..game::MouseSettings::DEFAULT });
    let mut phys = PhysicsWorld::new();
    phys.add(Shape::cuboid(Vec3::new(50.0, 0.5, 50.0)), Iso::at(Vec3::new(0.0, -0.5, 0.0)));
    phys.add(Shape::cuboid(Vec3::new(1.0, 1.0, 1.0)), Iso::at(Vec3::new(0.0, 1.0, -6.0)));
    let res = Res { input, tick: TickInput::default(), ids, fp: FirstPerson::new(Vec3::new(0.0, 1.7, 0.0)), phys, player: Vec3::new(0.0, 0.81, 0.0), rng: Rng::new(5) };
    let mut g = Game::new(res);
    g.systems.add("churn", sys_churn).add("fall", sys_fall).add("spin", sys_spin).add("player", sys_player);
    for i in 0..400 {
        let p = Vec3::new((i % 20) as f64 - 10.0, (i / 20) as f64 * 0.5, -((i % 7) as f64));
        g.world.spawn_with((Pose::at(p), Vel(Vec3::new(0.1 * (i % 3) as f64, 0.0, 0.0))));
    }
    let mut follow = Follow::new(Vec3::ZERO, 6.0);
    let mut fly = FreeFly::new(Vec3::new(0.0, 5.0, 10.0));
    let mut ev = Rng::new(17);
    let mut now = 0u64;
    g.frame(now);
    for frame in 0..900u32 {
        // the frame's events: the same every run
        let input = &mut g.res.input;
        match frame {
            3 => input.button(MOUSE_LEFT, true),
            4 => {
                if input.take_lock_request() {
                    input.set_lock(PointerLock::Locked);
                }
                input.button(MOUSE_LEFT, false);
            }
            500 => input.set_lock(PointerLock::Off),
            520 => {
                input.button(MOUSE_LEFT, true);
                input.set_lock(PointerLock::Locked);
                input.button(MOUSE_LEFT, false);
            }
            _ => {}
        }
        if frame % 37 == 5 {
            input.key(KC_W, true);
        }
        if frame % 37 == 30 {
            input.key(KC_W, false);
        }
        if frame % 53 == 7 {
            input.key(KC_D, frame % 2 == 1);
        }
        if frame % 11 == 0 {
            input.key(KC_SPACE, true);
            input.key(KC_SPACE, false);
        }
        input.motion(ev.range(-30.0, 30.0), ev.range(-10.0, 10.0));
        let (lx, ly) = (input.motion_of(ids[2]), input.motion_of(ids[3]));
        g.res.fp.look(lx, ly);
        follow.look(lx, ly);
        fly.look(lx, ly);
        // frames of 7 to 25 ms: 0 to 2 ticks each; the tick takes the input first
        now += 7_000_000 + ev.below(18_000_001) as u64;
        let n = g.clock.advance_to(now);
        for _ in 0..n {
            g.res.tick = g.res.input.tick();
            sum.u(g.res.tick.pressed(fire) as u64);
            g.step();
        }
        g.res.input.end_frame();
        let alpha = g.alpha();
        fly.fly(g.res.input.axis(ids[0]), g.res.input.axis(ids[1]), 0.0, 0.016);
        follow.target = g.res.player;
        g.res.fp.eye = g.res.player + Vec3::new(0.0, 0.9, 0.0);
        let fv = g.res.fp.view();
        sum.v(fv.eye);
        sum.v(fv.forward);
        sum.v(follow.view_in(&g.res.phys, &Filter::ALL, 0.2).0.eye);
        sum.v(fly.eye);
        sum.f(alpha);
        if frame % 10 == 0 {
            if let Some(poses) = g.world.column::<Pose>() {
                for (e, p) in poses.iter() {
                    sum.u(e.to_bits());
                    sum.t(&p.interpolate(alpha));
                }
            }
        }
    }
    sum.u(g.world.tick());
    sum.u(g.res.input.pauses() as u64);
    (sum.h, sum.n)
}
