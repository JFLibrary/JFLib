// net_lag -- lag compensation against brute force: the view time of an input, the rewind of the hit
// volumes against a plain list of every recorded tick, the hitscan against a ray-capsule intersection and
// a slab test written here, and the arena's shot: a hit where the shooter saw the target, not where it is

use alloc::format;
use alloc::vec::Vec;

use game::net::arena::{Arena, ArenaConfig};
use game::net::input::NetInput;
use game::net::lagcomp::{hitscan, view_time, Hit, HitHistory};
use game::net::quant::{angle_q, pitch_q};
use game::net::server::RoomSim;
use game::net::snapshot::tick_diff;
use game::net::walker::{WalkState, BTN_FIRE};
use game::Vec3;
use physics::{Iso, Shape};

use crate::{Ctx, Rng};

pub fn run(c: &mut Ctx) {
    views(c);
    rewinds(c);
    rays(c);
    arena_shot(c);
}

fn views(c: &mut Ctx) {
    let mut bad = 0;
    for &tick in &[1000u32, 5, u32::MAX - 2] {
        for view in (0..=20 * 256u32).step_by(7) {
            let (t, f) = view_time(tick, view as u16, 12);
            let back = (view as f64 / 256.0).min(12.0);
            let got = tick_diff(t, tick) as f64 + f;
            if (got + back).abs() > 1e-12 || !(0.0..1.0).contains(&f) {
                bad += 1;
            }
        }
    }
    c.check(bad == 0, || format!("view_time: {} wrong", bad));
}

fn rewinds(c: &mut Ctx) {
    let mut r = Rng::new(11);
    let mut bad = 0;
    let mut cases = 0;
    for round in 0..c.n(200) {
        let start = if round % 3 == 0 { u32::MAX - 15 } else { 500 + round as u32 };
        let cap = 8 + r.below(30);
        let mut h = HitHistory::new(cap);
        // the reference: every tick recorded, in plain vectors
        let mut all: Vec<(u32, Vec<(u16, Vec3)>)> = Vec::new();
        let figs = 1 + r.below(16);
        let mut pos: Vec<Vec3> = (0..figs).map(|_| Vec3::new(r.range(-20.0, 20.0), r.range(0.0, 3.0), r.range(-20.0, 20.0))).collect();
        let ticks = 5 + r.below(60);
        for k in 0..ticks {
            let t = start.wrapping_add(k as u32);
            let mut v = Vec::new();
            for (i, p) in pos.iter_mut().enumerate() {
                *p += Vec3::new(r.range(-0.2, 0.2), r.range(-0.05, 0.05), r.range(-0.2, 0.2));
                // some figures are missing in some ticks (joined late, left)
                if (i + k) % 7 != 3 {
                    v.push((i as u16 * 3 + 1, *p));
                }
            }
            // recorded in a shuffled order: the history sorts
            let mut shuffled = v.clone();
            for j in (1..shuffled.len()).rev() {
                shuffled.swap(j, r.below(j + 1));
            }
            h.record(t, &shuffled);
            all.push((t, v));
        }
        let kept = &all[all.len().saturating_sub(cap)..];
        let mut out = Vec::new();
        for _ in 0..50 {
            cases += 1;
            let back = r.below(ticks + 10) as u32;
            let tick = start.wrapping_add(ticks as u32).wrapping_sub(back);
            let frac = if r.below(4) == 0 { 0.0 } else { r.f() };
            h.rewind(tick, frac, &mut out);
            // brute force: clamp to the kept span, find the two ticks by scanning
            let (first, last) = (kept[0].0, kept[kept.len() - 1].0);
            let (t0, f) = if tick_diff(tick, first) < 0 {
                (first, 0.0)
            } else if tick_diff(tick, last) >= 0 {
                (last, 0.0)
            } else {
                (tick, frac)
            };
            let a = &kept.iter().find(|x| x.0 == t0).map(|x| x.1.clone()).unwrap_or_default();
            let b = kept.iter().find(|x| x.0 == t0.wrapping_add(1)).map(|x| x.1.clone());
            let mut want: Vec<(u16, Vec3)> = Vec::new();
            for &(id, p) in a {
                let q = if f > 0.0 { b.as_ref().and_then(|b| b.iter().find(|x| x.0 == id).map(|x| x.1)) } else { None };
                want.push((id, match q {
                    Some(q) => Vec3::new(p.x + (q.x - p.x) * f, p.y + (q.y - p.y) * f, p.z + (q.z - p.z) * f),
                    None => p,
                }));
            }
            if out != want {
                bad += 1;
            }
        }
    }
    c.check(bad == 0, || format!("rewind: {} of {} differ from brute force", bad, cases));
}

/// the reference: the first t >= 0 of origin + t dir (unit) on a capsule (axis on y through c, half height
/// hh, radius rad): an infinite cylinder clipped to the segment, and the two end spheres
fn ray_capsule(o: Vec3, d: Vec3, c: Vec3, hh: f64, rad: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    let mut take = |t: f64| {
        if t >= 0.0 && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    let p = o - c;
    // inside already
    let y = p.y.clamp(-hh, hh);
    if (p - Vec3::new(0.0, y, 0.0)).length() <= rad {
        return Some(0.0);
    }
    let a = d.x * d.x + d.z * d.z;
    if a > 1e-15 {
        let b = p.x * d.x + p.z * d.z;
        let cc = p.x * p.x + p.z * p.z - rad * rad;
        let disc = b * b - a * cc;
        if disc >= 0.0 {
            let t = (-b - physics::math::sqrt(disc)) / a;
            let yy = p.y + t * d.y;
            if (-hh..=hh).contains(&yy) {
                take(t);
            }
        }
    }
    for s in [hh, -hh] {
        let q = p - Vec3::new(0.0, s, 0.0);
        let b = q.dot(d);
        let cc = q.dot(q) - rad * rad;
        let disc = b * b - cc;
        if disc >= 0.0 {
            take(-b - physics::math::sqrt(disc));
        }
    }
    best
}

/// the reference: a ray against an axis-aligned box (slabs)
fn ray_box(o: Vec3, d: Vec3, c: Vec3, h: Vec3, max_t: f64) -> Option<f64> {
    let (mut lo, mut hi) = (0.0f64, max_t);
    for k in 0..3 {
        let (oo, dd, cc, hh) = (o[k], d[k], c[k], h[k]);
        if dd.abs() < 1e-300 {
            if oo < cc - hh || oo > cc + hh {
                return None;
            }
            continue;
        }
        let (mut t1, mut t2) = ((cc - hh - oo) / dd, (cc + hh - oo) / dd);
        if t1 > t2 {
            core::mem::swap(&mut t1, &mut t2);
        }
        lo = lo.max(t1);
        hi = hi.min(t2);
        if lo > hi {
            return None;
        }
    }
    Some(lo)
}

fn rays(c: &mut Ctx) {
    let mut r = Rng::new(12);
    let (hh, rad) = (0.6, 0.3);
    let shape = Shape::capsule(hh, rad);
    let (mut bad, mut hits_fig, mut hits_world, mut cases) = (0, 0, 0, 0);
    for _ in 0..c.n(400) {
        let mut map = physics::World::new();
        let boxes: Vec<(Vec3, Vec3)> = (0..r.below(6)).map(|_| (Vec3::new(r.range(-10.0, 10.0), r.range(0.0, 3.0), r.range(-10.0, 10.0)), Vec3::new(r.range(0.2, 2.0), r.range(0.2, 2.0), r.range(0.2, 2.0)))).collect();
        for (cc, h) in &boxes {
            map.add(Shape::cuboid(*h), Iso::at(*cc));
        }
        let figs: Vec<(u16, Vec3)> = (0..1 + r.below(16)).map(|i| (i as u16 + 1, Vec3::new(r.range(-12.0, 12.0), r.range(0.5, 2.5), r.range(-12.0, 12.0)))).collect();
        for _ in 0..40 {
            cases += 1;
            let shooter = figs[r.below(figs.len())].0;
            let o = Vec3::new(r.range(-14.0, 14.0), r.range(0.2, 3.0), r.range(-14.0, 14.0));
            // aim at a figure half of the time
            let target = if r.below(2) == 0 { figs[r.below(figs.len())].1 + Vec3::new(r.range(-0.4, 0.4), r.range(-0.9, 0.9), r.range(-0.4, 0.4)) } else { Vec3::new(r.range(-14.0, 14.0), r.range(0.0, 3.0), r.range(-14.0, 14.0)) };
            let Some(d) = (target - o).try_normalize() else { continue };
            let max_t = 40.0;
            let got = hitscan(&map, &figs, &shape, shooter, o, d, max_t);
            // brute force
            let wall = boxes.iter().filter_map(|(cc, h)| ray_box(o, d, *cc, *h, max_t)).fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.min(t))));
            let mut fig: Option<(u16, f64)> = None;
            for &(id, p) in &figs {
                if id == shooter {
                    continue;
                }
                if let Some(t) = ray_capsule(o, d, p, hh, rad) {
                    if t <= max_t && fig.is_none_or(|f| t < f.1) {
                        fig = Some((id, t));
                    }
                }
            }
            let want = match (fig, wall) {
                (Some((id, t)), Some(w)) if t <= w => Some((Some(id), t)),
                (Some((id, t)), None) => Some((Some(id), t)),
                (_, Some(w)) => Some((None, w)),
                (None, None) => None,
            };
            let got2 = got.map(|h| match h {
                Hit::Figure { id, t, .. } => (Some(id), t),
                Hit::World { t, .. } => (None, t),
            });
            let same = match (got2, want) {
                (None, None) => true,
                (Some((a, ta)), Some((b, tb))) => a == b && (ta - tb).abs() < 1e-9,
                _ => false,
            };
            if !same {
                // a graze within a micrometre of a surface may go either way
                let near = matches!((got2, want), (Some((_, ta)), Some((_, tb))) if (ta - tb).abs() < 1e-6);
                if !near {
                    bad += 1;
                }
            }
            match got2 {
                Some((Some(_), _)) => hits_fig += 1,
                Some((None, _)) => hits_world += 1,
                None => {}
            }
        }
    }
    c.check(bad == 0, || format!("hitscan: {} of {} differ from brute force", bad, cases));
    c.check(hits_fig > cases / 5 && hits_world > cases / 10, || format!("hitscan: {} figure hits, {} map hits of {}", hits_fig, hits_world, cases));
}

fn arena_shot(c: &mut Ctx) {
    let cfg = ArenaConfig { crates: 0, fire: true, max_rewind: 12, ..ArenaConfig::default() };
    let mut a = Arena::new(cfg);
    let shooter = a.join("A").unwrap_or(0);
    let target = a.join("B").unwrap_or(0);
    a.teleport(shooter, Vec3::new(0.0, 0.001, 10.0));
    a.teleport(target, Vec3::new(-6.0, 0.001, 0.0));
    let dt = 1.0 / 60.0;
    // the target runs along +x for 30 ticks (yaw -90 degrees looks along +x)
    let run = NetInput { move_z: 127, yaw: angle_q(-core::f64::consts::FRAC_PI_2), buttons: 2, ..NetInput::default() };
    let stand = NetInput::default();
    let mut where_at: Vec<Vec3> = Vec::new();
    for t in 1..=30u32 {
        a.step(t, dt, &[(shooter, stand, true), (target, run, true)]);
        where_at.push(a.figures()[&target].state.feet());
    }
    let eye = |a: &Arena| a.figures()[&shooter].state.feet() + Vec3::new(0.0, a.walker().eye(), 0.0);
    let aim = |a: &Arena, at: Vec3| -> (u16, i16) {
        let d = at + Vec3::new(0.0, 0.9, 0.0) - eye(a);
        let (yaw, pitch) = game::camera::yaw_pitch_of(d).unwrap_or((0.0, 0.0));
        (angle_q(yaw), pitch_q(pitch))
    };
    // the shooter saw the target 10 ticks ago (at the end of tick 21) and aims there: a hit with the rewind
    let seen = where_at[20];
    let (yaw, pitch) = aim(&a, seen);
    let shot = NetInput { buttons: BTN_FIRE, yaw, pitch, view: 10 * 256, ..NetInput::default() };
    let before = a.stats();
    a.step(31, dt, &[(shooter, shot, true), (target, run, true)]);
    let after = a.stats();
    c.check(after.shots == before.shots + 1 && after.hits == before.hits + 1, || format!("shot at the rewound place: shots {} hits {}", after.shots, after.hits));
    let health = a.figures()[&target].health;
    c.check(health == 75, || format!("damage: health {}", health));
    // the same aim without the rewind (view 0) misses: the target is 1.5 m further on
    let shot0 = NetInput { view: 0, ..shot };
    for t in 32..40 {
        a.step(t, dt, &[(shooter, stand, true), (target, stand, true)]);
    }
    let (yaw, pitch) = aim(&a, seen);
    let before = a.stats();
    a.step(40, dt, &[(shooter, NetInput { yaw, pitch, ..shot0 }, true), (target, stand, true)]);
    c.check(a.stats().shots == before.shots + 1 && a.stats().hits == before.hits, || format!("shot at an old place without rewind: hits {}", a.stats().hits - before.hits));
    // a view time further back than 200 ms is clamped (counted)
    let before = a.stats();
    a.step(50, dt, &[(shooter, NetInput { view: 40 * 256, ..shot0 }, true), (target, stand, true)]);
    c.check(a.stats().clamped == before.clamped + 1, || format!("clamped: {}", a.stats().clamped));
    let _ = WalkState::default();
}
