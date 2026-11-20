// t_move -- the group "move": the figure's movement and weapon (game1_core::movement) against the rules written out:
// the state survives the trip through the fields, the same inputs give the same bits (and a run continued from the
// fields of its middle gives the second half again: what a client does after a reconciliation), random figures stay
// inside the map and above the floor, running and walking reach their speeds, a jump lands, crouching lowers the
// eye, the weapon fires every FIRE_TICKS, an empty magazine reloads for RELOAD_TICKS, R reloads, a dead figure stands

use alloc::format;
use alloc::vec::Vec;

use game::net::input::NetInput;
use game::net::predict::Movement;
use game::net::quant::{angle_q, axis_q};
use game1_core::map::{fnv, Map, FNV0};
use game1_core::movement::{gun, MoveState, Mover, F_DEAD, F_FIRED, F_JUMPED, F_LANDED};
use game1_core::rules::*;
use physics::Vec3;

use crate::{Ctx, Rng};

const DT: f64 = 1.0 / RATE as f64;

/// a random input (buttons, axes, view)
pub fn random_input(rng: &mut Rng, yaw: &mut f64) -> NetInput {
    *yaw += rng.range(-0.15, 0.15);
    let mut b = 0u16;
    for (bit, p) in [(BTN_JUMP, 0.04), (BTN_WALK, 0.15), (BTN_FIRE, 0.3), (BTN_RELOAD, 0.01), (BTN_CROUCH, 0.08)] {
        if rng.f() < p {
            b |= bit;
        }
    }
    NetInput { buttons: b, move_x: axis_q(rng.range(-1.0, 1.0)), move_z: axis_q(rng.range(-0.3, 1.0)), yaw: angle_q(*yaw), pitch: 0, view: 0 }
}

/// FNV over a state's fields
pub fn fold(h: u64, s: &MoveState) -> u64 {
    let mut f = [0i32; 8];
    Mover::to_fields(s, &mut f);
    let mut h = h;
    for v in f {
        h = fnv(h, &v.to_le_bytes());
    }
    h
}

/// `figures` random figures over `ticks` ticks from the spawn points -> the checksum, and every state
pub fn random_run(map: &Map, figures: usize, ticks: usize, seed: u64) -> (u64, Vec<Vec<MoveState>>) {
    let mover = Mover::default();
    let mut rng = Rng::new(seed);
    let mut st: Vec<MoveState> = (0..figures).map(|k| MoveState::at(map.spawns[k % map.spawns.len()].feet)).collect();
    let mut yaw: Vec<f64> = (0..figures).map(|k| map.spawns[k % map.spawns.len()].yaw).collect();
    let mut all = Vec::new();
    let mut h = FNV0;
    for _ in 0..ticks {
        for k in 0..figures {
            let i = random_input(&mut rng, &mut yaw[k]);
            st[k] = mover.step(&map.world, &st[k], &i, DT);
            h = fold(h, &st[k]);
        }
        all.push(st.clone());
    }
    (h, all)
}

fn on_floor(map: &Map, at: Vec3) -> MoveState {
    let mover = Mover::default();
    let mut s = MoveState::at(at);
    for _ in 0..30 {
        s = mover.step(&map.world, &s, &NetInput::default(), DT);
    }
    s
}

pub fn run(c: &mut Ctx) {
    let map = game1_core::map::werft();
    let mover = Mover::default();
    let mut rng = Rng::new(3);
    // the fields: every state comes back the same
    for _ in 0..c.n(20_000) {
        let s = MoveState {
            pos: [rng.next_u64() as i32, rng.next_u64() as i32, rng.next_u64() as i32],
            vel: [rng.next_u64() as i32, rng.next_u64() as i32, rng.next_u64() as i32],
            flags: rng.next_u64() as u32,
            gun: rng.next_u64() as u32,
        };
        let mut f = [0i32; 8];
        Mover::to_fields(&s, &mut f);
        if !c.check(Mover::from_fields(&f) == s, || format!("fields of {:?}", s)) {
            break;
        }
    }
    // determinism: the same run twice; the second half again from the fields of the middle
    let (figs, ticks) = (16, c.n(1200).max(240));
    let (h1, a) = random_run(&map, figs, ticks, 9);
    let (h2, _) = random_run(&map, figs, ticks, 9);
    c.check(h1 == h2, || format!("two runs differ: {:016x} {:016x}", h1, h2));
    c.note(&format!("{} figures x {} ticks: move={:016x}", figs, ticks, h1));
    // (the same inputs again from the middle: replay the generator to the middle, then continue from the fields)
    let half = ticks / 2;
    let mut rng2 = Rng::new(9);
    let mut yaw: Vec<f64> = (0..figs).map(|k| map.spawns[k % map.spawns.len()].yaw).collect();
    let mut inputs = Vec::new();
    for _ in 0..ticks {
        let row: Vec<NetInput> = (0..figs).map(|k| random_input(&mut rng2, &mut yaw[k])).collect();
        inputs.push(row);
    }
    let mut st: Vec<MoveState> = a[half - 1].iter().map(|s| {
        let mut f = [0i32; 8];
        Mover::to_fields(s, &mut f);
        Mover::from_fields(&f)
    }).collect();
    let mut same = true;
    for (t, row) in inputs.iter().enumerate().skip(half) {
        for k in 0..figs {
            st[k] = mover.step(&map.world, &st[k], &row[k], DT);
        }
        same &= st == a[t];
    }
    c.check(same, || "the run continued from the fields of its middle differs".into());
    // inside the map, above the floor
    let mut out = 0;
    for row in &a {
        for s in row {
            let p = s.feet();
            if !(p.y > -0.05 && p.x.abs() < 28.0 && p.z.abs() < 20.0) {
                out += 1;
            }
        }
    }
    c.check(out == 0, || format!("{} states outside the yard or under the floor", out));
    // speeds on open ground (west of the platform, along z): run 9 m/s, walk 5
    let start = Vec3::new(-20.0, 0.0, -2.0);
    for (walk, want) in [(false, 9.0), (true, 5.0)] {
        let mut s = on_floor(&map, start);
        let i = NetInput { buttons: if walk { BTN_WALK } else { 0 }, move_z: axis_q(1.0), yaw: angle_q(core::f64::consts::PI), ..NetInput::default() };
        for _ in 0..40 {
            s = mover.step(&map.world, &s, &i, DT);
        }
        let v = s.velocity();
        let speed = Vec3::new(v.x, 0.0, v.z).length();
        c.check((speed - want).abs() < 0.01, || format!("{} at {:.3} m/s, expected {}", if walk { "walking" } else { "running" }, speed, want));
    }
    // a jump: off at once, back on the floor within a second
    let mut s = on_floor(&map, start);
    let jump = NetInput { buttons: BTN_JUMP, ..NetInput::default() };
    s = mover.step(&map.world, &s, &jump, DT);
    c.check(s.flags & F_JUMPED != 0, || "no jump".into());
    let mut landed = None;
    for t in 1..120 {
        s = mover.step(&map.world, &s, &NetInput::default(), DT);
        if s.flags & F_LANDED != 0 {
            landed = Some(t);
            break;
        }
    }
    c.check(landed.is_some_and(|t| (38..=45).contains(&t)), || format!("landed after {:?} ticks", landed));
    // crouching lowers the eye by 0.6 m
    let s0 = on_floor(&map, start);
    let mut s1 = s0;
    for _ in 0..3 {
        s1 = mover.step(&map.world, &s1, &NetInput { buttons: BTN_CROUCH, ..NetInput::default() }, DT);
    }
    c.check(s1.crouched() && (mover.eye(&s0).y - mover.eye(&s1).y - 0.6).abs() < 1e-3, || format!("crouched eye {:?} -> {:?}", mover.eye(&s0), mover.eye(&s1)));
    // the weapon: fire held from a full magazine
    let mut s = on_floor(&map, start);
    let fire = NetInput { buttons: BTN_FIRE, ..NetInput::default() };
    let mut shots = Vec::new();
    for t in 0..240u32 {
        s = mover.step(&map.world, &s, &fire, DT);
        if s.flags & F_FIRED != 0 {
            shots.push(t);
        }
    }
    let mut want: Vec<u32> = (0..MAGAZINE).map(|k| k * FIRE_TICKS).collect();
    let empty = (MAGAZINE - 1) * FIRE_TICKS;
    want.push(empty + 1 + RELOAD_TICKS + 1);
    c.check(shots.len() > want.len() && shots[..want.len()] == want[..], || format!("shots at {:?}, expected {:?} first", &shots[..shots.len().min(24)], want));
    // R reloads a magazine that is not full, then it is full again
    let mut s = on_floor(&map, start);
    s.gun = gun(7, 0, 0, 0);
    s = mover.step(&map.world, &s, &NetInput { buttons: BTN_RELOAD, ..NetInput::default() }, DT);
    let started = s.reload() == RELOAD_TICKS;
    for _ in 0..RELOAD_TICKS {
        s = mover.step(&map.world, &s, &NetInput { buttons: BTN_RELOAD, ..NetInput::default() }, DT);
    }
    c.check(started && s.ammo() == MAGAZINE && s.reload() == 0, || format!("reload: started {}, ammo {}, left {}", started, s.ammo(), s.reload()));
    // a dead figure stands still and does not fire
    let mut d = on_floor(&map, start).killed();
    let before = d.pos;
    for _ in 0..30 {
        d = mover.step(&map.world, &d, &NetInput { buttons: BTN_FIRE | BTN_JUMP, move_z: 127, ..NetInput::default() }, DT);
    }
    c.check(d.pos == before && d.flags & F_DEAD != 0 && d.flags & F_FIRED == 0, || format!("dead figure moved or fired: {:?}", d));
}
