// tick_t -- the group "tick": the fixed rate on a virtual clock against floor(time x rate / 10^9) in
// integers, the cap, the alpha; systems in their order; poses settled per tick and interpolated per frame

use alloc::format;
use alloc::vec::Vec;

use game::{FixedTick, Game, Pose, Quat, Step, Transform, Vec3, World, nlerp};

use crate::{Ctx, Rng, out};

pub fn run(c: &mut Ctx) {
    clock(c);
    cap(c);
    systems(c);
    poses(c);
}

/// steps due by time t (ns) at `rate`: the reference
fn due(t: u128, rate: u128) -> u128 {
    t * rate / 1_000_000_000
}

/// frames of 16 ms for 10 s, random frames, long runs at several rates: every step accounted for, alpha
/// the exact fraction
fn clock(c: &mut Ctx) {
    // 625 frames of 16 ms: 600 ticks at 60 Hz, alpha 0 at the end
    let mut t = FixedTick::new(60);
    c.check(t.rate() == 60 && t.dt() == 1.0 / 60.0 && t.max_steps() == game::DEFAULT_MAX_STEPS, || "defaults".into());
    c.check(t.advance_to(5_000_000_000) == 0, || "the first call only starts the clock".into());
    let (mut total, mut bad) = (0u128, 0u32);
    for f in 1..=625u64 {
        let n = t.advance_to(5_000_000_000 + f * 16_000_000) as u128;
        total += n;
        let want = due(f as u128 * 16_000_000, 60);
        let frac = ((f as u128 * 16_000_000 * 60) % 1_000_000_000) as f64 / 1e9;
        if total != want || t.alpha() != frac {
            bad += 1;
        }
    }
    c.check(bad == 0 && total == 600 && t.alpha() == 0.0, || format!("16 ms frames: {total} ticks, {bad} frames off"));
    // a clock that goes back stands still, reset forgets it
    let mut t = FixedTick::new(60);
    t.advance_to(1_000_000_000);
    c.check(t.advance_to(900_000_000) == 0 && t.advance_to(1_000_000_000) == 0, || "a clock going back counts as standing still".into());
    c.check(t.advance_to(1_016_666_667) == 1, || "one tick after it came back".into());
    t.reset();
    c.check(t.advance_to(9_000_000_000) == 0 && t.alpha() == 0.0 && t.advance_to(9_016_666_667) == 1, || "reset: no catch-up".into());
    // random frames of 0 to 50 ms at several rates (the cap of 8 is never reached below 160 Hz)
    let mut r = Rng::new(9);
    for rate in [1u32, 24, 30, 60, 120, 144] {
        let mut t = FixedTick::new(rate);
        t.advance_to(0);
        let (mut now, mut total, mut bad) = (0u128, 0u128, 0u32);
        for _ in 0..c.n(100_000) {
            now += r.below(50_000_001) as u128;
            total += t.advance_to(now as u64) as u128;
            let frac = ((now * rate as u128) % 1_000_000_000) as f64 / 1e9;
            if total != due(now, rate as u128) || t.alpha() != frac {
                bad += 1;
            }
        }
        c.check(bad == 0 && t.dropped_ns() == 0, || format!("{rate} Hz random frames: {bad} off"));
    }
    // an hour of 1 ms frames at 60 and 10 000 Hz (advance_by): nothing drifts
    for rate in [60u32, 10_000] {
        let mut t = FixedTick::new(rate).with_max_steps(100);
        let frames = c.n(3_600_000) as u128;
        let mut total = 0u128;
        for _ in 0..frames {
            total += t.advance_by(1_000_000) as u128;
        }
        c.check(total == due(frames * 1_000_000, rate as u128), || format!("{rate} Hz over {frames} ms: {total} ticks"));
    }
}

/// the cap: a frame that hung 1 s runs 8 ticks, drops the rest, keeps the fraction
fn cap(c: &mut Ctx) {
    let mut t = FixedTick::new(60);
    t.advance_to(0);
    let n = t.advance_to(1_008_000_000);
    // 1.008 s at 60 Hz: 60.48 ticks due -> 8 run, 52 dropped (866 666 666 ns), alpha 0.48
    c.check(n == 8 && t.dropped_ns() == 866_666_666 && (t.alpha() - 0.48).abs() < 1e-12, || format!("cap: {n} ticks, {} ns dropped, alpha {}", t.dropped_ns(), t.alpha()));
    let n = t.advance_to(1_016_000_000);
    c.check(n == 0 && (t.alpha() - 0.96).abs() < 1e-12, || format!("after the cap the clock goes on: {n}, alpha {}", t.alpha()));
    let mut t = FixedTick::new(60).with_max_steps(0);
    t.advance_to(0);
    c.check(t.max_steps() == 1 && t.advance_to(100_000_000) == 1, || "max_steps at least 1".into());
    c.check(FixedTick::new(0).rate() == 1 && FixedTick::new(1_000_000).rate() == 10_000, || "rates kept in 1 ..= 10 000".into());
}

/// what the systems of the tests write
#[derive(Default)]
struct Log {
    seen: Vec<(&'static str, u64)>,
    dt: f64,
}

fn sys_a(_: &mut World, r: &mut Log, s: &Step) {
    r.seen.push(("a", s.tick));
    r.dt = s.dt;
}

fn sys_b(_: &mut World, r: &mut Log, s: &Step) {
    r.seen.push(("b", s.tick));
}

fn sys_c(_: &mut World, r: &mut Log, s: &Step) {
    r.seen.push(("c", s.tick));
}

/// systems run in the order added (add_before, remove), Game::frame runs the ticks due
fn systems(c: &mut Ctx) {
    let mut g = Game::new(Log::default());
    g.systems.add("a", sys_a).add("c", sys_c).add_before("c", "b", sys_b);
    c.check(g.systems.names() == ["a", "b", "c"], || format!("order: {:?}", g.systems.names()));
    g.frame(0);
    let n = g.frame(50_000_000);
    c.check(n == 3 && g.world.tick() == 3, || format!("50 ms at 60 Hz: {n} ticks"));
    let want: Vec<(&str, u64)> = (1..=3).flat_map(|t| [("a", t), ("b", t), ("c", t)]).collect();
    c.check(g.res.seen == want && g.res.dt == 1.0 / 60.0, || format!("systems in order per tick: {:?}", g.res.seen));
    c.check(g.systems.remove("b") && !g.systems.remove("b") && g.systems.len() == 2, || "remove".into());
    g.res.seen.clear();
    g.step();
    c.check(g.res.seen == [("a", 4), ("c", 4)], || format!("step: {:?}", g.res.seen));
}

/// the movers of `poses`: one metre a tick along x for the entities with a Tag
#[derive(Clone, Copy)]
struct Mover;

fn sys_move(w: &mut World, _: &mut (), _: &Step) {
    let (poses, movers) = w.pair_mut::<Pose, Mover>();
    for (e, mut p) in poses.iter_mut() {
        if movers.contains(e) {
            p.now.pos.x += 1.0;
        }
    }
}

/// poses: settled at the tick, interpolated at the frame's alpha; only what moved is touched
fn poses(c: &mut Ctx) {
    let mut g = Game::new(());
    g.systems.add("move", sys_move);
    let n = 10_000;
    let mut movers = Vec::new();
    for i in 0..n {
        let e = g.world.spawn_with((Pose::at(Vec3::new(0.0, i as f64, 0.0)),));
        if i % 1000 == 0 {
            g.world.insert(e, Mover);
            movers.push(e);
        }
    }
    g.frame(0);
    g.frame(16_666_667); // one tick: the movers at x 1, prev x 0
    let p = *g.world.get::<Pose>(movers[0]).unwrap();
    c.check(p.prev.pos.x == 0.0 && p.now.pos.x == 1.0 && p.moving(), || format!("after a tick: prev {} now {}", p.prev.pos.x, p.now.pos.x));
    let mut bad = 0;
    for k in 0..=8u32 {
        let a = k as f64 / 8.0;
        if p.interpolate(a).pos.x != a {
            bad += 1;
        }
    }
    c.check(bad == 0, || "interpolate: prev + alpha (eighths exact)".into());
    // the poses that moved in the latest tick are those newer than pose_mark
    let poses = g.world.column::<Pose>().unwrap();
    let newer = poses.changed_since(g.world.pose_mark()).count();
    c.check(newer == movers.len(), || format!("{newer} poses newer than the mark, {} movers", movers.len()));
    // a tick in which nobody moves: begin_tick sets prev = now for the movers only
    g.systems.remove("move");
    g.step();
    let p = *g.world.get::<Pose>(movers[0]).unwrap();
    let standing = g.world.get::<Pose>(g.world.entities().at(1).unwrap()).copied().unwrap();
    c.check(!p.moving() && p.prev.pos.x == 1.0 && !standing.moving(), || "settled: prev = now".into());
    let newer = g.world.column::<Pose>().unwrap().changed_since(g.world.pose_mark()).count();
    c.check(newer == 0, || format!("nothing moved in the latest tick: {newer}"));
    // teleport: no frames in between
    let mut q = Pose::at(Vec3::new(1.0, 2.0, 3.0));
    q.teleport(Transform::at(Vec3::new(100.0, 0.0, 0.0)));
    c.check(q.interpolate(0.5).pos.x == 100.0 && !q.moving(), || "teleport".into());
    // the endpoints of a blend are exact; rotations by nlerp on the short way
    let a = Transform::new(Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY);
    let b = Transform::new(Vec3::new(4.0, -2.0, 8.0), Quat::from_axis_angle(Vec3::Y, core::f64::consts::FRAC_PI_2)).scaled([2.0, 2.0, 2.0]);
    c.check(a.lerp(&b, 0.0) == a && a.lerp(&b, 1.0) == b && a.lerp(&b, 0.25).pos == Vec3::new(1.0, -0.5, 2.0), || "lerp endpoints".into());
    let half = nlerp(Quat::IDENTITY, b.rot, 0.5);
    let want = Quat::from_axis_angle(Vec3::Y, core::f64::consts::FRAC_PI_4);
    let err = (half.x - want.x).abs() + (half.y - want.y).abs() + (half.z - want.z).abs() + (half.w - want.w).abs();
    c.check(err < 1e-15, || format!("nlerp halfway to 90 degrees is 45 degrees: {err:e}"));
    let neg = Quat::new(-b.rot.x, -b.rot.y, -b.rot.z, -b.rot.w);
    let h2 = nlerp(Quat::IDENTITY, neg, 0.5);
    let err2 = (h2.x - want.x).abs() + (h2.y - want.y).abs() + (h2.z - want.z).abs() + (h2.w - want.w).abs();
    c.check(err2 < 1e-15, || "nlerp takes the short way for -q".into());
    c.check(a.lerp(&b, 0.5).scale == [1.5, 1.5, 1.5], || "scale blends".into());
    out(&format!("  poses: {n} poses, {} moving, settled and interpolated\n", movers.len()));
}
